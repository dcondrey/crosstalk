//! Versioned interchange contract for external evolutionary ideation engines.
//! BlindMind can consume/produce this JSON without entering Crosstalk's trusted core.

use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::collections::BTreeSet;

use crate::core::agent_trait::PromptAgent;
use async_trait::async_trait;
use crosstalk_evolution::{
    CandidateEvaluator, CandidateGenerator, Concept, ConceptDraft, EvolutionConfig,
    EvolutionEngine, EvolutionState, Fitness, GenerationContext, GenerationReport,
    MutationOperator, ObjectiveFeedback,
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
    pub predicted_measurements: Vec<String>,
    pub kill_criteria: Vec<String>,
    pub external_scores: BTreeMap<String, f64>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct EvolutionResponse {
    pub schema: String,
    pub project: String,
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
        if self.evidence_context.len() > 32
            || self
                .evidence_context
                .iter()
                .any(|evidence| evidence.excerpt.len() > 8_000)
        {
            return Err("evolution evidence exceeds context limits".into());
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

struct AgentGenerator<'a> {
    agent: &'a dyn PromptAgent,
    constraints: &'a [String],
    evidence_ids: &'a [String],
    evidence_context: &'a [EvidenceExcerpt],
}

struct AgentEvaluator<'a> {
    agent: &'a dyn PromptAgent,
    evidence_context: &'a [EvidenceExcerpt],
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
            "You are the variation stage of a deterministic evolutionary discovery engine.\nOperator: {operator:?}\nDirective: {}\nConstraints: {}\nEvidence IDs: {}\nBounded evidence excerpts: {}\nParents: {}\nReturn exactly one JSON object matching: {{\"domain\":string,\"title\":string,\"mechanism\":string,\"rationale\":string,\"predicted_measurements\":[string],\"kill_criteria\":[string],\"tags\":[string]}}. A mechanism, measurable predictions, and falsifying kill criteria are mandatory. Cite evidence IDs in the rationale; do not claim the excerpts establish more than they say.",
            context.directive,
            serde_json::to_string(self.constraints).map_err(|e| e.to_string())?,
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
            "You are an adversarial evaluator, not the idea's advocate. Evaluate this candidate against physics/logic, prior art risk, evidence, safety, usefulness, and its parents.\nDirective: {}\nParent titles: {}\nEvidence excerpts: {}\nCandidate: {}\nReturn exactly one JSON object matching: {{\"novelty\":0..10,\"feasibility\":0..10,\"utility\":0..10,\"semantic_jump\":0..10,\"evidence\":0..10,\"safety\":0..10,\"prior_art_overlap\":0..10,\"fatal_flaws\":[string],\"next_directive\":string}}. Treat unsupported novelty and evidence claims skeptically. Do not inflate scores.",
            context.directive,
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
    request.validate()?;
    let config = EvolutionConfig {
        population_size: request.population_size,
        max_concurrency: request.max_concurrency,
        ..EvolutionConfig::default()
    };
    let mut state = EvolutionState::new(&request.project, &request.directive, seed, config);
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
        evidence_ids: &request.evidence_ids,
        evidence_context: &request.evidence_context,
    };
    let evaluator = AgentEvaluator {
        agent: critic_agent,
        evidence_context: &request.evidence_context,
    };
    let mut engine = EvolutionEngine::new(state, generator, evaluator);
    let mut reports = Vec::with_capacity(request.generations);
    for _ in 0..request.generations {
        reports.push(engine.run_generation().await.map_err(|e| e.to_string())?);
    }
    let ideas = engine
        .state
        .active_population
        .iter()
        .filter_map(|id| engine.state.concepts.get(id))
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
            external_scores: BTreeMap::from([
                ("novelty".into(), concept.fitness.novelty),
                ("feasibility".into(), concept.fitness.feasibility),
                ("utility".into(), concept.fitness.utility),
                ("evidence".into(), concept.fitness.evidence),
                ("safety".into(), concept.fitness.safety),
            ]),
        })
        .collect();
    let response = EvolutionResponse {
        schema: BLINDMIND_CONTRACT_VERSION.into(),
        project: request.project.clone(),
        ideas,
    };
    let checkpoint_json = engine.state.checkpoint_json().map_err(|e| e.to_string())?;
    Ok(NativeEvolutionOutcome {
        response,
        checkpoint_json,
        reports,
    })
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
