# TT / Codex Runtime Contract

## Purpose

This document defines the runtime boundary for the TT workspace mode
implemented inside this Codex fork.

TT is a first-class binary built from the same workspace as `codex` and
`codex-app-server`. TT owns the workspace control plane and uses
`codex-app-server` as the shared thread and turn transport.

## Ownership

Codex owns:

- agent execution
- thread and turn lifecycle
- app-server websocket transport
- sandbox behavior
- workspace-local `.codex` state

TT owns:

- workspace creation and layout
- detached runtime lifecycle
- workspace-local `.tt` state
- supervisor and worker session binding
- structured Director/Developer dispatch and result routing
- auto-loop and pause semantics
- operator attach and detach workflow

## Workspace Model

TT operates on an explicit workspace root:

```text
<workspace>/
  .tt/
  .codex/
  primary/
  worktrees/
```

The current command entrypoint is:

- `tt clone <repo-url> [dir]`

This creates the workspace, clones the repo into `primary/`, and scaffolds TT
artifacts under `.tt/` and `.codex/`.

The detached runtime:

- starts one shared `codex-app-server` on loopback websocket
- creates or resumes the TT supervisor thread
- creates or resumes registered worker threads
- lets the supervisor thread inspect and steer workers through TT-specific tools
- persists runtime/orchestration state in `.codex/tt/state.json`
- appends operational events to `.codex/tt/log.ndjson`

Current workspace artifacts:

- `<workspace>/.tt/plan.md`
- `<workspace>/.tt/roster.md`
- `<workspace>/.tt/roles/supervisor.md`
- `<workspace>/.tt/roles/director.md`
- `<workspace>/.tt/roles/developer.md`
- `<workspace>/.tt/activate`
- `<workspace>/.codex/tt/state.json`
- `<workspace>/.codex/tt/log.ndjson`
- `<workspace>/.codex/tt/daemon.log`

## Public Command Surface

Current TT commands:

- `tt clone <repo-url> [dir]`
- `tt start`
- `tt stop`
- `tt open`
- `tt status`
- `tt worker add <name>`
- `tt worker list`
- `tt worker read <name> [--turns <n> | --all]`
- `tt worker send <name> --message "<text>"`
- `tt worker adopt <name> --thread-id <id>`
- `tt worker remove <name>`
- `tt worker attach <name>`
- `tt auto on|off`
- `tt pause`

Command semantics:

- `tt clone`
  - creates a new TT workspace and clones the repo into `primary/`
- `tt start`
  - starts the detached TT runtime if it is not already running
- `tt stop`
  - stops the detached runtime and clears runtime-running metadata
- `tt open`
  - attach-only
  - attaches the operator to the current default view
  - defaults to the supervisor view
  - fails if the detached runtime is not running
- `tt worker add <name>`
  - creates a new git worktree under `worktrees/<name>` and registers it as a
    TT worker
- `tt worker list`
  - shows worker name, kind, cwd, thread id, and live runtime status when the
    runtime is reachable
- `tt worker read <name> [--turns <n> | --all]`
  - requires a running runtime
  - prints worker metadata first, then a human-readable transcript
  - defaults to the last 10 turns
  - renders non-message thread items as compact summaries instead of raw JSON
- `tt worker send <name> --message "<text>"`
  - requires a running runtime
  - creates the worker thread on demand when needed
  - starts a new turn when the worker is idle
  - steers the active turn when the worker already has an in-progress steerable
    turn
  - fails cleanly when the active turn exists but is not steerable
- `tt worker adopt <name> --thread-id <id>`
  - requires a running runtime
  - validates the supplied thread id through `thread/read`
  - only allows adoption when the thread cwd is inside the current workspace
  - registers the worker as a generic `worker` with no instruction path
- `tt worker remove <name>`
  - unregisters the worker from TT state only
  - does not delete the worktree or archive the underlying thread
  - rejects preset worker removal for `director` and `developer`
- `tt worker attach <name>`
  - attaches to the requested worker via the daemon-owned websocket app-server
- `tt auto on|off`
  - toggles loop continuation for the Director/Developer preset
- `tt pause`
  - toggles operator pause without killing the runtime

## Supervisor Tool Surface

The supervisor thread exposes TT-only dynamic tools:

- `tt_worker_list`
- `tt_worker_read`
- `tt_worker_send`
- `tt_worker_remove`

These tools are available only inside the supervisor thread. Other workers keep
their normal Codex tool surface and do not receive TT control-plane tools.

The tool behavior mirrors the CLI behavior:

- `tt_worker_list` returns structured worker summaries
- `tt_worker_read` returns worker metadata and transcript output
- `tt_worker_send` starts or steers a worker turn, depending on live thread
  state
- `tt_worker_remove` unregisters a generic worker without deleting the thread

Interactive server requests that are not one of these supervisor TT tools are
still rejected by the daemon.

## Routing Contract

TT currently keeps the Director/Developer loop as a preset implemented on top of
the generic worker registry.

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

- `supervisor_thread_id`
- `workers[]`
  - `name`
  - `kind`
  - `cwd`
  - `thread_id`
  - `instruction_path`
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

Registered workers, including adopted workers, are stored in `workers[]` and
are resumed on the next `tt start` when their thread ids remain valid.

## Current Limitations

The current detached runtime is local-only and loopback-only.

Known constraints:

- `tt open` requires a running runtime; it does not start one implicitly
- worker checkouts must live inside the workspace
- TUI attachment is implemented through the existing Codex TUI remote websocket
  path
- malformed or missing dispatch/result envelopes block TT auto-advancement
