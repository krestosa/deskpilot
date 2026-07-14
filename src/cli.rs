use std::path::PathBuf;
use thiserror::Error;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Invocation {
    pub command: Command,
    pub data_dir: Option<PathBuf>,
    pub json: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Command {
    Run(RunOptions),
    Status,
    Doctor,
    DesktopsList,
    DesktopsCurrent,
    DesktopsNext,
    DesktopsPrevious,
    DesktopsCreate,
    Reconcile,
    Enable,
    Disable,
    Reload,
    ConfigPath,
    ConfigShow,
    ConfigValidate(Option<PathBuf>),
    LogsPath,
    LogsTail,
    Events,
    SupportBundle,
    Shutdown,
    SelfTest { backend: Option<String> },
    Version,
    Help,
    StartupEnable,
    StartupDisable,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct RunOptions {
    pub foreground: bool,
    pub no_tray: bool,
    pub no_hook: bool,
    pub no_dynamic: bool,
}

#[derive(Debug, Error, PartialEq, Eq)]
pub enum CliError {
    #[error("unknown command or option: {0}")]
    Unknown(String),
    #[error("missing value for {0}")]
    MissingValue(String),
}

impl Invocation {
    pub fn parse<I, S>(args: I) -> Result<Self, CliError>
    where
        I: IntoIterator<Item = S>,
        S: Into<String>,
    {
        let mut values: Vec<String> = args.into_iter().map(Into::into).collect();
        if !values.is_empty() {
            values.remove(0);
        }
        let mut data_dir = None;
        let mut json = false;
        let mut filtered = Vec::new();
        let mut index = 0;
        while index < values.len() {
            match values[index].as_str() {
                "--data-dir" => {
                    index += 1;
                    let value = values
                        .get(index)
                        .ok_or_else(|| CliError::MissingValue("--data-dir".to_string()))?;
                    data_dir = Some(PathBuf::from(value));
                }
                "--json" => json = true,
                value => filtered.push(value.to_string()),
            }
            index += 1;
        }

        let command = parse_command(&filtered)?;
        Ok(Self {
            command,
            data_dir,
            json,
        })
    }

    pub fn needs_console(&self) -> bool {
        !matches!(
            self.command,
            Command::Run(RunOptions {
                foreground: false,
                ..
            })
        )
    }
}

fn parse_command(args: &[String]) -> Result<Command, CliError> {
    if args.is_empty() {
        return Ok(Command::Run(RunOptions::default()));
    }
    match args[0].as_str() {
        "run" => {
            let mut options = RunOptions::default();
            for option in &args[1..] {
                match option.as_str() {
                    "--foreground" => options.foreground = true,
                    "--no-tray" => options.no_tray = true,
                    "--no-hook" => options.no_hook = true,
                    "--no-dynamic" => options.no_dynamic = true,
                    unknown => return Err(CliError::Unknown(unknown.to_string())),
                }
            }
            Ok(Command::Run(options))
        }
        "status" => exact(args, Command::Status),
        "doctor" => exact(args, Command::Doctor),
        "reconcile" => exact(args, Command::Reconcile),
        "enable" => exact(args, Command::Enable),
        "disable" => exact(args, Command::Disable),
        "reload" => exact(args, Command::Reload),
        "events" => exact(args, Command::Events),
        "support-bundle" => exact(args, Command::SupportBundle),
        "shutdown" => exact(args, Command::Shutdown),
        "--version" | "version" => exact(args, Command::Version),
        "--help" | "help" => Ok(Command::Help),
        "desktops" => match args.get(1).map(String::as_str) {
            Some("list") if args.len() == 2 => Ok(Command::DesktopsList),
            Some("current") if args.len() == 2 => Ok(Command::DesktopsCurrent),
            Some("next") if args.len() == 2 => Ok(Command::DesktopsNext),
            Some("previous") if args.len() == 2 => Ok(Command::DesktopsPrevious),
            Some("create") if args.len() == 2 => Ok(Command::DesktopsCreate),
            Some(value) => Err(CliError::Unknown(format!("desktops {value}"))),
            None => Err(CliError::MissingValue("desktops subcommand".to_string())),
        },
        "config" => match args.get(1).map(String::as_str) {
            Some("path") if args.len() == 2 => Ok(Command::ConfigPath),
            Some("show") if args.len() == 2 => Ok(Command::ConfigShow),
            Some("validate") if args.len() <= 3 => {
                Ok(Command::ConfigValidate(args.get(2).map(PathBuf::from)))
            }
            Some(value) => Err(CliError::Unknown(format!("config {value}"))),
            None => Err(CliError::MissingValue("config subcommand".to_string())),
        },
        "logs" => match args.get(1).map(String::as_str) {
            Some("path") if args.len() == 2 => Ok(Command::LogsPath),
            Some("tail") if args.len() == 2 => Ok(Command::LogsTail),
            Some(value) => Err(CliError::Unknown(format!("logs {value}"))),
            None => Err(CliError::MissingValue("logs subcommand".to_string())),
        },
        "startup" => match args.get(1).map(String::as_str) {
            Some("enable") if args.len() == 2 => Ok(Command::StartupEnable),
            Some("disable") if args.len() == 2 => Ok(Command::StartupDisable),
            Some(value) => Err(CliError::Unknown(format!("startup {value}"))),
            None => Err(CliError::MissingValue("startup subcommand".to_string())),
        },
        "self-test" => {
            if args.len() == 1 {
                return Ok(Command::SelfTest { backend: None });
            }
            if args.len() == 3 && args[1] == "--backend" {
                return Ok(Command::SelfTest {
                    backend: Some(args[2].clone()),
                });
            }
            Err(CliError::Unknown(args[1..].join(" ")))
        }
        unknown => Err(CliError::Unknown(unknown.to_string())),
    }
}

fn exact(args: &[String], command: Command) -> Result<Command, CliError> {
    if args.len() == 1 {
        Ok(command)
    } else {
        Err(CliError::Unknown(args[1..].join(" ")))
    }
}

pub const HELP: &str = r#"DeskPilot 0.1.0

Usage:
  DeskPilot.exe [--data-dir PATH]
  DeskPilot.exe run [--foreground] [--no-tray] [--no-hook] [--no-dynamic]
  DeskPilot.exe status [--json]
  DeskPilot.exe doctor [--json]
  DeskPilot.exe desktops <list|current|next|previous|create> [--json]
  DeskPilot.exe reconcile
  DeskPilot.exe <enable|disable|reload|shutdown>
  DeskPilot.exe config <path|show|validate [FILE]>
  DeskPilot.exe logs <path|tail>
  DeskPilot.exe events --json
  DeskPilot.exe support-bundle
  DeskPilot.exe startup <enable|disable>
  DeskPilot.exe self-test [--backend mock]
  DeskPilot.exe --version
"#;
