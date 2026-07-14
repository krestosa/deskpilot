# Changelog

## Unreleased

- Preserved the active empty desktop instead of automatically switching the user to an occupied desktop.
- Prevented reconciliation from treating DWM-cloaked application windows on inactive desktops as absent.
- Blocked removal when any residual non-pinned window or uncertain mapping remains associated with a desktop.
- Changed internal-empty fallback to the trailing spare instead of the previous occupied desktop.
- Made circular wrap navigation the default for Win+wheel.

## 0.1.1

- Fixed Windows 11 version detection so manifest virtualization cannot report `6.2.9200` and incorrectly disable the virtual desktop backend.
- Corrected the Windows 10/11 compatibility GUID embedded in the executable manifest.
- Added runtime and compatibility regression tests for native Windows build detection.

## 0.1.0

- Added portable native Windows 11 tray application.
- Added Win+vertical-wheel virtual desktop navigation with high-resolution accumulation, cooldown, clamp and wrap modes.
- Added conservative dynamic reconciliation that preserves exactly one known-empty trailing desktop and never moves user windows.
- Added per-user named-pipe IPC, terminal commands, diagnostics, structured event streaming and redacted support bundles.
- Added portable configuration, log rotation, optional Startup shortcut, Explorer recovery, mock self-test and Windows CI packaging.
