# codex-session-context

This crate keeps the Linux TT fork's lightweight tracing/session helpers:

- `SessionTelemetry` for session-scoped event/log emission and local metrics hooks
- `TelemetryAuthMode` for auth-mode metadata without a `codex-core` dependency
- W3C trace-context helpers re-exported from `codex-trace-context`

What it does not do:

- no OTLP exporter wiring
- no OpenTelemetry provider setup
- no runtime metrics summary surface

The supported path in this fork is local Rust tracing plus trace-context propagation.
