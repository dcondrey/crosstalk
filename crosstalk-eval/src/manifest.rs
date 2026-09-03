use anyhow::{Context, Result, bail};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use sha2::{Digest, Sha256};
use std::collections::BTreeSet;
use std::fs::OpenOptions;
use std::io::Write;
use std::path::Path;

pub const EVALUATION_MANIFEST_SCHEMA: &str = "crosstalk.evaluation-manifest.v1";
pub const EVALUATION_RESULT_SCHEMA: &str = "crosstalk.evaluation-result.v1";
const MAX_MANIFEST_BYTES: u64 = 1024 * 1024;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RunClassification {
    Benchmark,
    SmokeTest,
    DevelopmentSimulation,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TaskSetManifest {
    pub name: String,
    pub version: String,
    pub content_sha256: String,
    pub private: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ModelManifest {
    pub role: String,
    pub provider: String,
    pub model_id: String,
    pub model_version: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct EvaluationBudget {
    pub max_cost_usd: f64,
    pub max_wall_time_secs: u64,
    pub max_turns_per_task: u32,
    pub max_model_calls: u64,
    pub max_concurrency: usize,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct FailurePolicy {
    pub timeout_is_failure: bool,
    pub refusal_is_failure: bool,
    pub malformed_output_is_failure: bool,
    pub infrastructure_error_is_failure: bool,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct EvaluationManifest {
    pub schema: String,
    pub id: String,
    pub git_commit: String,
    pub classification: RunClassification,
    pub task_set: TaskSetManifest,
    pub models: Vec<ModelManifest>,
    pub seed: u64,
    pub budget: EvaluationBudget,
    pub failure_policy: FailurePolicy,
    pub baselines: Vec<String>,
    pub ablations: Vec<String>,
    pub created_at: u64,
}

impl EvaluationManifest {
    pub fn load(path: &Path) -> Result<Self> {
        let metadata = std::fs::symlink_metadata(path)
            .with_context(|| format!("could not inspect manifest {}", path.display()))?;
        if metadata.file_type().is_symlink() || !metadata.is_file() {
            bail!("evaluation manifest must be a regular non-symlink file");
        }
        if metadata.len() == 0 || metadata.len() > MAX_MANIFEST_BYTES {
            bail!("evaluation manifest must contain between 1 byte and 1 MiB");
        }
        let bytes = std::fs::read(path)?;
        let manifest: Self = serde_json::from_slice(&bytes)
            .with_context(|| format!("invalid evaluation manifest {}", path.display()))?;
        manifest.validate()?;
        Ok(manifest)
    }

    pub fn validate(&self) -> Result<()> {
        if self.schema != EVALUATION_MANIFEST_SCHEMA {
            bail!("unsupported evaluation manifest schema: {}", self.schema);
        }
        if self.id.trim().is_empty() || self.id.len() > 512 {
            bail!("evaluation manifest id is empty or too large");
        }
        if self.git_commit.len() < 7
            || self.git_commit.len() > 64
            || !self.git_commit.bytes().all(|byte| byte.is_ascii_hexdigit())
        {
            bail!("evaluation manifest git_commit is not a hexadecimal revision");
        }
        if self.task_set.name.trim().is_empty()
            || self.task_set.version.trim().is_empty()
            || self.task_set.name.len() > 1024
            || self.task_set.version.len() > 1024
        {
            bail!("task-set name and version are required");
        }
        validate_sha256(&self.task_set.content_sha256, "task-set content")?;
        if self.models.is_empty() || self.models.len() > 64 {
            bail!("evaluation manifest must declare between 1 and 64 models");
        }
        let mut roles = BTreeSet::new();
        for model in &self.models {
            if model.role.trim().is_empty()
                || model.provider.trim().is_empty()
                || model.model_id.trim().is_empty()
                || model.model_version.trim().is_empty()
                || !roles.insert(model.role.as_str())
            {
                bail!("models require unique roles and complete identities");
            }
        }
        if !self.budget.max_cost_usd.is_finite()
            || self.budget.max_cost_usd < 0.0
            || self.budget.max_wall_time_secs == 0
            || self.budget.max_turns_per_task == 0
            || self.budget.max_model_calls == 0
            || self.budget.max_concurrency == 0
        {
            bail!("evaluation budget must contain finite, positive hard limits");
        }
        if self.created_at == 0 {
            bail!("evaluation manifest creation time is required");
        }
        if self.classification == RunClassification::Benchmark
            && (self.baselines.is_empty() || self.ablations.is_empty())
        {
            bail!("benchmark manifests require declared baselines and ablations");
        }
        if self
            .baselines
            .iter()
            .chain(&self.ablations)
            .any(|name| name.trim().is_empty() || name.len() > 1024)
        {
            bail!("baseline and ablation names must not be empty");
        }
        Ok(())
    }

    pub fn sha256(&self) -> Result<String> {
        self.validate()?;
        Ok(sha256(&serde_json::to_vec(self)?))
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RunStatus {
    Completed,
    Cancelled,
    Failed,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct EvaluationResultManifest {
    pub schema: String,
    pub manifest_id: String,
    pub manifest_sha256: String,
    pub classification: RunClassification,
    pub started_at: u64,
    pub completed_at: u64,
    pub status: RunStatus,
    pub total_tasks: usize,
    pub completed_tasks: usize,
    pub successful_tasks: usize,
    pub failed_tasks: usize,
    pub actual_cost_usd: f64,
    pub result_payload_sha256: String,
    pub diagnostics: String,
}

impl EvaluationResultManifest {
    pub fn validate(&self) -> Result<()> {
        if self.schema != EVALUATION_RESULT_SCHEMA || self.manifest_id.trim().is_empty() {
            bail!("evaluation result schema or manifest identity is invalid");
        }
        validate_sha256(&self.manifest_sha256, "manifest")?;
        validate_sha256(&self.result_payload_sha256, "result payload")?;
        if self.completed_at < self.started_at
            || self.completed_tasks > self.total_tasks
            || self.successful_tasks.saturating_add(self.failed_tasks) != self.completed_tasks
            || !self.actual_cost_usd.is_finite()
            || self.actual_cost_usd < 0.0
            || self.diagnostics.len() > 64 * 1024
        {
            bail!("evaluation result contains inconsistent counts, times, cost, or diagnostics");
        }
        if self.status == RunStatus::Completed && self.completed_tasks != self.total_tasks {
            bail!("a completed evaluation result must account for every task");
        }
        Ok(())
    }
}

/// Self-contained, hash-bound evaluation output. The result metadata can be
/// verified without trusting the producer, while `payload` remains flexible
/// enough for different benchmark families.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct EvaluationArtifact {
    pub result: EvaluationResultManifest,
    pub payload: Value,
}

impl EvaluationArtifact {
    pub fn new(mut result: EvaluationResultManifest, payload: Value) -> Result<Self> {
        result.result_payload_sha256 = sha256(&serde_json::to_vec(&payload)?);
        let artifact = Self { result, payload };
        artifact.validate()?;
        Ok(artifact)
    }

    pub fn validate(&self) -> Result<()> {
        self.result.validate()?;
        let actual = sha256(&serde_json::to_vec(&self.payload)?);
        if actual != self.result.result_payload_sha256 {
            bail!("evaluation result payload does not match its SHA-256 commitment");
        }
        Ok(())
    }

    /// Write once so an existing result cannot be silently replaced.
    pub fn write_create_new(&self, path: &Path) -> Result<()> {
        self.validate()?;
        if let Some(parent) = path
            .parent()
            .filter(|parent| !parent.as_os_str().is_empty())
        {
            std::fs::create_dir_all(parent)?;
        }
        let bytes = serde_json::to_vec_pretty(self)?;
        let mut file = OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(path)
            .with_context(|| {
                format!(
                    "could not create evaluation result {} (results are create-only)",
                    path.display()
                )
            })?;
        file.write_all(&bytes)?;
        file.sync_all()?;
        Ok(())
    }
}

pub fn file_sha256(path: &Path) -> Result<String> {
    let metadata = std::fs::symlink_metadata(path)?;
    if metadata.file_type().is_symlink() || !metadata.is_file() {
        bail!("task-set path must be a regular non-symlink file");
    }
    let mut file = std::fs::File::open(path)?;
    let mut hash = Sha256::new();
    std::io::copy(&mut file, &mut hash)?;
    Ok(format!("{:x}", hash.finalize()))
}

fn validate_sha256(value: &str, label: &str) -> Result<()> {
    if value.len() != 64 || !value.bytes().all(|byte| byte.is_ascii_hexdigit()) {
        bail!("{label} digest is not SHA-256");
    }
    Ok(())
}

fn sha256(bytes: &[u8]) -> String {
    format!("{:x}", Sha256::digest(bytes))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn manifest(classification: RunClassification) -> EvaluationManifest {
        EvaluationManifest {
            schema: EVALUATION_MANIFEST_SCHEMA.into(),
            id: "swebench-v1".into(),
            git_commit: "a".repeat(40),
            classification,
            task_set: TaskSetManifest {
                name: "SWE-bench".into(),
                version: "verified".into(),
                content_sha256: "b".repeat(64),
                private: false,
            },
            models: vec![ModelManifest {
                role: "solver".into(),
                provider: "provider".into(),
                model_id: "model".into(),
                model_version: "2026-01".into(),
            }],
            seed: 42,
            budget: EvaluationBudget {
                max_cost_usd: 100.0,
                max_wall_time_secs: 3600,
                max_turns_per_task: 10,
                max_model_calls: 1_000,
                max_concurrency: 4,
            },
            failure_policy: FailurePolicy {
                timeout_is_failure: true,
                refusal_is_failure: true,
                malformed_output_is_failure: true,
                infrastructure_error_is_failure: true,
            },
            baselines: vec!["single-model".into()],
            ablations: vec!["without-critique".into()],
            created_at: 1,
        }
    }

    #[test]
    fn benchmark_manifest_is_stable_and_hashed() {
        let value = manifest(RunClassification::Benchmark);
        value.validate().unwrap();
        assert_eq!(value.sha256().unwrap().len(), 64);
        assert_eq!(
            value.sha256().unwrap(),
            serde_json::from_str::<EvaluationManifest>(&serde_json::to_string(&value).unwrap())
                .unwrap()
                .sha256()
                .unwrap()
        );
    }

    #[test]
    fn benchmark_requires_baselines_and_ablations() {
        let mut value = manifest(RunClassification::Benchmark);
        value.ablations.clear();
        assert!(value.validate().is_err());
        value.classification = RunClassification::DevelopmentSimulation;
        value.validate().unwrap();
    }

    #[test]
    fn completed_results_must_account_for_every_task() {
        let value = EvaluationResultManifest {
            schema: EVALUATION_RESULT_SCHEMA.into(),
            manifest_id: "run".into(),
            manifest_sha256: "a".repeat(64),
            classification: RunClassification::Benchmark,
            started_at: 1,
            completed_at: 2,
            status: RunStatus::Completed,
            total_tasks: 2,
            completed_tasks: 1,
            successful_tasks: 1,
            failed_tasks: 0,
            actual_cost_usd: 1.0,
            result_payload_sha256: "b".repeat(64),
            diagnostics: String::new(),
        };
        assert!(value.validate().is_err());
    }

    #[test]
    fn artifact_detects_payload_mutation() {
        let result = EvaluationResultManifest {
            schema: EVALUATION_RESULT_SCHEMA.into(),
            manifest_id: "run".into(),
            manifest_sha256: "a".repeat(64),
            classification: RunClassification::SmokeTest,
            started_at: 1,
            completed_at: 2,
            status: RunStatus::Completed,
            total_tasks: 1,
            completed_tasks: 1,
            successful_tasks: 1,
            failed_tasks: 0,
            actual_cost_usd: 1.0,
            result_payload_sha256: String::new(),
            diagnostics: String::new(),
        };
        let mut artifact =
            EvaluationArtifact::new(result, serde_json::json!({"resolved": true})).unwrap();
        artifact.payload = serde_json::json!({"resolved": false});
        assert!(artifact.validate().is_err());
    }
}
