## Installing & building

This repository supports a Linux x86_64 Cargo workflow only.

The supported local workflow is:

- build or install `tt`
- start the detached TT runtime
- attach to Director with `tt open`

### System requirements

| Requirement                 | Details                                                         |
| --------------------------- | --------------------------------------------------------------- |
| Operating systems           | Linux x86_64                                                    |
| Git (optional, recommended) | 2.23+ for repository workflows                                  |
| RAM                         | 4-GB minimum (8-GB recommended)                                 |

### Build from source

```bash
git clone <your-tt-fork-url>
cd codex/codex-rs

curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh -s -- -y
source "$HOME/.cargo/env"
rustup component add rustfmt clippy
cargo install just
cargo install --locked cargo-nextest
```

Build the supported binaries from the workspace root:

```bash
cd ..
just build
```

Or build directly with Cargo:

```bash
cargo build -p codex-tt-cli --bin tt -p codex-cli --bin codex -p codex-app-server --bin codex-app-server
```

Install locally:

```bash
just install
```

### TT runtime workflow

From the repository you want TT to manage:

```bash
tt init
tt start
tt open
```

Useful follow-up commands:

```bash
tt status
tt attach developer
tt auto on
tt auto off
tt pause
tt stop
```

### Development workflow

After making Rust changes:

```bash
cd codex-rs
just fmt
just fix -p <crate-you-touched>
cargo test -p <crate-you-touched>
```

Only Linux x86_64 Cargo-based local development is supported in this fork.
