use crate::config::NavigationMode;
use crate::reconciliation::DesktopId;
use crate::wheel::{target_index, Step};
use std::ffi::c_void;
use windows::Win32::Foundation::HWND as WinHwnd;
use windows_sys::Win32::Foundation::HWND;

use super::system::{windows_version, WindowsVersion};

#[derive(Debug, Clone)]
pub struct DesktopInfo {
    pub id: DesktopId,
    pub index: usize,
}

#[derive(Debug, Clone)]
pub struct BackendCapabilities {
    pub enumerate: bool,
    pub switch: bool,
    pub create: bool,
    pub remove: bool,
    pub window_mapping: bool,
    pub pin_detection: bool,
}

#[derive(Debug, Clone)]
pub struct WinvdBackend {
    version: WindowsVersion,
    compatible: bool,
    compatibility_reason: String,
}

impl WinvdBackend {
    pub fn detect() -> Self {
        let version = windows_version();
        let compatible = is_supported_version(version);
        let compatibility_reason = if compatible {
            format!(
                "recognized Windows 11 build {}.{}",
                version.build, version.revision
            )
        } else {
            format!(
                "unsupported Windows build {}.{}.{}.{}; destructive desktop operations are disabled",
                version.major, version.minor, version.build, version.revision
            )
        };
        Self {
            version,
            compatible,
            compatibility_reason,
        }
    }

    pub fn version(&self) -> WindowsVersion {
        self.version
    }

    pub fn compatible(&self) -> bool {
        self.compatible
    }

    pub fn compatibility_reason(&self) -> &str {
        &self.compatibility_reason
    }

    pub fn capabilities(&self) -> BackendCapabilities {
        BackendCapabilities {
            enumerate: self.compatible,
            switch: self.compatible,
            create: self.compatible,
            remove: self.compatible,
            window_mapping: self.compatible,
            pin_detection: self.compatible,
        }
    }

    pub fn list(&self) -> Result<Vec<DesktopInfo>, String> {
        self.require_compatible()?;
        let desktops = winvd::get_desktops().map_err(format_error)?;
        desktops
            .iter()
            .enumerate()
            .map(|(index, desktop)| {
                desktop
                    .get_id()
                    .map(|guid| DesktopInfo {
                        id: DesktopId(format!("{guid:?}")),
                        index,
                    })
                    .map_err(format_error)
            })
            .collect()
    }

    pub fn current(&self) -> Result<DesktopInfo, String> {
        self.require_compatible()?;
        let desktop = winvd::get_current_desktop().map_err(format_error)?;
        let index = desktop.get_index().map_err(format_error)? as usize;
        let id = desktop.get_id().map_err(format_error)?;
        Ok(DesktopInfo {
            id: DesktopId(format!("{id:?}")),
            index,
        })
    }

    pub fn switch_to_id(&self, desktop: &DesktopId) -> Result<(), String> {
        let target = self.find(desktop)?;
        winvd::switch_desktop(target.index as u32).map_err(format_error)
    }

    pub fn switch_relative(&self, step: Step, mode: NavigationMode) -> Result<DesktopInfo, String> {
        let desktops = self.list()?;
        let current = self.current()?;
        let target = target_index(current.index, desktops.len(), step, mode)
            .ok_or_else(|| "navigation reached a clamped edge".to_string())?;
        winvd::switch_desktop(target as u32).map_err(format_error)?;
        desktops
            .get(target)
            .cloned()
            .ok_or_else(|| "target desktop disappeared".to_string())
    }

    pub fn create(&self) -> Result<DesktopInfo, String> {
        self.require_compatible()?;
        let desktop = winvd::create_desktop().map_err(format_error)?;
        let index = desktop.get_index().map_err(format_error)? as usize;
        let id = desktop.get_id().map_err(format_error)?;
        Ok(DesktopInfo {
            id: DesktopId(format!("{id:?}")),
            index,
        })
    }

    pub fn remove(&self, desktop: &DesktopId, fallback: &DesktopId) -> Result<(), String> {
        self.require_compatible()?;
        let desktop = self.find(desktop)?;
        let fallback = self.find(fallback)?;
        winvd::remove_desktop(desktop.index as u32, fallback.index as u32).map_err(format_error)
    }

    pub fn desktop_for_window(&self, hwnd: HWND) -> Result<DesktopId, String> {
        self.require_compatible()?;
        let desktop = winvd::get_desktop_by_window(to_win_hwnd(hwnd)).map_err(format_error)?;
        let id = desktop.get_id().map_err(format_error)?;
        Ok(DesktopId(format!("{id:?}")))
    }

    pub fn is_window_on_desktop(
        &self,
        hwnd: HWND,
        desktop: &DesktopId,
    ) -> Result<bool, String> {
        self.require_compatible()?;
        let desktop = self.find(desktop)?;
        winvd::is_window_on_desktop(desktop.index as u32, to_win_hwnd(hwnd)).map_err(format_error)
    }

    pub fn is_window_pinned(&self, hwnd: HWND) -> Result<bool, String> {
        self.require_compatible()?;
        winvd::is_pinned_window(to_win_hwnd(hwnd)).map_err(format_error)
    }

    fn find(&self, id: &DesktopId) -> Result<DesktopInfo, String> {
        self.list()?
            .into_iter()
            .find(|desktop| &desktop.id == id)
            .ok_or_else(|| format!("desktop {} is no longer present", id.0))
    }

    fn require_compatible(&self) -> Result<(), String> {
        if self.compatible {
            Ok(())
        } else {
            Err(self.compatibility_reason.clone())
        }
    }
}

fn is_supported_version(version: WindowsVersion) -> bool {
    version.major == 10
        && match version.build {
            26_100 => version.revision >= 2_605,
            26_200 => version.revision >= 8_117,
            _ => false,
        }
}

fn to_win_hwnd(hwnd: HWND) -> WinHwnd {
    WinHwnd(hwnd as *mut c_void)
}

fn format_error(error: impl std::fmt::Debug) -> String {
    format!("{error:?}")
}

#[cfg(test)]
mod tests {
    use super::{is_supported_version, WindowsVersion};

    #[test]
    fn accepts_supported_26100_revisions() {
        assert!(is_supported_version(WindowsVersion {
            major: 10,
            minor: 0,
            build: 26_100,
            revision: 8_655,
        }));
    }

    #[test]
    fn rejects_manifest_virtualized_windows_8_version() {
        assert!(!is_supported_version(WindowsVersion {
            major: 6,
            minor: 2,
            build: 9_200,
            revision: 8_655,
        }));
    }

    #[test]
    fn preserves_safe_failure_for_unknown_windows_11_builds() {
        assert!(!is_supported_version(WindowsVersion {
            major: 10,
            minor: 0,
            build: 22_631,
            revision: 5_000,
        }));
    }
}
