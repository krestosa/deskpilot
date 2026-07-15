// File purpose: Implements bounded file logging, live rotation, timestamps, and global logger access.
use crate::config::{LogLevel, LoggingConfig};
use std::collections::VecDeque;
use std::fs::{self, File, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::{Mutex, OnceLock};
use time::{format_description::well_known::Rfc3339, OffsetDateTime};

static LOGGER: OnceLock<Logger> = OnceLock::new();

#[derive(Debug)]
struct LoggerState {
    file: Option<File>,
    config: LoggingConfig,
}

#[derive(Debug)]
pub struct Logger {
    path: PathBuf,
    state: Mutex<LoggerState>,
    recent_errors: Mutex<VecDeque<String>>,
}

impl Logger {
    // Function purpose: Initializes the process-global logger and enforces the configured size bound immediately.
    pub fn initialize(data_dir: &Path, config: LoggingConfig) -> std::io::Result<&'static Self> {
        let logs = data_dir.join("logs");
        fs::create_dir_all(&logs)?;
        let path = logs.join("deskpilot.log");
        rotate_if_needed(&path, &config, 0)?;
        let file = open_log_file(&path)?;
        let logger = Self {
            path,
            state: Mutex::new(LoggerState {
                file: Some(file),
                config,
            }),
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

    // Function purpose: Returns the process-global logger when it has been initialized.
    pub fn global() -> Option<&'static Self> {
        LOGGER.get()
    }

    // Function purpose: Returns the active log file path.
    pub fn path(&self) -> &Path {
        &self.path
    }

    // Function purpose: Updates hot-reloadable logging limits without restarting the process.
    pub fn reconfigure(&self, config: LoggingConfig) -> std::io::Result<()> {
        let mut state = self
            .state
            .lock()
            .map_err(|_| std::io::Error::other("logger state lock poisoned"))?;
        state.config = config;
        rotate_locked(&self.path, &mut state, 0)
    }

    // Function purpose: Returns the bounded recent warning and error history used by diagnostics.
    pub fn recent_errors(&self) -> Vec<String> {
        self.recent_errors
            .lock()
            .map_or_else(|_| Vec::new(), |errors| errors.iter().cloned().collect())
    }

    // Function purpose: Writes one sanitized line and performs live rotation before the configured limit is exceeded.
    pub fn log(&self, level: LogLevel, message: &str) {
        let sanitized = message.replace(['\r', '\n'], " ");
        let line = format!(
            "{} {:<5} {}\n",
            timestamp_utc(),
            level_name(level),
            sanitized
        );

        let mut state = match self.state.lock() {
            Ok(state) => state,
            Err(_) => return,
        };
        if !enabled(state.config.level, level) {
            return;
        }
        if rotate_locked(&self.path, &mut state, line.len() as u64).is_err() {
            return;
        }
        if let Some(file) = state.file.as_mut() {
            let _ = file.write_all(line.as_bytes());
            let _ = file.flush();
        }
        drop(state);

        if matches!(level, LogLevel::Error | LogLevel::Warn) {
            if let Ok(mut errors) = self.recent_errors.lock() {
                if errors.len() == 32 {
                    errors.pop_front();
                }
                errors.push_back(line.trim().to_string());
            }
        }
    }
}

// Function purpose: Returns the current UTC timestamp in the format used by logs, events, and reports.
pub fn timestamp_utc() -> String {
    OffsetDateTime::now_utc()
        .format(&Rfc3339)
        .unwrap_or_else(|_| "1970-01-01T00:00:00Z".to_string())
}

// Function purpose: Sends a log message to the initialized process-global logger.
pub fn log(level: LogLevel, message: impl AsRef<str>) {
    if let Some(logger) = Logger::global() {
        logger.log(level, message.as_ref());
    }
}

fn open_log_file(path: &Path) -> std::io::Result<File> {
    OpenOptions::new().create(true).append(true).open(path)
}

fn rotate_locked(path: &Path, state: &mut LoggerState, incoming: u64) -> std::io::Result<()> {
    let limit = state
        .config
        .max_file_size_mb
        .saturating_mul(1024 * 1024);
    let size = fs::metadata(path).map_or(0, |metadata| metadata.len());
    if size.saturating_add(incoming) < limit {
        if state.file.is_none() {
            state.file = Some(open_log_file(path)?);
        }
        return Ok(());
    }

    state.file.take();
    rotate_files(path, &state.config)?;
    state.file = Some(open_log_file(path)?);
    Ok(())
}

fn enabled(configured: LogLevel, requested: LogLevel) -> bool {
    rank(requested) <= rank(configured)
}

const fn rank(level: LogLevel) -> u8 {
    match level {
        LogLevel::Error => 0,
        LogLevel::Warn => 1,
        LogLevel::Info => 2,
        LogLevel::Debug => 3,
    }
}

const fn level_name(level: LogLevel) -> &'static str {
    match level {
        LogLevel::Error => "ERROR",
        LogLevel::Warn => "WARN",
        LogLevel::Info => "INFO",
        LogLevel::Debug => "DEBUG",
    }
}

fn rotate_if_needed(path: &Path, config: &LoggingConfig, incoming: u64) -> std::io::Result<()> {
    let limit = config.max_file_size_mb.saturating_mul(1024 * 1024);
    let size = fs::metadata(path).map_or(0, |metadata| metadata.len());
    if size.saturating_add(incoming) < limit {
        return Ok(());
    }
    rotate_files(path, config)
}

fn rotate_files(path: &Path, config: &LoggingConfig) -> std::io::Result<()> {
    if config.max_files == 0 {
        return Ok(());
    }
    let oldest = path.with_extension(format!("log.{}", config.max_files));
    if oldest.exists() {
        fs::remove_file(oldest)?;
    }
    for index in (1..config.max_files).rev() {
        let from = path.with_extension(format!("log.{index}"));
        let to = path.with_extension(format!("log.{}", index + 1));
        if from.exists() {
            fs::rename(from, to)?;
        }
    }
    if path.exists() {
        fs::rename(path, path.with_extension("log.1"))?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::rotate_if_needed;
    use crate::config::{LogLevel, LoggingConfig};
    use std::fs;

    #[test]
    fn rotates_when_incoming_line_crosses_limit() {
        let temp = tempfile::tempdir().expect("temporary directory");
        let path = temp.path().join("deskpilot.log");
        fs::write(&path, vec![0_u8; 1024 * 1024 - 4]).expect("seed log");
        let config = LoggingConfig {
            level: LogLevel::Info,
            max_files: 2,
            max_file_size_mb: 1,
        };
        rotate_if_needed(&path, &config, 8).expect("rotation should succeed");
        assert!(!path.exists());
        assert!(path.with_extension("log.1").exists());
    }
}
