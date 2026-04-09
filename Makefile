SHELL := /bin/bash

FORK_SCRIPT := ./scripts/fork-release.sh

.PHONY: sync-main new-release new-alpha-release list-release-tags fork-release-ci fork-release-ci-docker

sync-main:
	$(FORK_SCRIPT) sync-main $(if $(PUSH),--push,)

new-release:
	$(FORK_SCRIPT) new-release $(if $(TAG),--tag $(TAG),) $(if $(PUSH),--push,)

new-alpha-release:
	$(FORK_SCRIPT) new-release --alpha $(if $(TAG),--tag $(TAG),) $(if $(PUSH),--push,)

list-release-tags:
	$(FORK_SCRIPT) list-tags

fork-release-ci:
	./scripts/run-fork-release-ci.sh

fork-release-ci-docker:
	./scripts/run-fork-release-ci-docker.sh
