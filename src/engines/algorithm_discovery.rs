//! Preregistered, fail-closed algorithm discovery tournaments.
//!
//! The lab deliberately separates proposal generation from judgment. Models may
//! propose arbitrary candidates, but only a registered objective evaluator can
//! select a winner, and every accepted result must survive an independent rerun.

use crate::engines::objective_evaluation::{
    CandidateArtifact, EvaluationError, EvaluationSpec, EvaluatorRegistry, MetricDirection,
    ObjectiveEvaluation, ReproductionOutcome,
};
use crate::types::investigation::{VerificationKind, VerificationRecord};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::collections::BTreeSet;

pub const ALGORITHM_CHALLENGE_SCHEMA: &str = "crosstalk.algorithm-challenge.v1";
pub const ALGORITHM_REPORT_SCHEMA: &str = "crosstalk.algorithm-report.v1";

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct AlgorithmChallenge {
    pub schema: String,
    pub id: String,
    pub title: String,
    pub evaluator_id: String,
    pub evaluation: EvaluationSpec,
    pub primary_metric: String,
    /// Required improvement in the natural direction of the primary metric.
    pub minimum_improvement: f64,
    /// Commitment to the exact private/held-out test material. The evaluator
    /// owns the corresponding material; candidates see only this digest.
    pub hidden_test_commitment_sha256: String,
    pub baseline: CandidateArtifact,
    pub max_candidates: usize,
}

impl AlgorithmChallenge {
    pub fn validate(&self) -> Result<(), AlgorithmDiscoveryError> {
        if self.schema != ALGORITHM_CHALLENGE_SCHEMA {
            return Err(AlgorithmDiscoveryError::InvalidChallenge(format!(
                "unsupported schema: {}",
                self.schema
            )));
        }
        if self.id.trim().is_empty()
            || self.title.trim().is_empty()
            || self.evaluator_id.trim().is_empty()
        {
            return Err(AlgorithmDiscoveryError::InvalidChallenge(
                "challenge id, title, and evaluator id are required".into(),
            ));
        }
        self.evaluation.validate()?;
        self.baseline.validate()?;
        if !self.evaluation.independent_reproduction_required {
            return Err(AlgorithmDiscoveryError::InvalidChallenge(
                "algorithm discovery requires independent reproduction".into(),
            ));
        }
        if self
            .evaluation
            .reproduction_evaluator_id
            .as_deref()
            .is_some_and(|id| id == self.evaluator_id.as_str())
        {
            return Err(AlgorithmDiscoveryError::InvalidChallenge(
                "primary and reproduction evaluator identities must differ".into(),
            ));
        }
        if self.evaluation.hard_constraints.is_empty() {
            return Err(AlgorithmDiscoveryError::InvalidChallenge(
                "at least one correctness or safety hard constraint is required".into(),
            ));
        }
        if !self.minimum_improvement.is_finite() || self.minimum_improvement <= 0.0 {
            return Err(AlgorithmDiscoveryError::InvalidChallenge(
                "minimum improvement must be finite and positive".into(),
            ));
        }
        if self.max_candidates == 0 || self.max_candidates > 1_000 {
            return Err(AlgorithmDiscoveryError::InvalidChallenge(
                "max candidates must be between 1 and 1000".into(),
            ));
        }
        validate_sha256(
            &self.hidden_test_commitment_sha256,
            "hidden-test commitment",
        )?;
        if !self
            .evaluation
            .metrics
            .iter()
            .any(|metric| metric.name == self.primary_metric)
        {
            return Err(AlgorithmDiscoveryError::InvalidChallenge(format!(
                "primary metric is not declared: {}",
                self.primary_metric
            )));
        }
        Ok(())
    }

    pub fn sha256(&self) -> Result<String, AlgorithmDiscoveryError> {
        self.validate()?;
        let bytes = serde_json::to_vec(self).map_err(|error| {
            AlgorithmDiscoveryError::InvalidChallenge(format!(
                "could not serialize challenge: {error}"
            ))
        })?;
        Ok(sha256(&bytes))
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CandidateTrial {
    pub candidate_id: String,
    pub candidate_sha256: String,
    pub outcome: Option<ReproductionOutcome>,
    pub primary_value: Option<f64>,
    pub improvement: Option<f64>,
    pub eligible: bool,
    pub rejection_reasons: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct AlgorithmDiscoveryReport {
    pub schema: String,
    pub challenge_id: String,
    pub challenge_sha256: String,
    pub hidden_test_commitment_sha256: String,
    pub baseline: ReproductionOutcome,
    pub baseline_value: f64,
    pub trials: Vec<CandidateTrial>,
    pub winner_candidate_id: Option<String>,
    pub winner_improvement: Option<f64>,
    pub verification_records: Vec<VerificationRecord>,
}

impl AlgorithmDiscoveryReport {
    #[must_use]
    pub fn winner(&self) -> Option<&CandidateTrial> {
        let id = self.winner_candidate_id.as_deref()?;
        self.trials.iter().find(|trial| trial.candidate_id == id)
    }
}

#[derive(Debug, thiserror::Error)]
pub enum AlgorithmDiscoveryError {
    #[error("invalid algorithm challenge: {0}")]
    InvalidChallenge(String),
    #[error("objective evaluation failed: {0}")]
    Evaluation(#[from] EvaluationError),
    #[error("baseline was not independently verified: {0}")]
    InvalidBaseline(String),
}

pub struct AlgorithmDiscoveryLab<'a> {
    evaluators: &'a EvaluatorRegistry,
}

impl<'a> AlgorithmDiscoveryLab<'a> {
    #[must_use]
    pub fn new(evaluators: &'a EvaluatorRegistry) -> Self {
        Self { evaluators }
    }

    pub async fn run(
        &self,
        challenge: &AlgorithmChallenge,
        candidates: &[CandidateArtifact],
    ) -> Result<AlgorithmDiscoveryReport, AlgorithmDiscoveryError> {
        challenge.validate()?;
        validate_candidates(challenge, candidates)?;
        self.verify_hidden_test_bindings(challenge)?;

        let baseline = self
            .evaluators
            .evaluate_with_reproduction(
                &challenge.evaluator_id,
                &challenge.evaluation,
                &challenge.baseline,
            )
            .await?;
        if !baseline.agreed || baseline.reproduction.is_none() {
            return Err(AlgorithmDiscoveryError::InvalidBaseline(
                describe_disagreement(&baseline),
            ));
        }
        let baseline_value =
            measurement(&baseline.primary, &challenge.primary_metric).ok_or_else(|| {
                AlgorithmDiscoveryError::InvalidBaseline(format!(
                    "missing primary metric: {}",
                    challenge.primary_metric
                ))
            })?;
        let direction = challenge
            .evaluation
            .metrics
            .iter()
            .find(|metric| metric.name == challenge.primary_metric)
            .expect("challenge validation guarantees the primary metric")
            .direction;

        let mut verification_records = outcome_records(&baseline, &challenge.baseline.id);
        let mut trials = Vec::with_capacity(candidates.len());
        for candidate in candidates {
            let result = self
                .evaluators
                .evaluate_with_reproduction(
                    &challenge.evaluator_id,
                    &challenge.evaluation,
                    candidate,
                )
                .await;
            let trial = match result {
                Ok(outcome) => {
                    verification_records.extend(outcome_records(&outcome, &candidate.id));
                    score_trial(challenge, candidate, outcome, baseline_value, direction)
                }
                Err(error) => CandidateTrial {
                    candidate_id: candidate.id.clone(),
                    candidate_sha256: candidate.sha256(),
                    outcome: None,
                    primary_value: None,
                    improvement: None,
                    eligible: false,
                    rejection_reasons: vec![format!("evaluation failed: {error}")],
                },
            };
            trials.push(trial);
        }

        let winner = trials
            .iter()
            .filter(|trial| trial.eligible)
            .max_by(|left, right| {
                left.improvement
                    .partial_cmp(&right.improvement)
                    .unwrap_or(std::cmp::Ordering::Equal)
                    .then_with(|| right.candidate_id.cmp(&left.candidate_id))
            });
        let winner_candidate_id = winner.map(|trial| trial.candidate_id.clone());
        let winner_improvement = winner.and_then(|trial| trial.improvement);
        Ok(AlgorithmDiscoveryReport {
            schema: ALGORITHM_REPORT_SCHEMA.into(),
            challenge_id: challenge.id.clone(),
            challenge_sha256: challenge.sha256()?,
            hidden_test_commitment_sha256: challenge.hidden_test_commitment_sha256.clone(),
            baseline,
            baseline_value,
            trials,
            winner_candidate_id,
            winner_improvement,
            verification_records,
        })
    }

    fn verify_hidden_test_bindings(
        &self,
        challenge: &AlgorithmChallenge,
    ) -> Result<(), AlgorithmDiscoveryError> {
        let expected = challenge.hidden_test_commitment_sha256.to_ascii_lowercase();
        let reproduction_id = challenge
            .evaluation
            .reproduction_evaluator_id
            .as_deref()
            .expect("challenge validation requires a reproduction evaluator");
        for evaluator_id in [challenge.evaluator_id.as_str(), reproduction_id] {
            let actual = self
                .evaluators
                .test_commitment(evaluator_id)?
                .ok_or_else(|| {
                    AlgorithmDiscoveryError::InvalidChallenge(format!(
                        "evaluator {evaluator_id} does not expose a hidden-test commitment"
                    ))
                })?;
            if actual != expected {
                return Err(AlgorithmDiscoveryError::InvalidChallenge(format!(
                    "hidden-test commitment does not match evaluator {evaluator_id}"
                )));
            }
        }
        Ok(())
    }
}

fn validate_candidates(
    challenge: &AlgorithmChallenge,
    candidates: &[CandidateArtifact],
) -> Result<(), AlgorithmDiscoveryError> {
    if candidates.is_empty() {
        return Err(AlgorithmDiscoveryError::InvalidChallenge(
            "at least one candidate is required".into(),
        ));
    }
    if candidates.len() > challenge.max_candidates {
        return Err(AlgorithmDiscoveryError::InvalidChallenge(format!(
            "candidate count {} exceeds the preregistered maximum {}",
            candidates.len(),
            challenge.max_candidates
        )));
    }
    let mut ids = BTreeSet::new();
    let mut digests = BTreeSet::new();
    digests.insert(challenge.baseline.sha256());
    for candidate in candidates {
        candidate.validate()?;
        if candidate.id == challenge.baseline.id || !ids.insert(candidate.id.as_str()) {
            return Err(AlgorithmDiscoveryError::InvalidChallenge(format!(
                "duplicate candidate id: {}",
                candidate.id
            )));
        }
        if !digests.insert(candidate.sha256()) {
            return Err(AlgorithmDiscoveryError::InvalidChallenge(format!(
                "candidate {} duplicates existing content",
                candidate.id
            )));
        }
    }
    Ok(())
}

fn score_trial(
    challenge: &AlgorithmChallenge,
    candidate: &CandidateArtifact,
    outcome: ReproductionOutcome,
    baseline_value: f64,
    direction: MetricDirection,
) -> CandidateTrial {
    let primary_value = measurement(&outcome.primary, &challenge.primary_metric);
    let improvement = primary_value.map(|value| improvement(direction, baseline_value, value));
    let hard_constraints_pass = outcome
        .primary
        .constraints
        .iter()
        .all(|constraint| constraint.passed)
        && outcome.reproduction.as_ref().is_some_and(|reproduction| {
            reproduction
                .constraints
                .iter()
                .all(|constraint| constraint.passed)
        });
    let mut rejection_reasons = Vec::new();
    if !outcome.agreed {
        rejection_reasons.push(describe_disagreement(&outcome));
    }
    if !hard_constraints_pass {
        rejection_reasons.push("one or more hard constraints failed".into());
    }
    match improvement {
        Some(value) if value < challenge.minimum_improvement => rejection_reasons.push(format!(
            "primary improvement {value} is below the preregistered minimum {}",
            challenge.minimum_improvement
        )),
        None => rejection_reasons.push(format!(
            "missing primary metric: {}",
            challenge.primary_metric
        )),
        _ => {}
    }
    CandidateTrial {
        candidate_id: candidate.id.clone(),
        candidate_sha256: candidate.sha256(),
        primary_value,
        improvement,
        eligible: rejection_reasons.is_empty(),
        rejection_reasons,
        outcome: Some(outcome),
    }
}

fn outcome_records(outcome: &ReproductionOutcome, subject_id: &str) -> Vec<VerificationRecord> {
    let mut records =
        vec![
            outcome
                .primary
                .to_verification_record(VerificationKind::Benchmark, subject_id, None),
        ];
    if let Some(reproduction) = &outcome.reproduction {
        records.push(reproduction.to_verification_record(
            VerificationKind::Reproduction,
            subject_id,
            Some(outcome.primary.id.clone()),
        ));
    }
    records
}

fn measurement(evaluation: &ObjectiveEvaluation, name: &str) -> Option<f64> {
    evaluation
        .measurements
        .iter()
        .find(|measurement| measurement.name == name)
        .map(|measurement| measurement.value)
}

fn improvement(direction: MetricDirection, baseline: f64, candidate: f64) -> f64 {
    match direction {
        MetricDirection::Minimize => baseline - candidate,
        MetricDirection::Maximize => candidate - baseline,
        MetricDirection::Target { value, .. } => {
            (baseline - value).abs() - (candidate - value).abs()
        }
    }
}

fn describe_disagreement(outcome: &ReproductionOutcome) -> String {
    if outcome.mismatches.is_empty() {
        "primary or reproduction was not verified".into()
    } else {
        outcome.mismatches.join("; ")
    }
}

fn validate_sha256(value: &str, label: &str) -> Result<(), AlgorithmDiscoveryError> {
    if value.len() != 64 || !value.bytes().all(|byte| byte.is_ascii_hexdigit()) {
        return Err(AlgorithmDiscoveryError::InvalidChallenge(format!(
            "{label} is not a SHA-256 digest"
        )));
    }
    Ok(())
}

fn sha256(bytes: &[u8]) -> String {
    format!("{:x}", Sha256::digest(bytes))
}
