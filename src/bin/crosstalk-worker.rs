use anyhow::{Context, Result, bail};
use clap::{Parser, Subcommand};
use crosstalk::engines::objective_evaluation::{WASM_I64_TEST_SCHEMA, WasmI64FunctionEvaluator};
use crosstalk::engines::sandbox::{I64TestCase, SandboxConfig, SandboxManager};
use crosstalk::engines::sealed_evaluation::{SealedEvaluationRequest, SealedEvaluatorWorker};
use ed25519_dalek::SigningKey;
use rand::Rng;
use serde::Deserialize;
use sha2::{Digest, Sha256};
use std::fs::{self, OpenOptions};
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use zeroize::Zeroizing;

const MAX_KEY_FILE_BYTES: u64 = 1024;
const MAX_TEST_FILE_BYTES: u64 = 8 * 1024 * 1024;
const MAX_REQUEST_BYTES: u64 = 192 * 1024 * 1024;
const MAX_DURABLE_REPLAYS: usize = 1_000_000;
const WORKER_KEY_SCHEMA: &str = "crosstalk.sealed-worker-key.v1";

#[derive(Debug, Parser)]
#[command(
    name = "crosstalk-worker",
    version,
    about = "Sealed, signed objective-evaluation worker"
)]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Debug, Subcommand)]
enum Command {
    /// Generate a new Ed25519 worker seed in a create-only 0600 file.
    Keygen {
        #[arg(long)]
        output: PathBuf,
    },
    /// Print the public identity derived from an existing worker seed.
    PublicKey {
        #[arg(long)]
        key: PathBuf,
    },
    /// Read one sealed request from stdin and write one signed receipt to stdout.
    EvaluateI64 {
        #[arg(long)]
        key: PathBuf,
        #[arg(long)]
        worker_id: String,
        #[arg(long)]
        evaluator_id: String,
        #[arg(long)]
        evaluator_version: String,
        #[arg(long)]
        hidden_tests: PathBuf,
        /// Durable Sled database used to reject request replays across processes.
        #[arg(long)]
        replay_db: PathBuf,
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

#[tokio::main]
async fn main() -> Result<()> {
    match Cli::parse().command {
        Command::Keygen { output } => keygen(&output),
        Command::PublicKey { key } => {
            let seed = read_seed(&key)?;
            print_public_identity(&seed)?;
            Ok(())
        }
        Command::EvaluateI64 {
            key,
            worker_id,
            evaluator_id,
            evaluator_version,
            hidden_tests,
            replay_db,
            memory_bytes,
            fuel,
            timeout_secs,
        } => {
            evaluate_i64(
                &key,
                &worker_id,
                &evaluator_id,
                &evaluator_version,
                &hidden_tests,
                &replay_db,
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

fn keygen(output: &Path) -> Result<()> {
    let seed = Zeroizing::new(rand::rng().random::<[u8; 32]>());
    let mut file = create_secret_file(output)?;
    file.write_all(encode_hex(&*seed).as_bytes())?;
    file.write_all(b"\n")?;
    file.sync_all()?;
    print_public_identity(&seed)?;
    Ok(())
}

fn print_public_identity(seed: &[u8; 32]) -> Result<()> {
    let key = SigningKey::from_bytes(seed).verifying_key();
    let key_bytes = key.to_bytes();
    let value = serde_json::json!({
        "schema": WORKER_KEY_SCHEMA,
        "verifying_key_hex": encode_hex(&key_bytes),
        "key_sha256": sha256(&key_bytes),
    });
    serde_json::to_writer(std::io::stdout(), &value)?;
    println!();
    Ok(())
}

async fn evaluate_i64(
    key_path: &Path,
    worker_id: &str,
    evaluator_id: &str,
    evaluator_version: &str,
    hidden_tests_path: &Path,
    replay_db_path: &Path,
    sandbox_config: SandboxConfig,
) -> Result<()> {
    let seed = read_seed(key_path)?;
    let hidden = load_hidden_tests(hidden_tests_path)?;
    let sandbox = Arc::new(SandboxManager::new(sandbox_config)?);
    let evaluator = Arc::new(WasmI64FunctionEvaluator::new(
        sandbox,
        evaluator_id,
        evaluator_version,
        hidden.export_name,
        hidden.cases,
    )?);
    let worker = SealedEvaluatorWorker::from_seed(worker_id, evaluator, *seed)?;

    let mut input = Vec::new();
    std::io::stdin()
        .take(MAX_REQUEST_BYTES + 1)
        .read_to_end(&mut input)?;
    if input.is_empty() || input.len() as u64 > MAX_REQUEST_BYTES {
        bail!("sealed request must contain between 1 byte and {MAX_REQUEST_BYTES} bytes");
    }
    let request: SealedEvaluationRequest =
        serde_json::from_slice(&input).context("invalid sealed evaluation request JSON")?;
    request.validate_at(unix_time())?;
    if request.evaluator_id != evaluator_id || request.evaluator_version != evaluator_version {
        bail!("request evaluator identity does not match worker configuration");
    }

    consume_durable_replay(replay_db_path, &request)?;
    let receipt = worker.handle(request).await?;
    serde_json::to_writer(std::io::stdout(), &receipt)?;
    println!();
    Ok(())
}

fn load_hidden_tests(path: &Path) -> Result<HiddenTestFile> {
    let bytes = read_regular_bounded(path, MAX_TEST_FILE_BYTES, "hidden-test file")?;
    let tests: HiddenTestFile = serde_json::from_slice(&bytes)
        .with_context(|| format!("invalid hidden-test JSON: {}", path.display()))?;
    if tests.schema != WASM_I64_TEST_SCHEMA {
        bail!("unsupported hidden-test schema: {}", tests.schema);
    }
    WasmI64FunctionEvaluator::commitment_for(&tests.export_name, &tests.cases)?;
    Ok(tests)
}

fn consume_durable_replay(path: &Path, request: &SealedEvaluationRequest) -> Result<()> {
    let db = sled::open(path)
        .with_context(|| format!("could not open replay database {}", path.display()))?;
    let tree = db.open_tree("sealed-request-replays")?;
    let current_time = unix_time();
    for entry in tree.iter() {
        let (key, value) = entry?;
        if value.len() == 8 {
            let mut bytes = [0_u8; 8];
            bytes.copy_from_slice(&value);
            if u64::from_be_bytes(bytes).saturating_add(60) < current_time {
                tree.remove(key)?;
            }
        }
    }
    if tree.len() >= MAX_DURABLE_REPLAYS {
        bail!("durable replay database is full");
    }
    let digest = request.sha256()?;
    let expiry = request.expires_at.to_be_bytes();
    if tree
        .compare_and_swap(digest.as_bytes(), None as Option<&[u8]>, Some(&expiry))?
        .is_err()
    {
        bail!("sealed evaluation request replay rejected by durable store");
    }
    tree.flush()?;
    Ok(())
}

fn read_seed(path: &Path) -> Result<Zeroizing<[u8; 32]>> {
    ensure_private_key_file(path)?;
    let bytes = Zeroizing::new(read_regular_bounded(
        path,
        MAX_KEY_FILE_BYTES,
        "worker key",
    )?);
    let text = std::str::from_utf8(&bytes)
        .context("worker key is not UTF-8 hexadecimal")?
        .trim();
    let decoded = decode_hex_32(text).context("worker key must contain a 32-byte hex seed")?;
    Ok(Zeroizing::new(decoded))
}

fn read_regular_bounded(path: &Path, max_bytes: u64, label: &str) -> Result<Vec<u8>> {
    let metadata = fs::symlink_metadata(path)
        .with_context(|| format!("could not inspect {label}: {}", path.display()))?;
    if metadata.file_type().is_symlink() || !metadata.is_file() {
        bail!("{label} must be a regular non-symlink file");
    }
    if metadata.len() == 0 || metadata.len() > max_bytes {
        bail!("{label} must contain between 1 and {max_bytes} bytes");
    }
    let bytes = fs::read(path)?;
    if bytes.is_empty() || bytes.len() as u64 > max_bytes {
        bail!("{label} changed size while being read");
    }
    Ok(bytes)
}

#[cfg(unix)]
fn create_secret_file(path: &Path) -> Result<fs::File> {
    use std::os::unix::fs::OpenOptionsExt;
    Ok(OpenOptions::new()
        .write(true)
        .create_new(true)
        .mode(0o600)
        .open(path)
        .with_context(|| {
            format!(
                "could not create worker key {} (existing files are not overwritten)",
                path.display()
            )
        })?)
}

#[cfg(not(unix))]
fn create_secret_file(path: &Path) -> Result<fs::File> {
    Ok(OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(path)
        .with_context(|| {
            format!(
                "could not create worker key {} (existing files are not overwritten)",
                path.display()
            )
        })?)
}

#[cfg(unix)]
fn ensure_private_key_file(path: &Path) -> Result<()> {
    use std::os::unix::fs::MetadataExt;
    let metadata = fs::symlink_metadata(path)?;
    if metadata.mode() & 0o077 != 0 {
        bail!(
            "worker key {} is accessible by group or other users; require mode 0600",
            path.display()
        );
    }
    Ok(())
}

#[cfg(not(unix))]
fn ensure_private_key_file(_path: &Path) -> Result<()> {
    Ok(())
}

fn decode_hex_32(value: &str) -> Result<[u8; 32]> {
    if value.len() != 64 || !value.bytes().all(|byte| byte.is_ascii_hexdigit()) {
        bail!("invalid hexadecimal seed");
    }
    let mut output = [0_u8; 32];
    for (index, byte) in output.iter_mut().enumerate() {
        *byte = u8::from_str_radix(&value[index * 2..index * 2 + 2], 16)?;
    }
    Ok(output)
}

fn encode_hex(bytes: &[u8]) -> String {
    bytes.iter().map(|byte| format!("{byte:02x}")).collect()
}

fn sha256(bytes: &[u8]) -> String {
    format!("{:x}", Sha256::digest(bytes))
}

fn unix_time() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}
