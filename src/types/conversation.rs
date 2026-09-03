use crate::types::artifact::{Artifact, ArtifactDiff};
use crate::types::compute::BudgetLedger;
use crate::types::fiduciary::PersonaDisclosure;
use crate::types::planning::GoalTree;
use serde::{Deserialize, Serialize};
use sha2::Digest;
use std::collections::BTreeMap;
use std::sync::Arc;
use std::time::{SystemTime, UNIX_EPOCH};

/// The observable result of a single model turn.
#[derive(Debug, Serialize, Deserialize, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum TurnOutcome {
    /// The generated artifact compiled successfully.
    Compiled,
    /// All tests passed after applying the turn's changes.
    TestsPassed,
    /// The turn moved the session closer to the goal.
    AdvancedConvergence,
    /// The turn was reverted; prior state restored.
    RolledBack,
    /// The turn was rejected by the consensus engine.
    Rejected,
    /// The artifact failed formal verification (Verus).
    VerificationFailed,
    /// No meaningful progress was made.
    Stalled,
    /// Outcome has not been evaluated yet.
    Unknown,
}

/// The prompt layout strategy used when generating a turn.
#[derive(Debug, Serialize, Deserialize, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum TurnStructure {
    /// Unconstrained free-form output.
    FreeForm,
    /// Numbered step-by-step reasoning.
    StepByStep,
    /// Explicit pros/cons enumeration.
    ProsCons,
    /// State a hypothesis then validate it.
    HypothesisTest,
    /// Lead with code, follow with explanation.
    CodeFirst,
    /// Symbolic logic and mathematical notation.
    Symbolic,
}

/// High-level category used for routing, analytics, and prompt selection.
#[derive(Debug, Serialize, Deserialize, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum TaskCategory {
    CodeGeneration,
    Debugging,
    Architecture,
    Refactoring,
    Research,
    Testing,
    General,
}

impl TaskCategory {
    pub fn preferred_structure(self) -> TurnStructure {
        match self {
            TaskCategory::Research => TurnStructure::Symbolic,
            TaskCategory::CodeGeneration => TurnStructure::CodeFirst,
            TaskCategory::General => TurnStructure::FreeForm,
            _ => TurnStructure::StepByStep,
        }
    }

    pub fn token_estimate(self) -> u32 {
        match self {
            TaskCategory::Architecture => 2500,
            TaskCategory::Research => 2200,
            TaskCategory::CodeGeneration => 2000,
            TaskCategory::Refactoring => 1800,
            TaskCategory::General => 1000,
            TaskCategory::Debugging | TaskCategory::Testing => 1500,
        }
    }
}

/// A single model response within a session, including its diff, metadata, and
/// cryptographic signature.
#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct Turn {
    pub index: u32,
    pub model_id: String,
    pub content: String,
    pub timestamp: u64,
    pub diffs: Vec<(String, ArtifactDiff)>,
    #[serde(default)]
    pub certainty: Option<f64>,
    #[serde(default = "default_outcome")]
    pub outcome: TurnOutcome,
    pub task_category: Option<TaskCategory>,
    pub structure: Option<TurnStructure>,
    #[serde(default)]
    pub signature: Vec<u8>,
    #[serde(default)]
    pub surprise_signal: Option<f64>,
    #[serde(default)]
    pub consistency_score: Option<f64>,
    /// Per-agent diff quality score at the time this turn was committed. Stored
    /// here for observability; the live score lives in IntelligenceEngine.
    #[serde(default)]
    pub diff_quality_score: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub persona_disclosure: Option<PersonaDisclosure>,
}

fn default_outcome() -> TurnOutcome {
    TurnOutcome::Unknown
}

/// SHA-256 over a turn's canonical serialization (content, metadata, and
/// signature). Field order is fixed by the struct, so the digest is stable.
fn turn_content_hash(turn: &Turn) -> [u8; 32] {
    let bytes = serde_json::to_vec(turn).unwrap_or_default();
    sha2::Sha256::digest(bytes).into()
}

/// Full mutable state of a running session, persisted to Sled on every checkpoint.
#[derive(Debug, Serialize, Deserialize, Clone, Default)]
pub struct ConversationState {
    pub session_id: String,
    pub iteration_index: u32,
    pub turns: Vec<Turn>,
    pub artifacts: BTreeMap<String, Arc<Artifact>>,
    #[serde(default)]
    pub agent_weights: BTreeMap<String, f64>,
    #[serde(default)]
    pub completion_probability: f64,
    #[serde(default)]
    pub state_hash: [u8; 32],
    #[serde(default)]
    pub budget: BudgetLedger,
    #[serde(default)]
    pub goal_tree: GoalTree,
    #[serde(default)]
    pub node_consensus: BTreeMap<String, f64>,
    #[serde(default)]
    pub last_verification: Vec<(String, String, bool)>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub principal_id: Option<String>,
    #[serde(default)]
    pub mode_library: crate::types::mode::ModeLibrary,
    #[serde(default)]
    pub novel_signal: Option<String>,
    #[serde(default)]
    pub last_tool_outputs: Vec<(String, String)>,
    /// Typed claims and evidence accumulated during domain-general inquiry.
    #[serde(default)]
    pub claim_ledger: crate::types::epistemics::ClaimLedger,
    /// Evidence-linked hypotheses, verification records, and measurements for
    /// the discovery process. Kept separate from the transcript so an audit can
    /// distinguish what agents said from what the system actually established.
    #[serde(default)]
    pub investigation: crate::types::investigation::Investigation,
    #[serde(default)]
    pub rejection_loop_active: bool,
    #[serde(default)]
    pub mode_active_turns: u32,
    /// Running hash chain over `turns`: `turn_hashes[i]` commits to
    /// `turn_hashes[i-1]` and the content of `turns[i]`, so any edit,
    /// reorder, insertion, or deletion within the retained window is
    /// detectable without a secret key. Maintained in lockstep by `push_turn`.
    #[serde(default)]
    pub turn_hashes: Vec<Vec<u8>>,
    /// Hash immediately preceding the first retained turn after the bounded
    /// transcript window drops older entries. This keeps the first retained
    /// turn verifiable instead of treating it as an uncommitted anchor.
    #[serde(default)]
    pub turn_chain_base: Option<Vec<u8>>,
}

impl ConversationState {
    pub fn new(session_id: &str) -> Self {
        Self {
            session_id: session_id.to_string(),
            iteration_index: 0,
            turns: vec![],
            artifacts: BTreeMap::new(),
            agent_weights: BTreeMap::new(),
            completion_probability: 0.0,
            state_hash: [0u8; 32],
            budget: BudgetLedger::default(),
            goal_tree: GoalTree::default(),
            node_consensus: BTreeMap::new(),
            last_verification: Vec::new(),
            principal_id: None,
            mode_library: crate::types::mode::ModeLibrary::new(),
            novel_signal: None,
            last_tool_outputs: Vec::new(),
            claim_ledger: crate::types::epistemics::ClaimLedger::default(),
            investigation: crate::types::investigation::Investigation::new(session_id, ""),
            rejection_loop_active: false,
            mode_active_turns: 0,
            turn_hashes: Vec::new(),
            turn_chain_base: None,
        }
    }

    pub fn ingest_file(&mut self, name: String, language: String, content: String) {
        const MAX_FILE_BYTES: usize = 10_000_000;
        if content.len() > MAX_FILE_BYTES {
            tracing::warn!(file = %name, bytes = content.len(), "ingest_file: file exceeds 10 MB limit; skipping");
            return;
        }
        use crate::engines::validation::AstValidator;
        use crate::types::artifact::Artifact;
        let content_sha256 = format!("{:x}", sha2::Sha256::digest(content.as_bytes()));
        let evidence_id = format!("artifact:{}:{}", name, &content_sha256[..16]);
        let evidence_kind = match language.to_ascii_lowercase().as_str() {
            "lean" | "lean4" | "coq" | "verus" => crate::types::investigation::EvidenceKind::Proof,
            "rust" | "rs" | "python" | "py" | "javascript" | "js" | "typescript" | "ts" | "go"
            | "java" | "c" | "cpp" => crate::types::investigation::EvidenceKind::Code,
            "csv" | "tsv" => crate::types::investigation::EvidenceKind::Dataset,
            _ if name.starts_with("research/") => {
                crate::types::investigation::EvidenceKind::SourceSnapshot
            }
            _ if name.starts_with("evolution/") => {
                crate::types::investigation::EvidenceKind::Observation
            }
            _ => crate::types::investigation::EvidenceKind::SourceSnapshot,
        };
        let media_type = media_type_for_language(&language);
        let skeleton = AstValidator::generate_skeleton(&content, &language);
        let artifact = Artifact {
            name: name.clone(),
            language,
            content,
            version: 0,
            history: vec![],
            ast_versions: std::collections::BTreeMap::new(),
            proof_attachments: vec![],
            metrics: crate::engines::quality::ArtifactMetrics::default(),
            skeleton,
        };
        let all_names: Vec<String> = self.artifacts.keys().cloned().collect();
        let metrics =
            crate::engines::quality::QualityEngine::analyze_artifact(&artifact, &all_names);
        self.artifacts.insert(
            name.clone(),
            std::sync::Arc::new(Artifact {
                metrics,
                ..artifact
            }),
        );
        if !self.investigation.evidence.contains_key(&evidence_id)
            && let Err(error) = self.investigation.register_evidence(
                crate::types::investigation::EvidenceArtifact {
                    id: evidence_id,
                    kind: evidence_kind,
                    title: name.clone(),
                    content_sha256,
                    media_type: media_type.into(),
                    source_uri: None,
                    locator: None,
                    artifact_name: Some(name),
                    verification_id: None,
                    captured_at: Self::now(),
                    independent: false,
                    metadata: BTreeMap::new(),
                },
            )
        {
            tracing::warn!(%error, "failed to register ingested artifact as evidence");
        }
    }

    pub fn push_turn(&mut self, turn: Turn) {
        const MAX_TURNS: usize = 200;
        if turn.model_id != "User" {
            self.claim_ledger.ingest_tagged(
                turn.index,
                &turn.content,
                turn.certainty.unwrap_or(0.5),
            );
        }
        // Extend the tamper-evident hash chain before appending the turn.
        let mut hasher = sha2::Sha256::new();
        if let Some(prev) = self.turn_hashes.last() {
            hasher.update(prev);
        }
        hasher.update(turn_content_hash(&turn));
        self.turn_hashes.push(hasher.finalize().to_vec());
        self.turns.push(turn);
        if self.turns.len() > MAX_TURNS {
            let excess = self.turns.len() - MAX_TURNS;
            self.turn_chain_base = self.turn_hashes.get(excess - 1).cloned();
            self.turns.drain(..excess);
            // Keep the chain aligned with the retained turns.
            if self.turn_hashes.len() >= excess {
                self.turn_hashes.drain(..excess);
            }
        }
    }

    /// Replace the most recently retained turn and rebuild the transcript hash
    /// chain. Orchestration uses this when a provisional outcome is finalized
    /// after external verification. Callers must provide a freshly signed turn.
    pub fn replace_last_turn(&mut self, turn: Turn) -> Result<(), String> {
        let Some(last) = self.turns.last_mut() else {
            return Err("cannot replace a turn in an empty transcript".into());
        };
        if last.index != turn.index {
            return Err(format!(
                "replacement turn index {} does not match retained tail {}",
                turn.index, last.index
            ));
        }
        *last = turn;
        self.rebuild_turn_hashes();
        Ok(())
    }

    /// Recompute the derived transcript commitments after a trusted internal
    /// finalization step. This is crate-visible so orchestration can close a
    /// provisional turn transaction before persisting or exporting state.
    pub(crate) fn rebuild_turn_hashes(&mut self) {
        let mut rebuilt = Vec::with_capacity(self.turns.len());
        for turn in &self.turns {
            let mut hasher = sha2::Sha256::new();
            if let Some(previous) = rebuilt.last() {
                hasher.update(previous);
            } else if let Some(base) = &self.turn_chain_base {
                hasher.update(base);
            }
            hasher.update(turn_content_hash(turn));
            rebuilt.push(hasher.finalize().to_vec());
        }
        self.turn_hashes = rebuilt;
    }

    /// The current head of the turn hash chain (hex), suitable for anchoring in
    /// an external append-only log (e.g. a git commit message). Empty when no
    /// turns have been recorded.
    #[must_use]
    pub fn chain_head_hex(&self) -> String {
        match self.turn_hashes.last() {
            Some(h) => h.iter().map(|b| format!("{b:02x}")).collect(),
            None => String::new(),
        }
    }

    /// Verify the internal consistency of the turn hash chain over the retained
    /// window. Returns the index of the first turn that fails to chain, or
    /// `None` if the chain is intact (or absent, for legacy states predating it).
    #[must_use]
    pub fn verify_chain(&self) -> Option<usize> {
        if self.turn_hashes.is_empty() {
            return (!self.turns.is_empty()).then_some(0);
        }
        if self.turn_hashes.len() != self.turns.len() {
            return Some(0); // chain/turn count diverged → tampering
        }
        if self.turn_chain_base.is_none() && self.turns.first().is_some_and(|turn| turn.index != 0)
        {
            return Some(0);
        }
        for i in 0..self.turns.len() {
            let mut hasher = sha2::Sha256::new();
            if i > 0 {
                hasher.update(&self.turn_hashes[i - 1]);
            } else if let Some(base) = &self.turn_chain_base {
                hasher.update(base);
            }
            hasher.update(turn_content_hash(&self.turns[i]));
            if self.turn_hashes[i].as_slice() != hasher.finalize().as_slice() {
                return Some(i);
            }
        }
        None
    }

    pub fn now() -> u64 {
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs()
    }
}

fn media_type_for_language(language: &str) -> &'static str {
    match language.to_ascii_lowercase().as_str() {
        "json" => "application/json",
        "csv" => "text/csv",
        "tsv" => "text/tab-separated-values",
        "markdown" | "md" => "text/markdown",
        "rust" | "rs" | "verus" => "text/x-rust",
        "lean" | "lean4" => "text/x-lean",
        "coq" => "text/x-coq",
        "python" | "py" => "text/x-python",
        _ => "text/plain",
    }
}
