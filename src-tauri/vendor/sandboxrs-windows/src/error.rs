use std::error::Error;
use std::fmt;

use crate::BackendKind;

/// Errors produced while validating, building, or executing a sandbox.
#[derive(Debug)]
pub enum SandboxError {
    /// No usable sandbox backend exists on this machine/platform.
    SandboxUnavailable,

    /// The crate is Windows-only for v1.
    UnsupportedPlatform,

    /// A backend could not be probed.
    BackendProbeFailed {
        backend: BackendKind,
        source: Box<dyn Error + Send + Sync>,
    },

    /// The requested policy cannot be faithfully represented by a backend.
    UnsupportedPolicy {
        backend: BackendKind,
        feature: &'static str,
    },

    /// A filesystem root is malformed or outside supported policy.
    InvalidPath {
        path: std::path::PathBuf,
        reason: String,
    },

    /// Filesystem rules could not be compiled for a backend.
    PolicyCompileFailed {
        backend: BackendKind,
        reason: String,
    },

    /// Sandboxed process creation failed.
    ProcessCreationFailed {
        backend: BackendKind,
        win32_code: Option<u32>,
        message: String,
    },

    /// The command exceeded its timeout and its process tree was terminated.
    Timeout,

    /// An underlying I/O or OS operation failed.
    Io(std::io::Error),
}

impl fmt::Display for SandboxError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::SandboxUnavailable => {
                write!(f, "no usable sandbox backend is available on this machine")
            }
            Self::UnsupportedPlatform => {
                write!(f, "sandboxrs-windows v1 supports Windows only")
            }
            Self::BackendProbeFailed { backend, source } => {
                write!(f, "{} backend probe failed: {source}", backend.label())
            }
            Self::UnsupportedPolicy { backend, feature } => {
                write!(
                    f,
                    "{} backend cannot enforce requested policy: {feature}",
                    backend.label()
                )
            }
            Self::InvalidPath { path, reason } => {
                write!(f, "invalid path {}: {reason}", path.display())
            }
            Self::PolicyCompileFailed { backend, reason } => {
                write!(
                    f,
                    "{} backend failed to compile filesystem policy: {reason}",
                    backend.label()
                )
            }
            Self::ProcessCreationFailed {
                backend,
                win32_code,
                message,
            } => {
                write!(f, "{} backend failed to create process", backend.label())?;
                if let Some(code) = win32_code {
                    write!(f, " (Win32 error: {code})")?;
                }
                if !message.is_empty() {
                    write!(f, ": {message}")?;
                }
                Ok(())
            }
            Self::Timeout => write!(f, "sandboxed command timed out"),
            Self::Io(err) => write!(f, "I/O error: {err}"),
        }
    }
}

impl Error for SandboxError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
            Self::BackendProbeFailed { source, .. } => Some(source.as_ref()),
            Self::Io(err) => Some(err),
            _ => None,
        }
    }
}

impl From<std::io::Error> for SandboxError {
    fn from(err: std::io::Error) -> Self {
        Self::Io(err)
    }
}
