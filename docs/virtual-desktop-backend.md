# Virtual desktop backend

## Why an internal backend is required

Microsoft's public `IVirtualDesktopManager` API exposes desktop membership, desktop ID lookup and window movement. It does not expose ordered enumeration, creation, removal or desktop switching. DeskPilot therefore isolates the undocumented shell COM contracts behind `src/windows/desktops.rs`.

## Selected implementation

DeskPilot pins `winvd` to exactly `0.0.49`. The crate is MIT-licensed and provides the required operations:

- ordered desktop enumeration;
- current desktop lookup;
- switch, create and remove;
- desktop lookup for a window;
- pinned-window detection;
- desktop/window change notifications.

The third-party copyright and permission notice is included in `THIRD_PARTY_NOTICES.md`.

## Compatibility gate

The backend is enabled only for recognized Windows 11 build families:

- 24H2: build `26100`;
- 25H2: build `26200`.

The pinned `winvd` release documents a minimum serviced 24H2 baseline of `26100.2605` and reports testing on `26200.8117`. Windows' basic version structure exposes the build family but not the cumulative update revision in the field used by DeskPilot; `doctor` reports the detected build and the exact application/backend versions so an unsupported servicing regression can be diagnosed.

On any unrecognized build the backend reports incompatible and DeskPilot does not create, remove, switch or reconcile desktops. Tray error state, configuration, logs, `doctor`, support bundles and mock self-test remain available.

## Maintenance boundary

All direct `winvd` calls and HWND type conversion are limited to `src/windows/desktops.rs`. Occupancy logic, reconciliation and CLI do not depend on COM interface layouts. Updating the backend requires:

1. auditing the new crate license and source changes;
2. updating the exact dependency version;
3. updating the compatibility table with objective evidence;
4. running unit, CLI and packaging CI;
5. running `scripts/smoke-windows.ps1` in a dedicated interactive session;
6. confirming Start-menu suppression and normal Windows shortcuts manually or through dedicated input automation.

DeskPilot never copies undocumented interface definitions into multiple modules and never downloads a backend at runtime.
