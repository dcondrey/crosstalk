//! Versioned interchange contract for external evolutionary ideation engines.
//! BlindMind can consume/produce this JSON without entering Crosstalk's trusted core.

use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::collections::BTreeSet;

use crate::core::agent_trait::PromptAgent;
use async_trait::async_trait;
use crosstalk_evolution::{
    CandidateEvaluator, CandidateGenerator, Concept, ConceptDraft, EvolutionConfig,
    EvolutionEngine, EvolutionError, EvolutionState, Fitness, GenerationContext, GenerationReport,
    MutationOperator, ObjectiveFeedback, StructuralExclusion,
};

use crate::engines::objective_evaluation::{ObjectiveEvaluation, ReproductionOutcome};

pub const BLINDMIND_CONTRACT_VERSION: &str = "crosstalk.blindmind.v1";

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct IdeaSeed {
    pub id: String,
    pub domain: String,
    pub title: String,
    pub mechanism: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct EvolutionRequest {
    pub schema: String,
    pub project: String,
    pub directive: String,
    pub constraints: Vec<String>,
    pub evidence_ids: Vec<String>,
    #[serde(default)]
    pub evidence_context: Vec<EvidenceExcerpt>,
    pub seeds: Vec<IdeaSeed>,
    pub population_size: usize,
    pub generations: usize,
    #[serde(default = "default_evolution_concurrency")]
    pub max_concurrency: usize,
    /// Hard admission budget shared by variation and critic calls. Every
    /// candidate reserves two slots before concurrent execution. Zero means
    /// unlimited for backward compatibility.
    #[serde(default)]
    pub max_model_call_slots: u64,
    /// Previously eliminated mechanism families. These are hard negative
    /// knowledge, not suggestions for the variation model to paraphrase.
    #[serde(default)]
    pub structural_exclusions: Vec<StructuralExclusion>,
    /// Require an exact, falsifiable composition contract on every generated
    /// candidate. Legacy request JSON defaults this off; new requests enable it.
    #[serde(default)]
    pub require_executable_contract: bool,
    /// Require complete bounded probe source rather than an objective-test
    /// description that a later researcher would still have to implement.
    #[serde(default)]
    pub require_falsification_probe: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct EvidenceExcerpt {
    pub id: String,
    pub content_sha256: String,
    pub excerpt: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct EvolvedIdea {
    pub id: String,
    pub parent_ids: Vec<String>,
    pub mutation_type: String,
    pub domain: String,
    pub title: String,
    pub mechanism: String,
    /// Absent in an archive whose source has no such column. An empty vector
    /// therefore means "not recorded", never "measured, found none".
    #[serde(default)]
    pub predicted_measurements: Vec<String>,
    #[serde(default)]
    pub kill_criteria: Vec<String>,
    /// Generation within the exporting run. Added compatibly to version 1;
    /// responses produced by native evolution leave it at the seed value.
    #[serde(default)]
    pub generation: u32,
    #[serde(default)]
    pub tags: BTreeSet<String>,
    pub external_scores: BTreeMap<String, f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub executable_contract: Option<crosstalk_evolution::ExecutableContract>,
    /// Model fitness is never reported as objective verification.
    #[serde(default)]
    pub objectively_verified: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct EvolutionResponse {
    pub schema: String,
    pub project: String,
    /// Directive the exporting run last recorded. Added compatibly to version
    /// 1; empty means the source had none, and it is never invented.
    #[serde(default)]
    pub directive: String,
    pub ideas: Vec<EvolvedIdea>,
}

impl EvolutionRequest {
    #[must_use]
    pub fn new(
        project: impl Into<String>,
        directive: impl Into<String>,
        seeds: Vec<IdeaSeed>,
    ) -> Self {
        Self {
            schema: BLINDMIND_CONTRACT_VERSION.into(),
            project: project.into(),
            directive: directive.into(),
            constraints: vec![],
            evidence_ids: vec![],
            evidence_context: vec![],
            seeds,
            population_size: 12,
            generations: 3,
            max_concurrency: 4,
            max_model_call_slots: 0,
            structural_exclusions: vec![],
            require_executable_contract: true,
            require_falsification_probe: true,
        }
    }

    pub fn validate(&self) -> Result<(), String> {
        if self.schema != BLINDMIND_CONTRACT_VERSION {
            return Err(format!(
                "unsupported idea-evolution schema: {}",
                self.schema
            ));
        }
        if self.project.trim().is_empty() || self.directive.trim().is_empty() {
            return Err("project and directive must not be empty".into());
        }
        if self.population_size == 0 || self.population_size > 100 {
            return Err("population size must be between 1 and 100".into());
        }
        if self.generations == 0 || self.generations > 50 {
            return Err("generations must be between 1 and 50".into());
        }
        if self.max_concurrency == 0 || self.max_concurrency > 32 {
            return Err("evolution concurrency must be between 1 and 32".into());
        }
        if self.max_model_call_slots == 1 {
            return Err("evolution model-call-slot budget must be zero or at least two".into());
        }
        if self.require_falsification_probe && !self.require_executable_contract {
            return Err(
                "a required falsification probe also requires an executable contract".into(),
            );
        }
        if self.constraints.len() > 128
            || self
                .constraints
                .iter()
                .any(|constraint| constraint.trim().is_empty() || constraint.len() > 8_000)
        {
            return Err("evolution constraints must be non-empty and bounded".into());
        }
        if self.evidence_context.len() > 32
            || self
                .evidence_context
                .iter()
                .any(|evidence| evidence.excerpt.len() > 8_000)
        {
            return Err("evolution evidence exceeds context limits".into());
        }
        for exclusion in &self.structural_exclusions {
            exclusion.validate()?;
        }
        let mut exclusion_ids = BTreeSet::new();
        if self
            .structural_exclusions
            .iter()
            .any(|exclusion| !exclusion_ids.insert(exclusion.id.as_str()))
        {
            return Err("evolution structural exclusions require unique IDs".into());
        }
        let mut seed_ids = BTreeSet::new();
        if self.seeds.iter().any(|seed| {
            seed.id.trim().is_empty()
                || seed.title.trim().is_empty()
                || seed.mechanism.trim().is_empty()
                || !seed_ids.insert(seed.id.as_str())
        }) {
            return Err("evolution seeds require unique IDs, titles, and mechanisms".into());
        }
        if self.evidence_context.iter().any(|evidence| {
            evidence.id.trim().is_empty()
                || evidence.content_sha256.len() != 64
                || !evidence
                    .content_sha256
                    .bytes()
                    .all(|byte| byte.is_ascii_hexdigit())
        }) {
            return Err("evolution evidence requires an ID and SHA-256 digest".into());
        }
        Ok(())
    }
}

impl EvolutionResponse {
    pub fn validate(&self) -> Result<(), String> {
        if self.schema != BLINDMIND_CONTRACT_VERSION {
            return Err(format!(
                "unsupported idea-evolution schema: {}",
                self.schema
            ));
        }
        for idea in &self.ideas {
            if idea.id.trim().is_empty()
                || idea.title.trim().is_empty()
                || idea.mechanism.trim().is_empty()
            {
                return Err("evolved ideas require id, title, and mechanism".into());
            }
            if idea
                .external_scores
                .values()
                .any(|score| !score.is_finite() || !(0.0..=10.0).contains(score))
            {
                return Err(format!(
                    "idea {} contains an invalid external score",
                    idea.id
                ));
            }
        }
        Ok(())
    }
}

/// Upper bound on an imported archive so a hostile file cannot exhaust memory
/// before serde reports a shape error.
pub const BLINDMIND_ARCHIVE_MAX_BYTES: usize = 64 * 1024 * 1024;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct BlindmindImportSummary {
    pub project: String,
    pub ideas_in_file: usize,
    pub concepts_restored: usize,
    pub lineage_edges: usize,
    pub seeds: usize,
    pub non_seeds_without_parents: usize,
    pub max_generation: u32,
    pub ideas_with_predicted_measurements: usize,
    pub ideas_with_kill_criteria: usize,
    pub ideas_with_external_scores: usize,
    pub directive_present: bool,
}

fn parse_mutation_operator(raw: &str) -> Result<MutationOperator, String> {
    match raw {
        "Crossover" | "CROSSOVER" => Ok(MutationOperator::Crossover),
        "PointMutation" | "POINT_MUTATION" => Ok(MutationOperator::PointMutation),
        "Inversion" | "INVERSION" => Ok(MutationOperator::Inversion),
        "Wildcard" | "WILDCARD" => Ok(MutationOperator::Wildcard),
        other => Err(format!("unknown mutation type: {other}")),
    }
}

/// Rebuild a checkpoint from a `crosstalk.blindmind.v1` archive.
///
/// IMPORTANT: the six-axis `Fitness` has no source in a BlindMind archive,
/// which records one composite scalar. Every axis is therefore set to zero and
/// `next_directive` to empty: a zero is the absence of a measurement, whereas
/// spreading the composite across seven axes would assert seven judgements the
/// exporter never made. The scalar survives in the archive's `external_scores`
/// and is reported, not silently reinterpreted. An imported checkpoint is an
/// archive, not a resumable population.
pub fn import_blindmind_archive(
    json: &str,
) -> Result<(EvolutionState, BlindmindImportSummary), String> {
    if json.len() > BLINDMIND_ARCHIVE_MAX_BYTES {
        return Err("blindmind archive exceeds the maximum import size".into());
    }
    let response: EvolutionResponse = serde_json::from_str(json).map_err(|e| e.to_string())?;
    response.validate()?;
    if response.project.trim().is_empty() {
        return Err("blindmind archive has an empty project".into());
    }

    let known: BTreeSet<&str> = response.ideas.iter().map(|idea| idea.id.as_str()).collect();
    if known.len() != response.ideas.len() {
        return Err("blindmind archive contains duplicate idea IDs".into());
    }

    // Python never produced executable contracts or falsification probes, so
    // the legacy path stays off; both already default to false.
    let config = EvolutionConfig {
        population_size: response.ideas.len().clamp(1, 1_000),
        ..EvolutionConfig::default()
    };

    let mut state = EvolutionState::new(
        response.project.clone(),
        response.directive.clone(),
        0,
        config,
    );

    for idea in &response.ideas {
        let operator = parse_mutation_operator(&idea.mutation_type)?;
        let mut parents = BTreeSet::new();
        for parent in &idea.parent_ids {
            // Invariant 6: a checkpoint never carries a dangling population
            // reference, so a parent outside the archive is fatal, not pruned.
            if !known.contains(parent.as_str()) {
                return Err(format!(
                    "idea {} references a parent absent from the archive: {parent}",
                    idea.id
                ));
            }
            if !parents.insert(parent.as_str()) {
                return Err(format!("idea {} repeats a parent reference", idea.id));
            }
            state.lineage.push(crosstalk_evolution::LineageEdge {
                parent_id: parent.clone(),
                child_id: idea.id.clone(),
                operator,
            });
        }
        let concept = Concept {
            id: idea.id.clone(),
            project: response.project.clone(),
            generation: idea.generation,
            operator,
            parent_ids: idea.parent_ids.clone(),
            draft: ConceptDraft {
                domain: idea.domain.clone(),
                title: idea.title.clone(),
                mechanism: idea.mechanism.clone(),
                rationale: String::new(),
                predicted_measurements: idea.predicted_measurements.clone(),
                kill_criteria: idea.kill_criteria.clone(),
                tags: idea.tags.clone(),
                executable_contract: idea.executable_contract.clone(),
            },
            fitness: Fitness {
                novelty: 0.0,
                feasibility: 0.0,
                utility: 0.0,
                semantic_jump: 0.0,
                evidence: 0.0,
                safety: 0.0,
                prior_art_overlap: 0.0,
                fatal_flaws: vec![],
                next_directive: String::new(),
            },
        };
        if state.concepts.insert(idea.id.clone(), concept).is_some() {
            return Err(format!("blindmind archive repeats idea {}", idea.id));
        }
    }

    state.generation = response
        .ideas
        .iter()
        .map(|idea| idea.generation)
        .max()
        .unwrap_or(0);
    state.active_population = state.concepts.keys().cloned().collect();

    let summary = BlindmindImportSummary {
        project: response.project.clone(),
        ideas_in_file: response.ideas.len(),
        concepts_restored: state.concepts.len(),
        lineage_edges: state.lineage.len(),
        seeds: response
            .ideas
            .iter()
            .filter(|idea| idea.generation == 0)
            .count(),
        non_seeds_without_parents: response
            .ideas
            .iter()
            .filter(|idea| idea.generation > 0 && idea.parent_ids.is_empty())
            .count(),
        max_generation: state.generation,
        ideas_with_predicted_measurements: response
            .ideas
            .iter()
            .filter(|idea| !idea.predicted_measurements.is_empty())
            .count(),
        ideas_with_kill_criteria: response
            .ideas
            .iter()
            .filter(|idea| !idea.kill_criteria.is_empty())
            .count(),
        ideas_with_external_scores: response
            .ideas
            .iter()
            .filter(|idea| !idea.external_scores.is_empty())
            .count(),
        directive_present: !response.directive.trim().is_empty(),
    };

    // Every remaining checkpoint invariant is the reader's, not a copy of it.
    let checkpoint = state.checkpoint_json().map_err(|e| e.to_string())?;
    let state = EvolutionState::restore_json(&checkpoint)?;
    Ok((state, summary))
}

struct AgentGenerator<'a> {
    agent: &'a dyn PromptAgent,
    constraints: &'a [String],
    structural_exclusions: &'a [StructuralExclusion],
    evidence_ids: &'a [String],
    evidence_context: &'a [EvidenceExcerpt],
}

struct AgentEvaluator<'a> {
    agent: &'a dyn PromptAgent,
    evidence_context: &'a [EvidenceExcerpt],
    constraints: &'a [String],
    structural_exclusions: &'a [StructuralExclusion],
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct NativeEvolutionOutcome {
    pub response: EvolutionResponse,
    pub checkpoint_json: String,
    pub reports: Vec<GenerationReport>,
}

/// Commit an objective evaluator result into a resumable evolution checkpoint.
/// Model-authored scores can rank hypotheses, but a failed tool result lowers
/// feasibility/evidence and a failed hard constraint removes the concept from
/// the active population.
pub fn apply_objective_evaluation(
    checkpoint_json: &str,
    concept_id: &str,
    evaluation: &ObjectiveEvaluation,
    independently_reproduced: bool,
) -> Result<String, String> {
    let mut state = EvolutionState::restore_json(checkpoint_json)?;
    let hard_constraints_passed = evaluation
        .constraints
        .iter()
        .all(|constraint| constraint.passed);
    let feedback = ObjectiveFeedback {
        verification_id: evaluation.id.clone(),
        evaluator_id: evaluation.evaluator_id.clone(),
        evaluator_version: evaluation.evaluator_version.clone(),
        output_sha256: evaluation.raw_output_sha256.clone(),
        passed: evaluation.is_verified(),
        hard_constraints_passed,
        independently_reproduced: independently_reproduced && evaluation.is_verified(),
        measurements: evaluation
            .measurements
            .iter()
            .map(|measurement| (measurement.name.clone(), measurement.value))
            .collect(),
        recorded_at: evaluation.completed_at,
    };
    state.apply_objective_feedback(concept_id, feedback)?;
    state.checkpoint_json().map_err(|error| error.to_string())
}

/// Commit a primary result together with its independent reproduction. A
/// disagreement is treated as a failed objective result even if both runs
/// individually returned success.
pub fn apply_reproduction_outcome(
    checkpoint_json: &str,
    concept_id: &str,
    outcome: &ReproductionOutcome,
) -> Result<String, String> {
    let mut evaluation = outcome.primary.clone();
    if !outcome.agreed {
        evaluation.status = crate::types::investigation::VerificationStatus::Rejected;
        if !outcome.mismatches.is_empty() {
            evaluation.diagnostics = format!(
                "{}\nindependent reproduction disagreed: {}",
                evaluation.diagnostics,
                outcome.mismatches.join("; ")
            );
        }
    }
    apply_objective_evaluation(
        checkpoint_json,
        concept_id,
        &evaluation,
        outcome.reproduction.is_some() && outcome.agreed,
    )
}

#[async_trait]
impl CandidateGenerator for AgentGenerator<'_> {
    async fn generate(
        &self,
        operator: MutationOperator,
        parents: &[&Concept],
        context: &GenerationContext<'_>,
    ) -> Result<ConceptDraft, String> {
        let parents = parents
            .iter()
            .map(|parent| &parent.draft)
            .collect::<Vec<_>>();
        let prompt = format!(
            "You are the variation stage of a deterministic evolutionary discovery engine.\nOperator: {operator:?}\nDirective: {}\nHard constraints: {}\nEliminated structural families: {}\nEvidence IDs: {}\nBounded evidence excerpts: {}\nParents: {}\nReturn exactly one JSON object matching: {{\"domain\":string,\"title\":string,\"mechanism\":string,\"rationale\":string,\"predicted_measurements\":[string],\"kill_criteria\":[string],\"tags\":[string],\"executable_contract\":{{\"representation\":string,\"exact_relation\":string,\"composition_rule\":string,\"complexity_argument\":string,\"objective_test\":string,\"falsification_probe\":{{\"language\":string,\"source\":string,\"argv\":[string],\"timeout_seconds\":1..300,\"falsifies_when\":string}},\"distinguishes_from\":{{\"exclusion-id\":string}}}}}}. The contract and complete bounded probe source are mandatory. The argv field is an argument vector, not shell text. Give the missing identity/composition rule explicitly, cover every exclusion ID, and make the probe emit a machine-observable counterexample. Do not rename or paraphrase an eliminated family. Cite evidence IDs in the rationale; do not claim the excerpts establish more than they say.",
            context.directive,
            serde_json::to_string(self.constraints).map_err(|e| e.to_string())?,
            serde_json::to_string(self.structural_exclusions).map_err(|e| e.to_string())?,
            serde_json::to_string(self.evidence_ids).map_err(|e| e.to_string())?,
            serde_json::to_string(self.evidence_context).map_err(|e| e.to_string())?,
            serde_json::to_string(&parents).map_err(|e| e.to_string())?,
        );
        let response = self
            .agent
            .prompt(&prompt)
            .await
            .map_err(|e| e.to_string())?;
        parse_json_object(&response)
    }
}

#[async_trait]
impl CandidateEvaluator for AgentEvaluator<'_> {
    async fn evaluate(
        &self,
        draft: &ConceptDraft,
        parents: &[&Concept],
        context: &GenerationContext<'_>,
    ) -> Result<Fitness, String> {
        let parent_titles = parents
            .iter()
            .map(|parent| parent.draft.title.as_str())
            .collect::<Vec<_>>();
        let prompt = format!(
            "You are an adversarial evaluator, not the idea's advocate. Evaluate this candidate against logic, hard constraints, eliminated structures, evidence, safety, usefulness, and its parents.\nDirective: {}\nHard constraints: {}\nEliminated structural families: {}\nParent titles: {}\nEvidence excerpts: {}\nCandidate: {}\nReturn exactly one JSON object matching: {{\"novelty\":0..10,\"feasibility\":0..10,\"utility\":0..10,\"semantic_jump\":0..10,\"evidence\":0..10,\"safety\":0..10,\"prior_art_overlap\":0..10,\"fatal_flaws\":[string],\"next_directive\":string}}. Put any missing exact relation, missing composition rule, missing complete falsification source, violated hard constraint, or disguised eliminated family in fatal_flaws. Treat an unexecuted falsification_probe as a proposal, never as evidence. Inspect its source for vacuous tests and mismatched quantifiers. Do not inflate scores.",
            context.directive,
            serde_json::to_string(self.constraints).map_err(|e| e.to_string())?,
            serde_json::to_string(self.structural_exclusions).map_err(|e| e.to_string())?,
            serde_json::to_string(&parent_titles).map_err(|e| e.to_string())?,
            serde_json::to_string(self.evidence_context).map_err(|e| e.to_string())?,
            serde_json::to_string(draft).map_err(|e| e.to_string())?,
        );
        let response = self
            .agent
            .prompt(&prompt)
            .await
            .map_err(|e| e.to_string())?;
        parse_json_object(&response)
    }
}

/// Run the native Rust evolution engine using a Crosstalk model for variation
/// and adversarial evaluation. The engine itself remains provider-independent.
pub async fn run_native_evolution(
    agent: &dyn PromptAgent,
    request: &EvolutionRequest,
    seed: u64,
) -> Result<EvolutionResponse, String> {
    Ok(
        run_native_evolution_with_agents(agent, agent, request, seed)
            .await?
            .response,
    )
}

/// Run native evolution with independent variation and critic agents and
/// return the complete resumable state and per-generation reports.
pub async fn run_native_evolution_with_agents(
    variation_agent: &dyn PromptAgent,
    critic_agent: &dyn PromptAgent,
    request: &EvolutionRequest,
    seed: u64,
) -> Result<NativeEvolutionOutcome, String> {
    run_native_evolution_with_agents_reporting(variation_agent, critic_agent, request, seed, |_| {})
        .await
}

/// Run native evolution and publish a durable snapshot after each completed
/// generation. Callers that impose an outer timeout can retain the latest
/// snapshot instead of losing every completed generation when a later one is
/// cancelled.
pub async fn run_native_evolution_with_agents_reporting<F>(
    variation_agent: &dyn PromptAgent,
    critic_agent: &dyn PromptAgent,
    request: &EvolutionRequest,
    seed: u64,
    mut report_progress: F,
) -> Result<NativeEvolutionOutcome, String>
where
    F: FnMut(&NativeEvolutionOutcome),
{
    request.validate()?;
    let config = EvolutionConfig {
        population_size: request.population_size,
        max_concurrency: request.max_concurrency,
        max_model_call_slots: request.max_model_call_slots,
        require_executable_contract: request.require_executable_contract,
        require_falsification_probe: request.require_falsification_probe,
        ..EvolutionConfig::default()
    };
    let mut state = EvolutionState::new(&request.project, &request.directive, seed, config);
    state.structural_exclusions = request.structural_exclusions.clone();
    for seed_idea in &request.seeds {
        let concept = Concept {
            id: seed_idea.id.clone(),
            project: request.project.clone(),
            generation: 0,
            operator: MutationOperator::Wildcard,
            parent_ids: vec![],
            draft: ConceptDraft {
                domain: seed_idea.domain.clone(),
                title: seed_idea.title.clone(),
                mechanism: seed_idea.mechanism.clone(),
                rationale: "Imported seed".into(),
                predicted_measurements: vec![],
                kill_criteria: vec![],
                tags: BTreeSet::new(),
                executable_contract: None,
            },
            fitness: Fitness {
                novelty: 5.0,
                feasibility: 5.0,
                utility: 5.0,
                semantic_jump: 0.0,
                evidence: 0.0,
                safety: 5.0,
                prior_art_overlap: 5.0,
                fatal_flaws: vec![],
                next_directive: request.directive.clone(),
            },
        };
        state.active_population.push(concept.id.clone());
        state.concepts.insert(concept.id.clone(), concept);
    }
    let generator = AgentGenerator {
        agent: variation_agent,
        constraints: &request.constraints,
        structural_exclusions: &request.structural_exclusions,
        evidence_ids: &request.evidence_ids,
        evidence_context: &request.evidence_context,
    };
    let evaluator = AgentEvaluator {
        agent: critic_agent,
        evidence_context: &request.evidence_context,
        constraints: &request.constraints,
        structural_exclusions: &request.structural_exclusions,
    };
    let mut engine = EvolutionEngine::new(state, generator, evaluator);
    let mut reports = Vec::with_capacity(request.generations);
    let total_model_call_slots = engine.state.config.max_model_call_slots;
    for generation_index in 0..request.generations {
        if total_model_call_slots > 0 {
            let used = engine.state.usage.model_call_slots_reserved;
            let remaining = total_model_call_slots.saturating_sub(used);
            if remaining < 2 {
                break;
            }

            // Give every requested generation a chance to learn from the prior
            // rejection ledger. Without a per-generation ceiling, an all-rejected
            // first generation can consume the entire session budget while trying
            // to fill its accepted population, and the completed negative results
            // are then lost when the next generation returns BudgetExhausted.
            let generations_remaining = request.generations - generation_index;
            let fair_share = remaining / generations_remaining as u64;
            let allocated = fair_share.max(2).min(remaining);
            let generation_slots = allocated.saturating_sub(allocated % 2);
            if generation_slots < 2 {
                break;
            }
            engine.state.config.max_model_call_slots = used.saturating_add(generation_slots);
        }

        match engine.run_generation().await {
            Ok(report) => {
                reports.push(report);
                let generation_ceiling = engine.state.config.max_model_call_slots;
                engine.state.config.max_model_call_slots = total_model_call_slots;
                let progress = build_native_outcome(&engine.state, request, &reports)?;
                engine.state.config.max_model_call_slots = generation_ceiling;
                report_progress(&progress);
            }
            Err(EvolutionError::BudgetExhausted) => break,
            Err(error) => return Err(error.to_string()),
        }
    }
    // The temporary generation ceilings are scheduling policy, not checkpoint
    // configuration. Persist the request's actual session-wide hard limit.
    engine.state.config.max_model_call_slots = total_model_call_slots;
    build_native_outcome(&engine.state, request, &reports)
}

fn build_native_outcome(
    state: &EvolutionState,
    request: &EvolutionRequest,
    reports: &[GenerationReport],
) -> Result<NativeEvolutionOutcome, String> {
    let ideas = state
        .active_population
        .iter()
        .filter_map(|id| state.concepts.get(id))
        .map(|concept| EvolvedIdea {
            id: concept.id.clone(),
            parent_ids: concept.parent_ids.clone(),
            mutation_type: match concept.operator {
                MutationOperator::Crossover => "CROSSOVER",
                MutationOperator::PointMutation => "POINT_MUTATION",
                MutationOperator::Inversion => "INVERSION",
                MutationOperator::Wildcard => "WILDCARD",
            }
            .into(),
            domain: concept.draft.domain.clone(),
            title: concept.draft.title.clone(),
            mechanism: concept.draft.mechanism.clone(),
            predicted_measurements: concept.draft.predicted_measurements.clone(),
            kill_criteria: concept.draft.kill_criteria.clone(),
            generation: concept.generation,
            tags: concept.draft.tags.clone(),
            external_scores: BTreeMap::from([
                ("novelty".into(), concept.fitness.novelty),
                ("feasibility".into(), concept.fitness.feasibility),
                ("utility".into(), concept.fitness.utility),
                ("evidence".into(), concept.fitness.evidence),
                ("safety".into(), concept.fitness.safety),
            ]),
            executable_contract: concept.draft.executable_contract.clone(),
            objectively_verified: state.objective_feedback.get(&concept.id).is_some_and(
                |feedback| {
                    feedback
                        .iter()
                        .any(|record| record.passed && record.hard_constraints_passed)
                },
            ),
        })
        .collect();
    let response = EvolutionResponse {
        schema: BLINDMIND_CONTRACT_VERSION.into(),
        project: request.project.clone(),
        directive: state.directive.clone(),
        ideas,
    };
    let checkpoint_json = state.checkpoint_json().map_err(|e| e.to_string())?;
    Ok(NativeEvolutionOutcome {
        response,
        checkpoint_json,
        reports: reports.to_vec(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use futures::Stream;
    use rig::completion::PromptError;
    use std::future::Future;
    use std::pin::Pin;
    use std::sync::Arc;
    use std::sync::atomic::{AtomicUsize, Ordering};

    struct StaticAgent {
        response: String,
        calls: Arc<AtomicUsize>,
    }

    impl PromptAgent for StaticAgent {
        fn name(&self) -> &str {
            "static"
        }

        fn prompt<'a>(
            &'a self,
            _prompt: &'a str,
        ) -> Pin<Box<dyn Future<Output = Result<String, PromptError>> + Send + 'a>> {
            self.calls.fetch_add(1, Ordering::SeqCst);
            let response = self.response.clone();
            Box::pin(async move { Ok(response) })
        }

        fn stream_prompt<'a>(
            &'a self,
            _prompt: &'a str,
        ) -> Pin<
            Box<
                dyn Future<
                        Output = Result<
                            Pin<Box<dyn Stream<Item = Result<String, anyhow::Error>> + Send + 'a>>,
                            anyhow::Error,
                        >,
                    > + Send
                    + 'a,
            >,
        > {
            Box::pin(async move {
                Ok(Box::pin(futures::stream::empty())
                    as Pin<
                        Box<dyn Stream<Item = Result<String, anyhow::Error>> + Send>,
                    >)
            })
        }
    }

    #[tokio::test]
    async fn rejected_generation_preserves_budget_for_next_generation() {
        let variation_calls = Arc::new(AtomicUsize::new(0));
        let critic_calls = Arc::new(AtomicUsize::new(0));
        let variation = StaticAgent {
            response: serde_json::json!({
                "domain": "Mathematics",
                "title": "Rejected mechanism",
                "mechanism": "A concrete but incorrect recurrence",
                "rationale": "fixture",
                "predicted_measurements": ["exact agreement"],
                "kill_criteria": ["one mismatch"],
                "tags": [],
                "executable_contract": {
                    "representation": "one bit",
                    "exact_relation": "f(n)=0",
                    "composition_rule": "f(2n)=f(n)",
                    "complexity_argument": "one step per input bit",
                    "objective_test": "compare with ground truth",
                    "falsification_probe": {
                        "language": "python3",
                        "source": "print('counterexample')",
                        "argv": ["python3", "probe.py"],
                        "timeout_seconds": 5,
                        "falsifies_when": "stdout begins with counterexample"
                    },
                    "distinguishes_from": {}
                }
            })
            .to_string(),
            calls: Arc::clone(&variation_calls),
        };
        let critic = StaticAgent {
            response: serde_json::json!({
                "novelty": 1.0,
                "feasibility": 1.0,
                "utility": 1.0,
                "semantic_jump": 1.0,
                "evidence": 0.0,
                "safety": 8.0,
                "prior_art_overlap": 1.0,
                "fatal_flaws": ["the asserted recurrence is false"],
                "next_directive": "replace the recurrence"
            })
            .to_string(),
            calls: Arc::clone(&critic_calls),
        };
        let mut request = EvolutionRequest::new(
            "budget-fairness",
            "find a mechanism",
            vec![IdeaSeed {
                id: "seed".into(),
                domain: "Mathematics".into(),
                title: "Seed".into(),
                mechanism: "Initial framing".into(),
            }],
        );
        request.population_size = 3;
        request.generations = 2;
        request.max_concurrency = 3;
        request.max_model_call_slots = 12;

        let outcome = run_native_evolution_with_agents(&variation, &critic, &request, 1)
            .await
            .unwrap();

        assert_eq!(outcome.reports.len(), 2);
        assert_eq!(outcome.reports[0].attempts, 3);
        assert_eq!(outcome.reports[1].attempts, 3);
        assert_eq!(outcome.reports[1].usage.model_call_slots_reserved, 12);
        assert_eq!(variation_calls.load(Ordering::SeqCst), 6);
        assert_eq!(critic_calls.load(Ordering::SeqCst), 6);
        let checkpoint = EvolutionState::restore_json(&outcome.checkpoint_json).unwrap();
        assert_eq!(checkpoint.config.max_model_call_slots, 12);
        assert_eq!(checkpoint.rejected_candidates.len(), 6);
    }

    #[tokio::test]
    async fn reports_durable_progress_after_each_generation() {
        let variation_calls = Arc::new(AtomicUsize::new(0));
        let critic_calls = Arc::new(AtomicUsize::new(0));
        let variation = StaticAgent {
            response: serde_json::json!({
                "domain": "Mathematics",
                "title": "Rejected mechanism",
                "mechanism": "A concrete but incorrect recurrence",
                "rationale": "fixture",
                "predicted_measurements": ["exact agreement"],
                "kill_criteria": ["one mismatch"],
                "tags": [],
                "executable_contract": {
                    "representation": "one bit",
                    "exact_relation": "f(n)=0",
                    "composition_rule": "f(2n)=f(n)",
                    "complexity_argument": "one step per input bit",
                    "objective_test": "compare with ground truth",
                    "falsification_probe": {
                        "language": "python3",
                        "source": "print('counterexample')",
                        "argv": ["python3", "probe.py"],
                        "timeout_seconds": 5,
                        "falsifies_when": "stdout begins with counterexample"
                    },
                    "distinguishes_from": {}
                }
            })
            .to_string(),
            calls: variation_calls,
        };
        let critic = StaticAgent {
            response: serde_json::json!({
                "novelty": 1.0,
                "feasibility": 1.0,
                "utility": 1.0,
                "semantic_jump": 1.0,
                "evidence": 0.0,
                "safety": 8.0,
                "prior_art_overlap": 1.0,
                "fatal_flaws": ["the asserted recurrence is false"],
                "next_directive": "replace the recurrence"
            })
            .to_string(),
            calls: critic_calls,
        };
        let mut request = EvolutionRequest::new(
            "progress",
            "find a mechanism",
            vec![IdeaSeed {
                id: "seed".into(),
                domain: "Mathematics".into(),
                title: "Seed".into(),
                mechanism: "Initial framing".into(),
            }],
        );
        request.population_size = 1;
        request.generations = 2;
        request.max_model_call_slots = 4;

        let mut completed = Vec::new();
        let outcome = run_native_evolution_with_agents_reporting(
            &variation,
            &critic,
            &request,
            1,
            |progress| completed.push(progress.clone()),
        )
        .await
        .unwrap();

        assert_eq!(completed.len(), 2);
        assert_eq!(completed[0].reports.len(), 1);
        assert_eq!(completed[1].reports.len(), 2);
        assert_eq!(completed[0].reports[0].usage.generations_completed, 1);
        assert_eq!(outcome, completed[1]);
        let first_checkpoint = EvolutionState::restore_json(&completed[0].checkpoint_json).unwrap();
        assert_eq!(first_checkpoint.config.max_model_call_slots, 4);
    }
}

const fn default_evolution_concurrency() -> usize {
    4
}

fn parse_json_object<T: serde::de::DeserializeOwned>(text: &str) -> Result<T, String> {
    let start = text
        .find('{')
        .ok_or_else(|| "model response contained no JSON object".to_string())?;
    let end = text
        .rfind('}')
        .ok_or_else(|| "model response contained an incomplete JSON object".to_string())?;
    serde_json::from_str(&text[start..=end]).map_err(|error| format!("invalid model JSON: {error}"))
}
