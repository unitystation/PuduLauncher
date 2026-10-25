//! Experimental Windows sandbox API backend.
//!
//! All experimental FFI stays inside this module. The Microsoft API is
//! unstable, so capability detection is runtime probing only: resolve the
//! export, query `Experimental_QuerySandboxSupport` when present, and perform
//! a real minimal sandbox launch before the backend is considered usable.

use crate::filesystem::FilesystemPlan;
use crate::{BackendKind, SandboxError};
use std::sync::OnceLock;

pub(crate) mod ffi;
#[cfg(windows)]
pub(crate) mod probe;
#[cfg(windows)]
pub(crate) mod spawn;
#[cfg(windows)]
pub(crate) mod spec;

#[derive(Clone)]
pub(crate) struct ProbeOutcome {
    pub(crate) export_present: bool,
    pub(crate) usable: bool,
    pub(crate) detail: String,
}

pub(crate) fn probe() -> ProbeOutcome {
    static PROBE: OnceLock<ProbeOutcome> = OnceLock::new();
    PROBE.get_or_init(build_probe).clone()
}

fn build_probe() -> ProbeOutcome {
    if let Err(err) = ffi::create_api() {
        return ProbeOutcome {
            export_present: false,
            usable: false,
            detail: err,
        };
    }

    #[cfg(not(windows))]
    {
        ProbeOutcome {
            export_present: true,
            usable: false,
            detail: "Windows backend cannot launch on non-Windows hosts".into(),
        }
    }
    #[cfg(windows)]
    {
        let export_present = true;
        if let Some(capabilities) = ffi::query_sandbox_capabilities() {
            if capabilities & ffi::SANDBOX_CAP_CREATE_PROCESS_IN_SANDBOX == 0 {
                return ProbeOutcome {
                    export_present,
                    usable: false,
                    detail:
                        "Experimental_QuerySandboxSupport reports the create-process capability is disabled"
                            .into(),
                };
            }
        } else if !probe_feature_enabled() {
            return ProbeOutcome {
                export_present,
                usable: false,
                detail: "create API call reports the sandbox feature is not enabled on this build"
                    .into(),
            };
        }

        match probe::launch_probe() {
            Ok(()) => ProbeOutcome {
                export_present,
                usable: true,
                detail: "export present; minimal sandbox launch and outside-write denial succeeded"
                    .into(),
            },
            Err(err) => ProbeOutcome {
                export_present,
                usable: false,
                detail: format!("minimal sandbox launch probe failed: {err}"),
            },
        }
    }
}

#[cfg(windows)]
fn probe_feature_enabled() -> bool {
    let api = match ffi::create_api() {
        Ok(api) => api,
        Err(_) => return false,
    };
    let mut process_information: windows_sys::Win32::System::Threading::PROCESS_INFORMATION =
        unsafe { std::mem::zeroed() };
    // SAFETY: All parameters are null/invalid so the call fails without
    // launching anything; only the error code matters.
    let result = unsafe {
        api(
            std::ptr::null(),
            std::ptr::null_mut(),
            std::ptr::null(),
            std::ptr::null(),
            0,
            0,
            std::ptr::null(),
            std::ptr::null(),
            std::ptr::null(),
            std::ptr::null(),
            std::ptr::null(),
            0,
            &mut process_information,
        )
    };
    let error = unsafe { windows_sys::Win32::Foundation::GetLastError() };
    if result != 0 {
        spawn::cleanup_process_info(&process_information);
        return true;
    }
    error != 120 /* ERROR_CALL_NOT_IMPLEMENTED */
        && error != 0x8000_4001 /* E_NOTIMPL */
}

pub(crate) fn validate(backend: BackendKind, plan: &FilesystemPlan) -> Result<(), SandboxError> {
    if backend != BackendKind::WindowsSandboxApi {
        return Ok(());
    }
    for rule in plan.rules() {
        if rule.access().is_writable() && is_drive_root(rule.path()) {
            return Err(SandboxError::UnsupportedPolicy {
                backend,
                feature:
                    "read-write drive root is not applied recursively by the Windows sandbox API",
            });
        }
    }
    Ok(())
}

fn is_drive_root(path: &std::path::Path) -> bool {
    let components: Vec<_> = path.components().collect();
    components.len() == 1 && path.as_os_str().len() <= 3
}
