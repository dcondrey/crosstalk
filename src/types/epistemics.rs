use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum ClaimKind {
    Fact,
    Assumption,
    Inference,
    Conjecture,
    Proposal,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum ClaimStatus {
    Open,
    Supported,
    Contested,
    Falsified,
    FormallyVerified,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct EvidenceRef {
    pub source_id: String,
    pub locator: Option<String>,
    pub content_sha256: Option<String>,
    pub supports: bool,
    pub strength: f64,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Claim {
    pub id: String,
    pub text: String,
    pub kind: ClaimKind,
    pub status: ClaimStatus,
    pub confidence: f64,
    pub evidence: Vec<EvidenceRef>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum ClaimRelation {
    Supports,
    Contradicts,
    DependsOn,
    Refines,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ClaimEdge {
    pub from: String,
    pub to: String,
    pub relation: ClaimRelation,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ClaimLedger {
    pub claims: BTreeMap<String, Claim>,
    pub edges: Vec<ClaimEdge>,
}

impl ClaimLedger {
    /// Capture explicitly tagged model claims without pretending to infer
    /// epistemic types from ordinary prose.
    pub fn ingest_tagged(&mut self, turn_index: u32, content: &str, confidence: f64) {
        for (line_index, line) in content.lines().enumerate() {
            let trimmed = line.trim();
            let tags = [
                ("[FACT]", ClaimKind::Fact),
                ("[ASSUMPTION]", ClaimKind::Assumption),
                ("[INFERENCE]", ClaimKind::Inference),
                ("[CONJECTURE]", ClaimKind::Conjecture),
                ("[PROPOSAL]", ClaimKind::Proposal),
            ];
            let Some((tag, kind)) = tags.iter().find(|(tag, _)| trimmed.starts_with(tag)) else {
                continue;
            };
            let text = trimmed[tag.len()..].trim();
            if text.is_empty() {
                continue;
            }
            let id = format!("turn-{turn_index}-claim-{line_index}");
            let _ = self.insert(Claim {
                id,
                text: text.to_string(),
                kind: *kind,
                status: ClaimStatus::Open,
                confidence: confidence.clamp(0.0, 1.0),
                evidence: vec![],
            });
        }
    }

    pub fn insert(&mut self, claim: Claim) -> Result<(), String> {
        if claim.id.trim().is_empty() || claim.text.trim().is_empty() {
            return Err("claim id and text must not be empty".into());
        }
        if !(0.0..=1.0).contains(&claim.confidence) {
            return Err("claim confidence must be between 0 and 1".into());
        }
        if claim
            .evidence
            .iter()
            .any(|e| !(0.0..=1.0).contains(&e.strength))
        {
            return Err("evidence strength must be between 0 and 1".into());
        }
        if self.claims.contains_key(&claim.id) {
            return Err(format!("duplicate claim id: {}", claim.id));
        }
        self.claims.insert(claim.id.clone(), claim);
        Ok(())
    }

    pub fn connect(&mut self, edge: ClaimEdge) -> Result<(), String> {
        if !self.claims.contains_key(&edge.from) || !self.claims.contains_key(&edge.to) {
            return Err("claim edge references an unknown claim".into());
        }
        if edge.from == edge.to {
            return Err("a claim cannot relate to itself".into());
        }
        if !self.edges.contains(&edge) {
            self.edges.push(edge);
        }
        Ok(())
    }

    #[must_use]
    pub fn unresolved_cruxes(&self) -> Vec<&Claim> {
        let depended_on: BTreeSet<&str> = self
            .edges
            .iter()
            .filter(|e| e.relation == ClaimRelation::DependsOn)
            .map(|e| e.to.as_str())
            .collect();
        self.claims
            .values()
            .filter(|claim| {
                depended_on.contains(claim.id.as_str())
                    && matches!(claim.status, ClaimStatus::Open | ClaimStatus::Contested)
            })
            .collect()
    }

    #[must_use]
    pub fn verification_coverage(&self) -> f64 {
        if self.claims.is_empty() {
            return 0.0;
        }
        let covered = self
            .claims
            .values()
            .filter(|claim| {
                matches!(
                    claim.status,
                    ClaimStatus::Supported | ClaimStatus::FormallyVerified
                ) && !claim.evidence.is_empty()
            })
            .count();
        covered as f64 / self.claims.len() as f64
    }
}
