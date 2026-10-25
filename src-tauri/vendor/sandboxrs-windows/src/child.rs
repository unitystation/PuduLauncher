#[cfg(windows)]
use std::fs::File;
#[cfg(windows)]
use std::io::{Read, Write};
#[cfg(windows)]
use std::os::windows::process::ExitStatusExt;
#[cfg(windows)]
use std::thread;
use std::time::{Duration, Instant};

use crate::output::SandboxOutput;
use crate::{BackendKind, SandboxError};

/// A spawned sandboxed process tree.
#[cfg_attr(not(windows), allow(dead_code))]
pub struct SandboxChild {
    backend: BackendKind,
    started: Instant,
    timeout: Option<Duration>,
    #[cfg(windows)]
    process: Option<windows_sys::Win32::Foundation::HANDLE>,
    #[cfg(windows)]
    thread: Option<windows_sys::Win32::Foundation::HANDLE>,
    #[cfg(windows)]
    pid: u32,
    #[cfg(windows)]
    job: Option<crate::job::Job>,
    #[cfg(windows)]
    pub stdin: Option<SandboxChildStdin>,
    #[cfg(windows)]
    pub stdout: Option<SandboxChildStdout>,
    #[cfg(windows)]
    pub stderr: Option<SandboxChildStderr>,
    #[cfg(not(windows))]
    _marker: (),
}

impl SandboxChild {
    #[cfg_attr(not(windows), allow(dead_code))]
    #[cfg(windows)]
    pub(crate) fn new(
        backend: BackendKind,
        process: windows_sys::Win32::Foundation::HANDLE,
        thread: windows_sys::Win32::Foundation::HANDLE,
        pid: u32,
        job: crate::job::Job,
        stdin: Option<File>,
        stdout: Option<File>,
        stderr: Option<File>,
        timeout: Option<Duration>,
    ) -> Self {
        Self {
            backend,
            started: Instant::now(),
            timeout,
            process: Some(process),
            thread: Some(thread),
            pid,
            job: Some(job),
            stdin: stdin.map(SandboxChildStdin),
            stdout: stdout.map(SandboxChildStdout),
            stderr: stderr.map(SandboxChildStderr),
        }
    }

    #[cfg(not(windows))]
    #[cfg_attr(not(windows), allow(dead_code))]
    pub(crate) fn unsupported(backend: BackendKind) -> Self {
        Self {
            backend,
            started: Instant::now(),
            timeout: None,
            _marker: (),
        }
    }

    /// Return the root process ID while it is available.
    pub fn id(&self) -> Option<u32> {
        #[cfg(windows)]
        {
            Some(self.pid)
        }
        #[cfg(not(windows))]
        {
            let _ = self.backend;
            None
        }
    }

    /// Return the backend enforcing this child process tree.
    pub fn backend(&self) -> BackendKind {
        self.backend
    }

    /// Check whether the root process has exited without blocking.
    pub fn try_wait(&mut self) -> Result<Option<std::process::ExitStatus>, SandboxError> {
        #[cfg(windows)]
        {
            use windows_sys::Win32::Foundation::WAIT_OBJECT_0;
            use windows_sys::Win32::System::Threading::{GetExitCodeProcess, WaitForSingleObject};

            let process = self.process.ok_or(no_child())?;
            let wait = unsafe { WaitForSingleObject(process, 0) };
            if wait == WAIT_OBJECT_0 {
                let mut code = 0u32;
                // SAFETY: `process` is a live handle owned by this child.
                let ok = unsafe { GetExitCodeProcess(process, &mut code) };
                if ok == 0 {
                    return Err(SandboxError::Io(std::io::Error::last_os_error()));
                }
                Ok(Some(std::process::ExitStatus::from_raw(code)))
            } else if wait == 258
            /* WAIT_TIMEOUT */
            {
                Ok(None)
            } else {
                Err(SandboxError::Io(std::io::Error::last_os_error()))
            }
        }
        #[cfg(not(windows))]
        {
            let _ = self.backend;
            Err(SandboxError::UnsupportedPlatform)
        }
    }

    /// Wait for completion, applying the configured timeout when present.
    ///
    /// A timeout terminates the entire Job Object before this method returns
    /// the final process status.
    pub fn wait(&mut self) -> Result<std::process::ExitStatus, SandboxError> {
        #[cfg(windows)]
        {
            let deadline = self.timeout.map(|timeout| Instant::now() + timeout);
            loop {
                if let Some(status) = self.try_wait()? {
                    return Ok(status);
                }
                if deadline.is_some_and(|deadline| Instant::now() >= deadline) {
                    self.terminate_tree()?;
                    return self.wait_until_exit();
                }
                thread::sleep(Duration::from_millis(25));
            }
        }
        #[cfg(not(windows))]
        {
            let _ = self.backend;
            Err(SandboxError::UnsupportedPlatform)
        }
    }

    /// Terminate every process in the sandboxed Job Object.
    pub fn kill(&mut self) -> Result<(), SandboxError> {
        #[cfg(windows)]
        {
            self.terminate_tree()?;
            Ok(())
        }
        #[cfg(not(windows))]
        {
            let _ = self.backend;
            Err(SandboxError::UnsupportedPlatform)
        }
    }

    /// Wait for completion while draining configured stdout and stderr pipes.
    ///
    /// Streams configured as [`crate::Stdio::Inherit`] or [`crate::Stdio::Null`]
    /// produce empty byte vectors in the returned [`SandboxOutput`].
    pub fn wait_with_output(&mut self) -> Result<SandboxOutput, SandboxError> {
        #[cfg(windows)]
        {
            let backend = self.backend;
            let started = self.started;

            let stdout = self
                .stdout
                .take()
                .map(|mut pipe| thread::spawn(move || read_all(&mut pipe.0)));
            let stderr = self
                .stderr
                .take()
                .map(|mut pipe| thread::spawn(move || read_all(&mut pipe.0)));

            let status = self.wait()?;
            let stdout = stdout
                .map(|reader| reader.join().unwrap_or_default())
                .unwrap_or_default();
            let stderr = stderr
                .map(|reader| reader.join().unwrap_or_default())
                .unwrap_or_default();

            Ok(SandboxOutput::from_output(
                std::process::Output {
                    status,
                    stdout,
                    stderr,
                },
                backend,
                started.elapsed(),
            ))
        }
        #[cfg(not(windows))]
        {
            let _ = self.backend;
            Err(SandboxError::UnsupportedPlatform)
        }
    }

    #[cfg(windows)]
    fn terminate_tree(&mut self) -> Result<(), SandboxError> {
        if let Some(job) = &self.job {
            let _ = job.terminate();
            return Ok(());
        }
        let process = self.process.ok_or(no_child())?;
        // SAFETY: `process` is a live handle owned by this child.
        let ok = unsafe { windows_sys::Win32::System::Threading::TerminateProcess(process, 1) };
        if ok == 0 {
            return Err(SandboxError::Io(std::io::Error::last_os_error()));
        }
        Ok(())
    }

    #[cfg(windows)]
    fn wait_until_exit(&mut self) -> Result<std::process::ExitStatus, SandboxError> {
        use windows_sys::Win32::Foundation::WAIT_OBJECT_0;
        use windows_sys::Win32::System::Threading::{GetExitCodeProcess, WaitForSingleObject};
        let process = self.process.ok_or(no_child())?;
        // SAFETY: `process` is a live handle owned by this child.
        let wait = unsafe { WaitForSingleObject(process, u32::MAX) };
        if wait != WAIT_OBJECT_0 {
            return Err(SandboxError::Io(std::io::Error::last_os_error()));
        }
        let mut code = 0u32;
        // SAFETY: Same handle ownership as above.
        let ok = unsafe { GetExitCodeProcess(process, &mut code) };
        if ok == 0 {
            return Err(SandboxError::Io(std::io::Error::last_os_error()));
        }
        Ok(std::process::ExitStatus::from_raw(code))
    }
}

#[cfg(windows)]
impl Drop for SandboxChild {
    fn drop(&mut self) {
        // The job is configured with KILL_ON_JOB_CLOSE; terminating first is
        // belt-and-braces for hosts where that flag was not applied.
        if let Some(job) = &self.job {
            let _ = job.terminate();
        }
        if let Some(process) = self.process.take() {
            // SAFETY: The child owns this handle and no longer needs it.
            unsafe {
                let _ = windows_sys::Win32::Foundation::CloseHandle(process);
            }
        }
        if let Some(thread) = self.thread.take() {
            // SAFETY: The child owns this handle and no longer needs it.
            unsafe {
                let _ = windows_sys::Win32::Foundation::CloseHandle(thread);
            }
        }
    }
}

/// Writable child stdin handle.
#[cfg(windows)]
pub struct SandboxChildStdin(File);

#[cfg(windows)]
impl Write for SandboxChildStdin {
    fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        self.0.write(buf)
    }

    fn flush(&mut self) -> std::io::Result<()> {
        self.0.flush()
    }
}

/// Readable child stdout handle.
#[cfg(windows)]
pub struct SandboxChildStdout(File);

#[cfg(windows)]
impl Read for SandboxChildStdout {
    fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
        self.0.read(buf)
    }
}

/// Readable child stderr handle.
#[cfg(windows)]
pub struct SandboxChildStderr(File);

#[cfg(windows)]
impl Read for SandboxChildStderr {
    fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
        self.0.read(buf)
    }
}

#[cfg(windows)]
fn read_all(pipe: &mut impl Read) -> Vec<u8> {
    let mut bytes = Vec::new();
    let _ = pipe.read_to_end(&mut bytes);
    bytes
}

#[cfg(windows)]
fn no_child() -> std::io::Error {
    std::io::Error::new(
        std::io::ErrorKind::NotFound,
        "sandbox child is no longer available",
    )
}
