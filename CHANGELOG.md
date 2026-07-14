# Changelog

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
