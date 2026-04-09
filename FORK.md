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

The current fork-only code changes are:

- release branch automation and release-branch CI in [`scripts/fork-release.sh`](/home/emmy/openai/codex/scripts/fork-release.sh), [`Makefile`](/home/emmy/openai/codex/Makefile), and [fork-release-ci.yml](/home/emmy/openai/codex/.github/workflows/fork-release-ci.yml)
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

Run the release-branch checks locally on your current machine:

```bash
make fork-release-ci
```

Run the same checks in a disposable Docker container:

```bash
make fork-release-ci-docker
```

By design, upstream `README.md` is left untouched to minimize merge churn. Fork-specific notes belong in this file.

## Release composition

`make new-release` and `make new-alpha-release` create release branches from upstream tags and then cherry-pick the fork overlay branches in this order:

- `fork/maint`
- `fork/dev-build-speedups`

That ensures each release branch contains both the release-branch CI workflow and the Cargo speedup patch.

## Local CI parity

The release-branch GitHub Action is mirrored locally by:

- [run-fork-release-ci.sh](/home/emmy/openai/codex/scripts/run-fork-release-ci.sh): runs on the current machine and expects Rust plus `sccache` to already be installed
- [run-fork-release-ci-docker.sh](/home/emmy/openai/codex/scripts/run-fork-release-ci-docker.sh): runs in a disposable Docker container and installs its own dependencies

The Docker path is the closer match to GitHub Actions because it starts from a fresh environment each time.
