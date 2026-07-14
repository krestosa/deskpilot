use deskpilot::config::{Config, ConfigError, CONFIG_SCHEMA_VERSION};
use std::fs;
use tempfile::tempdir;

#[test]
fn defaults_are_valid_and_versioned() {
    let config = Config::default();
    assert_eq!(config.schema_version, CONFIG_SCHEMA_VERSION);
    config.validate().expect("defaults must validate");
}

#[test]
fn example_configuration_is_valid() {
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("deskpilot.example.toml");
    Config::load(&path).expect("example configuration must validate");
}

#[test]
fn invalid_range_names_the_key() {
    let mut config = Config::default();
    config.wheel.threshold = 0;
    let error = config.validate().expect_err("threshold must be rejected");
    assert!(error.to_string().contains("wheel.threshold"));
}

#[test]
fn unknown_keys_are_rejected() {
    let directory = tempdir().expect("temporary directory");
    let path = directory.path().join("deskpilot.toml");
    fs::write(&path, "schema_version = 1\nenabled = true\nunknown = 1\n").expect("write fixture");
    let error = Config::load(&path).expect_err("unknown key must be rejected");
    assert!(matches!(error, ConfigError::Parse { .. }));
}

#[test]
fn atomic_write_round_trips() {
    let directory = tempdir().expect("temporary directory");
    let path = directory.path().join("deskpilot.toml");
    let config = Config::default();
    config.write_atomic(&path).expect("atomic write");
    assert_eq!(Config::load(&path).expect("reload"), config);
    assert!(!directory.path().join("deskpilot.toml.tmp").exists());
}

#[test]
fn missing_schema_version_migrates_to_current_default() {
    let directory = tempdir().expect("temporary directory");
    let path = directory.path().join("deskpilot.toml");
    fs::write(&path, "enabled = false\n").expect("write fixture");
    let config = Config::load(&path).expect("legacy v0-compatible file should load");
    assert_eq!(config.schema_version, CONFIG_SCHEMA_VERSION);
    assert!(!config.enabled);
}
