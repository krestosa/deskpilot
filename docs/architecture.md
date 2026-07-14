# DeskPilot architecture

## Runtime shape

DeskPilot is one native Rust process. It has no service, browser runtime, updater, network client or injected code. Double-clicking `DeskPilot.exe` starts tray mode without a console. A terminal invocation attaches to the parent console and either performs a local command or sends a request to the running tray instance.

The process uses six bounded execution contexts:

1. The application coordinator owns configuration, backend mutation, reconciliation and shutdown.
2. The tray thread owns its hidden Win32 window and notification icon.
3. The low-level mouse-hook thread owns `WH_MOUSE_LL` and performs only gesture classification and queue submission.
4. The named-pipe acceptor creates short-lived client handlers; command execution remains serialized by the application coordinator.
5. The `winvd` notification worker receives virtual-desktop events and forwards a coalesced signal.
6. The out-of-context WinEvent thread observes top-level window create, destroy, show and hide events and forwards only a coalesced reconcile signal.

No general asynchronous runtime is used. Channels are from `std::sync::mpsc`; the main coordinator sleeps on `recv_timeout` until an event, reconcile deadline or watchdog deadline.

## Components

- `src/main.rs`: GUI/CLI entrypoint, console attachment, local commands and exit codes.
- `src/app.rs`: lifecycle, single-instance mutex, coordinator loop, command dispatch and orderly shutdown.
- `src/tray.rs`: native notification icon and menu.
- `src/windows/hooks.rs`: global wheel hook and physical left/right Windows-key state.
- `src/windows/window_events.rs`: out-of-context top-level window lifecycle notifications.
- `src/windows/desktops.rs`: narrow adapter over `winvd`.
- `src/windows/inventory.rs`: conservative top-level-window eligibility and desktop occupancy.
- `src/reconciliation/`: pure state model, planner and bounded executor.
- `src/config.rs`: strict TOML schema and atomic persistence.
- `src/ipc.rs`: per-user named pipe with an ACL for the current user and SYSTEM.
- `src/logging.rs`: bounded rolling log plus a small recent-error ring buffer.
- `src/diagnostics.rs` and `src/support.rs`: redacted diagnostics and support ZIP.

## Message flow

```text
WH_MOUSE_LL ─┐
tray menu ───┤
IPC request ─┼─> AppSignal channel ─> coordinator ─> winvd backend
winvd event ─┤                              │
window event ┤                              │
watchdog ────┘                              ├─> structured event bus
                                               ├─> logs
                                               └─> tray state
```

Hooks never call COM or inspect windows. They accumulate wheel deltas, apply direction/threshold/cooldown, enqueue at most one navigation step, and consume the wheel event only when the enqueue succeeds.

## Desktop reconciliation

For every stable snapshot DeskPilot requires:

```text
desktop_count >= 1
known trailing empty desktops == 1
known internal empty desktops == 0
known user windows moved or closed == 0
```

The inventory maps every eligible top-level window to a desktop. Pinned windows are excluded. A failed or ambiguous inspection marks the affected desktop `unknown`; unknown state prevents destructive removal.

The planner is deterministic and has no Win32 dependencies. It can only emit:

- `CreateTrailing`
- `Switch { desktop }`
- `Remove { desktop, fallback }`

There is deliberately no move-window or close-window operation. A current empty desktop is switched away from before removal. Every mutation is followed by a fresh backend snapshot. The executor has a fixed iteration limit and detects no-progress cycles.

## Failure behavior

- Unknown Windows build: diagnostic and CLI remain available; create/switch/remove and dynamic reconciliation are disabled.
- Lost Explorer/backend connection: an event or watchdog retries after Explorer returns; errors are rate-bounded by event coalescing and log rotation.
- Invalid configuration: startup fails with file/key/cause; the existing file is not overwritten.
- Named-pipe timeout: terminal command exits with a stable software-unavailable code.
- Hook failure: startup reports the failure; `run --no-hook` remains available for recovery.
- Panic: `panic = abort` prevents unwinding through FFI; a bounded crash report is written before termination when the panic hook runs.

## Security boundaries

The process runs `asInvoker`. IPC uses a unique current-user SID in both the pipe name and security descriptor. There is no arbitrary-command IPC method. Support bundles reject symlinks and include only DeskPilot-owned configuration, diagnostic and log data. Runtime never performs network I/O.
