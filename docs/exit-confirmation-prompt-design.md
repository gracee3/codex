# Exit and shutdown flow (tui)

This document describes how exit, shutdown, and interruption work in the Rust TUI (`codex-rs/tui`).
It is intended for Codex developers and Codex itself when reasoning about future exit/shutdown
changes.

This doc replaces earlier separate history and design notes. High-level history is summarized
below; full details are captured in PR #8936.

## Terms

- **Exit**: end the UI event loop and terminate the process.
- **Shutdown**: request a graceful agent/core shutdown (`Op::Shutdown`) and wait for
  `ShutdownComplete` so cleanup can run.
- **Interrupt**: cancel a running operation (`Op::Interrupt`).

## Event model (AppEvent)

Exit is coordinated via a single event with explicit modes:

- `AppEvent::Exit(ExitMode::ShutdownFirst)`
  - Prefer this for user-initiated quits so cleanup runs.
- `AppEvent::Exit(ExitMode::Immediate)`
  - Escape hatch for immediate exit. This bypasses shutdown and can drop
    in-flight work (e.g., tasks, rollout flush, child process cleanup).

`App` is the coordinator: it submits `Op::Shutdown` and it exits the UI loop only when
`ExitMode::Immediate` arrives (typically after `ShutdownComplete`).

In repo-scoped project-server mode, immediate exit is also used as a deliberate
"detach the TUI, leave the shared app-server running" path.

## User-triggered quit flows

### Ctrl+C

Priority order in the UI layer:

1. Active modal/view gets the first chance to consume (`BottomPane::on_ctrl_c`).
   - If the modal handles it, the quit flow stops.
   - When a modal/popup handles Ctrl+C, the quit shortcut is cleared so dismissing a modal cannot
     accidentally prime a subsequent Ctrl+C to quit.
2. If the user has already armed Ctrl+C and the 1 second window has not expired, the second Ctrl+C
   triggers the configured quit path immediately.
3. Otherwise, `ChatWidget` arms Ctrl+C and shows the quit hint (`ctrl + c again to quit`) for
   1 second.
4. If cancellable work is active (streaming/tools/review), `ChatWidget` submits `Op::Interrupt`.

The configured quit path is mode-dependent:

- Default embedded/explicit-remote mode: shutdown-first quit.
- Repo-scoped project-server mode: immediate exit on the first Ctrl+C, which detaches the UI and
  leaves the shared app-server process and in-flight work running.

### Ctrl+D

- Only participates in quit when the composer is empty **and** no modal is active.
  - On first press, show the quit hint (same as Ctrl+C) and start the 1 second timer.
  - If pressed again while the hint is visible, request the same mode-dependent quit path as
    Ctrl+C.
- With any modal/popup open, key events are routed to the view and Ctrl+D does not attempt to
  quit.

### Slash commands

- `/quit`, `/exit`, `/logout` request shutdown-first quit **without** a prompt,
  because slash commands are harder to trigger accidentally and imply clear intent to quit.

### /new

- Uses shutdown without exit (suppresses `ShutdownComplete`) so the app can
  start a fresh session without terminating.

## Shutdown completion and suppression

`ShutdownComplete` is the signal that core cleanup has finished. The UI treats it as the boundary
for exit:

- `ChatWidget` requests `Exit(Immediate)` on `ShutdownComplete`.
- `App` can suppress a single `ShutdownComplete` when shutdown is used as a
  cleanup step (e.g., `/new`).

## Edge cases and invariants

- **Review mode** counts as cancellable work. Ctrl+C should interrupt review, not
  quit.
- **Modal open** means Ctrl+C/Ctrl+D should not quit unless the modal explicitly
  declines to handle Ctrl+C.
- **Immediate exit** is not a normal user path; it is a fallback for shutdown
  completion or an emergency exit. Use it sparingly because it skips cleanup.

## Testing expectations

At a minimum, we want coverage for:

- Ctrl+C while working interrupts, does not quit.
- Ctrl+C while idle and empty shows quit hint, then shutdown-first quit on second press.
- In project-server mode, Ctrl+C detaches immediately and does not interrupt active work.
- Ctrl+D with modal open does not quit.
- `/quit` / `/exit` / `/logout` quit without prompt, but still shutdown-first.
  - Ctrl+D while idle and empty shows quit hint, then shutdown-first quit on second press.

## History (high level)

Codex has historically mixed "exit immediately" and "shutdown-first" across quit gestures, largely
due to incremental changes and regressions in state tracking. The current default remains
shutdown-first, with project-server detach as the explicit exception. See PR #8936 for the
detailed history and rationale.
