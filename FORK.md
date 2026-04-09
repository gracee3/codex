# Fork Maintenance

This fork intentionally keeps `main` aligned with `upstream/main`.

Fork-specific behavior lives in two places:

- `fork/maint`: maintenance helpers for syncing upstream and creating release branches.
- `fork/dev-build-speedups`: a single-commit branch that carries the local Rust build tuning.

Release branches are created from exact upstream tags and then have the speedup commit cherry-picked on top:

- `releases/rust-v0.118.0`
- `releases/rust-v0.119.0`
- `releases/rust-v0.119.0-alpha.1`

## Fork delta

The current fork-only code change is in [`codex-rs/.cargo/config.toml`](/home/emmy/openai/codex/codex-rs/.cargo/config.toml):

- enable incremental dev builds
- use `sccache` as the Rust compiler wrapper
- increase `profile.dev` codegen units
- keep build script and proc-macro compilation optimized

## Workflow

Sync local `main` to upstream:

```bash
make sync-main
```

Sync local `main` and push the mirror to `origin/main`:

```bash
make sync-main PUSH=1
```

Create a release branch from the latest stable tag and cherry-pick the speedup patch:

```bash
make new-release
```

Create a release branch from an explicit tag:

```bash
make new-release TAG=rust-v0.118.0
```

Create a release branch from the latest alpha tag:

```bash
make new-alpha-release
```

Push the created release branch to the fork:

```bash
make new-release TAG=rust-v0.119.0 PUSH=1
```

By design, upstream `README.md` is left untouched to minimize merge churn. Fork-specific notes belong in this file.
