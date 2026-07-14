# Troubleshooting

## `DeskPilot is not running or the IPC timeout expired`

Start the tray process with `DeskPilot.exe`, or run `DeskPilot.exe run --foreground` to see initialization errors. Each Windows user has a separate IPC pipe and instance mutex.

## Backend is incompatible

Run `DeskPilot.exe doctor --json`. DeskPilot intentionally disables desktop mutation on unrecognized build families. Do not bypass the gate: update the pinned backend and compatibility evidence instead.

## Configuration fails to load

Run:

```powershell
.\DeskPilot.exe config validate .\deskpilot.toml
```

Unknown keys, invalid enum values and out-of-range numbers are errors. DeskPilot reports the file and parser/validation cause and does not rewrite the invalid file.

## Win+wheel does nothing

Check `enabled`, `wheel.threshold`, `wheel.cooldown_ms`, fullscreen suspension and `doctor.hook_state`. Use `run --foreground` and `events --json` in separate terminals. `run --no-hook` can isolate backend/IPC behavior.

## Start opens after Win+wheel

Stop DeskPilot and retain `doctor`, event stream and logs in a support bundle. This is a release-blocking input regression for the affected Windows build or device; do not work around it by globally disabling the Windows key.

## A desktop is not removed

DeskPilot will preserve a desktop when occupancy is unknown, a window is pinned, the empty grace period has not elapsed, or the backend rejects removal. This conservative behavior is intentional. Inspect the occupancy summary and recent errors in `doctor`.

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
