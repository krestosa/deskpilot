// File purpose: Re-exports the reconciliation model and engine as the module public API.
mod engine;
mod model;

pub use engine::{
    apply_plan, ReconcileBackend, ReconcileError, ReconcilePass, ReconcileReport, ReconcileRuntime,
};
pub use model::{plan, DesktopId, DesktopState, Mutation, Occupancy, Plan};
