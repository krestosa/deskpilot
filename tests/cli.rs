use deskpilot::cli::{CliError, Command, Invocation, RunOptions};
use std::path::PathBuf;

fn parse(args: &[&str]) -> Invocation {
    Invocation::parse(args.iter().copied()).expect("valid CLI")
}

#[test]
fn no_arguments_starts_tray_mode() {
    assert_eq!(
        parse(&["DeskPilot.exe"]).command,
        Command::Run(RunOptions::default())
    );
}

#[test]
fn run_safe_modes_are_parsed() {
    let invocation = parse(&[
        "DeskPilot.exe",
        "run",
        "--foreground",
        "--no-tray",
        "--no-hook",
        "--no-dynamic",
    ]);
    assert_eq!(
        invocation.command,
        Command::Run(RunOptions {
            foreground: true,
            no_tray: true,
            no_hook: true,
            no_dynamic: true,
        })
    );
}

#[test]
fn global_options_work_before_or_after_subcommand() {
    let invocation = parse(&[
        "DeskPilot.exe",
        "doctor",
        "--json",
        "--data-dir",
        r"C:\Portable\DeskPilot",
    ]);
    assert_eq!(invocation.command, Command::Doctor);
    assert!(invocation.json);
    assert_eq!(
        invocation.data_dir,
        Some(PathBuf::from(r"C:\Portable\DeskPilot"))
    );
}

#[test]
fn command_hierarchy_is_stable() {
    assert_eq!(parse(&["DeskPilot.exe", "desktops", "list"]).command, Command::DesktopsList);
    assert_eq!(parse(&["DeskPilot.exe", "config", "path"]).command, Command::ConfigPath);
    assert_eq!(parse(&["DeskPilot.exe", "logs", "tail"]).command, Command::LogsTail);
    assert_eq!(parse(&["DeskPilot.exe", "startup", "enable"]).command, Command::StartupEnable);
}

#[test]
fn mock_self_test_is_parsed() {
    assert_eq!(
        parse(&["DeskPilot.exe", "self-test", "--backend", "mock"]).command,
        Command::SelfTest {
            backend: Some("mock".to_string()),
        }
    );
}

#[test]
fn missing_data_directory_value_is_explicit() {
    let error = Invocation::parse(["DeskPilot.exe", "--data-dir"])
        .expect_err("missing value must fail");
    assert_eq!(error, CliError::MissingValue("--data-dir".to_string()));
}

#[test]
fn unknown_command_is_rejected() {
    let error = Invocation::parse(["DeskPilot.exe", "explode"])
        .expect_err("unknown command must fail");
    assert_eq!(error, CliError::Unknown("explode".to_string()));
}
