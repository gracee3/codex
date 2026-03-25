# GPT-OSS Codex Integration Plan

This document tracks the Codex-side work needed to make local `gpt-oss-20b`
on vLLM usable through the Responses API.

The goal is to keep changes on the Codex side whenever possible and use
`codex-responses-bench` as the live compatibility harness for provider behavior.

## Current Validated Behavior

Validated against local vLLM `gpt-oss-20b` on `http://127.0.0.1:8000/v1/responses`.

Working:

- `function` tool calls in non-streaming mode
- `function` tool calls in streaming mode with non-empty arguments
- stateless continuation using prior `function_call` plus `function_call_output`
- `developer` role messages
- `system` role messages

Broken or unsupported:

- streamed zero-argument `function` call shape is not stable
- `previous_response_id` continuation returns `404`
- `web_search`
- `image_generation`
- `custom`
- `local_shell`
- `tool_search`
- built-in `apply_patch`
- built-in `shell`
- `custom_tool_call_output`
- `tool_search_output`

## Main Gaps To Close In Codex

1. Restrict the exposed tool surface for local GPT-OSS to the subset vLLM
   actually supports.
2. Avoid websocket/incremental Responses behavior that depends on
   `previous_response_id`.
3. Use stateless continuation for tool follow-up turns.
4. Normalize malformed streamed zero-argument function-call payloads only if
   they continue to reproduce reliably.
5. Keep all behavior narrowly scoped to local GPT-OSS providers so upstream
   OpenAI-compatible providers remain unchanged.

## Implementation Plan

### 1. Restrict Tool Surface

For local `gpt-oss-20b`:

- expose only plain `function` tools
- keep `apply_patch` mapped to `function`
- map shell-style tools through `function`, or hide them if model behavior is
  poor
- drop unsupported tool types from every request path, not just the main
  streaming path

### 2. Force Plain HTTP Responses Transport

For local `gpt-oss-20b`:

- do not use websocket Responses transport
- do not rely on server-side incremental response state
- keep the session on the HTTP Responses path

This should avoid the broken `previous_response_id` continuation path entirely.

### 3. Use Stateless Tool Continuation

For follow-up turns after a tool call:

- resend prior user/assistant context
- resend the prior `function_call`
- append the new `function_call_output`
- do not rely on stored response state in vLLM

### 4. Normalize Streamed Zero-Arg Tool Calls

If the malformed zero-argument shape continues to reproduce:

- add a very small GPT-OSS-specific normalization step
- rewrite malformed empty-object payloads back to `{}`
- do not touch valid argument payloads

This should stay provider-scoped and minimal.

### 5. Keep Bench Coverage As The Live Harness

Use `codex-responses-bench` to verify:

- supported tool types
- supported output item types
- stateless continuation behavior
- `previous_response_id` behavior
- streamed argument-shape correctness

## Suggested Order

1. Finish Codex transport/tool sanitization for local GPT-OSS.
2. Add stateless continuation in Codex for local GPT-OSS tool loops.
3. Add streamed zero-arg normalization only if still reproducible.
4. Revalidate with:
   - `cargo run -p codex-responses-bench -- --compat-smoke ...`
   - a live Codex file-edit turn against local vLLM

## Likely Codex Touch Points

- `codex-rs/core/src/client.rs`
  - transport selection
  - request sanitization
  - continuation request shaping
- `codex-rs/core/src/models_manager/model_info.rs`
  - tool-type overrides for local GPT-OSS
- `codex-rs/core/models.json`
  - built-in model defaults
- `codex-rs/core/src/client_tests.rs`
  - provider-specific request-shaping coverage

## Bench Commands

Compatibility smoke:

```bash
cargo run -p codex-responses-bench -- \
  --model gpt-oss-20b \
  --base-url http://127.0.0.1:8000 \
  --api-key local \
  --compat-smoke
```

Short non-stream benchmark:

```bash
cargo run -p codex-responses-bench -- \
  --model gpt-oss-20b \
  --base-url http://127.0.0.1:8000 \
  --api-key local \
  --num-prompts 20 \
  --max-concurrency 4 \
  --input-tokens 1024 \
  --max-output-tokens 128
```

Short stream benchmark:

```bash
cargo run -p codex-responses-bench -- \
  --model gpt-oss-20b \
  --base-url http://127.0.0.1:8000 \
  --api-key local \
  --num-prompts 20 \
  --max-concurrency 4 \
  --input-tokens 1024 \
  --max-output-tokens 128 \
  --stream
```
