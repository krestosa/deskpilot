// File purpose: Verifies reconciliation planning, convergence, safety, and failure bounds.
use deskpilot::reconciliation::{
    apply_plan, plan, DesktopId, DesktopState, Mutation, Occupancy, ReconcileBackend,
    ReconcileError,
};

// Function purpose: Verifies the desktop scenario and its expected safety or state invariant.
fn desktop(index: usize, occupancy: Occupancy) -> DesktopState {
    DesktopState {
        id: DesktopId(format!("desktop-{index}")),
        occupancy,
        current: index == 0,
        empty_grace_elapsed: true,
    }
}

// Function purpose: Verifies the states scenario and its expected safety or state invariant.
fn states(values: &[Occupancy]) -> Vec<DesktopState> {
    values
        .iter()
        .copied()
        .enumerate()
        .map(|(index, occupancy)| desktop(index, occupancy))
        .collect()
}

// Function purpose: Verifies the set current scenario and its expected safety or state invariant.
fn set_current(snapshot: &mut [DesktopState], current: usize) {
    for (index, desktop) in snapshot.iter_mut().enumerate() {
        desktop.current = index == current;
    }
}

// Function purpose: Verifies the occupied creates trailing spare scenario and its expected safety or state invariant.
#[test]
fn occupied_creates_trailing_spare() {
    let result = plan(&states(&[Occupancy::Occupied]));
    assert_eq!(result.mutations, vec![Mutation::CreateTrailing]);
}

// Function purpose: Verifies the active occupied last desktop creates a new spare scenario and its expected safety or state invariant.
#[test]
fn active_occupied_last_desktop_creates_a_new_spare() {
    let mut snapshot = states(&[Occupancy::Occupied, Occupancy::Occupied]);
    set_current(&mut snapshot, 1);
    let result = plan(&snapshot);
    assert_eq!(result.mutations, vec![Mutation::CreateTrailing]);
}

// Function purpose: Verifies the occupied then empty is stable scenario and its expected safety or state invariant.
#[test]
fn occupied_then_empty_is_stable() {
    let result = plan(&states(&[Occupancy::Occupied, Occupancy::Empty]));
    assert!(result.stable);
    assert!(result.mutations.is_empty());
}

// Function purpose: Verifies the duplicate trailing empty is removed scenario and its expected safety or state invariant.
#[test]
fn duplicate_trailing_empty_is_removed() {
    let result = plan(&states(&[
        Occupancy::Occupied,
        Occupancy::Empty,
        Occupancy::Empty,
    ]));
    assert_eq!(
        result.mutations,
        vec![Mutation::Remove {
            desktop: DesktopId("desktop-2".to_string()),
            fallback: DesktopId("desktop-1".to_string()),
        }]
    );
}

// Function purpose: Verifies the active last trailing empty is preserved scenario and its expected safety or state invariant.
#[test]
fn active_last_trailing_empty_is_preserved() {
    let mut snapshot = states(&[Occupancy::Occupied, Occupancy::Empty, Occupancy::Empty]);
    set_current(&mut snapshot, 2);
    let result = plan(&snapshot);
    assert_eq!(
        result.mutations,
        vec![Mutation::Remove {
            desktop: DesktopId("desktop-1".to_string()),
            fallback: DesktopId("desktop-2".to_string()),
        }]
    );
}

// Function purpose: Verifies the active first trailing empty is preserved scenario and its expected safety or state invariant.
#[test]
fn active_first_trailing_empty_is_preserved() {
    let mut snapshot = states(&[Occupancy::Occupied, Occupancy::Empty, Occupancy::Empty]);
    set_current(&mut snapshot, 1);
    let result = plan(&snapshot);
    assert_eq!(
        result.mutations,
        vec![Mutation::Remove {
            desktop: DesktopId("desktop-2".to_string()),
            fallback: DesktopId("desktop-1".to_string()),
        }]
    );
}

// Function purpose: Verifies the internal empty is removed into trailing spare scenario and its expected safety or state invariant.
#[test]
fn internal_empty_is_removed_into_trailing_spare() {
    let result = plan(&states(&[
        Occupancy::Occupied,
        Occupancy::Empty,
        Occupancy::Occupied,
        Occupancy::Empty,
    ]));
    assert_eq!(
        result.mutations,
        vec![Mutation::Remove {
            desktop: DesktopId("desktop-1".to_string()),
            fallback: DesktopId("desktop-3".to_string()),
        }]
    );
}

// Function purpose: Verifies the single empty desktop is preserved scenario and its expected safety or state invariant.
#[test]
fn single_empty_desktop_is_preserved() {
    let result = plan(&states(&[Occupancy::Empty]));
    assert!(result.stable);
}

// Function purpose: Verifies the unknown desktop is never removed scenario and its expected safety or state invariant.
#[test]
fn unknown_desktop_is_never_removed() {
    let result = plan(&states(&[
        Occupancy::Occupied,
        Occupancy::Unknown,
        Occupancy::Empty,
    ]));
    assert!(result.stable);
    assert!(result.mutations.is_empty());
}

// Function purpose: Verifies the current internal empty is never switched or removed scenario and its expected safety or state invariant.
#[test]
fn current_internal_empty_is_never_switched_or_removed() {
    let mut snapshot = states(&[
        Occupancy::Occupied,
        Occupancy::Empty,
        Occupancy::Occupied,
        Occupancy::Empty,
    ]);
    set_current(&mut snapshot, 1);
    let result = plan(&snapshot);
    assert!(result.stable);
    assert!(result.mutations.is_empty());
}

// Function purpose: Verifies the empty grace period blocks internal removal scenario and its expected safety or state invariant.
#[test]
fn empty_grace_period_blocks_internal_removal() {
    let mut snapshot = states(&[
        Occupancy::Occupied,
        Occupancy::Empty,
        Occupancy::Occupied,
        Occupancy::Empty,
    ]);
    snapshot[1].empty_grace_elapsed = false;
    let result = plan(&snapshot);
    assert!(result.stable);
}

#[derive(Default)]
struct FakeBackend {
    desktops: Vec<DesktopState>,
    fail_create: bool,
    fail_remove: bool,
    operations: Vec<String>,
}

impl FakeBackend {
    // Function purpose: Verifies the from scenario and its expected safety or state invariant.
    fn from(values: &[Occupancy]) -> Self {
        Self {
            desktops: states(values),
            ..Self::default()
        }
    }
}

impl ReconcileBackend for FakeBackend {
    // Function purpose: Builds a fresh ordered desktop snapshot with current occupancy and empty-grace state.
    fn snapshot(&mut self) -> Result<Vec<DesktopState>, String> {
        Ok(self.desktops.clone())
    }

    // Function purpose: Verifies the create desktop scenario and its expected safety or state invariant.
    fn create_desktop(&mut self) -> Result<DesktopId, String> {
        if self.fail_create {
            return Err("creation failed".to_string());
        }
        let id = DesktopId(format!("desktop-{}", self.desktops.len()));
        self.desktops.push(DesktopState {
            id: id.clone(),
            occupancy: Occupancy::Empty,
            current: false,
            empty_grace_elapsed: true,
        });
        self.operations.push("create".to_string());
        Ok(id)
    }

    // Function purpose: Verifies the switch desktop scenario and its expected safety or state invariant.
    fn switch_desktop(&mut self, desktop: &DesktopId) -> Result<(), String> {
        for state in &mut self.desktops {
            state.current = &state.id == desktop;
        }
        self.operations.push(format!("switch:{}", desktop.0));
        Ok(())
    }

    // Function purpose: Verifies the remove desktop scenario and its expected safety or state invariant.
    fn remove_desktop(&mut self, desktop: &DesktopId, _fallback: &DesktopId) -> Result<(), String> {
        if self.fail_remove {
            return Err("removal failed".to_string());
        }
        assert!(
            self.desktops
                .iter()
                .find(|state| &state.id == desktop)
                .is_none_or(|state| !state.current),
            "the reconciler must never remove the active desktop"
        );
        self.desktops.retain(|state| &state.id != desktop);
        self.operations.push(format!("remove:{}", desktop.0));
        Ok(())
    }
}

// Function purpose: Verifies the occupying spare creates exactly one new spare scenario and its expected safety or state invariant.
#[test]
fn occupying_spare_creates_exactly_one_new_spare() {
    let mut backend = FakeBackend::from(&[Occupancy::Occupied, Occupancy::Occupied]);
    let report = apply_plan(&mut backend, 8).expect("reconciliation should complete");
    assert!(report.stable);
    assert_eq!(backend.desktops.len(), 3);
    assert_eq!(
        backend.desktops.last().map(|state| state.occupancy),
        Some(Occupancy::Empty)
    );
    assert_eq!(backend.operations, vec!["create"]);
}

// Function purpose: Verifies the duplicate events do not duplicate desktops scenario and its expected safety or state invariant.
#[test]
fn duplicate_events_do_not_duplicate_desktops() {
    let mut backend = FakeBackend::from(&[Occupancy::Occupied]);
    apply_plan(&mut backend, 8).expect("first reconciliation should complete");
    apply_plan(&mut backend, 8).expect("second reconciliation should be stable");
    assert_eq!(backend.desktops.len(), 2);
    assert_eq!(backend.operations, vec!["create"]);
}

// Function purpose: Verifies the failed creation is bounded scenario and its expected safety or state invariant.
#[test]
fn failed_creation_is_bounded() {
    let mut backend = FakeBackend::from(&[Occupancy::Occupied]);
    backend.fail_create = true;
    let error = apply_plan(&mut backend, 8).expect_err("creation must fail");
    assert!(matches!(error, ReconcileError::Mutation { .. }));
    assert!(backend.operations.is_empty());
}

// Function purpose: Verifies the failed removal does not lose state scenario and its expected safety or state invariant.
#[test]
fn failed_removal_does_not_lose_state() {
    let mut backend = FakeBackend::from(&[Occupancy::Occupied, Occupancy::Empty, Occupancy::Empty]);
    backend.fail_remove = true;
    let original = backend.desktops.clone();
    let error = apply_plan(&mut backend, 8).expect_err("removal must fail");
    assert!(matches!(error, ReconcileError::Mutation { .. }));
    assert_eq!(backend.desktops, original);
}

// Function purpose: Verifies the reconciler never switches desktops scenario and its expected safety or state invariant.
#[test]
fn reconciler_never_switches_desktops() {
    let mut backend = FakeBackend::from(&[
        Occupancy::Occupied,
        Occupancy::Empty,
        Occupancy::Occupied,
        Occupancy::Empty,
    ]);
    apply_plan(&mut backend, 8).expect("reconciliation should complete");
    assert!(backend
        .operations
        .iter()
        .all(|operation| operation.starts_with("create") || operation.starts_with("remove")));
}
