# Fork Maintenance

This fork now uses a three-branch model:

- `main` is a clean mirror of `upstream/main`
- `tt/cuts` is the long-lived structural reduction branch
- `tt/main` is the long-lived TT product branch on top of `tt/cuts`

This fork intentionally carries aggressive deletions and Linux-only Cargo
support on `tt/cuts`. TT product work lands on `tt/main` and `tt/feature/*`.

## Active branches

- `main`
  - exact upstream mirror
  - no TT product or cleanup work should land here directly
- `tt/cuts`
  - permanent hard-cut layer
  - receives upstream updates by merging `main`
  - owns repo reduction, Linux-only cleanup, and removal of upstream tooling
- `tt/main`
  - primary TT development branch
  - receives upstream updates by merging `tt/cuts`
- `tt/feature/<name>`
  - short-lived TT feature branches
- `releases/tt/<tag>`
  - TT release branches cut from `tt/main`

## Workflow

Refresh the upstream mirror:

```bash
make sync-main
```

Create the long-lived TT branches once from the current TT baseline:

```bash
make create-tt-main BASE=releases/tt/rust-v0.120.0
git switch -c tt/cuts
git switch -c tt/main
```

Merge upstream into the cut layer first:

```bash
git switch tt/cuts
git merge main
```

Then move the cut layer into TT product work:

```bash
git switch tt/main
git merge tt/cuts
```

Cut a new TT release from `tt/main`:

```bash
make new-release TAG=rust-v0.120.0
```

The `TAG` now controls release naming, not patch-stack replay. A TT release is
the TT product state after upstream sync and stabilization, not an upstream tag
plus replayed overlays.

## Current policy

- Keep `main` aligned with `upstream/main`
- Keep structural simplification on `tt/cuts`
- Keep TT product work on `tt/main`
- Merge `main` into `tt/cuts`, then `tt/cuts` into `tt/main`
- Cut `releases/tt/*` from `tt/main`
- Do not rebuild upstream compatibility surfaces in this fork

## Tooling

The fork maintenance helpers now support:

- `make sync-main`
- `make create-tt-main BASE=<ref>`
- `make new-release TAG=<rust-vX.Y.Z>`
- `make new-tt-release TAG=<rust-vX.Y.Z>`

`scripts/fork-release.sh list-patch-commits` remains only as a historical
inspection tool for the pre-`tt/cuts` workflow.
