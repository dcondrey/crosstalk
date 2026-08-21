# BlindMind Rust Migration Plan

**Status:** Native deterministic core and direct CLI integration are implemented. Production orchestration and scientific-validation phases remain in progress.

## Outcome

BlindMind becomes Crosstalk's native evolutionary-discovery subsystem without importing Python, LiteLLM, or global database state into the trusted orchestration process. The Python project remains a temporary behavioral oracle and rapid research sandbox until the native engine passes equivalence and production gates.

This is a selective port, not a line-for-line rewrite. Crosstalk adopts the useful evolutionary mechanics while replacing provider, storage, concurrency, and verification boundaries with native Rust interfaces. The adjacent Python checkout is not a runtime dependency of the Crosstalk binary.

## Architectural boundary

`crosstalk-evolution` owns deterministic evolutionary policy. It does not know about model providers, terminal UI, network clients, or Crosstalk session storage. Callers implement `CandidateGenerator` and `CandidateEvaluator`; therefore tests can use deterministic fixtures while Crosstalk can use any `PromptAgent`.

The `crosstalk.blindmind.v1` schema is the compatibility boundary between Python experiments, the Rust engine, and future MCP integrations.

## Component map

| Component | Responsibility |
|---|---|
| `crosstalk-evolution/src/types.rs` | Concepts, fitness, lineage, configuration, reports, failures, and checkpoint schema |
| `crosstalk-evolution/src/selection.rs` | BlindMind-compatible score, title similarity, and Pareto-front selection |
| `crosstalk-evolution/src/engine.rs` | Deterministic operators, bounded concurrency, validation gates, rejection memory, and generations |
| `src/engines/idea_evolution.rs` | Versioned interchange contract and adapters from Crosstalk `PromptAgent`s |
| `src/main.rs` | CLI configuration, evidence excerpts, stage timeout, and session artifact ingestion |

Objective tool results cross this boundary through versioned `ObjectiveFeedback` records. They remain separate from model-authored scores in the checkpoint so downstream evidence cannot be confused with an LLM judgment.

The core crate deliberately cannot call a model provider or research API. Tests can therefore supply deterministic generators and critics without credentials or network access.

## Non-negotiable invariants

1. Identical checkpoint, seed, generator outputs, and evaluator outputs produce identical populations and lineage.
2. Every retained non-seed concept has an operator and complete parent lineage.
3. Scores are finite, bounded, and never trusted as proof.
4. Default retention rejects fatal flaws and requires a mechanism, prediction, kill criterion, and minimum safety.
5. Pareto selection preserves useful disagreement between novelty, feasibility, evidence, utility, safety, and prior-art risk.
6. Checkpoints are versioned and reject dangling population references.
7. Evolution can be cancelled and resumed at generation boundaries without corrupting Crosstalk state.
8. Python compatibility is measured explicitly; it does not constrain improved native policy forever.
9. Candidate ordering is deterministic even when model calls execute concurrently; completed calls are processed by attempt index.
10. Individual malformed outputs, generator failures, and critic failures are recorded without aborting an otherwise viable generation.

## Delivery phases

### Phase 1 — Native deterministic core (implemented)

- Workspace crate and public provider-independent traits.
- Concepts, mechanisms, measurements, kill criteria, fitness, and lineage.
- Deterministic operator and parent selection.
- Crossover, point-mutation, inversion, and wildcard dispatch.
- Rejection memory and Python-compatible title-overlap behavior.
- BlindMind-compatible critic score fixtures.
- Hard validation gates, Pareto frontier, adaptive directives, and JSON checkpoints.

### Phase 2 — Direct Crosstalk integration (implemented baseline)

- Native generator/evaluator adapters over `PromptAgent`.
- Validated `crosstalk.blindmind.v1` request and response.
- CLI controls for generations, population, seed, concurrency, and overall timeout.
- Evolution output becomes an auditable session artifact before the main deliberation.
- Research and local artifacts are passed as evidence identifiers plus bounded, injection-screened excerpts and full-content hashes.
- Separate variation and critic agents are selected when at least two models are available.
- Candidate frontier, complete checkpoint, and generation reports are attached to session state.

### Phase 3 — Production orchestration (in progress)

- **Implemented:** separate variation and critic agents when two or more models are selected.
- **Implemented:** deterministic bounded-parallel candidate calls and per-attempt failure isolation.
- **Implemented:** full checkpoint and generation reports attached to the signed session state.
- **Implemented:** an overall evolution timeout that fails open into normal deliberation.
- **Implemented:** bounded evidence excerpts with source IDs and content hashes supplied independently to variation and critic agents.
- **Implemented:** objective evaluator/reproduction results can raise evidence and feasibility or override optimistic fitness; hard-constraint failures remove active candidates.
- Expand from one variation/critic agent pair to independently routed model pools.
- Connect calls to the existing token budget and cancellation scope, not only a wall-clock timeout.
- Checkpoint after every accepted candidate, not only every generation.
- Persist lineage in Crosstalk's signed state and expose it in the TUI.
- Expand objective feedback across claims, citations, proofs, simulations, and experiment adapters.
- Add duplicate detection using embeddings and prior-art retrieval rather than title overlap alone.

## Runtime behavior

The stage runs only when `--evolve-generations` is greater than zero. It starts from a seed derived from the user task, then uses the first available agent for variation and the second for adversarial evaluation. With one model, the same agent fills both roles.

Calls within a generation are bounded by `--evolve-concurrency`. Completion order does not change retained-state ordering: results are processed by deterministic attempt index. Attempt failures are included in `GenerationReport`; they do not erase successful siblings. A stage-level timeout or evolution error is fail-open for availability, meaning Crosstalk logs the condition and proceeds to ordinary deliberation without evolved candidates.

Fail-open does not mean verified. An absent evolution artifact carries no positive capability signal, and orchestration must not present a timed-out generation as a successful discovery run.

## Compatibility contract

`crosstalk.blindmind.v1` requests include:

- project and directive;
- constraints;
- evidence IDs;
- optional evidence excerpts with SHA-256 digests;
- uniquely identified seed concepts; and
- population, generation, and concurrency bounds.

Responses include retained idea IDs, complete parent IDs, mutation type, domain, title, mechanism, predicted measurements, kill criteria, and bounded external scores. Checkpoints contain the full concept map and active-population references, while generation reports preserve accepted IDs, rejected IDs, Pareto frontier, failures, and synthesized next directives.

The Rust reader supplies defaults for fields added compatibly to version 1, such as `max_concurrency`, so earlier v1 request fixtures continue to deserialize. A schema-breaking change requires a new version rather than silently reinterpreting stored state.

### Phase 4 — Scientific and invention validation

- Domain evaluator plugins for mathematics, cryptanalysis, linguistics, history, physics, chemistry, biology, CS, and AI.
- Formal-proof gates for mathematical candidates.
- Reproducible code/simulation artifacts with fixed seeds and environment manifests.
- Preregistered expected measurements, baselines, and kill criteria.
- Patent and literature novelty searches with immutable source snapshots.
- Human review gates for physical experiments and high-impact claims.

### Phase 5 — Python retirement

- Run fixed Python/Rust comparison suites across several historical BlindMind databases.
- Export Python concepts and lineage through `crosstalk.blindmind.v1`.
- Verify record counts, ancestry, scoring fixtures, and selection behavior.
- Make Rust the default after two releases of opt-in use.
- Archive the Python engine only after migration tooling and rollback instructions are tested.

## Evaluation gates

Each release reports accepted concepts per model call, Pareto-front diversity, duplicate rate, fatal-flaw escape rate, evidence coverage, downstream validation rate, cost, latency, and checkpoint reproducibility. Native evolution is successful only when downstream falsification and validation improve—not merely when it generates more ideas.

The minimum comparison set is: single-model ideation, equal-call independent sampling, evolution without a separate critic, evolution without research context, and the complete pipeline. See [Evaluation Strategy](evaluation.md) for budget and reporting rules.

## Current CLI

```bash
crosstalk --task "Invent a testable alternative to transformer attention" \
  --arxiv "efficient alternatives to transformer attention" \
  --evolve-generations 3 \
  --evolve-population 8 \
  --evolve-concurrency 4 \
  --evolve-seed 42 \
  --evolve-timeout-secs 900 \
  --auto
```

This runs native evolution first and attaches the retained frontier, the complete resumable checkpoint/lineage, and generation reports as `evolution/native-candidates.json`, `evolution/checkpoint.json`, and `evolution/generation-reports.json`. Crosstalk then sends those candidates through its normal adversarial deliberation and verification pipeline.

## Known limitations

- The overall timeout cancels by dropping the stage future; evolution is not yet attached to the orchestrator's shared `CancelScope`.
- Model token and monetary costs are not yet returned through `PromptAgent`, so evolutionary budget enforcement is incomplete.
- Variation and critic selection is positional, not routed by measured domain competence.
- A checkpoint is emitted after the requested generations, not after every accepted candidate.
- Title-token overlap is a weak duplicate and prior-art detector.
- Model fitness is predictive metadata. The checkpoint accepts typed objective updates, but the main evolution CLI does not yet schedule domain evaluators automatically.
- The TUI does not yet expose an interactive lineage graph.
