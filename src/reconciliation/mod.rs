// File purpose: Re-exports the reconciliation model and engine as the module public API.
mod engine;
mod model;
mod spare_guard;

pub use engine::{
    apply_plan, ReconcileBackend, ReconcileError, ReconcilePass, ReconcileReport, ReconcileRuntime,
};
pub use model::{plan, DesktopId, DesktopState, Mutation, Occupancy, Plan};
pub use spare_guard::{SpareGuard, SpareGuardResult, WindowToken};
