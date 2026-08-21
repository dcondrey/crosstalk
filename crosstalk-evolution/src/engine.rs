use crate::{
    Concept, ConceptDraft, EvolutionState, Fitness, GenerationContext, GenerationReport,
    LineageEdge, MutationOperator, blindmind_compatible_score, pareto_frontier, title_similarity,
};
use async_trait::async_trait;
use futures::future::join_all;
use sha2::{Digest, Sha256};
use thiserror::Error;

#[derive(Debug, Error)]
pub enum EvolutionError {
    #[error("candidate generation failed: {0}")]
    Generation(String),
    #[error("candidate evaluation failed: {0}")]
    Evaluation(String),
    #[error("invalid fitness: {0}")]
    InvalidFitness(String),
    #[error("evolution configuration has no operator weight")]
    NoOperators,
}

#[async_trait]
pub trait CandidateGenerator: Send + Sync {
    async fn generate(
        &self,
        operator: MutationOperator,
        parents: &[&Concept],
        context: &GenerationContext<'_>,
    ) -> Result<ConceptDraft, String>;
}

#[async_trait]
pub trait CandidateEvaluator: Send + Sync {
    async fn evaluate(
        &self,
        draft: &ConceptDraft,
        parents: &[&Concept],
        context: &GenerationContext<'_>,
    ) -> Result<Fitness, String>;
}

pub struct EvolutionEngine<G, E> {
    pub state: EvolutionState,
    generator: G,
    evaluator: E,
}

impl<G: CandidateGenerator, E: CandidateEvaluator> EvolutionEngine<G, E> {
    #[must_use]
    pub fn new(state: EvolutionState, generator: G, evaluator: E) -> Self {
        Self {
            state,
            generator,
            evaluator,
        }
    }

    pub async fn run_generation(&mut self) -> Result<GenerationReport, EvolutionError> {
        let target = self.state.config.population_size;
        let max_attempts = target.saturating_mul(self.state.config.max_attempt_multiplier.max(1));
        let next_generation = self.state.generation + 1;
        let mut accepted = Vec::new();
        let mut rejected = 0;
        let mut failures = Vec::new();
        let mut attempt = 0;
        while attempt < max_attempts && accepted.len() < target {
            let batch_end = (attempt + self.state.config.max_concurrency.max(1)).min(max_attempts);
            let batch = join_all((attempt..batch_end).map(|candidate_attempt| {
                self.evaluate_attempt(next_generation, candidate_attempt)
            }))
            .await;
            for candidate in batch {
                let candidate = match candidate {
                    Ok(candidate) => candidate,
                    Err(failure) => {
                        failures.push(failure);
                        continue;
                    }
                };
                if accepted.len() >= target {
                    break;
                }
                let AttemptCandidate {
                    attempt: candidate_attempt,
                    operator,
                    parent_ids,
                    draft,
                    fitness,
                } = candidate;
                if self.is_duplicate(&draft.title, &accepted) {
                    rejected += 1;
                    continue;
                }
                let score = blindmind_compatible_score(&fitness);
                if score < self.state.config.retention_threshold
                    || !fitness.passes_hard_gates(&draft, self.state.config.permissive)
                {
                    self.remember_rejection(draft.title);
                    rejected += 1;
                    continue;
                }
                let id = concept_id(
                    &self.state.project,
                    next_generation,
                    candidate_attempt,
                    &draft,
                );
                let concept = Concept {
                    id: id.clone(),
                    project: self.state.project.clone(),
                    generation: next_generation,
                    operator,
                    parent_ids: parent_ids.clone(),
                    draft,
                    fitness,
                };
                for parent_id in parent_ids {
                    self.state.lineage.push(LineageEdge {
                        parent_id,
                        child_id: id.clone(),
                        operator,
                    });
                }
                self.state.concepts.insert(id.clone(), concept);
                accepted.push(id);
            }
            attempt = batch_end;
        }
        self.state.generation = next_generation;
        let frontier =
            pareto_frontier(accepted.iter().filter_map(|id| self.state.concepts.get(id)))
                .into_iter()
                .map(|c| c.id.clone())
                .collect::<Vec<_>>();
        self.state.active_population = if frontier.is_empty() {
            accepted.clone()
        } else {
            frontier.clone()
        };
        let directives = self
            .state
            .active_population
            .iter()
            .filter_map(|id| self.state.concepts.get(id))
            .map(|c| {
                (
                    c.fitness.next_directive.clone(),
                    blindmind_compatible_score(&c.fitness),
                )
            })
            .collect();
        self.state.directive = synthesize_directives(directives);
        Ok(GenerationReport {
            generation: next_generation,
            attempts: attempt,
            accepted,
            rejected,
            failures,
            pareto_frontier: frontier,
        })
    }

    async fn evaluate_attempt(
        &self,
        generation: u32,
        attempt: usize,
    ) -> Result<AttemptCandidate, crate::AttemptFailure> {
        let operator =
            self.operator_for(generation, attempt)
                .map_err(|error| crate::AttemptFailure {
                    attempt,
                    stage: "policy".into(),
                    message: error.to_string(),
                })?;
        let parent_ids = self.parents_for(operator, generation, attempt);
        let parents: Vec<&Concept> = parent_ids
            .iter()
            .filter_map(|id| self.state.concepts.get(id))
            .collect();
        let context = GenerationContext {
            project: &self.state.project,
            directive: &self.state.directive,
            generation,
            attempt,
        };
        let draft = self
            .generator
            .generate(operator, &parents, &context)
            .await
            .map_err(|message| crate::AttemptFailure {
                attempt,
                stage: "generation".into(),
                message,
            })?;
        draft.validate().map_err(|message| crate::AttemptFailure {
            attempt,
            stage: "validation".into(),
            message,
        })?;
        let fitness = self
            .evaluator
            .evaluate(&draft, &parents, &context)
            .await
            .map_err(|message| crate::AttemptFailure {
                attempt,
                stage: "evaluation".into(),
                message,
            })?;
        fitness
            .validate()
            .map_err(|message| crate::AttemptFailure {
                attempt,
                stage: "validation".into(),
                message,
            })?;
        Ok(AttemptCandidate {
            attempt,
            operator,
            parent_ids,
            draft,
            fitness,
        })
    }

    fn operator_for(
        &self,
        generation: u32,
        attempt: usize,
    ) -> Result<MutationOperator, EvolutionError> {
        let weights = &self.state.config;
        let crossover = u64::from(weights.crossover_weight);
        let mutation_weight = u64::from(weights.mutation_weight);
        let inversion_weight = u64::from(weights.inversion_weight);
        let wildcard_weight = u64::from(weights.wildcard_weight);
        let total = crossover + mutation_weight + inversion_weight + wildcard_weight;
        if total == 0 {
            return Err(EvolutionError::NoOperators);
        }
        let roll =
            deterministic_u64(self.state.seed, generation, attempt as u64, b"operator") % total;
        let mutation = crossover + mutation_weight;
        let inversion = mutation + inversion_weight;
        Ok(if roll < crossover {
            MutationOperator::Crossover
        } else if roll < mutation {
            MutationOperator::PointMutation
        } else if roll < inversion {
            MutationOperator::Inversion
        } else {
            MutationOperator::Wildcard
        })
    }

    fn parents_for(
        &self,
        operator: MutationOperator,
        generation: u32,
        attempt: usize,
    ) -> Vec<String> {
        let count = match operator {
            MutationOperator::Crossover => 2,
            MutationOperator::Wildcard => 0,
            _ => 1,
        };
        let pool = if self.state.active_population.is_empty() {
            self.state.concepts.keys().cloned().collect::<Vec<_>>()
        } else {
            self.state.active_population.clone()
        };
        if pool.is_empty() || count == 0 {
            return vec![];
        }
        let start = deterministic_u64(self.state.seed, generation, attempt as u64, b"parents")
            as usize
            % pool.len();
        (0..count.min(pool.len()))
            .map(|offset| pool[(start + offset) % pool.len()].clone())
            .collect()
    }

    fn is_duplicate(&self, title: &str, accepted: &[String]) -> bool {
        let rejected_duplicate = self
            .state
            .rejected_titles
            .iter()
            .rev()
            .take(self.state.config.rejection_memory)
            .any(|prior| title_similarity(title, prior) > 0.7);
        rejected_duplicate
            || accepted
                .iter()
                .filter_map(|id| self.state.concepts.get(id))
                .any(|concept| title_similarity(title, &concept.draft.title) > 0.7)
    }
    fn remember_rejection(&mut self, title: String) {
        self.state.rejected_titles.push(title);
        let excess = self
            .state
            .rejected_titles
            .len()
            .saturating_sub(self.state.config.rejection_memory);
        if excess > 0 {
            self.state.rejected_titles.drain(..excess);
        }
    }
}

struct AttemptCandidate {
    attempt: usize,
    operator: MutationOperator,
    parent_ids: Vec<String>,
    draft: ConceptDraft,
    fitness: Fitness,
}

fn deterministic_u64(seed: u64, generation: u32, attempt: u64, domain: &[u8]) -> u64 {
    let mut hash = Sha256::new();
    hash.update(seed.to_le_bytes());
    hash.update(generation.to_le_bytes());
    hash.update(attempt.to_le_bytes());
    hash.update(domain);
    let bytes = hash.finalize();
    u64::from_le_bytes(bytes[..8].try_into().expect("fixed digest length"))
}

fn concept_id(project: &str, generation: u32, attempt: usize, draft: &ConceptDraft) -> String {
    let mut hash = Sha256::new();
    hash.update(project.as_bytes());
    hash.update(generation.to_le_bytes());
    hash.update(attempt.to_le_bytes());
    hash.update(draft.title.as_bytes());
    hash.update(draft.mechanism.as_bytes());
    format!("{:x}", hash.finalize())[..24].to_string()
}

fn synthesize_directives(mut directives: Vec<(String, f64)>) -> String {
    directives.retain(|(directive, _)| !directive.trim().is_empty());
    directives.sort_by(|a, b| b.1.total_cmp(&a.1));
    directives
        .into_iter()
        .take(3)
        .map(|(d, _)| d)
        .collect::<Vec<_>>()
        .join(" ")
}
