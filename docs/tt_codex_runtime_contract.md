# TT / Codex Runtime Contract

## Purpose

This document defines the runtime boundary for the TT product mode implemented
inside this Codex fork.

TT is no longer treated as an external orchestrator that discovers an arbitrary
Codex installation. In this fork, `tt` is a first-class binary built from the
same workspace as `codex` and `codex-app-server`.

The current contract is:

- `tt` owns orchestration, detached runtime lifecycle, and `.tt/` state
- `codex-app-server` remains the execution transport and thread/turn engine
- `codex` remains the generic interactive product

## Ownership

Codex owns:

- agent execution
- thread and turn lifecycle
- app-server websocket transport
- sandbox behavior
- `.codex` user and project state

TT owns:

- detached runtime lifecycle
- `.tt` project state
- Director and Developer session binding
- structured dispatch/result routing
- auto-loop and pause semantics
- operator attach/detach workflow

## Binaries

The fork now builds and installs:

- `codex`
- `codex-app-server`
- `tt`

There is no separate `tt-daemon` or `tt-tui` binary in v1. The detached TT
runtime is implemented as a hidden daemon mode inside `tt`, and TUI attachment
reuses the existing Codex TUI in remote mode.

## TT Runtime Model

TT uses a detached runtime with two long-lived role sessions:

- Director
- Developer

The detached runtime:

- starts `codex-app-server` on loopback websocket
- creates or resumes the TT Director and Developer threads
- owns turn routing between those threads
- persists runtime/orchestration state in `.tt/state.json`
- appends operational events to `.tt/log.ndjson`

Current TT project artifacts:

- `<repo>/.tt/plan.md`
- `<repo>/.tt/roster.md`
- `<repo>/.tt/state.json`
- `<repo>/.tt/log.ndjson`
- `<repo>/.tt/roles/director.md`
- `<repo>/.tt/roles/developer.md`

## Public Command Surface

Current TT commands:

- `tt init`
- `tt start`
- `tt stop`
- `tt open`
- `tt status`
- `tt attach director`
- `tt attach developer`
- `tt auto on`
- `tt auto off`
- `tt pause`

Command semantics:

- `tt start`
  - starts the detached TT runtime if it is not already running
- `tt stop`
  - stops the detached runtime and clears runtime-running metadata
- `tt open`
  - attach-only
  - attaches the operator to Director
  - fails if the detached runtime is not running
- `tt attach director|developer`
  - attaches to the requested role via the daemon-owned websocket app-server
- `tt auto on|off`
  - toggles loop continuation
- `tt pause`
  - toggles operator pause without killing the runtime

## Routing Contract

TT mediates all Director/Developer communication.

Director output must contain a plain-text dispatch envelope:

```text
[DISPATCH]
Dispatch-ID: <id>
Objective: ...
Scope: ...
Constraints: ...
Expected-Output: ...
Completion-Criteria: ...
Priority: ...
Plan-Refs: ...
Todo-Refs: ...
[/DISPATCH]
```

Developer output must contain a plain-text result envelope:

```text
[RESULT]
Dispatch-ID: <id>
Status: done|partial|blocked|failed
Summary: ...
Work-Completed: ...
Artifacts: ...
Open-Questions: ...
Recommended-Next-Step: ...
[/RESULT]
```

TT extracts the latest completed envelope from the latest assistant message in
the corresponding thread after `turn/completed`.

## State Contract

The TT runtime currently persists at least:

- `director_thread_id`
- `developer_thread_id`
- `runtime_running`
- `runtime_pid`
- `runtime_websocket_url`
- `runtime_auth_token`
- `last_runtime_started_at`
- `auto_loop`
- `operator_pause`
- `default_view`
- `active_dispatch_id`
- `pending_director_evaluation`
- `pending_developer_dispatch`

This is the inspectable control plane for TT.

## Current Limitations

The current detached runtime is local-only and loopback-only.

Known constraints:

- `tt open` requires a running runtime; it does not start one implicitly
- TUI attachment is implemented through the existing Codex TUI remote websocket
  path
- malformed or missing dispatch/result envelopes block TT auto-advancement
- TT runtime metadata currently lives directly in `.tt/state.json`

## Recommended Next Work

1. Add integration tests for daemon lifecycle and remote TUI attach
2. Add manual and CI smoke coverage for `tt start/status/stop`
3. Improve loop-blocked diagnostics and operator recovery flows
