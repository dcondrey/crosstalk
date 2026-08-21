# Formal Verification

## Purpose

Crosstalk uses external proof assistants to turn a subset of model output into machine-checkable artifacts. The verifier answers one narrow question: did a named checker accept this exact source under the configured policy?

It does not decide whether the theorem is useful, whether the formal statement matches the user's intent, whether imported axioms are appropriate, or whether a mathematical model describes the physical world.

## Supported backends

| Backend | Fence language | Executable | Typical use |
|---|---|---|---|
| Lean 4 | `lean` or `lean4` | `lean` | Mathematics and abstract specifications |
| Verus | `verus`, or Rust containing `verus!` | `verus` | Rust implementation invariants |
| Coq | `coq` | `coqc` | Existing Coq developments and interoperability |

The orchestrator extracts synthesized fenced artifacts, assigns their language, and invokes `FormalProofVerifier` during the verification stage. A checker must be installed on `PATH`; Crosstalk does not download proof tools automatically.

## Verification statuses

| Status | Meaning | Counts as verified? |
|---|---|---|
| `Verified` | Policy passed and the checker exited successfully | Yes |
| `Rejected` | The checker ran and returned a non-zero status | No |
| `CheckerUnavailable` | The executable was not found on `PATH` | No |
| `TimedOut` | The checker exceeded the wall-clock limit | No |
| `PolicyViolation` | Source contained a forbidden placeholder or escape hatch | No |

Errors starting the process or reading/writing proof source are verification errors, not successful checks.

## Strict source policy

The default policy rejects these tokens before execution:

| Backend | Rejected constructs |
|---|---|
| Lean 4 | `sorry`, `by?`, `admit` |
| Verus | `assume(false)`, `external_body` |
| Coq | `Admitted.`, `admit.` |

Matching is currently case-insensitive substring detection. This is intentionally conservative but not a complete semantic audit of axioms, imports, tactics, compiler plugins, or trusted declarations. A future policy layer should inspect dependency graphs and backend-specific trust reports rather than relying only on token rejection.

## Execution controls

`FormalProofVerifier` applies the following controls:

- maximum proof source: 2 MiB;
- default wall-clock limit: 30 seconds;
- maximum retained stdout/stderr diagnostics: 64 KiB;
- a newly created private scratch directory with mode `0700` on Unix;
- direct process execution without an intervening shell;
- cleared child environment, restoring only `PATH` and `HOME` when present;
- `kill_on_drop`, so a cancelled or timed-out future terminates the child;
- a SHA-256 digest over the exact submitted source in the typed verification result; and
- a bounded direct `--version` query whose result is recorded with the verification when available.

The scratch directory is removed after a normal checker result. Cleanup failures are logged.

## Security boundary

Lean, Verus, and Coq run as native child processes. The scratch directory and process limits reduce accidental exposure and resource use, but they are **not** a complete operating-system sandbox. Generated source may reference imports or exercise checker functionality outside the proof text. Use pinned checker versions, review imports and trusted axioms, and run high-risk verification in an OS/container sandbox with a read-only dependency store and no network.

The WASM sandbox used elsewhere in Crosstalk is not the execution environment for these proof assistants.

## Correct interpretation

An accepted proof means:

```text
checker(version, dependencies, trust configuration)
    accepted
SHA256(exact source)
```

It does not by itself mean:

- the formal statement matches the original natural-language claim;
- definitions encode the intended concepts;
- imported axioms or unsafe declarations are acceptable;
- an engineering system implements the proved specification;
- a scientific hypothesis is empirically true.

For high-confidence work, use two reviews: a kernel check for logical validity and a formalization audit that maps definitions and assumptions back to the user's claim.

## Local setup

Install only the backends needed for the task, then confirm they resolve from the same environment that starts Crosstalk:

```bash
command -v lean
command -v verus
command -v coqc
```

Backend installation is intentionally delegated to the official projects because supported toolchains and package managers change independently of Crosstalk. The repository's own Verus specifications have additional instructions in [VERUS.md](../VERUS.md).

## Example output contract

Ask a model to return a complete fenced artifact rather than prose alone:

````text
State every definition and assumption explicitly. If the conjecture survives
counterexample search, return a complete Lean 4 artifact in this form:

```lean4
-- imports, definitions, theorem, and proof
```

Do not use sorry, admit, or an equivalent placeholder.
````

If the checker is absent, the result remains `CheckerUnavailable`; Crosstalk must not silently downgrade that condition into a successful proof sketch.

## Engineering roadmap

1. Extend the implemented source/output digests, checker version, status, and diagnostics with dependency manifests and signed standalone verification artifacts.
2. Add backend-specific allow/deny policies for imports, axioms, unsafe declarations, and plugins.
3. Run proof tools inside a hardened, networkless execution sandbox.
4. Add a formalization-review stage that links natural-language claims to definitions and theorem statements.
5. Feed verified claim IDs—not only artifact-level booleans—back into the claim ledger and consensus reward.
6. Add reproducible toolchain manifests and CI images for benchmark proofs.
