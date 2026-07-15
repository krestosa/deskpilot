from pathlib import Path


def replace_once(path: str, old: str, new: str) -> None:
    target = Path(path)
    text = target.read_text(encoding="utf-8")
    count = text.count(old)
    if count != 1:
        raise SystemExit(f"{path}: expected exactly one match, found {count}")
    target.write_text(text.replace(old, new, 1), encoding="utf-8", newline="\n")


Path("src/reconciliation/spare_guard.rs").write_text(r'''// File purpose: Protects the trailing empty desktop until persistent eligible-window evidence proves that it was consumed.
use super::{DesktopId, DesktopState, Occupancy};
use std::collections::{HashMap, HashSet};

pub type WindowToken = u64;
const EVENT_CONFIRMATIONS: u8 = 2;
const PASSIVE_CONFIRMATIONS: u8 = 3;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct SpareGuardResult {
    pub protecting: bool,
    pub consumed: bool,
}

#[derive(Debug, Clone, Default)]
pub struct SpareGuard {
    protected: Option<DesktopId>,
    observed_streaks: HashMap<WindowToken, u8>,
}

impl SpareGuard {
    // Function purpose: Protects a desktop selected as the trailing navigation spare even if transient shell windows make it look occupied.
    pub fn arm(&mut self, desktop: DesktopId) {
        if self.protected.as_ref() != Some(&desktop) {
            self.observed_streaks.clear();
        }
        self.protected = Some(desktop);
    }

    // Function purpose: Returns the desktop currently treated as the protected empty spare.
    pub fn protected(&self) -> Option<&DesktopId> {
        self.protected.as_ref()
    }

    // Function purpose: Reports whether eligible window evidence needs another observation before it can consume the spare.
    pub fn needs_confirmation(&self) -> bool {
        !self.observed_streaks.is_empty()
    }

    // Function purpose: Overrides raw occupancy until one eligible window persists long enough to prove that the protected spare contains a user application.
    pub fn stabilize(
        &mut self,
        states: &mut [DesktopState],
        confirmable_windows: &HashMap<DesktopId, HashSet<WindowToken>>,
        occupancy_gain_candidates: &HashSet<WindowToken>,
    ) -> SpareGuardResult {
        let Some(last) = states.last() else {
            self.clear();
            return SpareGuardResult::default();
        };
        let last_id = last.id.clone();

        if self.protected.as_ref().is_some_and(|id| id != &last_id) {
            self.clear();
        }
        if self.protected.is_none() && last.occupancy == Occupancy::Empty {
            self.protected = Some(last_id.clone());
        }

        let Some(protected) = self.protected.clone() else {
            return SpareGuardResult::default();
        };
        let observed: HashSet<_> = confirmable_windows
            .get(&protected)
            .into_iter()
            .flat_map(|tokens| tokens.iter().copied())
            .collect();
        self.observed_streaks
            .retain(|token, _| observed.contains(token));
        for token in &observed {
            let streak = self.observed_streaks.entry(*token).or_insert(0);
            *streak = streak.saturating_add(1);
        }

        let consumed = self.observed_streaks.iter().any(|(token, streak)| {
            let required = if occupancy_gain_candidates.contains(token) {
                EVENT_CONFIRMATIONS
            } else {
                PASSIVE_CONFIRMATIONS
            };
            *streak >= required
        });
        if consumed {
            self.clear();
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
            self.clear();
            SpareGuardResult::default()
        }
    }

    // Function purpose: Clears protection and all pending evidence when the spare changes or is consumed.
    fn clear(&mut self) {
        self.protected = None;
        self.observed_streaks.clear();
    }
}
''', encoding="utf-8", newline="\n")

replace_once(
    "src/windows/inventory.rs",
    "            if ignored_shell_executable(&executable)\n",
    "            if ignored_process_window(&executable, &identity.class_name)\n",
)
replace_once(
    "src/windows/inventory.rs",
    "        if ignored_shell_executable(&executable) {\n",
    "        if ignored_process_window(&executable, &identity.class_name) {\n",
)
replace_once(
    "src/windows/inventory.rs",
    "            if window_is_visible_user_surface(hwnd) {\n",
    "            if window_is_confirmable_user_surface(hwnd) {\n",
)
replace_once(
    "src/windows/inventory.rs",
    '''// Function purpose: Restricts spare-consumption evidence to an actually visible, non-cloaked user surface on the current desktop.
fn window_is_visible_user_surface(hwnd: HWND) -> bool {
    (unsafe { IsWindowVisible(hwnd) != 0 }) && !window_is_cloaked(hwnd)
}
''',
    '''// Function purpose: Accepts a visible current-desktop window or a virtual-desktop-cloaked inactive window as persistent user-surface evidence.
fn window_is_confirmable_user_surface(hwnd: HWND) -> bool {
    let visible = unsafe { IsWindowVisible(hwnd) != 0 };
    let cloak_flags = window_cloak_flags(hwnd);
    (visible && cloak_flags == 0) || cloak_flags & 0x2 != 0
}
''',
)
replace_once(
    "src/windows/inventory.rs",
    '        "explorer.exe",\n',
    '',
)
replace_once(
    "src/windows/inventory.rs",
    '''// Function purpose: Performs the ignored class operation required by this module.
fn ignored_class(class_name: &str) -> bool {
''',
    '''// Function purpose: Ignores Explorer shell infrastructure while preserving actual File Explorer application windows.
fn ignored_process_window(executable: &str, class_name: &str) -> bool {
    if executable.eq_ignore_ascii_case("explorer.exe") {
        !is_file_explorer_class(class_name)
    } else {
        ignored_shell_executable(executable)
    }
}

// Function purpose: Recognizes the top-level Win32 classes used by real File Explorer windows.
fn is_file_explorer_class(class_name: &str) -> bool {
    ["CabinetWClass", "ExploreWClass"]
        .iter()
        .any(|value| value.eq_ignore_ascii_case(class_name))
}

// Function purpose: Performs the ignored class operation required by this module.
fn ignored_class(class_name: &str) -> bool {
''',
)
replace_once(
    "src/windows/inventory.rs",
    "    use super::{ignored_class, ignored_shell_executable, rect_covers};\n",
    "    use super::{ignored_class, ignored_process_window, ignored_shell_executable, rect_covers};\n",
)
replace_once(
    "src/windows/inventory.rs",
    '''        assert!(ignored_shell_executable("RuntimeBroker.exe"));
        assert!(ignored_shell_executable("explorer.exe"));
        assert!(!ignored_shell_executable("notepad.exe"));
''',
    '''        assert!(ignored_shell_executable("RuntimeBroker.exe"));
        assert!(!ignored_shell_executable("explorer.exe"));
        assert!(!ignored_shell_executable("notepad.exe"));
        assert!(ignored_process_window("explorer.exe", "Progman"));
        assert!(ignored_process_window("explorer.exe", "Shell_TrayWnd"));
        assert!(!ignored_process_window("explorer.exe", "CabinetWClass"));
        assert!(!ignored_process_window("EXPLORER.EXE", "ExploreWClass"));
''',
)

Path("tests/spare_guard.rs").write_text(r'''// File purpose: Reproduces noisy scroll navigation and proves both newly opened and pre-existing real applications consume exactly one trailing spare.
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

// Function purpose: Verifies repeated scroll visits cannot convert raw switch-time occupancy noise into additional desktop creation.
#[test]
fn one_app_plus_repeated_scroll_stays_at_two_desktops() {
    let mut guard = SpareGuard::default();
    let spare = DesktopId("desktop-1".to_string());

    let mut initial = vec![
        state(0, Occupancy::Occupied, true),
        state(1, Occupancy::Empty, false),
    ];
    guard.stabilize(&mut initial, &HashMap::new(), &HashSet::new());
    assert_eq!(guard.protected(), Some(&spare));

    for iteration in 0..200 {
        guard.arm(spare.clone());
        let mut noisy = vec![
            state(0, Occupancy::Occupied, iteration % 2 == 0),
            state(1, Occupancy::Occupied, iteration % 2 != 0),
        ];
        let result = guard.stabilize(&mut noisy, &HashMap::new(), &HashSet::new());
        assert!(result.protecting);
        assert!(!result.consumed);
        assert!(!guard.needs_confirmation());
        assert_eq!(noisy[1].occupancy, Occupancy::Empty);
        assert!(plan(&noisy).mutations.is_empty());
    }
}

// Function purpose: Verifies changing transient helper tokens never survive long enough to consume the protected spare.
#[test]
fn changing_transient_surfaces_cannot_consume_spare() {
    let mut guard = SpareGuard::default();
    let spare = DesktopId("desktop-1".to_string());
    guard.arm(spare.clone());

    for token in 1..=100 {
        let mut states = vec![
            state(0, Occupancy::Occupied, false),
            state(1, Occupancy::Occupied, true),
        ];
        let windows = HashMap::from([(spare.clone(), HashSet::from([token]))]);
        let result = guard.stabilize(&mut states, &windows, &HashSet::new());
        assert!(result.protecting);
        assert!(!result.consumed);
        assert_eq!(states[1].occupancy, Occupancy::Empty);
        assert!(plan(&states).mutations.is_empty());
    }
}

// Function purpose: Verifies a newly opened event-backed window needs two observations before authorizing one new trailing desktop.
#[test]
fn persistent_real_window_event_consumes_spare_once() {
    let mut guard = SpareGuard::default();
    let spare = DesktopId("desktop-1".to_string());
    guard.arm(spare.clone());
    let token: WindowToken = 42;
    let windows = HashMap::from([(spare.clone(), HashSet::from([token]))]);
    let candidates = HashSet::from([token]);

    let mut first = vec![
        state(0, Occupancy::Occupied, false),
        state(1, Occupancy::Occupied, true),
    ];
    let first_result = guard.stabilize(&mut first, &windows, &candidates);
    assert!(first_result.protecting);
    assert!(!first_result.consumed);
    assert!(guard.needs_confirmation());
    assert!(plan(&first).mutations.is_empty());

    let mut second = vec![
        state(0, Occupancy::Occupied, false),
        state(1, Occupancy::Occupied, true),
    ];
    let second_result = guard.stabilize(&mut second, &windows, &candidates);
    assert!(second_result.consumed);
    assert!(!second_result.protecting);
    assert_eq!(guard.protected(), None);
    assert!(!guard.needs_confirmation());
    assert_eq!(plan(&second).mutations, vec![Mutation::CreateTrailing]);
}

// Function purpose: Verifies a pre-existing File Explorer or other eligible application consumes the spare after three stable observations even when its create event was missed.
#[test]
fn preexisting_persistent_application_consumes_spare() {
    let mut guard = SpareGuard::default();
    let spare = DesktopId("desktop-1".to_string());
    guard.arm(spare.clone());
    let token: WindowToken = 84;
    let windows = HashMap::from([(spare.clone(), HashSet::from([token]))]);

    for observation in 1..=3 {
        let mut states = vec![
            state(0, Occupancy::Occupied, false),
            state(1, Occupancy::Occupied, true),
        ];
        let result = guard.stabilize(&mut states, &windows, &HashSet::new());
        if observation < 3 {
            assert!(result.protecting);
            assert!(!result.consumed);
            assert!(guard.needs_confirmation());
            assert_eq!(states[1].occupancy, Occupancy::Empty);
            assert!(plan(&states).mutations.is_empty());
        } else {
            assert!(result.consumed);
            assert_eq!(plan(&states).mutations, vec![Mutation::CreateTrailing]);
        }
    }
}

// Function purpose: Verifies a transient event-backed surface that disappears before confirmation cannot consume the spare.
#[test]
fn transient_event_surface_cannot_consume_spare() {
    let mut guard = SpareGuard::default();
    let spare = DesktopId("desktop-1".to_string());
    guard.arm(spare.clone());
    let token: WindowToken = 55;
    let candidates = HashSet::from([token]);
    let mut first = vec![
        state(0, Occupancy::Occupied, false),
        state(1, Occupancy::Occupied, true),
    ];
    let first_windows = HashMap::from([(spare.clone(), HashSet::from([token]))]);
    guard.stabilize(&mut first, &first_windows, &candidates);
    assert!(guard.needs_confirmation());

    let mut second = vec![
        state(0, Occupancy::Occupied, false),
        state(1, Occupancy::Occupied, true),
    ];
    let result = guard.stabilize(&mut second, &HashMap::new(), &candidates);
    assert!(result.protecting);
    assert!(!result.consumed);
    assert!(!guard.needs_confirmation());
    assert_eq!(second[1].occupancy, Occupancy::Empty);
    assert!(plan(&second).mutations.is_empty());
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
    let windows = HashMap::from([(DesktopId("desktop-0".to_string()), HashSet::from([77]))]);
    let candidates = HashSet::from([77]);

    let result = guard.stabilize(&mut states, &windows, &candidates);
    assert!(result.protecting);
    assert!(!result.consumed);
    assert!(!guard.needs_confirmation());
    assert_eq!(states[1].occupancy, Occupancy::Empty);
    assert!(plan(&states).mutations.is_empty());
}

// Function purpose: Verifies the guard follows a newly created trailing spare and discards stale confirmation evidence.
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
    assert!(!guard.needs_confirmation());
    assert_eq!(states[2].occupancy, Occupancy::Empty);
}
''', encoding="utf-8", newline="\n")

replace_once("Cargo.toml", 'version = "0.1.7"\n', 'version = "0.1.8"\n')
replace_once(
    "Cargo.lock",
    'name = "deskpilot"\nversion = "0.1.7"\n',
    'name = "deskpilot"\nversion = "0.1.8"\n',
)
replace_once(
    ".github/workflows/ci.yml",
    "DeskPilot 0.1.7",
    "DeskPilot 0.1.8",
)
replace_once(
    "CHANGELOG.md",
    "# Changelog\n\n## 0.1.7\n",
    "# Changelog\n\n## 0.1.8\n\n"
    "- Count real File Explorer windows (`CabinetWClass` and `ExploreWClass`) as user applications while continuing to ignore Explorer-owned desktop, taskbar and shell infrastructure.\n"
    "- Allow a pre-existing or event-missed eligible application to consume the protected trailing spare after three stable inventory observations.\n"
    "- Preserve the faster two-observation path for native CREATE/SHOW-confirmed windows and reject changing transient helper tokens.\n"
    "- Add regressions for an Explorer window on the final desktop, missed lifecycle events, transient surfaces and repeated noisy scrolling.\n\n"
    "## 0.1.7\n",
)

print("Explorer occupancy and missed-event spare correction applied")
