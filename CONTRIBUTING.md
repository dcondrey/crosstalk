# Contributing to Crosstalk

Thanks for your interest in improving Crosstalk. This document describes how to
report issues, set up a development environment, and submit changes.

## Code of Conduct

This project follows the [Contributor Covenant](CODE_OF_CONDUCT.md). By
participating, you are expected to uphold it.

## How to Contribute

### Reporting Issues

- Use the [issue templates](.github/ISSUE_TEMPLATE) for bugs and feature requests.
- **Do not** report security vulnerabilities in public issues — see
  [SECURITY.md](SECURITY.md).

### Development Setup

Requires **Rust 1.91+** (edition 2024).

```sh
git clone https://github.com/dcondrey/crosstalk.git
cd crosstalk
cargo build
cargo test --workspace
```

### Making Changes

1. Create a topic branch off `main`.
2. Make focused, minimal changes; keep commits as single logical units.
3. Before opening a PR, the quality gate must be green:

   ```sh
   cargo fmt --all -- --check                            # formatting
   cargo clippy --workspace --all-targets -- -D warnings # lints
   cargo test --workspace                                # tests
   ```

4. Add a regression test for every fix or new behavior where feasible.

### Documentation

- Update the capability table and relevant document under [`docs/`](docs/) when
  a user-visible feature or trust boundary changes.
- Label features as implemented, experimental, or planned. Do not describe a
  model score, consensus result, simulation, or provenance record as proof.
- Keep CLI examples synchronized with `crosstalk --help`.
- Use repository-relative links for files in this checkout and verify new links
  before opening a pull request.
- For benchmark claims, include the task set, model and tool versions, resource
  budget, seeds, failure policy, and uncertainty. See
  [Evaluation Strategy](docs/evaluation.md).

### Code Style

- Match the surrounding code's idiom, naming, and comment density.
- `cargo fmt` (default rustfmt + `rustfmt.toml`) is the source of truth.
- Conventional commit subjects: `<type>: <description>` where
  `type ∈ fix | feat | refactor | test | docs | perf | security | chore`.

### Security-Sensitive Code

Changes under `src/engines/security.rs`, the tool gateway (`src/mcp/`), or the
sandbox (`src/engines/sandbox.rs`) warrant extra care:

- Do not roll custom cryptographic primitives.
- Keep key material in `Zeroizing`; never log secrets.
- Preserve the workspace-confinement and allow-list checks on tool directives.
- Document the security implications in the PR.

## Pull Request Process

- Fill out the [pull request template](.github/PULL_REQUEST_TEMPLATE.md).
- Keep PRs scoped to one concern; link related issues.
- All CI checks must pass and at least one maintainer must approve.
- Describe new external processes, network sources, persisted data, and model
  providers in the pull request's security/privacy impact section.

## License and Contributor Agreement

Crosstalk is licensed under [Apache-2.0](LICENSE). By contributing, you agree
that your contributions are licensed under the same terms.

For questions about the contributor agreement, contact: admin@writerslogic.com
