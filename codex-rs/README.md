# Codex CLI (Rust Workspace)

This workspace contains the Rust implementation of Codex CLI for the Linux-only fork.

## Install

Build from source:

```bash
cargo build
cargo run --bin codex
```

Install the Linux release build:

```bash
curl -fsSL https://chatgpt.com/codex/install.sh | sh
codex
```

## Workspace overview

- `core/` contains the main agent runtime and tool routing logic.
- `cli/` contains the top-level CLI.
- `tui/` contains the terminal UI.
- `exec/` contains the non-interactive execution path.
- `app-server/` contains the app-server entrypoint and protocol support.

## Notes

- This fork no longer ships npm, Homebrew, TypeScript SDK, or Bazel workspace surfaces.
- The supported sandbox debug subcommand is `codex sandbox linux`.
- For configuration details, see [`../docs/config.md`](../docs/config.md).
