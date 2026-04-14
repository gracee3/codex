# TT Rust Workspace

This workspace contains the Rust code for the TT fork.

Supported environment:

- Linux
- x86_64
- Cargo-based local development

Unsupported in this fork:

- npm, pnpm, and Bun distribution flows
- Homebrew packaging
- Bazel
- GitHub Actions workflows
- Windows and macOS support
- SDK packaging outside the Rust workspace

## Main binaries

- `tt`: TT orchestration CLI and detached runtime
- `codex`: shared Codex-derived CLI binary used by the fork
- `codex-app-server`: app-server used by `tt` and related tooling

## Local development

From [`codex-rs/`](./):

```sh
cargo build -p codex-tt-cli --bin tt -p codex-cli --bin codex -p codex-app-server --bin codex-app-server
```

Common commands:

- `just fmt`
- `cargo test -p codex-tt-core`
- `cargo test -p codex-tt-cli`
- `cargo test -p codex-cli`
- `cargo test -p codex-app-server`

## Workspace layout

- [`tt-core/`](./tt-core): TT runtime state, artifacts, and routing helpers
- [`tt-cli/`](./tt-cli): `tt` binary
- [`cli/`](./cli): `codex` binary
- [`app-server/`](./app-server): local app-server
- [`core/`](./core): shared runtime and configuration logic
- [`tui/`](./tui): terminal UI used by `codex` and TT attach flows

The remaining crates exist to support these Linux/Cargo binaries. This fork is
maintained as a Rust-first codebase rather than a multi-language distribution
repo.
