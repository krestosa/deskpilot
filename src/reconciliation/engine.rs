// File purpose: Executes one race-proof reconciliation mutation at a time and waits until Windows exposes the resulting topology.
use std::time::{Duration, Instant};
use thiserror::Error;

use super::{plan, DesktopId, DesktopState, Mutation};

const DEFAULT_TOPOLOGY_TIMEOUT: Duration = Duration::from_secs(15);

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
    #[error("topology did not confirm {operation} within {timeout_ms} ms")]
    TopologyTimeout { operation: String, timeout_ms: u128 },
    #[error("reconciliation exceeded {0} iterations")]
    IterationLimit(usize),
}

#[derive(Debug, Clone)]
enum PendingTopologyMutation {
    Create {
        expected: DesktopId,
        requested_at: Instant,
    },
    Remove {
        desktop: DesktopId,
        requested_at: Instant,
    },
}

impl PendingTopologyMutation {
    fn requested_at(&self) -> Instant {
        match self {
            Self::Create { requested_at, .. } | Self::Remove { requested_at, .. } => *requested_at,
        }
    }

    fn description(&self) -> String {
        match self {
            Self::Create { expected, .. } => format!("desktop creation {}", expected.0),
            Self::Remove { desktop, .. } => format!("desktop removal {}", desktop.0),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ReconcilePass {
    Stable,
    Blocked,
    WaitingForTopology,
    Mutated(Mutation),
}

#[derive(Debug)]
pub struct ReconcileRuntime {
    pending: Option<PendingTopologyMutation>,
    topology_timeout: Duration,
}

impl Default for ReconcileRuntime {
    fn default() -> Self {
        Self {
            pending: None,
            topology_timeout: DEFAULT_TOPOLOGY_TIMEOUT,
        }
    }
}

impl ReconcileRuntime {
    #[cfg(test)]
    pub fn with_topology_timeout(topology_timeout: Duration) -> Self {
        Self {
            pending: None,
            topology_timeout,
        }
    }

    // Function purpose: Applies at most one mutation and refuses further mutations until a later snapshot confirms the previous topology change.
    pub fn reconcile_once<B: ReconcileBackend>(
        &mut self,
        backend: &mut B,
    ) -> Result<ReconcilePass, ReconcileError> {
        let snapshot = backend.snapshot().map_err(ReconcileError::Snapshot)?;
        if !self.pending_observed(&snapshot)? {
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
                    requested_at: Instant::now(),
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
                    requested_at: Instant::now(),
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

    // Function purpose: Clears the in-flight barrier only after a snapshot proves that the exact requested topology change became visible.
    fn pending_observed(&mut self, snapshot: &[DesktopState]) -> Result<bool, ReconcileError> {
        let Some(pending) = self.pending.as_ref() else {
            return Ok(true);
        };
        let observed = match pending {
            PendingTopologyMutation::Create { expected, .. } => {
                snapshot.iter().any(|desktop| &desktop.id == expected)
            }
            PendingTopologyMutation::Remove { desktop, .. } => {
                snapshot.iter().all(|state| &state.id != desktop)
            }
        };
        if observed {
            self.pending = None;
            return Ok(true);
        }
        if pending.requested_at().elapsed() >= self.topology_timeout {
            let operation = pending.description();
            self.pending = None;
            return Err(ReconcileError::TopologyTimeout {
                operation,
                timeout_ms: self.topology_timeout.as_millis(),
            });
        }
        Ok(false)
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

#[cfg(test)]
mod tests {
    use super::{ReconcileBackend, ReconcileError, ReconcilePass, ReconcileRuntime};
    use crate::reconciliation::{DesktopId, DesktopState, Occupancy};
    use std::thread;
    use std::time::Duration;

    struct NeverPublishedBackend {
        states: Vec<DesktopState>,
    }

    impl ReconcileBackend for NeverPublishedBackend {
        fn snapshot(&mut self) -> Result<Vec<DesktopState>, String> {
            Ok(self.states.clone())
        }

        fn create_desktop(&mut self) -> Result<DesktopId, String> {
            Ok(DesktopId("pending".to_string()))
        }

        fn switch_desktop(&mut self, _desktop: &DesktopId) -> Result<(), String> {
            Ok(())
        }

        fn remove_desktop(
            &mut self,
            _desktop: &DesktopId,
            _fallback: &DesktopId,
        ) -> Result<(), String> {
            Ok(())
        }
    }

    #[test]
    fn pending_topology_times_out_and_recovers() {
        let mut backend = NeverPublishedBackend {
            states: vec![DesktopState {
                id: DesktopId("occupied".to_string()),
                occupancy: Occupancy::Occupied,
                current: true,
                empty_grace_elapsed: true,
            }],
        };
        let mut runtime = ReconcileRuntime::with_topology_timeout(Duration::from_millis(1));
        assert!(matches!(
            runtime.reconcile_once(&mut backend),
            Ok(ReconcilePass::Mutated(_))
        ));
        thread::sleep(Duration::from_millis(2));
        assert!(matches!(
            runtime.reconcile_once(&mut backend),
            Err(ReconcileError::TopologyTimeout { .. })
        ));
        assert!(!runtime.is_waiting_for_topology());
    }
}
