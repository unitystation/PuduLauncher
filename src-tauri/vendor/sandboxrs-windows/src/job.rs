use std::io;
use windows_sys::Win32::Foundation::{CloseHandle, HANDLE};
use windows_sys::Win32::System::JobObjects::{
    AssignProcessToJobObject, CreateJobObjectW, JobObjectExtendedLimitInformation,
    SetInformationJobObject, TerminateJobObject, JOBOBJECT_EXTENDED_LIMIT_INFORMATION,
    JOB_OBJECT_LIMIT_ACTIVE_PROCESS, JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE,
    JOB_OBJECT_LIMIT_PROCESS_MEMORY,
};

use crate::ResourceLimits;
use crate::SandboxError;

/// Windows Job Object that owns a sandboxed process tree.
///
/// Closing the handle kills every contained process (`KILL_ON_JOB_CLOSE`), so
/// descendants cannot outlive the `SandboxChild`.
pub(crate) struct Job {
    handle: HANDLE,
}

impl Job {
    pub(crate) fn new(limits: ResourceLimits) -> Result<Self, SandboxError> {
        // SAFETY: A null name creates an unnamed job in the caller's job
        // hierarchy; no name, object security, or NT path is involved.
        let handle = unsafe { CreateJobObjectW(std::ptr::null(), std::ptr::null()) };
        if handle.is_null() {
            return Err(SandboxError::Io(io::Error::last_os_error()));
        }

        let job = Self { handle };
        job.apply_limits(limits)?;
        Ok(job)
    }

    pub(crate) fn assign_raw(&self, process_handle: HANDLE) -> Result<(), SandboxError> {
        // SAFETY: `process_handle` must be a live process handle supplied by
        // the caller, and the job handle remains valid for the lifetime of
        // `self`.
        let ok = unsafe { AssignProcessToJobObject(self.handle, process_handle) };
        if ok == 0 {
            return Err(SandboxError::Io(io::Error::last_os_error()));
        }
        Ok(())
    }

    /// Terminate every process currently contained in the job.
    pub(crate) fn terminate(&self) -> Result<(), SandboxError> {
        // SAFETY: Terminating a job is valid for any handle created by
        // `CreateJobObjectW` that has not been closed.
        let ok = unsafe { TerminateJobObject(self.handle, 1) };
        if ok == 0 {
            return Err(SandboxError::Io(io::Error::last_os_error()));
        }
        Ok(())
    }

    fn apply_limits(&self, limits: ResourceLimits) -> Result<(), SandboxError> {
        let mut info: JOBOBJECT_EXTENDED_LIMIT_INFORMATION = unsafe { std::mem::zeroed() };
        info.BasicLimitInformation.LimitFlags = JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE;

        if let Some(count) = limits.max_processes {
            info.BasicLimitInformation.ActiveProcessLimit = count;
            info.BasicLimitInformation.LimitFlags |= JOB_OBJECT_LIMIT_ACTIVE_PROCESS;
        }

        if let Some(bytes) = limits.max_memory {
            #[cfg(target_pointer_width = "64")]
            {
                info.ProcessMemoryLimit = bytes as usize;
                info.BasicLimitInformation.LimitFlags |= JOB_OBJECT_LIMIT_PROCESS_MEMORY;
            }
            #[cfg(target_pointer_width = "32")]
            {
                let _ = bytes;
                // 32-bit jobs cannot represent 64-bit memory limits faithfully.
                return Err(SandboxError::UnsupportedPolicy {
                    backend: crate::BackendKind::WindowsSandboxApi,
                    feature: "max_memory on 32-bit Windows",
                });
            }
        }

        // SAFETY: `info` is fully initialized and the length matches the
        // `JobObjectExtendedLimitInformation` structure.
        let ok = unsafe {
            SetInformationJobObject(
                self.handle,
                JobObjectExtendedLimitInformation,
                (&info as *const JOBOBJECT_EXTENDED_LIMIT_INFORMATION).cast::<core::ffi::c_void>(),
                std::mem::size_of::<JOBOBJECT_EXTENDED_LIMIT_INFORMATION>() as u32,
            )
        };
        if ok == 0 {
            return Err(SandboxError::Io(io::Error::last_os_error()));
        }
        Ok(())
    }
}

impl Drop for Job {
    fn drop(&mut self) {
        // SAFETY: The handle is valid and no other reference exists by Drop
        // time. Closing a job handle is infallible in the sense that no
        // user-controlled result can be reported from Drop.
        unsafe {
            let _ = CloseHandle(self.handle);
        }
    }
}

// The HANDLE is a raw pointer in windows-sys, but the Job owns exclusive
// access to it and must remain movable across threads.
unsafe impl Send for Job {}
unsafe impl Sync for Job {}
