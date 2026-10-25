use std::path::{Path, PathBuf};
use std::time::Duration;

use crate::backend;
use crate::builder::SandboxBuilder;
use crate::command::SandboxCommand;
use crate::filesystem::FilesystemPlan;
use crate::{BackendKind, BackendPreference, ResourceLimits};

/// Reusable, validated authority boundary.
///
/// A sandbox represents one policy. Multiple commands may run inside it
/// without rebuilding policy each time.
#[derive(Debug)]
#[allow(dead_code)]
pub struct Sandbox {
    workspace: PathBuf,
    plan: FilesystemPlan,
    backend: BackendKind,
    identity: String,
    limits: ResourceLimits,
    timeout: Option<Duration>,
    #[cfg(windows)]
    appcontainer: Option<crate::backend::appcontainer::AppContainerSandbox>,
}

#[allow(dead_code)]
impl Sandbox {
    /// Start building a sandbox whose workspace is read-write.
    pub fn builder(workspace: impl AsRef<Path>) -> SandboxBuilder {
        SandboxBuilder::new(workspace)
    }

    pub(crate) fn new(
        workspace: PathBuf,
        plan: FilesystemPlan,
        backend: BackendKind,
        identity: String,
        limits: ResourceLimits,
        timeout: Option<Duration>,
        #[cfg(windows)] appcontainer: Option<crate::backend::appcontainer::AppContainerSandbox>,
    ) -> Self {
        Self {
            workspace,
            plan,
            backend,
            identity,
            limits,
            timeout,
            #[cfg(windows)]
            appcontainer,
        }
    }

    /// Create a command that will execute inside this sandbox.
    pub fn command(&self, program: impl AsRef<std::ffi::OsStr>) -> SandboxCommand<'_> {
        SandboxCommand::new(self, program.as_ref())
    }

    /// The backend enforcing this sandbox.
    pub fn backend(&self) -> BackendKind {
        self.backend
    }

    pub(crate) fn identity(&self) -> &str {
        &self.identity
    }

    pub(crate) fn workspace(&self) -> &Path {
        &self.workspace
    }

    pub(crate) fn plan(&self) -> &FilesystemPlan {
        &self.plan
    }

    pub(crate) fn limits(&self) -> ResourceLimits {
        self.limits
    }

    pub(crate) fn timeout(&self) -> Option<Duration> {
        self.timeout
    }

    #[cfg(windows)]
    pub(crate) fn appcontainer(
        &self,
    ) -> Option<&crate::backend::appcontainer::AppContainerSandbox> {
        self.appcontainer.as_ref()
    }

    /// Backends that are usable on this machine, probed live.
    ///
    /// This does not select a backend or create a sandbox profile.
    pub fn available_backends() -> Vec<BackendKind> {
        backend::probe_report(BackendPreference::Auto)
            .entries
            .into_iter()
            .filter(|entry| entry.usable)
            .map(|entry| entry.backend)
            .collect()
    }

    /// Capability probe report for diagnostics.
    pub fn probe() -> BackendProbe {
        backend::probe_report(BackendPreference::Auto)
    }
}

/// Result of probing sandbox backends.
#[derive(Debug, Clone)]
pub struct BackendProbe {
    /// One live probe result for each supported backend.
    pub entries: Vec<BackendProbeEntry>,
}

/// Result of probing one sandbox backend.
#[derive(Debug, Clone)]
pub struct BackendProbeEntry {
    /// Backend represented by this entry.
    pub backend: BackendKind,
    /// Whether the backend's entry point could be found.
    pub export_present: bool,
    /// Whether the backend passed its real launch probe.
    pub usable: bool,
    /// Human-readable probe detail suitable for diagnostics.
    pub detail: String,
}
