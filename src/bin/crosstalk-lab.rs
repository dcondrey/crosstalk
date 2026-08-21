use anyhow::{Context, Result, bail};
use clap::{Parser, Subcommand};
use crosstalk::engines::algorithm_discovery::{
    ALGORITHM_CHALLENGE_SCHEMA, AlgorithmChallenge, AlgorithmDiscoveryLab,
};
use crosstalk::engines::objective_evaluation::{
    CandidateArtifact, EvaluationSpec, EvaluatorRegistry, WASM_I64_TEST_SCHEMA,
    WasmI64FunctionEvaluator,
};
use crosstalk::engines::sandbox::{I64TestCase, SandboxConfig, SandboxManager};
use serde::Deserialize;
use std::collections::BTreeMap;
use std::fs::{self, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::Arc;

const CHALLENGE_FILE_SCHEMA: &str = "crosstalk.algorithm-challenge-file.v1";
const MAX_CHALLENGE_BYTES: u64 = 1024 * 1024;
const MAX_TEST_FILE_BYTES: u64 = 8 * 1024 * 1024;
const MAX_WASM_BYTES: u64 = 32 * 1024 * 1024;

#[derive(Debug, Parser)]
#[command(
    name = "crosstalk-lab",
    version,
    about = "Fail-closed hidden-test tournaments for WASM algorithm candidates"
)]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Debug, Subcommand)]
enum Command {
    /// Compute the canonical commitment for a private test file.
    Commitment {
        #[arg(long)]
        hidden_tests: PathBuf,
    },
    /// Evaluate a baseline and candidates, then emit a non-secret report.
    Run {
        #[arg(long)]
        challenge: PathBuf,
        #[arg(long)]
        hidden_tests: PathBuf,
        #[arg(long)]
        baseline: PathBuf,
        /// Candidate in ID=PATH form. Repeat for each candidate.
        #[arg(long = "candidate", required = true)]
        candidates: Vec<String>,
        /// Create a report at this path. Existing files are never overwritten.
        #[arg(long)]
        output: Option<PathBuf>,
        #[arg(long, default_value_t = 256 * 1024 * 1024)]
        memory_bytes: usize,
        #[arg(long, default_value_t = 100_000_000)]
        fuel: u64,
        #[arg(long, default_value_t = 30)]
        timeout_secs: u64,
    },
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct HiddenTestFile {
    schema: String,
    export_name: String,
    cases: Vec<I64TestCase>,
}

impl HiddenTestFile {
    fn load(path: &Path) -> Result<Self> {
        let bytes = read_bounded(path, MAX_TEST_FILE_BYTES, "hidden-test file")?;
        let value: Self = serde_json::from_slice(&bytes)
            .with_context(|| format!("invalid hidden-test JSON: {}", path.display()))?;
        if value.schema != WASM_I64_TEST_SCHEMA {
            bail!("unsupported hidden-test schema: {}", value.schema);
        }
        WasmI64FunctionEvaluator::commitment_for(&value.export_name, &value.cases)?;
        Ok(value)
    }

    fn commitment(&self) -> Result<String> {
        Ok(WasmI64FunctionEvaluator::commitment_for(
            &self.export_name,
            &self.cases,
        )?)
    }
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct ChallengeFile {
    schema: String,
    id: String,
    title: String,
    evaluator_id: String,
    evaluation: EvaluationSpec,
    primary_metric: String,
    minimum_improvement: f64,
    hidden_test_commitment_sha256: String,
    baseline_id: String,
    max_candidates: usize,
}

#[tokio::main]
async fn main() -> Result<()> {
    match Cli::parse().command {
        Command::Commitment { hidden_tests } => {
            println!("{}", HiddenTestFile::load(&hidden_tests)?.commitment()?);
            Ok(())
        }
        Command::Run {
            challenge,
            hidden_tests,
            baseline,
            candidates,
            output,
            memory_bytes,
            fuel,
            timeout_secs,
        } => {
            run(
                &challenge,
                &hidden_tests,
                &baseline,
                &candidates,
                output.as_deref(),
                SandboxConfig {
                    memory_limit_bytes: memory_bytes,
                    cpu_fuel_limit: fuel,
                    timeout_secs,
                },
            )
            .await
        }
    }
}

async fn run(
    challenge_path: &Path,
    hidden_tests_path: &Path,
    baseline_path: &Path,
    candidate_args: &[String],
    output: Option<&Path>,
    sandbox_config: SandboxConfig,
) -> Result<()> {
    let challenge_bytes = read_bounded(challenge_path, MAX_CHALLENGE_BYTES, "challenge file")?;
    let definition: ChallengeFile = serde_json::from_slice(&challenge_bytes)
        .with_context(|| format!("invalid challenge JSON: {}", challenge_path.display()))?;
    if definition.schema != CHALLENGE_FILE_SCHEMA {
        bail!("unsupported challenge-file schema: {}", definition.schema);
    }
    let hidden_tests = HiddenTestFile::load(hidden_tests_path)?;
    let actual_commitment = hidden_tests.commitment()?;
    if !definition
        .hidden_test_commitment_sha256
        .eq_ignore_ascii_case(&actual_commitment)
    {
        bail!("challenge commitment does not match the supplied hidden-test file");
    }

    let baseline = CandidateArtifact {
        id: definition.baseline_id,
        media_type: "application/wasm".into(),
        content: read_bounded(baseline_path, MAX_WASM_BYTES, "baseline WASM")?,
        metadata: BTreeMap::new(),
    };
    let candidates = candidate_args
        .iter()
        .map(|argument| load_candidate(argument))
        .collect::<Result<Vec<_>>>()?;
    let challenge = AlgorithmChallenge {
        schema: ALGORITHM_CHALLENGE_SCHEMA.into(),
        id: definition.id,
        title: definition.title,
        evaluator_id: definition.evaluator_id,
        evaluation: definition.evaluation,
        primary_metric: definition.primary_metric,
        minimum_improvement: definition.minimum_improvement,
        hidden_test_commitment_sha256: actual_commitment,
        baseline,
        max_candidates: definition.max_candidates,
    };
    let reproduction_id = challenge
        .evaluation
        .reproduction_evaluator_id
        .clone()
        .context("challenge requires reproduction_evaluator_id")?;

    // Independent managers provide fresh engines, stores, epoch clocks, and
    // executions. Both are bound to the same committed cases.
    let primary_sandbox = Arc::new(SandboxManager::new(sandbox_config.clone())?);
    let reproduction_sandbox = Arc::new(SandboxManager::new(sandbox_config)?);
    let evaluator_version = format!("{}+wasmtime-29", env!("CARGO_PKG_VERSION"));
    let mut registry = EvaluatorRegistry::default();
    registry.register(Arc::new(WasmI64FunctionEvaluator::new(
        primary_sandbox,
        challenge.evaluator_id.clone(),
        evaluator_version.clone(),
        hidden_tests.export_name.clone(),
        hidden_tests.cases.clone(),
    )?))?;
    registry.register(Arc::new(WasmI64FunctionEvaluator::new(
        reproduction_sandbox,
        reproduction_id,
        evaluator_version,
        hidden_tests.export_name,
        hidden_tests.cases,
    )?))?;

    let report = AlgorithmDiscoveryLab::new(&registry)
        .run(&challenge, &candidates)
        .await?;
    let json = serde_json::to_vec_pretty(&report)?;
    if let Some(path) = output {
        let mut file = OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(path)
            .with_context(|| {
                format!(
                    "could not create report {} (existing files are not overwritten)",
                    path.display()
                )
            })?;
        file.write_all(&json)?;
        file.write_all(b"\n")?;
        file.sync_all()?;
    } else {
        std::io::stdout().write_all(&json)?;
        std::io::stdout().write_all(b"\n")?;
    }
    Ok(())
}

fn load_candidate(argument: &str) -> Result<CandidateArtifact> {
    let (id, path) = argument
        .split_once('=')
        .context("candidate must use ID=PATH syntax")?;
    if id.trim().is_empty() || path.trim().is_empty() {
        bail!("candidate ID and path must not be empty");
    }
    Ok(CandidateArtifact {
        id: id.to_owned(),
        media_type: "application/wasm".into(),
        content: read_bounded(Path::new(path), MAX_WASM_BYTES, "candidate WASM")?,
        metadata: BTreeMap::new(),
    })
}

fn read_bounded(path: &Path, max_bytes: u64, label: &str) -> Result<Vec<u8>> {
    let metadata = fs::metadata(path)
        .with_context(|| format!("could not inspect {label}: {}", path.display()))?;
    if !metadata.is_file() {
        bail!("{label} is not a regular file: {}", path.display());
    }
    if metadata.len() == 0 || metadata.len() > max_bytes {
        bail!("{label} must contain between 1 and {max_bytes} bytes");
    }
    let bytes = fs::read(path).with_context(|| format!("could not read {}", path.display()))?;
    if bytes.is_empty() || bytes.len() as u64 > max_bytes {
        bail!("{label} changed size while it was being read");
    }
    Ok(bytes)
}
