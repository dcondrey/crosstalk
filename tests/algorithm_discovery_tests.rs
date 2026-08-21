use async_trait::async_trait;
use crosstalk::engines::algorithm_discovery::{
    ALGORITHM_CHALLENGE_SCHEMA, AlgorithmChallenge, AlgorithmDiscoveryLab,
};
use crosstalk::engines::objective_evaluation::{
    CandidateArtifact, ConstraintResult, EvaluationError, EvaluationSpec, EvaluatorRegistry,
    MetricDirection, MetricSpec, ObjectiveEvaluation, ObjectiveEvaluator,
};
use crosstalk::types::investigation::{Measurement, VerificationStatus};
use sha2::{Digest, Sha256};
use std::collections::BTreeMap;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};

struct NumericEvaluator {
    id: &'static str,
    reproduction: bool,
    sequence: AtomicU64,
}

#[async_trait]
impl ObjectiveEvaluator for NumericEvaluator {
    fn id(&self) -> &str {
        self.id
    }

    fn version(&self) -> &str {
        "1"
    }

    fn test_commitment_sha256(&self) -> Option<&str> {
        Some("cccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccc")
    }

    async fn evaluate(
        &self,
        spec: &EvaluationSpec,
        candidate: &CandidateArtifact,
    ) -> Result<ObjectiveEvaluation, EvaluationError> {
        let sequence = self.sequence.fetch_add(1, Ordering::SeqCst) + 1;
        let text = std::str::from_utf8(&candidate.content)
            .map_err(|error| EvaluationError::Evaluator(error.to_string()))?;
        let (value, passes) = if let Some(value) = text.strip_prefix("bad:") {
            (value.parse::<f64>().unwrap(), false)
        } else if let Some(value) = text.strip_prefix("drift:") {
            let base = value.parse::<f64>().unwrap();
            (base + if self.reproduction { 1.0 } else { 0.0 }, true)
        } else {
            (text.parse::<f64>().unwrap(), true)
        };
        let raw_output_sha256 = format!(
            "{:x}",
            Sha256::digest(format!("{sequence}:{value}:{passes}").as_bytes())
        );
        Ok(ObjectiveEvaluation {
            id: format!("numeric:{sequence}"),
            evaluator_id: self.id().into(),
            evaluator_version: self.version().into(),
            specification_sha256: spec.sha256()?,
            candidate_id: candidate.id.clone(),
            candidate_sha256: candidate.sha256(),
            status: if passes {
                VerificationStatus::Verified
            } else {
                VerificationStatus::Rejected
            },
            measurements: vec![Measurement {
                name: "runtime_ms".into(),
                value,
                unit: "ms".into(),
                uncertainty: None,
                sample_size: Some(1),
            }],
            constraints: vec![ConstraintResult {
                name: "correctness".into(),
                passed: passes,
                diagnostics: if passes { "passed" } else { "failed" }.into(),
            }],
            diagnostics: String::new(),
            raw_output_sha256,
            started_at: sequence,
            completed_at: sequence,
            environment: BTreeMap::new(),
        })
    }
}

fn artifact(id: &str, content: &str) -> CandidateArtifact {
    CandidateArtifact {
        id: id.into(),
        media_type: "application/test-number".into(),
        content: content.as_bytes().to_vec(),
        metadata: BTreeMap::new(),
    }
}

fn challenge() -> AlgorithmChallenge {
    AlgorithmChallenge {
        schema: ALGORITHM_CHALLENGE_SCHEMA.into(),
        id: "sorting-runtime-v1".into(),
        title: "Improve runtime while preserving correctness".into(),
        evaluator_id: "numeric".into(),
        evaluation: EvaluationSpec {
            id: "numeric-runtime".into(),
            version: "1".into(),
            description: "A deterministic test evaluator".into(),
            metrics: vec![MetricSpec {
                name: "runtime_ms".into(),
                unit: "ms".into(),
                direction: MetricDirection::Minimize,
                reproduction_tolerance: 0.0,
            }],
            hard_constraints: vec!["correctness".into()],
            timeout_secs: 2,
            deterministic: true,
            independent_reproduction_required: true,
            reproduction_evaluator_id: Some("numeric-reproduction".into()),
        },
        primary_metric: "runtime_ms".into(),
        minimum_improvement: 1.0,
        hidden_test_commitment_sha256: "c".repeat(64),
        baseline: artifact("baseline", "10"),
        max_candidates: 4,
    }
}

fn registry() -> EvaluatorRegistry {
    let mut registry = EvaluatorRegistry::default();
    registry
        .register(Arc::new(NumericEvaluator {
            id: "numeric",
            reproduction: false,
            sequence: AtomicU64::new(0),
        }))
        .unwrap();
    registry
        .register(Arc::new(NumericEvaluator {
            id: "numeric-reproduction",
            reproduction: true,
            sequence: AtomicU64::new(0),
        }))
        .unwrap();
    registry
}

#[tokio::test]
async fn independently_reproduced_improvement_selects_the_best_candidate() {
    let registry = registry();
    let report = AlgorithmDiscoveryLab::new(&registry)
        .run(
            &challenge(),
            &[
                artifact("small-improvement", "8"),
                artifact("winner", "7"),
                artifact("regression", "11"),
            ],
        )
        .await
        .unwrap();
    assert_eq!(report.baseline_value, 10.0);
    assert_eq!(report.winner_candidate_id.as_deref(), Some("winner"));
    assert_eq!(report.winner_improvement, Some(3.0));
    assert_eq!(report.verification_records.len(), 8);
    assert!(report.winner().unwrap().outcome.as_ref().unwrap().agreed);
}

#[tokio::test]
async fn hard_constraint_failure_cannot_win_despite_a_better_metric() {
    let registry = registry();
    let report = AlgorithmDiscoveryLab::new(&registry)
        .run(&challenge(), &[artifact("incorrect", "bad:1")])
        .await
        .unwrap();
    assert!(report.winner_candidate_id.is_none());
    assert!(!report.trials[0].eligible);
    assert!(
        report.trials[0]
            .rejection_reasons
            .iter()
            .any(|reason| reason.contains("hard constraints"))
    );
}

#[tokio::test]
async fn reproduction_drift_is_rejected() {
    let registry = registry();
    let report = AlgorithmDiscoveryLab::new(&registry)
        .run(&challenge(), &[artifact("unstable", "drift:5")])
        .await
        .unwrap();
    assert!(report.winner_candidate_id.is_none());
    assert!(!report.trials[0].outcome.as_ref().unwrap().agreed);
}

#[test]
fn challenge_rejects_non_reproducible_or_uncommitted_protocols() {
    let mut invalid = challenge();
    invalid.evaluation.independent_reproduction_required = false;
    assert!(invalid.validate().is_err());

    invalid = challenge();
    invalid.hidden_test_commitment_sha256 = "not-a-digest".into();
    assert!(invalid.validate().is_err());
}

#[tokio::test]
async fn duplicate_content_is_rejected_before_evaluation() {
    let registry = registry();
    let error = AlgorithmDiscoveryLab::new(&registry)
        .run(&challenge(), &[artifact("copy-of-baseline", "10")])
        .await
        .unwrap_err();
    assert!(error.to_string().contains("duplicates existing content"));
}

#[tokio::test]
async fn challenge_commitment_must_match_both_evaluators() {
    struct UnboundEvaluator;

    #[async_trait]
    impl ObjectiveEvaluator for UnboundEvaluator {
        fn id(&self) -> &str {
            "numeric-reproduction"
        }

        fn version(&self) -> &str {
            "1"
        }

        async fn evaluate(
            &self,
            _spec: &EvaluationSpec,
            _candidate: &CandidateArtifact,
        ) -> Result<ObjectiveEvaluation, EvaluationError> {
            panic!("an unbound evaluator must be rejected before evaluation")
        }
    }

    let mut registry = EvaluatorRegistry::default();
    registry
        .register(Arc::new(NumericEvaluator {
            id: "numeric",
            reproduction: false,
            sequence: AtomicU64::new(0),
        }))
        .unwrap();
    registry.register(Arc::new(UnboundEvaluator)).unwrap();
    let error = AlgorithmDiscoveryLab::new(&registry)
        .run(&challenge(), &[artifact("candidate", "9")])
        .await
        .unwrap_err();
    assert!(error.to_string().contains("does not expose"));
}
