//! A job object, so that killing a hosted shell kills what the shell started.
//!
//! `TerminateProcess` — which is the whole of `portable_pty`'s `Child::kill`,
//! and the whole of what one process can do to another on Windows — ends one
//! process. It does not end the `cargo build` that process launched, and
//! Windows keeps no parent/child relationship for anything to walk afterwards:
//! an orphaned grandchild simply keeps running, still attached to a
//! pseudoconsole that is being torn down and still holding locks on `target/`.
//! `Alt+S`, `cargo build`, `Alt+Q` used to leave `cargo.exe` and `rustc.exe`
//! behind in exactly the case the command view exists for.
//!
//! A job object is the operating system's answer. Every process a member starts
//! joins the job with it, and `JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE` terminates
//! all of them when the last handle to the job closes — which happens when the
//! session drops, and equally if forge is killed outright, because the handle
//! goes with the process. It is also why the session closes the job *before*
//! the master: `ClosePseudoConsole` can block while clients are still attached,
//! and by then there are none.
//!
//! Best effort throughout, and deliberately so. Every failure here leaves the
//! caller exactly where it stood before this file existed. A shell whose
//! children outlive it is worse than one contained; a shell that refuses to
//! start because a job object could not be created is worse than both.
//!
//! One gap cannot be closed from here. The child is adopted *after*
//! `CreateProcessW` returns rather than being created suspended, so anything it
//! spawns in those first microseconds is outside the job. Closing that would
//! need portable-pty to offer a suspended spawn, and it does not.

use std::os::windows::io::RawHandle;
use std::ptr;

use winapi::um::handleapi::CloseHandle;
use winapi::um::jobapi2::{AssignProcessToJobObject, CreateJobObjectW, SetInformationJobObject};
use winapi::um::winnt::{
    HANDLE, JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE, JOBOBJECT_EXTENDED_LIMIT_INFORMATION,
    JobObjectExtendedLimitInformation,
};

pub struct Job(HANDLE);

// The handle is owned by this type and leaves it only as an argument to
// `AssignProcessToJobObject` and `CloseHandle`, both of which the kernel
// serialises. What makes `HANDLE` neither `Send` nor `Sync` by default is that
// it is spelled as a raw pointer; it is not one.
unsafe impl Send for Job {}
unsafe impl Sync for Job {}

impl Job {
    /// A new unnamed job that terminates its members when it closes, or `None`
    /// if the operating system would not give us one.
    pub fn kill_on_close() -> Option<Job> {
        let handle = unsafe { CreateJobObjectW(ptr::null_mut(), ptr::null()) };
        if handle.is_null() {
            return None;
        }
        let job = Job(handle);

        let mut info: JOBOBJECT_EXTENDED_LIMIT_INFORMATION = unsafe { std::mem::zeroed() };
        info.BasicLimitInformation.LimitFlags = JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE;
        let set = unsafe {
            SetInformationJobObject(
                job.0,
                JobObjectExtendedLimitInformation,
                std::ptr::from_mut(&mut info).cast(),
                size_of::<JOBOBJECT_EXTENDED_LIMIT_INFORMATION>() as u32,
            )
        };
        // A job without the limit set is a job that kills nothing, and that is
        // worse than having none: it would read, here and in every test below,
        // as the problem having been solved.
        (set != 0).then_some(job)
    }

    /// Put a running process in the job. Anything it starts from now on joins
    /// the job as it is created, which is what makes one call enough.
    pub fn adopt(&self, process: RawHandle) -> bool {
        unsafe { AssignProcessToJobObject(self.0, process as HANDLE) != 0 }
    }
}

impl Drop for Job {
    /// This is the kill. Closing the last handle to a `KILL_ON_JOB_CLOSE` job
    /// terminates everything still inside it, which is the whole of what this
    /// file is for.
    fn drop(&mut self) {
        unsafe { CloseHandle(self.0) };
    }
}
