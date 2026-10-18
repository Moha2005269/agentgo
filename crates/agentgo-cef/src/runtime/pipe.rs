//! Authenticated Windows named-pipe transport.
//!
//! `\\.\pipe\agentgo-<instance id>` accepts local clients only, is ACL'd to
//! the current Windows user, and requires the instance token in the first
//! message. Every connection becomes its own agent session with its own tab
//! leases. The pipe opens no TCP port (the optional localhost SDK server,
//! off by default, lives in `sdk.rs`).

use std::io;
use std::path::Path;
use std::ptr::{null, null_mut};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::mpsc;

use agentgo_protocol::{ErrorCode, FrameConfig, FrameDecoder, Request};
use serde_json::json;
use windows_sys::Win32::Foundation::{
    CloseHandle, GetLastError, LocalFree, ERROR_IO_PENDING, ERROR_PIPE_CONNECTED, HANDLE,
    INVALID_HANDLE_VALUE,
};
use windows_sys::Win32::Security::Authorization::{
    ConvertSidToStringSidW, ConvertStringSecurityDescriptorToSecurityDescriptorW,
};
use windows_sys::Win32::Security::{
    GetTokenInformation, TokenUser, PSECURITY_DESCRIPTOR, SECURITY_ATTRIBUTES, TOKEN_QUERY,
    TOKEN_USER,
};
use windows_sys::Win32::Storage::FileSystem::{
    ReadFile, WriteFile, FILE_FLAG_FIRST_PIPE_INSTANCE, FILE_FLAG_OVERLAPPED, PIPE_ACCESS_DUPLEX,
};
use windows_sys::Win32::System::Pipes::{
    ConnectNamedPipe, CreateNamedPipeW, DisconnectNamedPipe, PIPE_READMODE_BYTE,
    PIPE_REJECT_REMOTE_CLIENTS, PIPE_TYPE_BYTE, PIPE_UNLIMITED_INSTANCES, PIPE_WAIT,
};
use windows_sys::Win32::System::Threading::{CreateEventW, GetCurrentProcess, OpenProcessToken};
use windows_sys::Win32::System::IO::{GetOverlappedResult, OVERLAPPED};

use super::sessions::{self, Caller, Fail};
use super::{dispatch, exec, state};

const MAX_SESSIONS: usize = 16;
static LIVE: AtomicUsize = AtomicUsize::new(0);

/// A pipe handle shared by one reader and one writer thread. Overlapped I/O
/// lets a blocked read coexist with writes of responses and events.
#[derive(Clone, Copy)]
struct Pipe(usize);

// SAFETY: a Win32 pipe handle may be used from any thread; overlapped I/O
// keeps concurrent read and write independent.
unsafe impl Send for Pipe {}

impl Pipe {
    fn handle(self) -> HANDLE {
        self.0 as HANDLE
    }

    /// Runs one overlapped operation to completion.
    fn overlapped(self, start: impl FnOnce(*mut OVERLAPPED) -> i32) -> io::Result<usize> {
        unsafe {
            let event = CreateEventW(null(), 1, 0, null());
            if event.is_null() {
                return Err(io::Error::last_os_error());
            }
            let mut overlapped: OVERLAPPED = std::mem::zeroed();
            overlapped.hEvent = event;
            let mut transferred = 0u32;
            let ok = start(&mut overlapped) != 0 || GetLastError() == ERROR_IO_PENDING;
            let done =
                ok && GetOverlappedResult(self.handle(), &overlapped, &mut transferred, 1) != 0;
            let error = io::Error::last_os_error();
            CloseHandle(event);
            if done {
                Ok(transferred as usize)
            } else {
                Err(error)
            }
        }
    }

    fn read(self, buffer: &mut [u8]) -> io::Result<usize> {
        self.overlapped(|overlapped| unsafe {
            ReadFile(
                self.handle(),
                buffer.as_mut_ptr(),
                buffer.len() as u32,
                null_mut(),
                overlapped,
            )
        })
    }
}

/// One connection's pipe, closed when the last user lets go of it. The reader
/// (`serve`) and the writer thread each hold one, so the handle is never
/// closed while the other may still use it (Windows reuses handle values: a
/// late write could otherwise reach another client's pipe).
struct Connection(Pipe);

impl Drop for Connection {
    fn drop(&mut self) {
        unsafe {
            DisconnectNamedPipe(self.0.handle());
            CloseHandle(self.0.handle());
        }
    }
}

/// The writer thread's share of a [`Connection`].
struct Writer(std::sync::Arc<Connection>);

impl io::Write for Writer {
    fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
        let mut pipe = self.0 .0;
        io::Write::write(&mut pipe, bytes)
    }

    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

impl io::Write for Pipe {
    fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
        self.overlapped(|overlapped| unsafe {
            WriteFile(
                self.handle(),
                bytes.as_ptr(),
                bytes.len() as u32,
                null_mut(),
                overlapped,
            )
        })
    }

    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

fn wide(text: &str) -> Vec<u16> {
    text.encode_utf16().chain(std::iter::once(0)).collect()
}

/// Security descriptor granting the pipe to the current user only.
fn current_user_descriptor() -> io::Result<PSECURITY_DESCRIPTOR> {
    unsafe {
        let mut token: HANDLE = null_mut();
        if OpenProcessToken(GetCurrentProcess(), TOKEN_QUERY, &mut token) == 0 {
            return Err(io::Error::last_os_error());
        }
        // u64 storage keeps the TOKEN_USER view correctly aligned.
        let mut buffer = [0u64; 64];
        let mut length = 0u32;
        let ok = GetTokenInformation(
            token,
            TokenUser,
            buffer.as_mut_ptr().cast(),
            std::mem::size_of_val(&buffer) as u32,
            &mut length,
        );
        CloseHandle(token);
        if ok == 0 {
            return Err(io::Error::last_os_error());
        }
        let user = &*(buffer.as_ptr() as *const TOKEN_USER);
        let mut sid_text: *mut u16 = null_mut();
        if ConvertSidToStringSidW(user.User.Sid, &mut sid_text) == 0 {
            return Err(io::Error::last_os_error());
        }
        let mut len = 0;
        while *sid_text.add(len) != 0 {
            len += 1;
        }
        let sid = String::from_utf16_lossy(std::slice::from_raw_parts(sid_text, len));
        LocalFree(sid_text.cast());
        // Protected DACL: full access for this user, nobody else (no
        // inherited or Everyone/Anonymous entries).
        let sddl = wide(&format!("D:P(A;;GA;;;{sid})"));
        let mut descriptor: PSECURITY_DESCRIPTOR = null_mut();
        if ConvertStringSecurityDescriptorToSecurityDescriptorW(
            sddl.as_ptr(),
            1,
            &mut descriptor,
            null_mut(),
        ) == 0
        {
            return Err(io::Error::last_os_error());
        }
        Ok(descriptor)
    }
}

fn create_instance(
    name: &[u16],
    descriptor: PSECURITY_DESCRIPTOR,
    first: bool,
) -> io::Result<Pipe> {
    let attributes = SECURITY_ATTRIBUTES {
        nLength: std::mem::size_of::<SECURITY_ATTRIBUTES>() as u32,
        lpSecurityDescriptor: descriptor,
        bInheritHandle: 0,
    };
    let handle = unsafe {
        CreateNamedPipeW(
            name.as_ptr(),
            PIPE_ACCESS_DUPLEX
                | FILE_FLAG_OVERLAPPED
                // Refuse to start if another process already owns this name.
                | if first { FILE_FLAG_FIRST_PIPE_INSTANCE } else { 0 },
            PIPE_TYPE_BYTE | PIPE_READMODE_BYTE | PIPE_WAIT | PIPE_REJECT_REMOTE_CLIENTS,
            PIPE_UNLIMITED_INSTANCES,
            64 * 1024,
            64 * 1024,
            0,
            &attributes,
        )
    };
    if handle == INVALID_HANDLE_VALUE {
        Err(io::Error::last_os_error())
    } else {
        Ok(Pipe(handle as usize))
    }
}

fn constant_time_eq(left: &[u8], right: &[u8]) -> bool {
    left.len() == right.len()
        && left
            .iter()
            .zip(right)
            .fold(0u8, |acc, (a, b)| acc | (a ^ b))
            == 0
}

fn serve(pipe: Pipe, token: String, number: usize) {
    let session = format!("s-{number}");
    let mut decoder = FrameDecoder::new(FrameConfig::default());
    let mut buffer = [0u8; 16 * 1024];
    let mut authenticated = false;
    let connection = std::sync::Arc::new(Connection(pipe));
    let (line_tx, line_rx) = mpsc::channel::<String>();
    state::spawn_line_writer(
        "agentgo-pipe-writer",
        line_rx,
        Writer(std::sync::Arc::clone(&connection)),
    );
    'connection: loop {
        let read = match pipe.read(&mut buffer) {
            Ok(0) | Err(_) => break,
            Ok(read) => read,
        };
        if decoder.push(&buffer[..read]).is_err() {
            break; // oversized frame: drop the client
        }
        while let Some(message) = decoder.pop_message::<Request>() {
            let Ok(request) = message else {
                let _ = line_tx.send(dispatch::encode(
                    0,
                    Err(Fail::new(
                        ErrorCode::InvalidEnvelope,
                        "frame is not a valid request",
                    )),
                ));
                continue;
            };
            if !authenticated {
                let supplied = request.params["token"].as_str().unwrap_or("");
                let hello = request.method == "session.hello"
                    && constant_time_eq(supplied.as_bytes(), token.as_bytes());
                if !hello {
                    let _ = line_tx.send(dispatch::encode(
                        request.id,
                        Err(Fail::new(
                            ErrorCode::Unauthorized,
                            "first message must be session.hello with the instance token",
                        )),
                    ));
                    std::thread::sleep(std::time::Duration::from_millis(150));
                    break 'connection;
                }
                authenticated = true;
                let client = request.params["client"]
                    .as_str()
                    .unwrap_or("agent")
                    .to_string();
                let (id, session_id, out) = (request.id, session.clone(), line_tx.clone());
                exec::post(move || {
                    sessions::register(&session_id, &client, "pipe", out);
                    sessions::send_line(
                        &session_id,
                        dispatch::encode(
                            id,
                            Ok(json!({
                                "session": session_id,
                                "instanceId": state::state().instance_id,
                                "protocol": agentgo_protocol::PROTOCOL_VERSION,
                                "profile": state::profile_info(),
                                "controlMode": state::control_mode(),
                            })),
                        ),
                    );
                });
                state::wake_ui();
                continue;
            }
            let caller = Caller {
                session: session.clone(),
            };
            exec::post(move || dispatch::handle_request(caller, request));
            state::wake_ui();
        }
    }
    if authenticated {
        let session_id = session.clone();
        exec::post(move || sessions::unregister(&session_id));
        state::wake_ui();
    }
    drop(line_tx);
    // The writer thread closes the pipe once the session's last line is out.
    drop(connection);
    LIVE.fetch_sub(1, Ordering::SeqCst);
}

/// Pipe path for an instance id.
pub fn pipe_name(instance_id: &str) -> String {
    format!(r"\\.\pipe\agentgo-{instance_id}")
}

/// Starts the accept loop and writes the instance token next to the endpoint
/// record (inside the user's private AgentGo data directory).
pub fn start(instance_id: &str, instance_dir: &Path) -> io::Result<()> {
    let mut secret = [0u8; 32];
    getrandom::fill(&mut secret).map_err(|error| io::Error::other(error.to_string()))?;
    let token: String = secret.iter().map(|byte| format!("{byte:02x}")).collect();
    std::fs::create_dir_all(instance_dir)?;
    std::fs::write(instance_dir.join("token"), &token)?;

    let name = wide(&pipe_name(instance_id));
    let descriptor = current_user_descriptor()? as usize;
    let first = create_instance(&name, descriptor as PSECURITY_DESCRIPTOR, true)?;
    std::thread::Builder::new()
        .name("agentgo-pipe-accept".into())
        .spawn(move || {
            let mut pipe = first;
            for number in 1.. {
                let connected = pipe
                    .overlapped(|overlapped| unsafe { ConnectNamedPipe(pipe.handle(), overlapped) })
                    .is_ok()
                    || unsafe { GetLastError() } == ERROR_PIPE_CONNECTED;
                if connected && LIVE.load(Ordering::SeqCst) < MAX_SESSIONS {
                    LIVE.fetch_add(1, Ordering::SeqCst);
                    let token = token.clone();
                    let _ = std::thread::Builder::new()
                        .name("agentgo-pipe-session".into())
                        .spawn(move || serve(pipe, token, number));
                } else {
                    unsafe {
                        DisconnectNamedPipe(pipe.handle());
                        CloseHandle(pipe.handle());
                    }
                }
                match create_instance(&name, descriptor as PSECURITY_DESCRIPTOR, false) {
                    Ok(next) => pipe = next,
                    Err(_) => break,
                }
            }
        })?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn token_comparison_requires_exact_match() {
        assert!(constant_time_eq(b"abc", b"abc"));
        assert!(!constant_time_eq(b"abc", b"abd"));
        assert!(!constant_time_eq(b"abc", b"abcd"));
    }

    #[test]
    fn pipe_is_created_for_the_current_user_only() {
        let descriptor = current_user_descriptor().expect("descriptor");
        let name = wide(&pipe_name(&format!("test-{}", std::process::id())));
        let pipe = create_instance(&name, descriptor, true).expect("first instance");
        // A second "first instance" must be refused (name squatting guard).
        assert!(create_instance(&name, descriptor, true).is_err());
        unsafe { CloseHandle(pipe.handle()) };
    }
}
