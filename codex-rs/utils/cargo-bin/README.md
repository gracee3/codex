# codex-utils-cargo-bin

Helpers for Cargo-based tests in this fork.

This fork no longer supports Bazel runfiles. The helpers in this crate assume:

- tests are built and run with Cargo
- `CARGO_BIN_EXE_*` values are absolute paths
- test fixtures are resolved relative to `CARGO_MANIFEST_DIR`

The crate remains small on purpose: it exists to give integration tests a
stable way to find workspace binaries and local resources in a Cargo-only
environment.
