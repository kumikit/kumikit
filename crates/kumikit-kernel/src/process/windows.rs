//! Process trees on Windows: each child is in its own job object, which kills every
//! process in it when terminated or when its last handle closes.
//!
//! The child is assigned to the job right after it starts, so a grandchild it starts in
//! that first instant escapes the job (documented in 03-kernel).
//!
//! This is one of the modules allowed to use `unsafe` (Win32 FFI); each block says why
//! it is sound.
#![allow(unsafe_code)]

use tokio::process::Command;
use windows_sys::Win32::Foundation::{CloseHandle, HANDLE};
use windows_sys::Win32::System::JobObjects::{
    AssignProcessToJobObject, CreateJobObjectW, JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE,
    JOBOBJECT_EXTENDED_LIMIT_INFORMATION, JobObjectExtendedLimitInformation,
    SetInformationJobObject, TerminateJobObject,
};
use windows_sys::Win32::System::Threading::CREATE_NO_WINDOW;

/// The job object of one supervised child.
#[derive(Debug)]
pub(super) struct Tree {
    /// The job handle, stored as an integer so the type is `Send` and `Sync`. Owned:
    /// closed exactly once, in `Drop`.
    job: isize,
}

impl Tree {
    /// Console programs get no window of their own.
    pub(super) fn prepare(cmd: &mut Command) {
        cmd.creation_flags(CREATE_NO_WINDOW);
    }

    pub(super) fn attach(child: &tokio::process::Child) -> std::io::Result<Tree> {
        let process = child
            .raw_handle()
            .ok_or_else(|| std::io::Error::other("the child has already exited"))?;
        // SAFETY: both arguments may be null (default security, unnamed job). The result
        // is checked before use.
        let job: HANDLE = unsafe { CreateJobObjectW(std::ptr::null(), std::ptr::null()) };
        if job.is_null() {
            return Err(std::io::Error::last_os_error());
        }
        let tree = Tree { job: job as isize };
        // SAFETY: an all-zero JOBOBJECT_EXTENDED_LIMIT_INFORMATION is a valid value (plain
        // integers and nested plain structs).
        let mut info: JOBOBJECT_EXTENDED_LIMIT_INFORMATION = unsafe { std::mem::zeroed() };
        info.BasicLimitInformation.LimitFlags = JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE;
        let size = u32::try_from(std::mem::size_of_val(&info)).map_err(std::io::Error::other)?;
        // SAFETY: `job` is a live job handle; `info` is a valid structure of the class
        // named and `size` is its size; the call copies it.
        let ok = unsafe {
            SetInformationJobObject(
                job,
                JobObjectExtendedLimitInformation,
                (&raw const info).cast(),
                size,
            )
        };
        if ok == 0 {
            return Err(std::io::Error::last_os_error());
        }
        // SAFETY: `job` is live; `process` is the child's process handle, owned by the
        // tokio child, which outlives this call.
        let ok = unsafe { AssignProcessToJobObject(job, process as HANDLE) };
        if ok == 0 {
            return Err(std::io::Error::last_os_error());
        }
        Ok(tree)
    }

    /// Windows has no polite stop for console programs; the job is terminated.
    pub(super) fn stop(&self) {
        self.kill();
    }

    /// Terminates every process in the job.
    pub(super) fn kill(&self) {
        // SAFETY: `self.job` is a live job handle until `drop`. Failure (already empty)
        // is harmless.
        unsafe {
            TerminateJobObject(self.job as HANDLE, 1);
        }
    }
}

impl Drop for Tree {
    fn drop(&mut self) {
        // SAFETY: the handle is owned by this value and closed only here. Closing the last
        // handle also kills anything left in the job (KILL_ON_JOB_CLOSE).
        unsafe {
            CloseHandle(self.job as HANDLE);
        }
    }
}
