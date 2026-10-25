use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;

use crate::backend;
use crate::filesystem::FilesystemPlan;
use crate::sandbox::Sandbox;
#[cfg(windows)]
use crate::BackendKind;
use crate::{BackendPreference, ResourceLimits, SandboxError};

/// Validates and builds a reusable [`Sandbox`].
pub struct SandboxBuilder {
    workspace: PathBuf,
    read_only: Vec<PathBuf>,
    read_write: Vec<PathBuf>,
    timeout: Option<Duration>,
    max_memory: Option<u64>,
    max_processes: Option<u32>,
    preferred_backend: BackendPreference,
    identity: Option<String>,
}

impl SandboxBuilder {
    pub(crate) fn new(workspace: impl AsRef<Path>) -> Self {
        Self {
            workspace: workspace.as_ref().to_path_buf(),
            read_only: Vec::new(),
            read_write: Vec::new(),
            timeout: None,
            max_memory: None,
            max_processes: None,
            preferred_backend: BackendPreference::Auto,
            identity: None,
        }
    }

    /// Grant read-only access to an explicit root.
    pub fn read_only(mut self, path: impl AsRef<Path>) -> Self {
        self.read_only.push(path.as_ref().to_path_buf());
        self
    }

    /// Grant read-write access to an explicit root.
    pub fn read_write(mut self, path: impl AsRef<Path>) -> Self {
        self.read_write.push(path.as_ref().to_path_buf());
        self
    }

    /// Fail the command after `timeout`, terminating its process tree.
    pub fn timeout(mut self, timeout: Duration) -> Self {
        self.timeout = Some(timeout);
        self
    }

    /// Maximum memory (in bytes) the sandboxed process tree may commit.
    pub fn max_memory(mut self, bytes: u64) -> Self {
        self.max_memory = Some(bytes);
        self
    }

    /// Maximum number of concurrently active sandboxed processes.
    pub fn max_processes(mut self, count: u32) -> Self {
        self.max_processes = Some(count);
        self
    }

    /// Prefer a specific backend; `Auto` probes in plan order.
    pub fn preferred_backend(mut self, preference: BackendPreference) -> Self {
        self.preferred_backend = preference;
        self
    }

    /// Set a diagnostic label for this sandbox identity.
    ///
    /// A unique suffix is always appended. Reusing a label never reuses an
    /// AppContainer profile or leaks filesystem grants between sandboxes.
    pub fn identity(mut self, identity: impl Into<String>) -> Self {
        self.identity = Some(identity.into());
        self
    }

    /// Validate policy, probe/select a backend, and perform setup.
    ///
    /// This is a real initialization boundary. If any required step fails, no
    /// [`Sandbox`] value exists.
    pub fn build(self) -> Result<Sandbox, SandboxError> {
        let plan = FilesystemPlan::compile(&self.workspace, &self.read_only, &self.read_write)?;
        let backend_kind = backend::select(self.preferred_backend)?;
        backend::validate(&backend_kind, &plan)?;
        let identity = generate_sandbox_identity(self.identity.as_deref());

        #[cfg(windows)]
        let appcontainer = if backend_kind == BackendKind::AppContainer {
            Some(backend::appcontainer::build_state(&identity, &plan)?)
        } else {
            None
        };

        Ok(Sandbox::new(
            self.workspace,
            plan,
            backend_kind,
            identity,
            ResourceLimits {
                max_processes: self.max_processes,
                max_memory: self.max_memory,
            },
            self.timeout,
            #[cfg(windows)]
            appcontainer,
        ))
    }
}

fn generate_sandbox_identity(label: Option<&str>) -> String {
    static NEXT_ID: AtomicU64 = AtomicU64::new(0);
    let nonce = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|duration| duration.as_nanos() as u64)
        .unwrap_or_else(|_| std::process::id() as u64);
    let sequence = NEXT_ID.fetch_add(1, Ordering::Relaxed);
    let prefix = label.filter(|value| !value.is_empty()).unwrap_or("sandbox");
    format!(
        "{prefix}-{:08x}-{nonce:016x}-{sequence:016x}",
        std::process::id()
    )
}

#[cfg(test)]
mod tests {
    use super::generate_sandbox_identity;

    #[test]
    fn identity_labels_do_not_reuse_profiles() {
        assert_ne!(
            generate_sandbox_identity(Some("shared-label")),
            generate_sandbox_identity(Some("shared-label"))
        );
    }
}
