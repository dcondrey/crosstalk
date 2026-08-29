use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::sync::Arc;
use std::time::Instant;
use wasmtime::*;
use wasmtime_wasi::WasiCtxBuilder;
use wasmtime_wasi::pipe::MemoryOutputPipe;
use wasmtime_wasi::preview1::{self, WasiP1Ctx};

/// Default execution timeout in seconds for sandbox operations.
const DEFAULT_TIMEOUT_SECS: u64 = 30;
/// Epoch ticks fire once per second from the free-running incrementer in
/// `SandboxManager::new`, so a deadline of k ticks buys somewhere in (k-1, k]
/// seconds depending on when the call starts relative to the ticker.
///
/// IMPORTANT: the epoch deadline is a liveness backstop only. Fuel is the
/// deterministic bound, and the evaluator's reproduction gate requires
/// bit-identical measurements, so anything that makes the wall clock the
/// binding constraint turns a valid result into a coin flip. The deadline is
/// therefore derived from the operator's `timeout_secs` with one extra tick
/// covering the ticker phase.
const EPOCH_PHASE_SLACK_TICKS: u64 = 1;

#[derive(Debug, Clone)]
pub struct SandboxConfig {
    pub memory_limit_bytes: usize,
    pub cpu_fuel_limit: u64,
    /// Maximum wall-clock seconds before the sandbox execution is aborted.
    pub timeout_secs: u64,
}

struct SandboxState {
    wasi: WasiP1Ctx,
    limits: StoreLimits,
}

impl Default for SandboxConfig {
    fn default() -> Self {
        Self {
            memory_limit_bytes: 256 * 1024 * 1024,
            cpu_fuel_limit: 100_000_000,
            timeout_secs: DEFAULT_TIMEOUT_SECS,
        }
    }
}

pub struct SandboxResult {
    pub exit_code: i32,
    pub stdout: String,
    pub stderr: String,
    /// CPU fuel consumed by the execution, when fuel metering reported a value.
    pub fuel_consumed: Option<u64>,
    /// Wall-clock duration of the execution in milliseconds.
    pub elapsed_ms: u64,
    /// True when execution was killed by a resource limit (fuel exhaustion or
    /// the epoch deadline) rather than trapping for an ordinary reason.
    pub resource_limit_hit: bool,
}

/// One private test vector for a candidate that exports `(i64) -> i64`.
/// Test vectors are deliberately owned by the evaluator rather than embedded
/// in public discovery reports.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct I64TestCase {
    pub input: i64,
    pub expected: i64,
}

/// Non-secret outcome data for one hidden case. Inputs, expected values, and
/// actual values are intentionally omitted so a report cannot become an oracle
/// for the held-out test set.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct I64CaseOutcome {
    pub index: usize,
    pub passed: bool,
    pub fuel_consumed: u64,
    pub trapped: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct I64EvaluationResult {
    pub outcomes: Vec<I64CaseOutcome>,
    pub elapsed_ms: u64,
    pub fuel_consumed: u64,
    pub resource_limit_hit: bool,
    pub trapped: bool,
}

impl I64EvaluationResult {
    #[must_use]
    pub fn correct_cases(&self) -> usize {
        self.outcomes
            .iter()
            .filter(|outcome| outcome.passed)
            .count()
    }

    #[must_use]
    pub fn all_cases_correct(&self, expected_count: usize) -> bool {
        self.outcomes.len() == expected_count
            && self.outcomes.iter().all(|outcome| outcome.passed)
            && !self.trapped
    }
}

pub struct SandboxManager {
    engine: Engine,
    config: SandboxConfig,
    _epoch_task: tokio::task::JoinHandle<()>,
}

impl SandboxManager {
    /// Epoch ticks granted to a single call. REQUIRED: this MUST stay derived
    /// from `timeout_secs`. A constant here silently caps every execution at
    /// that many seconds regardless of what the operator configured, which
    /// breaks the evaluator's bit-identical reproduction contract.
    pub fn epoch_deadline_ticks(&self) -> u64 {
        self.config
            .timeout_secs
            .saturating_add(EPOCH_PHASE_SLACK_TICKS)
    }

    pub fn new(config: SandboxConfig) -> Result<Self> {
        anyhow::ensure!(
            config.memory_limit_bytes > 0,
            "SandboxConfig.memory_limit_bytes must be > 0"
        );
        anyhow::ensure!(
            config.cpu_fuel_limit > 0,
            "SandboxConfig.cpu_fuel_limit must be > 0"
        );
        anyhow::ensure!(
            config.timeout_secs > 0,
            "SandboxConfig.timeout_secs must be > 0"
        );
        let mut wasm_cfg = Config::new();
        wasm_cfg.consume_fuel(true);
        wasm_cfg.epoch_interruption(true);
        let engine = Engine::new(&wasm_cfg)?;

        // Start background epoch incrementer
        let engine_clone = engine.clone();
        let epoch_task = tokio::spawn(async move {
            let mut interval = tokio::time::interval(tokio::time::Duration::from_secs(1));
            loop {
                interval.tick().await;
                engine_clone.increment_epoch();
            }
        });

        Ok(Self {
            engine,
            config,
            _epoch_task: epoch_task,
        })
    }

    /// Execute WASM bytes synchronously (blocking). Prefer `execute_with_timeout`
    /// for async contexts to avoid hanging the executor.
    pub fn execute(&self, wasm_bytes: &[u8]) -> Result<SandboxResult> {
        let start = Instant::now();
        let mut linker = Linker::new(&self.engine);
        preview1::add_to_linker_sync(&mut linker, |s: &mut SandboxState| &mut s.wasi)?;

        let stdout_pipe = MemoryOutputPipe::new(1024 * 1024);
        let stderr_pipe = MemoryOutputPipe::new(1024 * 1024);

        let wasi = WasiCtxBuilder::new()
            .stdout(stdout_pipe.clone())
            .stderr(stderr_pipe.clone())
            .build_p1();

        let limits = StoreLimitsBuilder::new()
            .memory_size(self.config.memory_limit_bytes)
            .build();

        let mut store = Store::new(&self.engine, SandboxState { wasi, limits });
        store.limiter(|state| &mut state.limits);
        store.set_fuel(self.config.cpu_fuel_limit)?;
        store.set_epoch_deadline(self.epoch_deadline_ticks());

        let module = Module::from_binary(&self.engine, wasm_bytes)
            .context("failed to compile WASM module from provided bytes")?;
        linker
            .module(&mut store, "", &module)
            .context("failed to link WASM module")?;

        let func = linker
            .get_default(&mut store, "")
            .context("no default export in WASM module")?
            .typed::<(), ()>(&store)
            .context("default export has unexpected signature (expected () -> ())")?;

        let res = func.call(&mut store, ());
        let fuel_consumed = store
            .get_fuel()
            .ok()
            .map(|f| self.config.cpu_fuel_limit - f);
        let elapsed_ms = start.elapsed().as_millis() as u64;

        let (exit_code, resource_limit_hit) = match res {
            Ok(_) => (0, false),
            Err(e) => {
                // A fuel-exhaustion or epoch-deadline trap is a resource-limit
                // kill, not an ordinary program failure; surface the distinction
                // so callers can tell a runaway module from a normal non-zero exit.
                let limited = matches!(
                    e.downcast_ref::<Trap>(),
                    Some(Trap::OutOfFuel | Trap::Interrupt)
                );
                if limited {
                    tracing::warn!("WASM execution hit a resource limit: {e}");
                } else {
                    tracing::warn!("WASM execution failed: {e}");
                }
                (1, limited)
            }
        };

        let stdout = String::from_utf8_lossy(&stdout_pipe.contents()).into_owned();
        let stderr = String::from_utf8_lossy(&stderr_pipe.contents()).into_owned();

        Ok(SandboxResult {
            exit_code,
            stdout,
            stderr,
            fuel_consumed,
            elapsed_ms,
            resource_limit_hit,
        })
    }

    /// Execute WASM bytes with a wall-clock timeout guard. The blocking WASM
    /// execution runs on the tokio blocking thread pool so it cannot stall the
    /// async reactor, and `tokio::time::timeout` enforces the deadline.
    pub async fn execute_with_timeout(
        self: &Arc<Self>,
        wasm_bytes: &[u8],
    ) -> Result<SandboxResult> {
        let timeout = tokio::time::Duration::from_secs(self.config.timeout_secs);
        let bytes = wasm_bytes.to_vec();
        let this = Arc::clone(self);

        let result = tokio::time::timeout(
            timeout,
            tokio::task::spawn_blocking(move || this.execute(&bytes)),
        )
        .await;

        match result {
            Ok(Ok(inner)) => inner,
            Ok(Err(join_err)) => Err(anyhow::anyhow!(
                "sandbox execution task panicked: {join_err}"
            )),
            Err(_elapsed) => Err(anyhow::anyhow!(
                "sandbox execution timed out after {}s",
                self.config.timeout_secs
            )),
        }
    }

    /// Evaluate a pure `(i64) -> i64` export against held-out cases. The module
    /// is compiled once, but every case receives a fresh instance and store so
    /// mutable globals or linear memory cannot leak information between cases.
    /// A single fuel budget is shared by the complete evaluation.
    pub fn evaluate_i64_cases(
        &self,
        wasm_bytes: &[u8],
        export_name: &str,
        cases: &[I64TestCase],
    ) -> Result<I64EvaluationResult> {
        anyhow::ensure!(
            !cases.is_empty() && cases.len() <= 10_000,
            "hidden test set must contain between 1 and 10000 cases"
        );
        anyhow::ensure!(
            !export_name.trim().is_empty() && export_name.len() <= 256,
            "WASM export name must contain between 1 and 256 bytes"
        );

        let start = Instant::now();
        let module = Module::from_binary(&self.engine, wasm_bytes)
            .context("failed to compile WASM candidate")?;
        let mut remaining_fuel = self.config.cpu_fuel_limit;
        let mut outcomes = Vec::with_capacity(cases.len());
        let mut resource_limit_hit = false;
        let mut trapped = false;

        for (index, case) in cases.iter().enumerate() {
            if remaining_fuel == 0 {
                resource_limit_hit = true;
                break;
            }

            let mut linker = Linker::new(&self.engine);
            preview1::add_to_linker_sync(&mut linker, |state: &mut SandboxState| &mut state.wasi)?;
            let wasi = WasiCtxBuilder::new().build_p1();
            let limits = StoreLimitsBuilder::new()
                .memory_size(self.config.memory_limit_bytes)
                .build();
            let mut store = Store::new(&self.engine, SandboxState { wasi, limits });
            store.limiter(|state| &mut state.limits);
            store.set_fuel(remaining_fuel)?;
            store.set_epoch_deadline(self.epoch_deadline_ticks());

            let instance = linker
                .instantiate(&mut store, &module)
                .context("failed to instantiate WASM candidate")?;
            let function = instance
                .get_typed_func::<i64, i64>(&mut store, export_name)
                .with_context(|| {
                    format!("missing or invalid WASM export {export_name:?}; expected (i64) -> i64")
                })?;
            let call = function.call(&mut store, case.input);
            let fuel_after = store.get_fuel().unwrap_or_default();
            let consumed = remaining_fuel.saturating_sub(fuel_after);
            remaining_fuel = fuel_after;

            match call {
                Ok(actual) => outcomes.push(I64CaseOutcome {
                    index,
                    passed: actual == case.expected,
                    fuel_consumed: consumed,
                    trapped: false,
                }),
                Err(error) => {
                    resource_limit_hit = matches!(
                        error.downcast_ref::<Trap>(),
                        Some(Trap::OutOfFuel | Trap::Interrupt)
                    );
                    trapped = true;
                    outcomes.push(I64CaseOutcome {
                        index,
                        passed: false,
                        fuel_consumed: consumed,
                        trapped: true,
                    });
                    break;
                }
            }
        }

        Ok(I64EvaluationResult {
            outcomes,
            elapsed_ms: start.elapsed().as_millis() as u64,
            fuel_consumed: self.config.cpu_fuel_limit.saturating_sub(remaining_fuel),
            resource_limit_hit,
            trapped,
        })
    }

    /// Async wall-clock guard for [`Self::evaluate_i64_cases`].
    pub async fn evaluate_i64_cases_with_timeout(
        self: &Arc<Self>,
        wasm_bytes: &[u8],
        export_name: &str,
        cases: &[I64TestCase],
    ) -> Result<I64EvaluationResult> {
        let timeout = tokio::time::Duration::from_secs(self.config.timeout_secs);
        let bytes = wasm_bytes.to_vec();
        let export = export_name.to_owned();
        let cases = cases.to_vec();
        let this = Arc::clone(self);
        let result = tokio::time::timeout(
            timeout,
            tokio::task::spawn_blocking(move || this.evaluate_i64_cases(&bytes, &export, &cases)),
        )
        .await;

        match result {
            Ok(Ok(inner)) => inner,
            Ok(Err(join_error)) => Err(anyhow::anyhow!(
                "sandbox evaluation task panicked: {join_error}"
            )),
            Err(_) => Err(anyhow::anyhow!(
                "sandbox evaluation timed out after {}s",
                self.config.timeout_secs
            )),
        }
    }
}

impl Drop for SandboxManager {
    fn drop(&mut self) {
        self._epoch_task.abort();
    }
}
