// File purpose: Exposes DeskPilot library modules and shared application constants.
#![cfg_attr(not(test), deny(clippy::unwrap_used, clippy::expect_used))]

pub mod cli;
pub mod config;
pub mod diagnostics;
pub mod event;
pub mod logging;
pub mod reconciliation;
pub mod support;
pub mod wheel;

#[cfg(windows)]
pub mod app;
#[cfg(windows)]
pub mod ipc;
#[cfg(windows)]
pub mod tray;
#[cfg(windows)]
pub mod windows;

pub const APP_NAME: &str = "DeskPilot";
pub const APP_VERSION: &str = env!("CARGO_PKG_VERSION");
pub const CONFIG_FILE_NAME: &str = "deskpilot.toml";
pub const EXAMPLE_CONFIG_FILE_NAME: &str = "deskpilot.example.toml";
