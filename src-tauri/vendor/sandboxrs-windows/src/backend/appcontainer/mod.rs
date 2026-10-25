//! Regular AppContainer fallback via `rappct`.
//!
//! The fallback enforces the same public contract as the modern backend:
//! profile lifecycle, explicit filesystem grants, secure launch without admin,
//! Job-object tree containment, and fail-closed policy compilation.

use crate::filesystem::FilesystemPlan;
use crate::{BackendKind, SandboxError};

pub(crate) mod compile;
pub(crate) mod probe;
#[cfg(windows)]
pub(crate) mod spawn;

#[derive(Clone)]
pub(crate) struct ProbeOutcome {
    pub(crate) export_present: bool,
    pub(crate) usable: bool,
    pub(crate) detail: String,
}

#[cfg(windows)]
#[derive(Debug)]
pub(crate) struct AppContainerSandbox {
    pub(crate) profile: rappct::AppContainerProfile,
}

#[cfg(windows)]
impl Drop for AppContainerSandbox {
    fn drop(&mut self) {
        // AppContainer profiles are per-sandbox resources. Delete best-effort
        // on teardown; profile reuse is never required for correctness.
        let _ = self.profile.clone().delete();
    }
}

pub(crate) fn probe() -> ProbeOutcome {
    #[cfg(windows)]
    {
        static PROBE: std::sync::OnceLock<ProbeOutcome> = std::sync::OnceLock::new();
        PROBE.get_or_init(build_probe).clone()
    }
    #[cfg(not(windows))]
    {
        ProbeOutcome {
            export_present: false,
            usable: false,
            detail: "AppContainer fallback requires Windows".into(),
        }
    }
}

#[cfg(windows)]
fn build_probe() -> ProbeOutcome {
    match probe::launch_probe() {
        Ok(()) => ProbeOutcome {
            export_present: true,
            usable: true,
            detail: "AppContainer profile launch and outside-write denial succeeded".into(),
        },
        Err(err) => ProbeOutcome {
            export_present: false,
            usable: false,
            detail: format!("AppContainer probe failed: {err:?}"),
        },
    }
}

#[cfg(windows)]
pub(crate) fn build_state(
    identity: &str,
    plan: &FilesystemPlan,
) -> Result<AppContainerSandbox, SandboxError> {
    let profile = rappct::AppContainerProfile::ensure(identity, identity, Some("sandboxrs"))
        .map_err(map_rappct_error)?;
    let sandbox = AppContainerSandbox {
        profile: profile.clone(),
    };

    compile::apply_grants(plan, &profile).map_err(|err| {
        let _ = profile.delete();
        err
    })?;
    Ok(sandbox)
}

pub(crate) fn validate(backend: BackendKind, _plan: &FilesystemPlan) -> Result<(), SandboxError> {
    if backend == BackendKind::AppContainer && !cfg!(windows) {
        return Err(SandboxError::UnsupportedPlatform);
    }
    Ok(())
}

#[cfg(windows)]
pub(crate) fn map_rappct_error(err: rappct::AcError) -> SandboxError {
    SandboxError::BackendProbeFailed {
        backend: BackendKind::AppContainer,
        source: std::io::Error::other(err.to_string()).into(),
    }
}
