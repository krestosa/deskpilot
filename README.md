# DeskPilot

DeskPilot is a small native Windows 11 utility that changes virtual desktops while either Windows key is held and the vertical mouse wheel is moved. It also maintains one known-empty spare desktop at the end of the ordered desktop list and removes known-empty internal desktops after a grace period.

```text
Win + wheel up    previous desktop
Win + wheel down  next desktop
```

DeskPilot is portable: extract the ZIP and run `DeskPilot.exe`. It has no installer, service, browser runtime, telemetry, updater or runtime network access. It runs without elevation and stores configuration, logs and crash reports beside the executable unless `--data-dir` is provided.

## Platform and compatibility

- Windows 11 x64
- Native `x86_64-pc-windows-msvc` binary
- Recognized backend families: 24H2 build 26100 and 25H2 build 26200
- The pinned `winvd` 0.0.49 backend documents a minimum serviced 24H2 baseline of 26100.2605 and testing on 26200.8117

Virtual desktop creation, removal and switching rely on undocumented shell COM interfaces because Microsoft's public virtual desktop API does not provide those operations. On an unrecognized build DeskPilot fails safe: diagnostics remain available, but desktop mutation is disabled.

## Portable use

1. Extract `DeskPilot-portable-<version>.zip` to a writable folder.
2. Double-click `DeskPilot.exe` to start tray mode.
3. Hold left or right Windows and move the vertical wheel.
4. Right-click the tray icon for pause, dynamic desktop, direction, navigation, configuration, diagnostics, Startup and exit controls.

The first run creates:

```text
deskpilot.toml
logs/
crash-reports/
```

To keep state elsewhere:

```powershell
.\DeskPilot.exe --data-dir D:\PortableData\DeskPilot run
```

## Dynamic desktop behavior

DeskPilot classifies every desktop as occupied, empty or unknown from eligible top-level application windows. It does not count invisible, cloaked, auxiliary, shell, excluded or pinned windows. If inspection is ambiguous, it chooses `unknown` and will not remove the desktop.

After stabilization:

```text
one known-empty desktop exists at the end
no known-empty desktop exists between occupied desktops
no user window is moved or closed by reconciliation
```

When the final spare becomes occupied, a new spare is created. When an internal desktop becomes empty, DeskPilot waits `empty_grace_ms`, confirms it again, switches away first if necessary, and removes only that desktop with a safe fallback. Every mutation is followed by a fresh read and the cycle is bounded.

## Tray menu

- DeskPilot: Enabled / Disabled
- Dynamic desktops: Enabled / Disabled
- Direction: Normal / Inverted
- Navigation: Clamp / Wrap
- Reconcile now
- Reload configuration
- Open configuration
- Diagnostics
- Start with Windows
- Open logs
- Exit

The icon uses a normal, paused or backend-error state. Startup is opt-in and creates a shortcut only in the current user's Startup folder.

## Configuration

`deskpilot.example.toml` documents all settings. The active file is `deskpilot.toml` in the data directory.

```toml
schema_version = 1
enabled = true

[wheel]
direction = "normal"
navigation = "clamp"
threshold = 120
cooldown_ms = 180

[desktops]
dynamic = true
reconcile_delay_ms = 750
empty_grace_ms = 1500
watchdog_interval_ms = 3000

[windows]
suspend_in_exclusive_fullscreen = true
ignore_executables = []
ignore_classes = []

[logging]
level = "info"
max_files = 5
max_file_size_mb = 2
```

Configuration parsing is strict. Unknown keys and unsafe ranges fail with the file, key/value context and cause. Reload does not require a restart. Writes use a same-directory temporary file followed by a replacing, write-through move on Windows.

## CLI

Running without arguments starts the tray process. Terminal commands attach to the parent console and communicate with the per-user tray instance over an ACL-restricted named pipe.

```text
DeskPilot.exe
DeskPilot.exe run
DeskPilot.exe run --foreground
DeskPilot.exe run --no-tray --no-hook --no-dynamic
DeskPilot.exe status [--json]
DeskPilot.exe doctor [--json]
DeskPilot.exe desktops list [--json]
DeskPilot.exe desktops current
DeskPilot.exe desktops next
DeskPilot.exe desktops previous
DeskPilot.exe desktops create
DeskPilot.exe reconcile
DeskPilot.exe enable | disable | reload
DeskPilot.exe config path | show | validate [FILE]
DeskPilot.exe logs path | tail
DeskPilot.exe events --json
DeskPilot.exe support-bundle
DeskPilot.exe startup enable | disable
DeskPilot.exe shutdown
DeskPilot.exe self-test --backend mock
DeskPilot.exe --version | --help
```

Exit codes follow the `sysexits` categories used by the application: `0` success, `64` command usage, `69` unavailable backend/IPC, `70` internal failure, `74` I/O failure and `78` configuration failure.

Example:

```powershell
.\DeskPilot.exe status --json
```

```json
{
  "version": "0.1.0",
  "enabled": true,
  "dynamic": true,
  "direction": "normal",
  "navigation": "clamp",
  "backend_compatible": true
}
```

`events --json` emits one JSON object per line until interrupted. `doctor --json` includes version/build, paths, integrity/session/Explorer state, backend capabilities, desktop/occupancy summary, hook/IPC/reconciliation state, recent bounded errors and a portable write test. It omits window titles and document names.

## Support bundle

`DeskPilot.exe support-bundle` creates a local ZIP under the data directory containing:

- `doctor.json`;
- the DeskPilot configuration;
- recent DeskPilot logs;
- application version;
- a file manifest and SHA-256 checksums;
- backend errors.

It does not follow symlinks and does not include memory, window contents or unrelated files.

## Build

Requirements:

- current stable Rust with `x86_64-pc-windows-msvc`;
- Visual Studio 2022 Build Tools with MSVC v143;
- Windows 11 SDK;
- PowerShell 7 or Windows PowerShell 5.1.

```powershell
rustup target add x86_64-pc-windows-msvc
cargo generate-lockfile
cargo fmt --all -- --check
cargo clippy --workspace --all-targets --locked -- -D warnings
cargo test --workspace --locked
cargo build --release --locked --target x86_64-pc-windows-msvc
.\scripts\verify-license.ps1
.\scripts\build-portable.ps1
```

The release manifest uses `requestedExecutionLevel="asInvoker"`. The package contains no PDB, installer or runtime download.

## Validation

Hosted Windows CI proves formatting, linting, deterministic tests, release compilation, local CLI behavior, exact license bytes and portable packaging. Real Win+wheel, Explorer integration and desktop lifecycle require a dedicated interactive Windows 11 session; the separate self-hosted workflow never represents a non-interactive probe as a successful interactive test.

See:

- [Architecture](docs/architecture.md)
- [Virtual desktop backend](docs/virtual-desktop-backend.md)
- [Testing](docs/testing.md)
- [Troubleshooting](docs/troubleshooting.md)

## Privacy and security

DeskPilot makes no runtime network requests and contains no telemetry. It does not inject into other processes, install drivers, move or close user windows, accept arbitrary IPC commands, or require administrator rights. Named-pipe access is restricted to the current user and SYSTEM. Unsupported Windows builds disable destructive operations.

## License

DeskPilot is licensed under PolyForm Strict License 1.0.0. `LICENSE.md` is the byte-identical official license text verified by SHA-256 in CI. Third-party notices are in `THIRD_PARTY_NOTICES.md`.
