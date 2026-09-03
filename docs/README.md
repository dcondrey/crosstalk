<img src="../assets/icon.png" alt="Crosstalk" width="88" align="left">

<h1>Crosstalk documentation</h1>
<p><strong>Architecture, verification, research, evaluation, and contributor guides for Crosstalk.</strong></p>

<br clear="left">

Start with the document that matches the question you are trying to answer.

| Document | Use it for |
|---|---|
| [Domain-General Reasoning Architecture](general-intelligence.md) | System goals, deliberation loop, domain protocols, component boundaries, and roadmap |
| [Verifiable Discovery Workbench](discovery-workbench.md) | Investigation/evidence contracts, objective evaluators, Algorithm Discovery Lab, bundles, and headless operation |
| [Formal Verification](formal-verification.md) | Lean 4, Verus, and Coq execution policy, statuses, installation, and proof trust boundary |
| [Research and Evidence](research-and-evidence.md) | arXiv/Zenodo retrieval, provenance fields, evidence flow, limitations, and extension plan |
| [Evaluation Strategy](evaluation.md) | Reproducible capability comparisons, metrics, baselines, release gates, and reporting schema |
| [BlindMind Rust Migration Plan](blindmind-rust-migration.md) | Native evolutionary-discovery design, completed phases, compatibility contract, and remaining work |
| [Verus Formal Verification](../VERUS.md) | Repository-level Verus invariants and local proof commands |
| [`Sharded<K, V>` Design Note](substrate-sharded.md) | Planned concurrency substrate API |
| [Security Policy](../SECURITY.md) | Vulnerability reporting, security model, and deployment hardening |
| [Contributing](../CONTRIBUTING.md) | Development setup, quality gates, code style, and pull requests |

## Documentation rules

Documentation in this repository follows three evidence labels:

- **Implemented** means the code path exists and is covered by tests or a runnable validation path.
- **Experimental** means the code path exists but does not yet have enough operational or benchmark evidence for a stable capability claim.
- **Planned** means design work may exist, but users must not assume the feature is available.

Security and correctness claims must state their trust boundary. In particular, model consensus is not ground truth, provenance is not correctness, a simulation is not an experiment, and only an accepting proof checker establishes formal-verification status.
