use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};

pub const CHECKPOINT_SCHEMA: &str = "crosstalk.evolution.v1";

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum MutationOperator {
    Crossover,
    PointMutation,
    Inversion,
    Wildcard,
}

/// Untrusted source for a bounded counterexample search.
///
/// Evolution only transports and validates this artifact.  It must not be
/// executed without an objective evaluator or sandbox making that decision.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct FalsificationProbe {
    /// Runtime or source language, for example `python3` or `rust`.
    pub language: String,
    /// Complete source rather than a prose description of a future test.
    pub source: String,
    /// Argument vector; never reinterpret this as a shell command string.
    pub argv: Vec<String>,
    /// Declared wall-clock ceiling for a future sandbox execution.
    pub timeout_seconds: u64,
    /// Machine-observable output or exit condition that rejects the candidate.
    pub falsifies_when: String,
}

impl FalsificationProbe {
    pub fn validate(&self) -> Result<(), String> {
        if self.language.trim().is_empty()
            || self.language.len() > 128
            || self.source.trim().is_empty()
            || self.source.len() > 64_000
            || self.argv.is_empty()
            || self.argv.len() > 32
            || self
                .argv
                .iter()
                .any(|argument| argument.trim().is_empty() || argument.len() > 4_096)
            || !(1..=300).contains(&self.timeout_seconds)
            || self.falsifies_when.trim().is_empty()
            || self.falsifies_when.len() > 16_000
        {
            return Err("falsification probe fields must be executable-shaped and bounded".into());
        }
        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ExecutableContract {
    /// Concrete state or object on which the proposed method operates.
    pub representation: String,
    /// Exact identity, recurrence, or input/output relation being asserted.
    pub exact_relation: String,
    /// Rule that composes smaller instances or advances the construction.
    pub composition_rule: String,
    /// Why the rule avoids the work forbidden by the problem statement.
    pub complexity_argument: String,
    /// Deterministic probe capable of falsifying the central claim.
    pub objective_test: String,
    /// Complete, bounded source artifact for the proposed falsification probe.
    /// Legacy checkpoints may omit it; new discovery requests require it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub falsification_probe: Option<FalsificationProbe>,
    /// IDs of eliminated approaches and the structural difference from each.
    #[serde(default)]
    pub distinguishes_from: BTreeMap<String, String>,
}

impl ExecutableContract {
    pub fn validate(&self) -> Result<(), String> {
        if [
            &self.representation,
            &self.exact_relation,
            &self.composition_rule,
            &self.complexity_argument,
            &self.objective_test,
        ]
        .iter()
        .any(|field| field.trim().is_empty() || field.len() > 16_000)
        {
            return Err("executable contract fields must be non-empty and bounded".into());
        }
        if let Some(probe) = &self.falsification_probe {
            probe.validate()?;
        }
        if self.distinguishes_from.len() > 128
            || self.distinguishes_from.iter().any(|(id, difference)| {
                id.trim().is_empty()
                    || difference.trim().is_empty()
                    || id.len() > 1_024
                    || difference.len() > 16_000
            })
        {
            return Err("executable contract has invalid exclusion discriminators".into());
        }
        Ok(())
    }

    #[must_use]
    pub fn structural_text(&self) -> String {
        format!(
            "{} {} {}",
            self.representation, self.exact_relation, self.composition_rule
        )
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ConceptDraft {
    pub domain: String,
    pub title: String,
    pub mechanism: String,
    pub rationale: String,
    pub predicted_measurements: Vec<String>,
    pub kill_criteria: Vec<String>,
    pub tags: BTreeSet<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub executable_contract: Option<ExecutableContract>,
}

impl ConceptDraft {
    pub fn validate(&self) -> Result<(), String> {
        if self.domain.trim().is_empty()
            || self.title.trim().is_empty()
            || self.mechanism.trim().is_empty()
        {
            return Err("concept domain, title, and mechanism must not be empty".into());
        }
        if self.title.len() > 512 || self.mechanism.len() > 32_000 || self.rationale.len() > 32_000
        {
            return Err("concept draft exceeds field size limits".into());
        }
        if self.predicted_measurements.len() > 64
            || self.kill_criteria.len() > 64
            || self.tags.len() > 64
        {
            return Err("concept draft exceeds collection limits".into());
        }
        if let Some(contract) = &self.executable_contract {
            contract.validate()?;
        }
        Ok(())
    }

    #[must_use]
    pub fn structural_text(&self) -> String {
        self.executable_contract.as_ref().map_or_else(
            || format!("{} {}", self.title, self.mechanism),
            ExecutableContract::structural_text,
        )
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct StructuralExclusion {
    pub id: String,
    pub description: String,
    pub structural_features: Vec<String>,
    #[serde(default)]
    pub evidence_ids: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RejectedCandidate {
    pub generation: u32,
    pub attempt: usize,
    pub title: String,
    pub structural_text: String,
    pub reason: String,
    pub score: f64,
    pub fatal_flaws: Vec<String>,
}

impl StructuralExclusion {
    pub fn validate(&self) -> Result<(), String> {
        if self.id.trim().is_empty()
            || self.description.trim().is_empty()
            || self.structural_features.is_empty()
            || self.id.len() > 1_024
            || self.description.len() > 32_000
            || self.structural_features.len() > 128
            || self.evidence_ids.len() > 128
            || self
                .structural_features
                .iter()
                .any(|feature| feature.trim().is_empty() || feature.len() > 4_096)
            || self
                .evidence_ids
                .iter()
                .any(|evidence_id| evidence_id.trim().is_empty() || evidence_id.len() > 4_096)
        {
            return Err("structural exclusions require an ID, description, and features".into());
        }
        Ok(())
    }

    #[must_use]
    pub fn structural_text(&self) -> String {
        self.structural_features.join(" ")
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Fitness {
    pub novelty: f64,
    pub feasibility: f64,
    pub utility: f64,
    pub semantic_jump: f64,
    pub evidence: f64,
    pub safety: f64,
    pub prior_art_overlap: f64,
    #[serde(default, deserialize_with = "deserialize_string_or_vec")]
    pub fatal_flaws: Vec<String>,
    pub next_directive: String,
}

impl Fitness {
    pub fn validate(&self) -> Result<(), String> {
        for (name, value) in [
            ("novelty", self.novelty),
            ("feasibility", self.feasibility),
            ("utility", self.utility),
            ("semantic_jump", self.semantic_jump),
            ("evidence", self.evidence),
            ("safety", self.safety),
            ("prior_art_overlap", self.prior_art_overlap),
        ] {
            if !value.is_finite() || !(0.0..=10.0).contains(&value) {
                return Err(format!("{name} must be finite and between 0 and 10"));
            }
        }
        Ok(())
    }

    #[must_use]
    pub fn passes_hard_gates(&self, draft: &ConceptDraft, permissive: bool) -> bool {
        permissive
            || (self.fatal_flaws.is_empty()
                && self.safety >= 5.0
                && !draft.mechanism.trim().is_empty()
                && !draft.predicted_measurements.is_empty()
                && !draft.kill_criteria.is_empty())
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Concept {
    pub id: String,
    pub project: String,
    pub generation: u32,
    pub operator: MutationOperator,
    pub parent_ids: Vec<String>,
    pub draft: ConceptDraft,
    pub fitness: Fitness,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LineageEdge {
    pub parent_id: String,
    pub child_id: String,
    pub operator: MutationOperator,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct EvolutionConfig {
    pub population_size: usize,
    pub max_attempt_multiplier: usize,
    #[serde(default = "default_max_concurrency")]
    pub max_concurrency: usize,
    pub crossover_weight: u32,
    pub mutation_weight: u32,
    pub inversion_weight: u32,
    pub wildcard_weight: u32,
    pub retention_threshold: f64,
    pub permissive: bool,
    pub rejection_memory: usize,
    /// Maximum generator/evaluator call slots across the entire checkpoint.
    /// Each candidate attempt reserves two slots before it starts so bounded
    /// parallel batches can never oversubscribe. Zero means unlimited.
    #[serde(default)]
    pub max_model_call_slots: u64,
    /// Reject model prose that does not specify a falsifiable composition
    /// contract. Disabled for legacy checkpoints and generic embedding users.
    #[serde(default)]
    pub require_executable_contract: bool,
    /// Require a complete, bounded falsification source artifact in addition
    /// to the prose objective-test description. Disabled for old checkpoints.
    #[serde(default)]
    pub require_falsification_probe: bool,
}

impl Default for EvolutionConfig {
    fn default() -> Self {
        Self {
            population_size: 12,
            max_attempt_multiplier: 5,
            max_concurrency: 4,
            crossover_weight: 50,
            mutation_weight: 30,
            inversion_weight: 15,
            wildcard_weight: 5,
            retention_threshold: 7.0,
            permissive: false,
            rejection_memory: 100,
            max_model_call_slots: 0,
            require_executable_contract: false,
            require_falsification_probe: false,
        }
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct EvolutionUsage {
    pub generations_completed: u64,
    pub attempts_started: u64,
    /// Admission-control reservations. Actual model calls are less than or
    /// equal to this number when generation or validation fails early.
    pub model_call_slots_reserved: u64,
    pub failed_attempts: u64,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct EvolutionState {
    pub schema: String,
    pub project: String,
    pub directive: String,
    pub seed: u64,
    pub generation: u32,
    pub concepts: BTreeMap<String, Concept>,
    pub active_population: Vec<String>,
    pub lineage: Vec<LineageEdge>,
    pub rejected_titles: Vec<String>,
    #[serde(default)]
    pub rejected_structures: Vec<String>,
    /// Bounded negative-knowledge ledger used to steer later generations.
    #[serde(default)]
    pub rejected_candidates: Vec<RejectedCandidate>,
    #[serde(default)]
    pub structural_exclusions: Vec<StructuralExclusion>,
    /// Objective proof/test/benchmark feedback applied after model evaluation.
    /// Kept outside `Concept` for checkpoint compatibility and auditability.
    #[serde(default)]
    pub objective_feedback: BTreeMap<String, Vec<ObjectiveFeedback>>,
    #[serde(default)]
    pub usage: EvolutionUsage,
    pub config: EvolutionConfig,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ObjectiveFeedback {
    pub verification_id: String,
    pub evaluator_id: String,
    pub evaluator_version: String,
    pub output_sha256: String,
    pub passed: bool,
    pub hard_constraints_passed: bool,
    pub independently_reproduced: bool,
    pub measurements: BTreeMap<String, f64>,
    pub recorded_at: u64,
}

impl ObjectiveFeedback {
    pub fn validate(&self) -> Result<(), String> {
        if self.verification_id.trim().is_empty()
            || self.evaluator_id.trim().is_empty()
            || self.evaluator_version.trim().is_empty()
        {
            return Err("objective feedback requires verification and evaluator identities".into());
        }
        if self.output_sha256.len() != 64
            || !self
                .output_sha256
                .bytes()
                .all(|byte| byte.is_ascii_hexdigit())
        {
            return Err("objective feedback requires a SHA-256 output digest".into());
        }
        if self
            .measurements
            .iter()
            .any(|(name, value)| name.trim().is_empty() || !value.is_finite())
        {
            return Err("objective feedback contains an invalid measurement".into());
        }
        if self.passed && !self.hard_constraints_passed {
            return Err("passing objective feedback cannot fail a hard constraint".into());
        }
        if self.independently_reproduced && (!self.passed || !self.hard_constraints_passed) {
            return Err("only a passing result can be marked independently reproduced".into());
        }
        Ok(())
    }
}

impl EvolutionState {
    #[must_use]
    pub fn new(
        project: impl Into<String>,
        directive: impl Into<String>,
        seed: u64,
        config: EvolutionConfig,
    ) -> Self {
        Self {
            schema: CHECKPOINT_SCHEMA.into(),
            project: project.into(),
            directive: directive.into(),
            seed,
            generation: 0,
            concepts: BTreeMap::new(),
            active_population: vec![],
            lineage: vec![],
            rejected_titles: vec![],
            rejected_structures: vec![],
            rejected_candidates: vec![],
            structural_exclusions: vec![],
            objective_feedback: BTreeMap::new(),
            usage: EvolutionUsage::default(),
            config,
        }
    }

    /// Apply tool-derived feedback to a retained concept. Objective failures
    /// override optimistic model scores; independently reproduced passes raise
    /// evidence and feasibility but never manufacture novelty.
    pub fn apply_objective_feedback(
        &mut self,
        concept_id: &str,
        feedback: ObjectiveFeedback,
    ) -> Result<(), String> {
        feedback.validate()?;
        if !self.concepts.contains_key(concept_id) {
            return Err(format!("unknown concept: {concept_id}"));
        }
        if self
            .objective_feedback
            .values()
            .flatten()
            .any(|existing| existing.verification_id == feedback.verification_id)
        {
            return Err(format!(
                "duplicate objective verification: {}",
                feedback.verification_id
            ));
        }
        let concept = self
            .concepts
            .get_mut(concept_id)
            .expect("concept existence checked above");

        if feedback.passed && feedback.hard_constraints_passed {
            concept.fitness.evidence =
                concept
                    .fitness
                    .evidence
                    .max(if feedback.independently_reproduced {
                        10.0
                    } else {
                        8.0
                    });
            concept.fitness.feasibility = concept.fitness.feasibility.max(7.0);
        } else {
            concept.fitness.evidence = concept.fitness.evidence.min(3.0);
            concept.fitness.feasibility = concept.fitness.feasibility.min(3.0);
            let flaw = format!(
                "objective verification {} rejected the candidate",
                feedback.verification_id
            );
            if !concept.fitness.fatal_flaws.contains(&flaw) {
                concept.fitness.fatal_flaws.push(flaw);
            }
            if !feedback.hard_constraints_passed {
                self.active_population.retain(|id| id != concept_id);
            }
        }
        self.objective_feedback
            .entry(concept_id.to_string())
            .or_default()
            .push(feedback);
        Ok(())
    }

    pub fn checkpoint_json(&self) -> Result<String, serde_json::Error> {
        serde_json::to_string_pretty(self)
    }

    pub fn restore_json(json: &str) -> Result<Self, String> {
        let state: Self = serde_json::from_str(json).map_err(|e| e.to_string())?;
        if state.schema != CHECKPOINT_SCHEMA {
            return Err(format!("unsupported checkpoint schema: {}", state.schema));
        }
        if state.project.trim().is_empty() || state.directive.len() > 32_000 {
            return Err("checkpoint project is empty or directive is too large".into());
        }
        if state.config.population_size == 0
            || state.config.population_size > 1_000
            || state.config.max_concurrency == 0
            || state.config.max_concurrency > 128
            || state.config.max_attempt_multiplier == 0
            || state.config.max_attempt_multiplier > 1_000
            || state.config.rejection_memory > 100_000
            || !state.config.retention_threshold.is_finite()
            || (state.config.require_falsification_probe
                && !state.config.require_executable_contract)
        {
            return Err("checkpoint evolution configuration is invalid".into());
        }
        if state.config.max_model_call_slots > 0
            && state.usage.model_call_slots_reserved > state.config.max_model_call_slots
        {
            return Err("checkpoint exceeds its model-call-slot budget".into());
        }
        if state.usage.model_call_slots_reserved < state.usage.attempts_started.saturating_mul(2)
            || state.usage.failed_attempts > state.usage.attempts_started
            || state.usage.generations_completed > u64::from(state.generation)
        {
            return Err("checkpoint evolution usage is inconsistent".into());
        }
        if state.rejected_candidates.len() > state.config.rejection_memory
            || state.rejected_candidates.iter().any(|candidate| {
                candidate.generation > state.generation
                    || candidate.title.trim().is_empty()
                    || candidate.reason.trim().is_empty()
                    || !candidate.score.is_finite()
                    || candidate.structural_text.len() > 64_000
                    || candidate.fatal_flaws.len() > 128
            })
        {
            return Err("checkpoint rejected-candidate ledger is invalid".into());
        }
        if state.config.crossover_weight == 0
            && state.config.mutation_weight == 0
            && state.config.inversion_weight == 0
            && state.config.wildcard_weight == 0
        {
            return Err("checkpoint evolution configuration has no operator weight".into());
        }
        let mut population_ids = BTreeSet::new();
        if state
            .active_population
            .iter()
            .any(|id| !state.concepts.contains_key(id) || !population_ids.insert(id.as_str()))
        {
            return Err("checkpoint population contains an unknown or duplicate concept".into());
        }
        let mut exclusion_ids = BTreeSet::new();
        for exclusion in &state.structural_exclusions {
            exclusion.validate()?;
            if !exclusion_ids.insert(exclusion.id.as_str()) {
                return Err(format!("duplicate structural exclusion: {}", exclusion.id));
            }
        }
        for (key, concept) in &state.concepts {
            if key != &concept.id || concept.project != state.project {
                return Err(format!("invalid concept identity: {key}"));
            }
            concept.draft.validate()?;
            if state.config.require_executable_contract
                && concept.generation > 0
                && concept.draft.executable_contract.is_none()
            {
                return Err(format!(
                    "concept {} lacks its required executable contract",
                    concept.id
                ));
            }
            if state.config.require_falsification_probe
                && concept.generation > 0
                && concept
                    .draft
                    .executable_contract
                    .as_ref()
                    .and_then(|contract| contract.falsification_probe.as_ref())
                    .is_none()
            {
                return Err(format!(
                    "concept {} lacks its required falsification probe",
                    concept.id
                ));
            }
            if concept.generation > 0
                && concept
                    .draft
                    .executable_contract
                    .as_ref()
                    .is_some_and(|contract| {
                        state.structural_exclusions.iter().any(|exclusion| {
                            contract
                                .distinguishes_from
                                .get(&exclusion.id)
                                .is_none_or(|difference| difference.trim().is_empty())
                        })
                    })
            {
                return Err(format!(
                    "concept {} does not distinguish every structural exclusion",
                    concept.id
                ));
            }
            concept.fitness.validate()?;
            let mut parents = BTreeSet::new();
            if concept.parent_ids.iter().any(|parent| {
                parent == &concept.id
                    || !state.concepts.contains_key(parent)
                    || !parents.insert(parent.as_str())
            }) {
                return Err(format!("concept {} has invalid parents", concept.id));
            }
        }
        let mut lineage = BTreeSet::new();
        for edge in &state.lineage {
            let Some(child) = state.concepts.get(&edge.child_id) else {
                return Err(format!(
                    "lineage references unknown child: {}",
                    edge.child_id
                ));
            };
            let Some(parent) = state.concepts.get(&edge.parent_id) else {
                return Err(format!(
                    "lineage references unknown parent: {}",
                    edge.parent_id
                ));
            };
            if parent.generation >= child.generation
                || edge.parent_id == edge.child_id
                || !child.parent_ids.contains(&edge.parent_id)
                || child.operator != edge.operator
            {
                return Err(format!(
                    "invalid lineage edge: {} -> {}",
                    edge.parent_id, edge.child_id
                ));
            }
            let operator = format!("{:?}", edge.operator);
            if !lineage.insert((edge.parent_id.as_str(), edge.child_id.as_str(), operator)) {
                return Err(format!(
                    "duplicate lineage edge: {} -> {}",
                    edge.parent_id, edge.child_id
                ));
            }
        }
        for concept in state.concepts.values() {
            for parent in &concept.parent_ids {
                if !state.lineage.iter().any(|edge| {
                    edge.parent_id == *parent
                        && edge.child_id == concept.id
                        && edge.operator == concept.operator
                }) {
                    return Err(format!(
                        "concept {} is missing lineage for parent {parent}",
                        concept.id
                    ));
                }
            }
        }
        let mut verification_ids = BTreeSet::new();
        for (concept_id, feedback) in &state.objective_feedback {
            if !state.concepts.contains_key(concept_id) {
                return Err(format!(
                    "objective feedback references an unknown concept: {concept_id}"
                ));
            }
            for record in feedback {
                record.validate()?;
                if !verification_ids.insert(record.verification_id.as_str()) {
                    return Err(format!(
                        "duplicate objective verification: {}",
                        record.verification_id
                    ));
                }
            }
        }
        Ok(state)
    }
}

#[derive(Debug, Clone)]
pub struct GenerationContext<'a> {
    pub project: &'a str,
    pub directive: &'a str,
    pub generation: u32,
    pub attempt: usize,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct GenerationReport {
    pub generation: u32,
    pub attempts: usize,
    pub accepted: Vec<String>,
    pub rejected: usize,
    #[serde(default)]
    pub rejection_reasons: BTreeMap<String, usize>,
    pub failures: Vec<AttemptFailure>,
    pub pareto_frontier: Vec<String>,
    pub usage: EvolutionUsage,
    pub budget_exhausted: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AttemptFailure {
    pub attempt: usize,
    pub stage: String,
    pub message: String,
    /// Candidate context retained when generation succeeded but validation or
    /// evaluation failed. This lets later generations learn from malformed
    /// critic output instead of silently repeating the discarded mechanism.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub candidate_title: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub structural_text: Option<String>,
}

fn deserialize_string_or_vec<'de, D>(deserializer: D) -> Result<Vec<String>, D::Error>
where
    D: serde::Deserializer<'de>,
{
    #[derive(Deserialize)]
    #[serde(untagged)]
    enum StringOrVec {
        One(String),
        Many(Vec<String>),
    }

    Ok(match StringOrVec::deserialize(deserializer)? {
        StringOrVec::One(value) => vec![value],
        StringOrVec::Many(values) => values,
    })
}

const fn default_max_concurrency() -> usize {
    4
}
