# Fork Maintenance

This fork intentionally keeps `main` aligned with `upstream/main`.

Fork-specific behavior lives in two places:

- `fork/maint`: maintenance helpers for syncing upstream and creating release branches.
- `fork/dev-build-speedups`: a single-commit branch that carries the local Rust build tuning.
- `fork/tt-runtime-contract`: TT-facing docs and contract notes for TT-integrated release branches.
- `fork/app-server-rollout`: app-server runtime fixes that TT release builds should inherit.

Release branches are created from exact upstream tags and then have the fork overlays cherry-picked on top.

## Fork delta

The current fork-only code changes are:

- release branch automation in [`scripts/fork-release.sh`](/home/emmy/openai/codex/scripts/fork-release.sh) and [`Makefile`](/home/emmy/openai/codex/Makefile)
- local build and install tooling in [`Makefile`](/home/emmy/openai/codex/Makefile) and release automation in [`scripts/fork-release.sh`](/home/emmy/openai/codex/scripts/fork-release.sh)
- Rust build tuning in [`codex-rs/.cargo/config.toml`](/home/emmy/openai/codex/codex-rs/.cargo/config.toml)

The Rust build tuning does the following:

- enable incremental dev builds
- use `sccache` as the Rust compiler wrapper
- increase `profile.dev` codegen units
- keep build script and proc-macro compilation optimized

## Workflow

Sync local `main` to upstream:

```bash
make sync-main
```

That updates the local `main` ref directly, so you can run it from `fork/maint` without checking out `main` first.

Sync local `main` and push the mirror to `origin/main`:

```bash
make sync-main PUSH=1
```

Create a TT release branch from the latest stable tag and cherry-pick the fork overlays:

```bash
make new-release
```

Create a release branch from an explicit tag:

```bash
make new-release TAG=rust-v0.118.0
```

Add `RELEASE_SUFFIX=1` if you want a branch name like `releases/tt/rust-v0.120.0-1` instead of reusing the plain tag name.

Create a TT release branch that layers the TT contract overlay on top of the release:

```bash
make new-tt-release
```

Push the created release branch to the fork:

```bash
make new-release TAG=rust-v0.119.0 PUSH=1
```

By design, upstream `README.md` is left untouched to minimize merge churn. Fork-specific notes belong in this file.

## Release composition

`make new-release` and `make new-tt-release` create release branches from upstream tags and then cherry-pick the fork overlay branches in this order:

- `fork/maint`
- `fork/dev-build-speedups`
- `fork/tt-runtime-contract`
- `fork/app-server-rollout`

That ensures each release branch contains the release-branch CI workflow, the Cargo speedup patch, and the TT runtime/app-server overlays.

## Installer Model

There are local build and install paths:

- `make build` / `make build-release`
  Build the current checkout in debug or release mode
- `make install` / `make install-release`
  Build the current checkout in debug or release mode and install `codex` and `codex-app-server` into `~/.local/bin`
Use `make latest-upstream-tag` when you need the latest stable `rust-v*` tag from upstream without touching the build/install helpers. Use `make clean` to clear Cargo build artifacts.

## TT Overlay

TT-specific releases can use:

- `make new-tt-release`
Those commands add the TT overlay branches to the release composition order:

- `fork/maint`
- `fork/dev-build-speedups`
- `fork/tt-runtime-contract`
- `fork/app-server-rollout`

Put workflow notes that other agents should inherit in this file so they travel with `fork/maint` and the TT release branches that cherry-pick it.
