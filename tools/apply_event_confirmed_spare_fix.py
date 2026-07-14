from pathlib import Path


def replace_once(path: str, old: str, new: str) -> None:
    target = Path(path)
    text = target.read_text(encoding="utf-8")
    count = text.count(old)
    if count != 1:
        raise SystemExit(f"{path}: expected exactly one match, found {count}")
    target.write_text(text.replace(old, new, 1), encoding="utf-8", newline="\n")


Path("src/reconciliation/spare_guard.rs").write_text(r'''// File purpose: Protects the trailing empty desktop until a qualifying user-window lifecycle event proves that it was actually consumed.
use super::{DesktopId, DesktopState, Occupancy};
use std::collections::{HashMap, HashSet};

pub type WindowToken = u64;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct SpareGuardResult {
    pub protecting: bool,
    pub consumed: bool,
}

#[derive(Debug, Clone, Default)]
pub struct SpareGuard {
    protected: Option<DesktopId>,
}

impl SpareGuard {
    // Function purpose: Protects a desktop selected as the trailing navigation spare even if transient shell windows make it look occupied.
    pub fn arm(&mut self, desktop: DesktopId) {
        self.protected = Some(desktop);
    }

    // Function purpose: Returns the desktop currently treated as the event-confirmed empty spare.
    pub fn protected(&self) -> Option<&DesktopId> {
        self.protected.as_ref()
    }

    // Function purpose: Overrides transient occupancy on the protected trailing spare until a qualifying create or show event maps to that desktop.
    pub fn stabilize(
        &mut self,
        states: &mut [DesktopState],
        windows: &HashMap<DesktopId, HashSet<WindowToken>>,
        occupancy_gain_candidates: &HashSet<WindowToken>,
    ) -> SpareGuardResult {
        let Some(last) = states.last() else {
            self.protected = None;
            return SpareGuardResult::default();
        };
        let last_id = last.id.clone();

        if self.protected.as_ref().is_some_and(|id| id != &last_id) {
            self.protected = None;
        }
        if self.protected.is_none() && last.occupancy == Occupancy::Empty {
            self.protected = Some(last_id.clone());
        }

        let Some(protected) = self.protected.clone() else {
            return SpareGuardResult::default();
        };
        let consumed = windows
            .get(&protected)
            .is_some_and(|tokens| !tokens.is_disjoint(occupancy_gain_candidates));
        if consumed {
            self.protected = None;
            return SpareGuardResult {
                protecting: false,
                consumed: true,
            };
        }

        if let Some(state) = states.iter_mut().find(|state| state.id == protected) {
            state.occupancy = Occupancy::Empty;
            state.empty_grace_elapsed = false;
            SpareGuardResult {
                protecting: true,
                consumed: false,
            }
        } else {
            self.protected = None;
            SpareGuardResult::default()
        }
    }
}
''', encoding="utf-8", newline="\n")

replace_once(
    "src/reconciliation/mod.rs",
    "mod engine;\nmod model;\n",
    "mod engine;\nmod model;\nmod spare_guard;\n",
)
replace_once(
    "src/reconciliation/mod.rs",
    "pub use model::{plan, DesktopId, DesktopState, Mutation, Occupancy, Plan};\n",
    "pub use model::{plan, DesktopId, DesktopState, Mutation, Occupancy, Plan};\n"
    "pub use spare_guard::{SpareGuard, SpareGuardResult, WindowToken};\n",
)

replace_once(
    "src/windows/inventory.rs",
    "use std::collections::HashMap;\n",
    "use std::collections::{HashMap, HashSet};\n",
)

old_snapshot = r'''// Function purpose: Builds a fresh ordered desktop snapshot with current occupancy and empty-grace state.
pub fn snapshot(
    backend: &WinvdBackend,
    config: &Config,
    grace: &HashMap<DesktopId, bool>,
) -> Result<Vec<DesktopState>, String> {
    let desktops = backend.list()?;
    let current = backend.current()?;
    let mut occupancy: HashMap<DesktopId, Occupancy> = desktops
        .iter()
        .map(|desktop| (desktop.id.clone(), Occupancy::Empty))
        .collect();

    for hwnd in enumerate_windows() {
        let Some(identity) = inspect_identity(hwnd) else {
            continue;
        };
        if identity.process_id == current_process_id()
            || ignored_class(&identity.class_name)
            || config
                .windows
                .ignore_classes
                .iter()
                .any(|value| value.eq_ignore_ascii_case(&identity.class_name))
            || !is_eligible_application_window(hwnd)
        {
            continue;
        }

        if let Ok(executable) = executable_name(identity.process_id) {
            if ignored_shell_executable(&executable)
                || config
                    .windows
                    .ignore_executables
                    .iter()
                    .any(|value| value.eq_ignore_ascii_case(&executable))
            {
                continue;
            }
        }

        if backend.is_window_pinned(hwnd).is_ok_and(|pinned| pinned) {
            continue;
        }

        if let Some(desktop) = locate_window_desktop(backend, &desktops, &current.id, hwnd) {
            occupancy.insert(desktop, Occupancy::Occupied);
        }
    }

    Ok(desktops
        .into_iter()
        .map(|desktop| DesktopState {
            current: desktop.id == current.id,
            empty_grace_elapsed: grace.get(&desktop.id).copied().unwrap_or(false),
            occupancy: occupancy.remove(&desktop.id).unwrap_or(Occupancy::Empty),
            id: desktop.id,
        })
        .collect())
}
'''
new_snapshot = r'''#[derive(Debug)]
pub struct DesktopInventory {
    pub states: Vec<DesktopState>,
    pub windows: HashMap<DesktopId, HashSet<crate::reconciliation::WindowToken>>,
}

// Function purpose: Builds a fresh ordered desktop snapshot with current occupancy and empty-grace state.
pub fn snapshot(
    backend: &WinvdBackend,
    config: &Config,
    grace: &HashMap<DesktopId, bool>,
) -> Result<Vec<DesktopState>, String> {
    detailed_snapshot(backend, config, grace).map(|inventory| inventory.states)
}

// Function purpose: Builds desktop occupancy together with stable window tokens used to distinguish real application creation from switch-time shell noise.
pub fn detailed_snapshot(
    backend: &WinvdBackend,
    config: &Config,
    grace: &HashMap<DesktopId, bool>,
) -> Result<DesktopInventory, String> {
    let desktops = backend.list()?;
    let current = backend.current()?;
    let mut occupancy: HashMap<DesktopId, Occupancy> = desktops
        .iter()
        .map(|desktop| (desktop.id.clone(), Occupancy::Empty))
        .collect();
    let mut windows: HashMap<DesktopId, HashSet<crate::reconciliation::WindowToken>> = desktops
        .iter()
        .map(|desktop| (desktop.id.clone(), HashSet::new()))
        .collect();

    for hwnd in enumerate_windows() {
        let Some(identity) = inspect_identity(hwnd) else {
            continue;
        };
        if identity.process_id == current_process_id()
            || ignored_class(&identity.class_name)
            || config
                .windows
                .ignore_classes
                .iter()
                .any(|value| value.eq_ignore_ascii_case(&identity.class_name))
            || !is_eligible_application_window(hwnd)
        {
            continue;
        }

        if let Ok(executable) = executable_name(identity.process_id) {
            if ignored_shell_executable(&executable)
                || config
                    .windows
                    .ignore_executables
                    .iter()
                    .any(|value| value.eq_ignore_ascii_case(&executable))
            {
                continue;
            }
        }

        if backend.is_window_pinned(hwnd).is_ok_and(|pinned| pinned) {
            continue;
        }

        if let Some(desktop) = locate_window_desktop(backend, &desktops, &current.id, hwnd) {
            occupancy.insert(desktop.clone(), Occupancy::Occupied);
            windows
                .entry(desktop)
                .or_default()
                .insert(hwnd as usize as crate::reconciliation::WindowToken);
        }
    }

    let states = desktops
        .into_iter()
        .map(|desktop| DesktopState {
            current: desktop.id == current.id,
            empty_grace_elapsed: grace.get(&desktop.id).copied().unwrap_or(false),
            occupancy: occupancy.remove(&desktop.id).unwrap_or(Occupancy::Empty),
            id: desktop.id,
        })
        .collect();
    Ok(DesktopInventory { states, windows })
}
'''
replace_once("src/windows/inventory.rs", old_snapshot, new_snapshot)

replace_once(
    "src/windows/inventory.rs",
    "            && cloaked != 0\n",
    "            && cloaked & 0x2 != 0\n",
)

Path("src/windows/window_events.rs").write_text(r'''// File purpose: Listens for native top-level window lifecycle events and reports stable window tokens for event-confirmed occupancy.
use std::mem::zeroed;
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::mpsc::Sender;
use std::sync::OnceLock;
use std::thread::{self, JoinHandle};
use windows_sys::Win32::Foundation::HWND;
use windows_sys::Win32::System::Threading::GetCurrentThreadId;
use windows_sys::Win32::UI::Accessibility::{SetWinEventHook, UnhookWinEvent, HWINEVENTHOOK};
use windows_sys::Win32::UI::WindowsAndMessaging::{
    DispatchMessageW, GetMessageW, PostThreadMessageW, TranslateMessage, CHILDID_SELF,
    EVENT_OBJECT_CREATE, EVENT_OBJECT_HIDE, EVENT_OBJECT_SHOW, MSG, OBJID_WINDOW,
    WINEVENT_OUTOFCONTEXT, WINEVENT_SKIPOWNPROCESS, WM_QUIT,
};

use crate::reconciliation::WindowToken;

static EVENT_SENDER: OnceLock<Sender<WindowEvent>> = OnceLock::new();

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct WindowEvent {
    token: WindowToken,
    event: u32,
}

impl WindowEvent {
    // Function purpose: Returns the stable native token used to correlate this event with a later desktop inventory snapshot.
    pub fn token(self) -> WindowToken {
        self.token
    }

    // Function purpose: Reports whether the event can represent a newly opened or newly shown user application window.
    pub fn occupancy_gain(self) -> bool {
        self.event == EVENT_OBJECT_CREATE || self.event == EVENT_OBJECT_SHOW
    }
}

pub struct WindowEventController {
    thread_id: AtomicU32,
    thread: Option<JoinHandle<Result<(), String>>>,
}

impl WindowEventController {
    // Function purpose: Starts the component and returns the controller used to update or stop it.
    pub fn start(sender: Sender<WindowEvent>) -> Result<Self, String> {
        EVENT_SENDER
            .set(sender)
            .map_err(|_| "window event sender already initialized".to_string())?;
        let thread_id = AtomicU32::new(0);
        let (ready_tx, ready_rx) = std::sync::mpsc::channel();
        let thread = thread::Builder::new()
            .name("deskpilot-window-events".to_string())
            .spawn(move || run_loop(ready_tx))
            .map_err(|error| error.to_string())?;
        let registered_thread_id = ready_rx.recv().map_err(|error| error.to_string())??;
        thread_id.store(registered_thread_id, Ordering::Release);
        Ok(Self {
            thread_id,
            thread: Some(thread),
        })
    }

    // Function purpose: Stops the component, signals its worker thread, and waits for native resources to be released.
    pub fn stop(&mut self) {
        let thread_id = self.thread_id.load(Ordering::Acquire);
        if thread_id != 0 {
            unsafe { PostThreadMessageW(thread_id, WM_QUIT, 0, 0) };
        }
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }
}

impl Drop for WindowEventController {
    // Function purpose: Releases the native or background resource owned by this value when it leaves scope.
    fn drop(&mut self) {
        self.stop();
    }
}

// Function purpose: Performs the run loop operation required by this module.
fn run_loop(ready: Sender<Result<u32, String>>) -> Result<(), String> {
    unsafe {
        let hook = SetWinEventHook(
            EVENT_OBJECT_CREATE,
            EVENT_OBJECT_HIDE,
            0,
            Some(window_event_proc),
            0,
            0,
            WINEVENT_OUTOFCONTEXT | WINEVENT_SKIPOWNPROCESS,
        );
        if hook == 0 {
            let error = "SetWinEventHook failed".to_string();
            let _ = ready.send(Err(error.clone()));
            return Err(error);
        }
        let thread_id = GetCurrentThreadId();
        let _ = ready.send(Ok(thread_id));
        let mut message: MSG = zeroed();
        while GetMessageW(&mut message, 0, 0, 0) > 0 {
            TranslateMessage(&message);
            DispatchMessageW(&message);
        }
        UnhookWinEvent(hook);
    }
    Ok(())
}

// Function purpose: Filters native top-level window lifecycle callbacks and forwards the event plus a stable window token to the main loop.
unsafe extern "system" fn window_event_proc(
    _hook: HWINEVENTHOOK,
    event: u32,
    hwnd: HWND,
    object_id: i32,
    child_id: i32,
    _event_thread: u32,
    _event_time: u32,
) {
    if hwnd != 0 && object_id == OBJID_WINDOW && child_id == CHILDID_SELF as i32 {
        if let Some(sender) = EVENT_SENDER.get() {
            let _ = sender.send(WindowEvent {
                token: hwnd as usize as WindowToken,
                event,
            });
        }
    }
}

#[cfg(test)]
mod tests {
    use super::WindowEvent;
    use windows_sys::Win32::UI::WindowsAndMessaging::{
        EVENT_OBJECT_CREATE, EVENT_OBJECT_HIDE, EVENT_OBJECT_SHOW,
    };

    // Function purpose: Verifies only create and show events can authorize consumption of the protected spare.
    #[test]
    fn occupancy_gain_requires_create_or_show() {
        assert!(WindowEvent {
            token: 1,
            event: EVENT_OBJECT_CREATE,
        }
        .occupancy_gain());
        assert!(WindowEvent {
            token: 2,
            event: EVENT_OBJECT_SHOW,
        }
        .occupancy_gain());
        assert!(!WindowEvent {
            token: 3,
            event: EVENT_OBJECT_HIDE,
        }
        .occupancy_gain());
    }
}
''', encoding="utf-8", newline="\n")

replace_once(
    "src/app.rs",
    "    DesktopId, Occupancy, ReconcileBackend, ReconcilePass, ReconcileRuntime,\n",
    "    DesktopId, Occupancy, ReconcileBackend, ReconcilePass, ReconcileRuntime, SpareGuard,\n"
    "    WindowToken,\n",
)
replace_once(
    "src/app.rs",
    "    hooks::HookController, inventory, startup, system, window_events::WindowEventController,\n",
    "    hooks::HookController, inventory, startup, system,\n"
    "    window_events::{WindowEvent, WindowEventController},\n",
)
replace_once(
    "src/app.rs",
    "    WindowEvent,\n",
    "    WindowEvent(WindowEvent),\n",
)
replace_once(
    "src/app.rs",
    "    bridge(window_rx, signal_tx.clone(), |_| AppSignal::WindowEvent);\n",
    "    bridge(window_rx, signal_tx.clone(), AppSignal::WindowEvent);\n",
)
replace_once(
    "src/app.rs",
    "        reconciler: ReconcileRuntime::default(),\n        last_reconciliation: None,\n",
    "        reconciler: ReconcileRuntime::default(),\n"
    "        spare_guard: SpareGuard::default(),\n"
    "        window_candidates: HashMap::new(),\n"
    "        last_reconciliation: None,\n",
)
replace_once(
    "src/app.rs",
    "            Ok(AppSignal::DesktopEvent | AppSignal::WindowEvent) => {\n"
    "                let delay = state.config_read().desktops.reconcile_delay_ms;\n"
    "                schedule_reconcile_at_earliest(\n"
    "                    &mut pending_reconcile,\n"
    "                    Instant::now() + Duration::from_millis(delay),\n"
    "                );\n"
    "            }\n",
    "            Ok(AppSignal::DesktopEvent) => {\n"
    "                let delay = state.config_read().desktops.reconcile_delay_ms;\n"
    "                schedule_reconcile_at_earliest(\n"
    "                    &mut pending_reconcile,\n"
    "                    Instant::now() + Duration::from_millis(delay),\n"
    "                );\n"
    "            }\n"
    "            Ok(AppSignal::WindowEvent(event)) => {\n"
    "                state.note_window_event(event);\n"
    "                let delay = state.config_read().desktops.reconcile_delay_ms;\n"
    "                schedule_reconcile_at_earliest(\n"
    "                    &mut pending_reconcile,\n"
    "                    Instant::now() + Duration::from_millis(delay),\n"
    "                );\n"
    "            }\n",
)
replace_once(
    "src/app.rs",
    "    reconciler: ReconcileRuntime,\n    last_reconciliation: Option<String>,\n",
    "    reconciler: ReconcileRuntime,\n"
    "    spare_guard: SpareGuard,\n"
    "    window_candidates: HashMap<WindowToken, Instant>,\n"
    "    last_reconciliation: Option<String>,\n",
)
replace_once(
    "src/app.rs",
    "    empty_since: &'a mut HashMap<DesktopId, Instant>,\n}\n",
    "    empty_since: &'a mut HashMap<DesktopId, Instant>,\n"
    "    spare_guard: &'a mut SpareGuard,\n"
    "    window_candidates: &'a HashSet<WindowToken>,\n"
    "}\n",
)
replace_once(
    "src/app.rs",
    "        let mut states = inventory::snapshot(self.backend, self.config, &HashMap::new())?;\n",
    "        let detailed = inventory::detailed_snapshot(self.backend, self.config, &HashMap::new())?;\n"
    "        let mut states = detailed.states;\n",
)
replace_once(
    "src/app.rs",
    "        Ok(states)\n    }\n\n    // Function purpose: Creates exactly one desktop",
    "        self.spare_guard.stabilize(\n"
    "            &mut states,\n"
    "            &detailed.windows,\n"
    "            self.window_candidates,\n"
    "        );\n"
    "        Ok(states)\n"
    "    }\n\n"
    "    // Function purpose: Creates exactly one desktop",
)
replace_once(
    "src/app.rs",
    "    fn config_read(&self) -> Config {\n"
    "        self.config\n"
    "            .read()\n"
    "            .map_or_else(|_| Config::default(), |config| config.clone())\n"
    "    }\n",
    "    fn config_read(&self) -> Config {\n"
    "        self.config\n"
    "            .read()\n"
    "            .map_or_else(|_| Config::default(), |config| config.clone())\n"
    "    }\n\n"
    "    // Function purpose: Records only create or show events as short-lived evidence that a real eligible window may have consumed the protected spare.\n"
    "    fn note_window_event(&mut self, event: WindowEvent) {\n"
    "        if event.occupancy_gain() {\n"
    "            self.window_candidates.insert(event.token(), Instant::now());\n"
    "        }\n"
    "    }\n\n"
    "    // Function purpose: Expires stale native window tokens so unrelated historical events cannot consume a future spare.\n"
    "    fn prune_window_candidates(&mut self) {\n"
    "        let now = Instant::now();\n"
    "        self.window_candidates\n"
    "            .retain(|_, seen| now.duration_since(*seen) <= Duration::from_secs(5));\n"
    "    }\n",
)
old_navigate = r'''        match self.backend.switch_relative(step, config.wheel.navigation) {
            Ok(desktop) => self.publish(
                "desktop-switched",
                format!("desktop index {}", desktop.index),
            ),
            Err(error) if error.contains("clamped edge") => {}
            Err(error) => self.error(format!("desktop navigation failed: {error}")),
        }
'''
new_navigate = r'''        match self.backend.switch_relative(step, config.wheel.navigation) {
            Ok(desktop) => {
                let is_last = self
                    .backend
                    .list()
                    .is_ok_and(|desktops| desktop.index + 1 == desktops.len());
                if is_last {
                    self.spare_guard.arm(desktop.id.clone());
                }
                self.publish(
                    "desktop-switched",
                    format!("desktop index {}", desktop.index),
                );
            }
            Err(error) if error.contains("clamped edge") => {}
            Err(error) => self.error(format!("desktop navigation failed: {error}")),
        }
'''
replace_once("src/app.rs", old_navigate, new_navigate)

replace_once(
    "src/app.rs",
    "        let config = self.config_read();\n        let result = {\n",
    "        self.prune_window_candidates();\n"
    "        let window_candidates: HashSet<_> = self.window_candidates.keys().copied().collect();\n"
    "        let config = self.config_read();\n"
    "        let result = {\n",
)
replace_once(
    "src/app.rs",
    "                empty_since: &mut self.empty_since,\n            };\n            self.reconciler.reconcile_once(&mut backend)\n",
    "                empty_since: &mut self.empty_since,\n"
    "                spare_guard: &mut self.spare_guard,\n"
    "                window_candidates: &window_candidates,\n"
    "            };\n"
    "            self.reconciler.reconcile_once(&mut backend)\n",
)
replace_once(
    "src/app.rs",
    "    fn snapshot(&mut self) -> Result<Vec<crate::reconciliation::DesktopState>, String> {\n"
    "        let config = self.config_read();\n"
    "        let mut backend = AppReconcileBackend {\n"
    "            backend: &self.backend,\n"
    "            config: &config,\n"
    "            empty_since: &mut self.empty_since,\n"
    "        };\n"
    "        backend.snapshot()\n"
    "    }\n",
    "    fn snapshot(&mut self) -> Result<Vec<crate::reconciliation::DesktopState>, String> {\n"
    "        self.prune_window_candidates();\n"
    "        let window_candidates: HashSet<_> = self.window_candidates.keys().copied().collect();\n"
    "        let config = self.config_read();\n"
    "        let mut backend = AppReconcileBackend {\n"
    "            backend: &self.backend,\n"
    "            config: &config,\n"
    "            empty_since: &mut self.empty_since,\n"
    "            spare_guard: &mut self.spare_guard,\n"
    "            window_candidates: &window_candidates,\n"
    "        };\n"
    "        backend.snapshot()\n"
    "    }\n",
)

Path("tests/spare_guard.rs").write_text(r'''// File purpose: Reproduces repeated scroll visits to a noisy empty spare and proves only event-confirmed user windows can consume it.
use deskpilot::reconciliation::{
    plan, DesktopId, DesktopState, Mutation, Occupancy, SpareGuard, WindowToken,
};
use std::collections::{HashMap, HashSet};

fn state(index: usize, occupancy: Occupancy, current: bool) -> DesktopState {
    DesktopState {
        id: DesktopId(format!("desktop-{index}")),
        occupancy,
        current,
        empty_grace_elapsed: true,
    }
}

// Function purpose: Verifies repeated scroll visits cannot convert switch-time shell noise into additional desktop creation.
#[test]
fn one_app_plus_repeated_scroll_stays_at_two_desktops() {
    let mut guard = SpareGuard::default();
    let spare = DesktopId("desktop-1".to_string());
    let candidates = HashSet::new();

    let mut initial = vec![
        state(0, Occupancy::Occupied, true),
        state(1, Occupancy::Empty, false),
    ];
    guard.stabilize(&mut initial, &HashMap::new(), &candidates);
    assert_eq!(guard.protected(), Some(&spare));

    for iteration in 0..200 {
        guard.arm(spare.clone());
        let mut noisy = vec![
            state(0, Occupancy::Occupied, iteration % 2 == 0),
            state(1, Occupancy::Occupied, iteration % 2 != 0),
        ];
        let windows = HashMap::from([(spare.clone(), HashSet::from([9001]))]);
        let result = guard.stabilize(&mut noisy, &windows, &candidates);
        assert!(result.protecting);
        assert!(!result.consumed);
        assert_eq!(noisy[1].occupancy, Occupancy::Empty);
        assert!(plan(&noisy).mutations.is_empty());
    }
}

// Function purpose: Verifies a qualifying create or show event mapped to the spare consumes it and authorizes exactly one new trailing desktop.
#[test]
fn real_window_event_consumes_spare_once() {
    let mut guard = SpareGuard::default();
    let spare = DesktopId("desktop-1".to_string());
    guard.arm(spare.clone());
    let token: WindowToken = 42;
    let windows = HashMap::from([(spare.clone(), HashSet::from([token]))]);
    let candidates = HashSet::from([token]);
    let mut states = vec![
        state(0, Occupancy::Occupied, false),
        state(1, Occupancy::Occupied, true),
    ];

    let result = guard.stabilize(&mut states, &windows, &candidates);
    assert!(result.consumed);
    assert!(!result.protecting);
    assert_eq!(guard.protected(), None);
    assert_eq!(plan(&states).mutations, vec![Mutation::CreateTrailing]);
}

// Function purpose: Verifies unrelated application events on another desktop cannot consume the protected spare.
#[test]
fn unrelated_window_event_does_not_consume_spare() {
    let mut guard = SpareGuard::default();
    let spare = DesktopId("desktop-1".to_string());
    guard.arm(spare.clone());
    let mut states = vec![
        state(0, Occupancy::Occupied, true),
        state(1, Occupancy::Occupied, false),
    ];
    let windows = HashMap::from([
        (DesktopId("desktop-0".to_string()), HashSet::from([77])),
        (spare.clone(), HashSet::from([9001])),
    ]);
    let candidates = HashSet::from([77]);

    let result = guard.stabilize(&mut states, &windows, &candidates);
    assert!(result.protecting);
    assert!(!result.consumed);
    assert_eq!(states[1].occupancy, Occupancy::Empty);
    assert!(plan(&states).mutations.is_empty());
}

// Function purpose: Verifies the guard follows a newly created trailing spare and does not protect a stale internal desktop.
#[test]
fn guard_moves_to_new_trailing_spare() {
    let mut guard = SpareGuard::default();
    guard.arm(DesktopId("desktop-1".to_string()));
    let mut states = vec![
        state(0, Occupancy::Occupied, true),
        state(1, Occupancy::Occupied, false),
        state(2, Occupancy::Empty, false),
    ];

    let result = guard.stabilize(&mut states, &HashMap::new(), &HashSet::new());
    assert!(result.protecting);
    assert_eq!(guard.protected(), Some(&DesktopId("desktop-2".to_string())));
    assert_eq!(states[2].occupancy, Occupancy::Empty);
}
''', encoding="utf-8", newline="\n")

replace_once("Cargo.toml", 'version = "0.1.6"\n', 'version = "0.1.7"\n')
replace_once(
    "Cargo.lock",
    'name = "deskpilot"\nversion = "0.1.6"\n',
    'name = "deskpilot"\nversion = "0.1.7"\n',
)
replace_once(
    ".github/workflows/ci.yml",
    "DeskPilot 0.1.6",
    "DeskPilot 0.1.7",
)

replace_once(
    "CHANGELOG.md",
    "# Changelog\n\n## 0.1.6\n",
    "# Changelog\n\n"
    "## 0.1.7\n\n"
    "- Protect the trailing spare from switch-time occupancy noise and require a qualifying native window create or show event before treating it as consumed.\n"
    "- Stop repeated Win+wheel visits to an empty desktop from creating additional desktops when only one user application exists.\n"
    "- Add deterministic virtual tests for two hundred noisy scroll visits, real-window consumption, unrelated events, and spare replacement.\n"
    "- Count only shell-cloaked inactive application windows instead of every DWM-cloaked helper surface.\n\n"
    "## 0.1.6\n",
)

print("event-confirmed spare occupancy patch applied")
