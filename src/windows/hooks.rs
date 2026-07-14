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
    WH_MOUSE_LL, WM_MOUSEWHEEL, WM_QUIT,
};

static CONTEXT: OnceLock<Arc<HookContext>> = OnceLock::new();

struct HookContext {
    config: Arc<RwLock<Config>>,
    navigation: Sender<Step>,
    wheel: Mutex<WheelState>,
    enabled: AtomicBool,
    backend_ready: AtomicBool,
    suspended: AtomicBool,
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
        let hook = SetWindowsHookExW(WH_MOUSE_LL, Some(mouse_proc), module, 0);
        if hook == 0 {
            return Err("SetWindowsHookExW(WH_MOUSE_LL) failed".to_string());
        }
        let mut message: MSG = zeroed();
        while GetMessageW(&mut message, 0, 0, 0) > 0 {
            TranslateMessage(&message);
            DispatchMessageW(&message);
        }
        UnhookWindowsHookEx(hook);
    }
    Ok(())
}

unsafe extern "system" fn mouse_proc(code: i32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    if code == HC_ACTION as i32 && wparam as u32 == WM_MOUSEWHEEL {
        if let Some(context) = CONTEXT.get() {
            let active = context.enabled.load(Ordering::Acquire)
                && context.backend_ready.load(Ordering::Acquire)
                && !context.suspended.load(Ordering::Acquire);
            if !active || !win_pressed() {
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
                        suppress_start_menu();
                        if context.navigation.send(step).is_ok() {
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

fn win_pressed() -> bool {
    unsafe {
        (GetAsyncKeyState(VK_LWIN as i32) as u16 & 0x8000) != 0
            || (GetAsyncKeyState(VK_RWIN as i32) as u16 & 0x8000) != 0
    }
}

fn suppress_start_menu() {
    let inputs = start_suppression_inputs();
    unsafe {
        let _ = SendInput(
            inputs.len() as u32,
            inputs.as_ptr(),
            size_of::<INPUT>() as i32,
        );
    }
}

fn start_suppression_inputs() -> [INPUT; 2] {
    let down = INPUT {
        r#type: INPUT_KEYBOARD,
        Anonymous: INPUT_0 {
            ki: KEYBDINPUT {
                wVk: VK_CONTROL,
                wScan: 0,
                dwFlags: 0,
                time: 0,
                dwExtraInfo: 0,
            },
        },
    };
    let up = INPUT {
        r#type: INPUT_KEYBOARD,
        Anonymous: INPUT_0 {
            ki: KEYBDINPUT {
                wVk: VK_CONTROL,
                wScan: 0,
                dwFlags: KEYEVENTF_KEYUP,
                time: 0,
                dwExtraInfo: 0,
            },
        },
    };
    [down, up]
}

#[allow(dead_code)]
fn _assert_structs() {
    let _: Option<KBDLLHOOKSTRUCT> = None;
}

#[cfg(test)]
mod tests {
    use super::start_suppression_inputs;
    use windows_sys::Win32::UI::Input::KeyboardAndMouse::{VK_CONTROL, KEYEVENTF_KEYUP};

    #[test]
    fn start_suppression_emits_complete_control_chord() {
        let inputs = start_suppression_inputs();
        let down = unsafe { inputs[0].Anonymous.ki };
        let up = unsafe { inputs[1].Anonymous.ki };
        assert_eq!(down.wVk, VK_CONTROL);
        assert_eq!(down.dwFlags, 0);
        assert_eq!(up.wVk, VK_CONTROL);
        assert_eq!(up.dwFlags, KEYEVENTF_KEYUP);
    }
}
