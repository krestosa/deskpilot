use crate::config::{Config, LogLevel, NavigationMode, WheelDirection};
use crate::diagnostics::{BackendDiagnostic, DoctorReport, OccupancyDiagnostic};
use crate::event::{Event, EventBus};
use crate::ipc::{IpcResponse, IpcServer, ServerRequest};
use crate::logging::{log, timestamp_utc, Logger};
use crate::reconciliation::{plan, DesktopId, Mutation, Occupancy};
use crate::support::create_support_bundle;
use crate::tray::{Tray, TrayCommand};
use crate::wheel::Step;
use crate::windows::desktops::WinvdBackend;
use crate::windows::{
    hooks::HookController, inventory, startup, system, window_events::WindowEventController,
};
use crate::{APP_NAME, APP_VERSION};
use serde_json::json;
use std::collections::{HashMap, HashSet};
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::mpsc::{self, Receiver};
use std::sync::{Arc, RwLock};
use std::thread;
use std::time::{Duration, Instant};
use windows_sys::Win32::Foundation::{CloseHandle, GetLastError, ERROR_ALREADY_EXISTS, HANDLE};
use windows_sys::Win32::System::Threading::CreateMutexW;
use windows_sys::Win32::UI::Shell::ShellExecuteW;
use windows_sys::Win32::UI::WindowsAndMessaging::SW_SHOWNORMAL;

use crate::cli::RunOptions;
use crate::windows::util::wide;

#[derive(Debug)]
enum AppSignal {
    Navigation(Step),
    Tray(TrayCommand),
    Ipc(ServerRequest),
    DesktopEvent,
    WindowEvent,
}

pub fn run(data_dir: PathBuf, options: RunOptions) -> Result<(), String> {
    fs::create_dir_all(&data_dir).map_err(|error| {
        format!(
            "cannot create data directory {}: {error}",
            data_dir.display()
        )
    })?;
    install_panic_hook(&data_dir);
    let instance = InstanceGuard::acquire()?;
    if instance.already_exists {
        return Err("DeskPilot is already running for this user".to_string());
    }

    let (config, config_path) =
        Config::load_or_create(&data_dir).map_err(|error| error.to_string())?;
    let config = Arc::new(RwLock::new(config));
    let logging_config = config
        .read()
        .map_err(|_| "configuration lock poisoned".to_string())?
        .logging
        .clone();
    Logger::initialize(&data_dir, logging_config).map_err(|error| error.to_string())?;
    log(LogLevel::Info, format!("starting {APP_NAME} {APP_VERSION}"));

    let backend = WinvdBackend::detect();
    if !backend.compatible() {
        log(LogLevel::Warn, backend.compatibility_reason());
    }

    let events = Arc::new(EventBus::default());
    let (signal_tx, signal_rx) = mpsc::channel::<AppSignal>();

    let (ipc_tx, ipc_rx) = mpsc::channel();
    bridge(ipc_rx, signal_tx.clone(), AppSignal::Ipc);
    let ipc = IpcServer::start(ipc_tx, events.clone())?;

    let mut hook = if options.no_hook {
        None
    } else {
        let (navigation_tx, navigation_rx) = mpsc::channel();
        bridge(navigation_rx, signal_tx.clone(), AppSignal::Navigation);
        Some(HookController::start(config.clone(), navigation_tx)?)
    };
    if let Some(hook) = &hook {
        hook.set_backend_ready(backend.compatible());
    }

    let mut tray = if options.no_tray {
        None
    } else {
        let (tray_tx, tray_rx) = mpsc::channel();
        bridge(tray_rx, signal_tx.clone(), AppSignal::Tray);
        Some(Tray::start(tray_tx)?)
    };

    let (desktop_tx, desktop_rx) = mpsc::channel::<winvd::DesktopEvent>();
    let desktop_listener = if backend.compatible() {
        match winvd::listen_desktop_events(desktop_tx) {
            Ok(listener) => Some(listener),
            Err(error) => {
                log(
                    LogLevel::Warn,
                    format!("desktop event listener unavailable: {error:?}"),
                );
                None
            }
        }
    } else {
        None
    };
    bridge(desktop_rx, signal_tx.clone(), |_| AppSignal::DesktopEvent);

    let (window_tx, window_rx) = mpsc::channel();
    bridge(window_rx, signal_tx.clone(), |_| AppSignal::WindowEvent);
    let mut window_events = match WindowEventController::start(window_tx) {
        Ok(controller) => Some(controller),
        Err(error) => {
            log(
                LogLevel::Warn,
                format!("window event listener unavailable: {error}"),
            );
            None
        }
    };

    let mut state = AppState {
        data_dir,
        config_path,
        config,
        backend,
        events,
        empty_since: HashMap::new(),
        last_reconciliation: None,
        hook_state: if options.no_hook {
            "disabled by --no-hook".to_string()
        } else {
            "active".to_string()
        },
        ipc_state: format!("active: {}", ipc.pipe_name()),
        dynamic_forced_off: options.no_dynamic,
        foreground: options.foreground,
        shutdown: false,
    };
    state.update_tray(tray.as_ref());

    let mut pending_reconcile = Some(Instant::now());
    let mut next_watchdog = Instant::now();

    while !state.shutdown {
        let now = Instant::now();
        let wait_until = pending_reconcile
            .into_iter()
            .chain(std::iter::once(next_watchdog))
            .min()
            .unwrap_or_else(|| now + Duration::from_secs(1));
        let timeout = wait_until
            .saturating_duration_since(now)
            .min(Duration::from_secs(1));

        match signal_rx.recv_timeout(timeout) {
            Ok(AppSignal::Navigation(step)) => {
                state.navigate(step);
                let delay = state.config_read().desktops.reconcile_delay_ms;
                schedule_reconcile_at_earliest(
                    &mut pending_reconcile,
                    Instant::now() + Duration::from_millis(delay),
                );
            }
            Ok(AppSignal::Tray(command)) => {
                if state.handle_tray(command, tray.as_ref())? {
                    pending_reconcile = Some(Instant::now());
                }
            }
            Ok(AppSignal::Ipc(request)) => {
                let reconcile = state.handle_ipc(request);
                if reconcile {
                    pending_reconcile = Some(Instant::now());
                }
            }
            Ok(AppSignal::DesktopEvent | AppSignal::WindowEvent) => {
                let delay = state.config_read().desktops.reconcile_delay_ms;
                schedule_reconcile_at_earliest(
                    &mut pending_reconcile,
                    Instant::now() + Duration::from_millis(delay),
                );
            }
            Err(mpsc::RecvTimeoutError::Disconnected) => break,
            Err(mpsc::RecvTimeoutError::Timeout) => {}
        }

        let config_snapshot = state.config_read();
        if let Some(hook) = &hook {
            hook.set_enabled(config_snapshot.enabled);
            hook.set_backend_ready(state.backend.compatible());
            hook.set_suspended(
                config_snapshot.windows.suspend_in_exclusive_fullscreen
                    && inventory::exclusive_fullscreen_active(),
            );
        }

        let now = Instant::now();
        if now >= next_watchdog {
            schedule_reconcile_at_earliest(&mut pending_reconcile, now);
            next_watchdog =
                now + Duration::from_millis(config_snapshot.desktops.watchdog_interval_ms);
        }
        if pending_reconcile.is_some_and(|deadline| now >= deadline) {
            if config_snapshot.desktops.dynamic && !state.dynamic_forced_off {
                state.reconcile();
            }
            pending_reconcile = None;
            state.update_tray(tray.as_ref());
        }
    }

    state
        .events
        .publish(Event::new("shutdown", "DeskPilot is shutting down"));
    drop(desktop_listener);
    if let Some(window_events) = &mut window_events {
        window_events.stop();
    }
    if let Some(hook) = &mut hook {
        hook.stop();
    }
    if let Some(tray) = &mut tray {
        tray.stop();
    }
    drop(ipc);
    log(LogLevel::Info, "shutdown complete");
    drop(instance);
    Ok(())
}

fn schedule_reconcile_at_earliest(pending: &mut Option<Instant>, deadline: Instant) {
    match pending {
        Some(current) if *current <= deadline => {}
        _ => *pending = Some(deadline),
    }
}

struct AppState {
    data_dir: PathBuf,
    config_path: PathBuf,
    config: Arc<RwLock<Config>>,
    backend: WinvdBackend,
    events: Arc<EventBus>,
    empty_since: HashMap<DesktopId, Instant>,
    last_reconciliation: Option<String>,
    hook_state: String,
    ipc_state: String,
    dynamic_forced_off: bool,
    foreground: bool,
    shutdown: bool,
}

impl AppState {
    fn config_read(&self) -> Config {
        self.config
            .read()
            .map_or_else(|_| Config::default(), |config| config.clone())
    }

    fn save_config(&self, config: &Config) -> Result<(), String> {
        config
            .write_atomic(&self.config_path)
            .map_err(|error| error.to_string())?;
        let mut target = self
            .config
            .write()
            .map_err(|_| "configuration lock poisoned".to_string())?;
        *target = config.clone();
        Ok(())
    }

    fn navigate(&mut self, step: Step) {
        let config = self.config_read();
        if !config.enabled {
            return;
        }
        match self.backend.switch_relative(step, config.wheel.navigation) {
            Ok(desktop) => self.publish(
                "desktop-switched",
                format!("desktop index {}", desktop.index),
            ),
            Err(error) if error.contains("clamped edge") => {}
            Err(error) => self.error(format!("desktop navigation failed: {error}")),
        }
    }

    fn reconcile(&mut self) {
        if !self.backend.compatible() {
            return;
        }
        let mut mutations = Vec::new();
        for _ in 0..8 {
            let snapshot = match self.snapshot() {
                Ok(snapshot) => snapshot,
                Err(error) => {
                    self.error(format!("reconciliation snapshot failed: {error}"));
                    return;
                }
            };
            let next = plan(&snapshot);
            if next.stable {
                self.last_reconciliation = Some(timestamp_utc());
                if !mutations.is_empty() {
                    self.publish("reconciled", format!("{} mutation(s)", mutations.len()));
                }
                return;
            }
            if next.mutations.is_empty() {
                self.last_reconciliation = Some(timestamp_utc());
                return;
            }
            for mutation in next.mutations {
                let result = match &mutation {
                    Mutation::CreateTrailing => self.backend.create().map(|_| ()),
                    Mutation::Switch { desktop } => self.backend.switch_to_id(desktop),
                    Mutation::Remove { desktop, fallback } => {
                        self.backend.remove(desktop, fallback)
                    }
                };
                match result {
                    Ok(()) => mutations.push(mutation),
                    Err(error) => {
                        self.error(format!(
                            "reconciliation mutation failed: {mutation:?}: {error}"
                        ));
                        return;
                    }
                }
            }
        }
        self.error("reconciliation stopped at iteration limit".to_string());
    }

    fn snapshot(&mut self) -> Result<Vec<crate::reconciliation::DesktopState>, String> {
        let config = self.config_read();
        let mut states = inventory::snapshot(&self.backend, &config, &HashMap::new())?;
        let now = Instant::now();
        let existing: HashSet<_> = states.iter().map(|state| state.id.clone()).collect();
        self.empty_since.retain(|id, _| existing.contains(id));
        for state in &mut states {
            if state.occupancy == Occupancy::Empty {
                let since = self.empty_since.entry(state.id.clone()).or_insert(now);
                state.empty_grace_elapsed = now.duration_since(*since)
                    >= Duration::from_millis(config.desktops.empty_grace_ms);
            } else {
                self.empty_since.remove(&state.id);
                state.empty_grace_elapsed = false;
            }
        }
        Ok(states)
    }

    fn handle_tray(&mut self, command: TrayCommand, tray: Option<&Tray>) -> Result<bool, String> {
        let mut config = self.config_read();
        let mut reconcile = false;
        match command {
            TrayCommand::ToggleEnabled => {
                config.enabled = !config.enabled;
                self.save_config(&config)?;
            }
            TrayCommand::ToggleDynamic => {
                config.desktops.dynamic = !config.desktops.dynamic;
                self.save_config(&config)?;
                reconcile = true;
            }
            TrayCommand::ToggleDirection => {
                config.wheel.direction = match config.wheel.direction {
                    WheelDirection::Normal => WheelDirection::Inverted,
                    WheelDirection::Inverted => WheelDirection::Normal,
                };
                self.save_config(&config)?;
            }
            TrayCommand::ToggleNavigation => {
                config.wheel.navigation = match config.wheel.navigation {
                    NavigationMode::Clamp => NavigationMode::Wrap,
                    NavigationMode::Wrap => NavigationMode::Clamp,
                };
                self.save_config(&config)?;
            }
            TrayCommand::Reconcile => reconcile = true,
            TrayCommand::Reload => {
                self.reload()?;
                reconcile = true;
            }
            TrayCommand::OpenConfig => open_path(&self.config_path),
            TrayCommand::Diagnostics => {
                let report = self.doctor();
                let path = self.data_dir.join("logs").join("doctor.json");
                if let Ok(json) = serde_json::to_vec_pretty(&report) {
                    let _ = fs::write(&path, json);
                    open_path(&path);
                }
            }
            TrayCommand::ToggleStartup => {
                if startup::is_enabled() {
                    startup::disable()?;
                } else {
                    startup::enable(
                        &std::env::current_exe().map_err(|error| error.to_string())?,
                        &self.data_dir,
                    )?;
                }
            }
            TrayCommand::OpenLogs => open_path(&self.data_dir.join("logs")),
            TrayCommand::Exit => self.shutdown = true,
        }
        self.update_tray(tray);
        Ok(reconcile)
    }

    fn handle_ipc(&mut self, request: ServerRequest) -> bool {
        let command = request.request.command.clone();
        let mut reconcile = false;
        let response = match command.as_str() {
            "status" => IpcResponse::success(self.status()),
            "doctor" => IpcResponse::success(self.doctor()),
            "desktops list" => self.backend.list().map_or_else(
                |error| IpcResponse::failure(69, error),
                |desktops| {
                    IpcResponse::success(
                        desktops
                            .iter()
                            .map(|desktop| json!({"id": desktop.id.0, "index": desktop.index}))
                            .collect::<Vec<_>>(),
                    )
                },
            ),
            "desktops current" => self.backend.current().map_or_else(
                |error| IpcResponse::failure(69, error),
                |desktop| IpcResponse::success(json!({"id": desktop.id.0, "index": desktop.index})),
            ),
            "desktops next" => self
                .backend
                .switch_relative(Step::Next, self.config_read().wheel.navigation)
                .map_or_else(
                    |error| IpcResponse::failure(69, error),
                    |desktop| {
                        IpcResponse::success(json!({"id": desktop.id.0, "index": desktop.index}))
                    },
                ),
            "desktops previous" => self
                .backend
                .switch_relative(Step::Previous, self.config_read().wheel.navigation)
                .map_or_else(
                    |error| IpcResponse::failure(69, error),
                    |desktop| {
                        IpcResponse::success(json!({"id": desktop.id.0, "index": desktop.index}))
                    },
                ),
            "desktops create" => self.backend.create().map_or_else(
                |error| IpcResponse::failure(69, error),
                |desktop| IpcResponse::success(json!({"id": desktop.id.0, "index": desktop.index})),
            ),
            "reconcile" => {
                reconcile = true;
                IpcResponse::message("reconciliation scheduled")
            }
            "enable" | "disable" => {
                let mut config = self.config_read();
                config.enabled = command == "enable";
                self.save_config(&config).map_or_else(
                    |error| IpcResponse::failure(78, error),
                    |()| IpcResponse::success(json!({"enabled": config.enabled})),
                )
            }
            "reload" => {
                reconcile = true;
                self.reload().map_or_else(
                    |error| IpcResponse::failure(78, error),
                    |()| IpcResponse::message("configuration reloaded"),
                )
            }
            "config path" => IpcResponse::success(json!({"path": self.config_path})),
            "config show" => fs::read_to_string(&self.config_path).map_or_else(
                |error| IpcResponse::failure(74, error.to_string()),
                |text| IpcResponse::success(json!({"toml": text})),
            ),
            "config validate" => Config::load(&self.config_path).map_or_else(
                |error| IpcResponse::failure(78, error.to_string()),
                |_| IpcResponse::message("configuration valid"),
            ),
            "logs path" => IpcResponse::success(json!({"path": self.data_dir.join("logs")})),
            "logs tail" => IpcResponse::success(
                json!({"lines": tail_log(&self.data_dir.join("logs").join("deskpilot.log"), 100)}),
            ),
            "support-bundle" => {
                let doctor = serde_json::to_string_pretty(&self.doctor()).unwrap_or_default();
                let redacted = toml::to_string_pretty(&self.config_read()).unwrap_or_default();
                create_support_bundle(&self.data_dir, &doctor, &redacted).map_or_else(
                    |error| IpcResponse::failure(74, error),
                    |path| IpcResponse::success(json!({"path": path})),
                )
            }
            "startup enable" => {
                startup::enable(&std::env::current_exe().unwrap_or_default(), &self.data_dir)
                    .map_or_else(
                        |error| IpcResponse::failure(74, error),
                        |()| IpcResponse::message("startup enabled"),
                    )
            }
            "startup disable" => startup::disable().map_or_else(
                |error| IpcResponse::failure(74, error),
                |()| IpcResponse::message("startup disabled"),
            ),
            "shutdown" => {
                self.shutdown = true;
                IpcResponse::message("shutdown requested")
            }
            "__wake" => IpcResponse::message("awake"),
            _ => IpcResponse::failure(64, format!("unknown IPC command: {command}")),
        };
        let _ = request.response.send(response);
        reconcile
    }

    fn reload(&mut self) -> Result<(), String> {
        let config = Config::load(&self.config_path).map_err(|error| error.to_string())?;
        let mut target = self
            .config
            .write()
            .map_err(|_| "configuration lock poisoned".to_string())?;
        *target = config;
        self.publish("configuration-reloaded", "configuration reloaded");
        Ok(())
    }

    fn status(&self) -> serde_json::Value {
        let config = self.config_read();
        json!({
            "version": APP_VERSION,
            "enabled": config.enabled,
            "dynamic": config.desktops.dynamic && !self.dynamic_forced_off,
            "direction": config.wheel.direction,
            "navigation": config.wheel.navigation,
            "backend_compatible": self.backend.compatible(),
            "last_reconciliation": self.last_reconciliation,
            "data_directory": self.data_dir,
        })
    }

    fn doctor(&mut self) -> DoctorReport {
        let config = self.config_read();
        let version = self.backend.version();
        let capabilities = self.backend.capabilities();
        let snapshot = self.snapshot().ok();
        let occupancy = snapshot.as_ref().map_or(
            OccupancyDiagnostic {
                occupied: 0,
                empty: 0,
                unknown: 0,
            },
            |states| OccupancyDiagnostic {
                occupied: states
                    .iter()
                    .filter(|state| state.occupancy == Occupancy::Occupied)
                    .count(),
                empty: states
                    .iter()
                    .filter(|state| state.occupancy == Occupancy::Empty)
                    .count(),
                unknown: states
                    .iter()
                    .filter(|state| state.occupancy == Occupancy::Unknown)
                    .count(),
            },
        );
        let current = self.backend.current().ok();
        DoctorReport {
            timestamp: timestamp_utc(),
            app: APP_NAME.to_string(),
            app_version: APP_VERSION.to_string(),
            executable_path: std::env::current_exe().unwrap_or_default(),
            data_directory: self.data_dir.clone(),
            configuration_status: if config.validate().is_ok() {
                "valid".to_string()
            } else {
                "invalid".to_string()
            },
            windows_version: format!(
                "{}.{}.{}.{}",
                version.major, version.minor, version.build, version.revision
            ),
            windows_build: version.build,
            windows_revision: version.revision,
            process_architecture: std::env::consts::ARCH.to_string(),
            integrity_level: system::integrity_level(),
            interactive_session: system::is_interactive_session(),
            explorer_running: system::explorer_running(),
            backend: BackendDiagnostic {
                name: "winvd 0.0.49".to_string(),
                compatible: self.backend.compatible(),
                capabilities: [
                    (capabilities.enumerate, "enumerate"),
                    (capabilities.switch, "switch"),
                    (capabilities.create, "create"),
                    (capabilities.remove, "remove"),
                    (capabilities.window_mapping, "window-mapping"),
                    (capabilities.pin_detection, "pin-detection"),
                ]
                .into_iter()
                .filter_map(|(enabled, name)| enabled.then_some(name.to_string()))
                .collect(),
                error: (!self.backend.compatible())
                    .then(|| self.backend.compatibility_reason().to_string()),
            },
            desktop_count: snapshot.as_ref().map(Vec::len),
            current_desktop: current.map(|desktop| desktop.id.0),
            occupancy,
            hook_state: self.hook_state.clone(),
            ipc_state: self.ipc_state.clone(),
            dynamic_reconciliation: if config.desktops.dynamic && !self.dynamic_forced_off {
                "enabled"
            } else {
                "disabled"
            }
            .to_string(),
            last_reconciliation: self.last_reconciliation.clone(),
            recent_errors: Logger::global().map_or_else(Vec::new, Logger::recent_errors),
            portable_write_test: system::portable_write_test(&self.data_dir),
        }
    }

    fn update_tray(&self, tray: Option<&Tray>) {
        if let Some(tray) = tray {
            let config = self.config_read();
            tray.state().update(
                config.enabled,
                config.desktops.dynamic && !self.dynamic_forced_off,
                config.wheel.direction,
                config.wheel.navigation,
                startup::is_enabled(),
                !self.backend.compatible(),
            );
        }
    }

    fn publish(&self, kind: &str, message: impl Into<String>) {
        let message = message.into();
        log(LogLevel::Info, &message);
        if self.foreground {
            println!("{} {kind}: {message}", timestamp_utc());
        }
        self.events.publish(Event::new(kind, message));
    }

    fn error(&self, message: String) {
        log(LogLevel::Error, &message);
        if self.foreground {
            eprintln!("{} error: {message}", timestamp_utc());
        }
        self.events.publish(Event::new("error", message));
    }
}

fn bridge<T, F>(receiver: Receiver<T>, sender: mpsc::Sender<AppSignal>, map: F)
where
    T: Send + 'static,
    F: Fn(T) -> AppSignal + Send + 'static,
{
    thread::spawn(move || {
        for value in receiver {
            if sender.send(map(value)).is_err() {
                break;
            }
        }
    });
}

fn open_path(path: &Path) {
    let operation = wide("open");
    let target = wide(path);
    unsafe {
        ShellExecuteW(
            0,
            operation.as_ptr(),
            target.as_ptr(),
            std::ptr::null(),
            std::ptr::null(),
            SW_SHOWNORMAL,
        );
    }
}

fn tail_log(path: &Path, lines: usize) -> Vec<String> {
    fs::read_to_string(path)
        .map(|text| {
            text.lines()
                .rev()
                .take(lines)
                .collect::<Vec<_>>()
                .into_iter()
                .rev()
                .map(str::to_string)
                .collect()
        })
        .unwrap_or_default()
}

fn install_panic_hook(data_dir: &Path) {
    let crash_dir = data_dir.join("crash-reports");
    let _ = fs::create_dir_all(&crash_dir);
    std::panic::set_hook(Box::new(move |info| {
        let path = crash_dir.join(format!(
            "crash-{}.txt",
            timestamp_utc().replace([':', '.'], "-")
        ));
        let message = format!("DeskPilot {}\n{}\n", APP_VERSION, info);
        let _ = fs::write(
            path,
            message
                .as_bytes()
                .get(..16 * 1024)
                .unwrap_or(message.as_bytes()),
        );
    }));
}

struct InstanceGuard {
    handle: HANDLE,
    already_exists: bool,
}

impl InstanceGuard {
    fn acquire() -> Result<Self, String> {
        let sid = system::current_user_sid()?;
        let name = wide(format!("Local\\DeskPilot-{sid}"));
        unsafe {
            let handle = CreateMutexW(std::ptr::null(), 0, name.as_ptr());
            if handle == 0 {
                return Err(format!("CreateMutexW failed: {}", GetLastError()));
            }
            Ok(Self {
                handle,
                already_exists: GetLastError() == ERROR_ALREADY_EXISTS,
            })
        }
    }
}

impl Drop for InstanceGuard {
    fn drop(&mut self) {
        unsafe {
            CloseHandle(self.handle);
        }
    }
}

#[cfg(test)]
mod scheduling_tests {
    use super::schedule_reconcile_at_earliest;
    use std::time::{Duration, Instant};

    #[test]
    fn repeated_window_events_cannot_postpone_reconciliation() {
        let now = Instant::now();
        let first = now + Duration::from_millis(250);
        let mut pending = Some(first);
        schedule_reconcile_at_earliest(&mut pending, now + Duration::from_secs(2));
        assert_eq!(pending, Some(first));
    }

    #[test]
    fn watchdog_advances_a_later_pending_reconciliation() {
        let now = Instant::now();
        let mut pending = Some(now + Duration::from_secs(2));
        schedule_reconcile_at_earliest(&mut pending, now);
        assert_eq!(pending, Some(now));
    }
}
