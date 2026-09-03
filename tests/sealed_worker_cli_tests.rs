use crosstalk::engines::algorithm_discovery::AlgorithmDiscoveryReport;
use crosstalk::engines::objective_evaluation::{
    CandidateArtifact, EvaluationSpec, MetricDirection, MetricSpec, WASM_I64_TEST_SCHEMA,
    WasmI64FunctionEvaluator,
};
use crosstalk::engines::sandbox::I64TestCase;
use crosstalk::engines::sealed_evaluation::{
    ProcessSealedTransport, SEALED_REQUEST_SCHEMA, SealedEvaluationRequest,
    SealedEvaluationTransport, verify_embedded_attestation, verify_receipt,
};
use ed25519_dalek::VerifyingKey;
use serde_json::json;
use std::collections::BTreeMap;
use std::ffi::OsString;
use std::fs;
use std::process::Command;
use tempfile::tempdir;

fn worker_binary() -> &'static str {
    env!("CARGO_BIN_EXE_crosstalk-worker")
}

fn lab_binary() -> &'static str {
    env!("CARGO_BIN_EXE_crosstalk-lab")
}

fn cases() -> Vec<I64TestCase> {
    vec![
        I64TestCase {
            input: -31,
            expected: -31,
        },
        I64TestCase {
            input: 0,
            expected: 0,
        },
        I64TestCase {
            input: 65_537,
            expected: 65_537,
        },
    ]
}

fn spec(reproduction: Option<&str>, distinct_keys: bool) -> EvaluationSpec {
    EvaluationSpec {
        id: "remote-sealed-identity".into(),
        version: "1".into(),
        description: "Remote sealed exact cases".into(),
        metrics: vec![
            MetricSpec {
                name: "accuracy".into(),
                unit: "ratio".into(),
                direction: MetricDirection::Maximize,
                reproduction_tolerance: 0.0,
            },
            MetricSpec {
                name: "fuel_consumed".into(),
                unit: "fuel".into(),
                direction: MetricDirection::Minimize,
                reproduction_tolerance: 0.0,
            },
        ],
        hard_constraints: vec!["all_cases_correct".into(), "resource_limit_not_hit".into()],
        timeout_secs: 5,
        deterministic: true,
        independent_reproduction_required: reproduction.is_some(),
        reproduction_evaluator_id: reproduction.map(str::to_owned),
        distinct_attestation_keys_required: distinct_keys,
    }
}

fn candidate(id: &str, extra_work: usize) -> CandidateArtifact {
    let work = "i64.const 0\ni64.add\n".repeat(extra_work);
    CandidateArtifact {
        id: id.into(),
        media_type: "application/wasm".into(),
        content: wat::parse_str(format!(
            "(module (func (export \"solve\") (param i64) (result i64) local.get 0 {work}))"
        ))
        .unwrap(),
        metadata: BTreeMap::new(),
    }
}

fn write_hidden_tests(root: &std::path::Path) -> (std::path::PathBuf, String) {
    let hidden = cases();
    let commitment = WasmI64FunctionEvaluator::commitment_for("solve", &hidden).unwrap();
    let path = root.join("hidden.json");
    fs::write(
        &path,
        serde_json::to_vec(&json!({
            "schema": WASM_I64_TEST_SCHEMA,
            "export_name": "solve",
            "cases": hidden,
        }))
        .unwrap(),
    )
    .unwrap();
    (path, commitment)
}

fn keygen(path: &std::path::Path) -> serde_json::Value {
    let output = Command::new(worker_binary())
        .args(["keygen", "--output", path.to_str().unwrap()])
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    serde_json::from_slice(&output.stdout).unwrap()
}

fn decode_key(value: &str) -> VerifyingKey {
    let mut bytes = [0_u8; 32];
    for (index, byte) in bytes.iter_mut().enumerate() {
        *byte = u8::from_str_radix(&value[index * 2..index * 2 + 2], 16).unwrap();
    }
    VerifyingKey::from_bytes(&bytes).unwrap()
}

fn worker_args(
    key: &std::path::Path,
    worker_id: &str,
    evaluator_id: &str,
    hidden: &std::path::Path,
    replay_db: &std::path::Path,
) -> Vec<String> {
    vec![
        "evaluate-i64".into(),
        "--key".into(),
        key.display().to_string(),
        "--worker-id".into(),
        worker_id.into(),
        "--evaluator-id".into(),
        evaluator_id.into(),
        "--evaluator-version".into(),
        "1".into(),
        "--hidden-tests".into(),
        hidden.display().to_string(),
        "--replay-db".into(),
        replay_db.display().to_string(),
        "--timeout-secs".into(),
        "5".into(),
    ]
}

#[tokio::test]
async fn process_transport_verifies_receipts_and_durable_replay_rejection() {
    let temp = tempdir().unwrap();
    let key_path = temp.path().join("worker.key");
    let identity = keygen(&key_path);
    let key = decode_key(identity["verifying_key_hex"].as_str().unwrap());
    let (hidden, commitment) = write_hidden_tests(temp.path());
    let replay_db = temp.path().join("replays");
    let args = worker_args(
        &key_path,
        "remote-worker",
        "remote-evaluator",
        &hidden,
        &replay_db,
    );
    let transport = ProcessSealedTransport::new(
        worker_binary(),
        args.iter().cloned().map(OsString::from),
        10,
    )
    .unwrap();
    let issued_at = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_secs();
    let request = SealedEvaluationRequest {
        schema: SEALED_REQUEST_SCHEMA.into(),
        evaluator_id: "remote-evaluator".into(),
        evaluator_version: "1".into(),
        test_commitment_sha256: commitment,
        evaluation: spec(None, false),
        candidate: candidate("identity", 0),
        nonce: "ab".repeat(32),
        issued_at,
        expires_at: issued_at + 120,
    };
    let receipt = transport.submit(request.clone()).await.unwrap();
    verify_receipt(&request, &receipt, &key, issued_at).unwrap();
    let error = transport.submit(request).await.unwrap_err();
    assert!(error.to_string().contains("replay"));
}

#[test]
fn run_sealed_cli_keeps_holdouts_outside_the_lab_process() {
    let temp = tempdir().unwrap();
    let primary_key = temp.path().join("primary.key");
    let reproduction_key = temp.path().join("reproduction.key");
    let primary_identity = keygen(&primary_key);
    let reproduction_identity = keygen(&reproduction_key);
    let (hidden, commitment) = write_hidden_tests(temp.path());
    let baseline_path = temp.path().join("baseline.wasm");
    let candidate_path = temp.path().join("candidate.wasm");
    fs::write(&baseline_path, candidate("baseline", 100).content).unwrap();
    fs::write(&candidate_path, candidate("optimized", 0).content).unwrap();

    let challenge_path = temp.path().join("challenge.json");
    fs::write(
        &challenge_path,
        serde_json::to_vec_pretty(&json!({
            "schema": "crosstalk.algorithm-challenge-file.v1",
            "id": "remote-sealed-identity-v1",
            "title": "Remote sealed identity optimization",
            "evaluator_id": "remote-primary",
            "evaluation": spec(Some("remote-reproduction"), true),
            "primary_metric": "fuel_consumed",
            "minimum_improvement": 1.0,
            "hidden_test_commitment_sha256": commitment.clone(),
            "baseline_id": "baseline",
            "max_candidates": 2,
        }))
        .unwrap(),
    )
    .unwrap();

    let primary_endpoint = temp.path().join("primary-endpoint.json");
    let reproduction_endpoint = temp.path().join("reproduction-endpoint.json");
    let primary_args = worker_args(
        &primary_key,
        "primary-worker",
        "remote-primary",
        &hidden,
        &temp.path().join("primary-replays"),
    );
    let reproduction_args = worker_args(
        &reproduction_key,
        "reproduction-worker",
        "remote-reproduction",
        &hidden,
        &temp.path().join("reproduction-replays"),
    );
    for (path, evaluator_id, identity, args) in [
        (
            &primary_endpoint,
            "remote-primary",
            &primary_identity,
            primary_args,
        ),
        (
            &reproduction_endpoint,
            "remote-reproduction",
            &reproduction_identity,
            reproduction_args,
        ),
    ] {
        fs::write(
            path,
            serde_json::to_vec_pretty(&json!({
                "schema": "crosstalk.sealed-worker-endpoint.v1",
                "evaluator_id": evaluator_id,
                "evaluator_version": "1",
                "test_commitment_sha256": commitment.clone(),
                "verifying_key_hex": identity["verifying_key_hex"],
                "program": worker_binary(),
                "args": args,
                "process_timeout_secs": 10,
            }))
            .unwrap(),
        )
        .unwrap();
    }

    let candidate_argument = format!("optimized={}", candidate_path.display());
    let output = Command::new(lab_binary())
        .args([
            "run-sealed",
            "--challenge",
            challenge_path.to_str().unwrap(),
            "--primary-endpoint",
            primary_endpoint.to_str().unwrap(),
            "--reproduction-endpoint",
            reproduction_endpoint.to_str().unwrap(),
            "--baseline",
            baseline_path.to_str().unwrap(),
            "--candidate",
            &candidate_argument,
        ])
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let report: AlgorithmDiscoveryReport = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(report.winner_candidate_id.as_deref(), Some("optimized"));
    verify_embedded_attestation(&report.baseline.primary).unwrap();
    verify_embedded_attestation(report.baseline.reproduction.as_ref().unwrap()).unwrap();
    let serialized = String::from_utf8(output.stdout).unwrap();
    assert!(!serialized.contains("\"input\""));
    assert!(!serialized.contains("\"expected\""));
}

#[cfg(unix)]
#[test]
fn generated_worker_keys_are_owner_only_and_create_only() {
    use std::os::unix::fs::MetadataExt;
    let temp = tempdir().unwrap();
    let key_path = temp.path().join("worker.key");
    keygen(&key_path);
    assert_eq!(fs::metadata(&key_path).unwrap().mode() & 0o777, 0o600);
    let second = Command::new(worker_binary())
        .args(["keygen", "--output", key_path.to_str().unwrap()])
        .output()
        .unwrap();
    assert!(!second.status.success());
}
