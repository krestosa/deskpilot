# Changelog

## 0.1.4

- Intercepted the final physical Windows-key release after a consumed Win+wheel gesture and replaced it with a marked Control+Windows release sequence so Start cannot treat the gesture as a standalone Win press.
- Replaced global unknown-occupancy poisoning with direct per-desktop membership checks when normal window mapping fails.
- Counted inaccessible but mappable top-level application windows as occupied while excluding Explorer and DWM shell surfaces.
- Preserved the active desktop while allowing all other consecutive empty desktops to compact to exactly one trailing spare.

## 0.1.3

- Held a neutral `F24` chord from a consumed Win+wheel gesture until after the physical Windows-key release, preventing Start from treating the gesture as a standalone Win press.
- Added a low-level keyboard hook beside the mouse hook to track left and right Windows-key state and release the suppressor deterministically.
- Filtered shell and helper processes before attempting virtual-desktop mapping.
- Prevented one unmappable shell window from converting every empty desktop to `unknown`.
- Expanded exclusions for Start, input, tray, control-center, widget and broker surfaces.
- Added a regression test proving several trailing empty desktops converge to exactly one while preserving the active desktop.

## 0.1.2

- Prevented shell, desktop and Start surfaces from being misclassified as exclusive fullscreen applications and suspending Win+wheel navigation.
- Replaced the unreliable `VK_NONAME` Start-menu suppression with a complete synthetic Control key chord after a processed Win+wheel gesture.
- Reset partial wheel accumulation whenever DeskPilot is disabled, suspended, unavailable or the Windows key is not held.
- Stopped auxiliary and shell windows from forcing otherwise empty desktops into an indeterminate state.
- Continued counting DWM-cloaked application windows on inactive virtual desktops as occupied.
- Added built-in exclusions for Start, Search, Shell Experience, Text Input and Lock shell hosts.
- Preserved the active empty desktop, used circular wrap navigation by default and compacted non-current trailing empty desktops safely.

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
