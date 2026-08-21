# Domain-General Reasoning Architecture

## Mission and boundary

Crosstalk is an orchestration and verification system, not a foundation-model laboratory. Its path to frontier performance is to coordinate diverse models under explicit epistemic protocols, preserve disagreement, attach claims to evidence, invoke specialist tools, and measure downstream outcomes.

The project is designed for hard problems in mathematics, computer science, AI, cryptanalysis, linguistics, history, physics, chemistry, biology, engineering, policy, and software. That breadth is an architectural target, not a claim that every domain is already solved well. Capability claims must remain task-specific, budget-matched, and reproducible.

## Design principles

1. **Classify before prompting.** A proof, historical inference, cryptanalytic hypothesis, engineering design, and code patch require different evidence standards.
2. **Diversify before consensus.** Independent approaches are more valuable than several paraphrases of one premise.
3. **Falsify before polishing.** Counterexamples, contrary sources, boundary cases, and failure mechanisms receive a dedicated stage.
4. **Use the strongest available verifier.** Prefer a kernel, executable test, reproducible transformation, source audit, or experiment over model confidence.
5. **Preserve uncertainty.** Synthesis must retain contested claims, viable alternatives, and decisive missing evidence.
6. **Separate provenance from truth.** Signatures prove origin and integrity, not correctness.
7. **Reward validated novelty.** An unusual idea matters only if its mechanism and predictions survive increasingly expensive gates.

## System flow

```text
User task, workspace, constraints, repository records
                         |
                         v
              Domain classification
                         |
                         v
        Optional native evolutionary discovery
   variation -> critic -> hard gates -> Pareto frontier
                         |
                         v
     Multi-model proposal and adversarial deliberation
                         |
                         v
       Tagged claim ledger and unresolved cruxes
                         |
                         v
 Domain verification adapters and artifact validation
 proof checker | tests | sandbox | source audit | simulation
                         |
                         v
        Calibrated synthesis and next decisive test
                         |
                         v
       Signed turn, hash-chain head, checkpoint, audit
```

Native evolution currently runs before the main orchestrator and contributes candidates, a checkpoint, and generation reports as session artifacts. Moving it inside the shared budget/cancellation lifecycle is a production milestone.

## Deliberation loop

Every task receives a `DeliberationProtocol` containing roles, phases, evidence standards, and completion requirements:

1. **Frame** — define terms, objectives, assumptions, burdens of proof, and success metrics.
2. **Diversify** — produce independent approaches using domain roles and varied reasoning paths.
3. **Falsify** — seek counterexamples, contrary evidence, physical violations, confounders, and hidden assumptions.
4. **Verify** — select the appropriate checker, experiment, source audit, simulation, executable test, or uncertainty report.
5. **Synthesize** — merge compatible claims while preserving unresolved contradictions and cruxes.
6. **Design the next test** — state the cheapest observation most likely to change the conclusion.

The built-in mode library includes Debate, Theorem, and Invention modes in addition to general orchestration topologies. The domain protocol is applied independently of interaction topology so a debate topology cannot silently replace the evidence standard.

## Domain contracts

### Lawful cryptanalysis

- Preserve exact bytes, encoding, metadata, and transformations.
- Measure entropy, periodicity, structure, and leakage before choosing a story.
- Compare competing cipher/encoding hypotheses against null models.
- Require reproducible scripts and independent known-answer checks.
- Report ambiguity when several plaintexts or keys remain plausible.

This workflow is for public puzzles, research corpora, and material the user is authorized to analyze.

### Ancient-language decipherment

- Preserve sign readings, damage, direction, provenance, and segmentation uncertainty.
- Combine epigraphy, corpus statistics, historical linguistics, and archaeological context.
- Avoid assuming language ancestry or a preferred reading before evidence.
- Evaluate hypotheses on held-out inscriptions or segments.
- Retain competing decipherments where the corpus underdetermines a unique reading.

### Historical inquiry

- Separate primary records, later testimony, interpretation, and folklore.
- Build a provenance-aware timeline and identify missing records.
- Generate mutually exclusive hypotheses and their expected evidence.
- Audit source independence, incentives, anachronisms, and chain of custody.
- Never promote absence of evidence into proof without a justified expectation of preservation.

### Mathematics and theoretical problems

- Normalize definitions, quantifiers, axioms, and target statements.
- Search small cases, boundary cases, and counterexamples before proof search.
- Develop independent proof or impossibility strategies.
- Translate the best result into a complete proof-assistant artifact.
- Call it a theorem only when a trusted checker accepts the exact source.

### Physics, chemistry, and biology

- State units, scales, boundary conditions, conserved quantities, and known constraints.
- Propose competing causal mechanisms with quantitative predictions.
- Check dimensions, limiting cases, controls, confounders, uncertainty, and safety.
- Design a discriminating experiment or validated simulation.
- Keep derivation, simulation, observation, and independent replication as distinct evidence levels.

### Computer science, AI, and engineering invention

- Define objective metrics, constraints, interfaces, and forbidden failure modes.
- Generate orthogonal mechanisms, not only surface variations.
- State the technical delta relative to plausible prior art.
- Assess novelty, feasibility, evidence, safety, utility, cost, and scale separately.
- Specify a cheapest decisive prototype, predicted measurements, and kill criteria.
- Require executable benchmarks, red-team evaluation, and scaling analysis before high confidence.

### Debate and decisions

- Define the resolution, terms, burdens of proof, objectives, and constraints.
- Construct and steelman the strongest opposing cases.
- Expose pivotal premises and sensitivity to uncertain inputs.
- Judge claims individually rather than selecting a winner by majority vote.
- Report residual disagreement and the evidence that would reverse the recommendation.

## Claim and evidence model

Material model claims can be tagged as:

```text
[FACT]
[ASSUMPTION]
[INFERENCE]
[CONJECTURE]
[PROPOSAL]
```

Tagged, non-user lines enter `ClaimLedger` with a stable turn-derived ID, kind, initial status, confidence, and optional evidence references. The data model supports support, contradiction, dependency, and refinement edges, plus unresolved-crux and verification-coverage queries.

Current limitation: ingestion records explicit tags; it does not automatically determine whether a `[FACT]` is true, connect citations, or update every claim after downstream verification. Claim-to-evidence entailment and automatic status propagation are next-stage work.

## Verification layers

Different claims require different gates:

| Claim type | Preferred gate | What failure means |
|---|---|---|
| Mathematical theorem | Lean 4 or Coq kernel check | Remains unproved or rejected under the encoded statement |
| Rust safety invariant | Verus | Implementation invariant is not established |
| Software behavior | Tests, static checks, and bounded execution | Candidate does not meet the executable acceptance contract |
| Cryptanalytic solution | Independently reproducible transformation | Candidate plaintext/key is not established |
| Historical claim | Primary-source provenance and independent triangulation | Claim remains contested or under-supported |
| Scientific mechanism | Quantitative prediction, controls, experiment, replication | Mechanism remains a hypothesis |
| Engineering invention | Prior-art review, simulation, prototype, red team | Novelty, feasibility, safety, or utility remains speculative |

Formal proof verification is described in [Formal Verification](formal-verification.md). The crucial boundary is that a proof sketch, hash commitment, successful simulation, and model agreement are not formal proofs. The legacy `ProofAttachment` type records provenance claims only.

## Research architecture

`ResearchClient` provides bounded, typed access to arXiv and Zenodo. Records retain repository identity, canonical URL, authors, date, DOI when available, retrieval time, and a SHA-256 commitment to the source API response. Search results become JSON artifacts in the same session context as local evidence.

When native evolution is active, bounded and injection-screened artifact excerpts are supplied to both variation and critic agents with content hashes. Full-text retrieval, source-independent claim graphs, retraction checks, and archival snapshots are not yet implemented. See [Research and Evidence](research-and-evidence.md).

## Native evolutionary discovery

`crosstalk-evolution` ports BlindMind's algorithmic core into a provider-independent Rust crate. `CandidateGenerator` and `CandidateEvaluator` traits isolate evolutionary policy from model providers, UI, networking, and persistent session storage.

The engine implements:

- deterministic crossover, point mutation, inversion, and wildcard dispatch;
- deterministic operator and parent selection from checkpoint and seed;
- bounded-parallel candidate generation/evaluation with ordered processing;
- malformed-output and per-attempt failure isolation;
- rejection memory and duplicate title screening;
- hard mechanism, measurement, kill-criterion, safety, and fatal-flaw gates;
- multi-objective Pareto retention; and
- versioned checkpoints with validated lineage references.

The `crosstalk.blindmind.v1` JSON schema is the compatibility boundary for seed ideas, evidence excerpts, evolved mechanisms, scores, and lineage. Evolutionary fitness begins as an external model assessment. Typed objective evaluator and reproduction outcomes can now be applied to the native checkpoint: accepting results increase evidence/feasibility, while failures override optimistic scores and hard-constraint failures remove a candidate from the active population. See [BlindMind Rust Migration Plan](blindmind-rust-migration.md).

## Provenance and state

Each session maintains:

- versioned artifacts and diffs;
- a bounded turn history;
- a SHA-256 chain over retained turns;
- Ed25519 turn signatures and a pinned verifying identity;
- a typed claim ledger;
- budget, goal, consensus, and verification state;
- persistent Sled checkpoints; and
- an untagged COSE `Sign1` orchestration-audit statement committing the chain head.

On resume, Crosstalk checks signatures for signed turns and verifies the retained hash chain. These controls detect alteration; they do not validate the semantic correctness of the transcript.

## Implementation status

| Layer | Status | Main gaps |
|---|---|---|
| Domain classification and prompt contracts | Implemented | Learned classifier and multilingual/domain-specific evaluation |
| Debate/Theorem/Invention modes | Implemented | Domain competence routing and benchmark calibration |
| Typed claim ledger | Implemented baseline | Citation ingestion, entailment, automatic status propagation |
| Lean/Verus/Coq adapters | Implemented baseline | Hardened OS sandbox, dependency manifests, signed result artifacts |
| arXiv/Zenodo adapters | Implemented baseline | Full text, raw snapshots, patents, corrections/retractions |
| Native BlindMind core | Experimental | Shared token/cost budget and routed model pools; objective downstream fitness updates are implemented |
| Investigation/evidence ledger | Implemented | Automated semantic entailment and source-independence inference |
| Objective evaluator contracts | Implemented foundation | Domain-specific benchmark, simulation, instrument, and clean-room reproduction adapters |
| Algorithm Discovery Lab | Experimental CLI + API | More function signatures, domain evaluator packages, sealed remote holdouts, and visual UI |
| Headless runs and bundles | Implemented | External signature/attestation publication and bundle replay tooling |
| Signed session provenance | Implemented | External transparency-log anchoring and auditor tooling |
| Cross-domain evaluation | Designed | Executed public/private benchmark program and published results |

## Roadmap gates

### Gate 1 — Evidence-linked claims

- Parse explicit citations into `EvidenceRef` records.
- Build source-independence, supports, contradicts, and depends-on graphs.
- Update claim status only from named verifier outcomes.
- Surface unresolved cruxes and verification coverage in the TUI.

### Gate 2 — Hardened verification

- Persist signed proof/test/simulation results with tool versions and source hashes.
- Isolate proof assistants in a networkless operating-system sandbox.
- Add formalization audits and dependency/axiom policies.
- Introduce scientific simulation and experiment adapters with reproducible manifests.

### Gate 3 — Production evolutionary orchestration

- Move evolution into the orchestrator's shared cancellation and budget ledger.
- Route independent variation and critic pools by measured domain competence.
- Checkpoint every accepted candidate.
- Expand the implemented objective-feedback bridge across proof, experiment, simulation, citation, and prototype evaluators.
- Replace title-only duplication checks with embedding and prior-art retrieval.

### Gate 4 — Research depth

- Add content-addressed source snapshots and repository-approved full text.
- Add patents, datasets, historical corpora, epigraphic corpora, and domain ontologies.
- Track corrections, retractions, licenses, and source lineage.
- Defend the retrieval boundary against prompt injection and poisoned sources.

### Gate 5 — Measured frontier performance

- Extend `crosstalk-eval` across the capability matrix.
- Build contamination-resistant private holdouts.
- Compare equal-cost baselines and publish ablations, failures, and confidence intervals.
- Route models and protocols from measured competence rather than generic labels.

The full experimental standard is in [Evaluation Strategy](evaluation.md).

## What “superior” must mean

An assertion that Crosstalk is superior must specify the capability and conditions. A defensible example is:

> At a fixed dollar and latency budget, commit X achieved a higher checker-accepted Lean proof rate than baseline Y on holdout Z, across N preregistered seeds, with the published failure policy and confidence interval.

“Better than DeepMind” without a task, baseline, budget, and reproducible result is not an engineering metric and should not appear as a project claim.
