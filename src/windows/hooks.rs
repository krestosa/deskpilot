// File purpose: Implements low-level keyboard and mouse hooks for Win+wheel navigation and Start-menu suppression.
use crate::config::Config;
use crate::wheel::{Step, WheelState};
use std::mem::{size_of, zeroed};
use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use std::sync::mpsc::Sender;
use std::sync::{Arc, Mutex, OnceLock, RwLock};
use std::thread::{self, JoinHandle};
use windows_sys::Win32::Foundation::{LPARAM, LRESULT, WPARAM};
use windows_sys::Win32::System::LibraryLoader::GetModuleHandleW;
use windows_sys::Win32::System::SystemInformation::GetTickCount64;
use windows_sys::Win32::System::Threading::GetCurrentThreadId;
use windows_sys::Win32::UI::Input::KeyboardAndMouse::{
    GetAsyncKeyState, SendInput, INPUT, INPUT_0, INPUT_KEYBOARD, KEYBDINPUT, KEYEVENTF_KEYUP,
    VK_CONTROL, VK_LWIN, VK_RWIN,
};
use windows_sys::Win32::UI::WindowsAndMessaging::{
    CallNextHookEx, DispatchMessageW, GetMessageW, PostThreadMessageW, SetWindowsHookExW,
    TranslateMessage, UnhookWindowsHookEx, HC_ACTION, HHOOK, KBDLLHOOKSTRUCT, MSG, MSLLHOOKSTRUCT,
    WH_KEYBOARD_LL, WH_MOUSE_LL, WM_KEYDOWN, WM_KEYUP, WM_MOUSEHWHEEL, WM_MOUSEWHEEL, WM_QUIT,
    WM_SYSKEYDOWN, WM_SYSKEYUP,
};

const SUPPRESSION_MARKER: usize = 0x4450_5752;

static CONTEXT: OnceLock<Arc<HookContext>> = OnceLock::new();

struct HookContext {
    config: Arc<RwLock<Config>>,
    navigation: Sender<Step>,
    wheel: Mutex<WheelState>,
    enabled: AtomicBool,
    backend_ready: AtomicBool,
    suspended: AtomicBool,
    left_win_down: AtomicBool,
    right_win_down: AtomicBool,
    consumed_win_chord: AtomicBool,
    thread_id: AtomicU32,
}

pub struct HookController {
    context: Arc<HookContext>,
    thread: Option<JoinHandle<Result<(), String>>>,
}

impl HookController {
    // Function purpose: Starts the component and returns the controller used to update or stop it.
    pub fn start(config: Arc<RwLock<Config>>, navigation: Sender<Step>) -> Result<Self, String> {
        let initial_enabled = config.read().map_or(true, |value| value.enabled);
        let context = Arc::new(HookContext {
            config,
            navigation,
            wheel: Mutex::new(WheelState::default()),
            enabled: AtomicBool::new(initial_enabled),
            backend_ready: AtomicBool::new(false),
            suspended: AtomicBool::new(false),
            left_win_down: AtomicBool::new(false),
            right_win_down: AtomicBool::new(false),
            consumed_win_chord: AtomicBool::new(false),
            thread_id: AtomicU32::new(0),
        });
        CONTEXT
            .set(context.clone())
            .map_err(|_| "hook context already initialized".to_string())?;
        let context_for_thread = context.clone();
        let thread = thread::Builder::new()
            .name("deskpilot-input-hook".to_string())
            .spawn(move || run_hook_loop(&context_for_thread))
            .map_err(|error| error.to_string())?;
        Ok(Self {
            context,
            thread: Some(thread),
        })
    }

    // Function purpose: Updates enabled.
    pub fn set_enabled(&self, enabled: bool) {
        self.context.enabled.store(enabled, Ordering::Release);
    }

    // Function purpose: Updates backend ready.
    pub fn set_backend_ready(&self, ready: bool) {
        self.context.backend_ready.store(ready, Ordering::Release);
    }

    // Function purpose: Updates suspended.
    pub fn set_suspended(&self, suspended: bool) {
        self.context.suspended.store(suspended, Ordering::Release);
    }

    // Function purpose: Stops the component, signals its worker thread, and waits for native resources to be released.
    pub fn stop(&mut self) {
        let thread_id = self.context.thread_id.load(Ordering::Acquire);
        if thread_id != 0 {
            unsafe { PostThreadMessageW(thread_id, WM_QUIT, 0, 0) };
        }
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }
}

impl Drop for HookController {
    // Function purpose: Releases the native or background resource owned by this value when it leaves scope.
    fn drop(&mut self) {
        self.stop();
    }
}

// Function purpose: Performs the run hook loop operation required by this module.
fn run_hook_loop(context: &HookContext) -> Result<(), String> {
    unsafe {
        context
            .thread_id
            .store(GetCurrentThreadId(), Ordering::Release);
        let module = GetModuleHandleW(std::ptr::null());
        let keyboard_hook = SetWindowsHookExW(WH_KEYBOARD_LL, Some(keyboard_proc), module, 0);
        if keyboard_hook == 0 {
            return Err("SetWindowsHookExW(WH_KEYBOARD_LL) failed".to_string());
        }
        let mouse_hook = SetWindowsHookExW(WH_MOUSE_LL, Some(mouse_proc), module, 0);
        if mouse_hook == 0 {
            UnhookWindowsHookEx(keyboard_hook);
            return Err("SetWindowsHookExW(WH_MOUSE_LL) failed".to_string());
        }

        let mut message: MSG = zeroed();
        while GetMessageW(&mut message, 0, 0, 0) > 0 {
            TranslateMessage(&message);
            DispatchMessageW(&message);
        }

        UnhookWindowsHookEx(mouse_hook);
        UnhookWindowsHookEx(keyboard_hook);
    }
    Ok(())
}

// Function purpose: Handles low-level keyboard events, tracks physical Windows keys, and suppresses the final Win release after a consumed gesture.
unsafe extern "system" fn keyboard_proc(code: i32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    if code == HC_ACTION as i32 {
        if let Some(context) = CONTEXT.get() {
            let event = unsafe { &*(lparam as *const KBDLLHOOKSTRUCT) };
            if event.dwExtraInfo == SUPPRESSION_MARKER {
                return unsafe { CallNextHookEx(0 as HHOOK, code, wparam, lparam) };
            }

            let message = wparam as u32;
            let is_down = message == WM_KEYDOWN || message == WM_SYSKEYDOWN;
            let is_up = message == WM_KEYUP || message == WM_SYSKEYUP;
            let key = event.vkCode as u16;

            if key == VK_LWIN || key == VK_RWIN {
                let target = if key == VK_LWIN {
                    &context.left_win_down
                } else {
                    &context.right_win_down
                };

                if is_down {
                    target.store(true, Ordering::Release);
                } else if is_up {
                    target.store(false, Ordering::Release);
                    let other_win_down = if key == VK_LWIN {
                        context.right_win_down.load(Ordering::Acquire)
                    } else {
                        context.left_win_down.load(Ordering::Acquire)
                    };

                    if !other_win_down
                        && context.consumed_win_chord.swap(false, Ordering::AcqRel)
                        && send_suppressed_win_release(key)
                    {
                        reset_wheel(context);
                        return 1;
                    }
                }
            }
        }
    }
    unsafe { CallNextHookEx(0 as HHOOK, code, wparam, lparam) }
}

// Function purpose: Consumes every vertical or horizontal scroll message during an active Win gesture and queues vertical navigation steps asynchronously.
unsafe extern "system" fn mouse_proc(code: i32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    let message = wparam as u32;
    if code == HC_ACTION as i32 && is_scroll_message(message) {
        if let Some(context) = CONTEXT.get() {
            let (capture, navigate) = scroll_policy(
                context.enabled.load(Ordering::Acquire),
                context.backend_ready.load(Ordering::Acquire),
                win_pressed(context),
                context.suspended.load(Ordering::Acquire),
            );
            if !capture {
                reset_wheel(context);
                return unsafe { CallNextHookEx(0 as HHOOK, code, wparam, lparam) };
            }

            context.consumed_win_chord.store(true, Ordering::Release);
            if !navigate {
                reset_wheel(context);
                return 1;
            }
            if message == WM_MOUSEWHEEL {
                let event = unsafe { &*(lparam as *const MSLLHOOKSTRUCT) };
                let delta = ((event.mouseData >> 16) as u16) as i16 as i32;
                let config = context.config.read().ok().map(|value| value.wheel.clone());
                if let Some(config) = config {
                    if let Ok(mut wheel) = context.wheel.try_lock() {
                        let gesture = wheel.gesture(
                            true,
                            delta,
                            unsafe { GetTickCount64() },
                            config.threshold,
                            config.cooldown_ms,
                            config.direction,
                        );
                        if let Some(step) = gesture.step {
                            let _ = context.navigation.send(step);
                        }
                    }
                }
            }
            return 1;
        }
    }
    unsafe { CallNextHookEx(0 as HHOOK, code, wparam, lparam) }
}

// Function purpose: Separates unconditional Win-scroll capture from optional navigation suspension so foreground applications never receive the gesture.
fn scroll_policy(
    enabled: bool,
    backend_ready: bool,
    win_pressed: bool,
    suspended: bool,
) -> (bool, bool) {
    let capture = enabled && backend_ready && win_pressed;
    (capture, capture && !suspended)
}

// Function purpose: Identifies all mouse-wheel messages that must be blocked from the application beneath the pointer during a Win gesture.
fn is_scroll_message(message: u32) -> bool {
    message == WM_MOUSEWHEEL || message == WM_MOUSEHWHEEL
}

// Function purpose: Resets wheel.
fn reset_wheel(context: &HookContext) {
    if let Ok(mut wheel) = context.wheel.try_lock() {
        wheel.reset();
    }
}

// Function purpose: Performs the win pressed operation required by this module.
fn win_pressed(context: &HookContext) -> bool {
    context.left_win_down.load(Ordering::Acquire)
        || context.right_win_down.load(Ordering::Acquire)
        || unsafe {
            (GetAsyncKeyState(VK_LWIN as i32) as u16 & 0x8000) != 0
                || (GetAsyncKeyState(VK_RWIN as i32) as u16 & 0x8000) != 0
        }
}

// Function purpose: Sends suppressed win release.
fn send_suppressed_win_release(win_key: u16) -> bool {
    let inputs = suppressed_win_release_inputs(win_key);
    unsafe {
        SendInput(
            inputs.len() as u32,
            inputs.as_ptr(),
            size_of::<INPUT>() as i32,
        ) == inputs.len() as u32
    }
}

// Function purpose: Performs the suppressed win release inputs operation required by this module.
fn suppressed_win_release_inputs(win_key: u16) -> [INPUT; 3] {
    [
        keyboard_input(VK_CONTROL, false),
        keyboard_input(win_key, true),
        keyboard_input(VK_CONTROL, true),
    ]
}

// Function purpose: Performs the keyboard input operation required by this module.
fn keyboard_input(key: u16, key_up: bool) -> INPUT {
    INPUT {
        r#type: INPUT_KEYBOARD,
        Anonymous: INPUT_0 {
            ki: KEYBDINPUT {
                wVk: key,
                wScan: 0,
                dwFlags: if key_up { KEYEVENTF_KEYUP } else { 0 },
                time: 0,
                dwExtraInfo: SUPPRESSION_MARKER,
            },
        },
    }
}

#[cfg(test)]
mod tests {
    use super::{is_scroll_message, scroll_policy, suppressed_win_release_inputs};
    use windows_sys::Win32::UI::Input::KeyboardAndMouse::{KEYEVENTF_KEYUP, VK_CONTROL, VK_LWIN};
    use windows_sys::Win32::UI::WindowsAndMessaging::{WM_MOUSEHWHEEL, WM_MOUSEWHEEL};

    // Function purpose: Verifies fullscreen suspension blocks navigation but still consumes Win-modified scroll before it reaches the foreground application.
    #[test]
    fn fullscreen_suspension_preserves_scroll_capture() {
        assert_eq!(scroll_policy(true, true, true, true), (true, false));
        assert_eq!(scroll_policy(true, true, true, false), (true, true));
        assert_eq!(scroll_policy(true, true, false, false), (false, false));
    }

    // Function purpose: Verifies that both vertical and horizontal wheel messages are classified as scroll that must be captured.
    #[test]
    fn captures_vertical_and_horizontal_scroll_messages() {
        assert!(is_scroll_message(WM_MOUSEWHEEL));
        assert!(is_scroll_message(WM_MOUSEHWHEEL));
        assert!(!is_scroll_message(0));
    }

    // Function purpose: Verifies the start suppression replaces physical win up with control chord scenario and its expected safety or state invariant.
    #[test]
    fn start_suppression_replaces_physical_win_up_with_control_chord() {
        let inputs = suppressed_win_release_inputs(VK_LWIN);
        let control_down = unsafe { inputs[0].Anonymous.ki };
        let win_up = unsafe { inputs[1].Anonymous.ki };
        let control_up = unsafe { inputs[2].Anonymous.ki };

        assert_eq!(control_down.wVk, VK_CONTROL);
        assert_eq!(control_down.dwFlags, 0);
        assert_eq!(win_up.wVk, VK_LWIN);
        assert_eq!(win_up.dwFlags, KEYEVENTF_KEYUP);
        assert_eq!(control_up.wVk, VK_CONTROL);
        assert_eq!(control_up.dwFlags, KEYEVENTF_KEYUP);
        assert_ne!(control_down.dwExtraInfo, 0);
        assert_eq!(control_down.dwExtraInfo, win_up.dwExtraInfo);
        assert_eq!(win_up.dwExtraInfo, control_up.dwExtraInfo);
    }
}
