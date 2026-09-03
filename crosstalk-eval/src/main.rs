mod claude_agent;
mod dataset;
mod docker_env;
mod harness;
mod manifest;
mod report;
mod runner;
mod scenarios;
mod swe_bench_runner;

use anyhow::{Context, Result, bail};
use clap::Parser;
use std::path::PathBuf;
use std::time::{SystemTime, UNIX_EPOCH};

use manifest::{
    EVALUATION_RESULT_SCHEMA, EvaluationArtifact, EvaluationManifest, EvaluationResultManifest,
    RunClassification, RunStatus, file_sha256,
};

/// Load `~/.env` (KEY=value lines) into the process environment so that
/// clap's `env` attributes pick them up. Existing env vars are not overwritten.
fn load_dotenv() {
    let path = std::env::var("HOME")
        .map(|h| std::path::PathBuf::from(h).join(".env"))
        .unwrap_or_default();
    let Ok(contents) = std::fs::read_to_string(&path) else {
        return;
    };
    for line in contents.lines() {
        let line = line.trim();
        if line.starts_with('#') || line.is_empty() {
            continue;
        }
        if let Some((key, val)) = line.split_once('=') {
            let key = key.trim();
            let val = val.trim().trim_matches('"').trim_matches('\'');
            if std::env::var(key).is_err() {
                // SAFETY: single-threaded at this point (before tokio runtime starts).
                unsafe { std::env::set_var(key, val) };
            }
        }
    }
}

#[derive(Parser, Debug)]
#[command(
    name = "crosstalk-eval",
    about = "Fail-closed benchmarking harness for Crosstalk"
)]
struct Args {
    /// Preregistered JSON manifest for a live, smoke, or development evaluation.
    #[arg(long)]
    evaluation_manifest: Option<PathBuf>,

    /// Validate --evaluation-manifest, print its identity, and exit without running models.
    #[arg(long, requires = "evaluation_manifest")]
    validate_manifest: bool,

    /// Create-only path for the hash-bound live evaluation artifact.
    #[arg(long)]
    evaluation_result: Option<PathBuf>,

    /// Run the simulated UCB1 topology scenarios with synthetic fixtures.
    #[arg(long, default_value_t = false)]
    synthetic: bool,

    /// Output directory for CSV reports
    #[arg(short, long, default_value = "results")]
    output: PathBuf,

    /// Random seed for reproducibility
    #[arg(short, long, default_value_t = 42)]
    seed: u64,

    /// Run only Scenario 1 (budget pressure test)
    #[arg(long, conflicts_with = "scenario2_only")]
    scenario1_only: bool,

    /// Run only Scenario 2 (UCB1 convergence test)
    #[arg(long, conflicts_with = "scenario1_only")]
    scenario2_only: bool,

    /// Path to SWE-bench (or SWE-bench Lite) JSONL dataset; enables Scenario 3
    #[arg(long)]
    swe_bench: Option<std::path::PathBuf>,

    /// Maximum turns per SWE-bench instance
    #[arg(long, default_value_t = 10)]
    swe_max_turns: u32,

    /// Run SWE-bench with live Docker containers (requires --swe-bench)
    #[arg(long, conflicts_with = "smoke_test")]
    live_run: bool,

    /// Run N instances sequentially as a smoke test (implies --live-run)
    #[arg(long, conflicts_with = "live_run")]
    smoke_test: bool,

    /// Number of instances for --smoke-test (default: 5)
    #[arg(long, default_value_t = 5)]
    count: usize,

    /// Max concurrent Docker containers for --live-run
    #[arg(long, default_value_t = 10)]
    concurrency: usize,

    /// Docker image prefix for SWE-bench containers
    #[arg(long, default_value = "sweb.eval.x86_64")]
    image_prefix: String,

    /// SWE-bench harness version embedded in image tags (e.g. 1776)
    #[arg(long, default_value = "1776")]
    image_version: String,

    /// Path for incremental JSONL checkpoint (appended on resume)
    #[arg(long, default_value = "results/live_run_checkpoint.jsonl")]
    checkpoint: PathBuf,

    /// OpenRouter API key (reads OPENROUTER_API_KEY env / ~/.env)
    #[arg(long, env = "OPENROUTER_API_KEY", hide_env_values = true)]
    openrouter_key: Option<String>,

    /// Anthropic API key — use this to switch to Claude models
    #[arg(long, env = "ANTHROPIC_API_KEY", hide_env_values = true)]
    api_key: Option<String>,

    /// LLM model ID (default: Haiku — Fast tier)
    #[arg(long, default_value = claude_agent::DEFAULT_MODEL)]
    model: String,

    /// Reasoning-tier model used for complex topologies and temporal escalation
    #[arg(long, default_value = claude_agent::OPUS_MODEL)]
    reasoning_model: String,

    /// Acknowledge the mock repository and permit simulated model responses in development runs.
    #[arg(long, default_value_t = false)]
    mock: bool,

    /// API base URL for OpenAI-compatible providers; omit to use Anthropic
    #[arg(long, default_value = claude_agent::OPENROUTER_BASE)]
    api_base: String,
}

impl Args {
    /// Resolve the effective (api_key, api_base) pair.
    ///
    /// Priority: --api-key → Anthropic, --openrouter-key → OpenRouter.
    /// Anthropic wins so that ANTHROPIC_API_KEY in ~/.env is always used for
    /// the single-model rigorous benchmark path when both keys are present.
    fn effective_agent(&self) -> (Option<String>, Option<String>) {
        if let Some(k) = &self.api_key {
            (Some(k.clone()), None) // None api_base → Anthropic
        } else if let Some(k) = &self.openrouter_key {
            (Some(k.clone()), Some(self.api_base.clone()))
        } else {
            (None, None)
        }
    }
}

#[tokio::main]
async fn main() -> Result<()> {
    load_dotenv();

    tracing_subscriber::fmt()
        .with_env_filter("crosstalk_eval=info,warn")
        .init();

    let args = Args::parse();

    let evaluation_manifest = args
        .evaluation_manifest
        .as_deref()
        .map(EvaluationManifest::load)
        .transpose()?;

    if args.validate_manifest {
        let manifest = evaluation_manifest
            .as_ref()
            .expect("clap requires --evaluation-manifest");
        println!(
            "{}",
            serde_json::to_string_pretty(&serde_json::json!({
                "schema": manifest.schema,
                "id": manifest.id,
                "classification": manifest.classification,
                "sha256": manifest.sha256()?,
            }))?
        );
        return Ok(());
    }

    let (api_key, api_base) = args.effective_agent();

    if (args.live_run || args.smoke_test || args.swe_bench.is_some())
        && api_key.is_none()
        && !args.mock
    {
        anyhow::bail!(
            "SWE-bench evaluation requires provider credentials; pass --mock only for explicit development simulations"
        );
    }

    if args.live_run || args.smoke_test {
        let swe_path = args.swe_bench.as_ref().ok_or_else(|| {
            anyhow::anyhow!("--swe-bench is required with --live-run / --smoke-test")
        })?;
        let instances = swe_bench_runner::load_swe_bench(swe_path)?;
        let manifest = evaluation_manifest.as_ref().ok_or_else(|| {
            anyhow::anyhow!("--evaluation-manifest is required for every live or smoke evaluation")
        })?;
        validate_live_manifest(manifest, &args, swe_path, &api_base)?;
        let started_at = unix_timestamp()?;
        let mode = if args.smoke_test {
            runner::RunMode::SmokeTest { count: args.count }
        } else {
            runner::RunMode::FullRun {
                concurrency: args.concurrency,
            }
        };
        // When using OpenRouter (api_base is Some), the Anthropic key becomes
        // a Haiku fallback for when all free models are rate-limited.
        let fallback_anthropic_key = if api_base.is_some() {
            args.api_key.clone()
        } else {
            None
        };
        let cfg = runner::LiveRunConfig {
            mode,
            max_turns: args.swe_max_turns,
            seed: args.seed,
            checkpoint_path: args.checkpoint.clone(),
            image_prefix: args.image_prefix.clone(),
            image_version: args.image_version.clone(),
            api_key,
            model: args.model.clone(),
            api_base,
            fallback_anthropic_key,
            reasoning_model: args.reasoning_model.clone(),
        };
        let outcome = runner::run_live(&instances, cfg).await?;
        runner::print_live_summary(&outcome.results);
        let completed_at = unix_timestamp()?;
        let successful_tasks = outcome
            .results
            .iter()
            .filter(|result| result.metrics.patch_resolved)
            .count();
        let actual_cost_usd = outcome
            .results
            .iter()
            .map(|result| result.metrics.total_cost_usd)
            .sum();
        let status = if outcome.cancelled {
            RunStatus::Cancelled
        } else {
            RunStatus::Completed
        };
        let diagnostics = if outcome.infrastructure_failures == 0 {
            String::new()
        } else {
            format!(
                "{} task(s) ended in infrastructure failure and are counted as failures",
                outcome.infrastructure_failures
            )
        };
        let result = EvaluationResultManifest {
            schema: EVALUATION_RESULT_SCHEMA.into(),
            manifest_id: manifest.id.clone(),
            manifest_sha256: manifest.sha256()?,
            classification: manifest.classification,
            started_at,
            completed_at,
            status,
            total_tasks: outcome.total_tasks,
            completed_tasks: outcome.completed_tasks,
            successful_tasks,
            failed_tasks: outcome.completed_tasks.saturating_sub(successful_tasks),
            actual_cost_usd,
            result_payload_sha256: String::new(),
            diagnostics,
        };
        let artifact = EvaluationArtifact::new(result, serde_json::to_value(&outcome.results)?)?;
        let result_path = args.evaluation_result.clone().unwrap_or_else(|| {
            args.output.join(format!(
                "{}-{}-result.json",
                sanitize_filename(&manifest.id),
                completed_at
            ))
        });
        artifact.write_create_new(&result_path)?;
        println!("Evaluation artifact: {}", result_path.display());
        return Ok(());
    }

    if args.swe_bench.is_some() && !args.mock {
        anyhow::bail!(
            "non-live SWE-bench uses a simulated repository environment; pass --mock to acknowledge this development mode, or use --live-run"
        );
    }

    if let Some(manifest) = &evaluation_manifest {
        if manifest.classification != RunClassification::DevelopmentSimulation {
            bail!("synthetic and mock evaluations require classification=development_simulation");
        }
        if manifest.seed != args.seed {
            bail!("evaluation seed does not match the preregistered manifest");
        }
    }

    if !args.synthetic && args.swe_bench.is_none() {
        anyhow::bail!(
            "no evaluation mode selected; use --live-run with --swe-bench for a real container run, or explicitly select --synthetic / --mock for development"
        );
    }
    std::fs::create_dir_all(&args.output)?;

    if args.synthetic {
        tracing::warn!(
            "running an explicitly requested topology simulation; these are not model benchmark results"
        );
        let questions = dataset::synthetic_math_questions(200);
        if !args.scenario2_only {
            tracing::info!("=== Scenario 1: Simulated Budget Pressure Test ===");
            let records = scenarios::run_budget_pressure(&questions, args.seed)?;
            let path = args.output.join("topology_distribution.csv");
            report::write_budget_pressure_csv(&path, &records)?;
            tracing::info!("Written: {}", path.display());
            report::print_budget_pressure_summary(&records);
        }

        if !args.scenario1_only {
            tracing::info!("=== Scenario 2: Simulated UCB1 Convergence Test ===");
            let records = scenarios::run_ucb1_convergence(&questions, args.seed)?;
            let path = args.output.join("ucb1_convergence.csv");
            report::write_ucb1_convergence_csv(&path, &records)?;
            tracing::info!("Written: {}", path.display());
            report::print_ucb1_convergence_summary(&records);
        }
    }

    if let Some(swe_path) = args.swe_bench {
        tracing::warn!("=== Scenario 3: SWE-bench development run with mock repository ===");
        let instances = swe_bench_runner::load_swe_bench(&swe_path).map_err(|error| {
            anyhow::anyhow!(
                "failed to load real SWE-bench dataset {}: {error}",
                swe_path.display()
            )
        })?;
        tracing::info!("Loaded {} SWE-bench instances", instances.len());
        let fallback_anthropic_key = if api_base.is_some() {
            args.api_key.clone()
        } else {
            None
        };
        let cfg = swe_bench_runner::SweBenchRunnerConfig {
            max_turns: args.swe_max_turns,
            seed: args.seed,
            api_key,
            model: args.model.clone(),
            api_base,
            fallback_anthropic_key,
            reasoning_model: args.reasoning_model.clone(),
            ..Default::default()
        };
        let env = swe_bench_runner::MockSweBenchEnvironment::new(args.seed);
        let mut runner = swe_bench_runner::SweBenchRunner::with_environment(cfg, env);
        let results = runner.run_dataset(&instances).await?;
        let path = args.output.join("swe_bench_results.csv");
        swe_bench_runner::write_swe_bench_csv(&path, &results)?;
        tracing::info!("Written: {}", path.display());
        swe_bench_runner::print_swe_bench_summary(&results);
    }

    Ok(())
}

fn validate_live_manifest(
    manifest: &EvaluationManifest,
    args: &Args,
    task_set_path: &std::path::Path,
    api_base: &Option<String>,
) -> Result<()> {
    let expected_classification = if args.mock {
        RunClassification::DevelopmentSimulation
    } else if args.smoke_test {
        RunClassification::SmokeTest
    } else {
        RunClassification::Benchmark
    };
    if manifest.classification != expected_classification {
        bail!(
            "evaluation classification mismatch: command requires {:?}",
            expected_classification
        );
    }
    let actual_task_sha = file_sha256(task_set_path)?;
    if manifest.task_set.content_sha256.to_ascii_lowercase() != actual_task_sha {
        bail!("SWE-bench task-set bytes do not match the preregistered commitment");
    }
    if manifest.seed != args.seed {
        bail!("evaluation seed does not match the preregistered manifest");
    }
    let effective_concurrency = if args.smoke_test { 1 } else { args.concurrency };
    if args.swe_max_turns > manifest.budget.max_turns_per_task
        || effective_concurrency > manifest.budget.max_concurrency
    {
        bail!("requested turns or concurrency exceed the preregistered budget");
    }
    let provider = if api_base.is_some() {
        "openrouter"
    } else {
        "anthropic"
    };
    let solver = manifest
        .models
        .iter()
        .find(|model| model.role == "solver")
        .ok_or_else(|| anyhow::anyhow!("evaluation manifest requires a solver model role"))?;
    if solver.provider != provider || solver.model_id != args.model {
        bail!("effective solver provider/model does not match the preregistered manifest");
    }
    if !args.reasoning_model.is_empty() {
        let reasoning = manifest
            .models
            .iter()
            .find(|model| model.role == "reasoning")
            .ok_or_else(|| anyhow::anyhow!("manifest requires a reasoning model role"))?;
        if reasoning.provider != provider || reasoning.model_id != args.reasoning_model {
            bail!("effective reasoning provider/model does not match the manifest");
        }
    }
    Ok(())
}

fn unix_timestamp() -> Result<u64> {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .context("system clock is before Unix epoch")
        .map(|duration| duration.as_secs())
}

fn sanitize_filename(value: &str) -> String {
    let sanitized: String = value
        .chars()
        .map(|character| {
            if character.is_ascii_alphanumeric() || matches!(character, '-' | '_' | '.') {
                character
            } else {
                '_'
            }
        })
        .take(160)
        .collect();
    if sanitized.is_empty() {
        "evaluation".into()
    } else {
        sanitized
    }
}
