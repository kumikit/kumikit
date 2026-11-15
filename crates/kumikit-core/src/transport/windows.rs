//! The Windows named-pipe transport: first instance only, no remote clients, and a DACL
//! that grants access to the core's user alone.
//!
//! This is the one module allowed to use `unsafe` (see the workspace lints): building a
//! security descriptor for the current user needs Win32 calls that have no safe wrapper
//! in tokio or the standard library.

use std::ffi::c_void;
use std::io;
use std::ptr::null_mut;

use tokio::net::windows::named_pipe::{
    ClientOptions, NamedPipeClient, NamedPipeServer, ServerOptions,
};
use windows_sys::Win32::Foundation::{CloseHandle, ERROR_PIPE_BUSY, HANDLE, LocalFree};
use windows_sys::Win32::Security::Authorization::{
    ConvertSidToStringSidW, ConvertStringSecurityDescriptorToSecurityDescriptorW, SDDL_REVISION_1,
};
use windows_sys::Win32::Security::{
    GetTokenInformation, PSECURITY_DESCRIPTOR, SECURITY_ATTRIBUTES, TOKEN_QUERY, TOKEN_USER,
    TokenUser,
};
use windows_sys::Win32::System::Threading::{GetCurrentProcess, OpenProcessToken};

/// A security descriptor that grants GENERIC_ALL to the current user only.
struct OwnerOnly {
    descriptor: PSECURITY_DESCRIPTOR,
    attrs: SECURITY_ATTRIBUTES,
}

// SAFETY: `descriptor` points to memory this struct owns exclusively (allocated by
// ConvertStringSecurityDescriptorToSecurityDescriptorW, freed in Drop); nothing else
// holds the pointer, so moving the struct to another thread is sound.
unsafe impl Send for OwnerOnly {}
// SAFETY: the descriptor is only read (by CreateNamedPipeW) after construction.
unsafe impl Sync for OwnerOnly {}

impl OwnerOnly {
    fn new() -> io::Result<Box<OwnerOnly>> {
        let sid = current_user_sid()?;
        // D:P = protected DACL (no inherited entries); one ACE: allow GENERIC_ALL to sid.
        let sddl: Vec<u16> = format!("D:P(A;;GA;;;{sid})")
            .encode_utf16()
            .chain(std::iter::once(0))
            .collect();
        let mut descriptor: PSECURITY_DESCRIPTOR = null_mut();
        // SAFETY: `sddl` is NUL-terminated UTF-16 and outlives the call; `descriptor` is a
        // valid out pointer; the size out pointer may be null.
        let ok = unsafe {
            ConvertStringSecurityDescriptorToSecurityDescriptorW(
                sddl.as_ptr(),
                SDDL_REVISION_1,
                &mut descriptor,
                null_mut(),
            )
        };
        if ok == 0 {
            return Err(io::Error::last_os_error());
        }
        // Boxed so the SECURITY_ATTRIBUTES pointer handed to CreateNamedPipeW stays put.
        Ok(Box::new(OwnerOnly {
            descriptor,
            attrs: SECURITY_ATTRIBUTES {
                nLength: size_of::<SECURITY_ATTRIBUTES>() as u32,
                lpSecurityDescriptor: descriptor,
                bInheritHandle: 0,
            },
        }))
    }

    fn as_ptr(&mut self) -> *mut c_void {
        (&mut self.attrs as *mut SECURITY_ATTRIBUTES).cast()
    }
}

impl Drop for OwnerOnly {
    fn drop(&mut self) {
        // SAFETY: `descriptor` was allocated with LocalAlloc by the conversion call and is
        // freed exactly once, here.
        unsafe { LocalFree(self.descriptor) };
    }
}

/// Closes a handle on drop.
struct Handle(HANDLE);

impl Drop for Handle {
    fn drop(&mut self) {
        // SAFETY: the handle came from OpenProcessToken and is closed exactly once.
        unsafe { CloseHandle(self.0) };
    }
}

/// The current user's SID as a string, for example `S-1-5-21-…`.
fn current_user_sid() -> io::Result<String> {
    let mut token: HANDLE = null_mut();
    // SAFETY: GetCurrentProcess returns a pseudo-handle that needs no closing; `token` is a
    // valid out pointer.
    if unsafe { OpenProcessToken(GetCurrentProcess(), TOKEN_QUERY, &mut token) } == 0 {
        return Err(io::Error::last_os_error());
    }
    let token = Handle(token);

    let mut len = 0u32;
    // SAFETY: a null buffer with length 0 asks only for the required size; the call
    // "fails" with ERROR_INSUFFICIENT_BUFFER and sets `len`.
    unsafe { GetTokenInformation(token.0, TokenUser, null_mut(), 0, &mut len) };
    if len == 0 {
        return Err(io::Error::last_os_error());
    }
    // u64 elements keep the buffer 8-byte aligned for TOKEN_USER.
    let mut buf = vec![0u64; (len as usize).div_ceil(8)];
    // SAFETY: `buf` has at least `len` bytes and stays alive for the call.
    if unsafe { GetTokenInformation(token.0, TokenUser, buf.as_mut_ptr().cast(), len, &mut len) }
        == 0
    {
        return Err(io::Error::last_os_error());
    }
    // SAFETY: on success the buffer starts with a TOKEN_USER, and it is suitably aligned.
    let user = unsafe { &*buf.as_ptr().cast::<TOKEN_USER>() };

    let mut wide: *mut u16 = null_mut();
    // SAFETY: `user.User.Sid` points into `buf`, which is alive; `wide` is a valid out
    // pointer.
    if unsafe { ConvertSidToStringSidW(user.User.Sid, &mut wide) } == 0 {
        return Err(io::Error::last_os_error());
    }
    let mut n = 0usize;
    // SAFETY: `wide` is a NUL-terminated UTF-16 string allocated by the call above.
    while unsafe { *wide.add(n) } != 0 {
        n += 1;
    }
    // SAFETY: the `n` code units before the terminator are initialized.
    let sid = String::from_utf16_lossy(unsafe { std::slice::from_raw_parts(wide, n) });
    // SAFETY: allocated with LocalAlloc by ConvertSidToStringSidW; freed once.
    unsafe { LocalFree(wide.cast()) };
    Ok(sid)
}

/// A named pipe that accepts connections from this user only.
pub struct Listener {
    name: String,
    security: Box<OwnerOnly>,
    next: NamedPipeServer,
}

impl std::fmt::Debug for Listener {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Listener")
            .field("name", &self.name)
            .finish_non_exhaustive()
    }
}

fn create(name: &str, security: &mut OwnerOnly, first: bool) -> io::Result<NamedPipeServer> {
    let mut options = ServerOptions::new();
    options
        .first_pipe_instance(first)
        .reject_remote_clients(true);
    // SAFETY: `security.as_ptr()` points to a SECURITY_ATTRIBUTES whose descriptor is
    // alive for as long as `security`, which outlives this call.
    unsafe { options.create_with_security_attributes_raw(name, security.as_ptr()) }
}

impl Listener {
    /// Creates the pipe. Fails if another process already owns the name.
    pub async fn bind(name: &str) -> io::Result<Listener> {
        let mut security = OwnerOnly::new()?;
        let next = create(name, &mut security, true).map_err(|e| {
            io::Error::new(
                e.kind(),
                format!("cannot create {name} (is another core running?): {e}"),
            )
        })?;
        Ok(Listener {
            name: name.to_owned(),
            security,
            next,
        })
    }

    /// Waits for the next connection. Access is limited by the pipe's DACL.
    pub async fn accept(&mut self) -> io::Result<NamedPipeServer> {
        self.next.connect().await?;
        let fresh = create(&self.name, &mut self.security, false)?;
        Ok(std::mem::replace(&mut self.next, fresh))
    }

    /// Named pipes vanish with their last handle; nothing to remove.
    pub fn remove(&self) {}
}

/// Connects to a core's pipe, retrying while all instances are busy.
pub async fn connect(name: &str) -> io::Result<NamedPipeClient> {
    loop {
        match ClientOptions::new().open(name) {
            Ok(client) => return Ok(client),
            Err(e) if e.raw_os_error() == Some(ERROR_PIPE_BUSY as i32) => {
                // The server creates the next instance right after accepting one.
                tokio::time::sleep(std::time::Duration::from_millis(5)).await;
            }
            Err(e) => return Err(e),
        }
    }
}
