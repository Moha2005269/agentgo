//! Quiet runs: execute a command on a private Win32 desktop.
//!
//! Real-browser tests open real windows. On the person's own desktop those
//! windows flash in front of their work, take the keyboard focus and receive
//! their keystrokes. A window created on a private desktop is never composed
//! to the screen, can never become the foreground window, and cannot be
//! reached by the person's mouse or keyboard, so a whole test run stays
//! invisible. Processes started by the command inherit the desktop.
//!
//! The run is also *checked*: while the command runs, every process in its job
//! is watched, and a visible top-level window of one of them on the person's
//! own desktop is reported as a leak and fails the run.

use std::io;

/// Set to `1` in the environment of a quiet command. The browser test harness
/// refuses to start a browser without it, so a test cannot be run loudly by
/// accident.
pub const QUIET_ENV: &str = "AGENTGO_QUIET_DESKTOP";

/// Exit code of a quiet run whose command put a window on the user's desktop.
pub const LEAK_EXIT_CODE: i32 = 97;

/// What a quiet run observed.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct QuietReport {
    /// Exit code of the command.
    pub exit_code: i32,
    /// Most visible top-level windows the command had on the private desktop
    /// at one time.
    pub peak_private_windows: usize,
    /// Visible top-level windows of the command found on the person's own
    /// desktop (`pid class`). Always empty for a correct quiet run.
    pub leaked: Vec<String>,
    /// Most processes the command had running at one time (zero when the job
    /// could not be tracked, in which case leaks cannot be detected either).
    pub peak_processes: usize,
}

/// Quotes one argument for a Windows command line (the inverse of
/// `CommandLineToArgvW`).
pub fn quote_arg(arg: &str) -> String {
    let plain = !arg.is_empty()
        && !arg
            .chars()
            .any(|c| matches!(c, ' ' | '\t' | '\n' | '\u{b}' | '"'));
    if plain {
        return arg.to_string();
    }
    let mut out = String::from("\"");
    let mut backslashes = 0usize;
    for c in arg.chars() {
        match c {
            '\\' => backslashes += 1,
            '"' => {
                out.extend(std::iter::repeat_n('\\', backslashes * 2 + 1));
                out.push('"');
                backslashes = 0;
            }
            _ => {
                out.extend(std::iter::repeat_n('\\', backslashes));
                out.push(c);
                backslashes = 0;
            }
        }
    }
    // Backslashes before the closing quote must not escape it.
    out.extend(std::iter::repeat_n('\\', backslashes * 2));
    out.push('"');
    out
}

/// Builds a full command line from a program and its arguments.
pub fn command_line(program: &str, args: &[String]) -> String {
    let mut line = quote_arg(program);
    for arg in args {
        line.push(' ');
        line.push_str(&quote_arg(arg));
    }
    line
}

/// Runs `program args...` on a private desktop and waits for it.
#[cfg(windows)]
pub fn run(program: &str, args: &[String]) -> io::Result<QuietReport> {
    imp::run(program, args)
}

/// Quiet runs need Win32 desktops.
#[cfg(not(windows))]
pub fn run(_program: &str, _args: &[String]) -> io::Result<QuietReport> {
    Err(io::Error::new(
        io::ErrorKind::Unsupported,
        "quiet-run uses Win32 private desktops and only works on Windows",
    ))
}

#[cfg(windows)]
mod imp {
    // Win32 desktops, jobs and process creation have no safe wrapper in the
    // standard library.
    #![allow(unsafe_code)]

    use std::ffi::{c_void, OsStr};
    use std::io;
    use std::os::windows::ffi::OsStrExt;
    use std::ptr::{null, null_mut};
    use std::time::{SystemTime, UNIX_EPOCH};

    use windows_sys::core::BOOL;
    use windows_sys::Win32::Foundation::{
        CloseHandle, GENERIC_ALL, HANDLE, HWND, INVALID_HANDLE_VALUE, LPARAM, WAIT_OBJECT_0,
        WAIT_TIMEOUT,
    };
    use windows_sys::Win32::System::Console::{
        GetStdHandle, STD_ERROR_HANDLE, STD_INPUT_HANDLE, STD_OUTPUT_HANDLE,
    };
    use windows_sys::Win32::System::JobObjects::{
        AssignProcessToJobObject, CreateJobObjectW, JobObjectBasicProcessIdList,
        JobObjectExtendedLimitInformation, QueryInformationJobObject, SetInformationJobObject,
        JOBOBJECT_EXTENDED_LIMIT_INFORMATION, JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE,
    };
    use windows_sys::Win32::System::StationsAndDesktops::{
        CloseDesktop, CreateDesktopW, EnumDesktopWindows, HDESK,
    };
    use windows_sys::Win32::System::Threading::{
        CreateProcessW, GetExitCodeProcess, ResumeThread, WaitForSingleObject, CREATE_SUSPENDED,
        CREATE_UNICODE_ENVIRONMENT, INFINITE, PROCESS_INFORMATION, STARTF_USESTDHANDLES,
        STARTUPINFOW,
    };
    use windows_sys::Win32::UI::WindowsAndMessaging::{
        EnumWindows, GetClassNameW, GetWindowThreadProcessId, IsWindowVisible,
    };

    use super::{command_line, QuietReport, QUIET_ENV};

    /// How often the watcher samples while the command runs.
    const SAMPLE_MS: u32 = 100;

    struct Handle(HANDLE);

    impl Drop for Handle {
        fn drop(&mut self) {
            if !self.0.is_null() && self.0 != INVALID_HANDLE_VALUE {
                unsafe { CloseHandle(self.0) };
            }
        }
    }

    struct Desktop(HDESK);

    impl Drop for Desktop {
        fn drop(&mut self) {
            unsafe { CloseDesktop(self.0) };
        }
    }

    fn wide(text: impl AsRef<OsStr>) -> Vec<u16> {
        text.as_ref()
            .encode_wide()
            .chain(std::iter::once(0))
            .collect()
    }

    /// The command's environment plus the quiet marker, as a UTF-16 block.
    fn environment_block() -> Vec<u16> {
        let mut block = Vec::new();
        let mut push = |key: &OsStr, value: &OsStr| {
            block.extend(key.encode_wide());
            block.push(u16::from(b'='));
            block.extend(value.encode_wide());
            block.push(0);
        };
        for (key, value) in std::env::vars_os() {
            // Windows variable names are case-insensitive.
            if !key.to_string_lossy().eq_ignore_ascii_case(QUIET_ENV) {
                push(&key, &value);
            }
        }
        push(OsStr::new(QUIET_ENV), OsStr::new("1"));
        block.push(0);
        block
    }

    /// A job that kills every process of the command when `quiet-run` ends,
    /// so a crash here never leaves browsers behind.
    fn kill_on_close_job() -> Option<Handle> {
        unsafe {
            let job = CreateJobObjectW(null(), null());
            if job.is_null() {
                return None;
            }
            let job = Handle(job);
            let mut limits: JOBOBJECT_EXTENDED_LIMIT_INFORMATION = std::mem::zeroed();
            limits.BasicLimitInformation.LimitFlags = JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE;
            let ok = SetInformationJobObject(
                job.0,
                JobObjectExtendedLimitInformation,
                (&limits as *const JOBOBJECT_EXTENDED_LIMIT_INFORMATION).cast(),
                std::mem::size_of::<JOBOBJECT_EXTENDED_LIMIT_INFORMATION>() as u32,
            );
            (ok != 0).then_some(job)
        }
    }

    #[repr(C)]
    struct ProcessIdList {
        assigned: u32,
        listed: u32,
        ids: [usize; 1024],
    }

    fn job_processes(job: HANDLE) -> Vec<u32> {
        let mut list = ProcessIdList {
            assigned: 0,
            listed: 0,
            ids: [0; 1024],
        };
        let ok = unsafe {
            QueryInformationJobObject(
                job,
                JobObjectBasicProcessIdList,
                (&mut list as *mut ProcessIdList).cast::<c_void>(),
                std::mem::size_of::<ProcessIdList>() as u32,
                null_mut(),
            )
        };
        if ok == 0 {
            return Vec::new();
        }
        let listed = (list.listed as usize).min(list.ids.len());
        list.ids[..listed].iter().map(|id| *id as u32).collect()
    }

    fn class_name(hwnd: HWND) -> String {
        let mut buffer = [0u16; 128];
        let length = unsafe { GetClassNameW(hwnd, buffer.as_mut_ptr(), buffer.len() as i32) };
        String::from_utf16_lossy(&buffer[..length.max(0) as usize])
    }

    unsafe extern "system" fn count_visible(hwnd: HWND, lparam: LPARAM) -> BOOL {
        if IsWindowVisible(hwnd) != 0 {
            *(lparam as *mut usize) += 1;
        }
        1
    }

    struct Owned<'a> {
        processes: &'a [u32],
        found: Vec<String>,
    }

    unsafe extern "system" fn collect_owned(hwnd: HWND, lparam: LPARAM) -> BOOL {
        let owned = &mut *(lparam as *mut Owned);
        if IsWindowVisible(hwnd) != 0 {
            let mut pid = 0u32;
            GetWindowThreadProcessId(hwnd, &mut pid);
            if owned.processes.contains(&pid) {
                owned.found.push(format!("pid {pid} {}", class_name(hwnd)));
            }
        }
        1
    }

    pub fn run(program: &str, args: &[String]) -> io::Result<QuietReport> {
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_or(0, |elapsed| elapsed.as_nanos());
        let name = format!("agentgo-quiet-{}-{nanos}", std::process::id());
        let desktop =
            unsafe { CreateDesktopW(wide(&name).as_ptr(), null(), null(), 0, GENERIC_ALL, null()) };
        if desktop.is_null() {
            return Err(io::Error::last_os_error());
        }
        let desktop = Desktop(desktop);
        let job = kill_on_close_job();

        let mut desktop_path = wide(format!("WinSta0\\{name}"));
        let mut line = wide(command_line(program, args));
        let environment = environment_block();
        let mut startup: STARTUPINFOW = unsafe { std::mem::zeroed() };
        startup.cb = std::mem::size_of::<STARTUPINFOW>() as u32;
        startup.lpDesktop = desktop_path.as_mut_ptr();
        startup.dwFlags = STARTF_USESTDHANDLES;
        unsafe {
            startup.hStdInput = GetStdHandle(STD_INPUT_HANDLE);
            startup.hStdOutput = GetStdHandle(STD_OUTPUT_HANDLE);
            startup.hStdError = GetStdHandle(STD_ERROR_HANDLE);
        }
        let mut info: PROCESS_INFORMATION = unsafe { std::mem::zeroed() };
        // Suspended, so the process is inside the job before it can start
        // anything of its own.
        let created = unsafe {
            CreateProcessW(
                null(),
                line.as_mut_ptr(),
                null(),
                null(),
                1,
                CREATE_SUSPENDED | CREATE_UNICODE_ENVIRONMENT,
                environment.as_ptr().cast(),
                null(),
                &startup,
                &mut info,
            )
        };
        if created == 0 {
            return Err(io::Error::last_os_error());
        }
        let process = Handle(info.hProcess);
        let thread = Handle(info.hThread);
        let tracked = job
            .as_ref()
            .is_some_and(|job| unsafe { AssignProcessToJobObject(job.0, process.0) } != 0);
        unsafe { ResumeThread(thread.0) };

        let mut report = QuietReport::default();
        loop {
            match unsafe {
                WaitForSingleObject(process.0, if tracked { SAMPLE_MS } else { INFINITE })
            } {
                WAIT_OBJECT_0 => break,
                WAIT_TIMEOUT => sample(&desktop, job.as_ref().filter(|_| tracked), &mut report),
                _ => return Err(io::Error::last_os_error()),
            }
        }
        let mut code = 0u32;
        if unsafe { GetExitCodeProcess(process.0, &mut code) } == 0 {
            return Err(io::Error::last_os_error());
        }
        report.exit_code = code as i32;
        Ok(report)
    }

    fn sample(desktop: &Desktop, job: Option<&Handle>, report: &mut QuietReport) {
        let mut visible = 0usize;
        unsafe {
            EnumDesktopWindows(
                desktop.0,
                Some(count_visible),
                (&mut visible as *mut usize) as LPARAM,
            );
        }
        report.peak_private_windows = report.peak_private_windows.max(visible);
        let Some(job) = job else { return };
        let processes = job_processes(job.0);
        report.peak_processes = report.peak_processes.max(processes.len());
        let mut owned = Owned {
            processes: &processes,
            found: Vec::new(),
        };
        unsafe { EnumWindows(Some(collect_owned), (&mut owned as *mut Owned) as LPARAM) };
        for found in owned.found {
            if !report.leaked.contains(&found) {
                report.leaked.push(found);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn plain_arguments_are_left_alone() {
        assert_eq!(quote_arg("node"), "node");
        assert_eq!(quote_arg("--flag=1"), "--flag=1");
    }

    #[test]
    fn spaces_and_quotes_are_escaped_for_the_windows_parser() {
        assert_eq!(quote_arg(""), "\"\"");
        assert_eq!(quote_arg("a b"), "\"a b\"");
        assert_eq!(quote_arg("say \"hi\""), "\"say \\\"hi\\\"\"");
        // Backslashes only need doubling when they precede a quote.
        assert_eq!(
            quote_arg("C:\\Program Files\\x\\"),
            "\"C:\\Program Files\\x\\\\\""
        );
        assert_eq!(quote_arg("a\\b c"), "\"a\\b c\"");
        assert_eq!(quote_arg("a\\\"b"), "\"a\\\\\\\"b\"");
    }

    #[test]
    fn command_line_joins_quoted_parts() {
        let line = command_line(
            "C:\\Program Files\\nodejs\\node.exe",
            &["test file.js".to_string(), "--x".to_string()],
        );
        assert_eq!(
            line,
            "\"C:\\Program Files\\nodejs\\node.exe\" \"test file.js\" --x"
        );
    }

    #[cfg(windows)]
    #[test]
    fn the_exit_code_comes_back_from_the_private_desktop() {
        let report = run(
            "cmd",
            &["/c".to_string(), "exit".to_string(), "7".to_string()],
        )
        .expect("a command runs on a private desktop");
        assert_eq!(report.exit_code, 7);
        assert!(report.leaked.is_empty(), "leaked: {:?}", report.leaked);
    }

    #[cfg(windows)]
    #[test]
    fn a_missing_program_is_an_error_not_a_hang() {
        assert!(run("agentgo-no-such-program-xyz", &[]).is_err());
    }
}
