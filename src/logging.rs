// File purpose: Implements bounded file logging, rotation, timestamps, and global logger access.
use crate::config::{LogLevel, LoggingConfig};
use std::collections::VecDeque;
use std::fs::{self, File, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::{Mutex, OnceLock};
use time::{format_description::well_known::Rfc3339, OffsetDateTime};

static LOGGER: OnceLock<Logger> = OnceLock::new();

#[derive(Debug)]
pub struct Logger {
    path: PathBuf,
    config: LoggingConfig,
    file: Mutex<File>,
    recent_errors: Mutex<VecDeque<String>>,
}

impl Logger {
    // Function purpose: Performs the initialize operation required by this module.
    pub fn initialize(data_dir: &Path, config: LoggingConfig) -> std::io::Result<&'static Self> {
        let logs = data_dir.join("logs");
        fs::create_dir_all(&logs)?;
        let path = logs.join("deskpilot.log");
        rotate_if_needed(&path, &config)?;
        let file = OpenOptions::new().create(true).append(true).open(&path)?;
        let logger = Self {
            path,
            config,
            file: Mutex::new(file),
            recent_errors: Mutex::new(VecDeque::with_capacity(32)),
        };
        if LOGGER.set(logger).is_err() {
            return Err(std::io::Error::new(
                std::io::ErrorKind::AlreadyExists,
                "logger is already initialized",
            ));
        }
        LOGGER
            .get()
            .ok_or_else(|| std::io::Error::other("logger initialization failed"))
    }

    // Function purpose: Performs the global operation required by this module.
    pub fn global() -> Option<&'static Self> {
        LOGGER.get()
    }

    // Function purpose: Performs the path operation required by this module.
    pub fn path(&self) -> &Path {
        &self.path
    }

    // Function purpose: Performs the recent errors operation required by this module.
    pub fn recent_errors(&self) -> Vec<String> {
        self.recent_errors
            .lock()
            .map_or_else(|_| Vec::new(), |errors| errors.iter().cloned().collect())
    }

    // Function purpose: Performs the log operation required by this module.
    pub fn log(&self, level: LogLevel, message: &str) {
        if !enabled(self.config.level, level) {
            return;
        }
        let sanitized = message.replace(['\r', '\n'], " ");
        let line = format!(
            "{} {:<5} {}\n",
            timestamp_utc(),
            level_name(level),
            sanitized
        );
        if matches!(level, LogLevel::Error | LogLevel::Warn) {
            if let Ok(mut errors) = self.recent_errors.lock() {
                if errors.len() == 32 {
                    errors.pop_front();
                }
                errors.push_back(line.trim().to_string());
            }
        }
        if let Ok(mut file) = self.file.lock() {
            let _ = file.write_all(line.as_bytes());
            let _ = file.flush();
        }
    }
}

// Function purpose: Returns the current UTC timestamp in the format used by logs, events, and reports.
pub fn timestamp_utc() -> String {
    OffsetDateTime::now_utc()
        .format(&Rfc3339)
        .unwrap_or_else(|_| "1970-01-01T00:00:00Z".to_string())
}

// Function purpose: Performs the log operation required by this module.
pub fn log(level: LogLevel, message: impl AsRef<str>) {
    if let Some(logger) = Logger::global() {
        logger.log(level, message.as_ref());
    }
}

// Function purpose: Performs the enabled operation required by this module.
fn enabled(configured: LogLevel, requested: LogLevel) -> bool {
    rank(requested) <= rank(configured)
}

// Function purpose: Performs the rank operation required by this module.
const fn rank(level: LogLevel) -> u8 {
    match level {
        LogLevel::Error => 0,
        LogLevel::Warn => 1,
        LogLevel::Info => 2,
        LogLevel::Debug => 3,
    }
}

// Function purpose: Performs the level name operation required by this module.
const fn level_name(level: LogLevel) -> &'static str {
    match level {
        LogLevel::Error => "ERROR",
        LogLevel::Warn => "WARN",
        LogLevel::Info => "INFO",
        LogLevel::Debug => "DEBUG",
    }
}

// Function purpose: Performs the rotate if needed operation required by this module.
fn rotate_if_needed(path: &Path, config: &LoggingConfig) -> std::io::Result<()> {
    let limit = config.max_file_size_mb.saturating_mul(1024 * 1024);
    let size = fs::metadata(path).map_or(0, |metadata| metadata.len());
    if size < limit {
        return Ok(());
    }
    for index in (1..config.max_files).rev() {
        let from = path.with_extension(format!("log.{index}"));
        let to = path.with_extension(format!("log.{}", index + 1));
        if from.exists() {
            let _ = fs::rename(from, to);
        }
    }
    if path.exists() {
        fs::rename(path, path.with_extension("log.1"))?;
    }
    Ok(())
}
