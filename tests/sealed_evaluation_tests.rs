use crosstalk::engines::algorithm_discovery::{
    ALGORITHM_CHALLENGE_SCHEMA, AlgorithmChallenge, AlgorithmDiscoveryLab,
};
use crosstalk::engines::objective_evaluation::{
    CandidateArtifact, EvaluationSpec, EvaluatorRegistry, MetricDirection, MetricSpec,
    ObjectiveEvaluator, WasmI64FunctionEvaluator,
};
use crosstalk::engines::sandbox::{I64TestCase, SandboxConfig, SandboxManager};
use crosstalk::engines::sealed_evaluation::{
    InProcessSealedTransport, SEALED_REQUEST_SCHEMA, SealedEvaluationRequest,
    SealedEvaluatorClient, SealedEvaluatorWorker, verify_embedded_attestation, verify_receipt,
};
use std::collections::BTreeMap;
use std::sync::Arc;

fn now() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_secs()
}

fn cases() -> Vec<I64TestCase> {
    vec![
        I64TestCase {
            input: i64::MIN + 101,
            expected: i64::MIN + 101,
        },
        I64TestCase {
            input: -17,
            expected: -17,
        },
        I64TestCase {
            input: 0,
            expected: 0,
        },
        I64TestCase {
            input: 999_983,
            expected: 999_983,
        },
    ]
}

fn specification(reproduction_id: Option<&str>, distinct_keys: bool) -> EvaluationSpec {
    EvaluationSpec {
        id: "sealed-identity-efficiency".into(),
        version: "1".into(),
        description: "Exact private cases with signed worker receipts".into(),
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
        independent_reproduction_required: reproduction_id.is_some(),
        reproduction_evaluator_id: reproduction_id.map(str::to_owned),
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

fn worker(evaluator_id: &str, worker_id: &str, seed: u8) -> (Arc<SealedEvaluatorWorker>, String) {
    let hidden = cases();
    let commitment = WasmI64FunctionEvaluator::commitment_for("solve", &hidden).unwrap();
    let sandbox = Arc::new(
        SandboxManager::new(SandboxConfig {
            timeout_secs: 5,
            ..SandboxConfig::default()
        })
        .unwrap(),
    );
    let evaluator = Arc::new(
        WasmI64FunctionEvaluator::new(sandbox, evaluator_id, "1", "solve", hidden).unwrap(),
    );
    (
        Arc::new(SealedEvaluatorWorker::from_seed(worker_id, evaluator, [seed; 32]).unwrap()),
        commitment,
    )
}

fn client(
    evaluator_id: &str,
    worker: Arc<SealedEvaluatorWorker>,
    commitment: &str,
) -> Arc<SealedEvaluatorClient> {
    Arc::new(
        SealedEvaluatorClient::new(
            evaluator_id,
            "1",
            commitment,
            worker.verifying_key(),
            Arc::new(InProcessSealedTransport::new(worker)),
        )
        .unwrap(),
    )
}

fn request(
    evaluator_id: &str,
    commitment: &str,
    candidate: CandidateArtifact,
) -> SealedEvaluationRequest {
    let issued_at = now();
    SealedEvaluationRequest {
        schema: SEALED_REQUEST_SCHEMA.into(),
        evaluator_id: evaluator_id.into(),
        evaluator_version: "1".into(),
        test_commitment_sha256: commitment.into(),
        evaluation: specification(None, false),
        candidate,
        nonce: "11".repeat(32),
        issued_at,
        expires_at: issued_at + 120,
    }
}

#[tokio::test]
async fn sealed_algorithm_lab_requires_two_signed_independent_workers() {
    let (primary_worker, commitment) = worker("sealed-primary", "worker-primary", 7);
    let (reproduction_worker, second_commitment) =
        worker("sealed-reproduction", "worker-reproduction", 9);
    assert_eq!(commitment, second_commitment);
    let primary = client("sealed-primary", primary_worker, &commitment);
    let reproduction = client("sealed-reproduction", reproduction_worker, &commitment);
    assert_ne!(
        primary.attestation_key_sha256(),
        reproduction.attestation_key_sha256()
    );

    let mut registry = EvaluatorRegistry::default();
    registry.register(primary).unwrap();
    registry.register(reproduction).unwrap();
    let challenge = AlgorithmChallenge {
        schema: ALGORITHM_CHALLENGE_SCHEMA.into(),
        id: "sealed-identity-v1".into(),
        title: "Improve identity implementation under sealed tests".into(),
        evaluator_id: "sealed-primary".into(),
        evaluation: specification(Some("sealed-reproduction"), true),
        primary_metric: "fuel_consumed".into(),
        minimum_improvement: 1.0,
        hidden_test_commitment_sha256: commitment,
        baseline: candidate("baseline", 100),
        max_candidates: 2,
    };
    let report = AlgorithmDiscoveryLab::new(&registry)
        .run(&challenge, &[candidate("optimized", 0)])
        .await
        .unwrap();
    assert_eq!(report.winner_candidate_id.as_deref(), Some("optimized"));
    verify_embedded_attestation(&report.baseline.primary).unwrap();
    verify_embedded_attestation(report.baseline.reproduction.as_ref().unwrap()).unwrap();
    let winner = report.winner().unwrap().outcome.as_ref().unwrap();
    verify_embedded_attestation(&winner.primary).unwrap();
    verify_embedded_attestation(winner.reproduction.as_ref().unwrap()).unwrap();
    assert_ne!(
        winner.primary.environment["sealed_worker_key_sha256"],
        winner.reproduction.as_ref().unwrap().environment["sealed_worker_key_sha256"]
    );

    let public_report = serde_json::to_string(&report).unwrap();
    assert!(!public_report.contains("-9223372036854775707"));
    assert!(!public_report.contains("999983"));
}

#[tokio::test]
async fn worker_rejects_exact_request_replay_even_after_success() {
    let (worker, commitment) = worker("sealed-primary", "worker-primary", 1);
    let request = request("sealed-primary", &commitment, candidate("identity", 0));
    worker.handle(request.clone()).await.unwrap();
    let error = worker.handle(request).await.unwrap_err();
    assert!(error.to_string().contains("replay"));
}

#[tokio::test]
async fn receipt_verification_rejects_result_tampering_and_wrong_keys() {
    let (primary_worker, commitment) = worker("sealed-primary", "worker-primary", 2);
    let request = request("sealed-primary", &commitment, candidate("identity", 0));
    let receipt = primary_worker.handle(request.clone()).await.unwrap();
    verify_receipt(&request, &receipt, &primary_worker.verifying_key(), now()).unwrap();

    let mut tampered = receipt.clone();
    tampered.evaluation.measurements[0].value = 0.5;
    assert!(verify_receipt(&request, &tampered, &primary_worker.verifying_key(), now()).is_err());

    let (other_worker, _) = worker("other", "other-worker", 3);
    let error =
        verify_receipt(&request, &receipt, &other_worker.verifying_key(), now()).unwrap_err();
    assert!(error.to_string().contains("pinned"));
}

#[tokio::test]
async fn embedded_attestation_detects_post_verification_mutation() {
    let (worker, commitment) = worker("sealed-primary", "worker-primary", 4);
    let evaluator = client("sealed-primary", worker, &commitment);
    let mut result = evaluator
        .evaluate(&specification(None, false), &candidate("identity", 0))
        .await
        .unwrap();
    verify_embedded_attestation(&result).unwrap();
    result.measurements[0].value = 0.25;
    assert!(verify_embedded_attestation(&result).is_err());
}

#[tokio::test]
async fn distinct_worker_policy_rejects_different_labels_on_the_same_key() {
    let (primary_worker, commitment) = worker("sealed-primary", "primary-worker", 5);
    let (reproduction_worker, _) = worker("sealed-reproduction", "repro-worker", 5);
    let mut registry = EvaluatorRegistry::default();
    registry
        .register(client("sealed-primary", primary_worker, &commitment))
        .unwrap();
    registry
        .register(client(
            "sealed-reproduction",
            reproduction_worker,
            &commitment,
        ))
        .unwrap();
    let error = registry
        .evaluate_with_reproduction(
            "sealed-primary",
            &specification(Some("sealed-reproduction"), true),
            &candidate("identity", 0),
        )
        .await
        .unwrap_err();
    assert!(error.to_string().contains("distinct attestation keys"));
}

#[tokio::test]
async fn expired_requests_fail_before_hidden_tests_execute() {
    let (worker, commitment) = worker("sealed-primary", "worker-primary", 6);
    let mut request = request("sealed-primary", &commitment, candidate("identity", 0));
    request.issued_at = now() - 300;
    request.expires_at = now() - 120;
    let error = worker.handle(request).await.unwrap_err();
    assert!(error.to_string().contains("expired"));
}
