// File purpose: Listens for native top-level window lifecycle events and reports stable window tokens for event-confirmed occupancy.
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
    EVENT_OBJECT_CREATE, EVENT_OBJECT_HIDE, EVENT_OBJECT_SHOW, MSG, OBJID_WINDOW,
    WINEVENT_OUTOFCONTEXT, WINEVENT_SKIPOWNPROCESS, WM_QUIT,
};

use crate::reconciliation::WindowToken;

static EVENT_SENDER: OnceLock<Sender<WindowEvent>> = OnceLock::new();

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct WindowEvent {
    token: WindowToken,
    event: u32,
}

impl WindowEvent {
    // Function purpose: Returns the stable native token used to correlate this event with a later desktop inventory snapshot.
    pub fn token(self) -> WindowToken {
        self.token
    }

    // Function purpose: Reports whether the event can represent a newly opened or newly shown user application window.
    pub fn occupancy_gain(self) -> bool {
        self.event == EVENT_OBJECT_CREATE || self.event == EVENT_OBJECT_SHOW
    }
}

pub struct WindowEventController {
    thread_id: AtomicU32,
    thread: Option<JoinHandle<Result<(), String>>>,
}

impl WindowEventController {
    // Function purpose: Starts the component and returns the controller used to update or stop it.
    pub fn start(sender: Sender<WindowEvent>) -> Result<Self, String> {
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

    // Function purpose: Stops the component, signals its worker thread, and waits for native resources to be released.
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
    // Function purpose: Releases the native or background resource owned by this value when it leaves scope.
    fn drop(&mut self) {
        self.stop();
    }
}

// Function purpose: Performs the run loop operation required by this module.
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

// Function purpose: Filters native top-level window lifecycle callbacks and forwards the event plus a stable window token to the main loop.
unsafe extern "system" fn window_event_proc(
    _hook: HWINEVENTHOOK,
    event: u32,
    hwnd: HWND,
    object_id: i32,
    child_id: i32,
    _event_thread: u32,
    _event_time: u32,
) {
    if hwnd != 0 && object_id == OBJID_WINDOW && child_id == CHILDID_SELF as i32 {
        if let Some(sender) = EVENT_SENDER.get() {
            let _ = sender.send(WindowEvent {
                token: hwnd as usize as WindowToken,
                event,
            });
        }
    }
}

#[cfg(test)]
mod tests {
    use super::WindowEvent;
    use windows_sys::Win32::UI::WindowsAndMessaging::{
        EVENT_OBJECT_CREATE, EVENT_OBJECT_HIDE, EVENT_OBJECT_SHOW,
    };

    // Function purpose: Verifies only create and show events can authorize consumption of the protected spare.
    #[test]
    fn occupancy_gain_requires_create_or_show() {
        assert!(WindowEvent {
            token: 1,
            event: EVENT_OBJECT_CREATE,
        }
        .occupancy_gain());
        assert!(WindowEvent {
            token: 2,
            event: EVENT_OBJECT_SHOW,
        }
        .occupancy_gain());
        assert!(!WindowEvent {
            token: 3,
            event: EVENT_OBJECT_HIDE,
        }
        .occupancy_gain());
    }
}
