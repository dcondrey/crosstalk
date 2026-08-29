use anyhow::{Context, Result, bail};
use clap::{Parser, Subcommand};
use crosstalk::engines::algorithm_discovery::{
    ALGORITHM_CHALLENGE_SCHEMA, AlgorithmChallenge, AlgorithmDiscoveryLab,
};
use crosstalk::engines::objective_evaluation::{
    CandidateArtifact, EvaluationSpec, EvaluatorRegistry, ObjectiveEvaluator, WASM_I64_TEST_SCHEMA,
    WasmI64FunctionEvaluator,
};
use crosstalk::engines::sandbox::{I64TestCase, SandboxConfig, SandboxManager};
use crosstalk::engines::sealed_evaluation::{ProcessSealedTransport, SealedEvaluatorClient};
use ed25519_dalek::VerifyingKey;
use serde::Deserialize;
use std::collections::BTreeMap;
use std::ffi::OsString;
use std::fs::{self, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::Arc;

const CHALLENGE_FILE_SCHEMA: &str = "crosstalk.algorithm-challenge-file.v1";
const MAX_CHALLENGE_BYTES: u64 = 1024 * 1024;
const MAX_TEST_FILE_BYTES: u64 = 8 * 1024 * 1024;
const MAX_WASM_BYTES: u64 = 32 * 1024 * 1024;
const WORKER_ENDPOINT_SCHEMA: &str = "crosstalk.sealed-worker-endpoint.v1";

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
    /// Run without access to holdouts, using two signed worker processes.
    RunSealed {
        #[arg(long)]
        challenge: PathBuf,
        #[arg(long)]
        primary_endpoint: PathBuf,
        #[arg(long)]
        reproduction_endpoint: PathBuf,
        #[arg(long)]
        baseline: PathBuf,
        #[arg(long = "candidate", required = true)]
        candidates: Vec<String>,
        #[arg(long)]
        output: Option<PathBuf>,
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
    /// Per-artifact byte cap. IMPORTANT: a challenge whose answers are public
    /// data (the rule 30 centre column is published to 10^9 bits) is winnable
    /// by an embedded lookup table, so the cap is what separates an algorithm
    /// from a transcript. `MAX_WASM_BYTES` alone is 32 MiB, or ~2.7x10^8
    /// tabulated bits.
    #[serde(default)]
    max_candidate_bytes: Option<u64>,
}

impl ChallengeFile {
    fn artifact_byte_cap(&self) -> Result<u64> {
        match self.max_candidate_bytes {
            None => Ok(MAX_WASM_BYTES),
            Some(0) => bail!("max_candidate_bytes must be greater than zero"),
            Some(cap) if cap > MAX_WASM_BYTES => bail!(
                "max_candidate_bytes {cap} exceeds the transport limit {MAX_WASM_BYTES}"
            ),
            Some(cap) => Ok(cap),
        }
    }
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct WorkerEndpointFile {
    schema: String,
    evaluator_id: String,
    evaluator_version: String,
    test_commitment_sha256: String,
    verifying_key_hex: String,
    program: PathBuf,
    #[serde(default)]
    args: Vec<String>,
    process_timeout_secs: u64,
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
        Command::RunSealed {
            challenge,
            primary_endpoint,
            reproduction_endpoint,
            baseline,
            candidates,
            output,
        } => {
            run_sealed(
                &challenge,
                &primary_endpoint,
                &reproduction_endpoint,
                &baseline,
                &candidates,
                output.as_deref(),
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
    let definition = load_challenge(challenge_path)?;
    let hidden_tests = HiddenTestFile::load(hidden_tests_path)?;
    let actual_commitment = hidden_tests.commitment()?;
    if !definition
        .hidden_test_commitment_sha256
        .eq_ignore_ascii_case(&actual_commitment)
    {
        bail!("challenge commitment does not match the supplied hidden-test file");
    }

    let (challenge, candidates) =
        build_challenge(definition, actual_commitment, baseline_path, candidate_args)?;
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

    run_and_emit(&registry, &challenge, &candidates, output).await
}

async fn run_sealed(
    challenge_path: &Path,
    primary_endpoint_path: &Path,
    reproduction_endpoint_path: &Path,
    baseline_path: &Path,
    candidate_args: &[String],
    output: Option<&Path>,
) -> Result<()> {
    let definition = load_challenge(challenge_path)?;
    if !definition.evaluation.distinct_attestation_keys_required {
        bail!("run-sealed requires evaluation.distinct_attestation_keys_required=true");
    }
    let reproduction_id = definition
        .evaluation
        .reproduction_evaluator_id
        .clone()
        .context("sealed challenge requires reproduction_evaluator_id")?;
    let commitment = definition.hidden_test_commitment_sha256.clone();
    let primary = load_endpoint(primary_endpoint_path, &definition.evaluator_id, &commitment)?;
    let reproduction = load_endpoint(reproduction_endpoint_path, &reproduction_id, &commitment)?;
    if primary.attestation_key_sha256() == reproduction.attestation_key_sha256() {
        bail!("sealed endpoints must pin distinct worker keys");
    }
    let (challenge, candidates) =
        build_challenge(definition, commitment, baseline_path, candidate_args)?;
    let mut registry = EvaluatorRegistry::default();
    registry.register(primary)?;
    registry.register(reproduction)?;
    run_and_emit(&registry, &challenge, &candidates, output).await
}

async fn run_and_emit(
    registry: &EvaluatorRegistry,
    challenge: &AlgorithmChallenge,
    candidates: &[CandidateArtifact],
    output: Option<&Path>,
) -> Result<()> {
    let report = AlgorithmDiscoveryLab::new(registry)
        .run(challenge, candidates)
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

fn load_challenge(path: &Path) -> Result<ChallengeFile> {
    let bytes = read_bounded(path, MAX_CHALLENGE_BYTES, "challenge file")?;
    let definition: ChallengeFile = serde_json::from_slice(&bytes)
        .with_context(|| format!("invalid challenge JSON: {}", path.display()))?;
    if definition.schema != CHALLENGE_FILE_SCHEMA {
        bail!("unsupported challenge-file schema: {}", definition.schema);
    }
    Ok(definition)
}

fn build_challenge(
    definition: ChallengeFile,
    commitment: String,
    baseline_path: &Path,
    candidate_args: &[String],
) -> Result<(AlgorithmChallenge, Vec<CandidateArtifact>)> {
    let byte_cap = definition.artifact_byte_cap()?;
    let baseline = CandidateArtifact {
        id: definition.baseline_id,
        media_type: "application/wasm".into(),
        content: read_bounded(baseline_path, byte_cap, "baseline WASM")?,
        metadata: BTreeMap::new(),
    };
    let candidates = candidate_args
        .iter()
        .map(|argument| load_candidate(argument, byte_cap))
        .collect::<Result<Vec<_>>>()?;
    Ok((
        AlgorithmChallenge {
            schema: ALGORITHM_CHALLENGE_SCHEMA.into(),
            id: definition.id,
            title: definition.title,
            evaluator_id: definition.evaluator_id,
            evaluation: definition.evaluation,
            primary_metric: definition.primary_metric,
            minimum_improvement: definition.minimum_improvement,
            hidden_test_commitment_sha256: commitment,
            baseline,
            max_candidates: definition.max_candidates,
        },
        candidates,
    ))
}

fn load_endpoint(
    path: &Path,
    expected_evaluator_id: &str,
    expected_commitment: &str,
) -> Result<Arc<SealedEvaluatorClient>> {
    let bytes = read_bounded(path, MAX_CHALLENGE_BYTES, "worker endpoint")?;
    let endpoint: WorkerEndpointFile = serde_json::from_slice(&bytes)
        .with_context(|| format!("invalid worker endpoint JSON: {}", path.display()))?;
    if endpoint.schema != WORKER_ENDPOINT_SCHEMA {
        bail!("unsupported worker endpoint schema: {}", endpoint.schema);
    }
    if endpoint.evaluator_id != expected_evaluator_id {
        bail!(
            "worker endpoint evaluator {} does not match challenge evaluator {expected_evaluator_id}",
            endpoint.evaluator_id
        );
    }
    if !endpoint
        .test_commitment_sha256
        .eq_ignore_ascii_case(expected_commitment)
    {
        bail!("worker endpoint hidden-test commitment does not match challenge");
    }
    if endpoint.args.len() > 256
        || endpoint
            .args
            .iter()
            .any(|argument| argument.len() > 16 * 1024)
    {
        bail!("worker endpoint arguments exceed count or size limits");
    }
    let key = VerifyingKey::from_bytes(&decode_hex_32(&endpoint.verifying_key_hex)?)
        .context("invalid worker verifying key")?;
    let transport = ProcessSealedTransport::new(
        endpoint.program,
        endpoint.args.into_iter().map(OsString::from),
        endpoint.process_timeout_secs,
    )?;
    Ok(Arc::new(SealedEvaluatorClient::new(
        endpoint.evaluator_id,
        endpoint.evaluator_version,
        endpoint.test_commitment_sha256,
        key,
        Arc::new(transport),
    )?))
}

fn load_candidate(argument: &str, max_bytes: u64) -> Result<CandidateArtifact> {
    let (id, path) = argument
        .split_once('=')
        .context("candidate must use ID=PATH syntax")?;
    if id.trim().is_empty() || path.trim().is_empty() {
        bail!("candidate ID and path must not be empty");
    }
    Ok(CandidateArtifact {
        id: id.to_owned(),
        media_type: "application/wasm".into(),
        content: read_bounded(Path::new(path), max_bytes, "candidate WASM")?,
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

fn decode_hex_32(value: &str) -> Result<[u8; 32]> {
    if value.len() != 64 || !value.bytes().all(|byte| byte.is_ascii_hexdigit()) {
        bail!("worker verifying key must be 32-byte hexadecimal");
    }
    let mut output = [0_u8; 32];
    for (index, byte) in output.iter_mut().enumerate() {
        *byte = u8::from_str_radix(&value[index * 2..index * 2 + 2], 16)?;
    }
    Ok(output)
}
