//! Reproducible, content-addressed export for a completed investigation.

use crate::types::conversation::ConversationState;
use crate::types::investigation::{ChainOfEvidenceAudit, ScientificReleaseAssessment};
use anyhow::{Context, Result, anyhow};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::collections::BTreeSet;
use std::io::Read;
use std::path::{Path, PathBuf};

pub const BUNDLE_SCHEMA: &str = "crosstalk.bundle.v1";
pub const REPORT_SCHEMA: &str = "crosstalk.report.v2";
const MAX_MANIFEST_BYTES: u64 = 4 * 1024 * 1024;
const MAX_STATE_BYTES: usize = 256 * 1024 * 1024;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct BundleFile {
    pub path: String,
    pub media_type: String,
    pub content_sha256: String,
    pub size_bytes: usize,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BundleManifest {
    pub schema: String,
    pub session_id: String,
    pub generated_at: u64,
    pub transcript_chain_head: String,
    pub state_sha256: String,
    pub investigation_schema: String,
    /// Identifies the deterministic report renderer. Legacy v1 manifests did
    /// not carry this field, so their report is integrity-checked by its
    /// manifest hash without re-rendering it through newer presentation code.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub report_schema: Option<String>,
    pub audit: ChainOfEvidenceAudit,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub scientific_release: Option<ScientificReleaseAssessment>,
    pub files: Vec<BundleFile>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct BundleVerification {
    pub passed: bool,
    pub checked_files: usize,
    pub issues: Vec<String>,
}

#[derive(Debug, Clone, Copy)]
pub struct BundleOptions {
    pub include_transcript: bool,
    pub include_artifacts: bool,
}

impl Default for BundleOptions {
    fn default() -> Self {
        Self {
            include_transcript: true,
            include_artifacts: true,
        }
    }
}

pub struct InvestigationBundleExporter;

impl InvestigationBundleExporter {
    pub fn export(
        state: &ConversationState,
        output_dir: &Path,
        options: BundleOptions,
    ) -> Result<BundleManifest> {
        validate_output_dir(output_dir)?;
        if let Some(index) = state.verify_chain() {
            return Err(anyhow!(
                "refusing to export a broken transcript chain at retained index {index}"
            ));
        }
        prepare_output_dir(output_dir)?;

        let audit = state.investigation.audit(&state.claim_ledger);
        let scientific_release = state
            .investigation
            .scientific_release_assessment(&state.claim_ledger);
        let state_bytes = serde_json::to_vec(state)?;
        if state_bytes.len() > MAX_STATE_BYTES {
            return Err(anyhow!(
                "serialized investigation state exceeds {MAX_STATE_BYTES} bytes"
            ));
        }
        let mut files = Vec::new();

        write_bytes(
            output_dir,
            "state.json",
            &state_bytes,
            "application/json",
            &mut files,
        )?;
        write_json(
            output_dir,
            "investigation.json",
            &state.investigation,
            &mut files,
        )?;
        write_json(output_dir, "claims.json", &state.claim_ledger, &mut files)?;
        write_json(output_dir, "audit.json", &audit, &mut files)?;
        if options.include_transcript {
            write_json(output_dir, "transcript.json", &state.turns, &mut files)?;
        }

        if options.include_artifacts {
            let artifact_dir = output_dir.join("artifacts");
            std::fs::create_dir_all(&artifact_dir)?;
            for (name, artifact) in &state.artifacts {
                let digest = sha256(artifact.content.as_bytes());
                let safe_name = safe_artifact_name(name);
                let name_digest = sha256(name.as_bytes());
                let relative = format!("artifacts/{digest}-{}-{safe_name}", &name_digest[..16]);
                write_bytes(
                    output_dir,
                    &relative,
                    artifact.content.as_bytes(),
                    media_type_for_language(&artifact.language),
                    &mut files,
                )?;
            }
        }

        let report = render_report(state, &audit);
        write_bytes(
            output_dir,
            "report.md",
            report.as_bytes(),
            "text/markdown",
            &mut files,
        )?;

        files.sort_by(|left, right| left.path.cmp(&right.path));
        let manifest = BundleManifest {
            schema: BUNDLE_SCHEMA.into(),
            session_id: state.session_id.clone(),
            generated_at: now(),
            transcript_chain_head: state.chain_head_hex(),
            state_sha256: sha256(&state_bytes),
            investigation_schema: state.investigation.schema.clone(),
            report_schema: Some(REPORT_SCHEMA.into()),
            audit,
            scientific_release: Some(scientific_release),
            files,
        };
        let manifest_bytes = serde_json::to_vec_pretty(&manifest)?;
        std::fs::write(output_dir.join("manifest.json"), manifest_bytes)
            .context("failed to write bundle manifest")?;
        Ok(manifest)
    }

    /// Verify a bundle without executing any contained artifact. This checks
    /// the manifest, exact file set, content hashes/sizes, serialized state,
    /// transcript chain, and evidence audit.
    pub fn verify(output_dir: &Path) -> Result<BundleVerification> {
        validate_output_dir(output_dir)?;
        let metadata =
            std::fs::symlink_metadata(output_dir).context("failed to inspect bundle directory")?;
        if metadata.file_type().is_symlink() || !metadata.is_dir() {
            return Err(anyhow!("bundle path must be a real directory"));
        }
        let manifest_path = output_dir.join("manifest.json");
        let manifest_metadata = std::fs::symlink_metadata(&manifest_path)
            .context("failed to inspect bundle manifest")?;
        if manifest_metadata.file_type().is_symlink()
            || !manifest_metadata.is_file()
            || manifest_metadata.len() > MAX_MANIFEST_BYTES
        {
            return Err(anyhow!(
                "bundle manifest must be a regular file no larger than {MAX_MANIFEST_BYTES} bytes"
            ));
        }
        let manifest_bytes =
            std::fs::read(&manifest_path).context("failed to read bundle manifest")?;
        let manifest: BundleManifest =
            serde_json::from_slice(&manifest_bytes).context("failed to parse bundle manifest")?;
        let mut issues = Vec::new();
        if manifest.schema != BUNDLE_SCHEMA {
            issues.push(format!("unsupported bundle schema: {}", manifest.schema));
        }

        let mut expected = BTreeSet::from(["manifest.json".to_string()]);
        for file in &manifest.files {
            if safe_join(output_dir, &file.path).is_err() {
                issues.push(format!("unsafe manifest path: {}", file.path));
                continue;
            }
            if !expected.insert(file.path.clone()) {
                issues.push(format!("duplicate manifest path: {}", file.path));
                continue;
            }
            let path = output_dir.join(&file.path);
            let Ok(metadata) = std::fs::symlink_metadata(&path) else {
                issues.push(format!("missing declared file: {}", file.path));
                continue;
            };
            if metadata.file_type().is_symlink() || !metadata.is_file() {
                issues.push(format!(
                    "declared path is not a regular file: {}",
                    file.path
                ));
                continue;
            }
            match hash_file(&path) {
                Ok((digest, size)) => {
                    if size != file.size_bytes as u64 {
                        issues.push(format!("size mismatch: {}", file.path));
                    }
                    if digest != file.content_sha256 {
                        issues.push(format!("content hash mismatch: {}", file.path));
                    }
                }
                Err(error) => issues.push(format!("could not read {}: {error}", file.path)),
            }
        }

        let actual = collect_bundle_paths(output_dir, &mut issues)?;
        for extra in actual.difference(&expected) {
            issues.push(format!("unexpected file: {extra}"));
        }
        for missing in expected.difference(&actual) {
            issues.push(format!("missing file: {missing}"));
        }

        let state_path = output_dir.join("state.json");
        let state_metadata = std::fs::symlink_metadata(&state_path);
        let state_is_regular = state_metadata
            .as_ref()
            .is_ok_and(|metadata| metadata.is_file() && !metadata.file_type().is_symlink());
        if !state_is_regular {
            issues.push("state.json is missing or is not a regular file".into());
        }
        let state_too_large = state_metadata
            .as_ref()
            .is_ok_and(|metadata| metadata.len() > MAX_STATE_BYTES as u64);
        if state_too_large {
            issues.push(format!(
                "state.json exceeds the {MAX_STATE_BYTES}-byte verification limit"
            ));
        }
        match (state_is_regular && !state_too_large).then(|| std::fs::read(&state_path)) {
            Some(Ok(state_bytes)) => {
                if sha256(&state_bytes) != manifest.state_sha256 {
                    issues.push("serialized state hash does not match the manifest".into());
                }
                match serde_json::from_slice::<ConversationState>(&state_bytes) {
                    Ok(state) => {
                        if state.session_id != manifest.session_id {
                            issues.push("state session id does not match the manifest".into());
                        }
                        if state.chain_head_hex() != manifest.transcript_chain_head {
                            issues.push("transcript chain head does not match the manifest".into());
                        }
                        if let Some(index) = state.verify_chain() {
                            issues.push(format!(
                                "transcript chain verification failed at retained index {index}"
                            ));
                        }
                        if state.investigation.schema != manifest.investigation_schema {
                            issues.push("investigation schema does not match the manifest".into());
                        }
                        if state.investigation.audit(&state.claim_ledger) != manifest.audit {
                            issues
                                .push("evidence audit does not match the serialized state".into());
                        }
                        if let Some(manifest_release) = &manifest.scientific_release
                            && state
                                .investigation
                                .scientific_release_assessment(&state.claim_ledger)
                                != *manifest_release
                        {
                            issues.push(
                                "scientific release assessment does not match the serialized state"
                                    .into(),
                            );
                        }
                        verify_json_projection(
                            output_dir,
                            "investigation.json",
                            &state.investigation,
                            &mut issues,
                        );
                        verify_json_projection(
                            output_dir,
                            "claims.json",
                            &state.claim_ledger,
                            &mut issues,
                        );
                        let state_audit = state.investigation.audit(&state.claim_ledger);
                        verify_json_projection(output_dir, "audit.json", &state_audit, &mut issues);
                        if expected.contains("transcript.json") {
                            verify_json_projection(
                                output_dir,
                                "transcript.json",
                                &state.turns,
                                &mut issues,
                            );
                        }
                        if expected.contains("report.md") {
                            match manifest.report_schema.as_deref() {
                                Some(REPORT_SCHEMA) => {
                                    let expected_report = render_report(&state, &state_audit);
                                    match read_bounded(
                                        &output_dir.join("report.md"),
                                        MAX_STATE_BYTES,
                                    ) {
                                        Ok(report) if report == expected_report.as_bytes() => {}
                                        Ok(_) => issues.push(
                                            "report.md does not match its declared report schema"
                                                .into(),
                                        ),
                                        Err(error) => issues
                                            .push(format!("could not validate report.md: {error}")),
                                    }
                                }
                                Some(schema) => {
                                    issues.push(format!("unsupported report schema: {schema}"));
                                }
                                None => {
                                    // Legacy bundles predate versioned report renderers. The
                                    // manifest hash above remains the authoritative integrity
                                    // check; all JSON projections are still recomputed.
                                }
                            }
                        }
                    }
                    Err(error) => issues.push(format!("serialized state is invalid: {error}")),
                }
            }
            Some(Err(error)) => issues.push(format!("could not read state.json: {error}")),
            None => {}
        }

        Ok(BundleVerification {
            passed: issues.is_empty(),
            checked_files: manifest.files.len(),
            issues,
        })
    }
}

fn write_json<T: Serialize>(
    root: &Path,
    relative: &str,
    value: &T,
    files: &mut Vec<BundleFile>,
) -> Result<()> {
    let bytes = serde_json::to_vec_pretty(value)?;
    write_bytes(root, relative, &bytes, "application/json", files)
}

fn write_bytes(
    root: &Path,
    relative: &str,
    bytes: &[u8],
    media_type: &str,
    files: &mut Vec<BundleFile>,
) -> Result<()> {
    let path = safe_join(root, relative)?;
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    std::fs::write(&path, bytes)
        .with_context(|| format!("failed to write bundle file: {}", path.display()))?;
    files.push(BundleFile {
        path: relative.into(),
        media_type: media_type.into(),
        content_sha256: sha256(bytes),
        size_bytes: bytes.len(),
    });
    Ok(())
}

fn safe_join(root: &Path, relative: &str) -> Result<PathBuf> {
    let path = Path::new(relative);
    if path.is_absolute()
        || path.components().any(|component| {
            matches!(
                component,
                std::path::Component::ParentDir
                    | std::path::Component::RootDir
                    | std::path::Component::Prefix(_)
            )
        })
    {
        return Err(anyhow!("unsafe bundle path: {relative}"));
    }
    Ok(root.join(path))
}

fn validate_output_dir(output_dir: &Path) -> Result<()> {
    if output_dir.as_os_str().is_empty() {
        return Err(anyhow!("bundle output directory must not be empty"));
    }
    if output_dir == Path::new("/") {
        return Err(anyhow!(
            "refusing to use the filesystem root as a bundle directory"
        ));
    }
    Ok(())
}

fn prepare_output_dir(output_dir: &Path) -> Result<()> {
    match std::fs::symlink_metadata(output_dir) {
        Ok(metadata) => {
            if metadata.file_type().is_symlink() {
                return Err(anyhow!("bundle output directory must not be a symlink"));
            }
            if !metadata.is_dir() {
                return Err(anyhow!("bundle output path is not a directory"));
            }
            if std::fs::read_dir(output_dir)?.next().is_some() {
                return Err(anyhow!(
                    "bundle output directory must be empty to prevent stale files"
                ));
            }
        }
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            std::fs::create_dir_all(output_dir).with_context(|| {
                format!(
                    "failed to create bundle directory: {}",
                    output_dir.display()
                )
            })?;
        }
        Err(error) => return Err(error).context("failed to inspect bundle output directory"),
    }
    Ok(())
}

fn collect_bundle_paths(root: &Path, issues: &mut Vec<String>) -> Result<BTreeSet<String>> {
    fn visit(
        root: &Path,
        directory: &Path,
        paths: &mut BTreeSet<String>,
        issues: &mut Vec<String>,
    ) -> Result<()> {
        for entry in std::fs::read_dir(directory)? {
            let entry = entry?;
            let path = entry.path();
            let metadata = std::fs::symlink_metadata(&path)?;
            let relative = path
                .strip_prefix(root)
                .context("bundle entry escaped its root")?
                .to_string_lossy()
                .replace(std::path::MAIN_SEPARATOR, "/");
            if metadata.file_type().is_symlink() {
                issues.push(format!("symlink is forbidden in bundle: {relative}"));
            } else if metadata.is_dir() {
                visit(root, &path, paths, issues)?;
            } else if metadata.is_file() {
                paths.insert(relative);
            } else {
                issues.push(format!("unsupported bundle entry: {relative}"));
            }
        }
        Ok(())
    }

    let mut paths = BTreeSet::new();
    visit(root, root, &mut paths, issues)?;
    Ok(paths)
}

fn hash_file(path: &Path) -> Result<(String, u64)> {
    let mut file = std::fs::File::open(path)?;
    let mut hasher = Sha256::new();
    let mut size = 0u64;
    let mut buffer = [0u8; 64 * 1024];
    loop {
        let read = file.read(&mut buffer)?;
        if read == 0 {
            break;
        }
        size = size.saturating_add(read as u64);
        hasher.update(&buffer[..read]);
    }
    Ok((format!("{:x}", hasher.finalize()), size))
}

fn read_bounded(path: &Path, limit: usize) -> Result<Vec<u8>> {
    let metadata = std::fs::symlink_metadata(path)?;
    if metadata.file_type().is_symlink() || !metadata.is_file() || metadata.len() > limit as u64 {
        return Err(anyhow!("file is not regular or exceeds {limit} bytes"));
    }
    Ok(std::fs::read(path)?)
}

fn verify_json_projection<T: Serialize>(
    root: &Path,
    relative: &str,
    expected: &T,
    issues: &mut Vec<String>,
) {
    let result = read_bounded(&root.join(relative), MAX_STATE_BYTES).and_then(|bytes| {
        let actual: serde_json::Value = serde_json::from_slice(&bytes)?;
        let expected = serde_json::to_value(expected)?;
        Ok(actual == expected)
    });
    match result {
        Ok(true) => {}
        Ok(false) => issues.push(format!("{relative} does not match state.json")),
        Err(error) => issues.push(format!("could not validate {relative}: {error}")),
    }
}

fn safe_artifact_name(name: &str) -> String {
    let mut value = name
        .chars()
        .map(|character| {
            if character.is_ascii_alphanumeric() || matches!(character, '.' | '-' | '_') {
                character
            } else {
                '_'
            }
        })
        .collect::<String>();
    if value.is_empty() {
        value.push_str("artifact.bin");
    }
    while value.contains("..") {
        value = value.replace("..", "_");
    }
    value.truncate(value.floor_char_boundary(value.len().min(160)));
    value
}

fn media_type_for_language(language: &str) -> &'static str {
    match language.to_ascii_lowercase().as_str() {
        "json" => "application/json",
        "markdown" | "md" => "text/markdown",
        "rust" | "rs" => "text/x-rust",
        "lean" | "lean4" => "text/x-lean",
        "coq" => "text/x-coq",
        "verus" => "text/x-rust",
        "python" | "py" => "text/x-python",
        _ => "text/plain",
    }
}

fn render_report(state: &ConversationState, audit: &ChainOfEvidenceAudit) -> String {
    let mut report = String::new();
    report.push_str("# Crosstalk investigation report\n\n");
    report.push_str(&format!("- Session: `{}`\n", state.session_id));
    report.push_str(&format!(
        "- Evidence integrity audit: **{}**\n",
        if audit.passed { "PASS" } else { "FAIL" }
    ));
    report.push_str(&format!(
        "- Verification coverage: {:.1}%\n",
        audit.verification_coverage * 100.0
    ));
    let release = state
        .investigation
        .scientific_release_assessment(&state.claim_ledger);
    report.push_str(&format!(
        "- Scientific release: **{}**\n",
        if release.eligible {
            "ELIGIBLE"
        } else {
            "NOT ESTABLISHED"
        }
    ));
    if let Some(warning) = release.unverified_warning() {
        report.push_str(&format!("\n> **Warning:** {warning}\n"));
    }
    report.push_str(&format!("- Claims: {}\n", audit.claim_count));
    report.push_str(&format!(
        "- Evidence artifacts: {}\n",
        state.investigation.evidence.len()
    ));
    report.push_str(&format!(
        "- Verification records: {}\n\n",
        state.investigation.verifications.len()
    ));
    report.push_str("## Problem\n\n");
    if state.investigation.problem.statement.trim().is_empty() {
        report.push_str("_No structured problem statement was recorded._\n\n");
    } else {
        report.push_str(&state.investigation.problem.statement);
        report.push_str("\n\n");
    }

    report.push_str("## Claims\n\n");
    if state.claim_ledger.claims.is_empty() {
        report.push_str("_No explicitly tagged claims were recorded._\n\n");
    } else {
        for claim in state.claim_ledger.claims.values() {
            report.push_str(&format!(
                "- **{:?} / {:?}** `{}` — {}\n",
                claim.kind, claim.status, claim.id, claim.text
            ));
            for evidence in &claim.evidence {
                report.push_str(&format!(
                    "  - evidence `{}` (supports={}, strength={:.2})\n",
                    evidence.source_id, evidence.supports, evidence.strength
                ));
            }
        }
        report.push('\n');
    }

    report.push_str("## Verification records\n\n");
    if state.investigation.verifications.is_empty() {
        report.push_str("_No objective verification records were recorded._\n\n");
    } else {
        for verification in state.investigation.verifications.values() {
            report.push_str(&format!(
                "- **{:?}** `{}` on `{}` using `{}` `{}`\n",
                verification.status,
                verification.id,
                verification.subject_id,
                verification.evaluator_id,
                verification.evaluator_version
            ));
            for measurement in &verification.measurements {
                report.push_str(&format!(
                    "  - `{}` = {} {}\n",
                    measurement.name, measurement.value, measurement.unit
                ));
            }
        }
        report.push('\n');
    }

    report.push_str("## Audit findings\n\n");
    if audit.issues.is_empty() {
        report.push_str("No evidence-chain findings.\n");
    } else {
        for issue in &audit.issues {
            report.push_str(&format!(
                "- **{:?}** `{}` on `{}` — {}\n",
                issue.severity, issue.code, issue.subject_id, issue.message
            ));
        }
    }
    report
}

fn sha256(bytes: &[u8]) -> String {
    format!("{:x}", Sha256::digest(bytes))
}

fn now() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}
