use std::process::ExitStatus;
use std::time::Duration;

use crate::BackendKind;

/// Result of a completed sandboxed command.
#[derive(Debug, Clone)]
pub struct SandboxOutput {
    /// Exit status of the root process.
    pub status: ExitStatus,
    /// Captured stdout bytes, empty when stdout was not piped.
    pub stdout: Vec<u8>,
    /// Captured stderr bytes, empty when stderr was not piped.
    pub stderr: Vec<u8>,
    /// Backend that executed the command.
    pub backend: BackendKind,
    /// Elapsed wall-clock time from launch to collection.
    pub duration: Duration,
}

impl SandboxOutput {
    #[allow(dead_code)]
    pub(crate) fn from_output(
        output: std::process::Output,
        backend: BackendKind,
        duration: Duration,
    ) -> Self {
        Self {
            status: output.status,
            stdout: output.stdout,
            stderr: output.stderr,
            backend,
            duration,
        }
    }
}
