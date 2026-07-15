// File purpose: Implements the session-isolated named-pipe protocol, server, client requests, and bounded event streaming.
use crate::event::EventBus;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::mem::size_of;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
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
    CreateFileW, FlushFileBuffers, ReadFile, WriteFile, FILE_ATTRIBUTE_NORMAL,
    FILE_FLAG_FIRST_PIPE_INSTANCE, OPEN_EXISTING, PIPE_ACCESS_DUPLEX,
};
use windows_sys::Win32::System::Pipes::{
    ConnectNamedPipe, CreateNamedPipeW, DisconnectNamedPipe, SetNamedPipeHandleState,
    WaitNamedPipeW, PIPE_READMODE_MESSAGE, PIPE_REJECT_REMOTE_CLIENTS, PIPE_TYPE_MESSAGE,
    PIPE_UNLIMITED_INSTANCES, PIPE_WAIT,
};

use crate::windows::system::{current_session_id, current_user_sid};
use crate::windows::util::wide;

const MAX_MESSAGE: usize = 64 * 1024;
const IPC_TIMEOUT_MS: u32 = 5_000;
const SDDL_REVISION_1: u32 = 1;
const MAX_ACTIVE_CLIENTS: usize = 16;

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
    // Function purpose: Starts the first protected pipe instance and waits until its namespace is reserved.
    pub fn start(dispatch: Sender<ServerRequest>, events: Arc<EventBus>) -> Result<Self, String> {
        let pipe_name = pipe_name()?;
        let stop = Arc::new(AtomicBool::new(false));
        let active_clients = Arc::new(AtomicUsize::new(0));
        let stop_thread = stop.clone();
        let active_thread = active_clients.clone();
        let pipe_thread = pipe_name.clone();
        let (ready_tx, ready_rx) = mpsc::channel();
        let thread = thread::Builder::new()
            .name("deskpilot-ipc".to_string())
            .spawn(move || {
                server_loop(
                    &pipe_thread,
                    &dispatch,
                    &events,
                    &stop_thread,
                    &active_thread,
                    ready_tx,
                )
            })
            .map_err(|error| error.to_string())?;
        ready_rx.recv().map_err(|error| error.to_string())??;
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

#[derive(Debug)]
struct OwnedHandle(HANDLE);

impl OwnedHandle {
    fn new(handle: HANDLE) -> Result<Self, String> {
        if handle == INVALID_HANDLE_VALUE || handle == 0 {
            Err(format!("invalid Windows handle: {}", unsafe {
                GetLastError()
            }))
        } else {
            Ok(Self(handle))
        }
    }

    fn raw(&self) -> HANDLE {
        self.0
    }
}

impl Drop for OwnedHandle {
    fn drop(&mut self) {
        unsafe {
            CloseHandle(self.0);
        }
    }
}

pub fn send_request(request: &IpcRequest) -> Result<IpcResponse, String> {
    let name = pipe_name()?;
    let handle = open_client_pipe(&name)?;
    let payload = serde_json::to_vec(request).map_err(|error| error.to_string())?;
    unsafe {
        write_message(handle.raw(), &payload)?;
        let response = read_message(handle.raw())?;
        serde_json::from_slice(&response).map_err(|error| error.to_string())
    }
}

pub fn stream_events() -> Result<(), String> {
    let name = pipe_name()?;
    let handle = open_client_pipe(&name)?;
    let request = serde_json::to_vec(&IpcRequest {
        command: "events".to_string(),
        json: true,
    })
    .map_err(|error| error.to_string())?;
    unsafe {
        write_message(handle.raw(), &request)?;
        loop {
            match read_message(handle.raw()) {
                Ok(message) => println!("{}", String::from_utf8_lossy(&message)),
                Err(error) if error.contains("broken pipe") => break,
                Err(error) => return Err(error),
            }
        }
    }
    Ok(())
}

fn open_client_pipe(name: &str) -> Result<OwnedHandle, String> {
    let name_wide = wide(name);
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
        let handle = OwnedHandle::new(handle)
            .map_err(|_| format!("opening DeskPilot IPC failed: {}", GetLastError()))?;
        let mode = PIPE_READMODE_MESSAGE;
        if SetNamedPipeHandleState(handle.raw(), &mode, std::ptr::null(), std::ptr::null()) == 0 {
            return Err(format!(
                "configuring DeskPilot IPC failed: {}",
                GetLastError()
            ));
        }
        Ok(handle)
    }
}

fn server_loop(
    pipe_name: &str,
    dispatch: &Sender<ServerRequest>,
    events: &Arc<EventBus>,
    stop: &AtomicBool,
    active_clients: &Arc<AtomicUsize>,
    ready: Sender<Result<(), String>>,
) -> Result<(), String> {
    let mut first_instance = true;
    let mut readiness = Some(ready);
    while !stop.load(Ordering::Acquire) {
        let pipe = match create_server_pipe(pipe_name, first_instance) {
            Ok(pipe) => pipe,
            Err(error) if first_instance => {
                if let Some(ready) = readiness.take() {
                    let _ = ready.send(Err(error.clone()));
                }
                return Err(error);
            }
            Err(_) => {
                thread::sleep(Duration::from_millis(50));
                continue;
            }
        };
        if first_instance {
            first_instance = false;
            if let Some(ready) = readiness.take() {
                let _ = ready.send(Ok(()));
            }
        }

        let connected = unsafe { ConnectNamedPipe(pipe.raw(), std::ptr::null_mut()) };
        if connected == 0 && unsafe { GetLastError() } != ERROR_PIPE_CONNECTED {
            if stop.load(Ordering::Acquire) {
                break;
            }
            continue;
        }
        if stop.load(Ordering::Acquire) {
            unsafe {
                DisconnectNamedPipe(pipe.raw());
            }
            break;
        }

        let active_before = active_clients.fetch_add(1, Ordering::AcqRel);
        if active_before >= MAX_ACTIVE_CLIENTS {
            active_clients.fetch_sub(1, Ordering::AcqRel);
            let payload = serde_json::to_vec(&IpcResponse::failure(
                69,
                "IPC client limit reached; close an existing event stream and retry",
            ))
            .unwrap_or_default();
            unsafe {
                let _ = write_message(pipe.raw(), &payload);
                let _ = FlushFileBuffers(pipe.raw());
                DisconnectNamedPipe(pipe.raw());
            }
            continue;
        }

        let dispatch = dispatch.clone();
        let events = events.clone();
        let active_clients = active_clients.clone();
        thread::spawn(move || {
            let _ = handle_client(pipe.raw(), dispatch, events);
            unsafe {
                let _ = FlushFileBuffers(pipe.raw());
                DisconnectNamedPipe(pipe.raw());
            }
            active_clients.fetch_sub(1, Ordering::AcqRel);
            drop(pipe);
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
        let receiver = events.subscribe()?;
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

fn create_server_pipe(name: &str, first_instance: bool) -> Result<OwnedHandle, String> {
    let sid = current_user_sid()?;
    let sddl = wide(format!("D:P(A;;GRGW;;;SY)(A;;GRGW;;;{sid})"));
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
        let attributes = SECURITY_ATTRIBUTES {
            nLength: size_of::<SECURITY_ATTRIBUTES>() as u32,
            lpSecurityDescriptor: descriptor,
            bInheritHandle: 0,
        };
        let name_wide = wide(name);
        let open_mode = PIPE_ACCESS_DUPLEX
            | if first_instance {
                FILE_FLAG_FIRST_PIPE_INSTANCE
            } else {
                0
            };
        let pipe = CreateNamedPipeW(
            name_wide.as_ptr(),
            open_mode,
            PIPE_TYPE_MESSAGE | PIPE_READMODE_MESSAGE | PIPE_WAIT | PIPE_REJECT_REMOTE_CLIENTS,
            PIPE_UNLIMITED_INSTANCES,
            MAX_MESSAGE as u32,
            MAX_MESSAGE as u32,
            IPC_TIMEOUT_MS,
            &attributes,
        );
        LocalFree(descriptor);
        OwnedHandle::new(pipe).map_err(|_| format!("CreateNamedPipeW failed: {}", GetLastError()))
    }
}

fn pipe_name() -> Result<String, String> {
    let sid = current_user_sid()?;
    let session = current_session_id()?;
    Ok(format!(r"\\.\pipe\DeskPilot-{sid}-session-{session}"))
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

#[cfg(test)]
mod tests {
    use super::{IpcRequest, MAX_MESSAGE};

    #[test]
    fn request_payload_is_bounded_well_below_protocol_limit() {
        let payload = serde_json::to_vec(&IpcRequest {
            command: "status".to_string(),
            json: true,
        })
        .expect("request serialization");
        assert!(payload.len() < MAX_MESSAGE);
    }
}
