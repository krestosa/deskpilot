// File purpose: Converts between Rust strings and null-terminated UTF-16 Windows strings.
use std::ffi::OsStr;
use std::os::windows::ffi::OsStrExt;

// Function purpose: Performs the wide operation required by this module.
pub fn wide(value: impl AsRef<OsStr>) -> Vec<u16> {
    value
        .as_ref()
        .encode_wide()
        .chain(std::iter::once(0))
        .collect()
}

// Function purpose: Performs the from wide operation required by this module.
pub fn from_wide(buffer: &[u16]) -> String {
    let end = buffer
        .iter()
        .position(|value| *value == 0)
        .unwrap_or(buffer.len());
    String::from_utf16_lossy(&buffer[..end])
}
