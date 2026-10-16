//! Window probes for the presentation tests.
//!
//! The browser promises to open behind the person's own windows without
//! taking the focus. Proving that needs an independent observer: the order of
//! the top-level windows, which window is active in each input queue, and a
//! stand-in for the person's window to be in front of. Both run inside a
//! `quiet-run` private desktop, so nothing here can touch the real screen.

use serde_json::Value;

/// Lists the visible and hidden top-level windows of the caller's desktop from
/// the front of the z-order to the back, as JSON.
#[cfg(windows)]
pub fn windows_json() -> Value {
    imp::windows_json()
}

/// Opens a plain window titled `title`, brings it to the front and serves its
/// message loop until the process is stopped. `cover` stretches it over the
/// whole screen so that any window behind it is completely occluded.
#[cfg(windows)]
pub fn run_decoy(title: &str, cover: bool) -> std::io::Result<()> {
    imp::run_decoy(title, cover)
}

/// Prints one line per change of visibility, activation, focus or stacking of
/// the windows of `class`, stamped with the wall clock in milliseconds (the
/// same clock the browser's own log lines use), for `seconds` seconds.
#[cfg(windows)]
pub fn watch(seconds: u64, class: &str) {
    use std::collections::HashMap;
    use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};
    let end = Instant::now() + Duration::from_secs(seconds);
    let mut seen: HashMap<i64, String> = HashMap::new();
    while Instant::now() < end {
        let listing = imp::windows_json();
        let empty = Vec::new();
        for window in listing["windows"].as_array().unwrap_or(&empty) {
            if window["class"].as_str() != Some(class) {
                continue;
            }
            let handle = window["hwnd"].as_i64().unwrap_or(0);
            let state = format!(
                "pid={} visible={} active={} focus={} z={} noActivate={} appWindow={} title={:?}",
                window["pid"],
                window["visible"],
                window["activeInQueue"],
                window["focusInQueue"],
                window["z"],
                window["noActivate"],
                window["appWindow"],
                window["title"].as_str().unwrap_or("")
            );
            if seen.get(&handle) != Some(&state) {
                let now = SystemTime::now()
                    .duration_since(UNIX_EPOCH)
                    .map(|elapsed| elapsed.as_millis())
                    .unwrap_or(0);
                println!("{now} WATCH hwnd={handle} {state}");
                seen.insert(handle, state);
            }
        }
        std::thread::sleep(Duration::from_millis(4));
    }
}

#[cfg(not(windows))]
pub fn watch(_seconds: u64, _class: &str) {
    eprintln!("window probes only work on Windows");
}

/// Probes need Win32 windows.
#[cfg(not(windows))]
pub fn windows_json() -> Value {
    serde_json::json!({"error": "window probes only work on Windows"})
}

/// Probes need Win32 windows.
#[cfg(not(windows))]
pub fn run_decoy(_title: &str, _cover: bool) -> std::io::Result<()> {
    Err(std::io::Error::new(
        std::io::ErrorKind::Unsupported,
        "window probes only work on Windows",
    ))
}

#[cfg(windows)]
mod imp {
    // Win32 window enumeration and creation have no safe wrapper in the
    // standard library.
    #![allow(unsafe_code)]

    use std::io::Write;
    use std::ptr::{null, null_mut};

    use serde_json::{json, Value};
    use windows_sys::core::BOOL;
    use windows_sys::Win32::Foundation::{HWND, LPARAM, LRESULT, RECT, WPARAM};
    use windows_sys::Win32::Graphics::Gdi::{COLOR_WINDOW, HBRUSH};
    use windows_sys::Win32::System::LibraryLoader::GetModuleHandleW;
    use windows_sys::Win32::UI::WindowsAndMessaging::{
        CreateWindowExW, DefWindowProcW, DispatchMessageW, EnumWindows, GetClassNameW,
        GetForegroundWindow, GetGUIThreadInfo, GetMessageW, GetSystemMetrics, GetWindow,
        GetWindowLongPtrW, GetWindowRect, GetWindowTextW, GetWindowThreadProcessId, IsIconic,
        IsWindowVisible, LoadCursorW, PostQuitMessage, RegisterClassW, SetForegroundWindow,
        ShowWindow, TranslateMessage, GUITHREADINFO, GWL_EXSTYLE, GW_OWNER, IDC_ARROW, MSG,
        SM_CXSCREEN, SM_CYSCREEN, SW_SHOW, WM_DESTROY, WNDCLASSW, WS_EX_APPWINDOW,
        WS_EX_NOACTIVATE, WS_EX_TOOLWINDOW, WS_EX_TOPMOST, WS_OVERLAPPEDWINDOW, WS_POPUP,
        WS_VISIBLE,
    };

    fn wide(text: &str) -> Vec<u16> {
        text.encode_utf16().chain(std::iter::once(0)).collect()
    }

    fn read_text(fill: impl Fn(*mut u16, i32) -> i32) -> String {
        let mut buffer = [0_u16; 256];
        let length = fill(buffer.as_mut_ptr(), buffer.len() as i32).max(0) as usize;
        String::from_utf16_lossy(&buffer[..length.min(buffer.len())])
    }

    unsafe extern "system" fn collect(hwnd: HWND, list: LPARAM) -> BOOL {
        // SAFETY: `list` is the `&mut Vec<HWND>` that `windows_json` passed.
        let windows = unsafe { &mut *(list as *mut Vec<HWND>) };
        windows.push(hwnd);
        1
    }

    pub fn windows_json() -> Value {
        let mut handles: Vec<HWND> = Vec::new();
        // SAFETY: the callback only pushes into the vector that outlives the call.
        unsafe { EnumWindows(Some(collect), &mut handles as *mut Vec<HWND> as LPARAM) };
        let windows: Vec<Value> = handles
            .iter()
            .enumerate()
            .map(|(z, &hwnd)| describe(z, hwnd))
            .collect();
        json!({ "windows": windows })
    }

    fn describe(z: usize, hwnd: HWND) -> Value {
        let mut process = 0_u32;
        // SAFETY: `hwnd` came from EnumWindows a moment ago; a window that
        // vanished since makes the calls fail, which yields empty values.
        let thread = unsafe { GetWindowThreadProcessId(hwnd, &mut process) };
        let title = read_text(|buffer, size| unsafe { GetWindowTextW(hwnd, buffer, size) });
        let class = read_text(|buffer, size| unsafe { GetClassNameW(hwnd, buffer, size) });
        let mut rect = RECT {
            left: 0,
            top: 0,
            right: 0,
            bottom: 0,
        };
        // SAFETY: `rect` is a valid out parameter.
        unsafe { GetWindowRect(hwnd, &mut rect) };
        let styles = unsafe { GetWindowLongPtrW(hwnd, GWL_EXSTYLE) } as u32;
        let mut gui: GUITHREADINFO = unsafe { std::mem::zeroed() };
        gui.cbSize = std::mem::size_of::<GUITHREADINFO>() as u32;
        let have_gui = unsafe { GetGUIThreadInfo(thread, &mut gui) } != 0;
        let visible = unsafe { IsWindowVisible(hwnd) } != 0;
        let owned = !unsafe { GetWindow(hwnd, GW_OWNER) }.is_null();
        let tool_window = styles & WS_EX_TOOLWINDOW != 0;
        let app_window = styles & WS_EX_APPWINDOW != 0;
        let no_activate = styles & WS_EX_NOACTIVATE != 0;
        json!({
            "z": z,
            "hwnd": hwnd as isize,
            "title": title,
            "class": class,
            "pid": process,
            "visible": visible,
            "minimized": unsafe { IsIconic(hwnd) } != 0,
            "topmost": styles & WS_EX_TOPMOST != 0,
            "toolWindow": tool_window,
            "appWindow": app_window,
            "noActivate": no_activate,
            "owned": owned,
            // The shell's rule for a taskbar button and an Alt+Tab entry: a
            // visible window that is not a tool window, has no owner (or says
            // it is an app window), and is not a no-activate window (unless
            // it says it is an app window).
            "taskbar": visible && !tool_window && (!owned || app_window) && (!no_activate || app_window),
            "rect": {"left": rect.left, "top": rect.top, "right": rect.right, "bottom": rect.bottom},
            // The window the person is using (the system-wide foreground).
            "foreground": unsafe { GetForegroundWindow() } == hwnd,
            // Active window of the owning input queue, and its keyboard focus.
            // On a desktop shared with other programs a background program
            // still has an active window in its own queue; only "foreground"
            // says what the person sees as active.
            "activeInQueue": have_gui && gui.hwndActive == hwnd,
            "focusInQueue": have_gui && gui.hwndFocus == hwnd,
        })
    }

    unsafe extern "system" fn decoy_proc(
        hwnd: HWND,
        message: u32,
        wparam: WPARAM,
        lparam: LPARAM,
    ) -> LRESULT {
        if message == WM_DESTROY {
            // SAFETY: always valid on the thread that owns the window.
            unsafe { PostQuitMessage(0) };
            return 0;
        }
        // SAFETY: forwards the message this procedure received, unchanged.
        unsafe { DefWindowProcW(hwnd, message, wparam, lparam) }
    }

    pub fn run_decoy(title: &str, cover: bool) -> std::io::Result<()> {
        let class = wide("AgentGoProbeDecoy");
        let title = wide(title);
        // SAFETY: standard window-class registration and window creation; every
        // pointer refers to a buffer that outlives the call.
        unsafe {
            let instance = GetModuleHandleW(null());
            let class_info = WNDCLASSW {
                style: 0,
                lpfnWndProc: Some(decoy_proc),
                cbClsExtra: 0,
                cbWndExtra: 0,
                hInstance: instance,
                hIcon: null_mut(),
                hCursor: LoadCursorW(null_mut(), IDC_ARROW),
                hbrBackground: (COLOR_WINDOW + 1) as isize as HBRUSH,
                lpszMenuName: null(),
                lpszClassName: class.as_ptr(),
            };
            if RegisterClassW(&class_info) == 0 {
                return Err(std::io::Error::last_os_error());
            }
            let (style, x, y, width, height) = if cover {
                (
                    WS_POPUP | WS_VISIBLE,
                    0,
                    0,
                    GetSystemMetrics(SM_CXSCREEN),
                    GetSystemMetrics(SM_CYSCREEN),
                )
            } else {
                (WS_OVERLAPPEDWINDOW | WS_VISIBLE, 40, 40, 700, 500)
            };
            let hwnd = CreateWindowExW(
                0,
                class.as_ptr(),
                title.as_ptr(),
                style,
                x,
                y,
                width,
                height,
                null_mut(),
                null_mut(),
                instance,
                null(),
            );
            if hwnd.is_null() {
                return Err(std::io::Error::last_os_error());
            }
            ShowWindow(hwnd, SW_SHOW);
            // Best effort: a freshly created visible window is already the
            // active window of its own queue on this desktop.
            SetForegroundWindow(hwnd);
            println!("decoy-ready {}", hwnd as isize);
            let _ = std::io::stdout().flush();
            let mut message: MSG = std::mem::zeroed();
            while GetMessageW(&mut message, null_mut(), 0, 0) > 0 {
                TranslateMessage(&message);
                DispatchMessageW(&message);
            }
        }
        Ok(())
    }
}
