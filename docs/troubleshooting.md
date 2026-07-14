# Troubleshooting

## `DeskPilot is not running or the IPC timeout expired`

Start the tray process with `DeskPilot.exe`, or run `DeskPilot.exe run --foreground` to see initialization errors. Each Windows user has a separate IPC pipe and instance mutex.

## Backend is incompatible

Run `DeskPilot.exe doctor --json`. DeskPilot intentionally disables desktop mutation on unrecognized build families. Do not bypass the gate: update the pinned backend and compatibility evidence instead.

DeskPilot 0.1.0 could incorrectly report `6.2.9200.<revision>` on Windows 11 because the version API was manifest-virtualized. Upgrade to 0.1.1 or later. A real Windows 11 report must begin with `10.0` and include the actual build, for example `10.0.26100.<revision>`.

## Configuration fails to load

Run:

```powershell
.\DeskPilot.exe config validate .\deskpilot.toml
```

Unknown keys, invalid enum values and out-of-range numbers are errors. DeskPilot reports the file and parser/validation cause and does not rewrite the invalid file.

## Win+wheel works and then becomes blocked on an empty desktop

DeskPilot 0.1.1 could mistake the Windows desktop, Start or another shell surface for an exclusive-fullscreen application and suspend the input hook. Upgrade to 0.1.2 or later. The fullscreen detector now requires a visible, non-cloaked user application and explicitly excludes shell hosts.

If the problem persists, temporarily set:

```toml
[windows]
suspend_in_exclusive_fullscreen = false
```

Reload the configuration and collect `doctor --json`, `events --json` and a support bundle.

## Start opens after Win+wheel

Upgrade to 0.1.3 or later. Version 0.1.2 pressed and released a synthetic Control key entirely inside the wheel callback; on some Windows builds the chord was no longer active when the physical Windows key was released, so Start could still open.

Version 0.1.3 observes physical left and right Windows-key transitions with a low-level keyboard hook. After DeskPilot consumes Win+wheel it holds neutral `F24` until after the final physical Windows-key release and only then releases it from the hook thread. This keeps the Win press classified as a chord for its complete lifetime.

Do not globally disable the Windows key. If Start still opens on 0.1.3, retain `doctor`, event stream, logs and a support bundle because the remaining behavior would be device- or driver-specific.

## Extra empty desktops are not removed

Upgrade to 0.1.3 or later. Earlier inventory logic could attempt virtual-desktop mapping for a shell/helper window before identifying its process. A mapping failure then marked every empty desktop as `unknown`, which correctly prevented destructive removal but also stopped dynamic compaction indefinitely.

Version 0.1.3 filters Start, input, tray, control-center, widget, broker and other shell surfaces before mapping. Genuine mapping uncertainty is localized rather than poisoning all empty desktops. When several trailing desktops are empty, reconciliation removes non-current duplicates one at a time from fresh snapshots until exactly one empty spare remains.

DeskPilot always preserves the active desktop, even when empty. For example, if desktop 1 is occupied and desktops 2 and 3 are empty while you are on desktop 3, DeskPilot removes desktop 2. Your active desktop remains and Windows renumbers it as desktop 2.

A desktop is still preserved when a plausible user application cannot be safely inspected, a window is pinned, the empty grace period has not elapsed, or the backend rejects removal. Inspect `doctor --json`, recent errors and `events --json` when cleanup still does not occur.

## Portable directory is read-only

Choose a writable data directory:

```powershell
.\DeskPilot.exe --data-dir D:\DeskPilotData run
```

Pass the same `--data-dir` to later CLI commands and to Startup enablement.

## Collecting a support bundle

```powershell
.\DeskPilot.exe support-bundle
```

The generated ZIP contains DeskPilot diagnostics, configuration, bounded logs, manifest and checksums. It excludes window titles, document contents, memory dumps and unrelated files.
