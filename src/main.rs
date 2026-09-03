use crosstalk::core::agent_trait::PromptAgent;
use crosstalk::core::factory::ModelFactory;
use crosstalk::core::orchestrator::Orchestrator;
use crosstalk::core::state::StateManager;
use crosstalk::log_warn;
use crosstalk::types::conversation::{
    ConversationState, TaskCategory, Turn, TurnOutcome, TurnStructure,
};
use crosstalk::types::events::{ControlSignal, StreamEvent};
use crosstalk::ui::app::App;
use crosstalk::ui::events::{self as ui_events, Action};
use crosstalk::ui::model_select;
use crosstalk::ui::render;
use crosstalk_concurrency::CancelScope;
use crossterm::ExecutableCommand;
use crossterm::cursor::MoveTo;
use crossterm::terminal::{
    EnterAlternateScreen, LeaveAlternateScreen, disable_raw_mode, enable_raw_mode,
};
use futures::future::join_all;
use ratatui::Terminal;
use ratatui::backend::CrosstermBackend;
use std::io;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::Mutex;
use tokio::sync::mpsc;

use clap::{Parser, ValueEnum};

#[derive(Debug, Clone, Copy, ValueEnum)]
enum HeadlessFormat {
    Json,
    Markdown,
}

#[derive(Parser)]
#[command(
    name = "Crosstalk",
    version,
    about = "Domain-general multi-model reasoning and verification orchestrator"
)]
struct Args {
    #[arg(short, long)]
    task: Option<String>,

    #[arg(short, long, num_args = 0..)]
    models: Vec<String>,

    #[arg(short, long, default_value_t = 0)]
    iterations: u32,

    #[arg(short, long)]
    workspace: Option<String>,

    #[arg(short = 'f', long, num_args = 0..)]
    files: Vec<String>,

    #[arg(long, default_value_t = 300)]
    agent_timeout_secs: u64,

    #[arg(long, num_args = 1.., value_name = "SHELL")]
    generate_completions: Vec<String>,

    /// Automatically select the best available models for the task.
    #[arg(short = 'A', long, default_value_t = false)]
    auto: bool,

    /// After consensus, write agent-proposed changes back to the source files.
    #[arg(short = 'e', long, default_value_t = false)]
    edit: bool,

    /// Resume a prior session by its session ID (restores the latest checkpoint).
    #[arg(long)]
    resume: Option<String>,

    /// Search arXiv and attach provenance-rich records before deliberation.
    #[arg(long, value_name = "QUERY")]
    arxiv: Option<String>,

    /// Search Zenodo and attach provenance-rich records before deliberation.
    #[arg(long, value_name = "QUERY")]
    zenodo: Option<String>,

    /// Maximum records per research repository query (capped at 25).
    #[arg(long, default_value_t = 10)]
    research_limit: usize,

    /// Run the native BlindMind evolution engine for this many generations before deliberation.
    #[arg(long, default_value_t = 0, value_name = "N")]
    evolve_generations: usize,

    /// Candidate population requested from each native evolution generation.
    #[arg(long, default_value_t = 8, value_name = "N")]
    evolve_population: usize,

    /// Deterministic seed for native evolutionary discovery.
    #[arg(long, default_value_t = 1)]
    evolve_seed: u64,

    /// Maximum native evolution candidates evaluated concurrently.
    #[arg(long, default_value_t = 4, value_name = "N")]
    evolve_concurrency: usize,

    /// Wall-clock limit for the complete native evolution stage.
    #[arg(long, default_value_t = 900, value_name = "SECONDS")]
    evolve_timeout_secs: u64,

    /// Hard cap for variation + critic model-call slots. Zero automatically
    /// budgets two calls per requested candidate per generation.
    #[arg(long, default_value_t = 0, value_name = "N")]
    evolve_max_model_calls: u64,

    /// Hard requirement applied to every evolved candidate. Repeat the flag to
    /// provide multiple independent requirements.
    #[arg(long = "evolve-constraint", value_name = "TEXT")]
    evolve_constraints: Vec<String>,

    /// A previously eliminated mechanism family. Separate structural features
    /// with semicolons; repeat the flag for additional negative results.
    #[arg(long = "evolve-exclusion", value_name = "FEATURES")]
    evolve_exclusions: Vec<String>,

    /// Hard post-initialization provider call limit, including failed attempts
    /// and retries. Endpoint-validation pings currently occur before this ledger.
    #[arg(long, default_value_t = 0, value_name = "N")]
    max_model_calls: u64,

    /// Hard session-wide estimated input-token limit. Zero means unlimited.
    #[arg(long, default_value_t = 0, value_name = "N")]
    max_input_tokens: u64,

    /// Hard session-wide estimated streamed-output-token limit. Zero means unlimited.
    #[arg(long, default_value_t = 0, value_name = "N")]
    max_output_tokens: u64,

    /// Run without a terminal UI and write the final result to stdout.
    #[arg(long, default_value_t = false)]
    headless: bool,

    /// Serialization used for the final headless result.
    #[arg(long, value_enum, default_value_t = HeadlessFormat::Json)]
    headless_format: HeadlessFormat,

    /// Export a reproducible investigation bundle to this directory.
    #[arg(long, value_name = "DIR")]
    bundle_dir: Option<PathBuf>,

    /// Verify an exported investigation bundle and exit without starting models.
    #[arg(long, value_name = "DIR")]
    verify_bundle: Option<PathBuf>,

    /// Import a crosstalk.blindmind.v1 archive into a checkpoint and exit.
    /// Prints the import summary; writes the checkpoint when --import-blindmind-out is given.
    #[arg(long, value_name = "FILE")]
    import_blindmind: Option<PathBuf>,

    #[arg(long, value_name = "FILE", requires = "import_blindmind")]
    import_blindmind_out: Option<PathBuf>,
}

fn lang_from_ext(path: &str) -> String {
    match path.rsplit('.').next().unwrap_or("") {
        "rs" => "rust",
        "py" => "python",
        "js" => "javascript",
        "ts" => "typescript",
        "md" => "markdown",
        "toml" => "toml",
        "yaml" | "yml" => "yaml",
        "json" => "json",
        "sh" | "bash" => "shell",
        "html" => "html",
        "css" => "css",
        "sql" => "sql",
        "lean" => "lean4",
        "v" => "coq",
        "go" => "go",
        "java" => "java",
        "c" | "h" => "c",
        "cpp" | "hpp" | "cc" => "cpp",
        ext => ext,
    }
    .to_string()
}

const SKIP_DIRS: &[&str] = &[
    ".git",
    "__pycache__",
    "node_modules",
    "target",
    ".venv",
    "venv",
    ".tox",
    ".mypy_cache",
    ".pytest_cache",
    "dist",
    "build",
    ".eggs",
    "*.egg-info",
    ".DS_Store",
];

fn should_skip(path: &std::path::Path) -> bool {
    for component in path.components() {
        let name = component.as_os_str().to_string_lossy();
        if SKIP_DIRS
            .iter()
            .any(|s| name == *s || name.ends_with(".egg-info"))
        {
            return true;
        }
    }
    false
}

fn is_likely_text(path: &std::path::Path) -> bool {
    let ext = path.extension().and_then(|e| e.to_str()).unwrap_or("");
    matches!(
        ext,
        "rs" | "py"
            | "js"
            | "ts"
            | "tsx"
            | "jsx"
            | "md"
            | "txt"
            | "toml"
            | "yaml"
            | "yml"
            | "json"
            | "sh"
            | "bash"
            | "zsh"
            | "html"
            | "css"
            | "sql"
            | "lean"
            | "v"
            | "go"
            | "java"
            | "c"
            | "h"
            | "cpp"
            | "hpp"
            | "cc"
            | "rb"
            | "ex"
            | "exs"
            | "hs"
            | "ml"
            | "mli"
            | "r"
            | "R"
            | "jl"
            | "lua"
            | "pl"
            | "pm"
            | "swift"
            | "kt"
            | "scala"
            | "clj"
            | "erl"
            | "cfg"
            | "ini"
            | "conf"
            | "env"
            | "xml"
            | "csv"
            | "tsv"
            | "makefile"
            | "dockerfile"
            | "gitignore"
            | "lock"
    ) || path.file_name().is_some_and(|n| {
        let n = n.to_string_lossy().to_lowercase();
        n == "makefile" || n == "dockerfile" || n == ".gitignore" || n == "cargo.lock"
    })
}

/// Initialize structured logging: create the log directory, rotate to the 5
/// most recent logs, and install a non-blocking file subscriber.
///
/// Returns the worker guard (which must be kept alive for the process lifetime
/// so buffered logs are flushed on shutdown), the path of the active log file,
/// and the run timestamp (reused as the default session id).
fn init_logging() -> anyhow::Result<(
    tracing_appender::non_blocking::WorkerGuard,
    std::path::PathBuf,
    String,
)> {
    let log_dir = std::env::var("XDG_STATE_HOME").unwrap_or_else(|_| {
        std::env::var("HOME")
            .map(|h| format!("{h}/.local/state"))
            .unwrap_or_else(|_| "/tmp".to_string())
    });
    let log_path = format!("{log_dir}/crosstalk");
    log_warn!(
        std::fs::create_dir_all(&log_path),
        "Failed to create log directory"
    );
    // Rotate: keep the 5 most recent logs (ISO timestamps sort lexicographically)
    if let Ok(entries) = std::fs::read_dir(&log_path) {
        let mut logs: Vec<_> = entries
            .flatten()
            .filter(|e| e.path().extension().and_then(|x| x.to_str()) == Some("log"))
            .map(|e| e.path())
            .collect();
        logs.sort();
        for old in logs.iter().rev().skip(5) {
            log_warn!(std::fs::remove_file(old), "Failed to remove old log file");
        }
    }
    let run_ts = chrono::Utc::now().format("%Y%m%dT%H%M%SZ").to_string();
    let run_log = std::path::PathBuf::from(&log_path).join(format!("crosstalk-{run_ts}.log"));
    let log_file = std::fs::File::create(&run_log)?;
    let (non_blocking, guard) = tracing_appender::non_blocking(log_file);
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("crosstalk=debug")),
        )
        .with_writer(non_blocking)
        .with_ansi(false)
        .init();
    Ok((guard, run_log, run_ts))
}

/// Drive the background orchestrator loop: run turns until convergence, the
/// iteration cap, an unrecoverable error, or cancellation. Spawned (and later
/// drained) by the caller's [`CancelScope`]; checks `cancel` before each turn
/// so a shutdown takes effect at the next turn boundary.
async fn run_orchestrator_loop(
    app: Arc<Mutex<App>>,
    sigma: Arc<Mutex<ConversationState>>,
    omicron: Arc<Orchestrator>,
    iterations: u32,
    turn_timeout: Duration,
    cancel: CancelScope,
) {
    let mut i = 0u32;
    let mut consecutive_failures = 0u32;
    loop {
        if cancel.is_cancelled() || app.lock().await.shutdown {
            break;
        }
        let sigma_in = Arc::clone(&sigma);
        let omicron_in = Arc::clone(&omicron);
        let mut join = tokio::task::spawn(async move { omicron_in.run_turn(sigma_in).await });
        let res = match tokio::time::timeout(turn_timeout, &mut join).await {
            Err(_elapsed) => {
                join.abort();
                let _ = join.await;
                i += 1;
                consecutive_failures += 1;
                let mut a = app.lock().await;
                a.push_event(format!(
                    "Turn {} timed out after {}s",
                    i,
                    turn_timeout.as_secs()
                ));
                drop(a);
                if (iterations > 0 && i >= iterations) || consecutive_failures >= 3 {
                    break;
                }
                tokio::time::sleep(Duration::from_secs(2)).await;
                continue;
            }
            Ok(r) => r,
        };

        match res {
            Ok(Ok(optimal)) => {
                i += 1;
                consecutive_failures = 0;
                if optimal {
                    let mut a = app.lock().await;
                    a.push_event(format!("Converged after {} turn(s)", i));
                    break;
                }
                if iterations > 0 && i >= iterations {
                    let mut a = app.lock().await;
                    a.push_event(format!("Completed {} iteration(s)", i));
                    break;
                }
                let (session_id, chain_head) = {
                    let s = sigma.lock().await;
                    (s.session_id.clone(), s.chain_head_hex())
                };
                Orchestrator::git_commit_session(
                    &omicron.file_writer.root,
                    &session_id,
                    i,
                    &chain_head,
                )
                .await;
            }
            Ok(Err(e)) => {
                i += 1;
                consecutive_failures += 1;
                let mut app_err = app.lock().await;
                app_err.push_event(format!("Turn {} error: {}", i, e));
                drop(app_err);
                if (iterations > 0 && i >= iterations) || consecutive_failures >= 3 {
                    break;
                }
                tokio::time::sleep(Duration::from_secs(2)).await;
            }
            Err(e) => {
                let mut app_err = app.lock().await;
                app_err.push_event(format!("Turn {} panic: {}", i + 1, e));
                drop(app_err);
                tokio::time::sleep(Duration::from_secs(2)).await;
                break;
            }
        }
    }
    let mut a = app.lock().await;
    a.push_event("Session ending...".to_string());
    a.shutdown = true;
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    // Parse before initializing logging so --help, --version, and shell
    // completions are side-effect free and work in read-only environments.
    let args = Args::parse();

    if let Some(bundle_dir) = &args.verify_bundle {
        let verification =
            crosstalk::engines::investigation_bundle::InvestigationBundleExporter::verify(
                bundle_dir,
            )?;
        println!("{}", serde_json::to_string_pretty(&verification)?);
        if !verification.passed {
            anyhow::bail!("investigation bundle verification failed");
        }
        return Ok(());
    }

    if let Some(archive) = &args.import_blindmind {
        let json = std::fs::read_to_string(archive)?;
        let (state, summary) =
            crosstalk::engines::idea_evolution::import_blindmind_archive(&json)
                .map_err(anyhow::Error::msg)?;
        if let Some(out) = &args.import_blindmind_out {
            std::fs::write(out, state.checkpoint_json()?)?;
        }
        println!("{}", serde_json::to_string(&summary)?);
        return Ok(());
    }

    if !args.generate_completions.is_empty() {
        use clap::CommandFactory;
        use clap_complete::{Shell, generate};
        let mut cmd = Args::command();
        for shell_str in &args.generate_completions {
            match shell_str.parse::<Shell>() {
                Ok(shell) => generate(shell, &mut cmd, "crosstalk", &mut std::io::stdout()),
                Err(_) => anyhow::bail!(
                    "Unknown shell '{shell_str}'. Supported: bash, zsh, fish, powershell, elvish"
                ),
            }
        }
        return Ok(());
    }

    if args.headless && args.task.is_none() {
        anyhow::bail!("--headless requires --task so the run is non-interactive");
    }

    // Online runs load ~/.env then project-local .env after all offline command
    // paths have returned. This keeps help, completions, and bundle verification
    // from touching credential files.
    if let Ok(home) = std::env::var("HOME") {
        let _ = dotenv::from_path(std::path::Path::new(&home).join(".env"));
    }
    let _ = dotenv::dotenv();

    // 0. Initialize structured logging -- rotate, keeping last 5 logs
    let (_guard, run_log, run_ts) = init_logging()?;

    tracing::info!("crosstalk session starting");

    // First-run: if no API keys are configured, launch the setup wizard.
    if !model_select::has_any_api_key() && args.models.is_empty() {
        if args.headless {
            anyhow::bail!(
                "headless mode requires a configured provider API key or explicit model credentials"
            );
        }
        model_select::run_api_key_setup().await?;
    }

    // Load user config (~/.config/crosstalk/config.toml)
    let config = crosstalk::core::config::Config::load();

    // If no task provided, run the interactive wizard to collect task + optional workspace/iterations.
    let (task_str, wizard_workspace, wizard_iterations) = if args.task.is_none() {
        let (t, ws, iters) = model_select::run_task_wizard().await?;
        let ws = ws.or(config.default_workspace.clone());
        let iters = if iters == 0 {
            config.default_iterations.unwrap_or(0)
        } else {
            iters
        };
        (t, ws, iters)
    } else {
        let ws = args.workspace.clone().or(config.default_workspace.clone());
        let iters = if args.iterations == 0 {
            config.default_iterations.unwrap_or(0)
        } else {
            args.iterations
        };
        (args.task.unwrap_or_default(), ws, iters)
    };

    let use_auto = args.auto || config.auto_mode.unwrap_or(false);
    let model_ids: Vec<String> = if !args.models.is_empty() {
        args.models
    } else if let Some(ref defaults) = config.default_models
        && !defaults.is_empty()
        && !use_auto
    {
        defaults.clone()
    } else if use_auto {
        let ids = model_select::auto_select_models_dynamic(&task_str).await;
        if ids.is_empty() {
            anyhow::bail!(
                "No API keys found. Set at least one of: ANTHROPIC_API_KEY, OPENAI_API_KEY, GEMINI_API_KEY, OPENROUTER_API_KEY."
            );
        }
        ids
    } else {
        let selected = model_select::run_model_selector(&task_str).await?;
        if selected.is_empty() {
            anyhow::bail!("No models selected.");
        }
        selected
    };

    tracing::info!("crosstalk session starting, models: {:?}", model_ids);

    // Status helpers for the initialization phase (between model-select TUI and main TUI).
    // These write to stderr so they are visible in normal-mode terminal but do not interfere
    // with any stdout piping.
    let print_status = |msg: &str| {
        eprint!("\r\x1B[K  \x1B[36m▸\x1B[0m {msg}");
        let _ = io::Write::flush(&mut io::stderr());
    };
    let print_error = |msg: &str| {
        eprint!("\r\x1B[K  \x1B[31m✗\x1B[0m {msg}\n");
        let _ = io::Write::flush(&mut io::stderr());
    };

    // 2. Pre-flight credential check
    print_status("Checking credentials...");
    if let Err(e) = ModelFactory::check_env(&model_ids) {
        print_error(&format!("Credential check failed: {e}"));
        anyhow::bail!("Initialization aborted due to missing credentials.");
    }

    // 3. Initialize Agents
    print_status(&format!("Creating {} agent(s)...", model_ids.len()));
    let mut agents: Vec<Box<dyn PromptAgent>> = vec![];
    for m in &model_ids {
        agents.push(ModelFactory::create_agent(m)?);
    }
    if agents.is_empty() {
        anyhow::bail!("No valid models provided. Use --models <model_id>");
    }

    // 3b. Validate agent endpoints in parallel; fallback to OpenRouter on failure
    {
        print_status(&format!("Validating {} agent endpoint(s)...", agents.len()));
        let validation_futures: Vec<_> = agents
            .iter()
            .map(|a| crosstalk::core::factory::validate_agent(a.as_ref()))
            .collect();
        let results = join_all(validation_futures).await;
        let has_openrouter = std::env::var("OPENROUTER_API_KEY").is_ok();
        let mut valid_agents: Vec<Box<dyn PromptAgent>> = Vec::new();
        for (agent, ok) in agents.into_iter().zip(results.into_iter()) {
            if ok {
                valid_agents.push(agent);
            } else {
                let name = agent.name().to_string();
                if has_openrouter
                    && !name.starts_with("openrouter:")
                    && let Ok(fallback) = ModelFactory::create_openrouter_fallback(&name)
                    && crosstalk::core::factory::validate_agent(fallback.as_ref()).await
                {
                    tracing::info!(agent = %name, "native provider failed, using OpenRouter fallback");
                    print_status(&format!("Agent {name}: using OpenRouter fallback"));
                    valid_agents.push(fallback);
                    continue;
                }
                tracing::warn!(agent = %name, "agent validation failed, removing");
                print_status(&format!("Agent {name}: validation failed, skipping"));
            }
        }
        agents = valid_agents;
        if agents.is_empty() {
            print_error("All agents failed endpoint validation. Check model IDs and API keys.");
            anyhow::bail!("All agents failed endpoint validation. Check model IDs and API keys.");
        }
    }

    // 4. Initialize Core State
    print_status("Initializing session state...");
    let session_id = args.resume.clone().unwrap_or_else(|| run_ts.clone());
    let data_dir = std::env::var("XDG_DATA_HOME").unwrap_or_else(|_| {
        std::env::var("HOME")
            .map(|h| format!("{h}/.local/share"))
            .unwrap_or_else(|_| "/tmp".to_string())
    });
    let state_path = format!("{data_dir}/crosstalk/{session_id}");
    log_warn!(
        std::fs::create_dir_all(&state_path),
        "Failed to create state directory"
    );
    let manager = StateManager::new(&state_path)?;
    let sigma = Arc::new(Mutex::new(if args.resume.is_some() {
        // Restore the latest checkpoint if resuming a prior session
        manager
            .list_checkpoints()
            .ok()
            .and_then(|mut idxs| {
                idxs.sort_unstable();
                idxs.last().copied()
            })
            .and_then(|idx| manager.restore(idx).ok().flatten())
            .unwrap_or_else(|| ConversationState::new(&session_id))
    } else {
        ConversationState::new(&session_id)
    }));
    {
        let mut state = sigma.lock().await;
        if args.max_model_calls > 0 {
            state.budget.max_model_calls = args.max_model_calls;
        }
        if args.max_input_tokens > 0 {
            state.budget.max_input_tokens = args.max_input_tokens;
        }
        if args.max_output_tokens > 0 {
            state.budget.max_output_tokens = args.max_output_tokens;
        }
    }

    // Cross-session tamper evidence: verify restored turns against the *pinned*
    // public key (no secret needed), so a swapped seed cannot mask a forgery.
    if args.resume.is_some() {
        match crosstalk::engines::security::TurnVerifier::pinned(manager.db()) {
            Ok(Some(verifier)) => {
                let s = sigma.lock().await;
                let signed = s.turns.iter().filter(|t| !t.signature.is_empty());
                let mut total = 0usize;
                let mut bad = 0usize;
                for turn in signed {
                    total += 1;
                    if !matches!(verifier.verify_turn(turn), Ok(true)) {
                        bad += 1;
                    }
                }
                if bad > 0 {
                    tracing::warn!(
                        session = %session_id,
                        failed = bad,
                        total,
                        "restored turn signatures failed verification; persisted state may have been tampered with"
                    );
                } else if total > 0 {
                    tracing::info!(session = %session_id, verified = total, "restored turn signatures verified");
                }
            }
            Ok(None) => {}
            Err(e) => {
                tracing::warn!(error = %e, "could not load pinned signing identity to verify restored turns");
            }
        }

        // Independent of signatures: verify the transcript hash chain. This
        // catches reordering/insertion/deletion of turns and needs no key.
        let s = sigma.lock().await;
        match s.verify_chain() {
            Some(idx) => tracing::warn!(
                session = %session_id,
                turn = idx,
                "restored transcript hash chain is broken; turns may have been reordered or altered"
            ),
            None => {
                if !s.turn_hashes.is_empty() {
                    tracing::info!(session = %session_id, "restored transcript hash chain intact");
                }
            }
        }
    }

    let effective_workspace = wizard_workspace.as_deref().or(args.workspace.as_deref());

    if let Some(ws) = effective_workspace {
        let patterns = if args.files.is_empty() {
            vec!["**/*".to_string()]
        } else {
            args.files.clone()
        };
        let mut s = sigma.lock().await;
        for pattern in &patterns {
            let full_pattern = format!("{}/{}", ws, pattern);
            const MAX_GLOB_FILES: usize = 10_000;
            let paths: Vec<_> = match glob::glob(&full_pattern) {
                Ok(iter) => iter.flatten().take(MAX_GLOB_FILES + 1).collect(),
                Err(e) => {
                    tracing::warn!("invalid glob pattern '{}': {}", full_pattern, e);
                    continue;
                }
            };
            if paths.len() > MAX_GLOB_FILES {
                tracing::warn!(
                    pattern = %full_pattern,
                    "glob matched more than {} files, truncating",
                    MAX_GLOB_FILES
                );
            }
            let paths = &paths[..paths.len().min(MAX_GLOB_FILES)];
            for path in paths {
                if !path.is_file() || should_skip(path) || !is_likely_text(path) {
                    continue;
                }
                match tokio::fs::read_to_string(&path).await {
                    Ok(content) => {
                        let name = path.strip_prefix(ws).unwrap_or(path).display().to_string();
                        let lang = lang_from_ext(&name);
                        s.ingest_file(name, lang, content);
                    }
                    Err(e) => tracing::debug!("skipping {}: {}", path.display(), e),
                }
            }
        }
        drop(s);
    } else if !args.files.is_empty() {
        let mut s = sigma.lock().await;
        for file_path in &args.files {
            let path = std::path::Path::new(file_path);
            if !path.is_file() || should_skip(path) || !is_likely_text(path) {
                tracing::debug!("skipping {}: not a text file or filtered", file_path);
                continue;
            }
            match tokio::fs::read_to_string(path).await {
                Ok(content) => {
                    let name = path
                        .file_name()
                        .map(|n| n.to_string_lossy().to_string())
                        .unwrap_or_else(|| file_path.clone());
                    let lang = lang_from_ext(&name);
                    s.ingest_file(name, lang, content);
                }
                Err(e) => tracing::warn!("failed to read {}: {}", file_path, e),
            }
        }
        drop(s);
    }

    // Repository records become immutable, provenance-rich artifacts in the
    // same context path as local evidence. A failed repository must not erase
    // results from another source or prevent an offline session from running.
    let research_queries = [
        (
            crosstalk::engines::research::ResearchRepository::Arxiv,
            args.arxiv.as_ref(),
        ),
        (
            crosstalk::engines::research::ResearchRepository::Zenodo,
            args.zenodo.as_ref(),
        ),
    ];
    if research_queries.iter().any(|(_, query)| query.is_some()) {
        let research_client = crosstalk::engines::research::ResearchClient::new(
            std::env::var("ZENODO_ACCESS_TOKEN").ok(),
        )?;
        for (repository, query) in research_queries {
            let Some(query) = query else { continue };
            match research_client
                .search(repository, query, args.research_limit)
                .await
            {
                Ok(records) => {
                    let name = match repository {
                        crosstalk::engines::research::ResearchRepository::Arxiv => {
                            "research/arxiv.json"
                        }
                        crosstalk::engines::research::ResearchRepository::Zenodo => {
                            "research/zenodo.json"
                        }
                    };
                    let content = serde_json::to_string_pretty(&records)?;
                    let mut state = sigma.lock().await;
                    for record in &records {
                        use sha2::{Digest, Sha256};
                        let evidence_id =
                            format!("research:{:?}:{}", record.repository, record.identifier)
                                .to_ascii_lowercase();
                        if state.investigation.evidence.contains_key(&evidence_id) {
                            continue;
                        }
                        let record_bytes = serde_json::to_vec(record)?;
                        let mut metadata = std::collections::BTreeMap::new();
                        metadata.insert(
                            "repository_response_sha256".into(),
                            record.response_sha256.clone(),
                        );
                        metadata.insert("authors".into(), record.authors.join("; "));
                        if let Some(published) = &record.published {
                            metadata.insert("published".into(), published.clone());
                        }
                        if let Some(doi) = &record.doi {
                            metadata.insert("doi".into(), doi.clone());
                        }
                        if let Err(error) = state.investigation.register_evidence(
                            crosstalk::types::investigation::EvidenceArtifact {
                                id: evidence_id,
                                kind: crosstalk::types::investigation::EvidenceKind::SourceRecord,
                                title: record.title.clone(),
                                content_sha256: format!("{:x}", Sha256::digest(&record_bytes)),
                                media_type: "application/json".into(),
                                source_uri: Some(record.canonical_url.clone()),
                                locator: None,
                                artifact_name: Some(name.into()),
                                verification_id: None,
                                captured_at: record.retrieved_at,
                                independent: false,
                                metadata,
                            },
                        ) {
                            tracing::warn!(%error, "failed to register research evidence");
                        }
                    }
                    state.ingest_file(name.to_string(), "json".to_string(), content);
                    tracing::info!(
                        ?repository,
                        records = records.len(),
                        "research records attached"
                    );
                }
                Err(error) => {
                    tracing::warn!(?repository, %error, "research repository query failed")
                }
            }
        }
    }

    if args.evolve_generations > 0 {
        print_status(&format!(
            "Running {} native evolution generation(s)...",
            args.evolve_generations
        ));
        let mut request = crosstalk::engines::idea_evolution::EvolutionRequest::new(
            &session_id,
            &task_str,
            vec![crosstalk::engines::idea_evolution::IdeaSeed {
                id: "user-problem-seed".into(),
                domain: "Cross-domain".into(),
                title: task_str.chars().take(160).collect(),
                mechanism:
                    "Initial user problem framing; evolve concrete mechanisms from this seed".into(),
            }],
        );
        request.population_size = args.evolve_population;
        request.generations = args.evolve_generations;
        request.max_concurrency = args.evolve_concurrency;
        request.constraints = args.evolve_constraints.clone();
        request.structural_exclusions = args
            .evolve_exclusions
            .iter()
            .enumerate()
            .map(
                |(index, exclusion)| crosstalk_evolution::StructuralExclusion {
                    id: format!("excluded-{}", index + 1),
                    description: exclusion.clone(),
                    structural_features: exclusion
                        .split(';')
                        .map(str::trim)
                        .filter(|feature| !feature.is_empty())
                        .map(str::to_string)
                        .collect(),
                    evidence_ids: vec![],
                },
            )
            .collect();
        let requested_model_call_slots = if args.evolve_max_model_calls == 0 {
            (args.evolve_population as u64)
                .saturating_mul(args.evolve_generations as u64)
                .saturating_mul(2)
        } else {
            args.evolve_max_model_calls
        };
        request.max_model_call_slots = {
            let mut state = sigma.lock().await;
            state
                .budget
                .reserve_model_call_slots(requested_model_call_slots)
        };
        if request.max_model_call_slots < requested_model_call_slots {
            tracing::warn!(
                requested = requested_model_call_slots,
                admitted = request.max_model_call_slots,
                "native evolution was reduced by the shared session model-call limit"
            );
        }
        if request.max_model_call_slots < 2 {
            let mut state = sigma.lock().await;
            state
                .budget
                .release_unused_model_call_slots(request.max_model_call_slots);
            tracing::warn!("native evolution skipped because fewer than two call slots remain");
        } else {
            {
                use sha2::{Digest, Sha256};
                let state = sigma.lock().await;
                request.evidence_ids = state.artifacts.keys().cloned().collect();
                request.evidence_context = state
                    .artifacts
                    .iter()
                    .take(32)
                    .map(|(id, artifact)| {
                        let end = artifact
                            .content
                            .floor_char_boundary(artifact.content.len().min(8_000));
                        let sanitized = crosstalk::engines::security::InjectionShield::sanitize(
                            &artifact.content[..end],
                        );
                        let sanitized_end =
                            sanitized.floor_char_boundary(sanitized.len().min(8_000));
                        let excerpt = sanitized[..sanitized_end].to_string();
                        crosstalk::engines::idea_evolution::EvidenceExcerpt {
                            id: id.clone(),
                            content_sha256: format!(
                                "{:x}",
                                Sha256::digest(artifact.content.as_bytes())
                            ),
                            excerpt,
                        }
                    })
                    .collect();
            }
            let variation_agent = agents[0].as_ref();
            let critic_agent = agents
                .get(1)
                .map_or(variation_agent, |agent| agent.as_ref());
            let latest_evolution = Arc::new(std::sync::Mutex::new(None));
            let progress_slot = Arc::clone(&latest_evolution);
            let requested_generations = request.generations;
            let evolution = tokio::time::timeout(
                Duration::from_secs(args.evolve_timeout_secs.max(1)),
                crosstalk::engines::idea_evolution::run_native_evolution_with_agents_reporting(
                    variation_agent,
                    critic_agent,
                    &request,
                    args.evolve_seed,
                    move |progress| {
                        print_status(&format!(
                            "Native evolution generation {}/{} complete ({} survivor(s))...",
                            progress.reports.len(),
                            requested_generations,
                            progress.response.ideas.len()
                        ));
                        if let Ok(mut latest) = progress_slot.lock() {
                            *latest = Some(progress.clone());
                        }
                    },
                ),
            )
            .await;
            match evolution {
                Ok(Ok(outcome)) => {
                    let used_slots = outcome
                        .reports
                        .last()
                        .map_or(0, |report| report.usage.model_call_slots_reserved);
                    sigma.lock().await.budget.release_unused_model_call_slots(
                        request.max_model_call_slots.saturating_sub(used_slots),
                    );
                    outcome.response.validate().map_err(anyhow::Error::msg)?;
                    let candidates = serde_json::to_string_pretty(&outcome.response)?;
                    let reports = serde_json::to_string_pretty(&outcome.reports)?;
                    let status = serde_json::to_string_pretty(&serde_json::json!({
                        "schema": "crosstalk.evolution.status.v1",
                        "status": "completed",
                        "requested_generations": request.generations,
                        "completed_generations": outcome.reports.len(),
                        "requested_model_call_slots": requested_model_call_slots,
                        "admitted_model_call_slots": request.max_model_call_slots,
                        "used_model_call_slots": used_slots,
                        "survivors": outcome.response.ideas.len(),
                    }))?;
                    let mut state = sigma.lock().await;
                    state.ingest_file(
                        "evolution/native-candidates.json".into(),
                        "json".into(),
                        candidates,
                    );
                    state.ingest_file(
                        "evolution/checkpoint.json".into(),
                        "json".into(),
                        outcome.checkpoint_json,
                    );
                    state.ingest_file(
                        "evolution/generation-reports.json".into(),
                        "json".into(),
                        reports,
                    );
                    state.ingest_file("evolution/status.json".into(), "json".into(), status);
                    tracing::info!(
                        ideas = outcome.response.ideas.len(),
                        variation_agent = variation_agent.name(),
                        critic_agent = critic_agent.name(),
                        "native evolution completed"
                    );
                }
                Ok(Err(error)) => {
                    let status = serde_json::to_string_pretty(&serde_json::json!({
                        "schema": "crosstalk.evolution.status.v1",
                        "status": "failed",
                        "error": error,
                        "requested_generations": request.generations,
                        "completed_generations": 0,
                        "requested_model_call_slots": requested_model_call_slots,
                        "admitted_model_call_slots": request.max_model_call_slots,
                    }))?;
                    sigma.lock().await.ingest_file(
                        "evolution/status.json".into(),
                        "json".into(),
                        status,
                    );
                    tracing::warn!(%error, "native evolution failed; continuing without candidates")
                }
                Err(_) => {
                    let partial = latest_evolution
                        .lock()
                        .ok()
                        .and_then(|latest| latest.clone());
                    let completed_generations =
                        partial.as_ref().map_or(0, |outcome| outcome.reports.len());
                    let used_slots = partial
                        .as_ref()
                        .and_then(|outcome| outcome.reports.last())
                        .map_or(request.max_model_call_slots, |report| {
                            report.usage.model_call_slots_reserved
                        });
                    let status = serde_json::to_string_pretty(&serde_json::json!({
                        "schema": "crosstalk.evolution.status.v1",
                        "status": "timed_out",
                        "timeout_seconds": args.evolve_timeout_secs,
                        "requested_generations": request.generations,
                        "completed_generations": completed_generations,
                        "requested_model_call_slots": requested_model_call_slots,
                        "admitted_model_call_slots": request.max_model_call_slots,
                        "known_used_model_call_slots": used_slots,
                        "partial_checkpoint_retained": partial.is_some(),
                    }))?;
                    let mut state = sigma.lock().await;
                    if let Some(outcome) = partial {
                        outcome.response.validate().map_err(anyhow::Error::msg)?;
                        state.ingest_file(
                            "evolution/native-candidates.json".into(),
                            "json".into(),
                            serde_json::to_string_pretty(&outcome.response)?,
                        );
                        state.ingest_file(
                            "evolution/checkpoint.json".into(),
                            "json".into(),
                            outcome.checkpoint_json,
                        );
                        state.ingest_file(
                            "evolution/generation-reports.json".into(),
                            "json".into(),
                            serde_json::to_string_pretty(&outcome.reports)?,
                        );
                    }
                    state.ingest_file("evolution/status.json".into(), "json".into(), status);
                    tracing::warn!(
                        timeout_secs = args.evolve_timeout_secs,
                        completed_generations,
                        "native evolution timed out; retained completed progress"
                    );
                }
            }
        }
    }

    let (event_tx, event_rx) = mpsc::channel::<StreamEvent>(1000);
    let (control_tx, control_rx) = mpsc::channel::<ControlSignal>(100);

    // 5. Initialize Orchestrator (may fail if engines fail to init)
    print_status("Starting orchestration engine...");
    let workspace_root = effective_workspace.map(std::path::PathBuf::from);
    let omicron =
        match Orchestrator::new(manager, agents, event_tx, control_rx, workspace_root).await {
            Ok(o) => o,
            Err(e) => {
                print_error(&format!("Orchestrator init failed: {e}"));
                anyhow::bail!("Failed to start orchestration engine.");
            }
        };

    let task = task_str;
    {
        let mut state = sigma.lock().await;
        if state.investigation.id.is_empty() {
            state.investigation.id = session_id.clone();
        }
        if state.investigation.problem.statement.trim().is_empty() {
            state.investigation.problem.statement = task.clone();
            state.investigation.updated_at = ConversationState::now();
        }
    }
    let task_content = {
        let s = sigma.lock().await;
        if s.artifacts.is_empty() {
            task.clone()
        } else {
            let file_names: Vec<&str> = s.artifacts.keys().map(|k| k.as_str()).collect();
            let names_str = file_names.join(", ");
            let edit_instruction = if args.edit {
                format!(
                    "\n\n[EDIT MODE] After reaching consensus, produce a final artifact named exactly `{}` containing the complete revised document with all agreed changes applied.",
                    names_str
                )
            } else {
                String::new()
            };
            format!(
                "{}\n\n[GROUNDING CONSTRAINT] All claims must be grounded in the attached document(s): {}. Quote the specific section you are referencing. Do not assert implementation details, algorithms, or assumptions not explicitly stated in the text.{}\n\n[Workspace: {} file(s) loaded]",
                task,
                names_str,
                edit_instruction,
                s.artifacts.len()
            )
        }
    };

    {
        let mut s = sigma.lock().await;
        s.push_turn(Turn {
            index: 0,
            model_id: "User".to_string(),
            content: task_content,
            timestamp: ConversationState::now(),
            diffs: vec![],
            certainty: Some(1.0),
            outcome: TurnOutcome::Unknown,
            task_category: Some(TaskCategory::Research),
            structure: Some(TurnStructure::FreeForm),
            signature: vec![],
            surprise_signal: None,
            consistency_score: None,
            diff_quality_score: None,
            persona_disclosure: None,
        });
        s.iteration_index = 1;
    }

    let initial_mode_idx = {
        let mut s = sigma.lock().await;
        let idx = crosstalk::types::mode::ModeDefinition::detect_preset_index(&task);
        s.mode_library.switch_to_index(idx);
        idx
    };

    let app = Arc::new(Mutex::new(App::new(&session_id)));
    {
        let mut a = app.lock().await;
        {
            let s = sigma.lock().await;
            a.current_mode_name = s.mode_library.current_name().to_string();
        }
        if initial_mode_idx != 0 {
            let mode_name = a.current_mode_name.clone();
            a.push_event(format!("Initial mode: {}", mode_name));
        }
        a.push_event(format!("Session started with {} agent(s)", model_ids.len()));
        for m in &model_ids {
            a.agent_list.push(m.clone());
            a.push_event(format!("  Agent: {}", m));
        }
        let artifact_count = sigma.lock().await.artifacts.len();
        if artifact_count > 0 {
            a.push_event(format!("Workspace: {} files loaded", artifact_count));
        }
    }

    // 6. Set panic hook before spawning any tasks
    let prev_hook = std::panic::take_hook();
    let restore_terminal_on_panic = !args.headless;
    std::panic::set_hook(Box::new(move |info| {
        if restore_terminal_on_panic {
            log_warn!(disable_raw_mode(), "Failed to disable raw mode");
            log_warn!(
                io::stdout().execute(LeaveAlternateScreen),
                "Failed to leave alternate screen"
            );
        }
        prev_hook(info);
    }));

    // 7. Spawn background tasks
    let sigma_orch = Arc::clone(&sigma);
    let app_orch = Arc::clone(&app);
    let iterations = wizard_iterations;
    let timeout_secs = if args.agent_timeout_secs != 300 {
        args.agent_timeout_secs
    } else {
        config.agent_timeout_secs.unwrap_or(300)
    };
    let turn_timeout = Duration::from_secs(timeout_secs);
    let omicron_orch = Arc::new(omicron);
    let omicron_spawn = Arc::clone(&omicron_orch);
    // Structured cancellation for the background orchestrator loop so it can be
    // drained gracefully on shutdown instead of being dropped mid-turn (H-040).
    let cancel_scope = CancelScope::new();
    cancel_scope.spawn(run_orchestrator_loop(
        app_orch,
        sigma_orch,
        omicron_spawn,
        iterations,
        turn_timeout,
        cancel_scope.clone(),
    ));

    let mut event_rx = event_rx;
    let ctrl_tx = control_tx;

    // SIGTERM handler: mirrors Ctrl+C shutdown path
    #[cfg(unix)]
    {
        let ctrl_tx_sigterm = ctrl_tx.clone();
        let app_sigterm = Arc::clone(&app);
        tokio::spawn(async move {
            use tokio::signal::unix::{SignalKind, signal};
            if let Ok(mut sig) = signal(SignalKind::terminate()) {
                sig.recv().await;
                tracing::info!("SIGTERM received, initiating graceful shutdown");
                if let Err(e) = ctrl_tx_sigterm.send(ControlSignal::Shutdown).await {
                    tracing::warn!("Failed to send shutdown signal: {e}");
                }
                app_sigterm.lock().await.shutdown = true;
            }
        });
    }

    if args.headless {
        print_status("Running headless orchestration...\n");
        loop {
            {
                let mut current = app.lock().await;
                ui_events::drain_stream_events(&mut current, &mut event_rx);
                if current.shutdown {
                    break;
                }
            }
            tokio::select! {
                _ = tokio::time::sleep(Duration::from_millis(25)) => {}
                signal = tokio::signal::ctrl_c() => {
                    if let Err(error) = signal {
                        tracing::warn!(%error, "failed to listen for Ctrl-C");
                    }
                    let _ = ctrl_tx.send(ControlSignal::Shutdown).await;
                    app.lock().await.shutdown = true;
                    break;
                }
            }
        }
    } else {
        // 8. Initialize TUI
        print_status("Launching TUI...\n");
        enable_raw_mode()?;
        io::stdout().execute(MoveTo(0, 0))?;
        io::stdout().execute(EnterAlternateScreen)?;
        let mut terminal = Terminal::new(CrosstermBackend::new(io::stdout()))?;
        terminal.clear()?;

        // 9. Main loop: drain events, handle keys, render
        loop {
            let action = {
                let mut a = app.lock().await;
                ui_events::drain_stream_events(&mut a, &mut event_rx);
                if a.shutdown {
                    break;
                }

                let action = match ui_events::poll_key(Duration::from_millis(16)) {
                    Some(key) => ui_events::handle_key(&mut a, key),
                    None => Action::None,
                };

                a.tick_fps();
                action
            };

            {
                let a = app.lock().await;
                if a.shutdown {
                    break;
                }
                terminal.draw(|f| render::draw(f, &a))?;
            }

            match action {
                Action::Shutdown => {
                    if ctrl_tx.send(ControlSignal::Shutdown).await.is_err() {
                        tracing::warn!("failed to send shutdown signal; forcing shutdown flag");
                        app.lock().await.shutdown = true;
                    }
                    break;
                }
                Action::Send(sig) => {
                    if ctrl_tx.send(sig).await.is_err() {
                        tracing::warn!("failed to send control signal; shutting down");
                        app.lock().await.shutdown = true;
                        break;
                    }
                }
                Action::SendTwo(s1, s2) => {
                    if ctrl_tx.send(s1).await.is_err() || ctrl_tx.send(s2).await.is_err() {
                        tracing::warn!("failed to send control signal; shutting down");
                        app.lock().await.shutdown = true;
                        break;
                    }
                }
                Action::None => {}
            }
        }

        disable_raw_mode()?;
        io::stdout().execute(LeaveAlternateScreen)?;
    }

    // Drain the background orchestrator loop before finalizing so no turn is
    // mid-write. Bounded so shutdown can never hang on an in-flight turn.
    app.lock().await.shutdown = true;
    cancel_scope.cancel();
    if tokio::time::timeout(Duration::from_secs(5), cancel_scope.shutdown_graceful())
        .await
        .is_err()
    {
        tracing::warn!("background orchestrator did not drain within 5s; proceeding with shutdown");
    }

    // Graceful shutdown: finalize session, persist memory, shut down orchestrator.
    log_warn!(
        omicron_orch.finalize_session(Arc::clone(&sigma)).await,
        "session finalization failed"
    );
    {
        let bridge = omicron_orch.memory_bridge.lock().await;
        let session_id = sigma.lock().await.session_id.clone();
        let records = bridge.take_snapshot(&session_id);
        for record in records {
            log_warn!(
                omicron_orch.memory_store.store(record).await,
                "failed to persist memory record"
            );
        }
    }
    omicron_orch.shutdown().await;

    if args.edit {
        let ws_root = effective_workspace.map(std::path::Path::new);
        let canonical_root = ws_root.and_then(|p| p.canonicalize().ok());
        let s = sigma.lock().await;
        for (name, artifact) in &s.artifacts {
            if name.contains("..") || name.starts_with('/') || name.starts_with('\\') {
                tracing::warn!(artifact = %name, "skipping artifact with unsafe path");
                continue;
            }
            let target = ws_root
                .map(|ws| ws.join(name))
                .unwrap_or_else(|| std::path::PathBuf::from(name));
            if let Some(ref root) = canonical_root
                && let Ok(ct) = target.canonicalize()
                && !ct.starts_with(root)
            {
                tracing::warn!(artifact = %name, "artifact path escapes workspace");
                continue;
            }
            if target.exists() && artifact.version > 1 {
                match tokio::fs::write(&target, &artifact.content).await {
                    Ok(()) => {
                        tracing::info!(path = %target.display(), bytes = artifact.content.len(), "edit-mode wrote artifact")
                    }
                    Err(e) => {
                        tracing::warn!(path = %target.display(), err = %e, "edit-mode write failed")
                    }
                }
            }
        }
        drop(s);
    }

    if let Some(output_dir) = &args.bundle_dir {
        let state = sigma.lock().await;
        let manifest =
            crosstalk::engines::investigation_bundle::InvestigationBundleExporter::export(
                &state,
                output_dir,
                crosstalk::engines::investigation_bundle::BundleOptions::default(),
            )?;
        let verification =
            crosstalk::engines::investigation_bundle::InvestigationBundleExporter::verify(
                output_dir,
            )?;
        if !verification.passed {
            anyhow::bail!(
                "newly exported investigation bundle failed verification: {}",
                verification.issues.join("; ")
            );
        }
        eprintln!(
            "Investigation bundle: {} (integrity={}, scientific_release={}, files={})",
            output_dir.display(),
            if verification.passed { "PASS" } else { "FAIL" },
            if manifest
                .scientific_release
                .as_ref()
                .is_some_and(|assessment| assessment.eligible)
            {
                "ELIGIBLE"
            } else {
                "NOT_ESTABLISHED"
            },
            manifest.files.len()
        );
    }

    if args.headless {
        let state = sigma.lock().await;
        let audit = state.investigation.audit(&state.claim_ledger);
        let scientific_release = state
            .investigation
            .scientific_release_assessment(&state.claim_ledger);
        let release_warning = scientific_release.unverified_warning();
        let latest_attempt = state
            .turns
            .iter()
            .rev()
            .find(|turn| turn.model_id != "User")
            .map(|turn| format!("{:?}", turn.outcome));
        let final_response = state
            .turns
            .iter()
            .rev()
            .find(|turn| {
                turn.model_id != "User"
                    && !matches!(
                        turn.outcome,
                        TurnOutcome::Rejected
                            | TurnOutcome::RolledBack
                            | TurnOutcome::VerificationFailed
                    )
            })
            .map(|turn| turn.content.as_str())
            .unwrap_or("");
        match args.headless_format {
            HeadlessFormat::Json => {
                println!(
                    "{}",
                    serde_json::to_string_pretty(&serde_json::json!({
                        "schema": "crosstalk.headless.v1",
                        "session_id": state.session_id,
                        "final_response": final_response,
                        "latest_attempt_outcome": latest_attempt.as_deref(),
                        "completion_probability": state.completion_probability,
                        "chain_head": state.chain_head_hex(),
                        "audit": audit,
                        "scientific_release": scientific_release,
                        "result_status": if scientific_release.eligible {
                            "established"
                        } else {
                            "unverified_model_synthesis"
                        },
                        "release_warning": release_warning,
                        "artifact_names": state.artifacts.keys().collect::<Vec<_>>(),
                    }))?
                );
            }
            HeadlessFormat::Markdown => {
                println!("# Crosstalk result\n");
                println!("- Session: `{}`", state.session_id);
                println!(
                    "- Latest attempt outcome: `{}`",
                    latest_attempt.as_deref().unwrap_or("none")
                );
                println!(
                    "- Evidence integrity audit: **{}**",
                    if audit.passed { "PASS" } else { "FAIL" }
                );
                println!(
                    "- Verification coverage: {:.1}%\n",
                    audit.verification_coverage * 100.0
                );
                println!(
                    "- Scientific release: **{}**\n",
                    if scientific_release.eligible {
                        "ELIGIBLE"
                    } else {
                        "NOT ESTABLISHED"
                    }
                );
                if let Some(warning) = &release_warning {
                    println!("> [!WARNING]\n> {warning}\n");
                }
                println!(
                    "## {}\n\n{final_response}",
                    if scientific_release.eligible {
                        "Established result"
                    } else {
                        "Unverified synthesis"
                    }
                );
            }
        }
    }

    // Print session summary
    {
        let s = sigma.lock().await;
        let a = app.lock().await;
        let turns = s.turns.len().saturating_sub(1); // exclude initial user turn
        let artifacts = s.artifacts.len();
        let conv = s.completion_probability;
        let errors: Vec<&String> = a
            .recent_events
            .iter()
            .filter(|e| e.contains("ERROR") || e.contains("error:") || e.contains("PANIC"))
            .collect();
        eprintln!("\n--- Crosstalk Session Summary ---");
        eprintln!("  Turns completed: {}", turns);
        eprintln!("  Artifacts:       {}", artifacts);
        eprintln!("  Convergence:     {:.1}%", conv * 100.0);
        if !errors.is_empty() {
            eprintln!("  Errors ({}):", errors.len());
            for e in errors.iter().take(5) {
                eprintln!("    {}", e);
            }
            if errors.len() > 5 {
                eprintln!("    ... and {} more", errors.len() - 5);
            }
        }
        eprintln!("  Log: {}", run_log.display());
        eprintln!("---");
    }

    tracing::info!("session complete");
    drop(_guard);

    Ok(())
}
