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

Upgrade to 0.1.2 or later. Earlier builds used `VK_NONAME`, which does not reliably mark the Windows key as part of a chord on every Windows build. DeskPilot now emits a complete synthetic Control down/up pair only after processing Win+wheel.

Do not work around the issue by globally disabling the Windows key. If Start still opens, retain `doctor`, event stream and logs in a support bundle because the input sequence remains build/device-specific.

## A desktop is not removed

DeskPilot preserves the active desktop even when it is empty. If desktop 2 becomes empty while you remain on desktop 3, DeskPilot removes the non-current empty desktop and the current desktop is renumbered; it does not force you away from the desktop you are using.

DeskPilot also preserves a desktop when occupancy is unknown, a window is pinned, the empty grace period has not elapsed, or the backend rejects removal. Version 0.1.2 narrows unknown occupancy to plausible user-application windows and ignores shell/auxiliary surfaces that previously blocked cleanup.

Inspect the occupancy summary and recent errors in `doctor` when cleanup still does not occur.

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
