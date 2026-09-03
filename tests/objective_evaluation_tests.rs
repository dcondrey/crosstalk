use async_trait::async_trait;
use crosstalk::engines::objective_evaluation::{
    CandidateArtifact, ConstraintResult, EvaluationError, EvaluationSpec, EvaluatorRegistry,
    MetricDirection, MetricSpec, ObjectiveEvaluation, ObjectiveEvaluator, WasmExecutionEvaluator,
    WasmI64FunctionEvaluator,
};
use crosstalk::engines::sandbox::{I64TestCase, SandboxConfig, SandboxManager};
use crosstalk::types::investigation::{Measurement, VerificationStatus};
use sha2::{Digest, Sha256};
use std::collections::BTreeMap;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};

fn spec(reproduce: bool) -> EvaluationSpec {
    EvaluationSpec {
        id: "runtime".into(),
        version: "1".into(),
        description: "Execute a candidate under bounded WASM".into(),
        metrics: vec![MetricSpec {
            name: "elapsed_ms".into(),
            unit: "ms".into(),
            direction: MetricDirection::Minimize,
            reproduction_tolerance: 0.0,
        }],
        hard_constraints: vec!["exit_code_zero".into(), "resource_limit_not_hit".into()],
        timeout_secs: 5,
        deterministic: true,
        independent_reproduction_required: reproduce,
        reproduction_evaluator_id: reproduce.then(|| "counting-reproduction".into()),
        distinct_attestation_keys_required: false,
    }
}

fn candidate() -> CandidateArtifact {
    // A module whose default export is an empty () -> () function.
    CandidateArtifact {
        id: "candidate:1".into(),
        media_type: "application/wasm".into(),
        content: vec![
            0x00, 0x61, 0x73, 0x6D, 0x01, 0x00, 0x00, 0x00, // header
            0x01, 0x04, 0x01, 0x60, 0x00, 0x00, // type () -> ()
            0x03, 0x02, 0x01, 0x00, // function
            0x07, 0x04, 0x01, 0x00, 0x00, 0x00, // default export
            0x0A, 0x04, 0x01, 0x02, 0x00, 0x0B, // empty body
        ],
        metadata: BTreeMap::new(),
    }
}

#[tokio::test]
async fn wasm_evaluator_produces_a_valid_objective_result() {
    let sandbox = Arc::new(
        SandboxManager::new(SandboxConfig {
            timeout_secs: 5,
            ..SandboxConfig::default()
        })
        .unwrap(),
    );
    let mut registry = EvaluatorRegistry::default();
    registry
        .register(Arc::new(WasmExecutionEvaluator::new(sandbox, "29")))
        .unwrap();

    let result = registry
        .evaluate("wasm-execution", &spec(false), &candidate())
        .await
        .unwrap();
    assert!(result.is_verified(), "{}", result.diagnostics);
    assert_eq!(result.measurements[0].name, "elapsed_ms");
    assert_eq!(result.candidate_sha256.len(), 64);
    assert_eq!(result.raw_output_sha256.len(), 64);
}

#[tokio::test]
async fn wasm_function_evaluator_commits_to_cases_without_exposing_them() {
    let cases = vec![
        I64TestCase {
            input: -7,
            expected: -7,
        },
        I64TestCase {
            input: 42,
            expected: 42,
        },
    ];
    let sandbox = Arc::new(SandboxManager::new(SandboxConfig::default()).unwrap());
    let evaluator =
        WasmI64FunctionEvaluator::new(sandbox, "hidden-i64", "1", "solve", cases.clone()).unwrap();
    let commitment = evaluator.test_commitment_sha256().unwrap().to_owned();
    assert_eq!(commitment.len(), 64);
    assert_ne!(
        commitment,
        WasmI64FunctionEvaluator::commitment_for(
            "solve",
            &[I64TestCase {
                input: -7,
                expected: 0,
            }]
        )
        .unwrap()
    );

    let mut registry = EvaluatorRegistry::default();
    registry.register(Arc::new(evaluator)).unwrap();
    let candidate = CandidateArtifact {
        id: "identity".into(),
        media_type: "application/wasm".into(),
        content: wat::parse_str(
            r#"(module (func (export "solve") (param i64) (result i64) local.get 0))"#,
        )
        .unwrap(),
        metadata: BTreeMap::new(),
    };
    let specification = EvaluationSpec {
        id: "hidden-function".into(),
        version: "1".into(),
        description: "Committed i64 hidden tests".into(),
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
        independent_reproduction_required: false,
        reproduction_evaluator_id: None,
        distinct_attestation_keys_required: false,
    };
    let result = registry
        .evaluate("hidden-i64", &specification, &candidate)
        .await
        .unwrap();
    assert!(result.is_verified());
    assert_eq!(result.environment["test_set_commitment_sha256"], commitment);
    assert!(!result.diagnostics.contains("-7"));
    assert!(!result.diagnostics.contains("42"));
}

#[test]
fn duplicate_metrics_make_a_spec_invalid() {
    let mut invalid = spec(false);
    invalid.metrics.push(invalid.metrics[0].clone());
    assert!(invalid.validate().is_err());
}

#[test]
fn result_rejects_duplicate_or_undeclared_measurements() {
    let specification = spec(false);
    let mut result = ObjectiveEvaluation {
        id: "evaluation:shape".into(),
        evaluator_id: "shape".into(),
        evaluator_version: "1".into(),
        specification_sha256: specification.sha256().unwrap(),
        candidate_id: "candidate".into(),
        candidate_sha256: "a".repeat(64),
        status: VerificationStatus::Verified,
        measurements: vec![Measurement {
            name: "elapsed_ms".into(),
            value: 1.0,
            unit: "ms".into(),
            uncertainty: None,
            sample_size: Some(1),
        }],
        constraints: specification
            .hard_constraints
            .iter()
            .map(|name| ConstraintResult {
                name: name.clone(),
                passed: true,
                diagnostics: String::new(),
            })
            .collect(),
        diagnostics: String::new(),
        raw_output_sha256: "b".repeat(64),
        started_at: 1,
        completed_at: 1,
        environment: BTreeMap::new(),
    };
    result.measurements.push(result.measurements[0].clone());
    assert!(result.validate_against(&specification).is_err());
    result.measurements[1].name = "undeclared".into();
    assert!(result.validate_against(&specification).is_err());
}

struct CountingEvaluator {
    id: &'static str,
    offset: f64,
    calls: AtomicU64,
}

#[async_trait]
impl ObjectiveEvaluator for CountingEvaluator {
    fn id(&self) -> &str {
        self.id
    }

    fn version(&self) -> &str {
        "1"
    }

    async fn evaluate(
        &self,
        spec: &EvaluationSpec,
        candidate: &CandidateArtifact,
    ) -> Result<ObjectiveEvaluation, EvaluationError> {
        let value = self.calls.fetch_add(1, Ordering::SeqCst) as f64 + self.offset;
        let raw = format!("value={value}");
        Ok(ObjectiveEvaluation {
            id: format!("evaluation:{value}"),
            evaluator_id: self.id().into(),
            evaluator_version: self.version().into(),
            specification_sha256: spec.sha256()?,
            candidate_id: candidate.id.clone(),
            candidate_sha256: candidate.sha256(),
            status: VerificationStatus::Verified,
            measurements: vec![Measurement {
                name: "elapsed_ms".into(),
                value,
                unit: "ms".into(),
                uncertainty: None,
                sample_size: Some(1),
            }],
            constraints: vec![
                ConstraintResult {
                    name: "exit_code_zero".into(),
                    passed: true,
                    diagnostics: "passed".into(),
                },
                ConstraintResult {
                    name: "resource_limit_not_hit".into(),
                    passed: true,
                    diagnostics: "passed".into(),
                },
            ],
            diagnostics: "ok".into(),
            raw_output_sha256: format!("{:x}", Sha256::digest(raw.as_bytes())),
            started_at: 1,
            completed_at: 1,
            environment: BTreeMap::new(),
        })
    }
}

#[tokio::test]
async fn required_reproduction_detects_metric_drift() {
    let mut registry = EvaluatorRegistry::default();
    registry
        .register(Arc::new(CountingEvaluator {
            id: "counting",
            offset: 0.0,
            calls: AtomicU64::new(0),
        }))
        .unwrap();
    registry
        .register(Arc::new(CountingEvaluator {
            id: "counting-reproduction",
            offset: 1.0,
            calls: AtomicU64::new(0),
        }))
        .unwrap();
    let outcome = registry
        .evaluate_with_reproduction("counting", &spec(true), &candidate())
        .await
        .unwrap();
    assert!(!outcome.agreed);
    assert_eq!(outcome.mismatches.len(), 1);
}

#[tokio::test]
async fn registry_rejects_unknown_evaluators() {
    let registry = EvaluatorRegistry::default();
    let error = registry
        .evaluate("missing", &spec(false), &candidate())
        .await
        .unwrap_err();
    assert!(matches!(error, EvaluationError::UnknownEvaluator(_)));
}

#[tokio::test]
async fn reproduction_requires_a_distinct_registered_evaluator() {
    let mut registry = EvaluatorRegistry::default();
    registry
        .register(Arc::new(CountingEvaluator {
            id: "counting",
            offset: 0.0,
            calls: AtomicU64::new(0),
        }))
        .unwrap();
    let mut invalid = spec(true);
    invalid.reproduction_evaluator_id = Some("counting".into());
    let error = registry
        .evaluate_with_reproduction("counting", &invalid, &candidate())
        .await
        .unwrap_err();
    assert!(error.to_string().contains("identities must differ"));
}
