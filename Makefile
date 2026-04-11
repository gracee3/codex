SHELL := /bin/bash

FORK_SCRIPT := ./scripts/fork-release.sh
INSTALL_SCRIPT := ./scripts/install-codex.sh
INSTALL_DIR ?= $(HOME)/.local/bin
UNINSTALL_VERSION ?=

.DEFAULT_GOAL := help

.PHONY: help latest-upstream-tag build build-release clean install install-release uninstall
.PHONY: sync-main new-release new-tt-release list-patch-commits

help:
	@echo "codex fork maintenance"
	@echo ""
	@echo "Workflow:"
	@echo "  # Start on fork/maint, sync main from upstream, create the release branch, then build/install locally"
	@echo "  git checkout fork/maint"
	@echo "  make sync-main"
	@echo "  make new-release"
	@echo "  make build"
	@echo "  make install"
	@echo ""
	@echo "Release maintenance:"
	@echo "  sync-main              Update local main to upstream/main"
	@echo "  new-release            Create TT release branch from latest stable tag"
	@echo "  new-tt-release         Same as new-release"
	@echo "  list-patch-commits     Show the patch commits that will be cherry-picked"
	@echo ""
	@echo "Install helpers:"
	@echo "  latest-upstream-tag    Print latest stable rust-v tag from upstream"
	@echo "  clean                  Remove local Cargo build artifacts"
	@echo "  build                  Build local debug artifacts"
	@echo "  build-release          Build local release artifacts"
	@echo "  install                Build local debug artifacts and install them"
	@echo "  install-release        Build local release artifacts and install them"
	@echo "  uninstall             Uninstall local binaries from $(INSTALL_DIR)"
	@echo ""
	@echo "Variables:"
	@echo "  TAG               Explicit rust-v tag for release creation"
	@echo "  RELEASE_SUFFIX    Optional suffix appended to the release branch name"
	@echo "  PUSH              Set to 1 to push created or synced branches"
	@echo "  INSTALL_DIR       Install directory override"
	@echo ""
	@echo "Examples:"
	@echo "  make new-release TAG=rust-v0.120.0"
	@echo "  make new-release TAG=rust-v0.120.0 RELEASE_SUFFIX=1"
	@echo "  make latest-upstream-tag # rust-v0.120.0"

sync-main:
	$(FORK_SCRIPT) sync-main $(if $(PUSH),--push,)

new-release:
	PATCH_BRANCHES="fork/maint fork/dev-build-speedups fork/tt-runtime-contract fork/app-server-rollout" RELEASE_BRANCH_PREFIX="releases/tt/" RELEASE_SUFFIX="$(RELEASE_SUFFIX)" $(FORK_SCRIPT) new-release $(if $(TAG),--tag $(TAG),) $(if $(PUSH),--push,)

new-tt-release:
	PATCH_BRANCHES="fork/maint fork/dev-build-speedups fork/tt-runtime-contract fork/app-server-rollout" RELEASE_BRANCH_PREFIX="releases/tt/" RELEASE_SUFFIX="$(RELEASE_SUFFIX)" $(FORK_SCRIPT) new-release $(if $(TAG),--tag $(TAG),) $(if $(PUSH),--push,)

list-patch-commits:
	$(FORK_SCRIPT) list-patch-commits

latest-upstream-tag:
	@tag="$$(git ls-remote --refs --tags upstream 'rust-v*' | awk '{print $$2}' | sed -E 's#refs/tags/##' | grep -E '^rust-v[0-9]+(\.[0-9]+){2}$$' | sort -V | tail -n 1)"; \
	[ -n "$$tag" ] || { echo "Failed to resolve latest stable tag from upstream" >&2; exit 1; }; \
	printf '%s\n' "$$tag"

clean:
	@cd codex-rs && cargo clean

build:
	@cd codex-rs && cargo build -p codex-cli --bin codex -p codex-app-server --bin codex-app-server

build-release:
	@cd codex-rs && cargo build --release -p codex-cli --bin codex -p codex-app-server --bin codex-app-server

install: build
	@mkdir -p "$(INSTALL_DIR)"
	@install -m 0755 codex-rs/target/debug/codex "$(INSTALL_DIR)/codex"
	@install -m 0755 codex-rs/target/debug/codex-app-server "$(INSTALL_DIR)/codex-app-server"

install-release: build-release
	@mkdir -p "$(INSTALL_DIR)"
	@install -m 0755 codex-rs/target/release/codex "$(INSTALL_DIR)/codex"
	@install -m 0755 codex-rs/target/release/codex-app-server "$(INSTALL_DIR)/codex-app-server"

uninstall:
	@$(INSTALL_SCRIPT) --install-dir $(INSTALL_DIR) uninstall $(UNINSTALL_VERSION)
