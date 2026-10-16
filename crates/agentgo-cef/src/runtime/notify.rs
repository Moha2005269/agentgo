//! Operator attention: taskbar flash plus a Windows notification, used when a
//! page needs the human (CAPTCHA, login, approval).

use cef::*;
use windows_sys::Win32::System::LibraryLoader::GetModuleHandleW;
use windows_sys::Win32::UI::Shell::{
    Shell_NotifyIconW, NIF_ICON, NIF_INFO, NIF_MESSAGE, NIF_TIP, NIIF_LARGE_ICON, NIIF_USER,
    NIM_ADD, NIM_DELETE, NIM_MODIFY, NOTIFYICONDATAW,
};
use windows_sys::Win32::UI::WindowsAndMessaging::{
    FlashWindowEx, GetSystemMetrics, LoadIconW, LoadImageW, FLASHWINFO, FLASHW_TIMERNOFG,
    FLASHW_TRAY, IDI_APPLICATION, IMAGE_ICON, SM_CXICON, SM_CXSMICON,
};

const ICON_ID: u32 = 0xA6;
/// Message the notification icon sends to the window when it is clicked
/// (WM_APP + 0xA6); handled by the window procedure in `guard.rs`.
pub const CLICKED: u32 = 0x8000 + 0xA6;
/// Resource id of the AgentGo icon that `agentgo-app`'s build script embeds.
const APP_ICON_RESOURCE: usize = 1;

/// The AgentGo mark from the executable's resources at a given size, or the
/// generic icon if the binary was built without it.
fn app_icon(size: i32) -> windows_sys::Win32::UI::WindowsAndMessaging::HICON {
    unsafe {
        let module = GetModuleHandleW(std::ptr::null());
        let icon = LoadImageW(
            module,
            APP_ICON_RESOURCE as *const u16,
            IMAGE_ICON,
            size,
            size,
            0,
        );
        if icon.is_null() {
            LoadIconW(std::ptr::null_mut(), IDI_APPLICATION)
        } else {
            icon as _
        }
    }
}

fn copy_wide(target: &mut [u16], text: &str) {
    let limit = target.len() - 1;
    for (slot, unit) in target.iter_mut().zip(text.encode_utf16().take(limit)) {
        *slot = unit;
    }
}

fn icon_data(hwnd: windows_sys::Win32::Foundation::HWND) -> NOTIFYICONDATAW {
    let mut data: NOTIFYICONDATAW = unsafe { std::mem::zeroed() };
    data.cbSize = std::mem::size_of::<NOTIFYICONDATAW>() as u32;
    data.hWnd = hwnd;
    data.uID = ICON_ID;
    data
}

/// Flashes the taskbar button until the window is focused and shows a
/// notification with the reason. No content from the page is included
/// beyond the short, runtime-authored reason string.
pub fn attention(window: &Window, message: &str) {
    // Automated runs never flash the taskbar or raise notifications.
    if std::env::var_os("AGENTGO_NO_NOTIFY").is_some() {
        return;
    }
    let hwnd = window.window_handle().0 as windows_sys::Win32::Foundation::HWND;
    if hwnd.is_null() {
        return;
    }
    unsafe {
        let flash = FLASHWINFO {
            cbSize: std::mem::size_of::<FLASHWINFO>() as u32,
            hwnd,
            dwFlags: FLASHW_TRAY | FLASHW_TIMERNOFG,
            uCount: 0,
            dwTimeout: 0,
        };
        FlashWindowEx(&flash);

        let mut data = icon_data(hwnd);
        data.uFlags = NIF_ICON | NIF_TIP | NIF_INFO | NIF_MESSAGE;
        data.uCallbackMessage = CLICKED;
        data.hIcon = app_icon(GetSystemMetrics(SM_CXSMICON));
        // The notification shows the AgentGo mark, not Windows' info icon.
        data.hBalloonIcon = app_icon(GetSystemMetrics(SM_CXICON) * 2);
        copy_wide(&mut data.szTip, "AgentGo");
        copy_wide(&mut data.szInfoTitle, "AgentGo needs you");
        copy_wide(&mut data.szInfo, message);
        data.dwInfoFlags = NIIF_USER | NIIF_LARGE_ICON;
        // The icon exists only to carry the notification; reuse it if present.
        if Shell_NotifyIconW(NIM_MODIFY, &data) == 0 {
            Shell_NotifyIconW(NIM_ADD, &data);
        }
    }
}

/// The identity Windows shows on AgentGo's notifications (and groups its
/// windows under on the taskbar).
const APP_ID: &str = "AgentGo.Browser";

/// Gives this process AgentGo's own identity, registered for the current user
/// with a name and icon, so a notification's header shows the AgentGo mark
/// instead of a generic picture. Without it Windows has no icon for the ID
/// Chromium uses. Call before the first window is created.
pub fn register_identity() {
    use windows_sys::Win32::System::Registry::{
        RegCloseKey, RegCreateKeyExW, RegSetValueExW, HKEY, HKEY_CURRENT_USER, KEY_SET_VALUE,
        REG_SZ,
    };
    use windows_sys::Win32::UI::Shell::SetCurrentProcessExplicitAppUserModelID;
    let wide = |text: &str| text.encode_utf16().chain([0]).collect::<Vec<u16>>();
    // The icon must be a file; the per-user AgentGo folder outlives any run.
    let Some(dir) =
        std::env::var_os("LOCALAPPDATA").map(|base| std::path::Path::new(&base).join("AgentGo"))
    else {
        return;
    };
    let icon = dir.join("agentgo.ico");
    const ICON: &[u8] = include_bytes!("../../../agentgo-app/assets/agentgo.ico");
    if std::fs::read(&icon).ok().as_deref() != Some(ICON)
        && (std::fs::create_dir_all(&dir).is_err() || std::fs::write(&icon, ICON).is_err())
    {
        return;
    }
    let key_path = wide(&format!("Software\\Classes\\AppUserModelId\\{APP_ID}"));
    // SAFETY: plain registry calls on a key this function opens and closes;
    // every string passed is NUL-terminated UTF-16 that outlives the call.
    unsafe {
        let mut key: HKEY = std::ptr::null_mut();
        if RegCreateKeyExW(
            HKEY_CURRENT_USER,
            key_path.as_ptr(),
            0,
            std::ptr::null(),
            0,
            KEY_SET_VALUE,
            std::ptr::null(),
            &mut key,
            std::ptr::null_mut(),
        ) == 0
        {
            for (name, value) in [
                ("DisplayName", "AgentGo".to_string()),
                ("IconUri", icon.to_string_lossy().into_owned()),
            ] {
                let value = wide(&value);
                RegSetValueExW(
                    key,
                    wide(name).as_ptr(),
                    0,
                    REG_SZ,
                    value.as_ptr().cast(),
                    (value.len() * 2) as u32,
                );
            }
            RegCloseKey(key);
        }
        SetCurrentProcessExplicitAppUserModelID(wide(APP_ID).as_ptr());
    }
}

/// Removes the notification icon at shutdown.
pub fn clear(window: &Window) {
    let hwnd = window.window_handle().0 as windows_sys::Win32::Foundation::HWND;
    if !hwnd.is_null() {
        unsafe { Shell_NotifyIconW(NIM_DELETE, &icon_data(hwnd)) };
    }
}
