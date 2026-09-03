use crosstalk::engines::investigation_bundle::{BundleOptions, InvestigationBundleExporter};
use crosstalk::types::conversation::ConversationState;
use std::process::Command;

#[test]
fn help_exposes_constraints_and_structural_eliminations() {
    let output = Command::new(env!("CARGO_BIN_EXE_crosstalk"))
        .arg("--help")
        .output()
        .unwrap();
    assert!(output.status.success());
    let help = String::from_utf8_lossy(&output.stdout);
    assert!(help.contains("--evolve-constraint"));
    assert!(help.contains("--evolve-exclusion"));
}

#[test]
fn verify_bundle_cli_is_provider_free_and_machine_readable() {
    let temp = tempfile::tempdir().unwrap();
    let bundle = temp.path().join("bundle");
    InvestigationBundleExporter::export(
        &ConversationState::new("offline-cli"),
        &bundle,
        BundleOptions::default(),
    )
    .unwrap();

    let output = Command::new(env!("CARGO_BIN_EXE_crosstalk"))
        .arg("--verify-bundle")
        .arg(&bundle)
        .env("HOME", temp.path().join("empty-home"))
        .env_remove("ANTHROPIC_API_KEY")
        .env_remove("OPENAI_API_KEY")
        .env_remove("OPENROUTER_API_KEY")
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let report: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(report["passed"], true);
}

#[test]
fn verify_bundle_cli_returns_failure_for_tampering() {
    let temp = tempfile::tempdir().unwrap();
    let bundle = temp.path().join("bundle");
    InvestigationBundleExporter::export(
        &ConversationState::new("offline-cli-tamper"),
        &bundle,
        BundleOptions::default(),
    )
    .unwrap();
    std::fs::write(bundle.join("report.md"), "tampered").unwrap();

    let output = Command::new(env!("CARGO_BIN_EXE_crosstalk"))
        .arg("--verify-bundle")
        .arg(&bundle)
        .env("HOME", temp.path().join("empty-home"))
        .output()
        .unwrap();
    assert!(!output.status.success());
    let report: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(report["passed"], false);
}

#[test]
fn headless_without_a_task_fails_before_credential_loading() {
    let temp = tempfile::tempdir().unwrap();
    let output = Command::new(env!("CARGO_BIN_EXE_crosstalk"))
        .arg("--headless")
        .env("HOME", temp.path().join("empty-home"))
        .env("XDG_STATE_HOME", temp.path().join("state"))
        .output()
        .unwrap();
    assert!(!output.status.success());
    assert!(String::from_utf8_lossy(&output.stderr).contains("--headless requires --task"));
    assert!(!temp.path().join("state/crosstalk").exists());
}
