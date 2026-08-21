# Crosstalk

[![CI](https://github.com/dcondrey/crosstalk/actions/workflows/ci.yml/badge.svg)](https://github.com/dcondrey/crosstalk/actions/workflows/ci.yml)
[![Rust 1.91+](https://img.shields.io/badge/rust-1.91%2B-orange)](https://www.rust-lang.org)
[![License: Apache-2.0](https://img.shields.io/badge/license-Apache--2.0-blue.svg)](LICENSE)
[![ORCID](https://img.shields.io/badge/ORCID-0009--0003--1849--2963-green.svg)](https://orcid.org/0009-0003-1849-2963)

**A domain-general, multi-model reasoning system for debate, proof, research, invention, and verified implementation.**

Crosstalk coordinates several language models under an explicit reasoning protocol. It asks them to develop independent approaches, expose assumptions, attack each other's pivotal claims, use domain-appropriate evidence, and synthesize a result that preserves uncertainty instead of hiding it behind consensus.

The project began as a coding orchestrator. It is evolving into a general discovery system for theoretical mathematics, lawful cryptanalysis, language decipherment, historical inquiry, natural science, engineering, computer science, and AI research.

> **Project status:** Crosstalk is experimental and pre-1.0. It can organize research and run real verification tools, but model agreement is not truth, an evolutionary score is not evidence, and a plausible proposal is not a discovery. See [Trust boundaries](#trust-boundaries).

## Why Crosstalk

A single model can be articulate and wrong in one pass. Crosstalk makes the reasoning process harder to fool by separating it into explicit stages:

```text
task + constraints + evidence
            |
            v
  classify the reasoning domain
            |
            v
 independent proposals and hypotheses
            |
            v
 adversarial critique and falsification
            |
            v
 proof / test / source / simulation checks
            |
            v
 calibrated synthesis + unresolved cruxes
            |
            v
 signed checkpoint and next decisive test
```

The objective is not to win by model vote. It is to produce conclusions that are easier to inspect, reproduce, falsify, and improve.

## Capability status

| Capability | Status | What exists today |
|---|---|---|
| Domain-general deliberation | Implemented | Protocols for cryptanalysis, decipherment, history, natural science, debate, theory, invention, empirical inquiry, decisions, software, and general analysis |
| Debate and claim discipline | Implemented baseline | Role-based debate plus tagged facts, assumptions, inferences, conjectures, and proposals in a typed claim ledger |
| Investigation and evidence audit | Implemented | Typed problems, hypotheses, content-addressed evidence, measurements, verifier records, claim links, dangling-reference checks, and verification-coverage reports |
| Formal proof checking | Implemented | Lean 4, Verus, and Coq fenced artifacts are sent to installed checkers; placeholders, missing checkers, failures, and timeouts never count as verified |
| Research retrieval | Implemented baseline | Typed arXiv and Zenodo search with canonical identifiers, dates, DOI metadata, retrieval time, bounded responses, and response hashes |
| Native idea evolution | Experimental | Rust-native BlindMind crossover, point mutation, inversion, wildcard generation, adversarial scoring, Pareto retention, lineage, and resumable checkpoints |
| Objective evaluation | Implemented foundation | Versioned evaluator specifications commit inputs/outputs, metrics, constraints, environment, timeouts, and independent reproduction outcomes; WASM execution is the first backend |
| Algorithm Discovery Lab | Experimental CLI + API | Private-case WASM tournaments cryptographically bind both evaluators to one test set, isolate every case, enforce correctness/resource gates, and require agreeing reruns |
| Reproducible headless runs | Implemented | JSON/Markdown stdout plus portable investigation bundles containing reports, audits, transcript state, and content hashes |
| Software verification | Implemented | Workspace-scoped tools, code/artifact validation, WASM execution limits, and optional accepted-edit application |
| Provenance | Implemented | Ed25519-signed turns, transcript hash chains, persistent checkpoints, and COSE/SCITT orchestration-audit statements |
| Frontier evaluation | In progress | Evaluation design exists; broad public, contamination-resistant benchmark results do not yet exist |
| Autonomous scientific discovery | Not claimed | Physical experiments, expert review, independent replication, and validated novelty remain external gates |

The detailed implementation map and roadmap live in [Domain-General Reasoning Architecture](docs/general-intelligence.md).

## Install

Requirements:

- Rust 1.91 or newer
- an API key for at least one supported model provider
- optional `lean`, `verus`, or `coqc` executables on `PATH` for formal checking

```bash
git clone https://github.com/dcondrey/crosstalk.git
cd crosstalk
cargo build --release
./target/release/crosstalk --help
```

Or install directly from GitHub:

```bash
cargo install --git https://github.com/dcondrey/crosstalk.git
```

## Configure providers

Copy [`.env.example`](.env.example) to `.env`, then set at least one key:

| Provider route | Environment variable |
|---|---|
| Anthropic | `ANTHROPIC_API_KEY` |
| OpenAI | `OPENAI_API_KEY` |
| DeepSeek | `DEEPSEEK_API_KEY` |
| Mistral | `MISTRAL_API_KEY` |
| Groq | `GROQ_API_KEY` |
| OpenRouter | `OPENROUTER_API_KEY` |
| OpenRouter-compatible fallback | `LLM_API_KEY` |

Bare model IDs are routed to a recognized native provider. IDs containing `/`, or IDs prefixed with `openrouter:`, use OpenRouter. `--auto` selects from the providers for which credentials are available.

## Quick start

Let Crosstalk select the available models:

```bash
crosstalk --task "Compare the strongest arguments for and against this hypothesis" --auto
```

Run a theory task and require a proof-assistant-ready result:

```bash
crosstalk --task "Formalize this conjecture, search for counterexamples, and emit a complete Lean 4 proof if it is true" --auto
```

Search research repositories, evolve candidate mechanisms, and then deliberate:

```bash
crosstalk \
  --task "Invent a measurable alternative to quadratic transformer attention" \
  --arxiv "efficient alternatives transformer attention" \
  --zenodo "attention benchmark dataset" \
  --research-limit 12 \
  --evolve-generations 3 \
  --evolve-population 8 \
  --evolve-concurrency 4 \
  --evolve-seed 42 \
  --auto
```

Investigate a historical question with local primary-source transcriptions:

```bash
crosstalk \
  --task "Construct competing explanations, audit source independence, and identify the decisive missing evidence" \
  --workspace ./corpus \
  --files 'sources/**/*.md' 'metadata/**/*.json' \
  --auto
```

Apply an accepted software change to a checkout:

```bash
crosstalk \
  --task "Diagnose and fix the parser regression; add a focused test" \
  --workspace . \
  --files 'src/**/*.rs' 'tests/**/*.rs' \
  --edit \
  --auto
```

`--edit` authorizes Crosstalk to write accepted changes inside the selected workspace. Omit it for analysis-only use.

Run in automation and export an auditable result bundle:

```bash
crosstalk \
  --headless \
  --headless-format json \
  --bundle-dir ./runs/attention-experiment \
  --task "Compare these mechanisms and state the next decisive experiment" \
  --workspace ./evidence \
  --files '**/*.md' '**/*.json' \
  --auto
```

The command writes the final response to stdout. The bundle contains `manifest.json`, the complete `state.json`, `investigation.json`, `claims.json`, `audit.json`, `report.md`, the transcript, and content-addressed copies of session artifacts. The library verifier recomputes the file hashes, state digest, transcript chain, and evidence audit without executing artifacts.

Verify a bundle offline without configuring or contacting a model provider:

```bash
crosstalk --verify-bundle ./runs/attention-experiment
```

## Verifiable discovery workbench

Crosstalk now has a typed layer beneath the model conversation:

```text
ProblemSpec -> Hypotheses -> Evidence -> VerificationRecords -> Evidence audit
                                  ^              |
                                  |              v
                        content hashes     claim promotion
```

An evidence link never promotes a claim by itself. A claim becomes supported or formally verified only through an accepting typed verifier record. The audit reports broken references, digest mismatches, unsupported status promotion, missing formal-proof records, and total verification coverage.

`ObjectiveEvaluator` is the extension point for tests, simulations, proof tools, scientific instruments, and benchmarks. `AlgorithmDiscoveryLab` uses separately registered primary and reproduction evaluators to compare a baseline with proposed candidates under a preregistered primary metric. Candidates cannot win unless hard constraints pass, the minimum improvement is met, and the reproduction agrees within the declared tolerance.

`crosstalk-lab` is the first runnable correctness evaluator. It tests WASM functions with the signature `(i64) -> i64` against private cases, creates a fresh instance for every case, shares a bounded fuel budget across the test set, and emits only aggregates and commitments. Both evaluator identities must expose the exact SHA-256 commitment named by the challenge.

```bash
crosstalk-lab commitment --hidden-tests private-tests.json
crosstalk-lab run \
  --challenge challenge.json \
  --hidden-tests private-tests.json \
  --baseline baseline.wasm \
  --candidate optimized=optimized.wasm \
  --output report.json
```

See [Verifiable Discovery Workbench](docs/discovery-workbench.md) for the data contracts, Rust API, and remaining trust boundaries.

## Domain protocols

Crosstalk classifies each task before prompting the model swarm. The protocol determines roles, phases, evidence standards, and the completion contract.

| Domain | Required discipline |
|---|---|
| Cryptanalysis | Preserve exact inputs; compare cipher hypotheses to null models; require reproducible transformations |
| Decipherment | Preserve damaged signs and segmentation uncertainty; predict held-out text; triangulate linguistic and archaeological evidence |
| Historical inquiry | Separate primary evidence, later testimony, interpretation, and folklore; audit provenance and source independence |
| Mathematics and theory | Normalize definitions and quantifiers; search counterexamples; distinguish conjecture from kernel-checked theorem |
| Natural science | State units and boundary conditions; derive quantitative predictions; design controls and discriminating experiments |
| Invention | Specify a mechanism, technical delta, measurements, feasibility, safety, cost, scale, prototype, and kill criteria |
| Debate and decisions | Make burdens of proof and cruxes explicit; steelman opposing cases; report sensitivity and residual disagreement |
| Software | Reproduce, design, implement, test, verify, and keep edits scoped |

Models are instructed to prefix material claims with `[FACT]`, `[ASSUMPTION]`, `[INFERENCE]`, `[CONJECTURE]`, or `[PROPOSAL]`. Tagged claims enter the session's typed claim ledger. The current baseline records these categories; automatic citation-entailment checking and claim-status propagation remain roadmap work.

## Formal verification

When a synthesized artifact is tagged `lean`, `lean4`, `coq`, or `verus` (or Rust contains `verus!`), Crosstalk invokes the matching installed checker directly:

| Artifact | Executable | Success condition |
|---|---|---|
| Lean 4 | `lean` | Checker exits successfully and source passes placeholder policy |
| Verus | `verus` | Checker exits successfully and source passes escape-hatch policy |
| Coq | `coqc` | Checker exits successfully and source passes placeholder policy |

The verifier uses a private temporary directory, clears the child environment except for `PATH` and `HOME`, limits source and diagnostic sizes, applies a wall-clock timeout, and kills the process if the future is dropped. It does **not** treat proof prose, hashes, model agreement, or successful simulations as proof.

External proof checkers are native processes, not a complete operating-system sandbox. Review checker versions and generated imports before using this with sensitive data. See [Formal Verification](docs/formal-verification.md) for statuses, policy, installation, and the exact trust boundary.

## Research and evidence

`--arxiv` and `--zenodo` fetch bounded search results before deliberation. Records are attached as `research/arxiv.json` and `research/zenodo.json`. A repository failure is isolated so the remaining sources and offline deliberation can continue.

Research metadata helps locate evidence; it does not establish that a claim is true or that a paper supports a model's interpretation. Crosstalk currently stores a SHA-256 hash of the repository response in each parsed record, not a complete archival snapshot of the paper. See [Research and Evidence](docs/research-and-evidence.md).

## Native BlindMind evolution

The `crosstalk-evolution` workspace crate ports BlindMind's algorithmic core to Rust without importing its Python CLI, LiteLLM integration, or global ORM state. It provides:

- deterministic operator and parent selection from a seed;
- crossover, point mutation, inversion, and wildcard generation;
- independent variation and critic agents when at least two models are available;
- bounded parallel evaluation with per-attempt failure isolation;
- hard validation gates, rejection memory, and Pareto-front retention;
- complete lineage, generation reports, and versioned JSON checkpoints.

An evolution run adds these artifacts to session state:

```text
evolution/native-candidates.json
evolution/checkpoint.json
evolution/generation-reports.json
```

Evolution scores are model judgments, not proof of novelty or feasibility. The retained frontier must pass downstream prior-art, proof, simulation, prototype, or experiment gates. Objective results can now be committed back into an evolution checkpoint: reproduced passes raise evidence and feasibility without altering novelty; failures lower both, create a fatal flaw, and remove candidates that violate hard constraints. See the [BlindMind Rust Migration Plan](docs/blindmind-rust-migration.md).

## Trust boundaries

Crosstalk provides process integrity and verification adapters; it does not turn uncertain inputs into certain conclusions.

- **Model consensus is not ground truth.** Agreement can expose or conceal shared failure modes.
- **A claim tag is metadata.** `[FACT]` records the model's claim type; it does not validate the fact.
- **Search metadata is not evidence entailment.** A paper title, abstract, or DOI does not prove that it supports a conclusion.
- **Only checker acceptance is formal verification.** Missing checkers, timeouts, rejected proofs, and forbidden placeholders are non-success states.
- **Formal proof is conditional.** It establishes the encoded theorem under its definitions and axioms, not that the formalization matches the physical world.
- **Simulation is not empirical confirmation.** Scientific and engineering claims still need measurements, controls, replication, and safety review.
- **Provenance is not correctness.** Signatures and hash chains make alteration detectable; they do not make signed content true.
- **Novelty requires prior-art review.** Evolutionary distance and model novelty scores are not patent or literature clearance.

## CLI reference

| Flag | Meaning |
|---|---|
| `-t, --task <TASK>` | Task or research question; omitted launches the interactive wizard |
| `-m, --models <IDS>...` | Explicit model IDs |
| `-A, --auto` | Select models from available credentials |
| `-i, --iterations <N>` | Maximum orchestration rounds; `0` means until convergence |
| `-w, --workspace <DIR>` | Root for context and optional edits |
| `-f, --files <GLOBS>...` | Context paths or workspace-relative glob patterns |
| `-e, --edit` | Write accepted changes into the workspace |
| `--resume <SESSION_ID>` | Restore the latest checkpoint and verify retained transcript integrity |
| `--agent-timeout-secs <N>` | Wall-clock limit for each orchestration turn; default `300` |
| `--arxiv <QUERY>` | Attach arXiv search records |
| `--zenodo <QUERY>` | Attach Zenodo search records |
| `--research-limit <N>` | Results per repository; default `10`, hard cap `25` |
| `--evolve-generations <N>` | Native evolution generations; default `0` (disabled) |
| `--evolve-population <N>` | Requested candidates per generation; default `8` |
| `--evolve-seed <N>` | Deterministic evolution seed; default `1` |
| `--evolve-concurrency <N>` | Maximum concurrent candidate evaluations; default `4` |
| `--evolve-timeout-secs <N>` | Overall evolution-stage timeout; default `900` |
| `--headless` | Disable the terminal UI and write the final structured result to stdout; requires `--task` |
| `--headless-format <FORMAT>` | Headless output as `json` or `markdown`; default `json` |
| `--bundle-dir <DIR>` | Export a reproducible investigation bundle after the run |
| `--verify-bundle <DIR>` | Verify a bundle's exact file set, hashes, state, transcript chain, and evidence audit, then exit |
| `--generate-completions <SHELL>...` | Emit completions for bash, zsh, fish, PowerShell, or Elvish |

Run `crosstalk --help` for the generated reference.

## State, audit, and privacy

By default, Crosstalk writes session checkpoints beneath `$XDG_DATA_HOME/crosstalk/<session-id>` (or `~/.local/share/crosstalk/<session-id>`) and rotating logs beneath `$XDG_STATE_HOME/crosstalk` (or `~/.local/state/crosstalk`).

Each retained turn is linked into a SHA-256 transcript chain, and orchestrated model turns are Ed25519-signed. The session also emits an untagged COSE `Sign1` orchestration-audit statement over the chain head, session ID, turn count, and timestamp. Resuming a session checks signatures present on retained turns, the pinned signing identity, and the transcript chain.

When `--bundle-dir` is set, Crosstalk also exports a portable content-addressed bundle. Its manifest commits the serialized session state, transcript chain head, evidence audit, and every exported file. This makes later alteration detectable; it does not prove that the original evidence or conclusions were correct.

Set `CROSSTALK_SIGNING_PASSPHRASE` to encrypt the persisted signing seed and `CROSSTALK_EXPECTED_PUBKEY` to pin the expected identity. Model prompts and attached evidence are sent to the selected providers; do not attach secrets unless those providers and your data-handling policy permit it.

## Repository layout

```text
src/core/                 orchestration, agents, synthesis, lifecycle, verification
src/engines/              deliberation, research, proof, evaluation, discovery, bundles
src/types/                conversations, artifacts, epistemics, modes, governance
src/mcp/                  permissioned model-tool gateway
src/ui/                   terminal interface and model selection
crosstalk-evolution/      native evolutionary-discovery engine
crosstalk-concurrency/    cancellation and concurrency primitives
crosstalk-eval/           fail-closed simulation and live SWE-bench harness
proofs/ and verus/        Verus specifications and proof support
docs/                     architecture, trust boundaries, plans, and operations
```

Each orchestration turn follows `propose -> observe -> score -> adapt -> synthesize -> verify -> checkpoint`.

## Documentation

- [Documentation index](docs/README.md)
- [Domain-General Reasoning Architecture](docs/general-intelligence.md)
- [Verifiable Discovery Workbench](docs/discovery-workbench.md)
- [Formal Verification](docs/formal-verification.md)
- [Research and Evidence](docs/research-and-evidence.md)
- [Evaluation Strategy](docs/evaluation.md)
- [BlindMind Rust Migration Plan](docs/blindmind-rust-migration.md)
- [Security Policy](SECURITY.md)
- [Contributing](CONTRIBUTING.md)
- [Verus project proofs](VERUS.md)

## Develop

```bash
cargo fmt --all -- --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace --no-fail-fast
```

Contributions should add tests for new policy or verification behavior and avoid capability claims that exceed measured evidence. See [CONTRIBUTING.md](CONTRIBUTING.md).

## Evaluation goal

Crosstalk's ambition is frontier performance on clearly defined tasks—not an unqualified claim of general superiority. Comparisons must name the task set, model versions, tool versions, budgets, prompts, seeds, contamination controls, failure policy, and confidence intervals. The project will claim an advantage only where reproducible evaluation supports it.

## License

Apache-2.0. See [LICENSE](LICENSE).
