#!/usr/bin/env bash
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPO_ROOT="$(cd "${SCRIPT_DIR}/.." && pwd)"

die() {
  echo "$*" >&2
  exit 1
}

require_command() {
  command -v "$1" >/dev/null 2>&1 || die "Required command not found: $1"
}

ensure_linux_deps() {
  if [[ "$(uname -s)" != "Linux" ]]; then
    return
  fi

  require_command pkg-config
  pkg-config --exists libcap || die "Missing libcap development files. Install pkg-config and libcap-dev first."
}

main() {
  require_command cargo
  require_command rustup
  require_command sccache
  ensure_linux_deps

  export SCCACHE_GHA_ENABLED="${SCCACHE_GHA_ENABLED:-true}"
  export RUSTC_WRAPPER="${RUSTC_WRAPPER:-sccache}"
  export CARGO_TERM_COLOR="${CARGO_TERM_COLOR:-always}"

  cd "${REPO_ROOT}/codex-rs"

  cargo fmt -- --config imports_granularity=Item --check
  cargo build -p codex-cli
  cargo test -p codex-app-server-protocol
  cargo test -p codex-app-server
  sccache --show-stats || true
}

main "$@"
