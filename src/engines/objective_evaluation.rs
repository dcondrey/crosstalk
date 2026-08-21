//! Objective evaluator contracts for verifiable discovery.
//!
//! Model judgments can prioritize candidates, but only evaluator outputs enter
//! this layer. Results commit to the specification, candidate, raw output, tool
//! identity, measurements, constraints, and environment.

use crate::engines::sandbox::{I64TestCase, SandboxManager};
use crate::types::investigation::{
    Measurement, VerificationKind, VerificationRecord, VerificationStatus,
};
use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::collections::{BTreeMap, BTreeSet};
use std::sync::Arc;

const MAX_CANDIDATE_BYTES: usize = 32 * 1024 * 1024;
const MAX_DIAGNOSTIC_BYTES: usize = 64 * 1024;
const MAX_METADATA_ENTRIES: usize = 128;
const MAX_METADATA_TEXT_BYTES: usize = 16 * 1024;

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub enum MetricDirection {
    Minimize,
    Maximize,
    Target { value: f64, tolerance: f64 },
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct MetricSpec {
    pub name: String,
    pub unit: String,
    pub direction: MetricDirection,
    /// Maximum relative difference allowed between primary and independent
    /// reproduction. Zero requires an exact numeric match.
    pub reproduction_tolerance: f64,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct EvaluationSpec {
    pub id: String,
    pub version: String,
    pub description: String,
    pub metrics: Vec<MetricSpec>,
    pub hard_constraints: Vec<String>,
    pub timeout_secs: u64,
    pub deterministic: bool,
    pub independent_reproduction_required: bool,
    /// A separately registered evaluator used for the reproduction run. It is
    /// mandatory, and must differ from the primary evaluator, when independent
    /// reproduction is required.
    #[serde(default)]
    pub reproduction_evaluator_id: Option<String>,
}

impl EvaluationSpec {
    pub fn validate(&self) -> Result<(), EvaluationError> {
        if self.id.trim().is_empty()
            || self.version.trim().is_empty()
            || self.description.trim().is_empty()
        {
            return Err(EvaluationError::InvalidSpec(
                "id, version, and description must not be empty".into(),
            ));
        }
        if self.id.len() > 1_024
            || self.version.len() > 1_024
            || self.description.len() > MAX_DIAGNOSTIC_BYTES
            || self.metrics.len() > MAX_METADATA_ENTRIES
            || self.hard_constraints.len() > MAX_METADATA_ENTRIES
        {
            return Err(EvaluationError::InvalidSpec(
                "evaluation specification exceeds field or collection limits".into(),
            ));
        }
        if self.timeout_secs == 0 {
            return Err(EvaluationError::InvalidSpec(
                "timeout must be greater than zero".into(),
            ));
        }
        let mut names = BTreeSet::new();
        for metric in &self.metrics {
            if metric.name.trim().is_empty()
                || metric.unit.trim().is_empty()
                || metric.name.len() > 1_024
                || metric.unit.len() > 1_024
            {
                return Err(EvaluationError::InvalidSpec(
                    "metric name and unit must not be empty".into(),
                ));
            }
            if !names.insert(metric.name.as_str()) {
                return Err(EvaluationError::InvalidSpec(format!(
                    "duplicate metric: {}",
                    metric.name
                )));
            }
            if !metric.reproduction_tolerance.is_finite() || metric.reproduction_tolerance < 0.0 {
                return Err(EvaluationError::InvalidSpec(format!(
                    "invalid reproduction tolerance for {}",
                    metric.name
                )));
            }
            if let MetricDirection::Target { value, tolerance } = metric.direction
                && (!value.is_finite() || !tolerance.is_finite() || tolerance < 0.0)
            {
                return Err(EvaluationError::InvalidSpec(format!(
                    "invalid target for {}",
                    metric.name
                )));
            }
        }
        if self
            .hard_constraints
            .iter()
            .any(|constraint| constraint.trim().is_empty() || constraint.len() > 4_096)
        {
            return Err(EvaluationError::InvalidSpec(
                "hard constraints must not be empty".into(),
            ));
        }
        let mut constraints = BTreeSet::new();
        for constraint in &self.hard_constraints {
            if !constraints.insert(constraint.as_str()) {
                return Err(EvaluationError::InvalidSpec(format!(
                    "duplicate hard constraint: {constraint}"
                )));
            }
        }
        if self.independent_reproduction_required
            && self
                .reproduction_evaluator_id
                .as_ref()
                .is_none_or(|id| id.trim().is_empty())
        {
            return Err(EvaluationError::InvalidSpec(
                "independent reproduction requires a reproduction evaluator id".into(),
            ));
        }
        if !self.independent_reproduction_required && self.reproduction_evaluator_id.is_some() {
            return Err(EvaluationError::InvalidSpec(
                "a reproduction evaluator requires independent reproduction".into(),
            ));
        }
        Ok(())
    }

    pub fn sha256(&self) -> Result<String, EvaluationError> {
        self.validate()?;
        let bytes = serde_json::to_vec(self)
            .map_err(|error| EvaluationError::Serialization(error.to_string()))?;
        Ok(sha256(&bytes))
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CandidateArtifact {
    pub id: String,
    pub media_type: String,
    pub content: Vec<u8>,
    #[serde(default)]
    pub metadata: BTreeMap<String, String>,
}

impl CandidateArtifact {
    pub fn validate(&self) -> Result<(), EvaluationError> {
        if self.id.trim().is_empty()
            || self.media_type.trim().is_empty()
            || self.id.len() > 4_096
            || self.media_type.len() > 1_024
        {
            return Err(EvaluationError::InvalidCandidate(
                "candidate id and media type must not be empty".into(),
            ));
        }
        if self.content.is_empty() {
            return Err(EvaluationError::InvalidCandidate(
                "candidate content must not be empty".into(),
            ));
        }
        if self.content.len() > MAX_CANDIDATE_BYTES {
            return Err(EvaluationError::InvalidCandidate(format!(
                "candidate exceeds {MAX_CANDIDATE_BYTES} bytes"
            )));
        }
        if self.metadata.len() > MAX_METADATA_ENTRIES
            || self.metadata.iter().any(|(key, value)| {
                key.trim().is_empty()
                    || key.len() > MAX_METADATA_TEXT_BYTES
                    || value.len() > MAX_METADATA_TEXT_BYTES
            })
        {
            return Err(EvaluationError::InvalidCandidate(
                "candidate metadata exceeds count or size limits".into(),
            ));
        }
        Ok(())
    }

    #[must_use]
    pub fn sha256(&self) -> String {
        sha256(&self.content)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ConstraintResult {
    pub name: String,
    pub passed: bool,
    pub diagnostics: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ObjectiveEvaluation {
    pub id: String,
    pub evaluator_id: String,
    pub evaluator_version: String,
    pub specification_sha256: String,
    pub candidate_id: String,
    pub candidate_sha256: String,
    pub status: VerificationStatus,
    pub measurements: Vec<Measurement>,
    pub constraints: Vec<ConstraintResult>,
    pub diagnostics: String,
    pub raw_output_sha256: String,
    pub started_at: u64,
    pub completed_at: u64,
    #[serde(default)]
    pub environment: BTreeMap<String, String>,
}

impl ObjectiveEvaluation {
    #[must_use]
    pub fn is_verified(&self) -> bool {
        self.status == VerificationStatus::Verified
            && self.constraints.iter().all(|constraint| constraint.passed)
    }

    pub fn validate_against(&self, spec: &EvaluationSpec) -> Result<(), EvaluationError> {
        spec.validate()?;
        if self.id.trim().is_empty()
            || self.evaluator_id.trim().is_empty()
            || self.evaluator_version.trim().is_empty()
            || self.candidate_id.trim().is_empty()
        {
            return Err(EvaluationError::MalformedResult(
                "evaluation and candidate identities must not be empty".into(),
            ));
        }
        if self.specification_sha256 != spec.sha256()? {
            return Err(EvaluationError::MalformedResult(
                "evaluation specification digest mismatch".into(),
            ));
        }
        if self.completed_at < self.started_at {
            return Err(EvaluationError::MalformedResult(
                "evaluation completion precedes its start".into(),
            ));
        }
        if self.diagnostics.len() > MAX_DIAGNOSTIC_BYTES {
            return Err(EvaluationError::MalformedResult(
                "evaluation diagnostics exceed the size limit".into(),
            ));
        }
        if self.environment.len() > MAX_METADATA_ENTRIES
            || self.environment.iter().any(|(key, value)| {
                key.trim().is_empty()
                    || key.len() > MAX_METADATA_TEXT_BYTES
                    || value.len() > MAX_METADATA_TEXT_BYTES
            })
        {
            return Err(EvaluationError::MalformedResult(
                "evaluation environment exceeds count or size limits".into(),
            ));
        }
        let declared_metrics = spec
            .metrics
            .iter()
            .map(|metric| metric.name.as_str())
            .collect::<BTreeSet<_>>();
        let mut measurements = BTreeMap::new();
        for measurement in &self.measurements {
            if !declared_metrics.contains(measurement.name.as_str()) {
                return Err(EvaluationError::MalformedResult(format!(
                    "undeclared measurement: {}",
                    measurement.name
                )));
            }
            if measurements
                .insert(measurement.name.as_str(), measurement)
                .is_some()
            {
                return Err(EvaluationError::MalformedResult(format!(
                    "duplicate measurement: {}",
                    measurement.name
                )));
            }
            if measurement.name.trim().is_empty()
                || measurement.unit.trim().is_empty()
                || !measurement.value.is_finite()
                || measurement
                    .uncertainty
                    .is_some_and(|value| !value.is_finite() || value < 0.0)
                || measurement.sample_size == Some(0)
            {
                return Err(EvaluationError::MalformedResult(format!(
                    "invalid measurement: {}",
                    measurement.name
                )));
            }
        }
        for metric in &spec.metrics {
            let measurement = measurements.get(metric.name.as_str()).ok_or_else(|| {
                EvaluationError::MalformedResult(format!(
                    "missing required measurement: {}",
                    metric.name
                ))
            })?;
            if measurement.unit != metric.unit {
                return Err(EvaluationError::MalformedResult(format!(
                    "invalid measurement for {}",
                    metric.name
                )));
            }
            if let MetricDirection::Target { value, tolerance } = metric.direction
                && (measurement.value - value).abs() > tolerance
                && self.status == VerificationStatus::Verified
            {
                return Err(EvaluationError::MalformedResult(format!(
                    "verified result misses target for {}",
                    metric.name
                )));
            }
        }
        let declared_constraints = spec
            .hard_constraints
            .iter()
            .map(String::as_str)
            .collect::<BTreeSet<_>>();
        let mut constraints = BTreeSet::new();
        for constraint in &self.constraints {
            if !declared_constraints.contains(constraint.name.as_str()) {
                return Err(EvaluationError::MalformedResult(format!(
                    "undeclared hard-constraint result: {}",
                    constraint.name
                )));
            }
            if !constraints.insert(constraint.name.as_str()) {
                return Err(EvaluationError::MalformedResult(format!(
                    "duplicate hard-constraint result: {}",
                    constraint.name
                )));
            }
            if constraint.diagnostics.len() > MAX_DIAGNOSTIC_BYTES {
                return Err(EvaluationError::MalformedResult(format!(
                    "hard-constraint diagnostics exceed the size limit: {}",
                    constraint.name
                )));
            }
        }
        for required in &spec.hard_constraints {
            if !constraints.contains(required.as_str()) {
                return Err(EvaluationError::MalformedResult(format!(
                    "missing hard-constraint result: {required}"
                )));
            }
        }
        if self.status == VerificationStatus::Verified
            && self.constraints.iter().any(|constraint| !constraint.passed)
        {
            return Err(EvaluationError::MalformedResult(
                "verified result contains a failed hard constraint".into(),
            ));
        }
        validate_sha256(&self.candidate_sha256, "candidate")?;
        validate_sha256(&self.raw_output_sha256, "raw output")?;
        Ok(())
    }

    pub fn to_verification_record(
        &self,
        kind: VerificationKind,
        subject_id: impl Into<String>,
        reproduction_of: Option<String>,
    ) -> VerificationRecord {
        VerificationRecord {
            id: self.id.clone(),
            kind,
            status: self.status,
            subject_id: subject_id.into(),
            evaluator_id: self.evaluator_id.clone(),
            evaluator_version: self.evaluator_version.clone(),
            specification_sha256: self.specification_sha256.clone(),
            input_sha256: self.candidate_sha256.clone(),
            output_sha256: self.raw_output_sha256.clone(),
            measurements: self.measurements.clone(),
            diagnostics: self.diagnostics.clone(),
            started_at: self.started_at,
            completed_at: self.completed_at,
            reproduction_of,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ReproductionOutcome {
    pub primary: ObjectiveEvaluation,
    pub reproduction: Option<ObjectiveEvaluation>,
    pub agreed: bool,
    pub mismatches: Vec<String>,
}

#[derive(Debug, thiserror::Error)]
pub enum EvaluationError {
    #[error("invalid evaluation specification: {0}")]
    InvalidSpec(String),
    #[error("invalid candidate: {0}")]
    InvalidCandidate(String),
    #[error("unknown evaluator: {0}")]
    UnknownEvaluator(String),
    #[error("duplicate evaluator: {0}")]
    DuplicateEvaluator(String),
    #[error("evaluator failed: {0}")]
    Evaluator(String),
    #[error("malformed evaluator result: {0}")]
    MalformedResult(String),
    #[error("serialization failed: {0}")]
    Serialization(String),
}

#[async_trait]
pub trait ObjectiveEvaluator: Send + Sync {
    fn id(&self) -> &str;
    fn version(&self) -> &str;

    /// Commitment to evaluator-owned held-out material, when the evaluator
    /// uses a private test set. Discovery challenges verify this value before
    /// evaluating a baseline or candidate.
    fn test_commitment_sha256(&self) -> Option<&str> {
        None
    }

    async fn evaluate(
        &self,
        spec: &EvaluationSpec,
        candidate: &CandidateArtifact,
    ) -> Result<ObjectiveEvaluation, EvaluationError>;
}

#[derive(Default)]
pub struct EvaluatorRegistry {
    evaluators: BTreeMap<String, Arc<dyn ObjectiveEvaluator>>,
}

impl EvaluatorRegistry {
    pub fn register(
        &mut self,
        evaluator: Arc<dyn ObjectiveEvaluator>,
    ) -> Result<(), EvaluationError> {
        let id = evaluator.id().trim();
        if id.is_empty() || evaluator.version().trim().is_empty() {
            return Err(EvaluationError::InvalidSpec(
                "evaluator id and version must not be empty".into(),
            ));
        }
        if self.evaluators.contains_key(id) {
            return Err(EvaluationError::DuplicateEvaluator(id.into()));
        }
        self.evaluators.insert(id.into(), evaluator);
        Ok(())
    }

    pub fn test_commitment(&self, evaluator_id: &str) -> Result<Option<String>, EvaluationError> {
        let evaluator = self
            .evaluators
            .get(evaluator_id)
            .ok_or_else(|| EvaluationError::UnknownEvaluator(evaluator_id.into()))?;
        let Some(commitment) = evaluator.test_commitment_sha256() else {
            return Ok(None);
        };
        if commitment.len() != 64 || !commitment.bytes().all(|byte| byte.is_ascii_hexdigit()) {
            return Err(EvaluationError::InvalidSpec(format!(
                "evaluator {evaluator_id} exposed an invalid hidden-test commitment"
            )));
        }
        Ok(Some(commitment.to_ascii_lowercase()))
    }

    pub async fn evaluate(
        &self,
        evaluator_id: &str,
        spec: &EvaluationSpec,
        candidate: &CandidateArtifact,
    ) -> Result<ObjectiveEvaluation, EvaluationError> {
        spec.validate()?;
        candidate.validate()?;
        let evaluator = self
            .evaluators
            .get(evaluator_id)
            .ok_or_else(|| EvaluationError::UnknownEvaluator(evaluator_id.into()))?;
        let result = tokio::time::timeout(
            std::time::Duration::from_secs(spec.timeout_secs),
            evaluator.evaluate(spec, candidate),
        )
        .await
        .map_err(|_| EvaluationError::Evaluator("evaluation timed out".into()))??;
        result.validate_against(spec)?;
        if result.evaluator_id != evaluator.id() || result.evaluator_version != evaluator.version()
        {
            return Err(EvaluationError::MalformedResult(
                "result evaluator identity does not match registry entry".into(),
            ));
        }
        if result.candidate_id != candidate.id || result.candidate_sha256 != candidate.sha256() {
            return Err(EvaluationError::MalformedResult(
                "result candidate identity or digest mismatch".into(),
            ));
        }
        Ok(result)
    }

    pub async fn evaluate_with_reproduction(
        &self,
        evaluator_id: &str,
        spec: &EvaluationSpec,
        candidate: &CandidateArtifact,
    ) -> Result<ReproductionOutcome, EvaluationError> {
        let primary = self.evaluate(evaluator_id, spec, candidate).await?;
        if !spec.independent_reproduction_required {
            return Ok(ReproductionOutcome {
                agreed: primary.is_verified(),
                primary,
                reproduction: None,
                mismatches: vec![],
            });
        }
        let reproduction_evaluator_id =
            spec.reproduction_evaluator_id.as_deref().ok_or_else(|| {
                EvaluationError::InvalidSpec(
                    "independent reproduction requires a reproduction evaluator id".into(),
                )
            })?;
        if reproduction_evaluator_id == evaluator_id {
            return Err(EvaluationError::InvalidSpec(
                "primary and reproduction evaluator identities must differ".into(),
            ));
        }
        let mut reproduction = self
            .evaluate(reproduction_evaluator_id, spec, candidate)
            .await?;
        // A very fast deterministic evaluator can produce the same
        // time-and-output-derived ID twice. Preserve distinct provenance for
        // the independent run even when its committed output is identical.
        if reproduction.id == primary.id {
            reproduction.id = format!("{}:reproduction", reproduction.id);
        }
        let mismatches = reproduction_mismatches(spec, &primary, &reproduction);
        Ok(ReproductionOutcome {
            agreed: primary.is_verified() && reproduction.is_verified() && mismatches.is_empty(),
            primary,
            reproduction: Some(reproduction),
            mismatches,
        })
    }
}

pub const WASM_I64_TEST_SCHEMA: &str = "crosstalk.wasm-i64-tests.v1";

#[derive(Serialize)]
struct I64TestCommitment<'a> {
    schema: &'static str,
    export_name: &'a str,
    cases: &'a [I64TestCase],
}

/// Objective evaluator for algorithm candidates exporting a pure
/// `(i64) -> i64` function. Test vectors remain evaluator-owned; public
/// results expose only aggregate outcomes and their preregistered commitment.
pub struct WasmI64FunctionEvaluator {
    sandbox: Arc<SandboxManager>,
    id: String,
    version: String,
    export_name: String,
    cases: Vec<I64TestCase>,
    commitment_sha256: String,
}

impl WasmI64FunctionEvaluator {
    pub fn new(
        sandbox: Arc<SandboxManager>,
        id: impl Into<String>,
        version: impl Into<String>,
        export_name: impl Into<String>,
        cases: Vec<I64TestCase>,
    ) -> Result<Self, EvaluationError> {
        let id = id.into();
        let version = version.into();
        let export_name = export_name.into();
        if id.trim().is_empty() || version.trim().is_empty() {
            return Err(EvaluationError::InvalidSpec(
                "evaluator id and version must not be empty".into(),
            ));
        }
        let commitment_sha256 = Self::commitment_for(&export_name, &cases)?;
        Ok(Self {
            sandbox,
            id,
            version,
            export_name,
            cases,
            commitment_sha256,
        })
    }

    pub fn commitment_for(
        export_name: &str,
        cases: &[I64TestCase],
    ) -> Result<String, EvaluationError> {
        if export_name.trim().is_empty() || export_name.len() > 256 {
            return Err(EvaluationError::InvalidSpec(
                "WASM export name must contain between 1 and 256 bytes".into(),
            ));
        }
        if cases.is_empty() || cases.len() > 10_000 {
            return Err(EvaluationError::InvalidSpec(
                "hidden test set must contain between 1 and 10000 cases".into(),
            ));
        }
        let canonical = I64TestCommitment {
            schema: WASM_I64_TEST_SCHEMA,
            export_name,
            cases,
        };
        let bytes = serde_json::to_vec(&canonical)
            .map_err(|error| EvaluationError::Serialization(error.to_string()))?;
        Ok(sha256(&bytes))
    }
}

#[async_trait]
impl ObjectiveEvaluator for WasmI64FunctionEvaluator {
    fn id(&self) -> &str {
        &self.id
    }

    fn version(&self) -> &str {
        &self.version
    }

    fn test_commitment_sha256(&self) -> Option<&str> {
        Some(&self.commitment_sha256)
    }

    async fn evaluate(
        &self,
        spec: &EvaluationSpec,
        candidate: &CandidateArtifact,
    ) -> Result<ObjectiveEvaluation, EvaluationError> {
        if candidate.media_type != "application/wasm" {
            return Err(EvaluationError::InvalidCandidate(
                "WASM function evaluation requires application/wasm".into(),
            ));
        }
        for metric in &spec.metrics {
            let expected_unit = match metric.name.as_str() {
                "correct_cases" => "cases",
                "accuracy" => "ratio",
                "fuel_consumed" => "fuel",
                "elapsed_ms" => "ms",
                other => {
                    return Err(EvaluationError::InvalidSpec(format!(
                        "WASM function evaluator does not provide metric: {other}"
                    )));
                }
            };
            if metric.unit != expected_unit {
                return Err(EvaluationError::InvalidSpec(format!(
                    "metric {} requires unit {expected_unit}",
                    metric.name
                )));
            }
        }
        for constraint in &spec.hard_constraints {
            if !matches!(
                constraint.as_str(),
                "all_cases_correct" | "resource_limit_not_hit"
            ) {
                return Err(EvaluationError::InvalidSpec(format!(
                    "WASM function evaluator does not provide constraint: {constraint}"
                )));
            }
        }

        let started_at = now();
        let result = self
            .sandbox
            .evaluate_i64_cases_with_timeout(&candidate.content, &self.export_name, &self.cases)
            .await
            .map_err(|error| EvaluationError::Evaluator(error.to_string()))?;
        let completed_at = now();
        let correct_cases = result.correct_cases();
        let all_cases_correct = result.all_cases_correct(self.cases.len());
        let accuracy = correct_cases as f64 / self.cases.len() as f64;
        let measurements = spec
            .metrics
            .iter()
            .map(|metric| {
                let (value, sample_size) = match metric.name.as_str() {
                    "correct_cases" => (correct_cases as f64, Some(self.cases.len() as u64)),
                    "accuracy" => (accuracy, Some(self.cases.len() as u64)),
                    "fuel_consumed" => (result.fuel_consumed as f64, Some(self.cases.len() as u64)),
                    "elapsed_ms" => (result.elapsed_ms as f64, Some(1)),
                    _ => unreachable!("metrics were validated before execution"),
                };
                Measurement {
                    name: metric.name.clone(),
                    value,
                    unit: metric.unit.clone(),
                    uncertainty: None,
                    sample_size,
                }
            })
            .collect::<Vec<_>>();
        let constraints = spec
            .hard_constraints
            .iter()
            .map(|name| {
                let passed = match name.as_str() {
                    "all_cases_correct" => all_cases_correct,
                    "resource_limit_not_hit" => !result.resource_limit_hit,
                    _ => unreachable!("constraints were validated before execution"),
                };
                ConstraintResult {
                    name: name.clone(),
                    passed,
                    diagnostics: if passed { "passed" } else { "failed" }.into(),
                }
            })
            .collect::<Vec<_>>();
        let raw = serde_json::to_vec(&serde_json::json!({
            "schema": WASM_I64_TEST_SCHEMA,
            "test_set_commitment_sha256": self.commitment_sha256,
            "test_count": self.cases.len(),
            "outcomes": result.outcomes,
            "elapsed_ms": result.elapsed_ms,
            "fuel_consumed": result.fuel_consumed,
            "resource_limit_hit": result.resource_limit_hit,
            "trapped": result.trapped,
        }))
        .map_err(|error| EvaluationError::Serialization(error.to_string()))?;
        let constraints_pass = constraints.iter().all(|constraint| constraint.passed);
        let status = if all_cases_correct && constraints_pass {
            VerificationStatus::Verified
        } else if result.resource_limit_hit {
            VerificationStatus::TimedOut
        } else {
            VerificationStatus::Rejected
        };
        let diagnostics = bounded_diagnostics(&format!(
            "evaluated {} committed hidden cases; correct={correct_cases}; trapped={}; resource_limit_hit={}",
            self.cases.len(),
            result.trapped,
            result.resource_limit_hit
        ));
        let raw_output_sha256 = sha256(&raw);
        let id = evaluation_id(
            self.id(),
            &candidate.id,
            &candidate.sha256(),
            started_at,
            &raw_output_sha256,
        );
        Ok(ObjectiveEvaluation {
            id,
            evaluator_id: self.id().into(),
            evaluator_version: self.version().into(),
            specification_sha256: spec.sha256()?,
            candidate_id: candidate.id.clone(),
            candidate_sha256: candidate.sha256(),
            status,
            measurements,
            constraints,
            diagnostics,
            raw_output_sha256,
            started_at,
            completed_at,
            environment: BTreeMap::from([
                ("runtime".into(), "wasmtime".into()),
                ("function_export".into(), self.export_name.clone()),
                (
                    "test_set_commitment_sha256".into(),
                    self.commitment_sha256.clone(),
                ),
                ("test_count".into(), self.cases.len().to_string()),
            ]),
        })
    }
}

/// Objective evaluator for WASI-compatible modules. A successful default
/// export establishes execution under the configured sandbox and records fuel
/// and elapsed time; it does not establish domain correctness by itself.
pub struct WasmExecutionEvaluator {
    sandbox: Arc<SandboxManager>,
    version: String,
}

impl WasmExecutionEvaluator {
    #[must_use]
    pub fn new(sandbox: Arc<SandboxManager>, version: impl Into<String>) -> Self {
        Self {
            sandbox,
            version: version.into(),
        }
    }
}

#[async_trait]
impl ObjectiveEvaluator for WasmExecutionEvaluator {
    fn id(&self) -> &str {
        "wasm-execution"
    }

    fn version(&self) -> &str {
        &self.version
    }

    async fn evaluate(
        &self,
        spec: &EvaluationSpec,
        candidate: &CandidateArtifact,
    ) -> Result<ObjectiveEvaluation, EvaluationError> {
        if candidate.media_type != "application/wasm" {
            return Err(EvaluationError::InvalidCandidate(
                "wasm-execution requires application/wasm".into(),
            ));
        }
        let started_at = now();
        let result = self
            .sandbox
            .execute_with_timeout(&candidate.content)
            .await
            .map_err(|error| EvaluationError::Evaluator(error.to_string()))?;
        let completed_at = now();
        let mut measurements = Vec::new();
        for metric in &spec.metrics {
            let value = match metric.name.as_str() {
                "elapsed_ms" => result.elapsed_ms as f64,
                "fuel_consumed" => result.fuel_consumed.unwrap_or_default() as f64,
                other => {
                    return Err(EvaluationError::InvalidSpec(format!(
                        "wasm-execution does not provide metric: {other}"
                    )));
                }
            };
            measurements.push(Measurement {
                name: metric.name.clone(),
                value,
                unit: metric.unit.clone(),
                uncertainty: None,
                sample_size: Some(1),
            });
        }
        let constraints = spec
            .hard_constraints
            .iter()
            .map(|name| {
                let passed = match name.as_str() {
                    "exit_code_zero" => result.exit_code == 0,
                    "resource_limit_not_hit" => !result.resource_limit_hit,
                    _ => false,
                };
                ConstraintResult {
                    name: name.clone(),
                    passed,
                    diagnostics: if passed {
                        "passed".into()
                    } else {
                        "failed or unsupported by wasm-execution".into()
                    },
                }
            })
            .collect::<Vec<_>>();
        let raw = serde_json::to_vec(&serde_json::json!({
            "exit_code": result.exit_code,
            "stdout": result.stdout,
            "stderr": result.stderr,
            "fuel_consumed": result.fuel_consumed,
            "elapsed_ms": result.elapsed_ms,
            "resource_limit_hit": result.resource_limit_hit,
        }))
        .map_err(|error| EvaluationError::Serialization(error.to_string()))?;
        let constraints_pass = constraints.iter().all(|constraint| constraint.passed);
        let status = if result.exit_code == 0 && constraints_pass {
            VerificationStatus::Verified
        } else if result.resource_limit_hit {
            VerificationStatus::TimedOut
        } else {
            VerificationStatus::Rejected
        };
        let diagnostics = bounded_diagnostics(&format!(
            "exit_code={}\nstdout:\n{}\nstderr:\n{}",
            result.exit_code, result.stdout, result.stderr
        ));
        let raw_output_sha256 = sha256(&raw);
        let id = evaluation_id(
            self.id(),
            &candidate.id,
            &candidate.sha256(),
            started_at,
            &raw_output_sha256,
        );
        Ok(ObjectiveEvaluation {
            id,
            evaluator_id: self.id().into(),
            evaluator_version: self.version().into(),
            specification_sha256: spec.sha256()?,
            candidate_id: candidate.id.clone(),
            candidate_sha256: candidate.sha256(),
            status,
            measurements,
            constraints,
            diagnostics,
            raw_output_sha256,
            started_at,
            completed_at,
            environment: BTreeMap::from([("runtime".into(), "wasmtime-wasi-preview1".into())]),
        })
    }
}

fn reproduction_mismatches(
    spec: &EvaluationSpec,
    primary: &ObjectiveEvaluation,
    reproduction: &ObjectiveEvaluation,
) -> Vec<String> {
    let first = primary
        .measurements
        .iter()
        .map(|measurement| (measurement.name.as_str(), measurement.value))
        .collect::<BTreeMap<_, _>>();
    let second = reproduction
        .measurements
        .iter()
        .map(|measurement| (measurement.name.as_str(), measurement.value))
        .collect::<BTreeMap<_, _>>();
    let mut mismatches = Vec::new();
    for metric in &spec.metrics {
        let (Some(a), Some(b)) = (
            first.get(metric.name.as_str()),
            second.get(metric.name.as_str()),
        ) else {
            mismatches.push(format!("missing reproduction metric: {}", metric.name));
            continue;
        };
        let scale = a.abs().max(b.abs()).max(1.0);
        let relative = (a - b).abs() / scale;
        if relative > metric.reproduction_tolerance {
            mismatches.push(format!(
                "{} differs: primary={} reproduction={} relative_difference={relative}",
                metric.name, a, b
            ));
        }
    }
    mismatches
}

fn evaluation_id(
    evaluator_id: &str,
    candidate_id: &str,
    candidate_sha256: &str,
    started_at: u64,
    output_sha256: &str,
) -> String {
    let input = format!(
        "{evaluator_id}\0{candidate_id}\0{candidate_sha256}\0{started_at}\0{output_sha256}"
    );
    format!("evaluation:{}", &sha256(input.as_bytes())[..24])
}

fn bounded_diagnostics(value: &str) -> String {
    let end = value.floor_char_boundary(value.len().min(MAX_DIAGNOSTIC_BYTES));
    value[..end].to_string()
}

fn validate_sha256(value: &str, label: &str) -> Result<(), EvaluationError> {
    if value.len() != 64 || !value.bytes().all(|byte| byte.is_ascii_hexdigit()) {
        return Err(EvaluationError::MalformedResult(format!(
            "{label} digest is not SHA-256"
        )));
    }
    Ok(())
}

fn sha256(bytes: &[u8]) -> String {
    format!("{:x}", Sha256::digest(bytes))
}

fn now() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}
