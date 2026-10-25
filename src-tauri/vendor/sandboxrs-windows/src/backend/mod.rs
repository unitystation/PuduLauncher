use std::ffi::OsString;
use std::path::PathBuf;

use crate::child::SandboxChild;
use crate::filesystem::FilesystemPlan;
use crate::sandbox::Sandbox;
use crate::sandbox::{BackendProbe, BackendProbeEntry};
use crate::{BackendKind, BackendPreference, SandboxError, Stdio};

pub(crate) mod appcontainer;
pub(crate) mod modern;

/// Probe and select a backend, caching the result for process lifetime.
pub(crate) fn select(preference: BackendPreference) -> Result<BackendKind, SandboxError> {
    #[cfg(not(windows))]
    {
        let _ = preference;
        Err(SandboxError::UnsupportedPlatform)
    }

    #[cfg(windows)]
    {
        probe_report(preference)
            .entries
            .into_iter()
            .find(|entry| entry.usable)
            .map(|entry| entry.backend)
            .ok_or(SandboxError::SandboxUnavailable)
    }
}

/// Validate that the selected backend can represent the filesystem plan.
pub(crate) fn validate(backend: &BackendKind, plan: &FilesystemPlan) -> Result<(), SandboxError> {
    modern::validate(*backend, plan)?;
    appcontainer::validate(*backend, plan)
}

pub(crate) fn probe_report(preference: BackendPreference) -> BackendProbe {
    let mut entries = Vec::new();

    let modern_result = modern::probe();
    let modern_entry = BackendProbeEntry {
        backend: BackendKind::WindowsSandboxApi,
        export_present: modern_result.export_present,
        usable: modern_result.usable,
        detail: modern_result.detail,
    };
    // Explicit backend selection must not probe unrelated backends. The
    // AppContainer probe compiles recursive ACL grants for Windows system
    // directories, which is expensive and can launch a copy of the host app.
    if matches!(preference, BackendPreference::WindowsSandboxApi) {
        return BackendProbe {
            entries: vec![modern_entry],
        };
    }
    entries.push(modern_entry);

    let appcontainer_result = appcontainer::probe();
    entries.push(BackendProbeEntry {
        backend: BackendKind::AppContainer,
        export_present: appcontainer_result.export_present,
        usable: appcontainer_result.usable,
        detail: appcontainer_result.detail,
    });

    if matches!(preference, BackendPreference::AppContainer) && !entries[1].usable {
        return BackendProbe { entries };
    }

    BackendProbe { entries }
}

#[expect(
    clippy::too_many_arguments,
    reason = "the public command builder stores these independent process attributes"
)]
pub(crate) fn spawn(
    sandbox: &Sandbox,
    program: OsString,
    args: Vec<OsString>,
    env_clear: bool,
    envs: std::collections::BTreeMap<OsString, OsString>,
    removals: Vec<OsString>,
    current_dir: Option<PathBuf>,
    stdin: Stdio,
    stdout: Stdio,
    stderr: Stdio,
) -> Result<SandboxChild, SandboxError> {
    #[cfg(windows)]
    {
        match sandbox.backend() {
            BackendKind::WindowsSandboxApi => modern::spawn::spawn(
                sandbox,
                program,
                args,
                env_clear,
                envs,
                removals,
                current_dir,
                stdin,
                stdout,
                stderr,
            ),
            BackendKind::AppContainer => appcontainer::spawn::spawn(
                sandbox,
                program,
                args,
                env_clear,
                envs,
                removals,
                current_dir,
                stdin,
                stdout,
                stderr,
            ),
        }
    }
    #[cfg(not(windows))]
    {
        let _ = (
            sandbox,
            program,
            args,
            env_clear,
            envs,
            removals,
            current_dir,
            stdin,
            stdout,
            stderr,
        );
        Err(SandboxError::UnsupportedPlatform)
    }
}
