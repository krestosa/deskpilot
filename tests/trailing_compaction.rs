// File purpose: Verifies repeated trailing-empty compaction while preserving the active desktop.
use deskpilot::reconciliation::{apply_plan, DesktopId, DesktopState, Occupancy, ReconcileBackend};

struct FakeBackend {
    desktops: Vec<DesktopState>,
    removed: Vec<DesktopId>,
}

impl ReconcileBackend for FakeBackend {
    // Function purpose: Builds a fresh ordered desktop snapshot with current occupancy and empty-grace state.
    fn snapshot(&mut self) -> Result<Vec<DesktopState>, String> {
        Ok(self.desktops.clone())
    }

    // Function purpose: Verifies the create desktop scenario and its expected safety or state invariant.
    fn create_desktop(&mut self) -> Result<DesktopId, String> {
        Err("unexpected create".to_string())
    }

    // Function purpose: Verifies the switch desktop scenario and its expected safety or state invariant.
    fn switch_desktop(&mut self, _desktop: &DesktopId) -> Result<(), String> {
        Err("reconciliation must not switch the user".to_string())
    }

    // Function purpose: Verifies the remove desktop scenario and its expected safety or state invariant.
    fn remove_desktop(&mut self, desktop: &DesktopId, _fallback: &DesktopId) -> Result<(), String> {
        let state = self
            .desktops
            .iter()
            .find(|state| &state.id == desktop)
            .ok_or_else(|| "desktop disappeared".to_string())?;
        if state.current {
            return Err("attempted to remove active desktop".to_string());
        }
        self.desktops.retain(|state| &state.id != desktop);
        self.removed.push(desktop.clone());
        Ok(())
    }
}

// Function purpose: Verifies the state scenario and its expected safety or state invariant.
fn state(index: usize, occupancy: Occupancy, current: bool) -> DesktopState {
    DesktopState {
        id: DesktopId(format!("desktop-{index}")),
        occupancy,
        current,
        empty_grace_elapsed: true,
    }
}

// Function purpose: Verifies the several trailing empties compact to one without leaving active desktop scenario and its expected safety or state invariant.
#[test]
fn several_trailing_empties_compact_to_one_without_leaving_active_desktop() {
    let active = DesktopId("desktop-3".to_string());
    let mut backend = FakeBackend {
        desktops: vec![
            state(0, Occupancy::Occupied, false),
            state(1, Occupancy::Empty, false),
            state(2, Occupancy::Empty, false),
            state(3, Occupancy::Empty, true),
        ],
        removed: Vec::new(),
    };

    let report = apply_plan(&mut backend, 8).expect("compaction should converge");

    assert!(report.stable);
    assert_eq!(backend.desktops.len(), 2);
    assert_eq!(
        backend
            .desktops
            .iter()
            .filter(|desktop| desktop.occupancy == Occupancy::Empty)
            .count(),
        1
    );
    assert!(backend
        .desktops
        .iter()
        .any(|desktop| desktop.id == active && desktop.current));
    assert_eq!(backend.removed.len(), 2);
}
