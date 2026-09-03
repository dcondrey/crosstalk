//! Typed, auditable state for a discovery investigation.
//!
//! The conversation transcript records what agents said.  `Investigation`
//! records what the system can actually support: hypotheses, evidence,
//! measurements, verification results, and the links between them.

use crate::types::epistemics::{ClaimKind, ClaimLedger, ClaimStatus, EvidenceRef};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};

pub const INVESTIGATION_SCHEMA: &str = "crosstalk.investigation.v1";

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProblemSpec {
    pub statement: String,
    pub objectives: Vec<String>,
    pub constraints: Vec<String>,
    pub success_criteria: Vec<String>,
    pub forbidden_failures: Vec<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum EvidenceKind {
    SourceRecord,
    SourceSnapshot,
    Dataset,
    Code,
    Proof,
    ExperimentLog,
    Observation,
    Benchmark,
    ExpertReview,
    VerificationOutput,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct EvidenceArtifact {
    pub id: String,
    pub kind: EvidenceKind,
    pub title: String,
    pub content_sha256: String,
    pub media_type: String,
    pub source_uri: Option<String>,
    pub locator: Option<String>,
    pub artifact_name: Option<String>,
    pub verification_id: Option<String>,
    pub captured_at: u64,
    /// True only when this item was produced by a genuinely independent
    /// source or reproduction path. It is metadata, not an inferred property.
    pub independent: bool,
    #[serde(default)]
    pub metadata: BTreeMap<String, String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum HypothesisStatus {
    Proposed,
    UnderTest,
    Supported,
    Contested,
    Falsified,
    Validated,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Hypothesis {
    pub id: String,
    pub statement: String,
    pub parent_ids: Vec<String>,
    pub claim_ids: Vec<String>,
    pub evidence_ids: Vec<String>,
    pub verification_ids: Vec<String>,
    pub status: HypothesisStatus,
    pub created_at: u64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum VerificationKind {
    FormalProof,
    ExecutableTest,
    Benchmark,
    Simulation,
    SourceAudit,
    Reproduction,
    HumanReview,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum VerificationStatus {
    Verified,
    Rejected,
    Inconclusive,
    Unavailable,
    TimedOut,
    PolicyViolation,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Measurement {
    pub name: String,
    pub value: f64,
    pub unit: String,
    pub uncertainty: Option<f64>,
    pub sample_size: Option<u64>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct VerificationRecord {
    pub id: String,
    pub kind: VerificationKind,
    pub status: VerificationStatus,
    /// Claim, hypothesis, or artifact identifier evaluated by this record.
    pub subject_id: String,
    pub evaluator_id: String,
    pub evaluator_version: String,
    pub specification_sha256: String,
    pub input_sha256: String,
    pub output_sha256: String,
    pub measurements: Vec<Measurement>,
    pub diagnostics: String,
    pub started_at: u64,
    pub completed_at: u64,
    /// The earlier verification independently reproduced by this run.
    pub reproduction_of: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum AuditSeverity {
    Warning,
    Error,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct EvidenceAuditIssue {
    pub severity: AuditSeverity,
    pub code: String,
    pub subject_id: String,
    pub message: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ChainOfEvidenceAudit {
    pub passed: bool,
    pub claim_count: usize,
    pub evidence_linked_claims: usize,
    pub verified_claims: usize,
    pub verification_coverage: f64,
    pub issues: Vec<EvidenceAuditIssue>,
}

/// A stricter, separate decision from the integrity audit. `audit.passed`
/// means the evidence graph is internally consistent; it does not mean the
/// result has enough verified evidence to be reported as established.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct ScientificReleaseAssessment {
    pub eligible: bool,
    pub integrity_passed: bool,
    pub substantive_claims: usize,
    pub evidence_linked_claims: usize,
    pub objectively_verified_claims: usize,
    pub verification_coverage: f64,
    pub blocking_reasons: Vec<String>,
}

impl ScientificReleaseAssessment {
    /// User-facing boundary for model prose that has not passed the scientific
    /// release gate.  Integrity alone does not make a synthesis evidentially
    /// established.
    #[must_use]
    pub fn unverified_warning(&self) -> Option<String> {
        if self.eligible {
            return None;
        }
        let reasons = if self.blocking_reasons.is_empty() {
            "the scientific release requirements were not met".to_string()
        } else {
            self.blocking_reasons.join("; ")
        };
        Some(format!(
            "Unverified model synthesis—not an established conclusion. Blocking reasons: {reasons}. Rejected candidates and finite measurements do not by themselves establish universal claims."
        ))
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Investigation {
    pub schema: String,
    pub id: String,
    pub problem: ProblemSpec,
    pub hypotheses: BTreeMap<String, Hypothesis>,
    pub evidence: BTreeMap<String, EvidenceArtifact>,
    pub verifications: BTreeMap<String, VerificationRecord>,
    pub created_at: u64,
    pub updated_at: u64,
}

impl Default for Investigation {
    fn default() -> Self {
        Self::new("", "")
    }
}

impl Investigation {
    #[must_use]
    pub fn new(id: impl Into<String>, statement: impl Into<String>) -> Self {
        let now = now();
        Self {
            schema: INVESTIGATION_SCHEMA.into(),
            id: id.into(),
            problem: ProblemSpec {
                statement: statement.into(),
                ..ProblemSpec::default()
            },
            hypotheses: BTreeMap::new(),
            evidence: BTreeMap::new(),
            verifications: BTreeMap::new(),
            created_at: now,
            updated_at: now,
        }
    }

    pub fn register_evidence(&mut self, evidence: EvidenceArtifact) -> Result<(), String> {
        validate_evidence(&evidence)?;
        if self.evidence.contains_key(&evidence.id) {
            return Err(format!("duplicate evidence id: {}", evidence.id));
        }
        self.evidence.insert(evidence.id.clone(), evidence);
        self.touch();
        Ok(())
    }

    pub fn register_hypothesis(&mut self, hypothesis: Hypothesis) -> Result<(), String> {
        if hypothesis.id.trim().is_empty() || hypothesis.statement.trim().is_empty() {
            return Err("hypothesis id and statement must not be empty".into());
        }
        if hypothesis.parent_ids.iter().any(|id| id == &hypothesis.id) {
            return Err("a hypothesis cannot be its own parent".into());
        }
        if self.hypotheses.contains_key(&hypothesis.id) {
            return Err(format!("duplicate hypothesis id: {}", hypothesis.id));
        }
        self.hypotheses.insert(hypothesis.id.clone(), hypothesis);
        self.touch();
        Ok(())
    }

    pub fn record_verification(&mut self, record: VerificationRecord) -> Result<(), String> {
        validate_verification(&record)?;
        if self.verifications.contains_key(&record.id) {
            return Err(format!("duplicate verification id: {}", record.id));
        }
        if let Some(parent) = &record.reproduction_of
            && !self.verifications.contains_key(parent)
        {
            return Err(format!(
                "reproduction references unknown verification: {parent}"
            ));
        }
        self.verifications.insert(record.id.clone(), record);
        self.touch();
        Ok(())
    }

    /// Attach a registered evidence artifact to a claim without changing the
    /// claim's epistemic status. Status changes require a separate verifier or
    /// human decision so the mere presence of a link cannot manufacture truth.
    pub fn link_claim_evidence(
        &mut self,
        claims: &mut ClaimLedger,
        claim_id: &str,
        evidence_id: &str,
        supports: bool,
        strength: f64,
    ) -> Result<(), String> {
        let evidence = self
            .evidence
            .get(evidence_id)
            .ok_or_else(|| format!("unknown evidence id: {evidence_id}"))?;
        if !strength.is_finite() || !(0.0..=1.0).contains(&strength) {
            return Err("evidence strength must be finite and between 0 and 1".into());
        }
        let claim = claims
            .claims
            .get_mut(claim_id)
            .ok_or_else(|| format!("unknown claim id: {claim_id}"))?;
        if !claim
            .evidence
            .iter()
            .any(|item| item.source_id == evidence_id)
        {
            claim.evidence.push(EvidenceRef {
                source_id: evidence_id.to_string(),
                locator: evidence.locator.clone(),
                content_sha256: Some(evidence.content_sha256.clone()),
                supports,
                strength,
            });
        }
        self.touch();
        Ok(())
    }

    /// Promote a claim only from an accepting verification record, while also
    /// creating an evidence artifact that commits to the verifier output.
    pub fn apply_verification_to_claim(
        &mut self,
        claims: &mut ClaimLedger,
        verification_id: &str,
        strength: f64,
    ) -> Result<String, String> {
        let record = self
            .verifications
            .get(verification_id)
            .ok_or_else(|| format!("unknown verification id: {verification_id}"))?
            .clone();
        if record.status != VerificationStatus::Verified {
            return Err("only an accepting verification can promote a claim".into());
        }
        if !claims.claims.contains_key(&record.subject_id) {
            return Err(format!(
                "verification subject is not a known claim: {}",
                record.subject_id
            ));
        }
        let evidence_id = format!("verification:{}", record.id);
        if !self.evidence.contains_key(&evidence_id) {
            self.register_evidence(EvidenceArtifact {
                id: evidence_id.clone(),
                kind: match record.kind {
                    VerificationKind::FormalProof => EvidenceKind::Proof,
                    VerificationKind::Benchmark => EvidenceKind::Benchmark,
                    VerificationKind::ExecutableTest
                    | VerificationKind::Simulation
                    | VerificationKind::SourceAudit
                    | VerificationKind::Reproduction
                    | VerificationKind::HumanReview => EvidenceKind::VerificationOutput,
                },
                title: format!("Verification {}", record.id),
                content_sha256: record.output_sha256.clone(),
                media_type: "application/json".into(),
                source_uri: None,
                locator: None,
                artifact_name: None,
                verification_id: Some(record.id.clone()),
                captured_at: record.completed_at,
                independent: record.reproduction_of.is_some(),
                metadata: BTreeMap::from([
                    ("evaluator_id".into(), record.evaluator_id.clone()),
                    ("evaluator_version".into(), record.evaluator_version.clone()),
                ]),
            })?;
        }
        self.link_claim_evidence(claims, &record.subject_id, &evidence_id, true, strength)?;
        let claim = claims
            .claims
            .get_mut(&record.subject_id)
            .expect("claim existence checked above");
        claim.status = if record.kind == VerificationKind::FormalProof {
            ClaimStatus::FormallyVerified
        } else {
            ClaimStatus::Supported
        };
        self.touch();
        Ok(evidence_id)
    }

    #[must_use]
    pub fn audit(&self, claims: &ClaimLedger) -> ChainOfEvidenceAudit {
        let mut issues = Vec::new();
        if self.schema != INVESTIGATION_SCHEMA {
            error(
                &mut issues,
                "unsupported_schema",
                &self.id,
                format!("unsupported investigation schema: {}", self.schema),
            );
        }
        if self.problem.statement.trim().is_empty() {
            warning(
                &mut issues,
                "missing_problem_statement",
                &self.id,
                "investigation has no problem statement",
            );
        }

        for (key, evidence) in &self.evidence {
            if key != &evidence.id {
                error(
                    &mut issues,
                    "evidence_key_mismatch",
                    key,
                    "evidence map key does not match record id",
                );
            }
            if let Err(message) = validate_evidence(evidence) {
                error(&mut issues, "invalid_evidence", key, message);
            }
            if let Some(verification_id) = &evidence.verification_id
                && !self.verifications.contains_key(verification_id)
            {
                error(
                    &mut issues,
                    "dangling_evidence_verification",
                    key,
                    format!("unknown verification: {verification_id}"),
                );
            }
        }

        for (key, record) in &self.verifications {
            if key != &record.id {
                error(
                    &mut issues,
                    "verification_key_mismatch",
                    key,
                    "verification map key does not match record id",
                );
            }
            if let Err(message) = validate_verification(record) {
                error(&mut issues, "invalid_verification", key, message);
            }
            if let Some(parent) = &record.reproduction_of
                && !self.verifications.contains_key(parent)
            {
                error(
                    &mut issues,
                    "dangling_reproduction",
                    key,
                    format!("unknown verification: {parent}"),
                );
            }
        }

        for (key, hypothesis) in &self.hypotheses {
            if key != &hypothesis.id {
                error(
                    &mut issues,
                    "hypothesis_key_mismatch",
                    key,
                    "hypothesis map key does not match record id",
                );
            }
            for parent in &hypothesis.parent_ids {
                if !self.hypotheses.contains_key(parent) {
                    error(
                        &mut issues,
                        "dangling_hypothesis_parent",
                        key,
                        format!("unknown parent hypothesis: {parent}"),
                    );
                }
            }
            for claim in &hypothesis.claim_ids {
                if !claims.claims.contains_key(claim) {
                    error(
                        &mut issues,
                        "dangling_hypothesis_claim",
                        key,
                        format!("unknown claim: {claim}"),
                    );
                }
            }
            for evidence in &hypothesis.evidence_ids {
                if !self.evidence.contains_key(evidence) {
                    error(
                        &mut issues,
                        "dangling_hypothesis_evidence",
                        key,
                        format!("unknown evidence: {evidence}"),
                    );
                }
            }
            for verification in &hypothesis.verification_ids {
                if !self.verifications.contains_key(verification) {
                    error(
                        &mut issues,
                        "dangling_hypothesis_verification",
                        key,
                        format!("unknown verification: {verification}"),
                    );
                }
            }
        }

        let mut evidence_linked_claims = 0;
        let mut verified_claims = 0;
        for (key, claim) in &claims.claims {
            if key != &claim.id {
                error(
                    &mut issues,
                    "claim_key_mismatch",
                    key,
                    "claim map key does not match record id",
                );
            }
            if claim.id.trim().is_empty()
                || claim.text.trim().is_empty()
                || !claim.confidence.is_finite()
                || !(0.0..=1.0).contains(&claim.confidence)
            {
                error(
                    &mut issues,
                    "invalid_claim",
                    key,
                    "claim has an invalid identity, text, or confidence",
                );
            }
            if !claim.evidence.is_empty() {
                evidence_linked_claims += 1;
            }
            let mut linked_evidence = BTreeSet::new();
            for evidence_ref in &claim.evidence {
                if evidence_ref.source_id.trim().is_empty()
                    || !evidence_ref.strength.is_finite()
                    || !(0.0..=1.0).contains(&evidence_ref.strength)
                    || evidence_ref
                        .content_sha256
                        .as_ref()
                        .is_some_and(|hash| validate_sha256(hash, "claim evidence").is_err())
                {
                    error(
                        &mut issues,
                        "invalid_claim_evidence",
                        &claim.id,
                        "claim contains an invalid evidence reference",
                    );
                }
                if !linked_evidence.insert(evidence_ref.source_id.as_str()) {
                    error(
                        &mut issues,
                        "duplicate_claim_evidence",
                        &claim.id,
                        format!("duplicate evidence link: {}", evidence_ref.source_id),
                    );
                }
                match self.evidence.get(&evidence_ref.source_id) {
                    None => error(
                        &mut issues,
                        "dangling_claim_evidence",
                        &claim.id,
                        format!("unknown evidence: {}", evidence_ref.source_id),
                    ),
                    Some(evidence) => {
                        if evidence_ref
                            .content_sha256
                            .as_ref()
                            .is_some_and(|hash| hash != &evidence.content_sha256)
                        {
                            error(
                                &mut issues,
                                "claim_evidence_hash_mismatch",
                                &claim.id,
                                format!("hash mismatch for evidence {}", evidence.id),
                            );
                        }
                    }
                }
            }

            if matches!(
                claim.status,
                ClaimStatus::Supported | ClaimStatus::FormallyVerified
            ) && claim.evidence.is_empty()
            {
                error(
                    &mut issues,
                    "unsupported_promoted_claim",
                    &claim.id,
                    "supported or formally verified claim has no evidence",
                );
            }

            if claim.status == ClaimStatus::FormallyVerified {
                let formal = self.verifications.values().any(|record| {
                    record.subject_id == claim.id
                        && record.kind == VerificationKind::FormalProof
                        && record.status == VerificationStatus::Verified
                });
                if formal {
                    verified_claims += 1;
                } else {
                    error(
                        &mut issues,
                        "missing_formal_verification",
                        &claim.id,
                        "formally verified claim lacks an accepting formal-proof record",
                    );
                }
            } else if self.verifications.values().any(|record| {
                record.subject_id == claim.id && record.status == VerificationStatus::Verified
            }) {
                verified_claims += 1;
            }
        }

        let mut seen_edges = BTreeSet::new();
        for edge in &claims.edges {
            if edge.from == edge.to
                || !claims.claims.contains_key(&edge.from)
                || !claims.claims.contains_key(&edge.to)
            {
                error(
                    &mut issues,
                    "invalid_claim_edge",
                    &edge.from,
                    format!("invalid {:?} edge to {}", edge.relation, edge.to),
                );
            }
            let relation = format!("{:?}", edge.relation);
            if !seen_edges.insert((edge.from.as_str(), edge.to.as_str(), relation)) {
                error(
                    &mut issues,
                    "duplicate_claim_edge",
                    &edge.from,
                    format!("duplicate edge to {}", edge.to),
                );
            }
        }

        let claim_count = claims.claims.len();
        let verification_coverage = if claim_count == 0 {
            0.0
        } else {
            verified_claims as f64 / claim_count as f64
        };
        ChainOfEvidenceAudit {
            passed: !issues
                .iter()
                .any(|issue| issue.severity == AuditSeverity::Error),
            claim_count,
            evidence_linked_claims,
            verified_claims,
            verification_coverage,
            issues,
        }
    }

    /// Assess whether a result is ready to be described as scientifically
    /// established. Proposals, conjectures, and assumptions remain useful in a
    /// bundle, but they cannot by themselves satisfy this gate.
    #[must_use]
    pub fn scientific_release_assessment(
        &self,
        claims: &ClaimLedger,
    ) -> ScientificReleaseAssessment {
        let audit = self.audit(claims);
        let substantive = claims
            .claims
            .values()
            .filter(|claim| matches!(claim.kind, ClaimKind::Fact | ClaimKind::Inference))
            .collect::<Vec<_>>();
        let evidence_linked_claims = substantive
            .iter()
            .filter(|claim| claim.evidence.iter().any(|evidence| evidence.supports))
            .count();
        let objectively_verified_claims = substantive
            .iter()
            .filter(|claim| {
                matches!(
                    claim.status,
                    ClaimStatus::Supported | ClaimStatus::FormallyVerified
                ) && self.verifications.values().any(|record| {
                    record.subject_id == claim.id && record.status == VerificationStatus::Verified
                })
            })
            .count();
        let verification_coverage = if substantive.is_empty() {
            0.0
        } else {
            objectively_verified_claims as f64 / substantive.len() as f64
        };
        let mut blocking_reasons = Vec::new();
        if !audit.passed {
            blocking_reasons.push("evidence graph failed its integrity audit".into());
        }
        if substantive.is_empty() {
            blocking_reasons.push("no explicit fact or inference was recorded".into());
        } else {
            if evidence_linked_claims < substantive.len() {
                blocking_reasons.push("one or more substantive claims lack evidence links".into());
            }
            if objectively_verified_claims < substantive.len() {
                blocking_reasons.push(
                    "one or more substantive claims lack accepting objective verification".into(),
                );
            }
        }
        ScientificReleaseAssessment {
            eligible: blocking_reasons.is_empty(),
            integrity_passed: audit.passed,
            substantive_claims: substantive.len(),
            evidence_linked_claims,
            objectively_verified_claims,
            verification_coverage,
            blocking_reasons,
        }
    }

    fn touch(&mut self) {
        self.updated_at = now();
    }
}

fn validate_evidence(evidence: &EvidenceArtifact) -> Result<(), String> {
    if evidence.id.trim().is_empty()
        || evidence.title.trim().is_empty()
        || evidence.media_type.trim().is_empty()
    {
        return Err("evidence id, title, and media type must not be empty".into());
    }
    validate_sha256(&evidence.content_sha256, "evidence content")
}

fn validate_verification(record: &VerificationRecord) -> Result<(), String> {
    if record.id.trim().is_empty()
        || record.subject_id.trim().is_empty()
        || record.evaluator_id.trim().is_empty()
        || record.evaluator_version.trim().is_empty()
    {
        return Err(
            "verification id, subject, evaluator id, and evaluator version must not be empty"
                .into(),
        );
    }
    validate_sha256(&record.specification_sha256, "verification specification")?;
    validate_sha256(&record.input_sha256, "verification input")?;
    validate_sha256(&record.output_sha256, "verification output")?;
    if record.completed_at < record.started_at {
        return Err("verification completion precedes its start".into());
    }
    if record.measurements.iter().any(|measurement| {
        measurement.name.trim().is_empty()
            || measurement.unit.trim().is_empty()
            || !measurement.value.is_finite()
            || measurement
                .uncertainty
                .is_some_and(|value| !value.is_finite() || value < 0.0)
    }) {
        return Err("verification contains an invalid measurement".into());
    }
    Ok(())
}

fn validate_sha256(value: &str, label: &str) -> Result<(), String> {
    if value.len() != 64 || !value.bytes().all(|byte| byte.is_ascii_hexdigit()) {
        return Err(format!("{label} requires a 64-character SHA-256 digest"));
    }
    Ok(())
}

fn error(
    issues: &mut Vec<EvidenceAuditIssue>,
    code: &str,
    subject_id: &str,
    message: impl Into<String>,
) {
    issues.push(EvidenceAuditIssue {
        severity: AuditSeverity::Error,
        code: code.into(),
        subject_id: subject_id.into(),
        message: message.into(),
    });
}

fn warning(
    issues: &mut Vec<EvidenceAuditIssue>,
    code: &str,
    subject_id: &str,
    message: impl Into<String>,
) {
    issues.push(EvidenceAuditIssue {
        severity: AuditSeverity::Warning,
        code: code.into(),
        subject_id: subject_id.into(),
        message: message.into(),
    });
}

fn now() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}

/// Return duplicate identifiers in a stable order. Useful to validate imported
/// interchange arrays before they are converted into maps.
#[must_use]
pub fn duplicate_ids<'a>(ids: impl IntoIterator<Item = &'a str>) -> Vec<String> {
    let mut seen = BTreeSet::new();
    let mut duplicates = BTreeSet::new();
    for id in ids {
        if !seen.insert(id) {
            duplicates.insert(id.to_string());
        }
    }
    duplicates.into_iter().collect()
}
