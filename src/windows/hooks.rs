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
    VK_F24, VK_LWIN, VK_RWIN,
};
use windows_sys::Win32::UI::WindowsAndMessaging::{
    CallNextHookEx, DispatchMessageW, GetMessageW, PostThreadMessageW, SetWindowsHookExW,
    TranslateMessage, UnhookWindowsHookEx, HC_ACTION, HHOOK, KBDLLHOOKSTRUCT, MSG, MSLLHOOKSTRUCT,
    WH_KEYBOARD_LL, WH_MOUSE_LL, WM_APP, WM_KEYDOWN, WM_KEYUP, WM_MOUSEWHEEL, WM_QUIT,
    WM_SYSKEYDOWN, WM_SYSKEYUP,
};

const WM_RELEASE_START_SUPPRESSOR: u32 = WM_APP + 0x51;

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
    suppressor_down: AtomicBool,
    suppressor_release_queued: AtomicBool,
    thread_id: AtomicU32,
}

pub struct HookController {
    context: Arc<HookContext>,
    thread: Option<JoinHandle<Result<(), String>>>,
}

impl HookController {
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
            suppressor_down: AtomicBool::new(false),
            suppressor_release_queued: AtomicBool::new(false),
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

    pub fn set_enabled(&self, enabled: bool) {
        self.context.enabled.store(enabled, Ordering::Release);
    }

    pub fn set_backend_ready(&self, ready: bool) {
        self.context.backend_ready.store(ready, Ordering::Release);
    }

    pub fn set_suspended(&self, suspended: bool) {
        self.context.suspended.store(suspended, Ordering::Release);
    }

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
    fn drop(&mut self) {
        self.stop();
    }
}

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
            if message.message == WM_RELEASE_START_SUPPRESSOR {
                release_start_suppression(context);
                continue;
            }
            TranslateMessage(&message);
            DispatchMessageW(&message);
        }

        release_start_suppression(context);
        UnhookWindowsHookEx(mouse_hook);
        UnhookWindowsHookEx(keyboard_hook);
    }
    Ok(())
}

unsafe extern "system" fn keyboard_proc(code: i32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    if code == HC_ACTION as i32 {
        if let Some(context) = CONTEXT.get() {
            let event = unsafe { &*(lparam as *const KBDLLHOOKSTRUCT) };
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
                    queue_suppressor_release_after_win_up(context);
                }
            }
        }
    }
    unsafe { CallNextHookEx(0 as HHOOK, code, wparam, lparam) }
}

unsafe extern "system" fn mouse_proc(code: i32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    if code == HC_ACTION as i32 && wparam as u32 == WM_MOUSEWHEEL {
        if let Some(context) = CONTEXT.get() {
            let active = context.enabled.load(Ordering::Acquire)
                && context.backend_ready.load(Ordering::Acquire)
                && !context.suspended.load(Ordering::Acquire);
            if !active || !win_pressed(context) {
                reset_wheel(context);
                return unsafe { CallNextHookEx(0 as HHOOK, code, wparam, lparam) };
            }

            let event = unsafe { &*(lparam as *const MSLLHOOKSTRUCT) };
            let delta = ((event.mouseData >> 16) as u16) as i16 as i32;
            let config = context.config.read().ok().map(|value| value.wheel.clone());
            if let Some(config) = config {
                if let Ok(mut wheel) = context.wheel.try_lock() {
                    if let Some(step) = wheel.feed(
                        delta,
                        unsafe { GetTickCount64() },
                        config.threshold,
                        config.cooldown_ms,
                        config.direction,
                    ) {
                        if context.navigation.send(step).is_ok() {
                            arm_start_suppression(context);
                            return 1;
                        }
                    }
                }
            }
        }
    }
    unsafe { CallNextHookEx(0 as HHOOK, code, wparam, lparam) }
}

fn reset_wheel(context: &HookContext) {
    if let Ok(mut wheel) = context.wheel.try_lock() {
        wheel.reset();
    }
}

fn win_pressed(context: &HookContext) -> bool {
    context.left_win_down.load(Ordering::Acquire)
        || context.right_win_down.load(Ordering::Acquire)
        || unsafe {
            (GetAsyncKeyState(VK_LWIN as i32) as u16 & 0x8000) != 0
                || (GetAsyncKeyState(VK_RWIN as i32) as u16 & 0x8000) != 0
        }
}

fn queue_suppressor_release_after_win_up(context: &HookContext) {
    if context.left_win_down.load(Ordering::Acquire)
        || context.right_win_down.load(Ordering::Acquire)
        || !context.suppressor_down.load(Ordering::Acquire)
        || context
            .suppressor_release_queued
            .swap(true, Ordering::AcqRel)
    {
        return;
    }

    let thread_id = context.thread_id.load(Ordering::Acquire);
    if thread_id == 0
        || unsafe { PostThreadMessageW(thread_id, WM_RELEASE_START_SUPPRESSOR, 0, 0) } == 0
    {
        context
            .suppressor_release_queued
            .store(false, Ordering::Release);
    }
}

fn arm_start_suppression(context: &HookContext) {
    if context.suppressor_down.swap(true, Ordering::AcqRel) {
        return;
    }

    let input = suppressor_input(false);
    let sent = unsafe { SendInput(1, &input, size_of::<INPUT>() as i32) };
    if sent != 1 {
        context.suppressor_down.store(false, Ordering::Release);
    }
}

fn release_start_suppression(context: &HookContext) {
    context
        .suppressor_release_queued
        .store(false, Ordering::Release);
    if !context.suppressor_down.swap(false, Ordering::AcqRel) {
        return;
    }

    let input = suppressor_input(true);
    unsafe {
        let _ = SendInput(1, &input, size_of::<INPUT>() as i32);
    }
}

fn suppressor_input(key_up: bool) -> INPUT {
    INPUT {
        r#type: INPUT_KEYBOARD,
        Anonymous: INPUT_0 {
            ki: KEYBDINPUT {
                wVk: VK_F24,
                wScan: 0,
                dwFlags: if key_up { KEYEVENTF_KEYUP } else { 0 },
                time: 0,
                dwExtraInfo: 0,
            },
        },
    }
}

#[cfg(test)]
mod tests {
    use super::suppressor_input;
    use windows_sys::Win32::UI::Input::KeyboardAndMouse::{KEYEVENTF_KEYUP, VK_F24};

    #[test]
    fn start_suppression_holds_neutral_key_until_win_release() {
        let down = unsafe { suppressor_input(false).Anonymous.ki };
        let up = unsafe { suppressor_input(true).Anonymous.ki };
        assert_eq!(down.wVk, VK_F24);
        assert_eq!(down.dwFlags, 0);
        assert_eq!(up.wVk, VK_F24);
        assert_eq!(up.dwFlags, KEYEVENTF_KEYUP);
    }
}
