use crosstalk::engines::investigation_bundle::{
    BUNDLE_SCHEMA, BundleOptions, InvestigationBundleExporter,
};
use crosstalk::types::conversation::ConversationState;

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
    assert!(manifest.audit.passed);
    assert!(output.join("manifest.json").is_file());
    assert!(output.join("state.json").is_file());
    assert!(output.join("investigation.json").is_file());
    assert!(output.join("claims.json").is_file());
    assert!(output.join("audit.json").is_file());
    assert!(output.join("transcript.json").is_file());
    assert!(output.join("report.md").is_file());
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
