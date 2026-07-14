use deskpilot::reconciliation::{
    apply_plan, plan, DesktopId, DesktopState, Mutation, Occupancy, ReconcileBackend,
    ReconcileError,
};

fn desktop(index: usize, occupancy: Occupancy) -> DesktopState {
    DesktopState {
        id: DesktopId(format!("desktop-{index}")),
        occupancy,
        current: index == 0,
        empty_grace_elapsed: true,
    }
}

fn states(values: &[Occupancy]) -> Vec<DesktopState> {
    values
        .iter()
        .copied()
        .enumerate()
        .map(|(index, occupancy)| desktop(index, occupancy))
        .collect()
}

fn set_current(snapshot: &mut [DesktopState], current: usize) {
    for (index, desktop) in snapshot.iter_mut().enumerate() {
        desktop.current = index == current;
    }
}

#[test]
fn occupied_creates_trailing_spare() {
    let result = plan(&states(&[Occupancy::Occupied]));
    assert_eq!(result.mutations, vec![Mutation::CreateTrailing]);
}

#[test]
fn active_occupied_last_desktop_creates_a_new_spare() {
    let mut snapshot = states(&[Occupancy::Occupied, Occupancy::Occupied]);
    set_current(&mut snapshot, 1);
    let result = plan(&snapshot);
    assert_eq!(result.mutations, vec![Mutation::CreateTrailing]);
}

#[test]
fn occupied_then_empty_is_stable() {
    let result = plan(&states(&[Occupancy::Occupied, Occupancy::Empty]));
    assert!(result.stable);
    assert!(result.mutations.is_empty());
}

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

#[test]
fn single_empty_desktop_is_preserved() {
    let result = plan(&states(&[Occupancy::Empty]));
    assert!(result.stable);
}

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
    fn from(values: &[Occupancy]) -> Self {
        Self {
            desktops: states(values),
            ..Self::default()
        }
    }
}

impl ReconcileBackend for FakeBackend {
    fn snapshot(&mut self) -> Result<Vec<DesktopState>, String> {
        Ok(self.desktops.clone())
    }

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

    fn switch_desktop(&mut self, desktop: &DesktopId) -> Result<(), String> {
        for state in &mut self.desktops {
            state.current = &state.id == desktop;
        }
        self.operations.push(format!("switch:{}", desktop.0));
        Ok(())
    }

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

#[test]
fn duplicate_events_do_not_duplicate_desktops() {
    let mut backend = FakeBackend::from(&[Occupancy::Occupied]);
    apply_plan(&mut backend, 8).expect("first reconciliation should complete");
    apply_plan(&mut backend, 8).expect("second reconciliation should be stable");
    assert_eq!(backend.desktops.len(), 2);
    assert_eq!(backend.operations, vec!["create"]);
}

#[test]
fn failed_creation_is_bounded() {
    let mut backend = FakeBackend::from(&[Occupancy::Occupied]);
    backend.fail_create = true;
    let error = apply_plan(&mut backend, 8).expect_err("creation must fail");
    assert!(matches!(error, ReconcileError::Mutation { .. }));
    assert!(backend.operations.is_empty());
}

#[test]
fn failed_removal_does_not_lose_state() {
    let mut backend = FakeBackend::from(&[Occupancy::Occupied, Occupancy::Empty, Occupancy::Empty]);
    backend.fail_remove = true;
    let original = backend.desktops.clone();
    let error = apply_plan(&mut backend, 8).expect_err("removal must fail");
    assert!(matches!(error, ReconcileError::Mutation { .. }));
    assert_eq!(backend.desktops, original);
}

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
