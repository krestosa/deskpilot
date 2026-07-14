// File purpose: Simulates delayed Windows topology publication and verifies that event or scroll storms cannot create or remove multiple desktops concurrently.
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
    fn remove_desktop(&mut self, desktop: &DesktopId, _fallback: &DesktopId) -> Result<(), String> {
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
