// File purpose: Wraps Windows virtual desktop enumeration, navigation, creation, removal, pin detection, and window membership.
use crate::config::NavigationMode;
use crate::reconciliation::DesktopId;
use crate::wheel::{target_index, Step};
use std::ffi::c_void;
use windows::Win32::Foundation::HWND as WinHwnd;
use windows_sys::Win32::Foundation::HWND;

use super::system::{windows_version, WindowsVersion};

#[derive(Debug, Clone, PartialEq, Eq)]
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
pub struct DesktopTopology {
    pub desktops: Vec<DesktopInfo>,
    pub current: DesktopInfo,
}

#[derive(Debug, Clone)]
pub struct WinvdBackend {
    version: WindowsVersion,
    compatible: bool,
    compatibility_reason: String,
}

impl WinvdBackend {
    // Function purpose: Detects whether the current Windows shell family is explicitly supported.
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

    // Function purpose: Returns one ordered desktop enumeration.
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

    // Function purpose: Returns the current desktop identity and index.
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

    // Function purpose: Captures one ordered topology and verifies that the current desktop belongs to it.
    pub fn topology(&self) -> Result<DesktopTopology, String> {
        let desktops = self.list()?;
        let current = self.current()?;
        let Some(current_in_list) = desktops
            .iter()
            .find(|desktop| desktop.id == current.id)
            .cloned()
        else {
            return Err("desktop topology changed while current desktop was resolved".to_string());
        };
        Ok(DesktopTopology {
            desktops,
            current: current_in_list,
        })
    }

    // Function purpose: Verifies that an earlier ordered snapshot still matches the Windows shell topology.
    pub fn topology_matches(&self, expected: &[DesktopInfo]) -> Result<bool, String> {
        let current = self.list()?;
        Ok(same_topology(expected, &current))
    }

    // Function purpose: Switches to a desktop by stable identifier after resolving its current index.
    pub fn switch_to_id(&self, desktop: &DesktopId) -> Result<(), String> {
        let desktops = self.list()?;
        let target = self.find_in(&desktops, desktop)?;
        winvd::switch_desktop(target.index as u32).map_err(format_error)
    }

    // Function purpose: Switches relative to one current topology while targeting the selected stable desktop ID.
    pub fn switch_relative(&self, step: Step, mode: NavigationMode) -> Result<DesktopInfo, String> {
        let topology = self.topology()?;
        let target = target_index(
            topology.current.index,
            topology.desktops.len(),
            step,
            mode,
        )
        .ok_or_else(|| "navigation reached a clamped edge".to_string())?;
        let target = topology
            .desktops
            .get(target)
            .cloned()
            .ok_or_else(|| "target desktop disappeared".to_string())?;
        self.switch_to_id(&target.id)?;
        Ok(target)
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

    // Function purpose: Removes one stable desktop only after a second enumeration proves the topology did not change.
    pub fn remove(&self, desktop: &DesktopId, fallback: &DesktopId) -> Result<(), String> {
        self.require_compatible()?;
        if desktop == fallback {
            return Err("desktop and fallback must be different".to_string());
        }
        let first = self.list()?;
        let second = self.list()?;
        if !same_topology(&first, &second) {
            return Err("desktop topology changed before removal; retrying from a fresh snapshot is required".to_string());
        }
        let desktop = self.find_in(&second, desktop)?;
        let fallback = self.find_in(&second, fallback)?;
        winvd::remove_desktop(desktop.index as u32, fallback.index as u32).map_err(format_error)
    }

    pub fn desktop_for_window(&self, hwnd: HWND) -> Result<DesktopId, String> {
        self.require_compatible()?;
        let desktop = winvd::get_desktop_by_window(to_win_hwnd(hwnd)).map_err(format_error)?;
        let id = desktop.get_id().map_err(format_error)?;
        Ok(DesktopId(format!("{id:?}")))
    }

    // Function purpose: Tests membership using an index from a caller-owned stable topology snapshot.
    pub fn is_window_on_desktop_index(&self, hwnd: HWND, index: usize) -> Result<bool, String> {
        self.require_compatible()?;
        winvd::is_window_on_desktop(index as u32, to_win_hwnd(hwnd)).map_err(format_error)
    }

    pub fn is_window_on_desktop(&self, hwnd: HWND, desktop: &DesktopId) -> Result<bool, String> {
        let desktops = self.list()?;
        let desktop = self.find_in(&desktops, desktop)?;
        self.is_window_on_desktop_index(hwnd, desktop.index)
    }

    pub fn is_window_on_current_desktop(&self, hwnd: HWND) -> Result<bool, String> {
        self.require_compatible()?;
        winvd::is_window_on_current_desktop(to_win_hwnd(hwnd)).map_err(format_error)
    }

    pub fn is_window_pinned(&self, hwnd: HWND) -> Result<bool, String> {
        self.require_compatible()?;
        winvd::is_pinned_window(to_win_hwnd(hwnd)).map_err(format_error)
    }

    fn find_in<'a>(
        &self,
        desktops: &'a [DesktopInfo],
        id: &DesktopId,
    ) -> Result<&'a DesktopInfo, String> {
        desktops
            .iter()
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

fn same_topology(left: &[DesktopInfo], right: &[DesktopInfo]) -> bool {
    left.len() == right.len()
        && left
            .iter()
            .zip(right)
            .all(|(left, right)| left.id == right.id && left.index == right.index)
}

fn is_supported_version(version: WindowsVersion) -> bool {
    version.major == 10
        && match version.build {
            26_100 => version.revision >= 2_605,
            26_200 => version.revision >= 8_117,
            28_000 => true,
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
    use super::{is_supported_version, same_topology, DesktopInfo, WindowsVersion};
    use crate::reconciliation::DesktopId;

    #[test]
    fn accepts_supported_windows_families() {
        assert!(is_supported_version(WindowsVersion {
            major: 10,
            minor: 0,
            build: 26_100,
            revision: 8_655,
        }));
        assert!(is_supported_version(WindowsVersion {
            major: 10,
            minor: 0,
            build: 26_200,
            revision: 8_117,
        }));
        assert!(is_supported_version(WindowsVersion {
            major: 10,
            minor: 0,
            build: 28_000,
            revision: 1,
        }));
    }

    #[test]
    fn rejects_manifest_virtualized_and_unknown_builds() {
        assert!(!is_supported_version(WindowsVersion {
            major: 6,
            minor: 2,
            build: 9_200,
            revision: 8_655,
        }));
        assert!(!is_supported_version(WindowsVersion {
            major: 10,
            minor: 0,
            build: 22_631,
            revision: 5_000,
        }));
    }

    #[test]
    fn topology_comparison_detects_reordering() {
        let first = vec![
            DesktopInfo {
                id: DesktopId("a".to_string()),
                index: 0,
            },
            DesktopInfo {
                id: DesktopId("b".to_string()),
                index: 1,
            },
        ];
        let mut reordered = first.clone();
        reordered.swap(0, 1);
        assert!(same_topology(&first, &first));
        assert!(!same_topology(&first, &reordered));
    }
}
