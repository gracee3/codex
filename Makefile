# Local GPT-OSS 20B via vLLM

# ── Images ──────────────────────────────────────────────
IMAGE ?= vllm/vllm-openai:v0.18.0

# ── Model / paths ───────────────────────────────────────
MODEL_PATH ?= /data/models/openai/gpt-oss-20b
CACHE_PATH ?= $(abspath $(MODEL_PATH)/../.cache/vllm)
API_KEY ?= local

# ── Networking / ports ──────────────────────────────────
PORT ?= 8000
DOCKER_NETWORK ?= local-inference-net
DOCKER_RESTART_POLICY ?= no
DOCKER_PULL_POLICY ?= never

# ── Container names ─────────────────────────────────────
LLM_CONTAINER ?= vllm-gpt-oss-20b
CONTAINERS := $(LLM_CONTAINER)

# ── GPU pinning ─────────────────────────────────────────
# GPU=0, GPU=1, or GPU=all
GPU ?= 1

ifeq ($(GPU),all)
  GPU_FLAG := --gpus all
else
  GPU_FLAG := --gpus '"device=$(GPU)"'
endif

# ── vLLM defaults ───────────────────────────────────────
# Overridden by the run-gpt-* profile targets below.
GPU_MEM_UTIL ?= 0.92
MAX_MODEL_LEN ?= 16384
TP_SIZE ?= 1
MAX_NUM_SEQS ?= 4
MAX_NUM_BATCHED_TOKENS ?= 8192
TOOL_CALL_PARSER ?= openai
REASONING_PARSER ?= openai_gptoss
EXTRA_ARGS ?=
SERVED_MODEL_NAME ?= gpt-oss-20b

# ── Settings ─────────────────────────────────

.PHONY: check-llm-prereqs \
	run-llm run-gpt-balanced run-gpt-parallel run-gpt-32k \
	run-gpt-dual-fastest run-gpt-dual-fast run-gpt-dual-long run-gpt-dual-longest \
	stop stop-all stop-llm \
	logs logs-llm \
	healthcheck healthcheck-llm \
	models shell clean-llm clean-llm-all \
	smoke-single smoke-batch smoke \
	bench-llm bench-llm-4 bench-llm-6 bench-llm-8 \
	bench-llm-light bench-llm-medium bench-llm-heavy

check-llm-prereqs:
	@command -v docker >/dev/null 2>&1 || { echo "docker is required"; exit 1; }
	@command -v curl >/dev/null 2>&1 || { echo "curl is required"; exit 1; }
	@command -v python3 >/dev/null 2>&1 || { echo "python3 is required"; exit 1; }
	@test -e "$(MODEL_PATH)" || { echo "MODEL_PATH does not exist: $(MODEL_PATH)"; exit 1; }

# ── Run: vLLM server ────────────────────────────────────
run-llm: check-llm-prereqs stop-llm
	docker network inspect $(DOCKER_NETWORK) >/dev/null 2>&1 || docker network create $(DOCKER_NETWORK)
	mkdir -p $(CACHE_PATH)
	docker run -d \
		--name $(LLM_CONTAINER) \
		--network $(DOCKER_NETWORK) \
		$(GPU_FLAG) \
		--ipc=host \
		-p $(PORT):8000 \
		-v $(MODEL_PATH):/model:ro \
		-v $(CACHE_PATH):/root/.cache \
		--restart $(DOCKER_RESTART_POLICY) \
		--pull $(DOCKER_PULL_POLICY) \
		$(IMAGE) \
		/model \
		--served-model-name $(SERVED_MODEL_NAME) \
		--api-key $(API_KEY) \
		--host 0.0.0.0 \
		--port 8000 \
		--gpu-memory-utilization $(GPU_MEM_UTIL) \
		--max-model-len $(MAX_MODEL_LEN) \
		--tensor-parallel-size $(TP_SIZE) \
		--enable-prefix-caching \
		--tool-call-parser $(TOOL_CALL_PARSER) \
		--reasoning-parser $(REASONING_PARSER) \
		--max-num-seqs $(MAX_NUM_SEQS) \
		--max-num-batched-tokens $(MAX_NUM_BATCHED_TOKENS) \
		$(EXTRA_ARGS)
	docker logs -f $(LLM_CONTAINER)


# ── GPT profiles ────────────────────────────────────────
# Good everyday single-3090 default
run-gpt-balanced:
	$(MAKE) run-llm \
		GPU=1 \
		TP_SIZE=1 \
		GPU_MEM_UTIL=0.94 \
		MAX_MODEL_LEN=16384 \
		MAX_NUM_SEQS=6 \
		MAX_NUM_BATCHED_TOKENS=8192

run-gpt-32k:
	$(MAKE) run-llm \
		GPU=1 \
		TP_SIZE=1 \
		GPU_MEM_UTIL=0.94 \
		MAX_MODEL_LEN=32768 \
		MAX_NUM_SEQS=6 \
		MAX_NUM_BATCHED_TOKENS=8192

run-gpt-dual-fastest:
	$(MAKE) run-llm \
		GPU=all \
		TP_SIZE=2 \
		GPU_MEM_UTIL=0.88 \
		MAX_MODEL_LEN=16384 \
		MAX_NUM_SEQS=4 \
		MAX_NUM_BATCHED_TOKENS=4096

run-gpt-dual-fast:
	$(MAKE) run-llm \
		GPU=all \
		TP_SIZE=2 \
		GPU_MEM_UTIL=0.88 \
		MAX_MODEL_LEN=16384 \
		MAX_NUM_SEQS=6 \
		MAX_NUM_BATCHED_TOKENS=4096

run-gpt-dual-long:
	$(MAKE) run-llm \
		GPU=all \
		TP_SIZE=2 \
		GPU_MEM_UTIL=0.92 \
		MAX_MODEL_LEN=65536 \
		MAX_NUM_SEQS=1 \
		MAX_NUM_BATCHED_TOKENS=8192

run-gpt-dual-longest:
	$(MAKE) run-llm \
		GPU=all \
		TP_SIZE=2 \
		GPU_MEM_UTIL=0.94 \
		MAX_MODEL_LEN=98304 \
		MAX_NUM_SEQS=1 \
		MAX_NUM_BATCHED_TOKENS=8192

# ── Stop ────────────────────────────────────────────────
stop: stop-llm

stop-all:
	@for c in $(CONTAINERS); do \
		docker stop $$c 2>/dev/null || true; \
		docker rm $$c 2>/dev/null || true; \
	done
	@echo "All containers stopped"

stop-llm:
	docker stop $(LLM_CONTAINER) 2>/dev/null || true
	docker rm $(LLM_CONTAINER) 2>/dev/null || true

# ── Logs ────────────────────────────────────────────────
logs:
	@for c in $(CONTAINERS); do \
		if docker ps --format '{{.Names}}' | grep -q "^$$c$$"; then \
			docker logs -f $$c; \
			exit 0; \
		fi; \
	done; \
	echo "No running container found"

logs-llm:
	docker logs -f $(LLM_CONTAINER)

# ── Healthchecks ────────────────────────────────────────
healthcheck: healthcheck-llm

healthcheck-llm:
	@curl -sf http://localhost:$(PORT)/health && echo "healthy" || echo "unhealthy"

models:
	@curl -sf -H "Authorization: Bearer $(API_KEY)" http://localhost:$(PORT)/v1/models | python3 -m json.tool

shell:
	docker exec -it $(LLM_CONTAINER) /bin/bash

smoke-single:
	@curl -sf \
		-H "Authorization: Bearer $(API_KEY)" \
		-H "Content-Type: application/json" \
		http://localhost:$(PORT)/v1/chat/completions \
		-d '{"model":"$(SERVED_MODEL_NAME)","messages":[{"role":"user","content":"Reply with exactly READY"}]}' | python3 -m json.tool

smoke-batch:
	@curl -sf \
		-H "Authorization: Bearer $(API_KEY)" \
		-H "Content-Type: application/json" \
		http://localhost:$(PORT)/v1/chat/completions \
		-d '{"model":"$(SERVED_MODEL_NAME)","n":2,"messages":[{"role":"user","content":"Reply with exactly READY"}]}' | python3 -m json.tool

smoke: smoke-single


# ── Bench: vLLM server throughput ───────────────────────

BENCH_IMAGE ?= $(IMAGE)
BENCH_BASE_URL ?= http://$(LLM_CONTAINER):8000
BENCH_ENDPOINT ?= /v1/chat/completions
BENCH_BACKEND ?= openai-chat
BENCH_MODEL ?= $(SERVED_MODEL_NAME)
BENCH_TOKENIZER ?= /model
BENCH_READY_TIMEOUT ?= 600
BENCH_API_KEY ?= $(API_KEY)

BENCH_NUM_PROMPTS ?= 200
BENCH_MAX_CONCURRENCY ?= 6
BENCH_REQUEST_RATE ?= inf
BENCH_DATASET ?= random
BENCH_INPUT_LEN ?= 4096
BENCH_OUTPUT_LEN ?= 512
BENCH_EXTRA_ARGS ?=

bench-llm:
	docker network inspect $(DOCKER_NETWORK) >/dev/null 2>&1 || docker network create $(DOCKER_NETWORK)
	docker run --rm \
		--network $(DOCKER_NETWORK) \
		$(GPU_FLAG) \
		--entrypoint vllm \
		-e OPENAI_API_KEY=$(BENCH_API_KEY) \
		-v $(MODEL_PATH):/model:ro \
		-v $(CACHE_PATH):/root/.cache \
		$(BENCH_IMAGE) \
		bench serve \
		--backend $(BENCH_BACKEND) \
		--base-url $(BENCH_BASE_URL) \
		--endpoint $(BENCH_ENDPOINT) \
		--model $(BENCH_MODEL) \
		--tokenizer $(BENCH_TOKENIZER) \
		--dataset-name $(BENCH_DATASET) \
		--num-prompts $(BENCH_NUM_PROMPTS) \
		--max-concurrency $(BENCH_MAX_CONCURRENCY) \
		--request-rate $(BENCH_REQUEST_RATE) \
		--random-input-len $(BENCH_INPUT_LEN) \
		--random-output-len $(BENCH_OUTPUT_LEN) \
		--ready-check-timeout-sec $(BENCH_READY_TIMEOUT) \
		$(BENCH_EXTRA_ARGS)

bench-llm-4:
	$(MAKE) bench-llm BENCH_MAX_CONCURRENCY=4

bench-llm-6:
	$(MAKE) bench-llm BENCH_MAX_CONCURRENCY=6

bench-llm-8:
	$(MAKE) bench-llm BENCH_MAX_CONCURRENCY=8

bench-llm-light:
	$(MAKE) bench-llm BENCH_INPUT_LEN=1024 BENCH_OUTPUT_LEN=256 BENCH_MAX_CONCURRENCY=6

bench-llm-medium:
	$(MAKE) bench-llm BENCH_INPUT_LEN=4096 BENCH_OUTPUT_LEN=512 BENCH_MAX_CONCURRENCY=6

bench-llm-heavy:
	$(MAKE) bench-llm BENCH_INPUT_LEN=8192 BENCH_OUTPUT_LEN=768 BENCH_MAX_CONCURRENCY=6



# ── Clean ───────────────────────────────────────────────
clean-llm: stop-llm
	docker rmi $(IMAGE) 2>/dev/null || true

clean-llm-all: stop-all
	docker rmi $(IMAGE) 2>/dev/null || true
