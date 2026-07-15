// File purpose: Provides Windows version, session, Explorer, integrity, SID, console, and filesystem helpers.
use std::ffi::c_void;
use std::fs::OpenOptions;
use std::mem::{size_of, zeroed};
use std::path::Path;
use windows_sys::Win32::Foundation::{
    CloseHandle, GetLastError, LocalFree, ERROR_SUCCESS, HANDLE, HLOCAL,
};
use windows_sys::Win32::Security::Authorization::ConvertSidToStringSidW;
use windows_sys::Win32::Security::{
    GetSidSubAuthority, GetSidSubAuthorityCount, GetTokenInformation, TokenIntegrityLevel,
    TokenUser, TOKEN_MANDATORY_LABEL, TOKEN_QUERY, TOKEN_USER,
};
use windows_sys::Win32::System::Registry::{
    RegCloseKey, RegOpenKeyExW, RegQueryValueExW, HKEY_LOCAL_MACHINE, KEY_QUERY_VALUE, REG_DWORD,
};
use windows_sys::Win32::System::SystemInformation::{GetVersionExW, OSVERSIONINFOW};
use windows_sys::Win32::System::Threading::{
    GetCurrentProcess, GetCurrentProcessId, OpenProcessToken, ProcessIdToSessionId,
};
use windows_sys::Win32::UI::WindowsAndMessaging::{FindWindowW, GetShellWindow};

use super::util::wide;

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct WindowsVersion {
    pub major: u32,
    pub minor: u32,
    pub build: u32,
    pub revision: u32,
}

#[repr(C)]
struct RtlOsVersionInfoW {
    size: u32,
    major: u32,
    minor: u32,
    build: u32,
    platform_id: u32,
    service_pack: [u16; 128],
}

#[link(name = "ntdll")]
unsafe extern "system" {
    // Function purpose: Reads the native Windows version without manifest virtualization.
    #[link_name = "RtlGetVersion"]
    fn rtl_get_version(version_information: *mut RtlOsVersionInfoW) -> i32;
}

// Function purpose: Returns the native Windows build and servicing revision.
pub fn windows_version() -> WindowsVersion {
    let revision = read_ubr().unwrap_or(0);
    let mut version = rtl_windows_version()
        .or_else(legacy_windows_version)
        .unwrap_or_default();
    version.revision = revision;
    version
}

fn rtl_windows_version() -> Option<WindowsVersion> {
    unsafe {
        let mut info: RtlOsVersionInfoW = zeroed();
        info.size = size_of::<RtlOsVersionInfoW>() as u32;
        (rtl_get_version(&mut info) == 0).then_some(WindowsVersion {
            major: info.major,
            minor: info.minor,
            build: info.build,
            revision: 0,
        })
    }
}

fn legacy_windows_version() -> Option<WindowsVersion> {
    unsafe {
        let mut info: OSVERSIONINFOW = zeroed();
        info.dwOSVersionInfoSize = size_of::<OSVERSIONINFOW>() as u32;
        (GetVersionExW(&mut info) != 0).then_some(WindowsVersion {
            major: info.dwMajorVersion,
            minor: info.dwMinorVersion,
            build: info.dwBuildNumber,
            revision: 0,
        })
    }
}

fn read_ubr() -> Option<u32> {
    unsafe {
        let subkey = wide("SOFTWARE\\Microsoft\\Windows NT\\CurrentVersion");
        let mut key = 0;
        if RegOpenKeyExW(
            HKEY_LOCAL_MACHINE,
            subkey.as_ptr(),
            0,
            KEY_QUERY_VALUE,
            &mut key,
        ) != ERROR_SUCCESS
        {
            return None;
        }

        let value_name = wide("UBR");
        let mut value = 0_u32;
        let mut value_type = 0_u32;
        let mut size = size_of::<u32>() as u32;
        let result = RegQueryValueExW(
            key,
            value_name.as_ptr(),
            std::ptr::null_mut(),
            &mut value_type,
            (&mut value as *mut u32).cast::<u8>(),
            &mut size,
        );
        RegCloseKey(key);

        (result == ERROR_SUCCESS && value_type == REG_DWORD && size == size_of::<u32>() as u32)
            .then_some(value)
    }
}

// Function purpose: Returns whether the process has an interactive shell session.
pub fn is_interactive_session() -> bool {
    unsafe { GetShellWindow() != 0 }
}

// Function purpose: Returns whether Explorer's taskbar window is present.
pub fn explorer_running() -> bool {
    let class = wide("Shell_TrayWnd");
    unsafe { FindWindowW(class.as_ptr(), std::ptr::null()) != 0 }
}

// Function purpose: Returns the current process identifier.
pub fn current_process_id() -> u32 {
    unsafe { GetCurrentProcessId() }
}

// Function purpose: Returns the current Windows session identifier for IPC namespace isolation.
pub fn current_session_id() -> Result<u32, String> {
    unsafe {
        let mut session_id = 0_u32;
        if ProcessIdToSessionId(GetCurrentProcessId(), &mut session_id) == 0 {
            return Err(format!("ProcessIdToSessionId failed: {}", GetLastError()));
        }
        Ok(session_id)
    }
}

// Function purpose: Returns the string SID for the process token user.
pub fn current_user_sid() -> Result<String, String> {
    unsafe {
        let token = open_process_token()?;
        let result = token_user_sid(token);
        CloseHandle(token);
        result
    }
}

// Function purpose: Returns the process integrity level for diagnostics.
pub fn integrity_level() -> String {
    unsafe {
        let Ok(token) = open_process_token() else {
            return "unknown".to_string();
        };
        let mut size = 0;
        let _ = GetTokenInformation(
            token,
            TokenIntegrityLevel,
            std::ptr::null_mut(),
            0,
            &mut size,
        );
        if size == 0 {
            CloseHandle(token);
            return "unknown".to_string();
        }
        let mut buffer = vec![0_u8; size as usize];
        let ok = GetTokenInformation(
            token,
            TokenIntegrityLevel,
            buffer.as_mut_ptr().cast::<c_void>(),
            size,
            &mut size,
        );
        CloseHandle(token);
        if ok == 0 {
            return "unknown".to_string();
        }
        let label = &*(buffer.as_ptr().cast::<TOKEN_MANDATORY_LABEL>());
        let count = *GetSidSubAuthorityCount(label.Label.Sid) as u32;
        if count == 0 {
            return "unknown".to_string();
        }
        let rid = *GetSidSubAuthority(label.Label.Sid, count - 1);
        match rid {
            0x0000..=0x0FFF => "untrusted",
            0x1000..=0x1FFF => "low",
            0x2000..=0x2FFF => "medium",
            0x3000..=0x3FFF => "high",
            0x4000..=0x4FFF => "system",
            _ => "protected",
        }
        .to_string()
    }
}

// Function purpose: Tests portable directory writability without overwriting a pre-existing path.
pub fn portable_write_test(data_dir: &Path) -> bool {
    for attempt in 0..16_u32 {
        let path = data_dir.join(format!(
            ".deskpilot-write-test-{}-{attempt}",
            current_process_id()
        ));
        match OpenOptions::new().write(true).create_new(true).open(&path) {
            Ok(mut file) => {
                use std::io::Write as _;
                let written = file.write_all(b"deskpilot").and_then(|()| file.sync_all());
                drop(file);
                let removed = std::fs::remove_file(path);
                return written.is_ok() && removed.is_ok();
            }
            Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => continue,
            Err(_) => return false,
        }
    }
    false
}

unsafe fn open_process_token() -> Result<HANDLE, String> {
    let mut token = 0;
    if unsafe { OpenProcessToken(GetCurrentProcess(), TOKEN_QUERY, &mut token) } == 0 {
        return Err(format!("OpenProcessToken failed: {}", unsafe {
            GetLastError()
        }));
    }
    Ok(token)
}

unsafe fn token_user_sid(token: HANDLE) -> Result<String, String> {
    let mut size = 0;
    let _ = unsafe { GetTokenInformation(token, TokenUser, std::ptr::null_mut(), 0, &mut size) };
    if size == 0 {
        return Err("GetTokenInformation(TokenUser) returned no size".to_string());
    }
    let mut buffer = vec![0_u8; size as usize];
    if unsafe {
        GetTokenInformation(
            token,
            TokenUser,
            buffer.as_mut_ptr().cast::<c_void>(),
            size,
            &mut size,
        )
    } == 0
    {
        return Err(format!(
            "GetTokenInformation(TokenUser) failed: {}",
            unsafe { GetLastError() }
        ));
    }
    let token_user = unsafe { &*(buffer.as_ptr().cast::<TOKEN_USER>()) };
    let mut sid_string = std::ptr::null_mut();
    if unsafe { ConvertSidToStringSidW(token_user.User.Sid, &mut sid_string) } == 0 {
        return Err(format!("ConvertSidToStringSidW failed: {}", unsafe {
            GetLastError()
        }));
    }
    let mut length = 0;
    while unsafe { *sid_string.add(length) } != 0 {
        length += 1;
    }
    let result =
        String::from_utf16_lossy(unsafe { std::slice::from_raw_parts(sid_string, length) });
    unsafe { LocalFree(sid_string as HLOCAL) };
    Ok(result)
}

#[cfg(test)]
mod tests {
    use super::windows_version;

    #[test]
    fn native_version_detection_is_not_manifest_virtualized() {
        let version = windows_version();
        assert_eq!(
            version.major, 10,
            "expected native Windows 10/11 major version, got {version:?}"
        );
        assert!(
            version.build >= 22_000,
            "DeskPilot tests require Windows 11 or later, got {version:?}"
        );
    }
}
