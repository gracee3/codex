# TT

Linux x86_64 TT runtime built from a stripped-down Rust fork.

Supported binaries:

- `tt` for TT orchestration, detached runtime control, and role attachment
- `codex` for direct CLI and debugging workflows
- `codex-app-server` as the shared runtime backend

This repository intentionally supports only:

- Linux x86_64
- Cargo-based Rust development
- the TT-managed runtime and related Rust binaries

Everything else from upstream has been cut away.

## Quickstart

Build and install the local binaries:

```bash
just install
```

Initialize TT in a repository, start the detached runtime, and attach to the
Director view:

```bash
tt init
tt start
tt open
```

Useful runtime commands:

```bash
tt status
tt attach developer
tt auto on
tt auto off
tt pause
tt stop
```

## Build From Source

```bash
git clone <your-tt-fork-url>
cd codex/codex-rs

curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh -s -- -y
source "$HOME/.cargo/env"
rustup component add rustfmt clippy
cargo install just
cargo install --locked cargo-nextest

cargo build -p codex-tt-cli --bin tt -p codex-cli --bin codex -p codex-app-server --bin codex-app-server
```

The root `justfile` wraps the remaining local workflows:

```bash
just build
just install
just sync-main
```

## Repo Layout

- `codex-rs/tt-cli` contains the `tt` binary
- `codex-rs/tt-core` contains TT runtime state and routing helpers
- `docs/tt_codex_runtime_contract.md` describes the detached TT runtime
- `FORK.md` describes the `main -> tt/base -> tt/main` maintenance workflow

## Docs

- [Installing & building](./docs/install.md)
- [TT runtime contract](./docs/tt_codex_runtime_contract.md)
- [Contributing](./docs/contributing.md)
- [Fork maintenance](./FORK.md)

This repository is licensed under the [Apache-2.0 License](LICENSE).
