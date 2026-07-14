use std::path::{Path, PathBuf};
use windows::core::{Interface, PCWSTR};
use windows::Win32::System::Com::{
    CoCreateInstance, CoInitializeEx, CoUninitialize, IPersistFile, CLSCTX_INPROC_SERVER,
    COINIT_APARTMENTTHREADED,
};
use windows::Win32::UI::Shell::{IShellLinkW, ShellLink};

use super::util::wide;

pub fn shortcut_path() -> Result<PathBuf, String> {
    let appdata = std::env::var_os("APPDATA").ok_or_else(|| "APPDATA is not defined".to_string())?;
    Ok(PathBuf::from(appdata)
        .join("Microsoft")
        .join("Windows")
        .join("Start Menu")
        .join("Programs")
        .join("Startup")
        .join("DeskPilot.lnk"))
}

pub fn is_enabled() -> bool { shortcut_path().is_ok_and(|path| path.exists()) }

pub fn enable(executable: &Path, data_dir: &Path) -> Result<(), String> {
    let shortcut = shortcut_path()?;
    if let Some(parent) = shortcut.parent() { std::fs::create_dir_all(parent).map_err(|error| error.to_string())?; }
    unsafe {
        CoInitializeEx(None, COINIT_APARTMENTTHREADED).ok().map_err(|error| error.to_string())?;
        let result = (|| -> Result<(), String> {
            let link: IShellLinkW = CoCreateInstance(&ShellLink, None, CLSCTX_INPROC_SERVER)
                .map_err(|error| error.to_string())?;
            let executable_wide = wide(executable);
            link.SetPath(PCWSTR(executable_wide.as_ptr())).map_err(|error| error.to_string())?;
            let arguments = format!("run --data-dir \"{}\"", data_dir.display());
            let arguments_wide = wide(arguments);
            link.SetArguments(PCWSTR(arguments_wide.as_ptr())).map_err(|error| error.to_string())?;
            let working = executable.parent().unwrap_or_else(|| Path::new("."));
            let working_wide = wide(working);
            link.SetWorkingDirectory(PCWSTR(working_wide.as_ptr())).map_err(|error| error.to_string())?;
            let persist: IPersistFile = link.cast().map_err(|error| error.to_string())?;
            let shortcut_wide = wide(&shortcut);
            persist.Save(PCWSTR(shortcut_wide.as_ptr()), true).map_err(|error| error.to_string())?;
            Ok(())
        })();
        CoUninitialize();
        result
    }
}

pub fn disable() -> Result<(), String> {
    let path = shortcut_path()?;
    if path.exists() { std::fs::remove_file(path).map_err(|error| error.to_string())?; }
    Ok(())
}
