//! The few per-user registry entries TinyHand uses: the `tinyhand://` link handler,
//! "open when you log in", and reading whether the taskbar is light or dark.

use windows::core::{w, HSTRING, PCWSTR};
use windows::Win32::Foundation::ERROR_SUCCESS;
use windows::Win32::System::Registry::*;

fn exe_path() -> Option<String> {
    std::env::current_exe().ok().map(|p| p.to_string_lossy().into_owned())
}

fn set_string(root: HKEY, key: &str, name: Option<&str>, value: &str) -> bool {
    let wide: Vec<u16> = value.encode_utf16().chain(std::iter::once(0)).collect();
    let name_h = name.map(HSTRING::from);
    let name_p = name_h.as_ref().map(|h| PCWSTR(h.as_ptr())).unwrap_or(PCWSTR::null());
    unsafe {
        RegSetKeyValueW(
            root,
            &HSTRING::from(key),
            name_p,
            REG_SZ.0,
            Some(wide.as_ptr() as *const _),
            (wide.len() * 2) as u32,
        ) == ERROR_SUCCESS
    }
}

fn get_string(root: HKEY, key: &str, name: &str) -> Option<String> {
    let mut buf = [0u16; 1024];
    let mut size = (buf.len() * 2) as u32;
    let status = unsafe {
        RegGetValueW(
            root,
            &HSTRING::from(key),
            &HSTRING::from(name),
            RRF_RT_REG_SZ,
            None,
            Some(buf.as_mut_ptr() as *mut _),
            Some(&mut size),
        )
    };
    if status != ERROR_SUCCESS {
        return None;
    }
    let len = buf.iter().position(|c| *c == 0).unwrap_or(buf.len());
    Some(String::from_utf16_lossy(&buf[..len]))
}

/// Points `tinyhand://` links at this executable, for the current user only.
/// Quietly does nothing if it is already set up.
pub fn register_url_protocol() {
    let Some(exe) = exe_path() else { return };
    let command = format!("\"{exe}\" \"%1\"");
    let key = "Software\\Classes\\tinyhand\\shell\\open\\command";
    if get_string(HKEY_CURRENT_USER, key, "").as_deref() == Some(command.as_str()) {
        return;
    }
    set_string(HKEY_CURRENT_USER, "Software\\Classes\\tinyhand", None, "URL:TinyHand");
    set_string(HKEY_CURRENT_USER, "Software\\Classes\\tinyhand", Some("URL Protocol"), "");
    set_string(HKEY_CURRENT_USER, key, None, &command);
}

const RUN_KEY: &str = "Software\\Microsoft\\Windows\\CurrentVersion\\Run";
const RUN_NAME: &str = "TinyHand";

/// True if TinyHand (this copy of it) starts when the user logs in.
pub fn launch_at_login() -> bool {
    match (get_string(HKEY_CURRENT_USER, RUN_KEY, RUN_NAME), exe_path()) {
        (Some(value), Some(exe)) => value.to_lowercase().contains(&exe.to_lowercase()),
        _ => false,
    }
}

pub fn set_launch_at_login(on: bool) -> bool {
    if on {
        match exe_path() {
            Some(exe) => set_string(HKEY_CURRENT_USER, RUN_KEY, Some(RUN_NAME), &format!("\"{exe}\"")),
            None => false,
        }
    } else {
        unsafe { RegDeleteKeyValueW(HKEY_CURRENT_USER, &HSTRING::from(RUN_KEY), &HSTRING::from(RUN_NAME)) == ERROR_SUCCESS }
    }
}

fn personalize_flag(name: PCWSTR) -> Option<bool> {
    let mut value = 0u32;
    let mut size = 4u32;
    let status = unsafe {
        RegGetValueW(
            HKEY_CURRENT_USER,
            w!("Software\\Microsoft\\Windows\\CurrentVersion\\Themes\\Personalize"),
            name,
            RRF_RT_REG_DWORD,
            None,
            Some(&mut value as *mut u32 as *mut _),
            Some(&mut size),
        )
    };
    (status == ERROR_SUCCESS).then_some(value != 0)
}

/// True when the taskbar uses the light theme, so the tray icon should be dark.
pub fn taskbar_is_light() -> bool {
    personalize_flag(w!("SystemUsesLightTheme")).unwrap_or(false)
}

/// True when apps use the light theme.
pub fn apps_use_light_theme() -> bool {
    personalize_flag(w!("AppsUseLightTheme")).unwrap_or(true)
}
