use std::mem::zeroed;
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::mpsc::Sender;
use std::sync::OnceLock;
use std::thread::{self, JoinHandle};
use windows_sys::Win32::Foundation::HWND;
use windows_sys::Win32::System::Threading::GetCurrentThreadId;
use windows_sys::Win32::UI::Accessibility::{SetWinEventHook, UnhookWinEvent, HWINEVENTHOOK};
use windows_sys::Win32::UI::WindowsAndMessaging::{
    DispatchMessageW, GetMessageW, PostThreadMessageW, TranslateMessage, CHILDID_SELF,
    EVENT_OBJECT_CREATE, EVENT_OBJECT_HIDE, MSG, OBJID_WINDOW, WINEVENT_OUTOFCONTEXT,
    WINEVENT_SKIPOWNPROCESS, WM_QUIT,
};

static EVENT_SENDER: OnceLock<Sender<()>> = OnceLock::new();

pub struct WindowEventController {
    thread_id: AtomicU32,
    thread: Option<JoinHandle<Result<(), String>>>,
}

impl WindowEventController {
    pub fn start(sender: Sender<()>) -> Result<Self, String> {
        EVENT_SENDER
            .set(sender)
            .map_err(|_| "window event sender already initialized".to_string())?;
        let thread_id = AtomicU32::new(0);
        let (ready_tx, ready_rx) = std::sync::mpsc::channel();
        let thread = thread::Builder::new()
            .name("deskpilot-window-events".to_string())
            .spawn(move || run_loop(ready_tx))
            .map_err(|error| error.to_string())?;
        let registered_thread_id = ready_rx.recv().map_err(|error| error.to_string())??;
        thread_id.store(registered_thread_id, Ordering::Release);
        Ok(Self {
            thread_id,
            thread: Some(thread),
        })
    }

    pub fn stop(&mut self) {
        let thread_id = self.thread_id.load(Ordering::Acquire);
        if thread_id != 0 {
            unsafe { PostThreadMessageW(thread_id, WM_QUIT, 0, 0) };
        }
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }
}

impl Drop for WindowEventController {
    fn drop(&mut self) {
        self.stop();
    }
}

fn run_loop(ready: Sender<Result<u32, String>>) -> Result<(), String> {
    unsafe {
        let hook = SetWinEventHook(
            EVENT_OBJECT_CREATE,
            EVENT_OBJECT_HIDE,
            0,
            Some(window_event_proc),
            0,
            0,
            WINEVENT_OUTOFCONTEXT | WINEVENT_SKIPOWNPROCESS,
        );
        if hook == 0 {
            let error = "SetWinEventHook failed".to_string();
            let _ = ready.send(Err(error.clone()));
            return Err(error);
        }
        let thread_id = GetCurrentThreadId();
        let _ = ready.send(Ok(thread_id));
        let mut message: MSG = zeroed();
        while GetMessageW(&mut message, 0, 0, 0) > 0 {
            TranslateMessage(&message);
            DispatchMessageW(&message);
        }
        UnhookWinEvent(hook);
    }
    Ok(())
}

unsafe extern "system" fn window_event_proc(
    _hook: HWINEVENTHOOK,
    _event: u32,
    hwnd: HWND,
    object_id: i32,
    child_id: i32,
    _event_thread: u32,
    _event_time: u32,
) {
    if hwnd != 0 && object_id == OBJID_WINDOW && child_id == CHILDID_SELF as i32 {
        if let Some(sender) = EVENT_SENDER.get() {
            let _ = sender.send(());
        }
    }
}
