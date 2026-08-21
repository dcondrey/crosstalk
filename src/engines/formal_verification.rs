//! Sandboxed adapters for trusted, external proof checkers.
//!
//! Crosstalk never interprets model prose as a proof.  A proof is verified only
//! when an installed checker exits successfully under the configured policy.

use anyhow::{Context, Result, anyhow};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::path::{Path, PathBuf};
use std::time::Duration;

const MAX_PROOF_BYTES: usize = 2 * 1024 * 1024;
const MAX_DIAGNOSTIC_BYTES: usize = 64 * 1024;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum ProofBackend {
    Lean4,
    Verus,
    Coq,
}

impl ProofBackend {
    fn executable(self) -> &'static str {
        match self {
            Self::Lean4 => "lean",
            Self::Verus => "verus",
            Self::Coq => "coqc",
        }
    }
    fn extension(self) -> &'static str {
        match self {
            Self::Lean4 => "lean",
            Self::Verus => "rs",
            Self::Coq => "v",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProofPolicy {
    pub timeout_secs: u64,
    pub reject_placeholders: bool,
}

impl Default for ProofPolicy {
    fn default() -> Self {
        Self {
            timeout_secs: 30,
            reject_placeholders: true,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum ProofStatus {
    Verified,
    Rejected,
    CheckerUnavailable,
    TimedOut,
    PolicyViolation,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProofVerification {
    pub backend: ProofBackend,
    pub status: ProofStatus,
    pub source_sha256: String,
    pub diagnostics: String,
    pub checker_path: Option<PathBuf>,
    #[serde(default)]
    pub checker_version: Option<String>,
}

impl ProofVerification {
    #[must_use]
    pub fn is_verified(&self) -> bool {
        self.status == ProofStatus::Verified
    }
}

#[derive(Default)]
pub struct FormalProofVerifier {
    pub policy: ProofPolicy,
}

impl FormalProofVerifier {
    #[must_use]
    pub fn available_backends() -> Vec<ProofBackend> {
        [ProofBackend::Lean4, ProofBackend::Verus, ProofBackend::Coq]
            .into_iter()
            .filter(|b| which::which(b.executable()).is_ok())
            .collect()
    }

    pub async fn verify_source(
        &self,
        backend: ProofBackend,
        source: &str,
    ) -> Result<ProofVerification> {
        if source.len() > MAX_PROOF_BYTES {
            return Err(anyhow!("proof source exceeds {MAX_PROOF_BYTES} bytes"));
        }
        let digest = format!("{:x}", Sha256::digest(source.as_bytes()));
        if self.policy.reject_placeholders
            && let Some(token) = forbidden_placeholder(backend, source)
        {
            return Ok(ProofVerification {
                backend,
                status: ProofStatus::PolicyViolation,
                source_sha256: digest,
                diagnostics: format!("untrusted proof placeholder rejected: {token}"),
                checker_path: None,
                checker_version: None,
            });
        }
        let checker = match which::which(backend.executable()) {
            Ok(path) => path,
            Err(_) => {
                return Ok(ProofVerification {
                    backend,
                    status: ProofStatus::CheckerUnavailable,
                    source_sha256: digest,
                    diagnostics: format!(
                        "{} is not installed or not on PATH",
                        backend.executable()
                    ),
                    checker_path: None,
                    checker_version: None,
                });
            }
        };
        let checker_version = read_checker_version(&checker).await;
        let temp_dir = restricted_temp_dir()?;
        let proof_file = temp_dir.join(format!("Main.{}", backend.extension()));
        tokio::fs::write(&proof_file, source)
            .await
            .context("failed to write temporary proof")?;
        let mut command = tokio::process::Command::new(&checker);
        command
            .arg(&proof_file)
            .current_dir(&temp_dir)
            .kill_on_drop(true);
        command.env_clear();
        if let Some(path) = std::env::var_os("PATH") {
            command.env("PATH", path);
        }
        if let Some(home) = std::env::var_os("HOME") {
            command.env("HOME", home);
        }
        let result = tokio::time::timeout(
            Duration::from_secs(self.policy.timeout_secs.max(1)),
            command.output(),
        )
        .await;
        let verification = match result {
            Err(_) => ProofVerification {
                backend,
                status: ProofStatus::TimedOut,
                source_sha256: digest,
                diagnostics: "proof checker exceeded time limit".into(),
                checker_path: Some(checker),
                checker_version,
            },
            Ok(Err(error)) => return Err(error).context("failed to execute proof checker"),
            Ok(Ok(output)) => {
                let mut diagnostics = String::from_utf8_lossy(&output.stdout).into_owned();
                diagnostics.push_str(&String::from_utf8_lossy(&output.stderr));
                diagnostics.truncate(diagnostics.floor_char_boundary(MAX_DIAGNOSTIC_BYTES));
                ProofVerification {
                    backend,
                    status: if output.status.success() {
                        ProofStatus::Verified
                    } else {
                        ProofStatus::Rejected
                    },
                    source_sha256: digest,
                    diagnostics,
                    checker_path: Some(checker),
                    checker_version,
                }
            }
        };
        if let Err(error) = tokio::fs::remove_dir_all(&temp_dir).await {
            tracing::warn!(%error, path = %temp_dir.display(), "failed to remove proof scratch directory");
        }
        Ok(verification)
    }

    pub async fn verify_file(
        &self,
        backend: ProofBackend,
        path: &Path,
    ) -> Result<ProofVerification> {
        let source = tokio::fs::read_to_string(path)
            .await
            .context("failed to read proof source")?;
        self.verify_source(backend, &source).await
    }
}

async fn read_checker_version(checker: &Path) -> Option<String> {
    let mut command = tokio::process::Command::new(checker);
    command.arg("--version").env_clear().kill_on_drop(true);
    if let Some(path) = std::env::var_os("PATH") {
        command.env("PATH", path);
    }
    let output = tokio::time::timeout(Duration::from_secs(5), command.output())
        .await
        .ok()?
        .ok()?;
    let mut version = String::from_utf8_lossy(&output.stdout).into_owned();
    if version.trim().is_empty() {
        version = String::from_utf8_lossy(&output.stderr).into_owned();
    }
    version.truncate(version.floor_char_boundary(1024));
    let version = version.trim();
    (!version.is_empty()).then(|| version.to_string())
}

fn forbidden_placeholder(backend: ProofBackend, source: &str) -> Option<&'static str> {
    let normalized = source.to_ascii_lowercase();
    let tokens: &[(&str, &str)] = match backend {
        ProofBackend::Lean4 => &[("sorry", "sorry"), ("by?", "by?"), ("admit", "admit")],
        ProofBackend::Verus => &[
            ("assume(false)", "assume(false)"),
            ("external_body", "external_body"),
        ],
        ProofBackend::Coq => &[("admitted.", "Admitted."), ("admit.", "admit.")],
    };
    tokens
        .iter()
        .find_map(|(needle, label)| normalized.contains(needle).then_some(*label))
}

fn restricted_temp_dir() -> Result<PathBuf> {
    let nonce = format!(
        "{}-{}",
        std::process::id(),
        chrono::Utc::now().timestamp_nanos_opt().unwrap_or_default()
    );
    let path = std::env::temp_dir().join(format!("crosstalk-proof-{nonce}"));
    #[cfg(unix)]
    {
        use std::os::unix::fs::DirBuilderExt;
        std::fs::DirBuilder::new().mode(0o700).create(&path)?;
    }
    #[cfg(not(unix))]
    std::fs::create_dir(&path)?;
    Ok(path)
}
