// File purpose: Executes one race-proof reconciliation mutation at a time and waits until Windows exposes the resulting topology.
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
                let created =
                    backend
                        .create_desktop()
                        .map_err(|cause| ReconcileError::Mutation {
                            operation: format!("{mutation:?}"),
                            cause,
                        })?;
                self.pending = Some(PendingTopologyMutation::Create {
                    expected: created,
                    baseline_count: snapshot.len(),
                });
            }
            Mutation::Remove { desktop, fallback } => {
                backend.remove_desktop(desktop, fallback).map_err(|cause| {
                    ReconcileError::Mutation {
                        operation: format!("{mutation:?}"),
                        cause,
                    }
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
