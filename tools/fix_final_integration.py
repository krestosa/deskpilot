# File purpose: Applies the integration corrections discovered by Clippy after generating the final reconciliation fix.
from pathlib import Path


# Function purpose: Reads one UTF-8 repository file.
def read(path: str) -> str:
    return Path(path).read_text(encoding="utf-8")


# Function purpose: Writes one UTF-8 repository file with LF line endings.
def write(path: str, text: str) -> None:
    Path(path).write_text(text, encoding="utf-8", newline="\n")


# Function purpose: Replaces exactly one expected fragment and fails on an unexpected generated tree.
def replace_once(path: str, old: str, new: str) -> None:
    text = read(path)
    count = text.count(old)
    if count != 1:
        raise SystemExit(f"{path}: expected one replacement, found {count}: {old!r}")
    write(path, text.replace(old, new, 1))


replace_once(
    "src/reconciliation/mod.rs",
    "pub use engine::{apply_plan, ReconcileBackend, ReconcileError, ReconcileReport};",
    "pub use engine::{\n    apply_plan, ReconcileBackend, ReconcileError, ReconcilePass, ReconcileReport,\n    ReconcileRuntime,\n};",
)

replace_once(
    "src/app.rs",
    "    DesktopId, Mutation, Occupancy, ReconcileBackend, ReconcilePass, ReconcileRuntime,",
    "    DesktopId, Occupancy, ReconcileBackend, ReconcilePass, ReconcileRuntime,",
)

snapshot_method = '''    // Function purpose: Builds the same grace-aware snapshot used by reconciliation for diagnostics without applying a mutation.
    fn snapshot(&mut self) -> Result<Vec<crate::reconciliation::DesktopState>, String> {
        let config = self.config_read();
        let mut backend = AppReconcileBackend {
            backend: &self.backend,
            config: &config,
            empty_since: &mut self.empty_since,
        };
        backend.snapshot()
    }

'''
replace_once(
    "src/app.rs",
    "    // Function purpose: Handles tray.\n",
    snapshot_method + "    // Function purpose: Handles tray.\n",
)
