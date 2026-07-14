use thiserror::Error;

use super::{plan, DesktopId, DesktopState, Mutation};

pub trait ReconcileBackend {
    fn snapshot(&mut self) -> Result<Vec<DesktopState>, String>;
    fn create_desktop(&mut self) -> Result<DesktopId, String>;
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
    #[error("reconciliation made no progress")]
    NoProgress,
    #[error("reconciliation exceeded {0} iterations")]
    IterationLimit(usize),
}

pub fn apply_plan<B: ReconcileBackend>(
    backend: &mut B,
    max_iterations: usize,
) -> Result<ReconcileReport, ReconcileError> {
    let mut report = ReconcileReport::default();
    let mut previous_fingerprint = None;

    for iteration in 0..max_iterations {
        report.iterations = iteration + 1;
        let snapshot = backend.snapshot().map_err(ReconcileError::Snapshot)?;
        let fingerprint = format!("{snapshot:?}");
        let next = plan(&snapshot);
        if next.stable {
            report.stable = true;
            return Ok(report);
        }
        if next.mutations.is_empty() {
            return Ok(report);
        }
        if previous_fingerprint.as_ref() == Some(&fingerprint) && iteration > 0 {
            return Err(ReconcileError::NoProgress);
        }
        previous_fingerprint = Some(fingerprint);

        for mutation in next.mutations {
            let result = match &mutation {
                Mutation::CreateTrailing => backend.create_desktop().map(|_| ()),
                Mutation::Remove { desktop, fallback } => {
                    backend.remove_desktop(desktop, fallback)
                }
            };
            result.map_err(|cause| ReconcileError::Mutation {
                operation: format!("{mutation:?}"),
                cause,
            })?;
            report.mutations.push(mutation);
        }
    }

    Err(ReconcileError::IterationLimit(max_iterations))
}
