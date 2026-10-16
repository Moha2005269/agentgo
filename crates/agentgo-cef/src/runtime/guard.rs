//! Activation guard: the browser window never takes the foreground by itself.
//!
//! Chromium activates its window in many places (first show, page focus, tab
//! switches, dialogs), and an agent's work triggers all of them. A thread-level
//! CBT hook is the one documented place where an activation can be refused, so
//! every activation of a window of this thread passes through [`permitted`]:
//! it goes ahead only when the person asked for it.

use std::sync::atomic::{AtomicBool, Ordering};

static ALLOW: AtomicBool = AtomicBool::new(false);
/// Runs `action` with activation allowed. For the one case where the browser
/// itself may come to the front: a launch the person made on purpose.
pub fn allowing<R>(action: impl FnOnce() -> R) -> R {
    ALLOW.store(true, Ordering::SeqCst);
    let result = action();
    ALLOW.store(false, Ordering::SeqCst);
    result
}

/// Lets activations through for `ms` milliseconds. A launch by the person is
/// followed by activations that arrive later (Chromium focuses the page after
/// the window is shown), and they belong to the same request.
#[cfg(target_os = "windows")]
pub fn allow_for(ms: u64) {
    imp::allow_for(ms);
}

#[cfg(not(target_os = "windows"))]
pub fn allow_for(_ms: u64) {}

/// Installs the guard on the calling (CEF UI) thread for the process lifetime.
#[cfg(target_os = "windows")]
pub fn install() {
    use windows_sys::Win32::System::Threading::GetCurrentThreadId;
    use windows_sys::Win32::UI::WindowsAndMessaging::{SetWindowsHookExW, WH_CBT};
    // SAFETY: `hook` is a plain function that lives as long as the process,
    // and the hook is limited to this thread.
    let hook = unsafe {
        SetWindowsHookExW(
            WH_CBT,
            Some(imp::hook),
            std::ptr::null_mut(),
            GetCurrentThreadId(),
        )
    };
    if hook.is_null() {
        super::state::log_event("AG-RUNTIME activation-guard-failed");
    }
    // SAFETY: as above; this hook only reads the message and forwards it.
    let mouse = unsafe {
        SetWindowsHookExW(
            windows_sys::Win32::UI::WindowsAndMessaging::WH_GETMESSAGE,
            Some(imp::mouse_hook),
            std::ptr::null_mut(),
            GetCurrentThreadId(),
        )
    };
    if mouse.is_null() {
        super::state::log_event("AG-RUNTIME mouse-hook-failed");
    }
    // The two low-level hooks only note when the person used Alt+Tab, the
    // Windows key or the taskbar; they run on this thread's message loop.
    for kind in [
        windows_sys::Win32::UI::WindowsAndMessaging::WH_KEYBOARD_LL,
        windows_sys::Win32::UI::WindowsAndMessaging::WH_MOUSE_LL,
    ] {
        let procedure: windows_sys::Win32::UI::WindowsAndMessaging::HOOKPROC =
            if kind == windows_sys::Win32::UI::WindowsAndMessaging::WH_KEYBOARD_LL {
                Some(imp::keyboard_ll)
            } else {
                Some(imp::mouse_ll)
            };
        // SAFETY: the procedures are plain functions that live for the process.
        let hook = unsafe { SetWindowsHookExW(kind, procedure, std::ptr::null_mut(), 0) };
        if hook.is_null() {
            super::state::log_event("AG-RUNTIME input-hook-failed");
        }
    }
}

#[cfg(not(target_os = "windows"))]
pub fn install() {}

/// Keeps `hwnd` (the browser's top-level window) where it is in the stacking
/// order unless the person brings it forward. Refusing activation is not
/// enough: a window can be raised above the others without being activated.
#[cfg(target_os = "windows")]
pub fn protect(hwnd: windows_sys::Win32::Foundation::HWND) {
    use windows_sys::Win32::UI::WindowsAndMessaging::{SetWindowLongPtrW, GWLP_WNDPROC};
    // SAFETY: `hwnd` is a live window of this thread; `imp::window_proc`
    // forwards every message to the procedure it replaces.
    let previous =
        unsafe { SetWindowLongPtrW(hwnd, GWLP_WNDPROC, imp::window_proc as *const () as isize) };
    imp::PREVIOUS_PROC.store(previous, Ordering::SeqCst);
}

#[cfg(not(target_os = "windows"))]
pub fn protect<T>(_window: T) {}

#[cfg(target_os = "windows")]
mod imp {
    use super::{Ordering, ALLOW};
    use std::sync::atomic::{AtomicIsize, AtomicU64};
    use windows_sys::Win32::Foundation::{CloseHandle, HWND, LPARAM, LRESULT, WPARAM};
    use windows_sys::Win32::System::SystemInformation::GetTickCount64;
    use windows_sys::Win32::System::Threading::{
        GetCurrentProcessId, OpenProcess, QueryFullProcessImageNameW,
        PROCESS_QUERY_LIMITED_INFORMATION,
    };
    use windows_sys::Win32::UI::Input::KeyboardAndMouse::{GetAsyncKeyState, VK_MENU};
    use windows_sys::Win32::UI::WindowsAndMessaging::{
        CallNextHookEx, CallWindowProcW, GetAncestor, GetForegroundWindow,
        GetWindowThreadProcessId, WindowFromPoint, CBTACTIVATESTRUCT, GA_ROOT, HCBT_ACTIVATE,
        KBDLLHOOKSTRUCT, MSLLHOOKSTRUCT, SWP_NOZORDER, WINDOWPOS, WM_WINDOWPOSCHANGING,
    };

    /// The window procedure [`super::protect`] replaced.
    pub static PREVIOUS_PROC: AtomicIsize = AtomicIsize::new(0);
    /// Tick count of the last activation the guard let through.
    static PERMITTED_AT: AtomicU64 = AtomicU64::new(0);
    /// End of the launch grace period (tick count).
    static GRACE_UNTIL: AtomicU64 = AtomicU64::new(0);

    pub fn allow_for(ms: u64) {
        // SAFETY: no preconditions.
        GRACE_UNTIL.store(unsafe { GetTickCount64() } + ms, Ordering::SeqCst);
    }

    fn in_grace() -> bool {
        // SAFETY: no preconditions.
        let now = unsafe { GetTickCount64() };
        now < GRACE_UNTIL.load(Ordering::SeqCst)
    }
    /// Tick count of the last real Alt, Tab or Windows key event.
    static LAST_SWITCH_KEY: AtomicU64 = AtomicU64::new(0);
    /// Tick count of the last real left click on a shell (taskbar) window.
    static LAST_SHELL_CLICK: AtomicU64 = AtomicU64::new(0);
    /// How long after such an input an activation still counts as the person's.
    const PERSON_WINDOW_MS: u64 = 1_500;

    /// Whether the person just used Alt+Tab, the Windows key or the taskbar.
    /// Only physical input counts: an agent works through DevTools and never
    /// produces these, so it cannot borrow this to push the window forward.
    fn person_switched() -> bool {
        // SAFETY: no preconditions.
        let now = unsafe { GetTickCount64() };
        let recent = |tick: &AtomicU64| {
            let at = tick.load(Ordering::SeqCst);
            at != 0 && now.saturating_sub(at) < PERSON_WINDOW_MS
        };
        // SAFETY: no preconditions; the high bit is set while the key is down.
        let alt_down = unsafe { GetAsyncKeyState(i32::from(VK_MENU)) } < 0;
        alt_down || recent(&LAST_SWITCH_KEY) || recent(&LAST_SHELL_CLICK)
    }

    pub unsafe extern "system" fn keyboard_ll(
        code: i32,
        wparam: WPARAM,
        lparam: LPARAM,
    ) -> LRESULT {
        if code >= 0 {
            // SAFETY: for WH_KEYBOARD_LL the system passes a KBDLLHOOKSTRUCT.
            let key = unsafe { &*(lparam as *const KBDLLHOOKSTRUCT) };
            let injected = key.flags & 0x10 != 0;
            // Tab, Alt, left and right Alt, left and right Windows key.
            let switching = matches!(key.vkCode, 0x09 | 0x12 | 0xA4 | 0xA5 | 0x5B | 0x5C);
            if switching && !injected {
                // SAFETY: no preconditions.
                LAST_SWITCH_KEY.store(unsafe { GetTickCount64() }, Ordering::SeqCst);
            }
        }
        // SAFETY: forwards the unchanged hook arguments.
        unsafe { CallNextHookEx(std::ptr::null_mut(), code, wparam, lparam) }
    }

    pub unsafe extern "system" fn mouse_ll(code: i32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
        const WM_LBUTTONDOWN: usize = 0x0201;
        if code >= 0 && wparam == WM_LBUTTONDOWN {
            // SAFETY: for WH_MOUSE_LL the system passes an MSLLHOOKSTRUCT.
            let mouse = unsafe { &*(lparam as *const MSLLHOOKSTRUCT) };
            if mouse.flags & 1 == 0 {
                // SAFETY: plain queries on the window under the pointer.
                let root = unsafe { GetAncestor(WindowFromPoint(mouse.pt), GA_ROOT) };
                if !root.is_null() && shell_window(root) {
                    // SAFETY: no preconditions.
                    LAST_SHELL_CLICK.store(unsafe { GetTickCount64() }, Ordering::SeqCst);
                }
            }
        }
        // SAFETY: forwards the unchanged hook arguments.
        unsafe { CallNextHookEx(std::ptr::null_mut(), code, wparam, lparam) }
    }

    /// Whether `window` belongs to the Windows shell (taskbar, previews).
    fn shell_window(window: HWND) -> bool {
        process_of(window).is_some_and(|(_, name)| SHELL.contains(&name.as_str()))
    }

    /// A permitted activation restacks the window a moment later.
    const RESTACK_WINDOW_MS: u64 = 1_000;

    pub unsafe extern "system" fn window_proc(
        hwnd: HWND,
        message: u32,
        wparam: WPARAM,
        lparam: LPARAM,
    ) -> LRESULT {
        if message == super::super::notify::CLICKED {
            // The person clicked AgentGo's notification (or its tray icon):
            // that is a request to see the tab that needs them.
            const NIN_SELECT: u32 = 0x400;
            const NIN_BALLOONUSERCLICK: u32 = 0x405;
            const WM_LBUTTONUP: u32 = 0x202;
            if matches!(
                (lparam & 0xFFFF) as u32,
                NIN_SELECT | NIN_BALLOONUSERCLICK | WM_LBUTTONUP
            ) {
                super::allowing(super::super::state::show_waiting_tab);
            }
            return 0;
        }
        if message == WM_WINDOWPOSCHANGING {
            // SAFETY: for this message the system passes a WINDOWPOS it owns.
            let position = unsafe { &mut *(lparam as *mut WINDOWPOS) };
            if position.flags & SWP_NOZORDER == 0 && !may_restack() {
                position.flags |= SWP_NOZORDER;
            }
        }
        // SAFETY: `PREVIOUS_PROC` is the procedure this one replaced, stored
        // before any message can arrive here.
        unsafe {
            let previous = std::mem::transmute::<
                isize,
                windows_sys::Win32::UI::WindowsAndMessaging::WNDPROC,
            >(PREVIOUS_PROC.load(Ordering::SeqCst));
            CallWindowProcW(previous, hwnd, message, wparam, lparam)
        }
    }

    fn may_restack() -> bool {
        // SAFETY: no preconditions.
        let (now, own) = unsafe { (GetTickCount64(), GetCurrentProcessId()) };
        ALLOW.load(Ordering::SeqCst)
            || in_grace()
            || person_switched()
            || now.saturating_sub(PERMITTED_AT.load(Ordering::SeqCst)) < RESTACK_WINDOW_MS
            || foreground_process().is_some_and(|(pid, _)| pid == own)
    }

    /// Windows shell processes. When one of them holds the foreground and a
    /// window of ours is activated, the person picked it: taskbar button,
    /// Alt+Tab, Task View, Start, or a notification.
    const SHELL: [&str; 4] = [
        "explorer.exe",
        "shellexperiencehost.exe",
        "startmenuexperiencehost.exe",
        "searchhost.exe",
    ];

    pub unsafe extern "system" fn mouse_hook(code: i32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
        const WM_LBUTTONDOWN: u32 = 0x0201;
        if code >= 0 {
            // SAFETY: for WH_GETMESSAGE the system passes a pointer to the MSG.
            let msg =
                unsafe { &*(lparam as *const windows_sys::Win32::UI::WindowsAndMessaging::MSG) };
            if msg.message == WM_LBUTTONDOWN {
                super::super::state::page_mouse_down(msg.hwnd as isize, (msg.pt.x, msg.pt.y));
            }
        }
        // SAFETY: forwards the unchanged hook arguments.
        unsafe { CallNextHookEx(std::ptr::null_mut(), code, wparam, lparam) }
    }

    pub unsafe extern "system" fn hook(code: i32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
        if code == HCBT_ACTIVATE as i32 {
            // SAFETY: for HCBT_ACTIVATE the system passes a CBTACTIVATESTRUCT.
            let by_mouse = unsafe { (*(lparam as *const CBTACTIVATESTRUCT)).fMouse } != 0;
            if !permitted(by_mouse, foreground_process()) {
                return 1; // refuse: the window stays where it is, inactive
            }
            // SAFETY: no preconditions.
            PERMITTED_AT.store(unsafe { GetTickCount64() }, Ordering::SeqCst);
        }
        // SAFETY: forwards the unchanged hook arguments.
        unsafe { CallNextHookEx(std::ptr::null_mut(), code, wparam, lparam) }
    }

    /// (process id, lower-case executable name) of the foreground window.
    fn foreground_process() -> Option<(u32, String)> {
        // SAFETY: no preconditions.
        process_of(unsafe { GetForegroundWindow() })
    }

    /// (process id, lower-case executable name) of the process owning `window`.
    fn process_of(window: HWND) -> Option<(u32, String)> {
        // SAFETY: read-only queries; the handle is closed before returning.
        unsafe {
            let foreground = window;
            if foreground.is_null() {
                return None;
            }
            let mut pid = 0_u32;
            GetWindowThreadProcessId(foreground, &mut pid);
            let process = OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, 0, pid);
            let mut name = String::new();
            if !process.is_null() {
                let mut buffer = [0_u16; 520];
                let mut length = buffer.len() as u32;
                if QueryFullProcessImageNameW(process, 0, buffer.as_mut_ptr(), &mut length) != 0 {
                    let path = String::from_utf16_lossy(&buffer[..length as usize]);
                    name = path.rsplit('\\').next().unwrap_or("").to_ascii_lowercase();
                }
                CloseHandle(process);
            }
            Some((pid, name))
        }
    }

    fn permitted(by_mouse: bool, foreground: Option<(u32, String)>) -> bool {
        // SAFETY: no preconditions.
        let own = unsafe { GetCurrentProcessId() };
        decide(
            ALLOW.load(Ordering::SeqCst) || in_grace() || person_switched(),
            by_mouse,
            foreground,
            own,
        )
    }

    /// The whole policy. A click on the window, a window of ours that already
    /// is in front, or the Windows shell handing us the foreground: yes.
    /// Anything else is the browser pushing itself forward: no.
    pub fn decide(
        allow: bool,
        by_mouse: bool,
        foreground: Option<(u32, String)>,
        own: u32,
    ) -> bool {
        allow
            || by_mouse
            || foreground.is_some_and(|(pid, name)| pid == own || SHELL.contains(&name.as_str()))
    }

    #[cfg(test)]
    mod tests {
        use super::decide;

        #[test]
        fn only_the_person_can_bring_the_window_forward() {
            let other = Some((7, "code.exe".to_string()));
            assert!(
                !decide(false, false, other.clone(), 1),
                "self-activation over another app"
            );
            assert!(!decide(false, false, None, 1), "no foreground window");
            assert!(
                decide(false, true, other.clone(), 1),
                "mouse click on the window"
            );
            assert!(
                decide(false, false, Some((1, "agentgo.exe".into())), 1),
                "already ours"
            );
            assert!(
                decide(false, false, Some((9, "explorer.exe".into())), 1),
                "taskbar / Alt+Tab"
            );
            assert!(decide(true, false, other, 1), "explicit foreground launch");
        }
    }
}
