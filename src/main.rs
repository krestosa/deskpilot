#![windows_subsystem = "windows"]

use deskpilot::cli::{Command, Invocation, HELP};
use deskpilot::config::Config;
use deskpilot::ipc::{send_request, stream_events, IpcRequest};
use deskpilot::reconciliation::{plan, DesktopId, DesktopState, Occupancy};
use deskpilot::{APP_VERSION, CONFIG_FILE_NAME};
use std::path::{Path, PathBuf};
use windows_sys::Win32::System::Console::{AttachConsole, SetConsoleOutputCP, ATTACH_PARENT_PROCESS};

fn main() {
    let invocation = match Invocation::parse(std::env::args()) {
        Ok(invocation) => invocation,
        Err(error) => {
            attach_console();
            eprintln!("DeskPilot: {error}\n\n{HELP}");
            std::process::exit(64);
        }
    };
    if invocation.needs_console() { attach_console(); }
    let data_dir = resolve_data_dir(invocation.data_dir.as_deref());
    let code = execute(invocation, data_dir);
    std::process::exit(code);
}

fn execute(invocation: Invocation, data_dir: PathBuf) -> i32 {
    match invocation.command {
        Command::Run(options) => deskpilot::app::run(data_dir, options).map_or_else(|error| fail(69, &error, invocation.json), |()| 0),
        Command::Version => { println!("DeskPilot {APP_VERSION}"); 0 }
        Command::Help => { print!("{HELP}"); 0 }
        Command::ConfigPath => { println!("{}", data_dir.join(CONFIG_FILE_NAME).display()); 0 }
        Command::ConfigShow => match std::fs::read_to_string(data_dir.join(CONFIG_FILE_NAME)) {
            Ok(text) => { print!("{text}"); 0 }
            Err(error) => fail(74, &error.to_string(), invocation.json),
        },
        Command::ConfigValidate(path) => {
            let path = path.unwrap_or_else(|| data_dir.join(CONFIG_FILE_NAME));
            match Config::load(&path) {
                Ok(_) => { if invocation.json { println!("{{\"valid\":true,\"path\":{}}}", serde_json::to_string(&path).unwrap_or_else(|_| "null".to_string())); } else { println!("valid: {}", path.display()); } 0 }
                Err(error) => fail(78, &error.to_string(), invocation.json),
            }
        }
        Command::SelfTest { backend } => self_test(backend.as_deref(), invocation.json),
        Command::Events => stream_events().map_or_else(|error| fail(69, &error, true), |()| 0),
        command => {
            let request = IpcRequest { command: command_name(&command), json: invocation.json };
            match send_request(&request) {
                Ok(response) if response.ok => {
                    if let Some(data) = response.data {
                        if invocation.json {
                            println!("{}", serde_json::to_string(&data).unwrap_or_else(|_| "null".to_string()));
                        } else {
                            print_human(&data);
                        }
                    }
                    response.code
                }
                Ok(response) => fail(response.code, response.error.as_deref().unwrap_or("command failed"), invocation.json),
                Err(error) => fail(69, &error, invocation.json),
            }
        }
    }
}

fn command_name(command: &Command) -> String {
    match command {
        Command::Status => "status",
        Command::Doctor => "doctor",
        Command::DesktopsList => "desktops list",
        Command::DesktopsCurrent => "desktops current",
        Command::DesktopsNext => "desktops next",
        Command::DesktopsPrevious => "desktops previous",
        Command::DesktopsCreate => "desktops create",
        Command::Reconcile => "reconcile",
        Command::Enable => "enable",
        Command::Disable => "disable",
        Command::Reload => "reload",
        Command::ConfigPath => "config path",
        Command::ConfigShow => "config show",
        Command::ConfigValidate(_) => "config validate",
        Command::LogsPath => "logs path",
        Command::LogsTail => "logs tail",
        Command::SupportBundle => "support-bundle",
        Command::Shutdown => "shutdown",
        Command::StartupEnable => "startup enable",
        Command::StartupDisable => "startup disable",
        _ => "unsupported",
    }.to_string()
}

fn self_test(backend: Option<&str>, json: bool) -> i32 {
    if backend.is_some_and(|value| value != "mock") {
        return fail(64, "only --backend mock is supported by self-test", json);
    }
    let occupied = |index: usize| DesktopState {
        id: DesktopId(format!("d{index}")),
        occupancy: Occupancy::Occupied,
        current: index == 0,
        empty_grace_elapsed: true,
    };
    let empty = |index: usize| DesktopState {
        id: DesktopId(format!("d{index}")),
        occupancy: Occupancy::Empty,
        current: index == 0,
        empty_grace_elapsed: true,
    };
    let cases = [
        vec![occupied(0)],
        vec![occupied(0), empty(1)],
        vec![occupied(0), empty(1), empty(2)],
        vec![occupied(0), empty(1), occupied(2), empty(3)],
        vec![empty(0)],
    ];
    let valid = cases.iter().all(|case| {
        let result = plan(case);
        result.stable || !result.mutations.is_empty()
    });
    if valid {
        if json { println!("{{\"backend\":\"mock\",\"passed\":true,\"cases\":{}}}", cases.len()); }
        else { println!("self-test mock: PASS ({} cases)", cases.len()); }
        0
    } else {
        fail(70, "mock reconciliation self-test failed", json)
    }
}

fn resolve_data_dir(explicit: Option<&Path>) -> PathBuf {
    if let Some(path) = explicit { return absolute(path); }
    std::env::current_exe()
        .ok()
        .and_then(|path| path.parent().map(Path::to_path_buf))
        .unwrap_or_else(|| PathBuf::from("."))
}

fn absolute(path: &Path) -> PathBuf {
    if path.is_absolute() { path.to_path_buf() }
    else { std::env::current_dir().unwrap_or_default().join(path) }
}

fn attach_console() {
    unsafe {
        let _ = AttachConsole(ATTACH_PARENT_PROCESS);
        let _ = SetConsoleOutputCP(65001);
    }
}

fn print_human(value: &serde_json::Value) {
    match value {
        serde_json::Value::String(text) => println!("{text}"),
        serde_json::Value::Object(object) if object.len() == 1 && object.contains_key("message") => {
            if let Some(message) = object.get("message").and_then(serde_json::Value::as_str) { println!("{message}"); }
        }
        _ => println!("{}", serde_json::to_string_pretty(value).unwrap_or_else(|_| value.to_string())),
    }
}

fn fail(code: i32, message: &str, json: bool) -> i32 {
    if json {
        eprintln!("{}", serde_json::json!({"ok": false, "code": code, "error": message}));
    } else {
        eprintln!("DeskPilot: {message}");
    }
    code
}
