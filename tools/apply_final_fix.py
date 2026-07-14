# File purpose: Applies the race-proof reconciliation, delayed-topology tests, and complete Win-scroll capture changes to the feature branch.
from pathlib import Path


# Function purpose: Reads one repository text file using the repository UTF-8 convention.
def read(path: str) -> str:
    return Path(path).read_text(encoding="utf-8")


# Function purpose: Writes one repository text file with UTF-8 and LF line endings.
def write(path: str, text: str) -> None:
    Path(path).write_text(text, encoding="utf-8", newline="\n")


# Function purpose: Replaces exactly one expected fragment and fails rather than silently patching an unexpected tree.
def replace_once(path: str, old: str, new: str) -> None:
    text = read(path)
    count = text.count(old)
    if count != 1:
        raise SystemExit(f"{path}: expected one replacement, found {count}: {old[:100]!r}")
    write(path, text.replace(old, new, 1))


write(
    "src/reconciliation/engine.rs",
    '''// File purpose: Executes one race-proof reconciliation mutation at a time and waits until Windows exposes the resulting topology.
use thiserror::Error;

use super::{plan, DesktopId, DesktopState, Mutation};

pub trait ReconcileBackend {
    // Function purpose: Builds a fresh ordered desktop snapshot with current occupancy and empty-grace state.
    fn snapshot(&mut self) -> Result<Vec<DesktopState>, String>;
    // Function purpose: Creates one desktop and returns the identifier reported by the backend.
    fn create_desktop(&mut self) -> Result<DesktopId, String>;
    // Function purpose: Switches to a specific desktop when an explicit plan requests it.
    fn switch_desktop(&mut self, desktop: &DesktopId) -> Result<(), String>;
    // Function purpose: Removes one desktop using the supplied safe fallback desktop.
    fn remove_desktop(&mut self, desktop: &DesktopId, fallback: &DesktopId) -> Result<(), String>;
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct ReconcileReport {
    pub iterations: usize,
    pub mutations: Vec<Mutation>,
    pub stable: bool,
}

#[derive(Debug, Error, PartialEq, Eq)]
pub enum ReconcileError {
    #[error("snapshot failed: {0}")]
    Snapshot(String),
    #[error("mutation failed: {operation}: {cause}")]
    Mutation { operation: String, cause: String },
    #[error("reconciliation exceeded {0} iterations")]
    IterationLimit(usize),
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum PendingTopologyMutation {
    Create {
        expected: DesktopId,
        baseline_count: usize,
    },
    Remove {
        desktop: DesktopId,
    },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ReconcilePass {
    Stable,
    Blocked,
    WaitingForTopology,
    Mutated(Mutation),
}

#[derive(Debug, Default)]
pub struct ReconcileRuntime {
    pending: Option<PendingTopologyMutation>,
}

impl ReconcileRuntime {
    // Function purpose: Applies at most one mutation and refuses further mutations until a later snapshot confirms the previous topology change.
    pub fn reconcile_once<B: ReconcileBackend>(
        &mut self,
        backend: &mut B,
    ) -> Result<ReconcilePass, ReconcileError> {
        let snapshot = backend.snapshot().map_err(ReconcileError::Snapshot)?;
        if !self.pending_observed(&snapshot) {
            return Ok(ReconcilePass::WaitingForTopology);
        }

        let next = plan(&snapshot);
        if next.stable {
            return Ok(ReconcilePass::Stable);
        }
        let Some(mutation) = next.mutations.into_iter().next() else {
            return Ok(ReconcilePass::Blocked);
        };

        match &mutation {
            Mutation::CreateTrailing => {
                let created = backend.create_desktop().map_err(|cause| {
                    ReconcileError::Mutation {
                        operation: format!("{mutation:?}"),
                        cause,
                    }
                })?;
                self.pending = Some(PendingTopologyMutation::Create {
                    expected: created,
                    baseline_count: snapshot.len(),
                });
            }
            Mutation::Remove { desktop, fallback } => {
                backend
                    .remove_desktop(desktop, fallback)
                    .map_err(|cause| ReconcileError::Mutation {
                        operation: format!("{mutation:?}"),
                        cause,
                    })?;
                self.pending = Some(PendingTopologyMutation::Remove {
                    desktop: desktop.clone(),
                });
            }
            Mutation::Switch { desktop } => {
                backend
                    .switch_desktop(desktop)
                    .map_err(|cause| ReconcileError::Mutation {
                        operation: format!("{mutation:?}"),
                        cause,
                    })?;
            }
        }

        Ok(ReconcilePass::Mutated(mutation))
    }

    // Function purpose: Clears the in-flight barrier only after a snapshot proves that the requested create or remove became visible.
    fn pending_observed(&mut self, snapshot: &[DesktopState]) -> bool {
        let observed = match &self.pending {
            None => true,
            Some(PendingTopologyMutation::Create {
                expected,
                baseline_count,
            }) => {
                snapshot.iter().any(|desktop| &desktop.id == expected)
                    || snapshot.len() > *baseline_count
            }
            Some(PendingTopologyMutation::Remove { desktop }) => {
                snapshot.iter().all(|state| &state.id != desktop)
            }
        };
        if observed {
            self.pending = None;
        }
        observed
    }

    // Function purpose: Reports whether a successful mutation is still awaiting confirmation from Windows.
    pub fn is_waiting_for_topology(&self) -> bool {
        self.pending.is_some()
    }
}

// Function purpose: Runs deterministic backends to convergence while preserving the same one-mutation and observation barrier used by the application.
pub fn apply_plan<B: ReconcileBackend>(
    backend: &mut B,
    max_iterations: usize,
) -> Result<ReconcileReport, ReconcileError> {
    let mut report = ReconcileReport::default();
    let mut runtime = ReconcileRuntime::default();

    for iteration in 0..max_iterations {
        report.iterations = iteration + 1;
        match runtime.reconcile_once(backend)? {
            ReconcilePass::Stable => {
                report.stable = true;
                return Ok(report);
            }
            ReconcilePass::Blocked | ReconcilePass::WaitingForTopology => return Ok(report),
            ReconcilePass::Mutated(mutation) => report.mutations.push(mutation),
        }
    }

    Err(ReconcileError::IterationLimit(max_iterations))
}
''',
)

app = read("src/app.rs")
app = app.replace(
    "use crate::reconciliation::{plan, DesktopId, Mutation, Occupancy};",
    "use crate::reconciliation::{\n    DesktopId, Mutation, Occupancy, ReconcileBackend, ReconcilePass, ReconcileRuntime,\n};",
)
app = app.replace(
    "        empty_since: HashMap::new(),\n        last_reconciliation: None,",
    "        empty_since: HashMap::new(),\n        reconciler: ReconcileRuntime::default(),\n        last_reconciliation: None,",
)
app = app.replace(
    "    empty_since: HashMap<DesktopId, Instant>,\n    last_reconciliation: Option<String>,",
    "    empty_since: HashMap<DesktopId, Instant>,\n    reconciler: ReconcileRuntime,\n    last_reconciliation: Option<String>,",
)
old_loop = '''        if pending_reconcile.is_some_and(|deadline| now >= deadline) {
            if config_snapshot.desktops.dynamic && !state.dynamic_forced_off {
                state.reconcile();
            }
            pending_reconcile = None;
            state.update_tray(tray.as_ref());
        }'''
new_loop = '''        if pending_reconcile.is_some_and(|deadline| now >= deadline) {
            let follow_up = if config_snapshot.desktops.dynamic && !state.dynamic_forced_off {
                matches!(state.reconcile(), ReconcilePass::Mutated(_))
            } else {
                false
            };
            pending_reconcile = follow_up.then(|| {
                Instant::now()
                    + Duration::from_millis(config_snapshot.desktops.reconcile_delay_ms)
            });
            state.update_tray(tray.as_ref());
        }'''
if app.count(old_loop) != 1:
    raise SystemExit("src/app.rs: event-loop reconciliation block not found exactly once")
app = app.replace(old_loop, new_loop, 1)
start_marker = "    // Function purpose: Runs bounded desktop reconciliation and records or publishes the outcome.\n    fn reconcile(&mut self)"
start = app.find(start_marker)
end = app.find("    // Function purpose: Handles tray.", start)
if start < 0 or end < 0:
    raise SystemExit("src/app.rs: reconcile method boundaries not found")
replacement = '''    // Function purpose: Applies one race-proof reconciliation mutation and reports whether a short follow-up pass is required.
    fn reconcile(&mut self) -> ReconcilePass {
        if !self.backend.compatible() {
            return ReconcilePass::Blocked;
        }

        let config = self.config_read();
        let result = {
            let mut backend = AppReconcileBackend {
                backend: &self.backend,
                config: &config,
                empty_since: &mut self.empty_since,
            };
            self.reconciler.reconcile_once(&mut backend)
        };

        match result {
            Ok(ReconcilePass::Mutated(mutation)) => {
                self.last_reconciliation = Some(timestamp_utc());
                self.publish("reconciled", format!("applied {mutation:?}"));
                ReconcilePass::Mutated(mutation)
            }
            Ok(pass) => {
                self.last_reconciliation = Some(timestamp_utc());
                pass
            }
            Err(error) => {
                self.error(format!("reconciliation failed: {error}"));
                ReconcilePass::Blocked
            }
        }
    }

'''
app = app[:start] + replacement + app[end:]
insert_at = app.find("impl AppState {")
if insert_at < 0:
    raise SystemExit("src/app.rs: AppState impl not found")
adapter = '''struct AppReconcileBackend<'a> {
    backend: &'a WinvdBackend,
    config: &'a Config,
    empty_since: &'a mut HashMap<DesktopId, Instant>,
}

impl ReconcileBackend for AppReconcileBackend<'_> {
    // Function purpose: Builds the current desktop snapshot and applies the configured empty-desktop grace period.
    fn snapshot(&mut self) -> Result<Vec<crate::reconciliation::DesktopState>, String> {
        let mut states = inventory::snapshot(self.backend, self.config, &HashMap::new())?;
        let now = Instant::now();
        let existing: HashSet<_> = states.iter().map(|state| state.id.clone()).collect();
        self.empty_since.retain(|id, _| existing.contains(id));
        for state in &mut states {
            if state.occupancy == Occupancy::Empty {
                let since = self.empty_since.entry(state.id.clone()).or_insert(now);
                state.empty_grace_elapsed = now.duration_since(*since)
                    >= Duration::from_millis(self.config.desktops.empty_grace_ms);
            } else {
                self.empty_since.remove(&state.id);
                state.empty_grace_elapsed = false;
            }
        }
        Ok(states)
    }

    // Function purpose: Creates exactly one desktop and returns its stable backend identifier for later observation.
    fn create_desktop(&mut self) -> Result<DesktopId, String> {
        self.backend.create().map(|desktop| desktop.id)
    }

    // Function purpose: Switches only when an explicit reconciliation mutation requests a desktop change.
    fn switch_desktop(&mut self, desktop: &DesktopId) -> Result<(), String> {
        self.backend.switch_to_id(desktop)
    }

    // Function purpose: Removes exactly one confirmed-empty desktop into the supplied safe fallback.
    fn remove_desktop(
        &mut self,
        desktop: &DesktopId,
        fallback: &DesktopId,
    ) -> Result<(), String> {
        self.backend.remove(desktop, fallback)
    }
}

'''
app = app[:insert_at] + adapter + app[insert_at:]
write("src/app.rs", app)

write(
    "src/wheel.rs",
    '''// File purpose: Normalizes wheel deltas, captures Win-modified scroll, applies thresholds and cooldowns, and calculates wrapped targets.
use crate::config::{NavigationMode, WheelDirection};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Step {
    Previous,
    Next,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct WheelGesture {
    pub consume: bool,
    pub step: Option<Step>,
}

#[derive(Debug, Clone, Default)]
pub struct WheelState {
    accumulator: i32,
    last_step_ms: Option<u64>,
}

impl WheelState {
    // Function purpose: Captures every active Win-modified wheel message while emitting navigation only after threshold and cooldown rules pass.
    pub fn gesture(
        &mut self,
        modifier_active: bool,
        delta: i32,
        now_ms: u64,
        threshold: i32,
        cooldown_ms: u64,
        direction: WheelDirection,
    ) -> WheelGesture {
        if !modifier_active {
            self.reset();
            return WheelGesture {
                consume: false,
                step: None,
            };
        }
        WheelGesture {
            consume: true,
            step: self.feed(delta, now_ms, threshold, cooldown_ms, direction),
        }
    }

    // Function purpose: Accumulates high-resolution deltas and emits one normalized navigation step when permitted.
    pub fn feed(
        &mut self,
        delta: i32,
        now_ms: u64,
        threshold: i32,
        cooldown_ms: u64,
        direction: WheelDirection,
    ) -> Option<Step> {
        if threshold <= 0 || delta == 0 {
            return None;
        }
        self.accumulator = self.accumulator.saturating_add(delta);
        if self.accumulator.unsigned_abs() < threshold.unsigned_abs() {
            return None;
        }
        if self
            .last_step_ms
            .is_some_and(|last| now_ms.saturating_sub(last) < cooldown_ms)
        {
            self.accumulator = self.accumulator.clamp(-threshold + 1, threshold - 1);
            return None;
        }
        let positive = self.accumulator > 0;
        self.accumulator = 0;
        self.last_step_ms = Some(now_ms);
        Some(match (positive, direction) {
            (true, WheelDirection::Normal) | (false, WheelDirection::Inverted) => Step::Previous,
            (false, WheelDirection::Normal) | (true, WheelDirection::Inverted) => Step::Next,
        })
    }

    // Function purpose: Discards partial wheel movement when the Win gesture is inactive or suspended.
    pub fn reset(&mut self) {
        self.accumulator = 0;
    }
}

// Function purpose: Calculates the destination index for clamped or circular desktop navigation.
pub fn target_index(
    current: usize,
    count: usize,
    step: Step,
    mode: NavigationMode,
) -> Option<usize> {
    if count == 0 || current >= count {
        return None;
    }
    match (step, mode) {
        (Step::Previous, NavigationMode::Clamp) => current.checked_sub(1),
        (Step::Next, NavigationMode::Clamp) => (current + 1 < count).then_some(current + 1),
        (Step::Previous, NavigationMode::Wrap) => {
            Some(if current == 0 { count - 1 } else { current - 1 })
        }
        (Step::Next, NavigationMode::Wrap) => Some((current + 1) % count),
    }
}
''',
)

hooks = read("src/windows/hooks.rs")
replace_import_old = "WH_KEYBOARD_LL, WH_MOUSE_LL, WM_KEYDOWN, WM_KEYUP, WM_MOUSEWHEEL, WM_QUIT, WM_SYSKEYDOWN,\n    WM_SYSKEYUP,"
replace_import_new = "WH_KEYBOARD_LL, WH_MOUSE_LL, WM_KEYDOWN, WM_KEYUP, WM_MOUSEHWHEEL, WM_MOUSEWHEEL,\n    WM_QUIT, WM_SYSKEYDOWN, WM_SYSKEYUP,"
if hooks.count(replace_import_old) != 1:
    raise SystemExit("src/windows/hooks.rs: wheel import fragment not found")
hooks = hooks.replace(replace_import_old, replace_import_new, 1)
old_mouse_start = hooks.find("// Function purpose: Handles low-level wheel events, recognizes Win+wheel steps, queues navigation, and consumes handled input.\nunsafe extern \"system\" fn mouse_proc")
old_mouse_end = hooks.find("// Function purpose: Resets wheel.", old_mouse_start)
if old_mouse_start < 0 or old_mouse_end < 0:
    raise SystemExit("src/windows/hooks.rs: mouse callback boundaries not found")
new_mouse = '''// Function purpose: Consumes every vertical or horizontal scroll message during an active Win gesture and queues vertical navigation steps asynchronously.
unsafe extern "system" fn mouse_proc(code: i32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    let message = wparam as u32;
    if code == HC_ACTION as i32 && is_scroll_message(message) {
        if let Some(context) = CONTEXT.get() {
            let active = context.enabled.load(Ordering::Acquire)
                && context.backend_ready.load(Ordering::Acquire)
                && !context.suspended.load(Ordering::Acquire);
            if !active || !win_pressed(context) {
                reset_wheel(context);
                return unsafe { CallNextHookEx(0 as HHOOK, code, wparam, lparam) };
            }

            context.consumed_win_chord.store(true, Ordering::Release);
            if message == WM_MOUSEWHEEL {
                let event = unsafe { &*(lparam as *const MSLLHOOKSTRUCT) };
                let delta = ((event.mouseData >> 16) as u16) as i16 as i32;
                let config = context.config.read().ok().map(|value| value.wheel.clone());
                if let Some(config) = config {
                    if let Ok(mut wheel) = context.wheel.try_lock() {
                        let gesture = wheel.gesture(
                            true,
                            delta,
                            unsafe { GetTickCount64() },
                            config.threshold,
                            config.cooldown_ms,
                            config.direction,
                        );
                        if let Some(step) = gesture.step {
                            let _ = context.navigation.send(step);
                        }
                    }
                }
            }
            return 1;
        }
    }
    unsafe { CallNextHookEx(0 as HHOOK, code, wparam, lparam) }
}

// Function purpose: Identifies all mouse-wheel messages that must be blocked from the application beneath the pointer during a Win gesture.
fn is_scroll_message(message: u32) -> bool {
    message == WM_MOUSEWHEEL || message == WM_MOUSEHWHEEL
}

'''
hooks = hooks[:old_mouse_start] + new_mouse + hooks[old_mouse_end:]
hooks = hooks.replace(
    "    use super::suppressed_win_release_inputs;",
    "    use super::{is_scroll_message, suppressed_win_release_inputs};",
    1,
)
hooks = hooks.replace(
    "    use windows_sys::Win32::UI::Input::KeyboardAndMouse::{KEYEVENTF_KEYUP, VK_CONTROL, VK_LWIN};",
    "    use windows_sys::Win32::UI::Input::KeyboardAndMouse::{KEYEVENTF_KEYUP, VK_CONTROL, VK_LWIN};\n    use windows_sys::Win32::UI::WindowsAndMessaging::{WM_MOUSEHWHEEL, WM_MOUSEWHEEL};",
    1,
)
test_marker = "    // Function purpose: Verifies the start suppression replaces physical win up with control chord scenario and its expected safety or state invariant.\n    #[test]\n    fn start_suppression_replaces_physical_win_up_with_control_chord()"
if hooks.count(test_marker) != 1:
    raise SystemExit("src/windows/hooks.rs: suppression test marker not found")
hooks = hooks.replace(
    test_marker,
    "    // Function purpose: Verifies that both vertical and horizontal wheel messages are classified as scroll that must be captured.\n    #[test]\n    fn captures_vertical_and_horizontal_scroll_messages() {\n        assert!(is_scroll_message(WM_MOUSEWHEEL));\n        assert!(is_scroll_message(WM_MOUSEHWHEEL));\n        assert!(!is_scroll_message(0));\n    }\n\n" + test_marker,
    1,
)
write("src/windows/hooks.rs", hooks)

wheel_tests = read("tests/wheel.rs")
wheel_tests += '''

// Function purpose: Verifies that partial high-resolution Win+wheel input is consumed even before it produces a desktop step.
#[test]
fn partial_win_wheel_is_consumed_without_navigation() {
    let mut state = WheelState::default();
    let gesture = state.gesture(true, 30, 1_000, 120, 180, WheelDirection::Normal);
    assert!(gesture.consume);
    assert_eq!(gesture.step, None);
}

// Function purpose: Verifies that cooldown-suppressed wheel messages remain captured and cannot scroll the foreground application.
#[test]
fn cooldown_wheel_is_consumed_without_leaking_to_application() {
    let mut state = WheelState::default();
    let first = state.gesture(true, -120, 1_000, 120, 180, WheelDirection::Normal);
    let second = state.gesture(true, -120, 1_050, 120, 180, WheelDirection::Normal);
    assert!(first.consume);
    assert_eq!(first.step, Some(Step::Next));
    assert!(second.consume);
    assert_eq!(second.step, None);
}

// Function purpose: Verifies that ordinary wheel input remains available to applications whenever Win is not part of the gesture.
#[test]
fn ordinary_wheel_without_win_is_not_consumed() {
    let mut state = WheelState::default();
    let gesture = state.gesture(false, -120, 1_000, 120, 180, WheelDirection::Normal);
    assert!(!gesture.consume);
    assert_eq!(gesture.step, None);
}
'''
write("tests/wheel.rs", wheel_tests)

write(
    "tests/virtual_reconciliation.rs",
    '''// File purpose: Simulates delayed Windows topology publication and verifies that event or scroll storms cannot create or remove multiple desktops concurrently.
use deskpilot::reconciliation::{
    apply_plan, DesktopId, DesktopState, Occupancy, ReconcileBackend, ReconcilePass,
    ReconcileRuntime,
};

#[derive(Default)]
struct DelayedBackend {
    visible: Vec<DesktopState>,
    pending_create: Option<DesktopState>,
    pending_remove: Option<DesktopId>,
    create_calls: usize,
    remove_calls: usize,
}

impl DelayedBackend {
    // Function purpose: Builds a virtual desktop list with the supplied occupancies and the first desktop active.
    fn from(occupancies: &[Occupancy]) -> Self {
        Self {
            visible: occupancies
                .iter()
                .enumerate()
                .map(|(index, occupancy)| DesktopState {
                    id: DesktopId(format!("desktop-{index}")),
                    occupancy: *occupancy,
                    current: index == 0,
                    empty_grace_elapsed: true,
                })
                .collect(),
            ..Self::default()
        }
    }

    // Function purpose: Makes the previously requested create visible as if Explorer published the COM topology later.
    fn publish_create(&mut self) {
        if let Some(desktop) = self.pending_create.take() {
            self.visible.push(desktop);
        }
    }

    // Function purpose: Makes the previously requested remove visible as if Explorer completed compaction later.
    fn publish_remove(&mut self) {
        if let Some(desktop) = self.pending_remove.take() {
            self.visible.retain(|state| state.id != desktop);
        }
    }
}

impl ReconcileBackend for DelayedBackend {
    // Function purpose: Returns only topology changes that the virtual Windows shell has published.
    fn snapshot(&mut self) -> Result<Vec<DesktopState>, String> {
        Ok(self.visible.clone())
    }

    // Function purpose: Records one asynchronous create without exposing it in snapshots until publish_create is called.
    fn create_desktop(&mut self) -> Result<DesktopId, String> {
        self.create_calls += 1;
        let id = DesktopId(format!("desktop-created-{}", self.create_calls));
        self.pending_create = Some(DesktopState {
            id: id.clone(),
            occupancy: Occupancy::Empty,
            current: false,
            empty_grace_elapsed: true,
        });
        Ok(id)
    }

    // Function purpose: Updates the active desktop for completeness; reconciliation tests assert this path is not used.
    fn switch_desktop(&mut self, desktop: &DesktopId) -> Result<(), String> {
        for state in &mut self.visible {
            state.current = &state.id == desktop;
        }
        Ok(())
    }

    // Function purpose: Records one asynchronous removal without changing snapshots until publish_remove is called.
    fn remove_desktop(
        &mut self,
        desktop: &DesktopId,
        _fallback: &DesktopId,
    ) -> Result<(), String> {
        self.remove_calls += 1;
        self.pending_remove = Some(desktop.clone());
        Ok(())
    }
}

// Function purpose: Verifies the original eight-creation race is impossible when the newly created desktop is not immediately enumerable.
#[test]
fn delayed_create_never_fans_out_during_one_apply_plan() {
    let mut backend = DelayedBackend::from(&[Occupancy::Occupied, Occupancy::Occupied]);
    let report = apply_plan(&mut backend, 8).expect("delayed create should remain bounded");
    assert_eq!(backend.create_calls, 1);
    assert_eq!(report.mutations.len(), 1);
    assert!(!report.stable);
}

// Function purpose: Verifies one hundred rapid reconciliation triggers cannot create more than one in-flight spare desktop.
#[test]
fn one_hundred_scroll_triggers_create_only_one_pending_spare() {
    let mut backend = DelayedBackend::from(&[Occupancy::Occupied, Occupancy::Occupied]);
    let mut runtime = ReconcileRuntime::default();

    assert!(matches!(
        runtime.reconcile_once(&mut backend),
        Ok(ReconcilePass::Mutated(_))
    ));
    for _ in 0..100 {
        assert_eq!(
            runtime.reconcile_once(&mut backend),
            Ok(ReconcilePass::WaitingForTopology)
        );
    }
    assert_eq!(backend.create_calls, 1);
    assert_eq!(backend.visible.len(), 2);

    backend.publish_create();
    assert_eq!(
        runtime.reconcile_once(&mut backend),
        Ok(ReconcilePass::Stable)
    );
    assert_eq!(backend.visible.len(), 3);
    assert_eq!(backend.create_calls, 1);
}

// Function purpose: Verifies duplicate empty desktops compact one observed removal at a time without concurrent deletion requests.
#[test]
fn duplicate_empties_compact_serially_to_one_spare() {
    let mut backend = DelayedBackend::from(&[
        Occupancy::Occupied,
        Occupancy::Empty,
        Occupancy::Empty,
        Occupancy::Empty,
    ]);
    let mut runtime = ReconcileRuntime::default();

    assert!(matches!(
        runtime.reconcile_once(&mut backend),
        Ok(ReconcilePass::Mutated(_))
    ));
    for _ in 0..20 {
        assert_eq!(
            runtime.reconcile_once(&mut backend),
            Ok(ReconcilePass::WaitingForTopology)
        );
    }
    assert_eq!(backend.remove_calls, 1);

    backend.publish_remove();
    assert!(matches!(
        runtime.reconcile_once(&mut backend),
        Ok(ReconcilePass::Mutated(_))
    ));
    assert_eq!(backend.remove_calls, 2);
    backend.publish_remove();
    assert_eq!(
        runtime.reconcile_once(&mut backend),
        Ok(ReconcilePass::Stable)
    );
    assert_eq!(backend.visible.len(), 2);
    assert_eq!(backend.remove_calls, 2);
}
''',
)

replace_once("Cargo.toml", 'version = "0.1.5"', 'version = "0.1.6"')
replace_once(
    "Cargo.lock",
    'name = "deskpilot"\nversion = "0.1.5"',
    'name = "deskpilot"\nversion = "0.1.6"',
)
replace_once(".github/workflows/ci.yml", "DeskPilot 0.1.5", "DeskPilot 0.1.6")

changelog = read("CHANGELOG.md")
marker = "# Changelog\n"
section = '''# Changelog

## 0.1.6

- Serialize desktop creation and removal behind an observed-topology fence so delayed Windows enumeration cannot create up to eight duplicate desktops.
- Reconcile at most one topology mutation per pass and wait for the corresponding desktop event or watchdog snapshot before continuing.
- Consume every vertical and horizontal wheel message while an active Win gesture is in progress, including partial deltas and cooldown-suppressed events, so the foreground application never scrolls.
- Add virtual delayed-backend tests covering one hundred rapid triggers, delayed creation visibility, and serial empty-desktop compaction.
'''
if marker not in changelog:
    raise SystemExit("CHANGELOG.md header changed")
write("CHANGELOG.md", changelog.replace(marker, section, 1))

write(
    "docs/testing.md",
    read("docs/testing.md")
    + '''

## Virtual delayed-topology simulation

`tests/virtual_reconciliation.rs` models the observable delay between a successful virtual-desktop COM mutation and Explorer publishing the updated ordered desktop list. The simulator fires one hundred reconciliation triggers while a create remains invisible and asserts that only one create request exists. It also delays removals to prove that duplicate trailing empties compact serially to exactly one spare.
''',
)
write(
    "README.md",
    read("README.md")
    + '''

## Input isolation

While DeskPilot is active and either Windows key is held, all vertical and horizontal wheel messages are consumed by the low-level hook. Partial high-resolution deltas and events suppressed by the navigation cooldown never reach the application beneath the pointer. Wheel input behaves normally whenever the Windows key is not part of the gesture.
''',
)
