# Testing

## Deterministic tests

`cargo test --workspace --locked` covers:

- the reconciliation invariant and mutation ordering;
- unknown occupancy safety;
- grace periods and current-desktop fallback;
- bounded create/remove failures and duplicate events;
- wheel accumulation, direction, cooldown, clamp and wrap;
- strict TOML parsing, ranges, version defaults and atomic persistence;
- CLI hierarchy, JSON flag and safe run modes.

`DeskPilot.exe self-test --backend mock` provides a packaged smoke check without touching real desktops.

## Hosted Windows CI

`.github/workflows/ci.yml` runs formatting, Clippy with warnings denied, tests, locked release build, local CLI checks, exact license verification and portable ZIP construction on `windows-latest`. It uploads the package, checksums, logs and bootstrap lockfile for the exact commit SHA.

A hosted runner build proves compilation and deterministic behavior. It does not prove low-level input or Explorer-integrated virtual desktop behavior because hosted Actions sessions are not guaranteed to be interactive.

## Interactive Windows smoke

`.github/workflows/windows-integration.yml` targets a runner with all labels:

```text
self-hosted, Windows, X64, deskpilot-interactive
```

The runner must be a disposable/dedicated Windows 11 user session with Explorer active. `scripts/smoke-windows.ps1` aborts rather than altering an unsuitable session. It records the initial desktop state, starts a foreground no-tray instance, exercises IPC, creates controlled desktops and Notepad, verifies trailing-spare creation and compaction, tests navigation, closes only its own process, reconciles, and records diagnostics.

The script cannot safely claim success for physical Win+wheel handling when the environment blocks synthetic input. That check must fail or remain explicitly unverified; it must never be silently converted to PASS.

## Manual acceptance

On a supported Windows 11 machine:

1. Extract the portable ZIP to a writable folder.
2. Run `DeskPilot.exe doctor --json` and retain the output.
3. Start `DeskPilot.exe run --foreground`.
4. Confirm wheel scrolling is unchanged without a Windows key.
5. Hold left Windows and wheel in both directions; repeat with right Windows.
6. Confirm Start does not open after releasing Windows.
7. Confirm Win, Win+E, Win+D, Win+R, Win+L, Alt+Tab and Ctrl+wheel still behave normally.
8. Occupy the final spare and verify a new spare appears.
9. Empty an internal desktop and verify it is removed only after the grace period without moving or closing windows.
10. Restart Explorer and verify navigation/reconciliation recover.
11. Run `DeskPilot.exe shutdown` and confirm hooks, icon and process exit.
