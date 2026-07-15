// File purpose: Implements the accessible native notification-area icon, menu, commands, and Explorer recovery.
use std::mem::{size_of, zeroed};
use std::sync::atomic::{AtomicBool, AtomicU8, Ordering};
use std::sync::mpsc::Sender;
use std::sync::{Arc, OnceLock};
use std::thread::{self, JoinHandle};
use windows_sys::Win32::Foundation::{HWND, LPARAM, LRESULT, POINT, WPARAM};
use windows_sys::Win32::System::LibraryLoader::GetModuleHandleW;
use windows_sys::Win32::System::Threading::GetCurrentProcessId;
use windows_sys::Win32::UI::Shell::{
    Shell_NotifyIconW, NIF_ICON, NIF_MESSAGE, NIF_TIP, NIM_ADD, NIM_DELETE, NIM_MODIFY,
    NIM_SETFOCUS, NIM_SETVERSION, NOTIFYICONDATAW, NOTIFYICON_VERSION_4,
};
use windows_sys::Win32::UI::WindowsAndMessaging::{
    AppendMenuW, CheckMenuItem, CreatePopupMenu, CreateWindowExW, DefWindowProcW, DestroyMenu,
    DispatchMessageW, GetCursorPos, GetMessageW, LoadIconW, PostMessageW, PostQuitMessage,
    RegisterClassW, RegisterWindowMessageW, SetForegroundWindow, TrackPopupMenu, TranslateMessage,
    CS_HREDRAW, CS_VREDRAW, CW_USEDEFAULT, IDI_APPLICATION, IDI_ERROR, IDI_WARNING, MF_CHECKED,
    MF_GRAYED, MF_SEPARATOR, MF_STRING, MF_UNCHECKED, MSG, TPM_BOTTOMALIGN, TPM_LEFTALIGN,
    TPM_RIGHTBUTTON, WM_APP, WM_CLOSE, WM_COMMAND, WM_CONTEXTMENU, WM_DESTROY, WM_LBUTTONDBLCLK,
    WM_NULL, WM_RBUTTONUP, WM_USER, WNDCLASSW, WS_OVERLAPPED,
};

use crate::config::{NavigationMode, WheelDirection};
use crate::windows::util::wide;

const CALLBACK_MESSAGE: u32 = WM_APP + 42;
const TRAY_ID: u32 = 1;
const NIN_SELECT_EVENT: u32 = WM_USER;
const NIN_KEYSELECT_EVENT: u32 = WM_USER + 1;

const CMD_TOGGLE_ENABLED: usize = 1001;
const CMD_TOGGLE_DYNAMIC: usize = 1002;
const CMD_TOGGLE_DIRECTION: usize = 1003;
const CMD_TOGGLE_NAVIGATION: usize = 1004;
const CMD_RECONCILE: usize = 1005;
const CMD_RELOAD: usize = 1006;
const CMD_OPEN_CONFIG: usize = 1007;
const CMD_DIAGNOSTICS: usize = 1008;
const CMD_TOGGLE_STARTUP: usize = 1009;
const CMD_OPEN_LOGS: usize = 1010;
const CMD_EXIT: usize = 1011;

static COMMAND_SENDER: OnceLock<Sender<TrayCommand>> = OnceLock::new();
static STATE: OnceLock<Arc<TrayState>> = OnceLock::new();
static TASKBAR_CREATED: OnceLock<u32> = OnceLock::new();

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TrayCommand {
    ToggleEnabled,
    ToggleDynamic,
    ToggleDirection,
    ToggleNavigation,
    Reconcile,
    Reload,
    OpenConfig,
    Diagnostics,
    ToggleStartup,
    OpenLogs,
    Exit,
}

#[derive(Debug)]
pub struct TrayState {
    enabled: AtomicBool,
    dynamic: AtomicBool,
    direction: AtomicU8,
    navigation: AtomicU8,
    startup: AtomicBool,
    backend_ready: AtomicBool,
    error: AtomicBool,
    hwnd: std::sync::Mutex<HWND>,
}

impl TrayState {
    fn new() -> Self {
        Self {
            enabled: AtomicBool::new(true),
            dynamic: AtomicBool::new(true),
            direction: AtomicU8::new(0),
            navigation: AtomicU8::new(0),
            startup: AtomicBool::new(false),
            backend_ready: AtomicBool::new(false),
            error: AtomicBool::new(false),
            hwnd: std::sync::Mutex::new(0),
        }
    }

    pub fn update(
        &self,
        enabled: bool,
        dynamic: bool,
        direction: WheelDirection,
        navigation: NavigationMode,
        startup: bool,
        backend_ready: bool,
        error: bool,
    ) {
        self.enabled.store(enabled, Ordering::Release);
        self.dynamic.store(dynamic, Ordering::Release);
        self.direction.store(
            u8::from(matches!(direction, WheelDirection::Inverted)),
            Ordering::Release,
        );
        self.navigation.store(
            u8::from(matches!(navigation, NavigationMode::Wrap)),
            Ordering::Release,
        );
        self.startup.store(startup, Ordering::Release);
        self.backend_ready.store(backend_ready, Ordering::Release);
        self.error.store(error, Ordering::Release);
        if let Ok(hwnd) = self.hwnd.lock() {
            if *hwnd != 0 {
                modify_icon(*hwnd, error, enabled);
            }
        }
    }
}

pub struct Tray {
    state: Arc<TrayState>,
    thread: Option<JoinHandle<Result<(), String>>>,
}

impl Tray {
    pub fn start(sender: Sender<TrayCommand>) -> Result<Self, String> {
        COMMAND_SENDER
            .set(sender)
            .map_err(|_| "tray command sender already initialized".to_string())?;
        let state = Arc::new(TrayState::new());
        STATE
            .set(state.clone())
            .map_err(|_| "tray state already initialized".to_string())?;
        let (ready_tx, ready_rx) = std::sync::mpsc::channel();
        let thread = thread::Builder::new()
            .name("deskpilot-tray".to_string())
            .spawn(move || tray_loop(ready_tx))
            .map_err(|error| error.to_string())?;
        ready_rx.recv().map_err(|error| error.to_string())??;
        Ok(Self {
            state,
            thread: Some(thread),
        })
    }

    pub fn state(&self) -> &Arc<TrayState> {
        &self.state
    }

    pub fn stop(&mut self) {
        if let Ok(hwnd) = self.state.hwnd.lock() {
            if *hwnd != 0 {
                unsafe { PostMessageW(*hwnd, WM_CLOSE, 0, 0) };
            }
        }
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }
}

impl Drop for Tray {
    fn drop(&mut self) {
        self.stop();
    }
}

fn tray_loop(ready: Sender<Result<(), String>>) -> Result<(), String> {
    unsafe {
        let module = GetModuleHandleW(std::ptr::null());
        let taskbar_created = RegisterWindowMessageW(wide("TaskbarCreated").as_ptr());
        if taskbar_created != 0 {
            let _ = TASKBAR_CREATED.set(taskbar_created);
        }
        let class_name = wide(format!("DeskPilot.Tray.{}", GetCurrentProcessId()));
        let window_class = WNDCLASSW {
            style: CS_HREDRAW | CS_VREDRAW,
            lpfnWndProc: Some(window_proc),
            hInstance: module,
            lpszClassName: class_name.as_ptr(),
            ..zeroed()
        };
        if RegisterClassW(&window_class) == 0 {
            let _ = ready.send(Err("RegisterClassW failed".to_string()));
            return Err("RegisterClassW failed".to_string());
        }
        let hwnd = CreateWindowExW(
            0,
            class_name.as_ptr(),
            wide("DeskPilot").as_ptr(),
            WS_OVERLAPPED,
            CW_USEDEFAULT,
            CW_USEDEFAULT,
            0,
            0,
            0,
            0,
            module,
            std::ptr::null(),
        );
        if hwnd == 0 {
            let _ = ready.send(Err("CreateWindowExW failed".to_string()));
            return Err("CreateWindowExW failed".to_string());
        }
        if let Some(state) = STATE.get() {
            if let Ok(mut target) = state.hwnd.lock() {
                *target = hwnd;
            }
        }
        add_icon(hwnd);
        let _ = ready.send(Ok(()));
        let mut message: MSG = zeroed();
        while GetMessageW(&mut message, 0, 0, 0) > 0 {
            TranslateMessage(&message);
            DispatchMessageW(&message);
        }
        delete_icon(hwnd);
    }
    Ok(())
}

unsafe extern "system" fn window_proc(
    hwnd: HWND,
    message: u32,
    wparam: WPARAM,
    lparam: LPARAM,
) -> LRESULT {
    if TASKBAR_CREATED
        .get()
        .is_some_and(|registered| message == *registered)
    {
        add_icon(hwnd);
        return 0;
    }

    match message {
        CALLBACK_MESSAGE => {
            let event = lparam as u32 & 0xFFFF;
            match event {
                WM_RBUTTONUP | WM_CONTEXTMENU | NIN_SELECT_EVENT | NIN_KEYSELECT_EVENT => {
                    show_menu(hwnd)
                }
                WM_LBUTTONDBLCLK => send(TrayCommand::ToggleEnabled),
                _ => {}
            }
            0
        }
        WM_COMMAND => {
            match wparam & 0xFFFF {
                CMD_TOGGLE_ENABLED => send(TrayCommand::ToggleEnabled),
                CMD_TOGGLE_DYNAMIC => send(TrayCommand::ToggleDynamic),
                CMD_TOGGLE_DIRECTION => send(TrayCommand::ToggleDirection),
                CMD_TOGGLE_NAVIGATION => send(TrayCommand::ToggleNavigation),
                CMD_RECONCILE => send(TrayCommand::Reconcile),
                CMD_RELOAD => send(TrayCommand::Reload),
                CMD_OPEN_CONFIG => send(TrayCommand::OpenConfig),
                CMD_DIAGNOSTICS => send(TrayCommand::Diagnostics),
                CMD_TOGGLE_STARTUP => send(TrayCommand::ToggleStartup),
                CMD_OPEN_LOGS => send(TrayCommand::OpenLogs),
                CMD_EXIT => send(TrayCommand::Exit),
                _ => {}
            }
            0
        }
        WM_DESTROY => {
            delete_icon(hwnd);
            unsafe { PostQuitMessage(0) };
            0
        }
        _ => unsafe { DefWindowProcW(hwnd, message, wparam, lparam) },
    }
}

fn send(command: TrayCommand) {
    if let Some(sender) = COMMAND_SENDER.get() {
        let _ = sender.send(command);
    }
}

fn show_menu(hwnd: HWND) {
    unsafe {
        let menu = CreatePopupMenu();
        if menu == 0 {
            return;
        }
        let state = STATE.get();
        let enabled = state.is_some_and(|value| value.enabled.load(Ordering::Acquire));
        let dynamic = state.is_some_and(|value| value.dynamic.load(Ordering::Acquire));
        let inverted = state.is_some_and(|value| value.direction.load(Ordering::Acquire) == 1);
        let wrap = state.is_some_and(|value| value.navigation.load(Ordering::Acquire) == 1);
        let startup = state.is_some_and(|value| value.startup.load(Ordering::Acquire));
        let backend_ready = state.is_some_and(|value| value.backend_ready.load(Ordering::Acquire));

        append(menu, CMD_TOGGLE_ENABLED, if enabled {
            "DeskPilot: Enabled"
        } else {
            "DeskPilot: Disabled"
        }, true);
        check(menu, CMD_TOGGLE_ENABLED, enabled);
        append(menu, CMD_TOGGLE_DYNAMIC, if dynamic {
            "Dynamic desktops: Enabled"
        } else {
            "Dynamic desktops: Disabled"
        }, backend_ready);
        check(menu, CMD_TOGGLE_DYNAMIC, dynamic);
        append(menu, CMD_TOGGLE_DIRECTION, if inverted {
            "Direction: Inverted"
        } else {
            "Direction: Normal"
        }, backend_ready);
        append(menu, CMD_TOGGLE_NAVIGATION, if wrap {
            "Navigation: Wrap"
        } else {
            "Navigation: Clamp"
        }, backend_ready);
        AppendMenuW(menu, MF_SEPARATOR, 0, std::ptr::null());
        append(menu, CMD_RECONCILE, "Reconcile now", backend_ready);
        append(menu, CMD_RELOAD, "Reload configuration", true);
        append(menu, CMD_OPEN_CONFIG, "Open configuration", true);
        append(menu, CMD_DIAGNOSTICS, "Diagnostics", true);
        append(menu, CMD_TOGGLE_STARTUP, "Start with Windows", true);
        check(menu, CMD_TOGGLE_STARTUP, startup);
        append(menu, CMD_OPEN_LOGS, "Open logs", true);
        AppendMenuW(menu, MF_SEPARATOR, 0, std::ptr::null());
        append(menu, CMD_EXIT, "Exit", true);

        let mut point: POINT = zeroed();
        GetCursorPos(&mut point);
        SetForegroundWindow(hwnd);
        TrackPopupMenu(
            menu,
            TPM_BOTTOMALIGN | TPM_LEFTALIGN | TPM_RIGHTBUTTON,
            point.x,
            point.y,
            0,
            hwnd,
            std::ptr::null(),
        );
        PostMessageW(hwnd, WM_NULL, 0, 0);
        DestroyMenu(menu);
        focus_icon(hwnd);
    }
}

unsafe fn append(menu: isize, id: usize, label: &str, enabled: bool) {
    let flags = MF_STRING | if enabled { 0 } else { MF_GRAYED };
    unsafe { AppendMenuW(menu, flags, id, wide(label).as_ptr()) };
}

unsafe fn check(menu: isize, id: usize, checked: bool) {
    unsafe {
        CheckMenuItem(
            menu,
            id as u32,
            if checked { MF_CHECKED } else { MF_UNCHECKED },
        )
    };
}

fn add_icon(hwnd: HWND) {
    let (error, enabled) = icon_state();
    let mut data = icon_data(hwnd, error, enabled);
    unsafe {
        if Shell_NotifyIconW(NIM_ADD, &data) != 0 {
            data.Anonymous.uVersion = NOTIFYICON_VERSION_4;
            let _ = Shell_NotifyIconW(NIM_SETVERSION, &data);
        }
    }
}

fn modify_icon(hwnd: HWND, error: bool, enabled: bool) {
    let data = icon_data(hwnd, error, enabled);
    unsafe {
        if Shell_NotifyIconW(NIM_MODIFY, &data) == 0 {
            add_icon(hwnd);
        }
    }
}

fn icon_state() -> (bool, bool) {
    let state = STATE.get();
    (
        state.is_some_and(|value| value.error.load(Ordering::Acquire)),
        state.is_none_or(|value| value.enabled.load(Ordering::Acquire)),
    )
}

fn icon_data(hwnd: HWND, error: bool, enabled: bool) -> NOTIFYICONDATAW {
    unsafe {
        let mut data: NOTIFYICONDATAW = zeroed();
        data.cbSize = size_of::<NOTIFYICONDATAW>() as u32;
        data.hWnd = hwnd;
        data.uID = TRAY_ID;
        data.uFlags = NIF_MESSAGE | NIF_ICON | NIF_TIP;
        data.uCallbackMessage = CALLBACK_MESSAGE;
        data.hIcon = LoadIconW(
            0,
            if error {
                IDI_ERROR
            } else if enabled {
                IDI_APPLICATION
            } else {
                IDI_WARNING
            },
        );
        let tip = wide(if error {
            "DeskPilot — backend unavailable"
        } else if enabled {
            "DeskPilot — enabled"
        } else {
            "DeskPilot — paused"
        });
        for (target, source) in data.szTip.iter_mut().zip(tip) {
            *target = source;
        }
        data
    }
}

fn focus_icon(hwnd: HWND) {
    unsafe {
        let mut data: NOTIFYICONDATAW = zeroed();
        data.cbSize = size_of::<NOTIFYICONDATAW>() as u32;
        data.hWnd = hwnd;
        data.uID = TRAY_ID;
        let _ = Shell_NotifyIconW(NIM_SETFOCUS, &data);
    }
}

fn delete_icon(hwnd: HWND) {
    unsafe {
        let mut data: NOTIFYICONDATAW = zeroed();
        data.cbSize = size_of::<NOTIFYICONDATAW>() as u32;
        data.hWnd = hwnd;
        data.uID = TRAY_ID;
        let _ = Shell_NotifyIconW(NIM_DELETE, &data);
    }
}
