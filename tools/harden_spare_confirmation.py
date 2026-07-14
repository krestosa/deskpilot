from pathlib import Path


def replace_once(path: str, old: str, new: str) -> None:
    target = Path(path)
    text = target.read_text(encoding="utf-8")
    count = text.count(old)
    if count != 1:
        raise SystemExit(f"{path}: expected exactly one match, found {count}")
    target.write_text(text.replace(old, new, 1), encoding="utf-8", newline="\n")


Path("src/reconciliation/spare_guard.rs").write_text(r'''// File purpose: Protects the trailing empty desktop until a persistent qualifying user-window lifecycle event proves that it was consumed.
use super::{DesktopId, DesktopState, Occupancy};
use std::collections::{HashMap, HashSet};

pub type WindowToken = u64;
const REQUIRED_CONFIRMATIONS: u8 = 2;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct SpareGuardResult {
    pub protecting: bool,
    pub consumed: bool,
}

#[derive(Debug, Clone, Default)]
pub struct SpareGuard {
    protected: Option<DesktopId>,
    candidate_streaks: HashMap<WindowToken, u8>,
}

impl SpareGuard {
    // Function purpose: Protects a desktop selected as the trailing navigation spare even if transient shell windows make it look occupied.
    pub fn arm(&mut self, desktop: DesktopId) {
        if self.protected.as_ref() != Some(&desktop) {
            self.candidate_streaks.clear();
        }
        self.protected = Some(desktop);
    }

    // Function purpose: Returns the desktop currently treated as the event-confirmed empty spare.
    pub fn protected(&self) -> Option<&DesktopId> {
        self.protected.as_ref()
    }

    // Function purpose: Reports whether a visible event-backed window needs another observation before it can consume the spare.
    pub fn needs_confirmation(&self) -> bool {
        !self.candidate_streaks.is_empty()
    }

    // Function purpose: Overrides transient occupancy until the same visible event-backed user window survives two inventory observations on the spare.
    pub fn stabilize(
        &mut self,
        states: &mut [DesktopState],
        visible_windows: &HashMap<DesktopId, HashSet<WindowToken>>,
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
        let observed: HashSet<_> = visible_windows
            .get(&protected)
            .into_iter()
            .flat_map(|tokens| tokens.iter().copied())
            .filter(|token| occupancy_gain_candidates.contains(token))
            .collect();
        self.candidate_streaks
            .retain(|token, _| observed.contains(token));
        for token in observed {
            let streak = self.candidate_streaks.entry(token).or_insert(0);
            *streak = streak.saturating_add(1);
        }

        if self
            .candidate_streaks
            .values()
            .any(|streak| *streak >= REQUIRED_CONFIRMATIONS)
        {
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
        self.candidate_streaks.clear();
    }
}
''', encoding="utf-8", newline="\n")

replace_once(
    "src/windows/inventory.rs",
    "            windows\n                .entry(desktop)\n                .or_default()\n                .insert(hwnd as usize as crate::reconciliation::WindowToken);\n",
    "            if window_is_visible_user_surface(hwnd) {\n"
    "                windows\n"
    "                    .entry(desktop)\n"
    "                    .or_default()\n"
    "                    .insert(hwnd as usize as crate::reconciliation::WindowToken);\n"
    "            }\n",
)
replace_once(
    "src/windows/inventory.rs",
    "        IsWindowVisible(hwnd) != 0 || window_is_cloaked(hwnd)\n",
    "        IsWindowVisible(hwnd) != 0 || window_is_shell_cloaked(hwnd)\n",
)
old_cloak = r'''// Function purpose: Performs the window is cloaked operation required by this module.
fn window_is_cloaked(hwnd: HWND) -> bool {
    unsafe {
        let mut cloaked = 0_u32;
        DwmGetWindowAttribute(
            hwnd,
            DWMWA_CLOAKED as u32,
            (&mut cloaked as *mut u32).cast::<c_void>(),
            size_of::<u32>() as u32,
        ) >= 0
            && cloaked & 0x2 != 0
    }
}
'''
new_cloak = r'''// Function purpose: Returns the complete DWM cloak-reason bitset or zero when the attribute cannot be read.
fn window_cloak_flags(hwnd: HWND) -> u32 {
    unsafe {
        let mut cloaked = 0_u32;
        if DwmGetWindowAttribute(
            hwnd,
            DWMWA_CLOAKED as u32,
            (&mut cloaked as *mut u32).cast::<c_void>(),
            size_of::<u32>() as u32,
        ) >= 0
        {
            cloaked
        } else {
            0
        }
    }
}

// Function purpose: Reports any DWM cloak reason for foreground and visibility validation.
fn window_is_cloaked(hwnd: HWND) -> bool {
    window_cloak_flags(hwnd) != 0
}

// Function purpose: Counts inactive application windows only when Windows shell virtual-desktop cloaking is present.
fn window_is_shell_cloaked(hwnd: HWND) -> bool {
    window_cloak_flags(hwnd) & 0x2 != 0
}

// Function purpose: Restricts spare-consumption evidence to an actually visible, non-cloaked user surface on the current desktop.
fn window_is_visible_user_surface(hwnd: HWND) -> bool {
    unsafe { IsWindowVisible(hwnd) != 0 } && !window_is_cloaked(hwnd)
}
'''
replace_once("src/windows/inventory.rs", old_cloak, new_cloak)

replace_once(
    "src/app.rs",
    "                matches!(state.reconcile(), ReconcilePass::Mutated(_))\n",
    "                let pass = state.reconcile();\n"
    "                matches!(pass, ReconcilePass::Mutated(_))\n"
    "                    || state.spare_guard.needs_confirmation()\n",
)

Path("tests/spare_guard.rs").write_text(r'''// File purpose: Reproduces repeated scroll visits to a noisy empty spare and proves only persistent visible event-confirmed user windows can consume it.
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
        let visible_windows = HashMap::from([(spare.clone(), HashSet::from([9001]))]);
        let result = guard.stabilize(&mut noisy, &visible_windows, &candidates);
        assert!(result.protecting);
        assert!(!result.consumed);
        assert!(!guard.needs_confirmation());
        assert_eq!(noisy[1].occupancy, Occupancy::Empty);
        assert!(plan(&noisy).mutations.is_empty());
    }
}

// Function purpose: Verifies a visible qualifying window must survive two observations before it can authorize one new trailing desktop.
#[test]
fn persistent_real_window_event_consumes_spare_once() {
    let mut guard = SpareGuard::default();
    let spare = DesktopId("desktop-1".to_string());
    guard.arm(spare.clone());
    let token: WindowToken = 42;
    let visible_windows = HashMap::from([(spare.clone(), HashSet::from([token]))]);
    let candidates = HashSet::from([token]);
    let mut first = vec![
        state(0, Occupancy::Occupied, false),
        state(1, Occupancy::Occupied, true),
    ];

    let first_result = guard.stabilize(&mut first, &visible_windows, &candidates);
    assert!(first_result.protecting);
    assert!(!first_result.consumed);
    assert!(guard.needs_confirmation());
    assert!(plan(&first).mutations.is_empty());

    let mut second = vec![
        state(0, Occupancy::Occupied, false),
        state(1, Occupancy::Occupied, true),
    ];
    let second_result = guard.stabilize(&mut second, &visible_windows, &candidates);
    assert!(second_result.consumed);
    assert!(!second_result.protecting);
    assert_eq!(guard.protected(), None);
    assert!(!guard.needs_confirmation());
    assert_eq!(plan(&second).mutations, vec![Mutation::CreateTrailing]);
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
    let visible_windows = HashMap::from([
        (DesktopId("desktop-0".to_string()), HashSet::from([77])),
        (spare.clone(), HashSet::from([9001])),
    ]);
    let candidates = HashSet::from([77]);

    let result = guard.stabilize(&mut states, &visible_windows, &candidates);
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

print("spare confirmation hardening applied")
