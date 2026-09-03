use crosstalk::engines::idea_evolution::{apply_objective_evaluation, apply_reproduction_outcome};
use crosstalk::engines::objective_evaluation::{
    ConstraintResult, ObjectiveEvaluation, ReproductionOutcome,
};
use crosstalk::types::investigation::{Measurement, VerificationStatus};
use crosstalk_evolution::{
    Concept, ConceptDraft, EvolutionConfig, EvolutionState, Fitness, MutationOperator,
};
use std::collections::{BTreeMap, BTreeSet};

fn checkpoint() -> String {
    let mut state = EvolutionState::new("bridge", "evaluate", 1, EvolutionConfig::default());
    state.active_population.push("concept".into());
    state.concepts.insert(
        "concept".into(),
        Concept {
            id: "concept".into(),
            project: "bridge".into(),
            generation: 1,
            operator: MutationOperator::Wildcard,
            parent_ids: vec![],
            draft: ConceptDraft {
                domain: "Computer Science".into(),
                title: "Candidate".into(),
                mechanism: "Executable candidate".into(),
                rationale: String::new(),
                predicted_measurements: vec!["latency".into()],
                kill_criteria: vec!["failed correctness".into()],
                tags: BTreeSet::new(),
                executable_contract: None,
            },
            fitness: Fitness {
                novelty: 8.0,
                feasibility: 9.0,
                utility: 8.0,
                semantic_jump: 6.0,
                evidence: 1.0,
                safety: 8.0,
                prior_art_overlap: 2.0,
                fatal_flaws: vec![],
                next_directive: String::new(),
            },
        },
    );
    state.checkpoint_json().unwrap()
}

fn evaluation(
    id: &str,
    status: VerificationStatus,
    constraint_passed: bool,
) -> ObjectiveEvaluation {
    ObjectiveEvaluation {
        id: id.into(),
        evaluator_id: "test-suite".into(),
        evaluator_version: "1".into(),
        specification_sha256: "a".repeat(64),
        candidate_id: "concept".into(),
        candidate_sha256: "b".repeat(64),
        status,
        measurements: vec![Measurement {
            name: "latency".into(),
            value: 4.0,
            unit: "ms".into(),
            uncertainty: None,
            sample_size: Some(10),
        }],
        constraints: vec![ConstraintResult {
            name: "correctness".into(),
            passed: constraint_passed,
            diagnostics: String::new(),
        }],
        diagnostics: String::new(),
        raw_output_sha256: "c".repeat(64),
        started_at: 1,
        completed_at: 2,
        environment: BTreeMap::new(),
    }
}

#[test]
fn verified_tool_result_updates_the_native_checkpoint() {
    let updated = apply_objective_evaluation(
        &checkpoint(),
        "concept",
        &evaluation("verification:1", VerificationStatus::Verified, true),
        true,
    )
    .unwrap();
    let state = EvolutionState::restore_json(&updated).unwrap();
    assert_eq!(state.concepts["concept"].fitness.evidence, 10.0);
    assert_eq!(state.concepts["concept"].fitness.novelty, 8.0);
}

#[test]
fn reproduction_disagreement_overrides_two_plausible_model_scores() {
    let primary = evaluation("verification:primary", VerificationStatus::Verified, true);
    let reproduction = evaluation(
        "verification:reproduction",
        VerificationStatus::Verified,
        true,
    );
    let updated = apply_reproduction_outcome(
        &checkpoint(),
        "concept",
        &ReproductionOutcome {
            primary,
            reproduction: Some(reproduction),
            agreed: false,
            mismatches: vec!["latency drifted".into()],
        },
    )
    .unwrap();
    let state = EvolutionState::restore_json(&updated).unwrap();
    assert_eq!(state.concepts["concept"].fitness.feasibility, 3.0);
    assert!(!state.concepts["concept"].fitness.fatal_flaws.is_empty());
}
