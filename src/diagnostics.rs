use serde::Serialize;
use std::path::PathBuf;

use crate::logging::timestamp_utc;
use crate::{APP_NAME, APP_VERSION};

#[derive(Debug, Clone, Serialize)]
pub struct BackendDiagnostic {
    pub name: String,
    pub compatible: bool,
    pub capabilities: Vec<String>,
    pub error: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct OccupancyDiagnostic {
    pub occupied: usize,
    pub empty: usize,
    pub unknown: usize,
}

#[derive(Debug, Clone, Serialize)]
pub struct DoctorReport {
    pub timestamp: String,
    pub app: String,
    pub app_version: String,
    pub executable_path: PathBuf,
    pub data_directory: PathBuf,
    pub configuration_status: String,
    pub windows_version: String,
    pub windows_build: u32,
    pub windows_revision: u32,
    pub process_architecture: String,
    pub integrity_level: String,
    pub interactive_session: bool,
    pub explorer_running: bool,
    pub backend: BackendDiagnostic,
    pub desktop_count: Option<usize>,
    pub current_desktop: Option<String>,
    pub occupancy: OccupancyDiagnostic,
    pub hook_state: String,
    pub ipc_state: String,
    pub dynamic_reconciliation: String,
    pub last_reconciliation: Option<String>,
    pub recent_errors: Vec<String>,
    pub portable_write_test: bool,
}

impl DoctorReport {
    pub fn unavailable(data_directory: PathBuf, reason: impl Into<String>) -> Self {
        Self {
            timestamp: timestamp_utc(),
            app: APP_NAME.to_string(),
            app_version: APP_VERSION.to_string(),
            executable_path: std::env::current_exe().unwrap_or_default(),
            data_directory,
            configuration_status: "not loaded".to_string(),
            windows_version: "unavailable".to_string(),
            windows_build: 0,
            windows_revision: 0,
            process_architecture: std::env::consts::ARCH.to_string(),
            integrity_level: "unavailable".to_string(),
            interactive_session: false,
            explorer_running: false,
            backend: BackendDiagnostic {
                name: "winvd".to_string(),
                compatible: false,
                capabilities: Vec::new(),
                error: Some(reason.into()),
            },
            desktop_count: None,
            current_desktop: None,
            occupancy: OccupancyDiagnostic {
                occupied: 0,
                empty: 0,
                unknown: 0,
            },
            hook_state: "not running".to_string(),
            ipc_state: "not running".to_string(),
            dynamic_reconciliation: "not running".to_string(),
            last_reconciliation: None,
            recent_errors: Vec::new(),
            portable_write_test: false,
        }
    }
}
