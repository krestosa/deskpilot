mod engine;
mod model;

pub use engine::{apply_plan, ReconcileBackend, ReconcileError, ReconcileReport};
pub use model::{plan, DesktopId, DesktopState, Mutation, Occupancy, Plan};
