use std::collections::BTreeMap;
use std::ffi::{OsStr, OsString};
use std::path::{Path, PathBuf};

use crate::child::SandboxChild;
use crate::sandbox::Sandbox;
use crate::{SandboxError, SandboxOutput, Stdio};

/// A sandboxed command, mirroring the familiar `std::process::Command` API.
pub struct SandboxCommand<'a> {
    sandbox: &'a Sandbox,
    program: OsString,
    args: Vec<OsString>,
    env_clear: bool,
    envs: BTreeMap<OsString, OsString>,
    removals: Vec<OsString>,
    current_dir: Option<PathBuf>,
    stdin: Option<Stdio>,
    stdout: Option<Stdio>,
    stderr: Option<Stdio>,
}

impl<'a> SandboxCommand<'a> {
    pub(crate) fn new(sandbox: &'a Sandbox, program: &OsStr) -> Self {
        Self {
            sandbox,
            program: program.to_os_string(),
            args: Vec::new(),
            env_clear: false,
            envs: BTreeMap::new(),
            removals: Vec::new(),
            current_dir: None,
            stdin: None,
            stdout: None,
            stderr: None,
        }
    }

    /// Append one argument to the program invocation.
    pub fn arg(&mut self, arg: impl AsRef<OsStr>) -> &mut Self {
        self.args.push(arg.as_ref().to_os_string());
        self
    }

    /// Append multiple arguments to the program invocation.
    pub fn args<I, S>(&mut self, args: I) -> &mut Self
    where
        I: IntoIterator<Item = S>,
        S: AsRef<OsStr>,
    {
        self.args
            .extend(args.into_iter().map(|arg| arg.as_ref().to_os_string()));
        self
    }

    /// Set an environment variable for the child process.
    pub fn env(&mut self, key: impl AsRef<OsStr>, value: impl AsRef<OsStr>) -> &mut Self {
        self.envs
            .insert(key.as_ref().to_os_string(), value.as_ref().to_os_string());
        self
    }

    /// Remove an inherited environment variable from the child process.
    pub fn env_remove(&mut self, key: impl AsRef<OsStr>) -> &mut Self {
        self.removals.push(key.as_ref().to_os_string());
        self
    }

    /// Start the child with a cleared environment.
    ///
    /// The Windows variables required by `CreateProcessW` are rebuilt by the
    /// backend; add application-specific variables with [`Self::env`].
    pub fn env_clear(&mut self) -> &mut Self {
        self.env_clear = true;
        self
    }

    /// Set the child's working directory.
    pub fn current_dir(&mut self, dir: impl AsRef<Path>) -> &mut Self {
        self.current_dir = Some(dir.as_ref().to_path_buf());
        self
    }

    /// Configure the child's standard input stream.
    pub fn stdin(&mut self, stdio: Stdio) -> &mut Self {
        self.stdin = Some(stdio);
        self
    }

    /// Configure the child's standard output stream.
    pub fn stdout(&mut self, stdio: Stdio) -> &mut Self {
        self.stdout = Some(stdio);
        self
    }

    /// Configure the child's standard error stream.
    pub fn stderr(&mut self, stdio: Stdio) -> &mut Self {
        self.stderr = Some(stdio);
        self
    }

    /// Spawn the command with inherited standard streams by default.
    pub fn spawn(&mut self) -> Result<SandboxChild, SandboxError> {
        self.spawn_with_defaults(Stdio::inherit(), Stdio::inherit(), Stdio::inherit())
    }

    /// Run the command and capture stdout and stderr.
    ///
    /// This follows [`std::process::Command::output`]: stdin defaults to null,
    /// while stdout and stderr default to pipes. Explicit [`Stdio`] settings on
    /// this command always take precedence over those defaults.
    pub fn output(&mut self) -> Result<SandboxOutput, SandboxError> {
        let mut child = self.spawn_with_defaults(Stdio::null(), Stdio::piped(), Stdio::piped())?;
        child.wait_with_output()
    }

    fn spawn_with_defaults(
        &mut self,
        default_stdin: Stdio,
        default_stdout: Stdio,
        default_stderr: Stdio,
    ) -> Result<SandboxChild, SandboxError> {
        crate::backend::spawn(
            self.sandbox,
            self.program.clone(),
            self.args.clone(),
            self.env_clear,
            self.envs.clone(),
            self.removals.clone(),
            self.current_dir.clone(),
            self.stdin.take().unwrap_or(default_stdin),
            self.stdout.take().unwrap_or(default_stdout),
            self.stderr.take().unwrap_or(default_stderr),
        )
    }
}
