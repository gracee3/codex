# Codex CLI

This fork is a Rust-only, Linux-only baseline cut of Codex CLI.

## Quickstart

Install Rust and build from source:

```bash
git clone https://github.com/openai/codex.git
cd codex/codex-rs
cargo build
cargo run --bin codex
```

For a release install on Linux, use the standalone installer:

```bash
curl -fsSL https://chatgpt.com/codex/install.sh | sh
codex
```

## Scope

- Rust binaries and source builds are the supported distribution path.
- Linux is the supported runtime platform for this fork.
- The JS/TS packaging, Bazel workspace, and non-Linux release surfaces have been removed from the active repo.

## Docs

- [Installing & building](./docs/install.md)
- [Contributing](./docs/contributing.md)
- [Open source fund](./docs/open-source-fund.md)

This repository is licensed under the [Apache-2.0 License](LICENSE).
