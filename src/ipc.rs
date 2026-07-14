use crate::event::EventBus;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::mem::size_of;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{self, Sender};
use std::sync::Arc;
use std::thread::{self, JoinHandle};
use std::time::Duration;
use windows_sys::Win32::Foundation::{
    CloseHandle, GetLastError, LocalFree, ERROR_BROKEN_PIPE, ERROR_PIPE_CONNECTED, GENERIC_READ,
    GENERIC_WRITE, HANDLE, INVALID_HANDLE_VALUE,
};
use windows_sys::Win32::Security::Authorization::ConvertStringSecurityDescriptorToSecurityDescriptorW;
use windows_sys::Win32::Security::SECURITY_ATTRIBUTES;
use windows_sys::Win32::Storage::FileSystem::{
    CreateFileW, FlushFileBuffers, ReadFile, WriteFile, FILE_ATTRIBUTE_NORMAL, OPEN_EXISTING,
    PIPE_ACCESS_DUPLEX,
};
use windows_sys::Win32::System::Pipes::{
    ConnectNamedPipe, CreateNamedPipeW, DisconnectNamedPipe, SetNamedPipeHandleState,
    WaitNamedPipeW, PIPE_READMODE_MESSAGE, PIPE_TYPE_MESSAGE, PIPE_WAIT,
};

use crate::windows::system::current_user_sid;
use crate::windows::util::wide;

const MAX_MESSAGE: usize = 64 * 1024;
const IPC_TIMEOUT_MS: u32 = 5_000;
const SDDL_REVISION_1: u32 = 1;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct IpcRequest {
    pub command: String,
    #[serde(default)]
    pub json: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct IpcResponse {
    pub ok: bool,
    pub code: i32,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub data: Option<Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

impl IpcResponse {
    pub fn success(data: impl Serialize) -> Self {
        match serde_json::to_value(data) {
            Ok(value) => Self {
                ok: true,
                code: 0,
                data: Some(value),
                error: None,
            },
            Err(error) => Self::failure(70, error.to_string()),
        }
    }

    pub fn message(message: impl Into<String>) -> Self {
        Self::success(serde_json::json!({ "message": message.into() }))
    }

    pub fn failure(code: i32, error: impl Into<String>) -> Self {
        Self {
            ok: false,
            code,
            data: None,
            error: Some(error.into()),
        }
    }
}

#[derive(Debug)]
pub struct ServerRequest {
    pub request: IpcRequest,
    pub response: Sender<IpcResponse>,
}

pub struct IpcServer {
    stop: Arc<AtomicBool>,
    thread: Option<JoinHandle<Result<(), String>>>,
    pipe_name: String,
}

impl IpcServer {
    pub fn start(dispatch: Sender<ServerRequest>, events: Arc<EventBus>) -> Result<Self, String> {
        let pipe_name = pipe_name()?;
        let stop = Arc::new(AtomicBool::new(false));
        let stop_thread = stop.clone();
        let pipe_thread = pipe_name.clone();
        let thread = thread::Builder::new()
            .name("deskpilot-ipc".to_string())
            .spawn(move || server_loop(&pipe_thread, &dispatch, &events, &stop_thread))
            .map_err(|error| error.to_string())?;
        Ok(Self {
            stop,
            thread: Some(thread),
            pipe_name,
        })
    }

    pub fn pipe_name(&self) -> &str {
        &self.pipe_name
    }

    pub fn stop(&mut self) {
        self.stop.store(true, Ordering::Release);
        let _ = send_request(&IpcRequest {
            command: "__wake".to_string(),
            json: true,
        });
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }
}

impl Drop for IpcServer {
    fn drop(&mut self) {
        self.stop();
    }
}

pub fn send_request(request: &IpcRequest) -> Result<IpcResponse, String> {
    let name = pipe_name()?;
    let name_wide = wide(&name);
    unsafe {
        if WaitNamedPipeW(name_wide.as_ptr(), IPC_TIMEOUT_MS) == 0 {
            return Err("DeskPilot is not running or the IPC timeout expired".to_string());
        }
        let handle = CreateFileW(
            name_wide.as_ptr(),
            GENERIC_READ | GENERIC_WRITE,
            0,
            std::ptr::null(),
            OPEN_EXISTING,
            FILE_ATTRIBUTE_NORMAL,
            0,
        );
        if handle == INVALID_HANDLE_VALUE {
            return Err(format!("opening DeskPilot IPC failed: {}", GetLastError()));
        }
        let mut mode = PIPE_READMODE_MESSAGE;
        let _ = SetNamedPipeHandleState(handle, &mut mode, std::ptr::null(), std::ptr::null());
        let payload = serde_json::to_vec(request).map_err(|error| error.to_string())?;
        write_message(handle, &payload)?;
        let response = read_message(handle)?;
        CloseHandle(handle);
        serde_json::from_slice(&response).map_err(|error| error.to_string())
    }
}

pub fn stream_events() -> Result<(), String> {
    let name = pipe_name()?;
    let name_wide = wide(&name);
    unsafe {
        if WaitNamedPipeW(name_wide.as_ptr(), IPC_TIMEOUT_MS) == 0 {
            return Err("DeskPilot is not running or the IPC timeout expired".to_string());
        }
        let handle = CreateFileW(
            name_wide.as_ptr(),
            GENERIC_READ | GENERIC_WRITE,
            0,
            std::ptr::null(),
            OPEN_EXISTING,
            FILE_ATTRIBUTE_NORMAL,
            0,
        );
        if handle == INVALID_HANDLE_VALUE {
            return Err("opening DeskPilot IPC failed".to_string());
        }
        let request = serde_json::to_vec(&IpcRequest {
            command: "events".to_string(),
            json: true,
        })
        .map_err(|error| error.to_string())?;
        write_message(handle, &request)?;
        loop {
            match read_message(handle) {
                Ok(message) => println!("{}", String::from_utf8_lossy(&message)),
                Err(error) if error.contains("broken pipe") => break,
                Err(error) => {
                    CloseHandle(handle);
                    return Err(error);
                }
            }
        }
        CloseHandle(handle);
    }
    Ok(())
}

fn server_loop(
    pipe_name: &str,
    dispatch: &Sender<ServerRequest>,
    events: &Arc<EventBus>,
    stop: &AtomicBool,
) -> Result<(), String> {
    while !stop.load(Ordering::Acquire) {
        let pipe = create_server_pipe(pipe_name)?;
        let connected = unsafe { ConnectNamedPipe(pipe, std::ptr::null_mut()) };
        if connected == 0 && unsafe { GetLastError() } != ERROR_PIPE_CONNECTED {
            unsafe { CloseHandle(pipe) };
            if stop.load(Ordering::Acquire) {
                break;
            }
            continue;
        }
        if stop.load(Ordering::Acquire) {
            unsafe {
                DisconnectNamedPipe(pipe);
                CloseHandle(pipe);
            }
            break;
        }
        let dispatch = dispatch.clone();
        let events = events.clone();
        thread::spawn(move || {
            let _ = handle_client(pipe, dispatch, events);
            unsafe {
                FlushFileBuffers(pipe);
                DisconnectNamedPipe(pipe);
                CloseHandle(pipe);
            }
        });
    }
    Ok(())
}

fn handle_client(
    pipe: HANDLE,
    dispatch: Sender<ServerRequest>,
    events: Arc<EventBus>,
) -> Result<(), String> {
    let payload = unsafe { read_message(pipe)? };
    let request: IpcRequest =
        serde_json::from_slice(&payload).map_err(|error| error.to_string())?;
    if request.command == "events" {
        let receiver = events.subscribe();
        for event in receiver {
            let payload = serde_json::to_vec(&event).map_err(|error| error.to_string())?;
            if unsafe { write_message(pipe, &payload) }.is_err() {
                break;
            }
        }
        return Ok(());
    }
    let (response_tx, response_rx) = mpsc::channel();
    dispatch
        .send(ServerRequest {
            request,
            response: response_tx,
        })
        .map_err(|error| error.to_string())?;
    let response = response_rx
        .recv_timeout(Duration::from_secs(10))
        .map_err(|error| error.to_string())?;
    let payload = serde_json::to_vec(&response).map_err(|error| error.to_string())?;
    unsafe { write_message(pipe, &payload) }
}

fn create_server_pipe(name: &str) -> Result<HANDLE, String> {
    let sid = current_user_sid()?;
    let sddl = wide(format!("D:P(A;;GA;;;SY)(A;;GA;;;{sid})"));
    let mut descriptor = std::ptr::null_mut();
    unsafe {
        if ConvertStringSecurityDescriptorToSecurityDescriptorW(
            sddl.as_ptr(),
            SDDL_REVISION_1,
            &mut descriptor,
            std::ptr::null_mut(),
        ) == 0
        {
            return Err(format!(
                "creating IPC security descriptor failed: {}",
                GetLastError()
            ));
        }
        let mut attributes = SECURITY_ATTRIBUTES {
            nLength: size_of::<SECURITY_ATTRIBUTES>() as u32,
            lpSecurityDescriptor: descriptor,
            bInheritHandle: 0,
        };
        let name_wide = wide(name);
        let pipe = CreateNamedPipeW(
            name_wide.as_ptr(),
            PIPE_ACCESS_DUPLEX,
            PIPE_TYPE_MESSAGE | PIPE_READMODE_MESSAGE | PIPE_WAIT,
            4,
            MAX_MESSAGE as u32,
            MAX_MESSAGE as u32,
            IPC_TIMEOUT_MS,
            &mut attributes,
        );
        LocalFree(descriptor);
        if pipe == INVALID_HANDLE_VALUE {
            return Err(format!("CreateNamedPipeW failed: {}", GetLastError()));
        }
        Ok(pipe)
    }
}

fn pipe_name() -> Result<String, String> {
    let sid = current_user_sid()?;
    Ok(format!(r"\\.\pipe\DeskPilot-{sid}"))
}

unsafe fn write_message(handle: HANDLE, data: &[u8]) -> Result<(), String> {
    if data.len() > MAX_MESSAGE {
        return Err("IPC message exceeds 64 KiB".to_string());
    }
    let mut written = 0;
    if unsafe {
        WriteFile(
            handle,
            data.as_ptr(),
            data.len() as u32,
            &mut written,
            std::ptr::null_mut(),
        )
    } == 0
        || written != data.len() as u32
    {
        return Err(format!("IPC write failed: {}", unsafe { GetLastError() }));
    }
    Ok(())
}

unsafe fn read_message(handle: HANDLE) -> Result<Vec<u8>, String> {
    let mut buffer = vec![0_u8; MAX_MESSAGE];
    let mut read = 0;
    if unsafe {
        ReadFile(
            handle,
            buffer.as_mut_ptr(),
            buffer.len() as u32,
            &mut read,
            std::ptr::null_mut(),
        )
    } == 0
    {
        let error = unsafe { GetLastError() };
        if error == ERROR_BROKEN_PIPE {
            return Err("IPC broken pipe".to_string());
        }
        return Err(format!("IPC read failed: {error}"));
    }
    buffer.truncate(read as usize);
    Ok(buffer)
}
