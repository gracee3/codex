## Installing & building

This repository supports a Linux x86_64 Cargo workflow only.

The supported local workflow is:

- build or install `tt`
- create a TT workspace with `tt clone`
- start the detached TT runtime
- attach to the supervisor view with `tt open`

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

Create and use a TT workspace:

```bash
tt clone <repo-url>
cd <workspace>/primary
tt start
tt open
```

Useful follow-up commands:

```bash
tt worker add feature-a
tt worker list
tt worker read director
tt worker send feature-a --message "pick up the next task"
tt worker adopt imported --thread-id <thread-id>
tt worker remove imported
tt worker attach feature-a
tt status
tt auto on
tt auto off
tt pause
tt stop
```

### Development workflow

The supervisor MVP keeps one shared runtime per TT workspace. The supervisor
thread can call TT-only worker control tools (`tt_worker_list`, `tt_worker_read`,
`tt_worker_send`, `tt_worker_remove`), while other worker threads cannot.

`tt worker send` is state-aware: it starts a new turn for an idle worker, and
steers the active turn when the worker already has an in-progress steerable
turn.

`tt worker adopt` only accepts threads whose `cwd` is inside the current
workspace. Adopted workers persist in TT state as strict thread bindings:
`tt stop` and `tt start` will only resume the exact adopted `thread_id`, and
TT will not silently replace it with a new thread. If that adopted thread can
no longer be resumed, the runtime still starts, but the worker remains
unavailable until it is fixed or removed.

After making Rust changes:

```bash
cd codex-rs
just fmt
just fix -p <crate-you-touched>
cargo test -p <crate-you-touched>
```

No Bazel, npm, pnpm, Nix, macOS, Windows, or GitHub Actions workflow is
supported in this fork.
