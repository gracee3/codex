#!/usr/bin/env bash
set -euo pipefail

SOURCE_DIR="${CODEX_SOURCE_DIR:-$HOME/git/codex}"
SOURCE_REMOTE="${CODEX_SOURCE_REMOTE:-git@github.com:gracee3/codex.git}"
UPSTREAM_REMOTE_URL="${CODEX_UPSTREAM_REMOTE_URL:-https://github.com/openai/codex.git}"
INSTALL_DIR="${CODEX_INSTALL_DIR:-$HOME/.local/bin}"
VERSION="${CODEX_INSTALL_VERSION:-latest}"
PROFILE="${CODEX_SOURCE_PROFILE:-debug}"
PATCH_BRANCHES="${CODEX_SOURCE_PATCH_BRANCHES:-fork/maint fork/dev-build-speedups}"
RELEASE_BRANCH_PREFIX="${CODEX_SOURCE_RELEASE_BRANCH_PREFIX:-releases/}"

usage() {
  cat <<EOF
Usage: scripts/source-install.sh [options]

Options:
  --dir <path>           Source checkout directory
  --remote <url>         Fork remote URL
  --install-dir <path>   Install target directory
  --tag <tag>            Release tag or version, default latest stable
  --alpha                Use latest alpha tag
  --profile <profile>    debug or release, default debug
  --refresh              Recreate the local release branch if it exists
EOF
}

die() {
  echo "$*" >&2
  exit 1
}

require_command() {
  command -v "$1" >/dev/null 2>&1 || die "Required command not found: $1"
}

normalize_tag() {
  local input="${1:-latest}"
  case "$input" in
    "" | latest)
      printf 'latest\n'
      ;;
    rust-v*)
      printf '%s\n' "$input"
      ;;
    v*)
      printf 'rust-v%s\n' "${input#v}"
      ;;
    [0-9]*)
      printf 'rust-v%s\n' "$input"
      ;;
    *)
      printf '%s\n' "$input"
      ;;
  esac
}

ensure_repo() {
  if git -C "$SOURCE_DIR" rev-parse --is-inside-work-tree >/dev/null 2>&1; then
    return
  fi

  mkdir -p "$(dirname "$SOURCE_DIR")"
  git clone --branch fork/maint "$SOURCE_REMOTE" "$SOURCE_DIR"
}

ensure_remote() {
  local name="$1"
  local url="$2"
  if git -C "$SOURCE_DIR" remote get-url "$name" >/dev/null 2>&1; then
    git -C "$SOURCE_DIR" remote set-url "$name" "$url"
  else
    git -C "$SOURCE_DIR" remote add "$name" "$url"
  fi
}

ensure_local_branch() {
  local branch="$1"
  if git -C "$SOURCE_DIR" show-ref --verify --quiet "refs/heads/$branch"; then
    return
  fi
  git -C "$SOURCE_DIR" show-ref --verify --quiet "refs/remotes/origin/$branch" || die "Missing remote branch origin/$branch"
  git -C "$SOURCE_DIR" branch --track "$branch" "origin/$branch" >/dev/null
}

resolve_tag() {
  local requested="$1"
  local alpha="$2"
  if [ "$requested" != "latest" ]; then
    printf '%s\n' "$requested"
    return
  fi

  if [ "$alpha" = "1" ]; then
    git -C "$SOURCE_DIR" tag -l 'rust-v*' | grep -E '^rust-v[0-9]+(\.[0-9]+){2}-alpha\.[0-9]+$' | sort -V | tail -n 1
  else
    git -C "$SOURCE_DIR" tag -l 'rust-v*' | grep -E '^rust-v[0-9]+(\.[0-9]+){2}$' | sort -V | tail -n 1
  fi
}

install_from_source() {
  local branch="$1"
  local cargo_args=()
  local target_dir="target/debug"

  git -C "$SOURCE_DIR" switch "$branch" >/dev/null
  cd "$SOURCE_DIR/codex-rs"

  if [ "$PROFILE" = "release" ]; then
    cargo_args+=(--release)
    target_dir="target/release"
  elif [ "$PROFILE" != "debug" ]; then
    die "Unsupported profile: $PROFILE"
  fi

  cargo build "${cargo_args[@]}" -p codex-cli --bin codex -p codex-app-server --bin codex-app-server

  mkdir -p "$INSTALL_DIR"
  install -m 0755 "${target_dir}/codex" "$INSTALL_DIR/codex"
  install -m 0755 "${target_dir}/codex-app-server" "$INSTALL_DIR/codex-app-server"
}

main() {
  require_command git
  require_command cargo
  require_command install

  local refresh=0
  local alpha=0

  while [ "$#" -gt 0 ]; do
    case "$1" in
      --dir)
        SOURCE_DIR="$2"
        shift 2
        ;;
      --remote)
        SOURCE_REMOTE="$2"
        shift 2
        ;;
      --install-dir)
        INSTALL_DIR="$2"
        shift 2
        ;;
      --tag)
        VERSION="$2"
        shift 2
        ;;
      --alpha)
        alpha=1
        shift
        ;;
      --profile)
        PROFILE="$2"
        shift 2
        ;;
      --refresh)
        refresh=1
        shift
        ;;
      -h|--help)
        usage
        exit 0
        ;;
      *)
        die "Unknown argument: $1"
        ;;
    esac
  done

  VERSION="$(normalize_tag "$VERSION")"
  ensure_repo
  ensure_remote origin "$SOURCE_REMOTE"
  ensure_remote upstream "$UPSTREAM_REMOTE_URL"

  git -C "$SOURCE_DIR" fetch --prune origin --tags
  git -C "$SOURCE_DIR" fetch --prune upstream --tags

  local branch
  for branch in $PATCH_BRANCHES; do
    ensure_local_branch "$branch"
  done
  ensure_local_branch main

  local tag
  tag="$(resolve_tag "$VERSION" "$alpha")"
  [ -n "$tag" ] || die "Unable to resolve release tag"

  local release_branch="${RELEASE_BRANCH_PREFIX}${tag}"
  if git -C "$SOURCE_DIR" show-ref --verify --quiet "refs/heads/${release_branch}" && [ "$refresh" = "1" ]; then
    git -C "$SOURCE_DIR" switch fork/maint >/dev/null
    git -C "$SOURCE_DIR" branch -D "$release_branch" >/dev/null
  fi

  if ! git -C "$SOURCE_DIR" show-ref --verify --quiet "refs/heads/${release_branch}"; then
    PATCH_BRANCHES="$PATCH_BRANCHES" "$SOURCE_DIR/scripts/fork-release.sh" new-release --tag "$tag"
  fi

  install_from_source "$release_branch"
  printf 'Installed %s from %s to %s using %s profile\n' "$release_branch" "$SOURCE_DIR" "$INSTALL_DIR" "$PROFILE"
}

main "$@"
