use crosstalk::engines::investigation_bundle::{
    BUNDLE_SCHEMA, BundleOptions, InvestigationBundleExporter, REPORT_SCHEMA,
};
use crosstalk::types::conversation::ConversationState;
use sha2::{Digest, Sha256};

#[test]
fn bundle_exports_manifest_audit_and_content_addressed_artifacts() {
    let temp = tempfile::tempdir().unwrap();
    let output = temp.path().join("bundle");
    let mut state = ConversationState::new("bundle-test");
    state.investigation.problem.statement = "Evaluate the candidate".into();
    state.ingest_file(
        "../unsafe/result.json".into(),
        "json".into(),
        "{\"score\": 1}".into(),
    );

    let manifest =
        InvestigationBundleExporter::export(&state, &output, BundleOptions::default()).unwrap();
    assert_eq!(manifest.schema, BUNDLE_SCHEMA);
    assert_eq!(manifest.report_schema.as_deref(), Some(REPORT_SCHEMA));
    assert!(manifest.audit.passed);
    assert!(
        !manifest
            .scientific_release
            .as_ref()
            .expect("new bundles include a release assessment")
            .eligible
    );
    assert!(output.join("manifest.json").is_file());
    assert!(output.join("state.json").is_file());
    assert!(output.join("investigation.json").is_file());
    assert!(output.join("claims.json").is_file());
    assert!(output.join("audit.json").is_file());
    assert!(output.join("transcript.json").is_file());
    assert!(output.join("report.md").is_file());
    let report = std::fs::read_to_string(output.join("report.md")).unwrap();
    assert!(report.contains("Unverified model synthesis"));
    assert!(report.contains("do not by themselves establish universal claims"));
    let artifact = manifest
        .files
        .iter()
        .find(|file| file.path.starts_with("artifacts/"))
        .unwrap();
    assert!(!artifact.path.contains(".."));
    assert_eq!(artifact.content_sha256.len(), 64);
    assert!(output.join(&artifact.path).is_file());
    let verification = InvestigationBundleExporter::verify(&output).unwrap();
    assert!(verification.passed, "{:?}", verification.issues);
}

#[test]
fn bundle_can_exclude_transcript_and_artifacts() {
    let temp = tempfile::tempdir().unwrap();
    let output = temp.path().join("bundle");
    let state = ConversationState::new("minimal");
    let manifest = InvestigationBundleExporter::export(
        &state,
        &output,
        BundleOptions {
            include_transcript: false,
            include_artifacts: false,
        },
    )
    .unwrap();
    assert!(
        !manifest
            .files
            .iter()
            .any(|file| file.path == "transcript.json")
    );
    assert!(
        !manifest
            .files
            .iter()
            .any(|file| file.path.starts_with("artifacts/"))
    );
}

#[test]
fn bundle_refuses_filesystem_root() {
    let state = ConversationState::new("root");
    assert!(
        InvestigationBundleExporter::export(
            &state,
            std::path::Path::new("/"),
            BundleOptions::default()
        )
        .is_err()
    );
}

#[test]
fn bundle_refuses_a_nonempty_output_directory() {
    let temp = tempfile::tempdir().unwrap();
    std::fs::write(temp.path().join("stale.txt"), "old run").unwrap();
    let state = ConversationState::new("stale");
    let error = InvestigationBundleExporter::export(&state, temp.path(), BundleOptions::default())
        .unwrap_err();
    assert!(error.to_string().contains("must be empty"));
}

#[test]
fn bundle_refuses_a_broken_transcript_chain() {
    let temp = tempfile::tempdir().unwrap();
    let output = temp.path().join("bundle");
    let mut state = ConversationState::new("broken-chain");
    state.push_turn(crosstalk::types::conversation::Turn {
        index: 0,
        model_id: "User".into(),
        content: "original".into(),
        timestamp: 0,
        diffs: vec![],
        certainty: Some(1.0),
        outcome: crosstalk::types::conversation::TurnOutcome::Unknown,
        task_category: None,
        structure: None,
        signature: vec![],
        surprise_signal: None,
        consistency_score: None,
        diff_quality_score: None,
        persona_disclosure: None,
    });
    state.turns[0].content = "tampered".into();

    let error =
        InvestigationBundleExporter::export(&state, &output, BundleOptions::default()).unwrap_err();
    assert!(error.to_string().contains("broken transcript chain"));
    assert!(!output.exists());
}

#[test]
fn bundle_verifier_detects_mutation_and_unexpected_files() {
    let temp = tempfile::tempdir().unwrap();
    let output = temp.path().join("bundle");
    let state = ConversationState::new("mutation");
    InvestigationBundleExporter::export(&state, &output, BundleOptions::default()).unwrap();

    std::fs::write(output.join("report.md"), "tampered").unwrap();
    std::fs::write(output.join("extra.txt"), "unexpected").unwrap();
    let verification = InvestigationBundleExporter::verify(&output).unwrap();
    assert!(!verification.passed);
    assert!(
        verification
            .issues
            .iter()
            .any(|issue| issue.contains("content hash mismatch: report.md"))
    );
    assert!(
        verification
            .issues
            .iter()
            .any(|issue| issue.contains("unexpected file: extra.txt"))
    );
}

#[test]
fn legacy_bundle_report_is_not_rerendered_with_a_new_template() {
    let temp = tempfile::tempdir().unwrap();
    let output = temp.path().join("bundle");
    let state = ConversationState::new("legacy-report");
    InvestigationBundleExporter::export(&state, &output, BundleOptions::default()).unwrap();

    let legacy_report = b"# Legacy Crosstalk report\n\nA historical renderer.\n";
    std::fs::write(output.join("report.md"), legacy_report).unwrap();
    let manifest_path = output.join("manifest.json");
    let mut manifest: serde_json::Value =
        serde_json::from_slice(&std::fs::read(&manifest_path).unwrap()).unwrap();
    manifest.as_object_mut().unwrap().remove("report_schema");
    let report = manifest["files"]
        .as_array_mut()
        .unwrap()
        .iter_mut()
        .find(|file| file["path"] == "report.md")
        .unwrap();
    report["content_sha256"] =
        serde_json::Value::String(format!("{:x}", Sha256::digest(legacy_report)));
    report["size_bytes"] = serde_json::Value::from(legacy_report.len());
    std::fs::write(
        &manifest_path,
        serde_json::to_vec_pretty(&manifest).unwrap(),
    )
    .unwrap();

    let verification = InvestigationBundleExporter::verify(&output).unwrap();
    assert!(verification.passed, "{:?}", verification.issues);
}
