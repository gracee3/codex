## Contributing

External code contributions are still by invitation only.

If you are invited to work in this fork, keep changes small, add meaningful tests, and update user-facing docs when behavior changes.

### Development workflow

```bash
cd codex-rs
just fmt
just fix -p <crate>
cargo test -p <crate>
```

If you touch shared crates, run the broader crate tests that depend on them before opening a PR.

### Notes

- This repo is the Rust-only, Linux-only active fork.
- JS/TS packaging, Bazel workspace files, and non-Linux release flows are intentionally out of scope here.
- If you change config schema types, run `just write-config-schema`.

### Security

If you discover a vulnerability, email `security@openai.com`.
