#!/usr/bin/env bash
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPO_ROOT="$(cd "${SCRIPT_DIR}/.." && pwd)"
IMAGE="${FORK_RELEASE_CI_IMAGE:-rust:1.93-bookworm}"

die() {
  echo "$*" >&2
  exit 1
}

require_command() {
  command -v "$1" >/dev/null 2>&1 || die "Required command not found: $1"
}

main() {
  require_command docker

  docker run --rm -t \
    -v "${REPO_ROOT}:/work" \
    -w /work \
    "${IMAGE}" \
    bash -lc '
      set -euo pipefail
      export DEBIAN_FRONTEND=noninteractive
      apt-get update -y
      apt-get install -y --no-install-recommends pkg-config libcap-dev
      cargo install sccache --locked
      export PATH="${CARGO_HOME:-/usr/local/cargo}/bin:${PATH}"
      ./scripts/run-fork-release-ci.sh
    '
}

main "$@"
