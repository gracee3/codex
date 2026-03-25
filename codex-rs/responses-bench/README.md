# codex-responses-bench

Benchmark the OpenAI-compatible `/v1/responses` endpoint with concurrent
streaming or non-streaming requests.

## Examples

Non-streaming benchmark:

```bash
cargo run -p codex-responses-bench -- \
  --model gpt-oss-20b \
  --base-url http://127.0.0.1:8000 \
  --max-concurrency 6
```

Streaming benchmark:

```bash
cargo run -p codex-responses-bench -- \
  --model gpt-oss-20b \
  --base-url http://127.0.0.1:8000 \
  --max-concurrency 6 \
  --stream
```

Compatibility smoke test:

```bash
cargo run -p codex-responses-bench -- \
  --model gpt-oss-20b \
  --base-url http://127.0.0.1:8000 \
  --compat-smoke
```
