#!/usr/bin/env bash
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPO_ROOT="$(cd "${SCRIPT_DIR}/.." && pwd)"

UPSTREAM_REMOTE="${UPSTREAM_REMOTE:-upstream}"
FORK_REMOTE="${FORK_REMOTE:-origin}"
MIRROR_BRANCH="${MIRROR_BRANCH:-main}"
PATCH_BRANCH="${PATCH_BRANCH:-fork/dev-build-speedups}"
PATCH_BRANCHES="${PATCH_BRANCHES:-fork/maint fork/dev-build-speedups fork/tt-runtime-contract fork/app-server-rollout}"
RELEASE_BRANCH_PREFIX="${RELEASE_BRANCH_PREFIX:-releases/}"
RELEASE_SUFFIX="${RELEASE_SUFFIX:-}"
TAG_PREFIX="rust-v"

usage() {
  cat <<'EOF'
Usage:
  scripts/fork-release.sh sync-main [--push]
  scripts/fork-release.sh new-release [--tag <tag>] [--alpha] [--push]
  scripts/fork-release.sh list-patch-commits
  scripts/fork-release.sh list-tags

Examples:
  scripts/fork-release.sh sync-main
  scripts/fork-release.sh sync-main --push
  scripts/fork-release.sh new-release
  scripts/fork-release.sh new-release --tag rust-v0.118.0
  scripts/fork-release.sh new-release --alpha
  scripts/fork-release.sh list-patch-commits

Environment overrides:
  UPSTREAM_REMOTE         default: upstream
  FORK_REMOTE             default: origin
  MIRROR_BRANCH           default: main
  PATCH_BRANCHES          default: "fork/maint fork/dev-build-speedups fork/tt-runtime-contract fork/app-server-rollout"
  RELEASE_BRANCH_PREFIX   default: releases/
  RELEASE_SUFFIX          default: ""
EOF
}

die() {
  echo "$*" >&2
  exit 1
}

require_command() {
  command -v "$1" >/dev/null 2>&1 || die "Required command not found: $1"
}

ensure_repo_root() {
  git -C "$REPO_ROOT" rev-parse --is-inside-work-tree >/dev/null 2>&1 || die "Not a git repository: $REPO_ROOT"
}

normalize_tag() {
  local input="$1"
  case "$input" in
    "${TAG_PREFIX}"*)
      printf '%s\n' "$input"
      ;;
    v[0-9]*)
      printf '%s%s\n' "$TAG_PREFIX" "${input#v}"
      ;;
    [0-9]*)
      printf '%s%s\n' "$TAG_PREFIX" "$input"
      ;;
    *)
      printf '%s\n' "$input"
      ;;
  esac
}

validate_tag() {
  local tag="$1"
  [[ "$tag" =~ ^rust-v[0-9]+(\.[0-9]+){2}([.-][A-Za-z0-9._-]+)?$ ]] || die "Invalid tag format: $tag"
}

fetch_remotes() {
  git -C "$REPO_ROOT" fetch --prune "$UPSTREAM_REMOTE" --tags --force
  git -C "$REPO_ROOT" fetch --prune "$FORK_REMOTE"
}

latest_stable_tag() {
  git -C "$REPO_ROOT" tag -l 'rust-v*' | grep -E '^rust-v[0-9]+(\.[0-9]+){2}$' | sort -V | tail -n 1
}

latest_alpha_tag() {
  git -C "$REPO_ROOT" tag -l 'rust-v*' | grep -E '^rust-v[0-9]+(\.[0-9]+){2}-alpha\.[0-9]+$' | sort -V | tail -n 1
}

resolve_patch_commits() {
  local branch
  local merge_base
  local branch_commits
  local seen=()

  for branch in $PATCH_BRANCHES; do
    git -C "$REPO_ROOT" show-ref --verify --quiet "refs/heads/$branch" || die "Missing patch branch: $branch"
    merge_base="$(git -C "$REPO_ROOT" merge-base "$branch" "$MIRROR_BRANCH")"
    mapfile -t branch_commits < <(git -C "$REPO_ROOT" rev-list --reverse "${merge_base}..${branch}")
    [ "${#branch_commits[@]}" -gt 0 ] || die "Patch branch has no commits ahead of $MIRROR_BRANCH: $branch"

    local commit
    for commit in "${branch_commits[@]}"; do
      if [[ " ${seen[*]} " == *" ${commit} "* ]]; then
        continue
      fi
      seen+=("$commit")
      printf '%s\n' "$commit"
    done
  done
}

list_patch_commits() {
  local commit

  while IFS= read -r commit; do
    git -C "$REPO_ROOT" show -s --format='%h %s' "$commit"
  done < <(resolve_patch_commits)
}

apply_branch_snapshot() {
  local branch="$1"
  local commit="$2"
  local merge_base
  local path

  merge_base="$(git -C "$REPO_ROOT" merge-base "$branch" "$MIRROR_BRANCH")"

  while IFS= read -r path; do
    git -C "$REPO_ROOT" checkout "$branch" -- "$path"
  done < <(git -C "$REPO_ROOT" diff --no-renames --name-only --diff-filter=ACMRT "${merge_base}..${branch}")

  while IFS= read -r path; do
    git -C "$REPO_ROOT" rm -f --ignore-unmatch -- "$path" >/dev/null
  done < <(git -C "$REPO_ROOT" diff --no-renames --name-only --diff-filter=D "${merge_base}..${branch}")

  git -C "$REPO_ROOT" commit -m "$(git -C "$REPO_ROOT" show -s --format='%s' "$commit")" >/dev/null
}

cherry_pick_commits() {
  local commit
  local output

  for commit in "$@"; do
    if output="$(git -C "$REPO_ROOT" cherry-pick -X theirs "$commit" 2>&1)"; then
      continue
    fi

    if [[ "$output" == *"previous cherry-pick is now empty"* ]]; then
      printf 'Skipping empty cherry-pick: %s\n' "$(git -C "$REPO_ROOT" show -s --format='%h %s' "$commit")"
      git -C "$REPO_ROOT" cherry-pick --skip >/dev/null
      continue
    fi

    printf '%s\n' "$output" >&2
    return 1
  done
}

sync_main() {
  local push="${1:-0}"

  fetch_remotes
  git -C "$REPO_ROOT" update-ref "refs/heads/${MIRROR_BRANCH}" "${UPSTREAM_REMOTE}/${MIRROR_BRANCH}"

  if [ "$push" = "1" ]; then
    git -C "$REPO_ROOT" push --force-with-lease "$FORK_REMOTE" "$MIRROR_BRANCH"
  fi

  git -C "$REPO_ROOT" status --short --branch
}

create_release_branch() {
  local tag="$1"
  local push="${2:-0}"

  validate_tag "$tag"
  git -C "$REPO_ROOT" show-ref --verify --quiet "refs/tags/$tag" || die "Tag not found: $tag"

  local release_branch="${RELEASE_BRANCH_PREFIX}${tag}"
  if [ -n "$RELEASE_SUFFIX" ]; then
    release_branch="${release_branch}-${RELEASE_SUFFIX}"
  fi
  git -C "$REPO_ROOT" show-ref --verify --quiet "refs/heads/$release_branch" && die "Release branch already exists: $release_branch"

  mapfile -t patch_commits < <(resolve_patch_commits)
  [ "${#patch_commits[@]}" -gt 0 ] || die "No patch commits resolved"

  git -C "$REPO_ROOT" switch -c "$release_branch" "$tag" >/dev/null
  if [ "${#patch_commits[@]}" -gt 1 ]; then
    cherry_pick_commits "${patch_commits[@]:1}"
  fi
  set -- $PATCH_BRANCHES
  apply_branch_snapshot "$1" "${patch_commits[0]}"

  if [ "$push" = "1" ]; then
    git -C "$REPO_ROOT" push -u "$FORK_REMOTE" "$release_branch"
  fi

  git -C "$REPO_ROOT" status --short --branch
}

main() {
  require_command git
  ensure_repo_root

  [ "$#" -gt 0 ] || {
    usage
    exit 1
  }

  local cmd="$1"
  shift

  case "$cmd" in
    sync-main)
      local push=0
      while [ "$#" -gt 0 ]; do
        case "$1" in
          --push)
            push=1
            ;;
          -h|--help)
            usage
            exit 0
            ;;
          *)
            die "Unknown argument for sync-main: $1"
            ;;
        esac
        shift
      done
      sync_main "$push"
      ;;
    new-release)
      local tag=""
      local push=0
      local alpha=0
      while [ "$#" -gt 0 ]; do
        case "$1" in
          --tag)
            [ "$#" -ge 2 ] || die "--tag requires a value"
            tag="$(normalize_tag "$2")"
            shift
            ;;
          --push)
            push=1
            ;;
          --alpha)
            alpha=1
            ;;
          -h|--help)
            usage
            exit 0
            ;;
          *)
            die "Unknown argument for new-release: $1"
            ;;
        esac
        shift
      done

      fetch_remotes
      if [ -z "$tag" ]; then
        if [ "$alpha" = "1" ]; then
          tag="$(latest_alpha_tag)"
          [ -n "$tag" ] || die "Unable to resolve the latest alpha tag"
        else
          tag="$(latest_stable_tag)"
          [ -n "$tag" ] || die "Unable to resolve the latest stable tag"
        fi
      fi

      create_release_branch "$tag" "$push"
      ;;
    list-patch-commits)
      while [ "$#" -gt 0 ]; do
        case "$1" in
          -h|--help)
            usage
            exit 0
            ;;
          *)
            die "Unknown argument for list-patch-commits: $1"
            ;;
        esac
        shift
      done

      list_patch_commits
      ;;
    list-tags)
      fetch_remotes
      git -C "$REPO_ROOT" tag -l 'rust-v*' | sort -V
      ;;
    -h|--help)
      usage
      ;;
    *)
      die "Unknown command: $cmd"
      ;;
  esac
}

main "$@"
