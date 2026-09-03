use crosstalk::types::compute::{BudgetLedger, BudgetMode};
use crosstalk::types::conversation::{
    ConversationState, TaskCategory, Turn, TurnOutcome, TurnStructure,
};

#[test]
fn test_conversation_state_initialization() {
    let s = ConversationState::new("test-session");
    assert_eq!(s.session_id, "test-session");
    assert_eq!(s.iteration_index, 0);
    assert!(s.artifacts.is_empty());
}

#[test]
fn test_budget_ledger_mode_transitions() {
    let mut ledger = BudgetLedger {
        session_budget: 10.0,
        spent: 0.0,
        entries: vec![],
        ..BudgetLedger::default()
    };
    assert_eq!(ledger.mode(), BudgetMode::Normal);

    ledger.spent = 8.5; // 15% left
    assert_eq!(ledger.mode(), BudgetMode::CostReduction);

    ledger.spent = 9.8; // 2% left
    assert_eq!(ledger.mode(), BudgetMode::Emergency);
}

#[test]
fn shared_budget_reservations_fail_closed_across_calls_and_tokens() {
    let mut ledger = BudgetLedger {
        max_model_calls: 2,
        max_input_tokens: 10,
        max_output_tokens: 5,
        ..BudgetLedger::default()
    };
    ledger.try_reserve_model_call(4).unwrap();
    ledger.try_consume_output_tokens(3).unwrap();
    ledger.try_reserve_model_call(6).unwrap();
    assert!(ledger.try_reserve_model_call(0).is_err());
    assert!(ledger.try_consume_output_tokens(3).is_err());
    assert_eq!(ledger.model_calls, 2);
    assert_eq!(ledger.estimated_input_tokens, 10);
    assert_eq!(ledger.estimated_output_tokens, 3);
}

#[test]
fn subsystem_call_slots_are_conservatively_reserved_and_released() {
    let mut ledger = BudgetLedger {
        max_model_calls: 5,
        ..BudgetLedger::default()
    };
    assert_eq!(ledger.reserve_model_call_slots(8), 5);
    assert_eq!(ledger.model_calls, 5);
    ledger.release_unused_model_call_slots(2);
    assert_eq!(ledger.model_calls, 3);
    assert_eq!(ledger.reserve_model_call_slots(4), 2);
}

#[test]
fn test_budget_ledger_zero_budget_is_normal() {
    // session_budget == 0 means "no limit configured" — must not trigger Emergency.
    let ledger = BudgetLedger::default();
    assert_eq!(ledger.mode(), BudgetMode::Normal);
}

#[test]
fn test_turn_serialization_roundtrip() {
    let turn = Turn {
        index: 42,
        model_id: "gpt-4".to_string(),
        content: "Proposing a change.".to_string(),
        timestamp: 123456789,
        diffs: vec![],
        certainty: Some(0.85),
        outcome: TurnOutcome::Compiled,
        task_category: Some(TaskCategory::CodeGeneration),
        structure: Some(TurnStructure::StepByStep),
        signature: vec![1, 2, 3],
        surprise_signal: None,
        consistency_score: None,
        diff_quality_score: None,
        persona_disclosure: None,
    };

    let serialized = serde_json::to_string(&turn).unwrap();
    let deserialized: Turn = serde_json::from_str(&serialized).unwrap();

    assert_eq!(deserialized.index, 42);
    assert_eq!(deserialized.model_id, "gpt-4");
}

fn mk_turn(index: u32, content: &str) -> Turn {
    Turn {
        index,
        model_id: "agent".to_string(),
        content: content.to_string(),
        timestamp: index as u64,
        diffs: vec![],
        certainty: None,
        outcome: TurnOutcome::Unknown,
        task_category: None,
        structure: None,
        signature: vec![],
        surprise_signal: None,
        consistency_score: None,
        diff_quality_score: None,
        persona_disclosure: None,
    }
}

#[test]
fn hash_chain_is_intact_after_pushes() {
    let mut s = ConversationState::new("chain");
    for i in 0..3 {
        s.push_turn(mk_turn(i, &format!("turn {i}")));
    }
    assert_eq!(s.turn_hashes.len(), 3);
    assert_eq!(s.verify_chain(), None);
    assert!(!s.chain_head_hex().is_empty());
}

#[test]
fn tampering_a_turn_breaks_the_chain() {
    let mut s = ConversationState::new("chain");
    for i in 0..3 {
        s.push_turn(mk_turn(i, &format!("turn {i}")));
    }
    // Edit a committed turn's content without updating its chain hash.
    s.turns[1].content = "tampered".to_string();
    assert_eq!(s.verify_chain(), Some(1));
}

#[test]
fn tampering_the_first_turn_breaks_the_chain() {
    let mut s = ConversationState::new("chain");
    for i in 0..3 {
        s.push_turn(mk_turn(i, &format!("turn {i}")));
    }
    s.turns[0].content = "tampered first turn".to_string();
    assert_eq!(s.verify_chain(), Some(0));
}

#[test]
fn reordering_turns_breaks_the_chain() {
    let mut s = ConversationState::new("chain");
    for i in 0..3 {
        s.push_turn(mk_turn(i, &format!("turn {i}")));
    }
    s.turns.swap(1, 2);
    assert_eq!(s.verify_chain(), Some(1));
}

#[test]
fn deleting_a_turn_is_detected_as_count_divergence() {
    let mut s = ConversationState::new("chain");
    for i in 0..3 {
        s.push_turn(mk_turn(i, &format!("turn {i}")));
    }
    s.turns.remove(1);
    assert_eq!(s.verify_chain(), Some(0));
}

#[test]
fn state_with_turns_but_no_chain_fails_closed() {
    let mut s = ConversationState::new("chain");
    for i in 0..3 {
        s.push_turn(mk_turn(i, &format!("turn {i}")));
    }
    // Removing the chain must not downgrade the state into a trusted legacy mode.
    s.turn_hashes.clear();
    assert_eq!(s.verify_chain(), Some(0));
}

#[test]
fn chain_stays_aligned_and_consistent_after_drain() {
    let mut s = ConversationState::new("chain");
    for i in 0..205 {
        s.push_turn(mk_turn(i, &format!("turn {i}")));
    }
    assert_eq!(s.turns.len(), 200);
    assert_eq!(s.turn_hashes.len(), 200);
    assert!(s.turn_chain_base.is_some());
    assert_eq!(s.verify_chain(), None);

    s.turns[0].content = "tampered retained anchor".into();
    assert_eq!(s.verify_chain(), Some(0));
}

#[test]
fn replacing_finalized_turn_rebuilds_chain() {
    let mut state = ConversationState::new("finalized-turn");
    state.push_turn(mk_turn(0, "provisional"));
    state.push_turn(mk_turn(1, "second"));

    let mut finalized = state.turns[1].clone();
    finalized.outcome = TurnOutcome::VerificationFailed;
    finalized.signature = vec![7; 64];
    state
        .replace_last_turn(finalized.clone())
        .expect("replace final turn");

    assert_eq!(state.turns[1].index, finalized.index);
    assert_eq!(state.turns[1].outcome, finalized.outcome);
    assert_eq!(state.turns[1].signature, finalized.signature);
    assert_eq!(state.verify_chain(), None);
}

#[test]
fn transcript_chain_survives_json_round_trip_with_full_turn_metadata() {
    use crosstalk::types::artifact::ArtifactDiff;
    use crosstalk::types::fiduciary::PersonaDisclosure;

    let mut state = ConversationState::new("round-trip");
    state.push_turn(mk_turn(0, "user"));
    let mut turn = mk_turn(1, "synthesis");
    turn.diffs = vec![(
        "proof.lean".into(),
        ArtifactDiff::new(0, 0, "+ theorem candidate".into()).unwrap(),
    )];
    turn.certainty = Some(0.98);
    turn.outcome = TurnOutcome::VerificationFailed;
    turn.task_category = Some(TaskCategory::Research);
    turn.structure = Some(TurnStructure::Symbolic);
    turn.signature = vec![17; 64];
    turn.surprise_signal = Some(0.020000000000000018);
    turn.consistency_score = Some(0.5443331226706505);
    turn.diff_quality_score = Some(1.0);
    turn.persona_disclosure = Some(PersonaDisclosure {
        turn_index: 1,
        agent_id: "agent".into(),
        persona_name: "agent".into(),
        system_prompt_hash: [23; 32],
        signature: vec![29; 64],
    });
    state.push_turn(turn);
    assert_eq!(state.verify_chain(), None);

    let serialized = serde_json::to_vec(&state).unwrap();
    let restored: ConversationState = serde_json::from_slice(&serialized).unwrap();
    for field in [
        (
            "certainty",
            state.turns[1].certainty,
            restored.turns[1].certainty,
        ),
        (
            "surprise_signal",
            state.turns[1].surprise_signal,
            restored.turns[1].surprise_signal,
        ),
        (
            "consistency_score",
            state.turns[1].consistency_score,
            restored.turns[1].consistency_score,
        ),
        (
            "diff_quality_score",
            state.turns[1].diff_quality_score,
            restored.turns[1].diff_quality_score,
        ),
    ] {
        assert_eq!(
            field.1.map(f64::to_bits),
            field.2.map(f64::to_bits),
            "{field:?}"
        );
    }
    assert_eq!(restored.verify_chain(), None);
    assert_eq!(restored.turn_hashes, state.turn_hashes);
}
