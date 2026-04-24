# Workflow Strategy

This fork uses Linux-only Cargo-based verification.

## Pull Requests

- `rust-ci.yml` is the main pre-merge workflow.
- Keep PR checks focused on formatting, linting, and targeted Rust tests.

## Releases

- Release automation in this fork is expected to publish Linux Rust artifacts only.
- npm, Homebrew, Bazel, and cross-platform packaging flows are intentionally out of scope here.
