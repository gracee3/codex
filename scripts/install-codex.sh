#!/usr/bin/env bash
set -euo pipefail

REPO_SLUG="${CODEX_INSTALL_REPO_SLUG:-gracee3/codex}"
INSTALL_DIR="${CODEX_INSTALL_DIR:-$HOME/.local/bin}"
CONFIG_DIR="${CODEX_CONFIG_DIR:-$HOME/.codex}"
VERSION="${CODEX_INSTALL_VERSION:-latest}"

usage() {
  cat <<EOF
Usage: scripts/install-codex.sh [--install-dir <path>] <command> [arg]

Commands:
  latest
  status [config]
  install [latest|<version>|v<version>|rust-v<version>|config]
  uninstall [all]
EOF
}

die() {
  echo "$*" >&2
  exit 1
}

require_command() {
  command -v "$1" >/dev/null 2>&1 || die "Required command not found: $1"
}

download_text() {
  local url="$1"
  if command -v curl >/dev/null 2>&1; then
    curl -fsSL "$url"
    return
  fi
  if command -v wget >/dev/null 2>&1; then
    wget -q -O - "$url"
    return
  fi
  die "curl or wget is required"
}

download_file() {
  local url="$1"
  local output="$2"
  if command -v curl >/dev/null 2>&1; then
    curl -fsSL "$url" -o "$output"
    return
  fi
  if command -v wget >/dev/null 2>&1; then
    wget -q -O "$output" "$url"
    return
  fi
  die "curl or wget is required"
}

normalize_version() {
  local raw="${1:-latest}"
  case "$raw" in
    "" | latest)
      printf 'latest\n'
      ;;
    rust-v*)
      printf '%s\n' "${raw#rust-v}"
      ;;
    v*)
      printf '%s\n' "${raw#v}"
      ;;
    *)
      printf '%s\n' "$raw"
      ;;
  esac
}

resolve_latest_version() {
  local release_json
  local resolved
  release_json="$(download_text "https://api.github.com/repos/${REPO_SLUG}/releases/latest")"
  resolved="$(printf '%s\n' "$release_json" | sed -n 's/.*"tag_name":[[:space:]]*"rust-v\([^"]*\)".*/\1/p' | head -n 1)"
  [ -n "$resolved" ] || die "Failed to resolve latest release for ${REPO_SLUG}"
  printf '%s\n' "$resolved"
}

resolved_version() {
  local normalized
  normalized="$(normalize_version "$VERSION")"
  if [ "$normalized" = "latest" ]; then
    resolve_latest_version
  else
    printf '%s\n' "$normalized"
  fi
}

release_url_for_asset() {
  local asset="$1"
  local version="$2"
  printf 'https://github.com/%s/releases/download/rust-v%s/%s\n' "$REPO_SLUG" "$version" "$asset"
}

detect_platform() {
  case "$(uname -s)" in
    Darwin)
      os="darwin"
      ;;
    Linux)
      os="linux"
      ;;
    *)
      die "install-codex.sh supports macOS and Linux"
      ;;
  esac

  case "$(uname -m)" in
    x86_64 | amd64)
      arch="x86_64"
      ;;
    arm64 | aarch64)
      arch="aarch64"
      ;;
    *)
      die "Unsupported architecture: $(uname -m)"
      ;;
  esac

  if [ "$os" = "darwin" ] && [ "$arch" = "x86_64" ] && [ "$(sysctl -n sysctl.proc_translated 2>/dev/null || true)" = "1" ]; then
    arch="aarch64"
  fi

  if [ "$os" = "darwin" ]; then
    if [ "$arch" = "aarch64" ]; then
      npm_tag="darwin-arm64"
      vendor_target="aarch64-apple-darwin"
    else
      npm_tag="darwin-x64"
      vendor_target="x86_64-apple-darwin"
    fi
  else
    if [ "$arch" = "aarch64" ]; then
      npm_tag="linux-arm64"
      vendor_target="aarch64-unknown-linux-musl"
    else
      npm_tag="linux-x64"
      vendor_target="x86_64-unknown-linux-musl"
    fi
  fi
}

show_status() {
  local mode="${1:-}"
  if [ "$mode" = "config" ]; then
    if [ -f "codex-rs/.cargo/config.toml" ]; then
      sed -n '1,200p' "codex-rs/.cargo/config.toml"
      return
    fi
    die "Repository config not found at codex-rs/.cargo/config.toml"
  fi

  local bin
  for bin in tt codex codex-app-server rg; do
    if command -v "$bin" >/dev/null 2>&1; then
      printf '%s\t%s\n' "$bin" "$(command -v "$bin")"
      "$bin" --version 2>/dev/null | head -n 1 || true
    else
      printf '%s\tnot found on PATH\n' "$bin"
    fi
  done
  printf 'install-dir\t%s\n' "$INSTALL_DIR"
}

install_config() {
  mkdir -p "$CONFIG_DIR"
  install -m 0644 codex-rs/.cargo/config.toml "$CONFIG_DIR/config.toml"
  printf 'Installed config to %s/config.toml\n' "$CONFIG_DIR"
}

install_release() {
  require_command mktemp
  require_command tar
  detect_platform

  local version
  local asset
  local url
  local tmp_dir
  local archive_path

  version="$(resolved_version)"
  asset="codex-npm-${npm_tag}-${version}.tgz"
  url="$(release_url_for_asset "$asset" "$version")"

  tmp_dir="$(mktemp -d)"
  trap 'rm -rf "$tmp_dir"' EXIT INT TERM
  archive_path="${tmp_dir}/${asset}"

  download_file "$url" "$archive_path"
  tar -xzf "$archive_path" -C "$tmp_dir"

  mkdir -p "$INSTALL_DIR"
  install -m 0755 "$tmp_dir/package/vendor/${vendor_target}/codex/codex" "$INSTALL_DIR/codex"
  if [ -f "$tmp_dir/package/vendor/${vendor_target}/path/rg" ]; then
    install -m 0755 "$tmp_dir/package/vendor/${vendor_target}/path/rg" "$INSTALL_DIR/rg"
  fi

  printf 'Installed codex %s to %s\n' "$version" "$INSTALL_DIR"
}

uninstall_bins() {
  local mode="${1:-}"
  local path
  for path in "$INSTALL_DIR/tt" "$INSTALL_DIR/codex" "$INSTALL_DIR/codex-app-server" "$INSTALL_DIR/rg"; do
    rm -f "$path"
  done
  if [ -n "$mode" ] && [ "$mode" != "all" ]; then
    printf 'Uninstall ignores version selector and removed current local binaries from %s\n' "$INSTALL_DIR"
  fi
}

main() {
  while [ "$#" -gt 0 ]; do
    case "$1" in
      --install-dir)
        [ "$#" -ge 2 ] || die "--install-dir requires a value"
        INSTALL_DIR="$2"
        shift 2
        ;;
      -h|--help)
        usage
        exit 0
        ;;
      *)
        break
        ;;
    esac
  done

  [ "$#" -gt 0 ] || {
    usage
    exit 1
  }

  local cmd="$1"
  shift

  case "$cmd" in
    latest)
      resolved_version
      ;;
    status)
      show_status "${1:-}"
      ;;
    install)
      VERSION="${1:-latest}"
      if [ "$VERSION" = "config" ]; then
        install_config
      else
        install_release
      fi
      ;;
    uninstall)
      uninstall_bins "${1:-}"
      ;;
    *)
      die "Unknown command: $cmd"
      ;;
  esac
}

main "$@"
