//! `agentgo-browser.exe`: what the Start menu shortcut starts. A windows
//! program (no console), it starts `agentgo.exe run` beside it without a
//! console window and exits. Its arguments are passed on.
#![windows_subsystem = "windows"]

fn main() {
    let Ok(exe) = std::env::current_exe() else {
        return;
    };
    let browser = exe.with_file_name("agentgo.exe");
    let mut args: Vec<std::ffi::OsString> = std::env::args_os().skip(1).collect();
    if args.first().is_none_or(|first| first != "run") {
        args.insert(0, "run".into());
    }
    let mut command = std::process::Command::new(browser);
    command.args(args).env("AGENTGO_NO_CONSOLE", "1");
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        command.creation_flags(0x0800_0000); // CREATE_NO_WINDOW
    }
    // The person launched AgentGo, so its window comes to the front. Windows
    // lets only the process the person started take the foreground; pass
    // that right on to the browser (agents' launches never come here).
    #[cfg(windows)]
    {
        #[link(name = "user32")]
        extern "system" {
            fn AllowSetForegroundWindow(process_id: u32) -> i32;
        }
        const ASFW_ANY: u32 = u32::MAX;
        // SAFETY: plain Win32 call without pointers.
        unsafe { AllowSetForegroundWindow(ASFW_ANY) };
    }
    let _ = command.spawn();
}
