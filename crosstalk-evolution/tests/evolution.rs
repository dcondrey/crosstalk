use async_trait::async_trait;
use crosstalk_evolution::*;
use std::collections::{BTreeMap, BTreeSet};
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::Duration;

#[derive(serde::Deserialize)]
struct CompatibilityCase {
    name: String,
    novelty: f64,
    feasibility: f64,
    utility: f64,
    semantic_jump: f64,
    prior_art_overlap: f64,
    fatal_flaws: Vec<String>,
    expected: f64,
}

struct Generator;

#[async_trait]
impl CandidateGenerator for Generator {
    async fn generate(
        &self,
        operator: MutationOperator,
        _parents: &[&Concept],
        context: &GenerationContext<'_>,
    ) -> Result<ConceptDraft, String> {
        Ok(ConceptDraft {
            domain: "Computer Science".into(),
            title: format!("Candidate {} {operator:?}", context.attempt),
            mechanism: "A measurable adaptive routing mechanism".into(),
            rationale: "Generated fixture".into(),
            predicted_measurements: vec!["reduces held-out error".into()],
            kill_criteria: vec!["no improvement over preregistered baseline".into()],
            tags: BTreeSet::new(),
            executable_contract: None,
        })
    }
}

struct Evaluator;

#[async_trait]
impl CandidateEvaluator for Evaluator {
    async fn evaluate(
        &self,
        _draft: &ConceptDraft,
        _parents: &[&Concept],
        context: &GenerationContext<'_>,
    ) -> Result<Fitness, String> {
        Ok(Fitness {
            novelty: 9.0,
            feasibility: 9.0,
            utility: 9.0,
            semantic_jump: 8.0,
            evidence: 7.0,
            safety: 8.0,
            prior_art_overlap: 1.0,
            fatal_flaws: vec![],
            next_directive: format!("refine generation {}", context.generation),
        })
    }
}

struct FailingEvaluator;

#[async_trait]
impl CandidateEvaluator for FailingEvaluator {
    async fn evaluate(
        &self,
        _draft: &ConceptDraft,
        _parents: &[&Concept],
        _context: &GenerationContext<'_>,
    ) -> Result<Fitness, String> {
        Err("critic returned malformed JSON".into())
    }
}

#[tokio::test]
async fn generation_is_deterministic_and_checkpointable() {
    let config = EvolutionConfig {
        population_size: 4,
        ..Default::default()
    };
    let state = EvolutionState::new("p", "invent", 42, config);
    let mut a = EvolutionEngine::new(state.clone(), Generator, Evaluator);
    let mut b = EvolutionEngine::new(state, Generator, Evaluator);
    let report_a = a.run_generation().await.unwrap();
    let report_b = b.run_generation().await.unwrap();
    assert_eq!(report_a, report_b);
    assert_eq!(a.state, b.state);
    assert_eq!(a.state.active_population.len(), 4);
    let restored = EvolutionState::restore_json(&a.state.checkpoint_json().unwrap()).unwrap();
    assert_eq!(restored, a.state);
}

#[tokio::test]
async fn model_call_slots_are_reserved_before_parallel_evolution() {
    let config = EvolutionConfig {
        population_size: 4,
        max_concurrency: 4,
        max_model_call_slots: 4,
        ..Default::default()
    };
    let state = EvolutionState::new("budgeted", "invent", 42, config);
    let mut engine = EvolutionEngine::new(state, Generator, Evaluator);
    let report = engine.run_generation().await.unwrap();
    assert_eq!(report.attempts, 2);
    assert_eq!(report.accepted.len(), 2);
    assert!(report.budget_exhausted);
    assert_eq!(report.usage.attempts_started, 2);
    assert_eq!(report.usage.model_call_slots_reserved, 4);
    assert_eq!(report.usage.generations_completed, 1);
    assert!(matches!(
        engine.run_generation().await,
        Err(crosstalk_evolution::EvolutionError::BudgetExhausted)
    ));
    let restored = EvolutionState::restore_json(&engine.state.checkpoint_json().unwrap()).unwrap();
    assert_eq!(restored.usage, report.usage);
}

#[tokio::test]
async fn evaluator_failures_are_retained_as_negative_knowledge() {
    let config = EvolutionConfig {
        population_size: 1,
        max_model_call_slots: 2,
        ..Default::default()
    };
    let state = EvolutionState::new("failure-ledger", "invent", 42, config);
    let mut engine = EvolutionEngine::new(state, Generator, FailingEvaluator);

    let report = engine.run_generation().await.unwrap();

    assert_eq!(report.failures.len(), 1);
    assert!(
        report.failures[0]
            .candidate_title
            .as_deref()
            .is_some_and(|title| title.starts_with("Candidate 0 "))
    );
    assert_eq!(engine.state.rejected_candidates.len(), 1);
    assert_eq!(
        engine.state.rejected_candidates[0].reason,
        "evaluation_failure"
    );
    assert!(
        engine
            .state
            .directive
            .contains("critic returned malformed JSON")
    );
}

#[test]
fn fitness_accepts_single_fatal_flaw_from_model_json() {
    let fitness: Fitness = serde_json::from_value(serde_json::json!({
        "novelty": 1.0,
        "feasibility": 1.0,
        "utility": 1.0,
        "semantic_jump": 1.0,
        "evidence": 0.0,
        "safety": 8.0,
        "prior_art_overlap": 1.0,
        "fatal_flaws": "an unproved recurrence",
        "next_directive": "replace it"
    }))
    .unwrap();

    assert_eq!(fitness.fatal_flaws, vec!["an unproved recurrence"]);
}

#[test]
fn python_compatibility_score_matches_blindmind_formula() {
    let fitness = Fitness {
        novelty: 8.0,
        feasibility: 7.0,
        utility: 8.0,
        semantic_jump: 6.0,
        evidence: 0.0,
        safety: 5.0,
        prior_art_overlap: 1.0,
        fatal_flaws: vec![],
        next_directive: String::new(),
    };
    assert!((blindmind_compatible_score(&fitness) - 8.15).abs() < 1e-9);
    let flagged = Fitness {
        fatal_flaws: vec!["broken".into()],
        ..fitness
    };
    assert!((blindmind_compatible_score(&flagged) - 5.65).abs() < 1e-9);
}

#[test]
fn python_compatibility_fixtures_match_native_policy() {
    let cases: Vec<CompatibilityCase> =
        serde_json::from_str(include_str!("fixtures/blindmind_critic_cases.json")).unwrap();
    for case in cases {
        let fitness = Fitness {
            novelty: case.novelty,
            feasibility: case.feasibility,
            utility: case.utility,
            semantic_jump: case.semantic_jump,
            evidence: 0.0,
            safety: 5.0,
            prior_art_overlap: case.prior_art_overlap,
            fatal_flaws: case.fatal_flaws,
            next_directive: String::new(),
        };
        assert!(
            (blindmind_compatible_score(&fitness) - case.expected).abs() < 1e-9,
            "compatibility case {} diverged",
            case.name
        );
    }
}

#[test]
fn pareto_retention_preserves_distinct_strengths() {
    let make = |id: &str, novelty, feasibility| Concept {
        id: id.into(),
        project: "p".into(),
        generation: 1,
        operator: MutationOperator::Wildcard,
        parent_ids: vec![],
        draft: ConceptDraft {
            domain: "d".into(),
            title: id.into(),
            mechanism: "m".into(),
            rationale: String::new(),
            predicted_measurements: vec!["x".into()],
            kill_criteria: vec!["y".into()],
            tags: BTreeSet::new(),
            executable_contract: None,
        },
        fitness: Fitness {
            novelty,
            feasibility,
            utility: 5.0,
            semantic_jump: 5.0,
            evidence: 5.0,
            safety: 5.0,
            prior_art_overlap: 5.0,
            fatal_flaws: vec![],
            next_directive: String::new(),
        },
    };
    let novelty = make("novel", 10.0, 5.0);
    let feasible = make("feasible", 5.0, 10.0);
    let dominated = make("dominated", 4.0, 4.0);
    let ids = pareto_frontier([&novelty, &feasible, &dominated])
        .into_iter()
        .map(|c| c.id.as_str())
        .collect::<Vec<_>>();
    assert_eq!(ids, ["novel", "feasible"]);
}

#[test]
fn checkpoint_rejects_unknown_population_ids() {
    let mut state = EvolutionState::new("p", "d", 1, EvolutionConfig::default());
    state.active_population.push("missing".into());
    assert!(EvolutionState::restore_json(&state.checkpoint_json().unwrap()).is_err());
}

#[test]
fn title_overlap_matches_python_rejection_memory_behavior() {
    assert_eq!(
        title_similarity(
            "Quantum Neural Blockchain Integration",
            "Quantum Blockchain Neural Integration"
        ),
        1.0
    );
    assert!(
        title_similarity(
            "Mycelial Computing",
            "Quantum Blockchain Neural Integration"
        ) < 0.7
    );
}

#[test]
fn malformed_drafts_are_rejected_before_evaluation() {
    let draft = ConceptDraft {
        domain: "Physics".into(),
        title: String::new(),
        mechanism: "mechanism".into(),
        rationale: String::new(),
        predicted_measurements: vec![],
        kill_criteria: vec![],
        tags: BTreeSet::new(),
        executable_contract: None,
    };
    assert!(draft.validate().is_err());
}

#[test]
fn falsification_probe_rejects_missing_argv_and_unbounded_timeout() {
    let mut probe = FalsificationProbe {
        language: "python3".into(),
        source: "print('ok')".into(),
        argv: vec![],
        timeout_seconds: 301,
        falsifies_when: "stdout contains counterexample".into(),
    };
    assert!(probe.validate().is_err());
    probe.argv = vec!["python3".into(), "probe.py".into()];
    probe.timeout_seconds = 5;
    assert!(probe.validate().is_ok());
}

struct ContractGenerator {
    distinguishes_from: BTreeMap<String, String>,
}

#[async_trait]
impl CandidateGenerator for ContractGenerator {
    async fn generate(
        &self,
        _operator: MutationOperator,
        _parents: &[&Concept],
        context: &GenerationContext<'_>,
    ) -> Result<ConceptDraft, String> {
        Ok(ConceptDraft {
            domain: "Computer Science".into(),
            title: format!("Cosmetic title {}", context.attempt),
            mechanism: "Compose exact boundary summaries".into(),
            rationale: "The composition identity is the candidate".into(),
            predicted_measurements: vec!["matches exhaustive outputs".into()],
            kill_criteria: vec!["one counterexample rejects it".into()],
            tags: BTreeSet::new(),
            executable_contract: Some(ExecutableContract {
                representation: "boundary state tuple".into(),
                exact_relation: "summary(left ++ right) = merge(summary(left), summary(right))"
                    .into(),
                composition_rule: "merge the two boundary state tuples".into(),
                complexity_argument: "balanced composition visits logarithmically many tuples"
                    .into(),
                objective_test: "compare every input through size 20 against exhaustive evolution"
                    .into(),
                falsification_probe: Some(FalsificationProbe {
                    language: "python3".into(),
                    source: "print('ok')".into(),
                    argv: vec!["python3".into(), "probe.py".into()],
                    timeout_seconds: 5,
                    falsifies_when: "the probe prints a counterexample".into(),
                }),
                distinguishes_from: self.distinguishes_from.clone(),
            }),
        })
    }
}

#[tokio::test]
async fn structural_contract_deduplicates_cosmetic_variants() {
    let config = EvolutionConfig {
        population_size: 2,
        max_attempt_multiplier: 2,
        require_executable_contract: true,
        ..Default::default()
    };
    let state = EvolutionState::new("structural", "find a composition", 8, config);
    let mut engine = EvolutionEngine::new(
        state,
        ContractGenerator {
            distinguishes_from: BTreeMap::new(),
        },
        Evaluator,
    );

    let report = engine.run_generation().await.unwrap();

    assert_eq!(report.accepted.len(), 1);
    assert!(report.rejected >= 1);
}

#[tokio::test]
async fn required_falsification_probe_rejects_prose_only_contracts() {
    struct ProseOnlyGenerator;
    #[async_trait]
    impl CandidateGenerator for ProseOnlyGenerator {
        async fn generate(
            &self,
            _operator: MutationOperator,
            _parents: &[&Concept],
            _context: &GenerationContext<'_>,
        ) -> Result<ConceptDraft, String> {
            Ok(ConceptDraft {
                domain: "Computer Science".into(),
                title: "Prose-only contract".into(),
                mechanism: "A claimed exact composition".into(),
                rationale: "The test is not implemented".into(),
                predicted_measurements: vec!["agreement".into()],
                kill_criteria: vec!["one mismatch".into()],
                tags: BTreeSet::new(),
                executable_contract: Some(ExecutableContract {
                    representation: "boundary tuple".into(),
                    exact_relation: "summary(a ++ b) = merge(summary(a), summary(b))".into(),
                    composition_rule: "merge two summaries".into(),
                    complexity_argument: "balanced merges".into(),
                    objective_test: "someone should compare it with exhaustive data".into(),
                    falsification_probe: None,
                    distinguishes_from: BTreeMap::new(),
                }),
            })
        }
    }

    let config = EvolutionConfig {
        population_size: 1,
        max_attempt_multiplier: 1,
        require_executable_contract: true,
        require_falsification_probe: true,
        ..Default::default()
    };
    let state = EvolutionState::new("probe-required", "supply source", 18, config);
    let mut engine = EvolutionEngine::new(state, ProseOnlyGenerator, Evaluator);
    let report = engine.run_generation().await.unwrap();
    assert!(report.accepted.is_empty());
    assert_eq!(report.rejection_reasons["missing_falsification_probe"], 1);
}

#[tokio::test]
async fn eliminated_family_requires_an_explicit_structural_distinction() {
    let config = EvolutionConfig {
        population_size: 1,
        max_attempt_multiplier: 1,
        require_executable_contract: true,
        ..Default::default()
    };
    let mut state = EvolutionState::new("elimination", "avoid the dead end", 9, config);
    state.structural_exclusions.push(StructuralExclusion {
        id: "row-parity".into(),
        description: "Row parity cancellation leaves half the rows".into(),
        structural_features: vec!["parity cancellation popcount rows".into()],
        evidence_ids: vec!["probe:parity-survival".into()],
    });
    let mut missing = EvolutionEngine::new(
        state.clone(),
        ContractGenerator {
            distinguishes_from: BTreeMap::new(),
        },
        Evaluator,
    );
    let missing_report = missing.run_generation().await.unwrap();
    assert!(missing_report.accepted.is_empty());
    assert_eq!(
        missing_report.rejection_reasons["exclusion_not_distinguished"],
        1
    );
    assert_eq!(
        missing.state.rejected_candidates[0].reason,
        "exclusion_not_distinguished"
    );
    assert!(missing.state.directive.contains("Do not repeat"));
    assert_eq!(
        missing.state.rejected_titles.last().map(String::as_str),
        Some("Cosmetic title 0")
    );

    let mut covered = EvolutionEngine::new(
        state,
        ContractGenerator {
            distinguishes_from: BTreeMap::from([(
                "row-parity".into(),
                "uses exact boundary-state composition, not row cancellation".into(),
            )]),
        },
        Evaluator,
    );
    assert_eq!(covered.run_generation().await.unwrap().accepted.len(), 1);
}

#[test]
fn interchange_shapes_can_represent_external_scores() {
    let scores = BTreeMap::from([("novelty".to_string(), 8.0)]);
    assert_eq!(scores["novelty"], 8.0);
}

fn objective_feedback_state() -> EvolutionState {
    let mut state = EvolutionState::new("objective", "measure it", 11, Default::default());
    let concept = Concept {
        id: "candidate-1".into(),
        project: "objective".into(),
        generation: 1,
        operator: MutationOperator::Wildcard,
        parent_ids: vec![],
        draft: ConceptDraft {
            domain: "Computer Science".into(),
            title: "Measured candidate".into(),
            mechanism: "A candidate with an executable implementation".into(),
            rationale: "Awaiting objective evaluation".into(),
            predicted_measurements: vec!["runtime".into()],
            kill_criteria: vec!["fails correctness suite".into()],
            tags: BTreeSet::new(),
            executable_contract: None,
        },
        fitness: Fitness {
            novelty: 9.0,
            feasibility: 9.0,
            utility: 8.0,
            semantic_jump: 7.0,
            evidence: 2.0,
            safety: 8.0,
            prior_art_overlap: 1.0,
            fatal_flaws: vec![],
            next_directive: String::new(),
        },
    };
    state.active_population.push(concept.id.clone());
    state.concepts.insert(concept.id.clone(), concept);
    state
}

#[test]
fn reproduced_objective_pass_raises_evidence_without_inventing_novelty() {
    let mut state = objective_feedback_state();
    state
        .apply_objective_feedback(
            "candidate-1",
            ObjectiveFeedback {
                verification_id: "verification:pass".into(),
                evaluator_id: "benchmark".into(),
                evaluator_version: "1".into(),
                output_sha256: "a".repeat(64),
                passed: true,
                hard_constraints_passed: true,
                independently_reproduced: true,
                measurements: BTreeMap::from([("runtime_ms".into(), 4.0)]),
                recorded_at: 10,
            },
        )
        .unwrap();
    let concept = &state.concepts["candidate-1"];
    assert_eq!(concept.fitness.evidence, 10.0);
    assert_eq!(concept.fitness.feasibility, 9.0);
    assert_eq!(concept.fitness.novelty, 9.0);
    assert_eq!(state.objective_feedback["candidate-1"].len(), 1);
}

#[test]
fn failed_hard_constraint_overrides_optimism_and_removes_candidate() {
    let mut state = objective_feedback_state();
    state
        .apply_objective_feedback(
            "candidate-1",
            ObjectiveFeedback {
                verification_id: "verification:fail".into(),
                evaluator_id: "benchmark".into(),
                evaluator_version: "1".into(),
                output_sha256: "b".repeat(64),
                passed: false,
                hard_constraints_passed: false,
                independently_reproduced: false,
                measurements: BTreeMap::new(),
                recorded_at: 11,
            },
        )
        .unwrap();
    let concept = &state.concepts["candidate-1"];
    assert_eq!(concept.fitness.evidence, 2.0);
    assert_eq!(concept.fitness.feasibility, 3.0);
    assert!(!concept.fitness.fatal_flaws.is_empty());
    assert!(state.active_population.is_empty());
}

#[test]
fn objective_feedback_round_trips_and_rejects_dangling_references() {
    let mut state = objective_feedback_state();
    state
        .objective_feedback
        .insert("missing".into(), Vec::new());
    let json = state.checkpoint_json().unwrap();
    assert!(EvolutionState::restore_json(&json).is_err());

    state.objective_feedback.remove("missing");
    assert_eq!(
        EvolutionState::restore_json(&state.checkpoint_json().unwrap()).unwrap(),
        state
    );
}

#[test]
fn checkpoint_rejects_missing_or_forward_lineage() {
    let mut state = objective_feedback_state();
    let parent = Concept {
        id: "parent".into(),
        project: "objective".into(),
        generation: 0,
        operator: MutationOperator::Wildcard,
        parent_ids: vec![],
        draft: state.concepts["candidate-1"].draft.clone(),
        fitness: state.concepts["candidate-1"].fitness.clone(),
    };
    state.concepts.insert(parent.id.clone(), parent);
    state.concepts.get_mut("candidate-1").unwrap().parent_ids = vec!["parent".into()];
    assert!(EvolutionState::restore_json(&state.checkpoint_json().unwrap()).is_err());

    state.lineage.push(LineageEdge {
        parent_id: "parent".into(),
        child_id: "candidate-1".into(),
        operator: MutationOperator::Wildcard,
    });
    assert!(EvolutionState::restore_json(&state.checkpoint_json().unwrap()).is_ok());

    state.concepts.get_mut("parent").unwrap().generation = 2;
    assert!(EvolutionState::restore_json(&state.checkpoint_json().unwrap()).is_err());
}

struct ConcurrentGenerator {
    current: Arc<AtomicUsize>,
    maximum: Arc<AtomicUsize>,
}

#[async_trait]
impl CandidateGenerator for ConcurrentGenerator {
    async fn generate(
        &self,
        _operator: MutationOperator,
        _parents: &[&Concept],
        context: &GenerationContext<'_>,
    ) -> Result<ConceptDraft, String> {
        let current = self.current.fetch_add(1, Ordering::SeqCst) + 1;
        self.maximum.fetch_max(current, Ordering::SeqCst);
        tokio::time::sleep(Duration::from_millis(20)).await;
        self.current.fetch_sub(1, Ordering::SeqCst);
        Ok(ConceptDraft {
            domain: "d".into(),
            title: format!("parallel candidate {}", context.attempt),
            mechanism: "measurable mechanism".into(),
            rationale: String::new(),
            predicted_measurements: vec!["measurement".into()],
            kill_criteria: vec!["failed measurement".into()],
            tags: BTreeSet::new(),
            executable_contract: None,
        })
    }
}

#[tokio::test]
async fn generation_uses_bounded_parallelism_but_commits_deterministically() {
    let current = Arc::new(AtomicUsize::new(0));
    let maximum = Arc::new(AtomicUsize::new(0));
    let config = EvolutionConfig {
        population_size: 3,
        max_concurrency: 3,
        ..Default::default()
    };
    let state = EvolutionState::new("parallel", "test", 7, config);
    let generator = ConcurrentGenerator {
        current: Arc::clone(&current),
        maximum: Arc::clone(&maximum),
    };
    let mut engine = EvolutionEngine::new(state, generator, Evaluator);
    let report = engine.run_generation().await.unwrap();
    assert_eq!(maximum.load(Ordering::SeqCst), 3);
    assert_eq!(report.accepted.len(), 3);
    assert_eq!(report.attempts, 3);
    assert!(
        report
            .accepted
            .iter()
            .all(|id| engine.state.concepts.contains_key(id))
    );
}

struct PartiallyFailingGenerator;

#[async_trait]
impl CandidateGenerator for PartiallyFailingGenerator {
    async fn generate(
        &self,
        _operator: MutationOperator,
        _parents: &[&Concept],
        context: &GenerationContext<'_>,
    ) -> Result<ConceptDraft, String> {
        if context.attempt.is_multiple_of(2) {
            return Err("synthetic provider failure".into());
        }
        Generator
            .generate(MutationOperator::Wildcard, &[], context)
            .await
    }
}

#[tokio::test]
async fn individual_provider_failures_do_not_abort_the_generation() {
    let config = EvolutionConfig {
        population_size: 2,
        max_concurrency: 2,
        ..Default::default()
    };
    let state = EvolutionState::new("resilient", "test", 9, config);
    let mut engine = EvolutionEngine::new(state, PartiallyFailingGenerator, Evaluator);
    let report = engine.run_generation().await.unwrap();
    assert_eq!(report.accepted.len(), 2);
    assert!(!report.failures.is_empty());
    assert!(
        report
            .failures
            .iter()
            .all(|failure| failure.stage == "generation")
    );
}
