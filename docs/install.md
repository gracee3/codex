## Installing & building

### Supported platform

| Requirement                 | Details                          |
| --------------------------- | -------------------------------- |
| Operating system            | Linux                            |
| Git (optional, recommended) | 2.23+ for built-in PR helpers    |
| RAM                         | 4 GB minimum, 8 GB recommended   |

### Install on Linux

```bash
curl -fsSL https://chatgpt.com/codex/install.sh | sh
codex
```

### Build from source

```bash
git clone https://github.com/openai/codex.git
cd codex/codex-rs

curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh -s -- -y
source "$HOME/.cargo/env"
rustup component add rustfmt clippy
cargo install just
cargo install --locked cargo-nextest

cargo build
cargo run --bin codex -- "explain this codebase to me"

just fmt
just fix -p <crate-you-touched>
cargo test -p codex-tui
```

### Logging

Codex honors `RUST_LOG`.

The TUI defaults to `RUST_LOG=codex_core=info,codex_tui=info,codex_rmcp_client=info` and writes logs to `~/.codex/log/codex-tui.log`.

```bash
tail -F ~/.codex/log/codex-tui.log
```
