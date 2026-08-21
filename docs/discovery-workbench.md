# Verifiable Discovery Workbench

## Purpose

Crosstalk separates creative reasoning from the machinery allowed to establish a result. Models can propose hypotheses, proofs, algorithms, mechanisms, and experiments. Typed evidence and objective evaluators determine what the application may call supported, reproduced, or formally verified.

This is an implemented foundation, not a claim of autonomous scientific discovery. A domain-specific evaluator is only as valid as its specification, test data, instrumentation, and independence assumptions.

## Investigation and chain of evidence

Every `ConversationState` owns a versioned `Investigation` with:

- a `ProblemSpec` for the question, objectives, constraints, success criteria, and forbidden failures;
- hypotheses with parent, claim, evidence, and verification links;
- content-addressed `EvidenceArtifact` records;
- `VerificationRecord` entries that commit the evaluator identity/version, specification, input, output, measurements, diagnostics, timestamps, and reproduction lineage; and
- a `ChainOfEvidenceAudit` over all references and promoted claims.

Ingested local artifacts and arXiv/Zenodo records are registered as evidence. Formal Lean, Coq, and Verus checks are persisted as verifier records with source/output hashes and the checker version when available.

The status rule is deliberately strict: attaching evidence does not change a claim's status. `apply_verification_to_claim` accepts only a `Verified` record, creates evidence committed to its output, and then promotes the subject claim. Formal status additionally requires a successful `FormalProof` record.

The audit detects:

- map-key/record-ID mismatches and malformed hashes;
- dangling claim, hypothesis, evidence, verification, and reproduction references;
- evidence digest mismatches;
- supported claims without evidence; and
- formally verified claims without an accepting formal checker record.

The audit validates structure and provenance. It cannot decide whether a source entails a claim or whether the formal statement faithfully represents the real-world question.

## Objective evaluator contract

`EvaluationSpec` preregisters named metrics, direction (`Minimize`, `Maximize`, or target), units, reproduction tolerances, hard constraints, timeouts, determinism, and whether independent reproduction is mandatory.

An `ObjectiveEvaluator` returns an `ObjectiveEvaluation` committed to:

```text
evaluator ID + evaluator version
specification SHA-256
candidate ID + content SHA-256
raw output SHA-256
measurements + hard-constraint results
environment + start/completion times
```

`EvaluatorRegistry` rejects unknown or duplicate evaluators, validates the returned identity and hashes, enforces the stage timeout, and optionally invokes a separately registered reproduction evaluator. The primary and reproduction evaluator IDs must differ. Reproduction succeeds only when both results verify and every metric agrees within its preregistered relative tolerance. Separate identities make provenance explicit; deployment policy must still ensure they represent genuinely independent workers, environments, or implementations when that distinction matters.

The initial `WasmExecutionEvaluator` runs WASI-compatible code through Crosstalk's Wasmtime sandbox. It reports elapsed time and fuel consumption and can enforce successful exit and resource-limit constraints. Successful execution is not algorithmic correctness.

`WasmI64FunctionEvaluator` is the first built-in correctness evaluator. It runs a candidate export with the signature `(i64) -> i64` over evaluator-owned cases. The module is compiled once, but every case is instantiated with a fresh store, memory, globals, and WASI context. A single fuel budget covers the full set. Public reports contain aggregate measurements, constraint results, the test-set commitment, and a digest of raw evaluator output, but never inputs, expected outputs, or actual outputs.

## Algorithm Discovery Lab

`AlgorithmDiscoveryLab` is a provider-independent tournament for executable candidates. A challenge commits:

- its complete evaluation specification;
- a baseline candidate;
- one primary metric and minimum improvement;
- at least one hard correctness/safety constraint;
- a maximum number of candidates; and
- the SHA-256 commitment of held-out test material.

Before any candidate is run, the lab validates the challenge, rejects duplicate IDs/content, requires both registered evaluator identities to expose the challenge's exact hidden-test commitment, and independently verifies the baseline. A digest that is merely present in challenge metadata is insufficient. Every candidate is evaluated twice. A candidate is eligible only if:

1. both runs are verified;
2. metric values agree within tolerance;
3. all hard constraints pass in both runs; and
4. improvement over the baseline reaches the preregistered threshold.

The winner is the largest eligible improvement, with deterministic ID tie-breaking. The report retains the baseline, every failure, every candidate outcome, committed hashes, reproduction mismatches, and typed verification records. Evaluator errors are recorded as rejected trials rather than silently dropped.

Minimal library flow:

```rust,no_run
use crosstalk::engines::algorithm_discovery::AlgorithmDiscoveryLab;
use crosstalk::engines::objective_evaluation::EvaluatorRegistry;

# async fn run(registry: EvaluatorRegistry,
# challenge: crosstalk::engines::algorithm_discovery::AlgorithmChallenge,
# candidates: Vec<crosstalk::engines::objective_evaluation::CandidateArtifact>) -> anyhow::Result<()> {
let report = AlgorithmDiscoveryLab::new(&registry)
    .run(&challenge, &candidates)
    .await?;
if let Some(winner) = report.winner() {
    println!("verified winner: {}", winner.candidate_id);
}
# Ok(())
# }
```

### Standalone hidden-test lab

Build `crosstalk-lab` with the workspace. A private test file has this strict schema:

```json
{
  "schema": "crosstalk.wasm-i64-tests.v1",
  "export_name": "solve",
  "cases": [
    { "input": -2, "expected": 4 },
    { "input": 9, "expected": 81 }
  ]
}
```

Compute the canonical commitment before publishing the challenge:

```bash
crosstalk-lab commitment --hidden-tests private-tests.json
```

Put that 64-character digest into a challenge file:

```json
{
  "schema": "crosstalk.algorithm-challenge-file.v1",
  "id": "square-efficiency-v1",
  "title": "Reduce metered work while preserving exact squares",
  "evaluator_id": "square-primary",
  "evaluation": {
    "id": "square-hidden-tests",
    "version": "1",
    "description": "Exact private cases plus metered efficiency",
    "metrics": [
      {
        "name": "accuracy",
        "unit": "ratio",
        "direction": "Maximize",
        "reproduction_tolerance": 0.0
      },
      {
        "name": "fuel_consumed",
        "unit": "fuel",
        "direction": "Minimize",
        "reproduction_tolerance": 0.0
      }
    ],
    "hard_constraints": [
      "all_cases_correct",
      "resource_limit_not_hit"
    ],
    "timeout_secs": 30,
    "deterministic": true,
    "independent_reproduction_required": true,
    "reproduction_evaluator_id": "square-reproduction"
  },
  "primary_metric": "fuel_consumed",
  "minimum_improvement": 1.0,
  "hidden_test_commitment_sha256": "REPLACE_WITH_COMMITMENT",
  "baseline_id": "published-baseline",
  "max_candidates": 32
}
```

Run one or more candidates:

```bash
crosstalk-lab run \
  --challenge challenge.json \
  --hidden-tests private-tests.json \
  --baseline baseline.wasm \
  --candidate first=first.wasm \
  --candidate second=second.wasm \
  --memory-bytes 268435456 \
  --fuel 100000000 \
  --timeout-secs 30 \
  --output report.json
```

The output path is create-only: the command will not replace an existing report. Omit `--output` for JSON on stdout.

### Hidden-test trust boundary

The commitment proves which cases an evaluator used and detects substitution after challenge publication. By itself, it does not prove that the cases were chosen before candidate authors saw the challenge, that they are representative, or that primary and reproduction workers are organizationally independent. Repeated adaptive access to aggregate scores can also overfit a private set. Serious competitions should timestamp or externally attest the commitment, cap submissions, separate test custodians from candidate authors, and reproduce winners on a second sealed set or implementation. The local CLI uses separate Wasmtime engines and evaluator identities; a clean-room remote reproduction service remains a stronger release gate.

## Objective feedback into evolution

The native BlindMind engine stores objective feedback separately from model-authored fitness, preserving the audit trail and checkpoint compatibility. `apply_objective_evaluation` and `apply_reproduction_outcome` update a serialized checkpoint.

- An accepting result raises evidence and feasibility, with the highest evidence level reserved for an independent reproduction.
- It never raises novelty: novelty still requires prior-art evidence.
- A rejection lowers feasibility/evidence and adds a fatal flaw.
- A failed hard constraint also removes the concept from the active population.
- Duplicate verification IDs and feedback for unknown concepts are rejected.

## Headless runs and bundles

Use `--headless` for CI, scripts, remote jobs, or a future graphical client:

```bash
crosstalk \
  --headless \
  --headless-format json \
  --bundle-dir ./runs/example \
  --task "State competing hypotheses and the next decisive test" \
  --auto
```

`--headless` requires `--task` and fails when no usable provider credential is configured. It does not fall into the interactive wizard. JSON or Markdown is written to stdout after the real orchestrator completes.

The bundle exporter writes:

| File | Contents |
|---|---|
| `manifest.json` | Bundle schema, state digest, transcript-chain head, audit, and hashes/sizes of exported files |
| `state.json` | Complete serialized conversation/investigation state committed by `state_sha256` |
| `investigation.json` | Problem, hypotheses, evidence, and verifier records |
| `claims.json` | Typed claim ledger |
| `audit.json` | Chain-of-evidence audit |
| `transcript.json` | Retained session turns |
| `report.md` | Human-readable problem, claims, verification, and audit summary |
| `artifacts/` | Content-addressed copies of session artifacts |

Bundle paths are sanitized; filesystem root, symlink targets, and non-empty output directories are rejected. Requiring an empty directory prevents stale files from a previous run being mistaken for current evidence. The manifest is a tamper-evident commitment, not an external timestamp or third-party attestation.

`InvestigationBundleExporter::verify` performs a non-executing replay of the integrity checks. It requires the exact declared file set, streams every file through SHA-256, validates sizes, deserializes `state.json`, recomputes its digest, verifies the retained transcript chain (including its truncation anchor), and recomputes the evidence audit. A party able to replace both files and the manifest can still create a new internally consistent bundle; publish or sign the manifest digest when an external authenticity guarantee is required.

The same verifier is available as a standalone, provider-free command:

```bash
crosstalk --verify-bundle ./runs/example
```

## Evaluation discipline

`crosstalk-eval` now fails closed by default. The UCB1 topology scenarios are simulations and run only with `--synthetic`; they are explicitly logged as non-benchmark results. Non-live SWE-bench uses a simulated repository and requires `--mock`. A real container run requires `--live-run` (or `--smoke-test`), a SWE-bench manifest, Docker images, and provider credentials unless simulated model output was also explicitly selected. This prevents a development fixture from being mistaken for a benchmark result.

The next priorities are additional typed function/data interfaces, domain evaluator packages, shared token/cost budgets across evolution and deliberation, sealed clean-room reproduction workers, and a visual investigation graph built on the headless contracts.
