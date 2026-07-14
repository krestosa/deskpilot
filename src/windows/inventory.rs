use crate::config::Config;
use crate::reconciliation::{DesktopId, DesktopState, Occupancy};
use std::collections::{HashMap, HashSet};
use std::mem::{size_of, zeroed};
use windows_sys::Win32::Foundation::{CloseHandle, BOOL, HWND, LPARAM, RECT};
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
    let mut removal_blocked = HashSet::new();

    for hwnd in enumerate_windows() {
        let Some(identity) = inspect_identity(hwnd) else {
            continue;
        };
        if identity.process_id == current_process_id() || ignored_class(&identity.class_name) {
            continue;
        }

        let desktop = match backend.desktop_for_window(hwnd) {
            Ok(desktop) => desktop,
            Err(_) => {
                for state in occupancy.values_mut() {
                    if *state == Occupancy::Empty {
                        *state = Occupancy::Unknown;
                    }
                }
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
                removal_blocked.insert(desktop.clone());
            }
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

        match executable_name(identity.process_id) {
            Ok(executable) => {
                if config
                    .windows
                    .ignore_executables
                    .iter()
                    .any(|value| value.eq_ignore_ascii_case(&executable))
                {
                    continue;
                }
                occupancy.insert(desktop, Occupancy::Occupied);
            }
            Err(_) => {
                occupancy.insert(desktop, Occupancy::Unknown);
            }
        }
    }

    for desktop in removal_blocked {
        if occupancy.get(&desktop) == Some(&Occupancy::Empty) {
            occupancy.insert(desktop, Occupancy::Unknown);
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
        window_rect.left <= info.rcMonitor.left
            && window_rect.top <= info.rcMonitor.top
            && window_rect.right >= info.rcMonitor.right
            && window_rect.bottom >= info.rcMonitor.bottom
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
        if IsWindowVisible(hwnd) == 0 || GetWindow(hwnd, GW_OWNER) != 0 {
            return false;
        }
        let ex_style = GetWindowLongPtrW(hwnd, GWL_EXSTYLE) as u32;
        ex_style & (WS_EX_TOOLWINDOW | WS_EX_NOACTIVATE) == 0
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

fn ignored_class(class_name: &str) -> bool {
    const CLASSES: &[&str] = &[
        "Shell_TrayWnd",
        "Shell_SecondaryTrayWnd",
        "Progman",
        "WorkerW",
        "Windows.UI.Core.CoreWindow",
        "XamlExplorerHostIslandWindow",
        "MultitaskingViewFrame",
        "ApplicationManager_DesktopShellWindow",
        "SearchHost",
        "StartMenuExperienceHost",
    ];
    CLASSES
        .iter()
        .any(|value| value.eq_ignore_ascii_case(class_name))
}
