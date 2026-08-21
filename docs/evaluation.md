# Evaluation Strategy

## Objective

Crosstalk should be judged by reproducible performance on named capabilities, not by an unqualified claim of being generally superior to another laboratory or model. Every comparison must fix the task set, available tools, model versions, budget, latency policy, and success criteria.

The primary question is:

> Under the same declared resource budget, does Crosstalk produce more correct, calibrated, independently verifiable outcomes than strong single-model and orchestration baselines?

## Capability matrix

| Capability | Example evaluation source | Primary metric | Hard success gate |
|---|---|---|---|
| Formal mathematics | miniF2F, ProofNet, private Lean holdouts | checker-accepted proof rate | No placeholders; pinned toolchain accepts exact source |
| Scientific reasoning | held-out expert questions and preregistered prediction tasks | accuracy, Brier score, calibration error | Predictions and confidence recorded before outcome access |
| Debate | blind expert adjudication and claim audits | judge accuracy, crux recall, calibration | Strongest counterargument and unresolved cruxes retained |
| Invention | blinded expert review followed by prototype stages | validated mechanisms per dollar/model call | Novelty search plus quantitative prototype or simulation gate |
| Cryptanalysis | public/authorized challenge corpora with hidden answers | exact recovery and reproducibility | Independent script reproduces transformation from original bytes |
| Decipherment | held-out inscriptions or segments | predictive accuracy and ambiguity calibration | Reading generalizes beyond fitted corpus |
| Historical inquiry | source packets with blinded adjudication | claim accuracy, provenance coverage, alternative-hypothesis recall | Material claims trace to independent source classes |
| Planning | simulated and real long-horizon workflows | completion under time/token budget | Goal completed without policy violation |
| Software | SWE-bench-family and private repository tasks | verified resolution rate | Tests pass in clean environment; regression risk reviewed |

Benchmark names are starting points, not endorsements. Public suites require contamination analysis and should be paired with private, versioned holdouts.

## Required baselines

Every evaluation should compare at least:

1. the strongest individual model used by the run;
2. equal-budget independent sampling with simple selection;
3. Crosstalk without adversarial critique;
4. Crosstalk without research retrieval;
5. Crosstalk without native evolution;
6. Crosstalk without downstream verification; and
7. the full configured pipeline.

These ablations determine whether added machinery improves outcomes rather than only adding tokens and latency.

## Budget accounting

Report all resources that can change the comparison:

- input and output tokens by model and phase;
- model/API cost at the recorded pricing date;
- number of model calls and retries;
- wall-clock and model latency;
- proof-checker, simulation, test, and retrieval time;
- CPU, memory, accelerator, and storage use where material;
- human review time; and
- failed, timed-out, filtered, or policy-rejected attempts.

Timeouts and missing tools must have a predetermined scoring policy. Excluding failures after seeing results invalidates the comparison.

## Core metrics

Do not collapse performance into one score until the component metrics are visible:

- task success rate with confidence intervals;
- calibration and Brier score;
- formal or executable verification coverage;
- claim-level evidence coverage;
- unresolved-crux recall;
- diversity of independent approaches;
- fatal-flaw escape rate;
- duplicate and prior-art-overlap rate;
- cost and latency per successful outcome;
- reproducibility from checkpoints and seeds; and
- safety or policy violation rate.

For evolutionary discovery, generation count and idea volume are diagnostic metrics, not success metrics. The meaningful endpoint is downstream-validated novelty and utility.

## Experimental protocol

1. Freeze the benchmark manifest, prompts, model/provider identifiers, toolchain, and budget.
2. Register primary metrics, failure policy, and statistical analysis before running the test set.
3. Separate development tasks from final holdouts; record every exposure.
4. Randomize run order and blind human judges to system identity where possible.
5. Use multiple seeds for stochastic systems and preserve checkpoints.
6. Score outputs with executable or formal checks before subjective review.
7. Report every run, including refusals, malformed outputs, timeouts, and infrastructure failures.
8. Publish aggregate results, confidence intervals, and representative failures without leaking private holdouts.

## Reproducibility manifest

Each published run should include a machine-readable manifest with:

```text
evaluation ID and git commit
task-set name, version, and content hash
model provider, exact model ID, and reported model version
system and task prompts or prompt hashes
tool names and versions
reasoning mode and topology
random seeds
token, call, cost, and time budgets
retrieval queries and timestamps
proof/test/simulation environment digest
success, timeout, and failure policy
raw result artifact hashes
```

Secrets and private benchmark content may be replaced by commitments, but an independent auditor must be able to validate them under controlled access.

## Release gates

A capability moves from experimental to supported only when:

- its success condition is executable or independently adjudicated;
- the evaluation includes a strong equal-budget baseline;
- the result repeats across multiple seeds and task subsets;
- failures and negative results are included;
- calibration is measured when confidence is reported;
- the relevant trust boundary is documented; and
- a regression suite protects the measured behavior.

Claims of superiority must be capability-scoped: for example, “higher Lean proof completion on suite X at budget Y,” never simply “better than organization Z.”

## Near-term implementation plan

`crosstalk-eval` now fails closed when no mode is selected. Its UCB1 topology scenarios are simulations and require `--synthetic`; they are not labeled as GSM8K accuracy. Non-live SWE-bench uses a simulated repository and requires `--mock`. Real repository evaluation requires `--live-run` or `--smoke-test`, a SWE-bench manifest and Docker images, and provider credentials unless simulated model output is explicitly selected. These controls prevent a development run from being mislabeled as a benchmark.

1. Extend `crosstalk-eval` from topology selection to the capability matrix above.
2. Add a stable JSON evaluation manifest and result schema.
3. Connect token/cost telemetry to native evolution and orchestration phases.
4. Build private, contamination-resistant holdouts for each target domain.
5. Add claim-ledger coverage and contradiction metrics.
6. Publish baseline, ablation, and full-pipeline reports from CI-compatible runners.
