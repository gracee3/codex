set working-directory := "codex-rs"
set positional-arguments

# Display help
help:
    just -l

FORK_SCRIPT := "../scripts/fork-release.sh"
TT_MAIN_BRANCH := "tt/main"
RELEASE_BRANCH_PREFIX := "releases/tt/"
INSTALL_DIR := "{{env_var('HOME')}}/.local/bin"

# `codex`
alias c := codex
codex *args:
    cargo run --bin codex -- "$@"

# `codex exec`
exec *args:
    cargo run --bin codex -- exec "$@"

# Start codex-exec-server and run codex-tui.
[no-cd]
tui-with-exec-server *args:
    ./scripts/run_tui_with_exec_server.sh "$@"

# Run the CLI version of the file-search crate.
file-search *args:
    cargo run --bin codex-file-search -- "$@"

# Build the CLI and run the app-server test client
app-server-test-client *args:
    cargo build -p codex-cli
    cargo run -p codex-app-server-test-client -- --codex-bin ./target/debug/codex "$@"

# format code
fmt:
    cargo fmt -- --config imports_granularity=Item 2>/dev/null

fix *args:
    cargo clippy --fix --tests --allow-dirty "$@"

clippy *args:
    cargo clippy --tests "$@"

toolchain:
    rustup show active-toolchain
    cargo fetch

# Run `cargo nextest` since it's faster than `cargo test`, though including
# --no-fail-fast is important to ensure all tests are run.
#
# Run `cargo install cargo-nextest` if you don't have it installed.
# Prefer this for routine local runs. Workspace crate features are banned, so
# there should be no need to add `--all-features`.
test:
    cargo nextest run --no-fail-fast

# Build local debug artifacts.
build:
    cargo build -p codex-cli --bin codex -p codex-tt-cli --bin tt -p codex-app-server --bin codex-app-server

build-release:
    cargo build --release -p codex-cli --bin codex -p codex-tt-cli --bin tt -p codex-app-server --bin codex-app-server

clean:
    cargo clean

install:
    mkdir -p {{INSTALL_DIR}}
    install -m 0755 target/debug/codex {{INSTALL_DIR}}/codex
    install -m 0755 target/debug/tt {{INSTALL_DIR}}/tt
    install -m 0755 target/debug/codex-app-server {{INSTALL_DIR}}/codex-app-server

install-release:
    mkdir -p {{INSTALL_DIR}}
    install -m 0755 target/release/codex {{INSTALL_DIR}}/codex
    install -m 0755 target/release/tt {{INSTALL_DIR}}/tt
    install -m 0755 target/release/codex-app-server {{INSTALL_DIR}}/codex-app-server

uninstall:
    rm -f {{INSTALL_DIR}}/codex {{INSTALL_DIR}}/tt {{INSTALL_DIR}}/codex-app-server

# Run the MCP server
mcp-server-run *args:
    cargo run -p codex-mcp-server -- "$@"

# Fork maintenance and release helpers (replacement for Makefile targets).
sync-main:
    {{FORK_SCRIPT}} sync-main

create-tt-main BASE:
    {{FORK_SCRIPT}} create-tt-main --base {{BASE}}

new-release TAG:
    TT_MAIN_BRANCH={{TT_MAIN_BRANCH}} RELEASE_BRANCH_PREFIX={{RELEASE_BRANCH_PREFIX}} RELEASE_SUFFIX="" {{FORK_SCRIPT}} new-release --tag {{TAG}}

new-tt-release TAG:
    TT_MAIN_BRANCH={{TT_MAIN_BRANCH}} RELEASE_BRANCH_PREFIX={{RELEASE_BRANCH_PREFIX}} RELEASE_SUFFIX="" {{FORK_SCRIPT}} new-release --tag {{TAG}}

list-patch-commits:
    {{FORK_SCRIPT}} list-patch-commits

latest-upstream-tag:
    @tag="$$(git ls-remote --refs --tags upstream 'rust-v*' | awk '{print $$2}' | sed -E 's#refs/tags/##' | grep -E '^rust-v[0-9]+(\.[0-9]+){2}$$' | sort -V | tail -n 1)"; \
    [ -n "$$tag" ] || { echo "Failed to resolve latest stable tag from upstream" >&2; exit 1; }; \
    printf '%s\n' "$$tag"

# Regenerate the json schema for config.toml from the current config types.
write-config-schema:
    cargo run -p codex-core --bin codex-write-config-schema

# Regenerate vendored app-server protocol schema artifacts.
write-app-server-schema *args:
    cargo run -p codex-app-server-protocol --bin write_schema_fixtures -- "$@"

[no-cd]
write-hooks-schema:
    cargo run --manifest-path ./codex-rs/Cargo.toml -p codex-hooks --bin write_hooks_schema_fixtures

# Tail logs from the state SQLite database
log *args:
    if [ "${1:-}" = "--" ]; then shift; fi; cargo run -p codex-state --bin logs_client -- "$@"
