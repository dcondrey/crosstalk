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
    #[error("evolution model-call-slot budget is exhausted")]
    BudgetExhausted,
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
        if self.state.config.max_model_call_slots > 0
            && self
                .state
                .config
                .max_model_call_slots
                .saturating_sub(self.state.usage.model_call_slots_reserved)
                < 2
        {
            return Err(EvolutionError::BudgetExhausted);
        }
        let target = self.state.config.population_size;
        let max_attempts = target.saturating_mul(self.state.config.max_attempt_multiplier.max(1));
        let next_generation = self.state.generation + 1;
        let mut accepted = Vec::new();
        let mut rejected = 0;
        let mut rejection_reasons = std::collections::BTreeMap::new();
        let mut failures = Vec::new();
        let mut attempt = 0;
        let mut budget_exhausted = false;
        while attempt < max_attempts && accepted.len() < target {
            let concurrency = self.state.config.max_concurrency.max(1);
            let budget_capacity = if self.state.config.max_model_call_slots == 0 {
                usize::MAX
            } else {
                usize::try_from(
                    self.state
                        .config
                        .max_model_call_slots
                        .saturating_sub(self.state.usage.model_call_slots_reserved)
                        / 2,
                )
                .unwrap_or(usize::MAX)
            };
            if budget_capacity == 0 {
                budget_exhausted = true;
                break;
            }
            let batch_size = concurrency
                .min(max_attempts.saturating_sub(attempt))
                .min(budget_capacity);
            let batch_end = attempt + batch_size;
            self.state.usage.attempts_started = self
                .state
                .usage
                .attempts_started
                .saturating_add(batch_size as u64);
            self.state.usage.model_call_slots_reserved = self
                .state
                .usage
                .model_call_slots_reserved
                .saturating_add((batch_size as u64).saturating_mul(2));
            let batch = join_all((attempt..batch_end).map(|candidate_attempt| {
                self.evaluate_attempt(next_generation, candidate_attempt)
            }))
            .await;
            for candidate in batch {
                let candidate = match candidate {
                    Ok(candidate) => candidate,
                    Err(failure) => {
                        self.remember_attempt_failure(&failure, next_generation);
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
                if self.is_structural_duplicate(&draft, &accepted) {
                    let score = blindmind_compatible_score(&fitness);
                    self.remember_rejection(
                        &draft,
                        next_generation,
                        candidate_attempt,
                        "duplicate_or_excluded_structure",
                        score,
                        &fitness.fatal_flaws,
                    );
                    rejected += 1;
                    *rejection_reasons
                        .entry("duplicate_or_excluded_structure".to_string())
                        .or_insert(0) += 1;
                    continue;
                }
                let score = blindmind_compatible_score(&fitness);
                let rejection_reason = if score < self.state.config.retention_threshold {
                    Some("score_below_threshold")
                } else if !fitness.passes_hard_gates(&draft, self.state.config.permissive) {
                    Some("model_hard_gate")
                } else if self.state.config.require_executable_contract
                    && draft.executable_contract.is_none()
                {
                    Some("missing_executable_contract")
                } else if self.state.config.require_falsification_probe
                    && draft
                        .executable_contract
                        .as_ref()
                        .and_then(|contract| contract.falsification_probe.as_ref())
                        .is_none()
                {
                    Some("missing_falsification_probe")
                } else if !self.covers_structural_exclusions(&draft) {
                    Some("exclusion_not_distinguished")
                } else {
                    None
                };
                if let Some(reason) = rejection_reason {
                    self.remember_rejection(
                        &draft,
                        next_generation,
                        candidate_attempt,
                        reason,
                        score,
                        &fitness.fatal_flaws,
                    );
                    rejected += 1;
                    *rejection_reasons.entry(reason.to_string()).or_insert(0) += 1;
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
        self.state.usage.generations_completed =
            self.state.usage.generations_completed.saturating_add(1);
        self.state.usage.failed_attempts = self
            .state
            .usage
            .failed_attempts
            .saturating_add(failures.len() as u64);
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
        if self.state.directive.is_empty() {
            self.state.directive = self.rejection_directive();
        }
        Ok(GenerationReport {
            generation: next_generation,
            attempts: attempt,
            accepted,
            rejected,
            rejection_reasons,
            failures,
            pareto_frontier: frontier,
            usage: self.state.usage.clone(),
            budget_exhausted,
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
                    candidate_title: None,
                    structural_text: None,
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
                candidate_title: None,
                structural_text: None,
            })?;
        draft.validate().map_err(|message| crate::AttemptFailure {
            attempt,
            stage: "draft_validation".into(),
            message,
            candidate_title: Some(draft.title.clone()),
            structural_text: Some(draft.structural_text()),
        })?;
        let fitness = self
            .evaluator
            .evaluate(&draft, &parents, &context)
            .await
            .map_err(|message| crate::AttemptFailure {
                attempt,
                stage: "evaluation".into(),
                message,
                candidate_title: Some(draft.title.clone()),
                structural_text: Some(draft.structural_text()),
            })?;
        fitness
            .validate()
            .map_err(|message| crate::AttemptFailure {
                attempt,
                stage: "fitness_validation".into(),
                message,
                candidate_title: Some(draft.title.clone()),
                structural_text: Some(draft.structural_text()),
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

    fn is_structural_duplicate(&self, draft: &ConceptDraft, accepted: &[String]) -> bool {
        let structural_text = draft
            .executable_contract
            .as_ref()
            .map(crate::ExecutableContract::structural_text);
        let excluded = structural_text.as_ref().is_some_and(|structural_text| {
            self.state.structural_exclusions.iter().any(|exclusion| {
                structural_similarity(structural_text, &exclusion.structural_text()) > 0.72
            })
        });
        let rejected_duplicate = self
            .state
            .rejected_titles
            .iter()
            .rev()
            .take(self.state.config.rejection_memory)
            .any(|prior| title_similarity(&draft.title, prior) > 0.7)
            || self
                .state
                .rejected_structures
                .iter()
                .rev()
                .take(self.state.config.rejection_memory)
                .any(|prior| {
                    structural_text.as_ref().is_some_and(|structural_text| {
                        structural_similarity(structural_text, prior) > 0.72
                    })
                });
        excluded
            || rejected_duplicate
            || accepted
                .iter()
                .filter_map(|id| self.state.concepts.get(id))
                .any(|concept| {
                    title_similarity(&draft.title, &concept.draft.title) > 0.7
                        || structural_text.as_ref().is_some_and(|structural_text| {
                            concept
                                .draft
                                .executable_contract
                                .as_ref()
                                .is_some_and(|contract| {
                                    structural_similarity(
                                        structural_text,
                                        &contract.structural_text(),
                                    ) > 0.72
                                })
                        })
                })
    }

    fn covers_structural_exclusions(&self, draft: &ConceptDraft) -> bool {
        if self.state.structural_exclusions.is_empty() {
            return true;
        }
        let Some(contract) = &draft.executable_contract else {
            return false;
        };
        self.state.structural_exclusions.iter().all(|exclusion| {
            contract
                .distinguishes_from
                .get(&exclusion.id)
                .is_some_and(|difference| !difference.trim().is_empty())
        })
    }
    fn remember_rejection(
        &mut self,
        draft: &ConceptDraft,
        generation: u32,
        attempt: usize,
        reason: &str,
        score: f64,
        fatal_flaws: &[String],
    ) {
        self.state.rejected_titles.push(draft.title.clone());
        if let Some(contract) = &draft.executable_contract {
            self.state
                .rejected_structures
                .push(contract.structural_text());
        }
        let excess = self
            .state
            .rejected_titles
            .len()
            .saturating_sub(self.state.config.rejection_memory);
        if excess > 0 {
            self.state.rejected_titles.drain(..excess);
        }
        let excess = self
            .state
            .rejected_structures
            .len()
            .saturating_sub(self.state.config.rejection_memory);
        if excess > 0 {
            self.state.rejected_structures.drain(..excess);
        }
        self.state
            .rejected_candidates
            .push(crate::RejectedCandidate {
                generation,
                attempt,
                title: draft.title.clone(),
                structural_text: draft.structural_text(),
                reason: reason.to_string(),
                score,
                fatal_flaws: fatal_flaws.to_vec(),
            });
        let excess = self
            .state
            .rejected_candidates
            .len()
            .saturating_sub(self.state.config.rejection_memory);
        if excess > 0 {
            self.state.rejected_candidates.drain(..excess);
        }
    }

    fn remember_attempt_failure(&mut self, failure: &crate::AttemptFailure, generation: u32) {
        let Some(title) = failure
            .candidate_title
            .as_ref()
            .filter(|title| !title.trim().is_empty())
        else {
            return;
        };
        self.state.rejected_titles.push(title.clone());
        if let Some(structural_text) = failure
            .structural_text
            .as_ref()
            .filter(|text| !text.trim().is_empty())
        {
            self.state.rejected_structures.push(structural_text.clone());
        }
        self.state
            .rejected_candidates
            .push(crate::RejectedCandidate {
                generation,
                attempt: failure.attempt,
                title: title.clone(),
                structural_text: failure.structural_text.clone().unwrap_or_default(),
                reason: format!("{}_failure", failure.stage),
                score: 0.0,
                fatal_flaws: vec![failure.message.clone()],
            });
        for collection in [
            &mut self.state.rejected_titles,
            &mut self.state.rejected_structures,
        ] {
            let excess = collection
                .len()
                .saturating_sub(self.state.config.rejection_memory);
            if excess > 0 {
                collection.drain(..excess);
            }
        }
        let excess = self
            .state
            .rejected_candidates
            .len()
            .saturating_sub(self.state.config.rejection_memory);
        if excess > 0 {
            self.state.rejected_candidates.drain(..excess);
        }
    }

    fn rejection_directive(&self) -> String {
        let lessons = self
            .state
            .rejected_candidates
            .iter()
            .rev()
            .take(3)
            .map(|candidate| {
                let flaw = candidate
                    .fatal_flaws
                    .first()
                    .map(String::as_str)
                    .unwrap_or(candidate.reason.as_str());
                format!("Do not repeat '{}': {}.", candidate.title, flaw)
            })
            .collect::<Vec<_>>()
            .join(" ");
        if lessons.is_empty() {
            self.state.directive.clone()
        } else {
            format!(
                "{} Produce a structurally different contract that resolves these failures.",
                lessons
            )
        }
    }
}

fn structural_similarity(left: &str, right: &str) -> f64 {
    const STOP_WORDS: &[&str] = &[
        "a", "an", "and", "as", "at", "by", "for", "from", "in", "is", "of", "on", "or", "that",
        "the", "to", "using", "with",
    ];
    let terms = |text: &str| {
        text.split(|character: char| !character.is_alphanumeric())
            .map(str::to_ascii_lowercase)
            .filter(|term| term.len() > 1 && !STOP_WORDS.contains(&term.as_str()))
            .collect::<std::collections::BTreeSet<_>>()
    };
    let left = terms(left);
    let right = terms(right);
    if left.is_empty() || right.is_empty() {
        return 0.0;
    }
    let intersection = left.intersection(&right).count() as f64;
    let union = left.union(&right).count() as f64;
    intersection / union
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
