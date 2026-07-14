// File purpose: Defines portable configuration models, defaults, validation, migration, loading, and atomic persistence.
use serde::{Deserialize, Serialize};
use std::fs::{self, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};
use thiserror::Error;

use crate::{CONFIG_FILE_NAME, EXAMPLE_CONFIG_FILE_NAME};

pub const CONFIG_SCHEMA_VERSION: u32 = 1;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(default, deny_unknown_fields)]
pub struct Config {
    pub schema_version: u32,
    pub enabled: bool,
    pub wheel: WheelConfig,
    pub desktops: DesktopConfig,
    pub windows: WindowConfig,
    pub logging: LoggingConfig,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(default, deny_unknown_fields)]
pub struct WheelConfig {
    pub direction: WheelDirection,
    pub navigation: NavigationMode,
    pub threshold: i32,
    pub cooldown_ms: u64,
}

#[derive(Debug, Clone, Copy, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum WheelDirection {
    #[default]
    Normal,
    Inverted,
}

#[derive(Debug, Clone, Copy, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum NavigationMode {
    Clamp,
    #[default]
    Wrap,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(default, deny_unknown_fields)]
pub struct DesktopConfig {
    pub dynamic: bool,
    pub reconcile_delay_ms: u64,
    pub empty_grace_ms: u64,
    pub watchdog_interval_ms: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(default, deny_unknown_fields)]
pub struct WindowConfig {
    pub suspend_in_exclusive_fullscreen: bool,
    pub ignore_executables: Vec<String>,
    pub ignore_classes: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(default, deny_unknown_fields)]
pub struct LoggingConfig {
    pub level: LogLevel,
    pub max_files: usize,
    pub max_file_size_mb: u64,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum LogLevel {
    Error,
    Warn,
    Info,
    Debug,
}

impl Default for Config {
    // Function purpose: Constructs the documented safe default value for this type.
    fn default() -> Self {
        Self {
            schema_version: CONFIG_SCHEMA_VERSION,
            enabled: true,
            wheel: WheelConfig::default(),
            desktops: DesktopConfig::default(),
            windows: WindowConfig::default(),
            logging: LoggingConfig::default(),
        }
    }
}

impl Default for WheelConfig {
    // Function purpose: Constructs the documented safe default value for this type.
    fn default() -> Self {
        Self {
            direction: WheelDirection::Normal,
            navigation: NavigationMode::Wrap,
            threshold: 120,
            cooldown_ms: 180,
        }
    }
}

impl Default for DesktopConfig {
    // Function purpose: Constructs the documented safe default value for this type.
    fn default() -> Self {
        Self {
            dynamic: true,
            reconcile_delay_ms: 750,
            empty_grace_ms: 1_500,
            watchdog_interval_ms: 3_000,
        }
    }
}

impl Default for WindowConfig {
    // Function purpose: Constructs the documented safe default value for this type.
    fn default() -> Self {
        Self {
            suspend_in_exclusive_fullscreen: true,
            ignore_executables: Vec::new(),
            ignore_classes: Vec::new(),
        }
    }
}

impl Default for LoggingConfig {
    // Function purpose: Constructs the documented safe default value for this type.
    fn default() -> Self {
        Self {
            level: LogLevel::Info,
            max_files: 5,
            max_file_size_mb: 2,
        }
    }
}

#[derive(Debug, Error)]
pub enum ConfigError {
    #[error("configuration I/O error at {path}: {source}")]
    Io {
        path: PathBuf,
        source: std::io::Error,
    },
    #[error("invalid TOML in {path}: {source}")]
    Parse {
        path: PathBuf,
        source: toml::de::Error,
    },
    #[error("invalid configuration key or value: {0}")]
    Validation(String),
}

impl Config {
    // Function purpose: Performs the validate operation required by this module.
    pub fn validate(&self) -> Result<(), ConfigError> {
        if self.schema_version != CONFIG_SCHEMA_VERSION {
            return Err(ConfigError::Validation(format!(
                "schema_version must be {CONFIG_SCHEMA_VERSION}, got {}",
                self.schema_version
            )));
        }
        validate_range(
            "wheel.threshold",
            i64::from(self.wheel.threshold),
            40,
            1_200,
        )?;
        validate_range(
            "wheel.cooldown_ms",
            to_i64(self.wheel.cooldown_ms)?,
            0,
            5_000,
        )?;
        validate_range(
            "desktops.reconcile_delay_ms",
            to_i64(self.desktops.reconcile_delay_ms)?,
            50,
            60_000,
        )?;
        validate_range(
            "desktops.empty_grace_ms",
            to_i64(self.desktops.empty_grace_ms)?,
            0,
            300_000,
        )?;
        validate_range(
            "desktops.watchdog_interval_ms",
            to_i64(self.desktops.watchdog_interval_ms)?,
            500,
            300_000,
        )?;
        validate_range("logging.max_files", self.logging.max_files as i64, 1, 50)?;
        validate_range(
            "logging.max_file_size_mb",
            to_i64(self.logging.max_file_size_mb)?,
            1,
            100,
        )?;
        for value in self
            .windows
            .ignore_executables
            .iter()
            .chain(self.windows.ignore_classes.iter())
        {
            if value.trim().is_empty() {
                return Err(ConfigError::Validation(
                    "ignore rules cannot contain empty strings".to_string(),
                ));
            }
        }
        Ok(())
    }

    // Function purpose: Performs the load operation required by this module.
    pub fn load(path: &Path) -> Result<Self, ConfigError> {
        let text = fs::read_to_string(path).map_err(|source| ConfigError::Io {
            path: path.to_path_buf(),
            source,
        })?;
        let config: Self = toml::from_str(&text).map_err(|source| ConfigError::Parse {
            path: path.to_path_buf(),
            source,
        })?;
        config.validate()?;
        Ok(config)
    }

    // Function purpose: Loads or create.
    pub fn load_or_create(data_dir: &Path) -> Result<(Self, PathBuf), ConfigError> {
        let path = data_dir.join(CONFIG_FILE_NAME);
        if path.exists() {
            return Self::load(&path).map(|config| (config, path));
        }
        let example = data_dir.join(EXAMPLE_CONFIG_FILE_NAME);
        let config = if example.exists() {
            Self::load(&example)?
        } else {
            Self::default()
        };
        config.write_atomic(&path)?;
        Ok((config, path))
    }

    // Function purpose: Writes atomic.
    pub fn write_atomic(&self, path: &Path) -> Result<(), ConfigError> {
        self.validate()?;
        let parent = path.parent().unwrap_or_else(|| Path::new("."));
        fs::create_dir_all(parent).map_err(|source| ConfigError::Io {
            path: parent.to_path_buf(),
            source,
        })?;
        let serialized = toml::to_string_pretty(self)
            .map_err(|error| ConfigError::Validation(error.to_string()))?;
        let temporary = path.with_extension("toml.tmp");
        let mut file = OpenOptions::new()
            .create(true)
            .truncate(true)
            .write(true)
            .open(&temporary)
            .map_err(|source| ConfigError::Io {
                path: temporary.clone(),
                source,
            })?;
        file.write_all(serialized.as_bytes())
            .and_then(|()| file.sync_all())
            .map_err(|source| ConfigError::Io {
                path: temporary.clone(),
                source,
            })?;
        replace_file(&temporary, path).map_err(|source| ConfigError::Io {
            path: path.to_path_buf(),
            source,
        })
    }
}

// Function purpose: Validates range.
fn validate_range(key: &str, value: i64, minimum: i64, maximum: i64) -> Result<(), ConfigError> {
    if (minimum..=maximum).contains(&value) {
        Ok(())
    } else {
        Err(ConfigError::Validation(format!(
            "{key} must be in {minimum}..={maximum}, got {value}"
        )))
    }
}

// Function purpose: Performs the to i64 operation required by this module.
fn to_i64(value: u64) -> Result<i64, ConfigError> {
    i64::try_from(value)
        .map_err(|_| ConfigError::Validation("numeric value is too large".to_string()))
}

// Function purpose: Performs the replace file operation required by this module.
#[cfg(windows)]
fn replace_file(source: &Path, destination: &Path) -> std::io::Result<()> {
    use std::os::windows::ffi::OsStrExt;
    use windows_sys::Win32::Storage::FileSystem::{
        MoveFileExW, MOVEFILE_REPLACE_EXISTING, MOVEFILE_WRITE_THROUGH,
    };

    let source: Vec<u16> = source.as_os_str().encode_wide().chain(Some(0)).collect();
    let destination: Vec<u16> = destination
        .as_os_str()
        .encode_wide()
        .chain(Some(0))
        .collect();
    let result = unsafe {
        MoveFileExW(
            source.as_ptr(),
            destination.as_ptr(),
            MOVEFILE_REPLACE_EXISTING | MOVEFILE_WRITE_THROUGH,
        )
    };
    if result == 0 {
        Err(std::io::Error::last_os_error())
    } else {
        Ok(())
    }
}

// Function purpose: Performs the replace file operation required by this module.
#[cfg(not(windows))]
fn replace_file(source: &Path, destination: &Path) -> std::io::Result<()> {
    fs::rename(source, destination)
}
