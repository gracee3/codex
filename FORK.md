# Fork Maintenance

This fork now uses a two-lane model:

- `main` is a clean mirror of `upstream/main`
- `tt/main` is the long-lived TT product branch

TT is no longer treated as a small replayable overlay. Ongoing TT development,
including the separate `tt` binary and deeper runtime changes, should land on
`tt/main` and `tt/feature/*` branches. TT releases are cut from `tt/main`.

## Active branches

- `main`
  - exact upstream mirror
  - no TT product work should land here directly
- `tt/main`
  - primary TT development branch
  - receives upstream updates by merging `main`
- `tt/feature/<name>`
  - short-lived TT feature branches
- `releases/tt/<tag>`
  - TT release branches cut from `tt/main`

The legacy overlay branches remain as historical bootstrap material only:

- `fork/tt-runtime-contract`
- `fork/app-server-rollout`

Those branches should not be used as the primary ongoing TT workflow.

`fork/maint` may continue to carry generic fork-maintenance automation. Keep it
small and utility-focused.

## Workflow

Refresh the upstream mirror:

```bash
make sync-main
```

Create the long-lived TT branch once from the current TT baseline:

```bash
make create-tt-main BASE=releases/tt/rust-v0.120.0
```

Move to the TT product branch and do TT work there:

```bash
git switch tt/main
```

Merge upstream mirror updates into the TT branch at explicit sync points:

```bash
git switch tt/main
git merge main
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
- Keep TT work on `tt/main`
- Merge `main` into `tt/main` instead of rebasing the TT product branch
- Cut `releases/tt/*` from `tt/main`
- Do not rely on TT-specific `fork/*` branches for day-to-day product work

## Tooling

The fork maintenance helpers now support:

- `make sync-main`
- `make create-tt-main BASE=<ref>`
- `make new-release TAG=<rust-vX.Y.Z>`
- `make new-tt-release TAG=<rust-vX.Y.Z>`

`scripts/fork-release.sh list-patch-commits` remains available only as a
historical inspection tool for the old overlay workflow.
