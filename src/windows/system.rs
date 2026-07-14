use std::ffi::c_void;
use std::mem::{size_of, zeroed};
use std::path::Path;
use windows_sys::Win32::Foundation::{CloseHandle, GetLastError, HANDLE, HLOCAL};
use windows_sys::Win32::Security::Authorization::ConvertSidToStringSidW;
use windows_sys::Win32::Security::{
    GetSidSubAuthority, GetSidSubAuthorityCount, GetTokenInformation, OpenProcessToken,
    TokenIntegrityLevel, TokenUser, TOKEN_MANDATORY_LABEL, TOKEN_QUERY, TOKEN_USER,
};
use windows_sys::Win32::System::Memory::LocalFree;
use windows_sys::Win32::System::SystemInformation::{GetVersionExW, OSVERSIONINFOW};
use windows_sys::Win32::System::Threading::{GetCurrentProcess, GetCurrentProcessId};
use windows_sys::Win32::UI::WindowsAndMessaging::{FindWindowW, GetShellWindow};

use super::util::wide;

#[derive(Debug, Clone, Copy)]
pub struct WindowsVersion {
    pub major: u32,
    pub minor: u32,
    pub build: u32,
}

pub fn windows_version() -> WindowsVersion {
    unsafe {
        let mut info: OSVERSIONINFOW = zeroed();
        info.dwOSVersionInfoSize = size_of::<OSVERSIONINFOW>() as u32;
        if GetVersionExW(&mut info) != 0 {
            WindowsVersion {
                major: info.dwMajorVersion,
                minor: info.dwMinorVersion,
                build: info.dwBuildNumber,
            }
        } else {
            WindowsVersion {
                major: 0,
                minor: 0,
                build: 0,
            }
        }
    }
}

pub fn is_interactive_session() -> bool {
    unsafe { GetShellWindow() != 0 }
}

pub fn explorer_running() -> bool {
    let class = wide("Shell_TrayWnd");
    unsafe { FindWindowW(class.as_ptr(), std::ptr::null()) != 0 }
}

pub fn current_process_id() -> u32 {
    unsafe { GetCurrentProcessId() }
}

pub fn current_user_sid() -> Result<String, String> {
    unsafe {
        let token = open_process_token()?;
        let result = token_user_sid(token);
        CloseHandle(token);
        result
    }
}

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

pub fn portable_write_test(data_dir: &Path) -> bool {
    let path = data_dir.join(".deskpilot-write-test");
    std::fs::write(&path, b"deskpilot")
        .and_then(|()| std::fs::remove_file(path))
        .is_ok()
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
