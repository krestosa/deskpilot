use crate::config::Config;
use crate::reconciliation::{DesktopId, DesktopState, Occupancy};
use std::collections::HashMap;
use std::ffi::c_void;
use std::mem::{size_of, zeroed};
use windows_sys::Win32::Foundation::{CloseHandle, BOOL, HWND, LPARAM, RECT};
use windows_sys::Win32::Graphics::Dwm::{DwmGetWindowAttribute, DWMWA_CLOAKED};
use windows_sys::Win32::Graphics::Gdi::{
    GetMonitorInfoW, MonitorFromWindow, MONITORINFO, MONITOR_DEFAULTTONEAREST,
};
use windows_sys::Win32::System::ProcessStatus::K32GetModuleFileNameExW;
use windows_sys::Win32::System::Threading::{OpenProcess, PROCESS_QUERY_LIMITED_INFORMATION};
use windows_sys::Win32::UI::WindowsAndMessaging::{
    EnumWindows, GetClassNameW, GetForegroundWindow, GetWindow, GetWindowLongPtrW, GetWindowRect,
    GetWindowThreadProcessId, IsWindow, IsWindowVisible, GWL_EXSTYLE, GW_OWNER, WS_EX_NOACTIVATE,
    WS_EX_TOOLWINDOW,
};

use super::desktops::WinvdBackend;
use super::system::current_process_id;
use super::util::from_wide;

pub fn snapshot(
    backend: &WinvdBackend,
    config: &Config,
    grace: &HashMap<DesktopId, bool>,
) -> Result<Vec<DesktopState>, String> {
    let desktops = backend.list()?;
    let current = backend.current()?;
    let mut occupancy: HashMap<DesktopId, Occupancy> = desktops
        .iter()
        .map(|desktop| (desktop.id.clone(), Occupancy::Empty))
        .collect();

    for hwnd in enumerate_windows() {
        let Some(identity) = inspect_identity(hwnd) else {
            continue;
        };
        if identity.process_id == current_process_id() || ignored_class(&identity.class_name) {
            continue;
        }
        if !is_eligible_application_window(hwnd) {
            continue;
        }
        if config
            .windows
            .ignore_classes
            .iter()
            .any(|value| value.eq_ignore_ascii_case(&identity.class_name))
        {
            continue;
        }

        let executable = match executable_name(identity.process_id) {
            Ok(executable) => executable,
            Err(_) => {
                mark_unmapped_candidate_unknown(
                    &mut occupancy,
                    &current.id,
                    window_is_cloaked(hwnd),
                );
                continue;
            }
        };
        if ignored_shell_executable(&executable)
            || config
                .windows
                .ignore_executables
                .iter()
                .any(|value| value.eq_ignore_ascii_case(&executable))
        {
            continue;
        }

        let desktop = match backend.desktop_for_window(hwnd) {
            Ok(desktop) => desktop,
            Err(_) => {
                mark_unmapped_candidate_unknown(
                    &mut occupancy,
                    &current.id,
                    window_is_cloaked(hwnd),
                );
                continue;
            }
        };
        if !occupancy.contains_key(&desktop) {
            continue;
        }

        match backend.is_window_pinned(hwnd) {
            Ok(true) => continue,
            Err(_) => {
                occupancy.insert(desktop, Occupancy::Unknown);
                continue;
            }
            Ok(false) => {
                occupancy.insert(desktop, Occupancy::Occupied);
            }
        }
    }

    Ok(desktops
        .into_iter()
        .map(|desktop| DesktopState {
            current: desktop.id == current.id,
            empty_grace_elapsed: grace.get(&desktop.id).copied().unwrap_or(false),
            occupancy: occupancy.remove(&desktop.id).unwrap_or(Occupancy::Unknown),
            id: desktop.id,
        })
        .collect())
}

pub fn exclusive_fullscreen_active() -> bool {
    unsafe {
        let hwnd = GetForegroundWindow();
        if hwnd == 0 {
            return false;
        }
        let Some(identity) = inspect_identity(hwnd) else {
            return false;
        };
        if identity.process_id == current_process_id()
            || ignored_class(&identity.class_name)
            || !is_foreground_application_window(hwnd)
        {
            return false;
        }
        let Ok(executable) = executable_name(identity.process_id) else {
            return false;
        };
        if ignored_shell_executable(&executable) {
            return false;
        }

        let mut window_rect: RECT = zeroed();
        if GetWindowRect(hwnd, &mut window_rect) == 0 {
            return false;
        }
        let monitor = MonitorFromWindow(hwnd, MONITOR_DEFAULTTONEAREST);
        if monitor == 0 {
            return false;
        }
        let mut info = MONITORINFO {
            cbSize: size_of::<MONITORINFO>() as u32,
            rcMonitor: zeroed(),
            rcWork: zeroed(),
            dwFlags: 0,
        };
        if GetMonitorInfoW(monitor, &mut info) == 0 {
            return false;
        }
        rect_covers(window_rect, info.rcMonitor)
    }
}

#[derive(Debug)]
struct BasicWindow {
    class_name: String,
    process_id: u32,
}

fn enumerate_windows() -> Vec<HWND> {
    unsafe extern "system" fn callback(hwnd: HWND, parameter: LPARAM) -> BOOL {
        let windows = unsafe { &mut *(parameter as *mut Vec<HWND>) };
        windows.push(hwnd);
        1
    }
    let mut windows = Vec::new();
    unsafe { EnumWindows(Some(callback), (&mut windows as *mut Vec<HWND>) as LPARAM) };
    windows
}

fn inspect_identity(hwnd: HWND) -> Option<BasicWindow> {
    unsafe {
        if IsWindow(hwnd) == 0 {
            return None;
        }
        let mut class = [0_u16; 256];
        let length = GetClassNameW(hwnd, class.as_mut_ptr(), class.len() as i32);
        if length <= 0 {
            return None;
        }
        let mut process_id = 0;
        GetWindowThreadProcessId(hwnd, &mut process_id);
        if process_id == 0 {
            return None;
        }
        Some(BasicWindow {
            class_name: from_wide(&class[..length as usize]),
            process_id,
        })
    }
}

fn is_eligible_application_window(hwnd: HWND) -> bool {
    unsafe {
        if GetWindow(hwnd, GW_OWNER) != 0 {
            return false;
        }
        let ex_style = GetWindowLongPtrW(hwnd, GWL_EXSTYLE) as u32;
        if ex_style & (WS_EX_TOOLWINDOW | WS_EX_NOACTIVATE) != 0 {
            return false;
        }
        IsWindowVisible(hwnd) != 0 || window_is_cloaked(hwnd)
    }
}

fn is_foreground_application_window(hwnd: HWND) -> bool {
    unsafe {
        if IsWindowVisible(hwnd) == 0 || GetWindow(hwnd, GW_OWNER) != 0 || window_is_cloaked(hwnd) {
            return false;
        }
        let ex_style = GetWindowLongPtrW(hwnd, GWL_EXSTYLE) as u32;
        ex_style & (WS_EX_TOOLWINDOW | WS_EX_NOACTIVATE) == 0
    }
}

fn window_is_cloaked(hwnd: HWND) -> bool {
    unsafe {
        let mut cloaked = 0_u32;
        DwmGetWindowAttribute(
            hwnd,
            DWMWA_CLOAKED as u32,
            (&mut cloaked as *mut u32).cast::<c_void>(),
            size_of::<u32>() as u32,
        ) >= 0
            && cloaked != 0
    }
}

fn executable_name(process_id: u32) -> Result<String, String> {
    unsafe {
        let process = OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, 0, process_id);
        if process == 0 {
            return Err("OpenProcess failed".to_string());
        }
        let mut buffer = vec![0_u16; 32_768];
        let length = K32GetModuleFileNameExW(process, 0, buffer.as_mut_ptr(), buffer.len() as u32);
        CloseHandle(process);
        if length == 0 {
            return Err("K32GetModuleFileNameExW failed".to_string());
        }
        let path = from_wide(&buffer[..length as usize]);
        Ok(path.rsplit(['\\', '/']).next().unwrap_or(&path).to_string())
    }
}

fn mark_unmapped_candidate_unknown(
    occupancy: &mut HashMap<DesktopId, Occupancy>,
    current: &DesktopId,
    cloaked: bool,
) {
    if cloaked {
        for (desktop, state) in occupancy.iter_mut() {
            if desktop != current && *state == Occupancy::Empty {
                *state = Occupancy::Unknown;
            }
        }
    } else if let Some(state) = occupancy.get_mut(current) {
        if *state == Occupancy::Empty {
            *state = Occupancy::Unknown;
        }
    }
}

fn rect_covers(window: RECT, monitor: RECT) -> bool {
    window.left <= monitor.left
        && window.top <= monitor.top
        && window.right >= monitor.right
        && window.bottom >= monitor.bottom
}

fn ignored_shell_executable(executable: &str) -> bool {
    const EXECUTABLES: &[&str] = &[
        "backgroundTaskHost.exe",
        "ctfmon.exe",
        "LockApp.exe",
        "RuntimeBroker.exe",
        "SearchHost.exe",
        "SecurityHealthSystray.exe",
        "ShellExperienceHost.exe",
        "ShellHost.exe",
        "sihost.exe",
        "StartMenuExperienceHost.exe",
        "SystemSettingsBroker.exe",
        "taskhostw.exe",
        "TextInputHost.exe",
        "WidgetService.exe",
        "Widgets.exe",
    ];
    EXECUTABLES
        .iter()
        .any(|value| value.eq_ignore_ascii_case(executable))
}

fn ignored_class(class_name: &str) -> bool {
    const CLASSES: &[&str] = &[
        "ApplicationManager_DesktopShellWindow",
        "ControlCenterWindow",
        "EdgeUiInputTopWndClass",
        "ForegroundStaging",
        "MultitaskingViewFrame",
        "NotifyIconOverflowWindow",
        "Progman",
        "SearchHost",
        "Shell_InputSwitchTopLevelWindow",
        "Shell_SecondaryTrayWnd",
        "Shell_TrayWnd",
        "StartMenuExperienceHost",
        "SystemTray_Main",
        "SystemTray_Secondary",
        "TopLevelWindowForOverflowXamlIsland",
        "Windows.UI.Composition.DesktopWindowContentBridge",
        "Windows.UI.Core.CoreWindow",
        "Windows.UI.Input.InputSite.WindowClass",
        "WorkerW",
        "XamlExplorerHostIslandWindow",
        "Xaml_WindowedPopupClass",
    ];
    CLASSES
        .iter()
        .any(|value| value.eq_ignore_ascii_case(class_name))
}

#[cfg(test)]
mod tests {
    use super::{
        ignored_class, ignored_shell_executable, mark_unmapped_candidate_unknown, rect_covers,
    };
    use crate::reconciliation::{DesktopId, Occupancy};
    use std::collections::HashMap;
    use windows_sys::Win32::Foundation::RECT;

    #[test]
    fn shell_surfaces_are_not_user_applications() {
        assert!(ignored_class("Progman"));
        assert!(ignored_class("WorkerW"));
        assert!(ignored_class("Windows.UI.Input.InputSite.WindowClass"));
        assert!(ignored_shell_executable("StartMenuExperienceHost.exe"));
        assert!(ignored_shell_executable("searchhost.EXE"));
        assert!(ignored_shell_executable("RuntimeBroker.exe"));
        assert!(!ignored_shell_executable("notepad.exe"));
    }

    #[test]
    fn visible_unmapped_candidate_only_blocks_current_desktop() {
        let current = DesktopId("current".to_string());
        let other = DesktopId("other".to_string());
        let mut occupancy = HashMap::from([
            (current.clone(), Occupancy::Empty),
            (other.clone(), Occupancy::Empty),
        ]);
        mark_unmapped_candidate_unknown(&mut occupancy, &current, false);
        assert_eq!(occupancy[&current], Occupancy::Unknown);
        assert_eq!(occupancy[&other], Occupancy::Empty);
    }

    #[test]
    fn cloaked_unmapped_candidate_only_blocks_non_current_desktops() {
        let current = DesktopId("current".to_string());
        let other = DesktopId("other".to_string());
        let mut occupancy = HashMap::from([
            (current.clone(), Occupancy::Empty),
            (other.clone(), Occupancy::Empty),
        ]);
        mark_unmapped_candidate_unknown(&mut occupancy, &current, true);
        assert_eq!(occupancy[&current], Occupancy::Empty);
        assert_eq!(occupancy[&other], Occupancy::Unknown);
    }

    #[test]
    fn fullscreen_requires_monitor_coverage() {
        let monitor = RECT {
            left: 0,
            top: 0,
            right: 1920,
            bottom: 1080,
        };
        assert!(rect_covers(monitor, monitor));
        assert!(rect_covers(
            RECT {
                left: -1,
                top: -1,
                right: 1921,
                bottom: 1081,
            },
            monitor,
        ));
        assert!(!rect_covers(
            RECT {
                left: 0,
                top: 0,
                right: 1919,
                bottom: 1080,
            },
            monitor,
        ));
    }
}
