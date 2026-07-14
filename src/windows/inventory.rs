// File purpose: Enumerates top-level windows and classifies desktop occupancy conservatively.
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

use super::desktops::{DesktopInfo, WinvdBackend};
use super::system::current_process_id;
use super::util::from_wide;

// Function purpose: Builds a fresh ordered desktop snapshot with current occupancy and empty-grace state.
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
        if identity.process_id == current_process_id()
            || ignored_class(&identity.class_name)
            || config
                .windows
                .ignore_classes
                .iter()
                .any(|value| value.eq_ignore_ascii_case(&identity.class_name))
            || !is_eligible_application_window(hwnd)
        {
            continue;
        }

        if let Ok(executable) = executable_name(identity.process_id) {
            if ignored_shell_executable(&executable)
                || config
                    .windows
                    .ignore_executables
                    .iter()
                    .any(|value| value.eq_ignore_ascii_case(&executable))
            {
                continue;
            }
        }

        if backend.is_window_pinned(hwnd).is_ok_and(|pinned| pinned) {
            continue;
        }

        if let Some(desktop) = locate_window_desktop(backend, &desktops, &current.id, hwnd) {
            occupancy.insert(desktop, Occupancy::Occupied);
        }
    }

    Ok(desktops
        .into_iter()
        .map(|desktop| DesktopState {
            current: desktop.id == current.id,
            empty_grace_elapsed: grace.get(&desktop.id).copied().unwrap_or(false),
            occupancy: occupancy.remove(&desktop.id).unwrap_or(Occupancy::Empty),
            id: desktop.id,
        })
        .collect())
}

// Function purpose: Locates window desktop.
fn locate_window_desktop(
    backend: &WinvdBackend,
    desktops: &[DesktopInfo],
    current: &DesktopId,
    hwnd: HWND,
) -> Option<DesktopId> {
    if backend
        .is_window_on_current_desktop(hwnd)
        .is_ok_and(|present| present)
    {
        return Some(current.clone());
    }

    if let Ok(desktop) = backend.desktop_for_window(hwnd) {
        if desktops.iter().any(|candidate| candidate.id == desktop) {
            return Some(desktop);
        }
    }

    let mut matched = None;
    for desktop in desktops {
        match backend.is_window_on_desktop(hwnd, &desktop.id) {
            Ok(true) if matched.is_none() => matched = Some(desktop.id.clone()),
            Ok(true) => return None,
            Ok(false) | Err(_) => {}
        }
    }
    matched
}

// Function purpose: Performs the exclusive fullscreen active operation required by this module.
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

// Function purpose: Enumerates windows.
fn enumerate_windows() -> Vec<HWND> {
    // Function purpose: Handles the native callback callback and forwards only the relevant event.
    unsafe extern "system" fn callback(hwnd: HWND, parameter: LPARAM) -> BOOL {
        let windows = unsafe { &mut *(parameter as *mut Vec<HWND>) };
        windows.push(hwnd);
        1
    }
    let mut windows = Vec::new();
    unsafe { EnumWindows(Some(callback), (&mut windows as *mut Vec<HWND>) as LPARAM) };
    windows
}

// Function purpose: Performs the inspect identity operation required by this module.
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

// Function purpose: Returns whether eligible application window.
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

// Function purpose: Returns whether foreground application window.
fn is_foreground_application_window(hwnd: HWND) -> bool {
    unsafe {
        if IsWindowVisible(hwnd) == 0 || GetWindow(hwnd, GW_OWNER) != 0 || window_is_cloaked(hwnd) {
            return false;
        }
        let ex_style = GetWindowLongPtrW(hwnd, GWL_EXSTYLE) as u32;
        ex_style & (WS_EX_TOOLWINDOW | WS_EX_NOACTIVATE) == 0
    }
}

// Function purpose: Performs the window is cloaked operation required by this module.
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

// Function purpose: Performs the executable name operation required by this module.
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

// Function purpose: Performs the rect covers operation required by this module.
fn rect_covers(window: RECT, monitor: RECT) -> bool {
    window.left <= monitor.left
        && window.top <= monitor.top
        && window.right >= monitor.right
        && window.bottom >= monitor.bottom
}

// Function purpose: Performs the ignored shell executable operation required by this module.
fn ignored_shell_executable(executable: &str) -> bool {
    const EXECUTABLES: &[&str] = &[
        "backgroundTaskHost.exe",
        "ctfmon.exe",
        "dwm.exe",
        "explorer.exe",
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

// Function purpose: Performs the ignored class operation required by this module.
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
    use super::{ignored_class, ignored_shell_executable, rect_covers};
    use windows_sys::Win32::Foundation::RECT;

    // Function purpose: Verifies the shell surfaces are not user applications scenario and its expected safety or state invariant.
    #[test]
    fn shell_surfaces_are_not_user_applications() {
        assert!(ignored_class("Progman"));
        assert!(ignored_class("WorkerW"));
        assert!(ignored_class("Windows.UI.Input.InputSite.WindowClass"));
        assert!(ignored_shell_executable("StartMenuExperienceHost.exe"));
        assert!(ignored_shell_executable("searchhost.EXE"));
        assert!(ignored_shell_executable("RuntimeBroker.exe"));
        assert!(ignored_shell_executable("explorer.exe"));
        assert!(!ignored_shell_executable("notepad.exe"));
    }

    // Function purpose: Verifies the fullscreen requires monitor coverage scenario and its expected safety or state invariant.
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
