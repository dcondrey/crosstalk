use crosstalk::engines::algorithm_discovery::AlgorithmDiscoveryReport;
use crosstalk::engines::objective_evaluation::{
    EvaluationSpec, MetricDirection, MetricSpec, WASM_I64_TEST_SCHEMA, WasmI64FunctionEvaluator,
};
use crosstalk::engines::sandbox::I64TestCase;
use serde_json::json;
use std::fs;
use std::process::Command;
use tempfile::tempdir;

fn binary() -> &'static str {
    env!("CARGO_BIN_EXE_crosstalk-lab")
}

fn write_fixture_files(
    root: &std::path::Path,
) -> (
    std::path::PathBuf,
    std::path::PathBuf,
    std::path::PathBuf,
    String,
) {
    let cases = vec![
        I64TestCase {
            input: i64::MIN + 17,
            expected: i64::MIN + 17,
        },
        I64TestCase {
            input: -91,
            expected: -91,
        },
        I64TestCase {
            input: 0,
            expected: 0,
        },
        I64TestCase {
            input: 8_675_309,
            expected: 8_675_309,
        },
    ];
    let commitment = WasmI64FunctionEvaluator::commitment_for("solve", &cases).unwrap();
    let hidden_tests_path = root.join("private-tests.json");
    fs::write(
        &hidden_tests_path,
        serde_json::to_vec_pretty(&json!({
            "schema": WASM_I64_TEST_SCHEMA,
            "export_name": "solve",
            "cases": cases,
        }))
        .unwrap(),
    )
    .unwrap();

    let extra_work = "i64.const 0\ni64.add\n".repeat(100);
    let baseline_wat = format!(
        "(module (func (export \"solve\") (param i64) (result i64) local.get 0 {extra_work}))"
    );
    let baseline_path = root.join("baseline.wasm");
    fs::write(&baseline_path, wat::parse_str(baseline_wat).unwrap()).unwrap();
    let candidate_path = root.join("candidate.wasm");
    fs::write(
        &candidate_path,
        wat::parse_str(r#"(module (func (export "solve") (param i64) (result i64) local.get 0))"#)
            .unwrap(),
    )
    .unwrap();

    let spec = EvaluationSpec {
        id: "identity-efficiency".into(),
        version: "1".into(),
        description: "Preserve exact outputs while reducing metered work".into(),
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
        independent_reproduction_required: true,
        reproduction_evaluator_id: Some("wasm-i64-reproduction".into()),
        distinct_attestation_keys_required: false,
    };
    let challenge_path = root.join("challenge.json");
    fs::write(
        &challenge_path,
        serde_json::to_vec_pretty(&json!({
            "schema": "crosstalk.algorithm-challenge-file.v1",
            "id": "identity-efficiency-v1",
            "title": "Reduce metered instructions",
            "evaluator_id": "wasm-i64-primary",
            "evaluation": spec,
            "primary_metric": "fuel_consumed",
            "minimum_improvement": 1.0,
            "hidden_test_commitment_sha256": commitment,
            "baseline_id": "baseline",
            "max_candidates": 4,
        }))
        .unwrap(),
    )
    .unwrap();
    (
        challenge_path,
        hidden_tests_path,
        baseline_path,
        candidate_path.display().to_string(),
    )
}

#[test]
fn commitment_command_matches_the_library_canonicalization() {
    let temp = tempdir().unwrap();
    let (_, hidden_tests, _, _) = write_fixture_files(temp.path());
    let output = Command::new(binary())
        .args([
            "commitment",
            "--hidden-tests",
            hidden_tests.to_str().unwrap(),
        ])
        .output()
        .unwrap();
    assert!(output.status.success(), "{:?}", output);
    let file: serde_json::Value = serde_json::from_slice(&fs::read(hidden_tests).unwrap()).unwrap();
    let cases: Vec<I64TestCase> = serde_json::from_value(file["cases"].clone()).unwrap();
    let expected = WasmI64FunctionEvaluator::commitment_for("solve", &cases).unwrap();
    assert_eq!(String::from_utf8(output.stdout).unwrap().trim(), expected);
}

#[test]
fn lab_cli_selects_only_a_correct_reproduced_improvement() {
    let temp = tempdir().unwrap();
    let (challenge, hidden_tests, baseline, candidate) = write_fixture_files(temp.path());
    let incorrect_path = temp.path().join("incorrect.wasm");
    fs::write(
        &incorrect_path,
        wat::parse_str(r#"(module (func (export "solve") (param i64) (result i64) i64.const 0))"#)
            .unwrap(),
    )
    .unwrap();
    let optimized_argument = format!("optimized={candidate}");
    let incorrect_argument = format!("incorrect={}", incorrect_path.display());
    let output = Command::new(binary())
        .args([
            "run",
            "--challenge",
            challenge.to_str().unwrap(),
            "--hidden-tests",
            hidden_tests.to_str().unwrap(),
            "--baseline",
            baseline.to_str().unwrap(),
            "--candidate",
            &optimized_argument,
            "--candidate",
            &incorrect_argument,
            "--fuel",
            "100000",
            "--timeout-secs",
            "5",
        ])
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let text = String::from_utf8(output.stdout).unwrap();
    let report: AlgorithmDiscoveryReport = serde_json::from_str(&text).unwrap();
    assert_eq!(report.winner_candidate_id.as_deref(), Some("optimized"));
    assert!(report.winner_improvement.unwrap() > 0.0);
    assert!(report.baseline.agreed);
    assert!(report.winner().unwrap().outcome.as_ref().unwrap().agreed);
    let incorrect = report
        .trials
        .iter()
        .find(|trial| trial.candidate_id == "incorrect")
        .unwrap();
    assert!(!incorrect.eligible);
    assert!(
        incorrect
            .rejection_reasons
            .iter()
            .any(|reason| reason.contains("hard constraints"))
    );
    assert!(!text.contains("\"input\""));
    assert!(!text.contains("\"expected\""));
    assert!(!text.contains("-9223372036854775791"));
}

#[test]
fn lab_cli_rejects_a_test_file_that_does_not_match_the_challenge() {
    let temp = tempdir().unwrap();
    let (challenge, hidden_tests, baseline, candidate) = write_fixture_files(temp.path());
    let mut definition: serde_json::Value =
        serde_json::from_slice(&fs::read(&challenge).unwrap()).unwrap();
    definition["hidden_test_commitment_sha256"] = json!("a".repeat(64));
    fs::write(&challenge, serde_json::to_vec_pretty(&definition).unwrap()).unwrap();

    let output = Command::new(binary())
        .args([
            "run",
            "--challenge",
            challenge.to_str().unwrap(),
            "--hidden-tests",
            hidden_tests.to_str().unwrap(),
            "--baseline",
            baseline.to_str().unwrap(),
            "--candidate",
            &format!("optimized={candidate}"),
        ])
        .output()
        .unwrap();
    assert!(!output.status.success());
    assert!(String::from_utf8_lossy(&output.stderr).contains("commitment does not match"));
}
