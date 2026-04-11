SHELL := /bin/bash

FORK_SCRIPT := ./scripts/fork-release.sh
INSTALL_SCRIPT := ./scripts/install-codex.sh
SOURCE_INSTALL_SCRIPT := ./scripts/source-install.sh
INSTALL_DIR ?= $(HOME)/.local/bin
SOURCE_DIR ?= $(HOME)/git/codex
VERSION ?= latest
UNINSTALL_VERSION ?=

.DEFAULT_GOAL := help

.PHONY: help latest status install uninstall source-install tt-source-install lint config
.PHONY: sync-main new-release new-alpha-release new-tt-release new-tt-alpha-release list-release-tags
.PHONY: fork-release-ci fork-release-ci-docker

ifeq ($(filter config,$(MAKECMDGOALS)),config)
INSTALL_CMD_ARG = config
STATUS_CMD_ARG = config
else
INSTALL_CMD_ARG = $(VERSION)
STATUS_CMD_ARG =
endif

help:
	@echo "codex fork maintenance"
	@echo ""
	@echo "Release maintenance:"
	@echo "  sync-main              Sync local main to upstream/main"
	@echo "  new-release            Create release branch from latest stable tag"
	@echo "  new-alpha-release      Create release branch from latest alpha tag"
	@echo "  new-tt-release         Create TT release branch from latest stable tag"
	@echo "  new-tt-alpha-release   Create TT release branch from latest alpha tag"
	@echo "  list-release-tags      List available rust-v tags"
	@echo ""
	@echo "Install helpers:"
	@echo "  latest                 Fetch and print latest codex version"
	@echo "  status                 Show codex executables on PATH and install dir"
	@echo "  status config          Show the repository cargo config.toml"
	@echo "  install                Install released codex to $(INSTALL_DIR)"
	@echo "  source-install         Sync $(SOURCE_DIR) and install codex + codex-app-server"
	@echo "  tt-source-install      Same as source-install, but include the TT overlay branch"
	@echo "  uninstall             Uninstall local binaries from $(INSTALL_DIR)"
	@echo ""
	@echo "CI helpers:"
	@echo "  fork-release-ci        Run release-branch checks locally"
	@echo "  fork-release-ci-docker Run release-branch checks in Docker"
	@echo ""
	@echo "Variables:"
	@echo "  VERSION           Version to install or use as tag selector (default: latest)"
	@echo "  TAG               Explicit rust-v tag for release creation"
	@echo "  PUSH              Set to 1 to push created or synced branches"
	@echo "  INSTALL_DIR       Install directory override"
	@echo "  SOURCE_DIR        Source checkout directory override"
	@echo ""
	@echo "Examples:"
	@echo "  make new-release"
	@echo "  make new-release TAG=rust-v0.118.0 PUSH=1"
	@echo "  make new-tt-release TAG=rust-v0.119.0-alpha.1"
	@echo "  make source-install SOURCE_DIR=~/git/codex VERSION=0.118.0"

sync-main:
	$(FORK_SCRIPT) sync-main $(if $(PUSH),--push,)

new-release:
	$(FORK_SCRIPT) new-release $(if $(TAG),--tag $(TAG),) $(if $(PUSH),--push,)

new-alpha-release:
	$(FORK_SCRIPT) new-release --alpha $(if $(TAG),--tag $(TAG),) $(if $(PUSH),--push,)

new-tt-release:
	PATCH_BRANCHES="fork/maint fork/dev-build-speedups fork/tt-runtime-contract" RELEASE_BRANCH_PREFIX="releases/tt/" $(FORK_SCRIPT) new-release $(if $(TAG),--tag $(TAG),) $(if $(PUSH),--push,)

new-tt-alpha-release:
	PATCH_BRANCHES="fork/maint fork/dev-build-speedups fork/tt-runtime-contract" RELEASE_BRANCH_PREFIX="releases/tt/" $(FORK_SCRIPT) new-release --alpha $(if $(TAG),--tag $(TAG),) $(if $(PUSH),--push,)

list-release-tags:
	$(FORK_SCRIPT) list-tags

latest:
	@$(INSTALL_SCRIPT) latest

status:
	@$(INSTALL_SCRIPT) --install-dir $(INSTALL_DIR) status $(STATUS_CMD_ARG)

install:
	@$(INSTALL_SCRIPT) --install-dir $(INSTALL_DIR) install $(INSTALL_CMD_ARG)

uninstall:
	@$(INSTALL_SCRIPT) --install-dir $(INSTALL_DIR) uninstall $(UNINSTALL_VERSION)

source-install:
	@"$(SOURCE_INSTALL_SCRIPT)" --dir "$(SOURCE_DIR)" --install-dir "$(INSTALL_DIR)" --tag "$(VERSION)"

tt-source-install:
	@CODEX_SOURCE_PATCH_BRANCHES="fork/maint fork/dev-build-speedups fork/tt-runtime-contract" CODEX_SOURCE_RELEASE_BRANCH_PREFIX="releases/tt/" "$(SOURCE_INSTALL_SCRIPT)" --dir "$(SOURCE_DIR)" --install-dir "$(INSTALL_DIR)" --tag "$(VERSION)"

lint:
	@bash -n $(INSTALL_SCRIPT) $(SOURCE_INSTALL_SCRIPT) $(FORK_SCRIPT) scripts/run-fork-release-ci.sh scripts/run-fork-release-ci-docker.sh

config:
	@:

fork-release-ci:
	./scripts/run-fork-release-ci.sh

fork-release-ci-docker:
	./scripts/run-fork-release-ci-docker.sh
