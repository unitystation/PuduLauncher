//! Compile a `FilesystemPlan` into AppContainer ACL grants.
//!
//! Explicit rules are applied recursively and fail closed. Baseline Windows
//! roots needed to launch executables are granted read/execute best-effort;
//! user policy is never flattened silently.

#[cfg(windows)]
use std::path::Path;

#[cfg(windows)]
use crate::filesystem::Access;
#[cfg(windows)]
use crate::filesystem::FilesystemPlan;
#[cfg(windows)]
use crate::BackendKind;
#[cfg(windows)]
use crate::SandboxError;

#[cfg(windows)]
const FILE_GENERIC_READ: u32 = 0x0012_0089;
#[cfg(windows)]
const FILE_GENERIC_EXECUTE: u32 = 0x0012_00A0;
#[cfg(windows)]
const FILE_ALL_ACCESS: u32 = 0x001F_01FF;

#[cfg(windows)]
pub(crate) fn apply_grants(
    plan: &FilesystemPlan,
    profile: &rappct::AppContainerProfile,
) -> Result<(), SandboxError> {
    for rule in plan.rules() {
        let access = match rule.access() {
            Access::ReadWrite => FILE_ALL_ACCESS,
            Access::ReadOnly => FILE_GENERIC_READ | FILE_GENERIC_EXECUTE,
            Access::Hidden => continue,
        };
        grant_recursive(rule.path(), profile, access).map_err(|err| {
            SandboxError::PolicyCompileFailed {
                backend: BackendKind::AppContainer,
                reason: format!("failed granting {}: {err}", rule.path().display()),
            }
        })?;
    }
    apply_baseline_readonly(profile);
    Ok(())
}

#[cfg(windows)]
fn apply_baseline_readonly(profile: &rappct::AppContainerProfile) {
    let read_execute = FILE_GENERIC_READ | FILE_GENERIC_EXECUTE;
    for env in [
        "SystemRoot",
        "ProgramFiles",
        "ProgramFiles(x86)",
        "CommonProgramFiles",
        "CommonProgramFiles(x86)",
    ] {
        if let Some(value) = std::env::var_os(env) {
            let path = Path::new(&value);
            let _ = grant_recursive(path, profile, read_execute);
        }
    }
}

#[cfg(windows)]
fn grant_recursive(
    path: &Path,
    profile: &rappct::AppContainerProfile,
    access: u32,
) -> Result<(), String> {
    let metadata = std::fs::symlink_metadata(path).map_err(|err| err.to_string())?;
    let target = if metadata.is_dir() {
        rappct::acl::ResourcePath::Directory(path.to_path_buf())
    } else {
        rappct::acl::ResourcePath::File(path.to_path_buf())
    };
    rappct::acl::grant_to_package(target, &profile.sid, rappct::acl::AccessMask(access))
        .map_err(|err| err.to_string())?;

    if metadata.is_dir() {
        let entries = std::fs::read_dir(path).map_err(|err| err.to_string())?;
        for entry in entries {
            let entry = entry.map_err(|err| err.to_string())?;
            let entry_path = entry.path();
            if let Err(err) = grant_recursive(&entry_path, profile, access) {
                // Existing protected system files may reject explicit grants;
                // those paths are not user policy and are allowed to remain
                // inaccessible to the sandbox.
                let _ = err;
            }
        }
    }
    Ok(())
}
