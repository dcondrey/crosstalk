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

Calls within a generation are bounded by `--evolve-concurrency`. A finite call-slot budget is apportioned across the remaining requested generations, with unused slots carried forward, so an all-rejected early generation cannot starve every later generation. Completion order does not change retained-state ordering: results are processed by deterministic attempt index. Attempt failures are included in `GenerationReport`; when a draft exists, its title, structural text, and failure reason also enter bounded rejection memory so later generations can avoid it. A stage-level timeout or non-budget evolution error is fail-open for availability, meaning Crosstalk logs the condition and proceeds to ordinary deliberation without evolved candidates.

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

The archive path added three such fields: `generation` and `tags` on `EvolvedIdea`, and `directive` on `EvolutionResponse`. All three are `#[serde(default)]`, so responses written before them still deserialize. `predicted_measurements` and `kill_criteria` also became defaulted, so an exporter whose source has no such column omits the keys entirely rather than emitting an empty list a reader could mistake for a measurement that found nothing.

### Phase 4 — Scientific and invention validation

- Domain evaluator plugins for mathematics, cryptanalysis, linguistics, history, physics, chemistry, biology, CS, and AI.
- Formal-proof gates for mathematical candidates.
- Reproducible code/simulation artifacts with fixed seeds and environment manifests.
- Preregistered expected measurements, baselines, and kill criteria.
- Patent and literature novelty searches with immutable source snapshots.
- Human review gates for physical experiments and high-impact claims.

### Phase 5 — Python retirement

- **Implemented:** `blindmind export-v1 <file> -p <project>` emits the `crosstalk.blindmind.v1` shape, one file per project because a checkpoint holds a single project.
- **Implemented:** `crosstalk --import-blindmind <file>` reconstructs concepts and lineage into `EvolutionState` through the existing serde types, prints a `BlindmindImportSummary`, and writes the checkpoint with `--import-blindmind-out`. Dangling parent references are rejected per invariant 6.
- Run fixed Python/Rust comparison suites across several historical BlindMind databases.
- Verify scoring fixtures and selection behavior. Record counts and ancestry are verified below; scoring is not, and cannot be from this data (see the fitness gap).
- Make Rust the default after two releases of opt-in use.
- Archive the Python engine only after migration tooling and rollback instructions are tested.

#### What the export actually recovered

Measured 2026-08-28 against `/Volumes/A/blindmind/data/blindmind.db` (194 concepts, 104 lineage rows). Exporter counts on the left, Rust reader counts on the right; every project was accepted.

| project | concepts in | exported | restored | unexportable | valid edges | exported | reader edges | accepted |
|---|---|---|---|---|---|---|---|---|
| essays | 86 | 86 | 86 | 0 | 8 | 8 | 8 | yes |
| conscious | 29 | 29 | 29 | 0 | 26 | 21 | 21 | yes |
| riemann | 27 | 27 | 27 | 0 | 20 | 20 | 20 | yes |
| embodied | 25 | 25 | 25 | 0 | 25 | 25 | 25 | yes |
| asds | 14 | 14 | 14 | 0 | 14 | 14 | 14 | yes |
| erdos | 9 | 9 | 9 | 0 | 5 | 5 | 5 | yes |
| default | 4 | 4 | 4 | 0 | 0 | 0 | 0 | yes |
| **total** | **194** | **194** | **194** | **0** | **98** | **93** | **93** | **7/7** |

Nothing in the source is unexportable. Every v1 required field has a BlindMind source: `Concept.description` is carried as `mechanism`, which is a rename of the same field rather than a reconstruction, since `MutationOutput.description` is specified as "the concept and its mechanics". No title exceeds the 512-byte cap and the longest description is 18,337 bytes, well inside the 32,000-byte mechanism cap.

Lineage reconciles against the 104 stored rows as 93 exported + 5 truncated + 4 with an endpoint outside the owning project + 2 whose endpoints are both absent from `concept` and which therefore appear in no project's export.

#### What the export does not recover

- **Fitness.** BlindMind records one composite scalar; `crosstalk.evolution.v1` has no slot for a scalar and requires seven axes. Every axis on an imported concept is therefore `0.0` and `next_directive` is empty, meaning *not measured*. Spreading one number across seven axes would assert seven judgements the critic never made, and `prior_art_overlap` runs in the opposite sense from the rest. The scalar survives only in the archive's `external_scores.blindmind_composite`. It was present on 69 of 194 concepts: essays 6/86, conscious 17/29, riemann 15/27, embodied 16/25, asds 10/14, erdos 5/9, default 0/4. The 125 without one are exactly the generation-0 seeds, which BlindMind never scored. **An imported checkpoint is an archive, not a resumable population**: with all-zero fitness it would select nothing.
- **Predicted measurements and kill criteria.** Zero of 194. BlindMind has no column for either, so the exporter omits the keys rather than writing `[]`.
- **Executable contracts and falsification probes.** None exist in Python, so imported configurations leave `require_executable_contract` and `require_falsification_probe` off.
- **Five lineage edges**, all in `conscious`, whose recorded parent is not from a strictly earlier generation. `crosstalk.blindmind.v1` imposes no generation ordering, but a checkpoint lineage edge does, so the edge is dropped and the child is exported with truncated ancestry rather than being discarded. Eleven concepts across five projects (essays 1, conscious 4, riemann 3, embodied 2, erdos 1) reach the reader as non-seeds with no parents, five of them from this truncation and six because the source recorded no lineage row at all.
- **The evolution directive** for four of seven projects. `EvolutionRun.latest_directive` exists only for `essays`, `riemann`, and `default`; the rest export an empty directive rather than an invented one.

Phase 5 is not complete. The comparison suites have not run, Rust is not the default, and the two opt-in releases have not happened. The Python checkout stays in place.

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
  --evolve-constraint "the result must satisfy the exact recurrence" \
  --evolve-exclusion "parity cancellation;linear surviving support" \
  --evolve-seed 42 \
  --evolve-timeout-secs 900 \
  --auto
```

This runs native evolution first and attaches the retained frontier, the complete resumable checkpoint/lineage, and generation reports as `evolution/native-candidates.json`, `evolution/checkpoint.json`, and `evolution/generation-reports.json`. Generated candidates must include an exact relation, composition rule, complexity argument, deterministic objective test, and an explicit distinction from every supplied exclusion. Structural fingerprints, rather than titles alone, suppress cosmetic variants. Crosstalk then sends the surviving proposals through its normal adversarial deliberation and verification pipeline.

## Known limitations

- The overall timeout cancels by dropping the stage future; evolution is not yet attached to the orchestrator's shared `CancelScope`.
- Model token and monetary costs are not yet returned through `PromptAgent`, so evolutionary budget enforcement is incomplete.
- Startup endpoint-validation and fallback pings occur before the shared session budget is initialized; `--max-model-calls` therefore does not yet cap those preflight calls.
- Variation and critic selection is positional, not routed by measured domain competence.
- A checkpoint is emitted after the requested generations, not after every accepted candidate.
- Structural token overlap is still only a deterministic duplicate heuristic, not a semantic equivalence proof or prior-art search.
- Model fitness is predictive metadata. The checkpoint accepts typed objective updates, but the main evolution CLI does not yet schedule domain evaluators automatically.
- The TUI does not yet expose an interactive lineage graph.
