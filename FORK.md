# Fork Maintenance

This fork now uses a three-branch model:

- `main` is the upstream release-sync branch
- `tt/base` is the long-lived structural reduction branch
- `tt/main` is the long-lived TT product branch on top of `tt/base`

This fork intentionally carries aggressive deletions and Linux-only Cargo
support on `tt/base`. TT product work lands on `tt/main` and `tt/feature/*`.

## Active branches

- `main`
  - usually refreshed from upstream before each TT release merge
  - may be pinned to a specific upstream release tag during a sync cycle
  - no TT product or cleanup work should land here directly
- `tt/base`
  - permanent hard-cut layer
  - receives upstream updates by merging `main`
  - owns repo reduction, Linux-only cleanup, and removal of upstream tooling
- `tt/main`
  - primary TT development branch
  - receives upstream updates by merging `tt/base`
- `tt/feature/<name>`
  - short-lived TT feature branches
- `releases/tt/<tag>`
  - TT release branches cut from `tt/main`

## Workflow

Refresh `main` from upstream:

```bash
just sync-main
```

If the fork is intentionally syncing to a stable upstream release instead of
upstream tip, pin `main` to the chosen release tag before merging it into
`tt/base`.

Create the long-lived TT branches once from the current TT baseline:

```bash
just create-tt-main releases/tt/rust-v0.120.0
git switch -c tt/base
git switch -c tt/main
```

Merge upstream into the base layer first:

```bash
git switch tt/base
git branch -f tt/base tt/main
git merge main
```

Then move the base layer into TT product work:

```bash
git switch tt/main
git merge tt/base
```

Cut a new TT release from `tt/main`:

```bash
just new-release rust-v0.120.0
```

The `TAG` now controls release naming, not patch-stack replay. A TT release is
the TT product state after upstream sync and stabilization, not an upstream tag
plus replayed overlays.

## Current policy

- Keep `main` aligned with the selected upstream sync target for the cycle
- Keep structural simplification on `tt/base`
- Keep TT product work on `tt/main`
- Merge `main` into `tt/base`, then `tt/base` into `tt/main`
- Cut `releases/tt/*` from `tt/main`
- Do not rebuild upstream compatibility surfaces in this fork

## Release Merge Notes

For the first upstream release merge, resolve conflicts in a way that teaches
`rerere` repeatable patterns instead of optimizing for a one-off clean merge.

Recurring conflict classes so far:

- Keep TT deletions for removed surfaces:
  - `.github/` upstream automation that does not apply to this fork
  - lockfiles and crate metadata for removed build integrations
  - `code-mode`, JS REPL, and related tests
  - removed platform-specific sandbox code and dead compatibility surfaces
- Re-review manually for retained Rust crates:
  - `codex-rs/core`
  - `codex-rs/app-server*`
  - `codex-rs/tui`
  - `codex-rs/exec*`
  - `codex-rs/tools`
- Check the top-level Rust workspace after every sync:
  - workspace version
  - newly added retained crates in `members`
  - matching `workspace.dependencies` entries
- Prefer deleting newly added upstream files that live entirely inside already
  cut subsystems rather than keeping dead support code around.

Recommended merge order for each release cycle:

```bash
git switch main
# Pin to upstream tag or update to upstream branch tip for the selected cycle.

git switch tt/base
git branch -f tt/base tt/main
git merge main

git switch tt/main
git merge tt/base
```

## Tooling

The fork maintenance helpers now support:

- `just sync-main`
- `just create-tt-main <ref>`
- `just new-release <rust-vX.Y.Z>`
- `just new-tt-release <rust-vX.Y.Z>`

`scripts/fork-release.sh list-patch-commits` remains only as a historical
inspection tool for the pre-`tt/base` workflow.
