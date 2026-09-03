use crosstalk::types::epistemics::{
    Claim, ClaimEdge, ClaimKind, ClaimLedger, ClaimRelation, ClaimStatus, EvidenceRef,
};
use crosstalk::types::investigation::{
    EvidenceArtifact, EvidenceKind, Investigation, Measurement, VerificationKind,
    VerificationRecord, VerificationStatus, duplicate_ids,
};
use std::collections::BTreeMap;

fn digest(byte: char) -> String {
    std::iter::repeat_n(byte, 64).collect()
}

fn claim(id: &str) -> Claim {
    Claim {
        id: id.into(),
        text: "The candidate preserves the invariant".into(),
        kind: ClaimKind::Fact,
        status: ClaimStatus::Open,
        confidence: 0.7,
        evidence: vec![],
    }
}

fn verified_proof(id: &str, subject_id: &str) -> VerificationRecord {
    VerificationRecord {
        id: id.into(),
        kind: VerificationKind::FormalProof,
        status: VerificationStatus::Verified,
        subject_id: subject_id.into(),
        evaluator_id: "lean4".into(),
        evaluator_version: "4.test".into(),
        specification_sha256: digest('a'),
        input_sha256: digest('b'),
        output_sha256: digest('c'),
        measurements: vec![],
        diagnostics: "accepted".into(),
        started_at: 10,
        completed_at: 11,
        reproduction_of: None,
    }
}

#[test]
fn evidence_registration_requires_a_real_digest_shape() {
    let mut investigation = Investigation::new("case", "Find a better algorithm");
    let result = investigation.register_evidence(EvidenceArtifact {
        id: "source:1".into(),
        kind: EvidenceKind::SourceRecord,
        title: "Relevant paper".into(),
        content_sha256: "not-a-digest".into(),
        media_type: "application/json".into(),
        source_uri: None,
        locator: None,
        artifact_name: None,
        verification_id: None,
        captured_at: 1,
        independent: true,
        metadata: BTreeMap::new(),
    });
    assert!(result.is_err());
}

#[test]
fn linking_evidence_does_not_promote_a_claim() {
    let mut investigation = Investigation::new("case", "Audit the claim");
    investigation
        .register_evidence(EvidenceArtifact {
            id: "source:1".into(),
            kind: EvidenceKind::SourceSnapshot,
            title: "Primary source".into(),
            content_sha256: digest('d'),
            media_type: "text/plain".into(),
            source_uri: None,
            locator: Some("line 12".into()),
            artifact_name: None,
            verification_id: None,
            captured_at: 1,
            independent: true,
            metadata: BTreeMap::new(),
        })
        .unwrap();
    let mut claims = ClaimLedger::default();
    claims.insert(claim("claim:1")).unwrap();
    investigation
        .link_claim_evidence(&mut claims, "claim:1", "source:1", true, 0.8)
        .unwrap();
    assert_eq!(claims.claims["claim:1"].status, ClaimStatus::Open);
    assert_eq!(claims.claims["claim:1"].evidence.len(), 1);
}

#[test]
fn accepting_formal_verification_promotes_claim_and_passes_audit() {
    let mut investigation = Investigation::new("case", "Prove the invariant");
    let mut claims = ClaimLedger::default();
    claims.insert(claim("claim:invariant")).unwrap();
    investigation
        .record_verification(verified_proof("proof:1", "claim:invariant"))
        .unwrap();
    let evidence_id = investigation
        .apply_verification_to_claim(&mut claims, "proof:1", 1.0)
        .unwrap();

    assert_eq!(evidence_id, "verification:proof:1");
    assert_eq!(
        claims.claims["claim:invariant"].status,
        ClaimStatus::FormallyVerified
    );
    let audit = investigation.audit(&claims);
    assert!(audit.passed, "{:?}", audit.issues);
    assert_eq!(audit.verified_claims, 1);
    assert_eq!(audit.verification_coverage, 1.0);
    let release = investigation.scientific_release_assessment(&claims);
    assert!(release.eligible, "{:?}", release.blocking_reasons);
}

#[test]
fn integrity_pass_without_verified_claims_is_not_scientific_release() {
    let investigation = Investigation::new("case", "Unverified result");
    let claims = ClaimLedger::default();

    let audit = investigation.audit(&claims);
    let release = investigation.scientific_release_assessment(&claims);

    assert!(audit.passed);
    assert!(!release.eligible);
    assert!(
        release
            .blocking_reasons
            .iter()
            .any(|reason| reason.contains("no explicit fact or inference"))
    );
    let warning = release.unverified_warning().unwrap();
    assert!(warning.contains("Unverified model synthesis"));
    assert!(warning.contains("do not by themselves establish universal claims"));
}

#[test]
fn audit_rejects_claim_links_to_unknown_evidence() {
    let investigation = Investigation::new("case", "Check provenance");
    let mut claims = ClaimLedger::default();
    let mut unsupported = claim("claim:1");
    unsupported.status = ClaimStatus::Supported;
    unsupported.evidence.push(EvidenceRef {
        source_id: "missing".into(),
        locator: None,
        content_sha256: Some(digest('e')),
        supports: true,
        strength: 1.0,
    });
    claims.insert(unsupported).unwrap();
    let audit = investigation.audit(&claims);
    assert!(!audit.passed);
    assert!(
        audit
            .issues
            .iter()
            .any(|issue| issue.code == "dangling_claim_evidence")
    );
}

#[test]
fn reproduction_must_reference_an_existing_verification() {
    let mut investigation = Investigation::new("case", "Reproduce a result");
    let mut record = verified_proof("proof:2", "claim:1");
    record.kind = VerificationKind::Reproduction;
    record.reproduction_of = Some("proof:missing".into());
    assert!(investigation.record_verification(record).is_err());
}

#[test]
fn verification_rejects_non_finite_measurements() {
    let mut investigation = Investigation::new("case", "Measure a result");
    let mut record = verified_proof("benchmark:1", "candidate:1");
    record.kind = VerificationKind::Benchmark;
    record.measurements.push(Measurement {
        name: "throughput".into(),
        value: f64::NAN,
        unit: "items/s".into(),
        uncertainty: None,
        sample_size: Some(10),
    });
    assert!(investigation.record_verification(record).is_err());
}

#[test]
fn duplicate_identifier_reporting_is_stable() {
    assert_eq!(
        duplicate_ids(["b", "a", "b", "a", "a"]),
        vec!["a".to_string(), "b".to_string()]
    );
}

#[test]
fn claim_ledger_rejects_duplicate_ids() {
    let mut claims = ClaimLedger::default();
    claims.insert(claim("same")).unwrap();
    assert!(claims.insert(claim("same")).is_err());
}

#[test]
fn audit_rejects_malformed_imported_claim_edges() {
    let investigation = Investigation::new("case", "Audit imported state");
    let mut claims = ClaimLedger::default();
    claims.insert(claim("known")).unwrap();
    claims.edges.push(ClaimEdge {
        from: "known".into(),
        to: "missing".into(),
        relation: ClaimRelation::DependsOn,
    });
    let audit = investigation.audit(&claims);
    assert!(!audit.passed);
    assert!(
        audit
            .issues
            .iter()
            .any(|issue| issue.code == "invalid_claim_edge")
    );
}
