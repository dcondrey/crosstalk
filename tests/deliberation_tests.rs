use crosstalk::engines::deliberation::{DeliberationProtocol, ReasoningDomain, classify_task};
use crosstalk::engines::formal_verification::{FormalProofVerifier, ProofBackend, ProofStatus};
use crosstalk::engines::idea_evolution::{
    BLINDMIND_CONTRACT_VERSION, EvolutionRequest, EvolutionResponse, EvolvedIdea,
    import_blindmind_archive,
};
use crosstalk::types::epistemics::{
    Claim, ClaimEdge, ClaimKind, ClaimLedger, ClaimRelation, ClaimStatus, EvidenceRef,
};
use crosstalk::types::mode::ModeDefinition;
use std::collections::{BTreeMap, BTreeSet};

#[test]
fn routes_domain_general_tasks() {
    assert_eq!(
        classify_task("Debate the case for universal basic income"),
        ReasoningDomain::Debate
    );
    assert_eq!(
        classify_task("Prove this theorem about finite groups"),
        ReasoningDomain::Theoretical
    );
    assert_eq!(
        classify_task("Invent a novel technology for desalination"),
        ReasoningDomain::Invention
    );
    assert_eq!(
        classify_task("Debug this Rust parser"),
        ReasoningDomain::Software
    );
    assert_eq!(
        classify_task("Solve this unsolved cryptographic puzzle and ciphertext"),
        ReasoningDomain::Cryptanalysis
    );
    assert_eq!(
        classify_task("Decipher this undeciphered ancient language inscription"),
        ReasoningDomain::Decipherment
    );
    assert_eq!(
        classify_task("Investigate this unsolved historical mystery"),
        ReasoningDomain::HistoricalInquiry
    );
    assert_eq!(
        classify_task("Develop a testable theory in particle physics"),
        ReasoningDomain::NaturalScience
    );
}

#[test]
fn protocols_have_evidence_and_falsifiable_completion_contracts() {
    for task in [
        "debate this resolution",
        "prove the lemma",
        "invent a battery",
        "run an empirical study",
        "choose an option",
        "solve a cryptographic puzzle",
        "decipher an ancient inscription",
        "investigate a historical mystery",
        "solve a chemistry problem",
    ] {
        let protocol = DeliberationProtocol::for_task(task);
        assert!(!protocol.roles.is_empty());
        assert!(!protocol.phases.is_empty());
        assert!(!protocol.evidence.is_empty());
        assert!(!protocol.completion_contract.is_empty());
        assert!(protocol.prompt_contract().contains("[CONJECTURE]"));
    }
}

#[test]
fn specialized_modes_are_detected_before_generic_generation() {
    let presets = ModeDefinition::presets();
    assert_eq!(
        presets[ModeDefinition::detect_preset_index("debate both sides")].name,
        "Debate"
    );
    assert_eq!(
        presets[ModeDefinition::detect_preset_index("prove that this theorem holds")].name,
        "Theorem"
    );
    assert_eq!(
        presets[ModeDefinition::detect_preset_index("invent novel technology")].name,
        "Invention"
    );
}

#[tokio::test]
async fn strict_proof_policy_rejects_lean_placeholders_without_needing_lean() {
    let result = FormalProofVerifier::default()
        .verify_source(ProofBackend::Lean4, "theorem fake : False := by sorry")
        .await
        .unwrap();
    assert_eq!(result.status, ProofStatus::PolicyViolation);
    assert!(!result.is_verified());
}

#[tokio::test]
async fn unavailable_checker_is_not_reported_as_verified() {
    let result = FormalProofVerifier::default()
        .verify_source(
            ProofBackend::Lean4,
            "theorem identity (p : Prop) (h : p) : p := h",
        )
        .await
        .unwrap();
    if result.status == ProofStatus::CheckerUnavailable {
        assert!(!result.is_verified());
    }
}

#[test]
fn claim_ledger_exposes_unresolved_cruxes_and_verification_coverage() {
    let mut ledger = ClaimLedger::default();
    ledger
        .insert(Claim {
            id: "premise".into(),
            text: "The inscription is read left-to-right".into(),
            kind: ClaimKind::Assumption,
            status: ClaimStatus::Contested,
            confidence: 0.5,
            evidence: vec![],
        })
        .unwrap();
    ledger
        .insert(Claim {
            id: "reading".into(),
            text: "The repeated group denotes a title".into(),
            kind: ClaimKind::Conjecture,
            status: ClaimStatus::Supported,
            confidence: 0.7,
            evidence: vec![EvidenceRef {
                source_id: "corpus:1".into(),
                locator: Some("signs 4-8".into()),
                content_sha256: Some("abc".into()),
                supports: true,
                strength: 0.8,
            }],
        })
        .unwrap();
    ledger
        .connect(ClaimEdge {
            from: "reading".into(),
            to: "premise".into(),
            relation: ClaimRelation::DependsOn,
        })
        .unwrap();
    assert_eq!(ledger.unresolved_cruxes()[0].id, "premise");
    assert_eq!(ledger.verification_coverage(), 0.5);
}

#[test]
fn tagged_claims_enter_the_ledger_without_classifying_ordinary_prose() {
    let mut ledger = ClaimLedger::default();
    ledger.ingest_tagged(4, "ordinary explanation\n[FACT] The measured period is 12.0 s\n[CONJECTURE] The marks encode a calendar", 0.75);
    assert_eq!(ledger.claims.len(), 2);
    assert_eq!(ledger.claims["turn-4-claim-1"].kind, ClaimKind::Fact);
    assert_eq!(ledger.claims["turn-4-claim-2"].status, ClaimStatus::Open);
}

#[test]
fn blindmind_contract_rejects_score_inflation_and_empty_mechanisms() {
    let response = EvolutionResponse {
        schema: BLINDMIND_CONTRACT_VERSION.into(),
        project: "discovery".into(),
        directive: "invent".into(),
        ideas: vec![EvolvedIdea {
            id: "idea-1".into(),
            parent_ids: vec![],
            mutation_type: "WILDCARD".into(),
            domain: "Physics".into(),
            title: "Candidate".into(),
            mechanism: "A testable interaction".into(),
            predicted_measurements: vec!["signal exceeds control by 3 sigma".into()],
            kill_criteria: vec!["no signal in preregistered test".into()],
            generation: 0,
            tags: BTreeSet::new(),
            external_scores: BTreeMap::from([("novelty".into(), 11.0)]),
            executable_contract: None,
            objectively_verified: false,
        }],
    };
    assert!(response.validate().is_err());
}

#[test]
fn blindmind_v1_requests_without_new_optional_fields_remain_compatible() {
    let json = r#"{
        "schema":"crosstalk.blindmind.v1",
        "project":"legacy",
        "directive":"invent",
        "constraints":[],
        "evidence_ids":[],
        "seeds":[],
        "population_size":4,
        "generations":1
    }"#;
    let request: EvolutionRequest = serde_json::from_str(json).unwrap();
    assert_eq!(request.max_concurrency, 4);
    assert!(request.evidence_context.is_empty());
    request.validate().unwrap();
}

#[test]
fn blindmind_archive_import_rejects_a_dangling_parent_reference() {
    let json = r#"{
        "schema": "crosstalk.blindmind.v1",
        "project": "riemann",
        "directive": "",
        "ideas": [
            {"id": "seed-1", "generation": 0, "parent_ids": [], "mutation_type": "Wildcard",
             "domain": "Mathematics", "title": "Seed", "mechanism": "A stated mechanism",
             "external_scores": {}, "objectively_verified": false},
            {"id": "child-1", "generation": 1, "parent_ids": ["seed-1", "ghost-9"],
             "mutation_type": "Crossover", "domain": "Mathematics", "title": "Child",
             "mechanism": "A derived mechanism", "external_scores": {},
             "objectively_verified": false}
        ]
    }"#;

    let error = import_blindmind_archive(json).expect_err("dangling parent must not import");
    assert!(error.contains("ghost-9"), "unexpected rejection: {error}");
}
