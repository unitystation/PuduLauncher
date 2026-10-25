//! Sandboxed process creation through `Experimental_CreateProcessInSandbox`.

use std::collections::BTreeMap;
use std::ffi::{OsStr, OsString};
use std::fs::{File, OpenOptions};
use std::io;
use std::os::windows::io::{AsRawHandle, FromRawHandle, RawHandle};
use std::path::PathBuf;

use windows_sys::Win32::Foundation::{
    CloseHandle, GetLastError, SetHandleInformation, HANDLE, HANDLE_FLAG_INHERIT,
};
use windows_sys::Win32::Security::SECURITY_ATTRIBUTES;
use windows_sys::Win32::System::Console::{
    GetStdHandle, STD_ERROR_HANDLE, STD_INPUT_HANDLE, STD_OUTPUT_HANDLE,
};
use windows_sys::Win32::System::Pipes::CreatePipe;
use windows_sys::Win32::System::Threading::{
    ResumeThread, TerminateProcess, WaitForSingleObject, CREATE_NO_WINDOW, CREATE_SUSPENDED,
    CREATE_UNICODE_ENVIRONMENT, PROCESS_INFORMATION, STARTF_USESTDHANDLES, STARTUPINFOW,
};

use crate::child::SandboxChild;
use crate::job::Job;
use crate::sandbox::Sandbox;
use crate::{BackendKind, ResourceLimits, SandboxError, Stdio};

use super::ffi::{create_api, to_wide};
use super::spec::build_sandbox_spec;

pub(crate) struct SpawnedProcess {
    pub(crate) process: HANDLE,
    pub(crate) thread: HANDLE,
    pub(crate) pid: u32,
    pub(crate) job: Job,
}

struct StdioSetup {
    startup: STARTUPINFOW,
    stdin_parent: Option<File>,
    stdout_parent: Option<File>,
    stderr_parent: Option<File>,
    child_ends: Vec<HANDLE>,
    keepalive: Vec<File>,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum IoSlot {
    Stdin,
    Stdout,
    Stderr,
}

pub(crate) fn spawn(
    sandbox: &Sandbox,
    program: OsString,
    args: Vec<OsString>,
    env_clear: bool,
    envs: BTreeMap<OsString, OsString>,
    removals: Vec<OsString>,
    current_dir: Option<PathBuf>,
    stdin: Stdio,
    stdout: Stdio,
    stderr: Stdio,
) -> Result<SandboxChild, SandboxError> {
    let spec = build_sandbox_spec(sandbox.plan())?;
    let identity = to_wide(sandbox.identity());
    let command_line = build_command_line(&program, &args)?;
    let env_block = build_env_block(env_clear, &envs, &removals);
    let current_dir = current_dir
        .as_deref()
        .map(|path| {
            let value = path.to_str().ok_or_else(|| SandboxError::InvalidPath {
                path: path.to_path_buf(),
                reason: "working directory is not valid Unicode".into(),
            })?;
            Ok::<_, SandboxError>(to_wide(value))
        })
        .transpose()?;

    let stdio = setup_stdio(stdin, stdout, stderr)?;
    let limits = sandbox.limits();
    let timeout = sandbox.timeout();

    let spawned = launch(
        &command_line,
        current_dir.as_deref(),
        env_block.as_deref(),
        &stdio.startup,
        &identity,
        &spec,
        limits,
    )?;

    close_child_ends(&stdio.child_ends);

    Ok(SandboxChild::new(
        BackendKind::WindowsSandboxApi,
        spawned.process,
        spawned.thread,
        spawned.pid,
        spawned.job,
        stdio.stdin_parent,
        stdio.stdout_parent,
        stdio.stderr_parent,
        timeout,
    ))
}

pub(crate) fn launch(
    command_line: &[u16],
    current_dir: Option<&[u16]>,
    env_block: Option<&[u16]>,
    startup: &STARTUPINFOW,
    identity: &[u16],
    spec: &[u8],
    limits: ResourceLimits,
) -> Result<SpawnedProcess, SandboxError> {
    let api = create_api().map_err(|message| SandboxError::BackendProbeFailed {
        backend: BackendKind::WindowsSandboxApi,
        source: std::io::Error::other(message).into(),
    })?;

    let mut command_line = command_line.to_vec();
    let mut process_information: PROCESS_INFORMATION;

    let mut environment = env_block.map(|block| block.as_ptr() as *const core::ffi::c_void);
    let mut creation_flags = CREATE_SUSPENDED
        | CREATE_NO_WINDOW
        | if environment.is_some() {
            CREATE_UNICODE_ENVIRONMENT
        } else {
            0
        };
    let mut retries_remaining = if environment.is_some() { 1 } else { 0 };

    let success = loop {
        process_information = unsafe { std::mem::zeroed() };
        // SAFETY: All pointers are valid for the call. `command_line` is a
        // mutable, double-terminated wide buffer; reserved parameters are
        // NULL/FALSE as the experimental API requires; the identity and spec
        // buffers outlive the call.
        let result = unsafe {
            api(
                std::ptr::null(),
                command_line.as_mut_ptr(),
                std::ptr::null(),
                std::ptr::null(),
                0,
                creation_flags,
                environment.unwrap_or(std::ptr::null()),
                current_dir
                    .map(|dir| dir.as_ptr())
                    .unwrap_or(std::ptr::null()),
                startup,
                identity.as_ptr(),
                spec.as_ptr(),
                spec.len() as u32,
                &mut process_information,
            )
        };
        if result != 0 {
            break true;
        }

        let error = unsafe { GetLastError() };
        if retries_remaining > 0 && environment.is_some() && error == 50
        /* ERROR_NOT_SUPPORTED */
        {
            cleanup_process_info(&process_information);
            environment = None;
            creation_flags &= !CREATE_UNICODE_ENVIRONMENT;
            retries_remaining -= 1;
            continue;
        }

        cleanup_process_info(&process_information);
        return Err(SandboxError::ProcessCreationFailed {
            backend: BackendKind::WindowsSandboxApi,
            win32_code: Some(error),
            message: describe_win32_error(error),
        });
    };
    debug_assert!(success);

    let job = Job::new(limits).map_err(|err| {
        terminate_and_reap(process_information.hProcess, process_information.hThread);
        err
    })?;

    // SAFETY: `process_information.hProcess` is a live process handle owned by
    // this function until ownership transfers to `SpawnedProcess`; the job
    // handle remains valid for the lifetime of `job`.
    if let Err(_) = job.assign_raw(process_information.hProcess) {
        let error = unsafe { GetLastError() };
        terminate_and_reap(process_information.hProcess, process_information.hThread);
        return Err(SandboxError::ProcessCreationFailed {
            backend: BackendKind::WindowsSandboxApi,
            win32_code: Some(error),
            message: format!(
                "process could not be assigned to the lifecycle job: {}",
                describe_win32_error(error)
            ),
        });
    }

    // SAFETY: `hThread` is the just-created main thread handle; ResumeThread
    // only adjusts its suspend count.
    let resumed = unsafe { ResumeThread(process_information.hThread) };
    if resumed == u32::MAX {
        let error = unsafe { GetLastError() };
        let _ = job.terminate();
        terminate_and_reap(process_information.hProcess, process_information.hThread);
        return Err(SandboxError::ProcessCreationFailed {
            backend: BackendKind::WindowsSandboxApi,
            win32_code: Some(error),
            message: describe_win32_error(error),
        });
    }

    Ok(SpawnedProcess {
        process: process_information.hProcess,
        thread: process_information.hThread,
        pid: process_information.dwProcessId,
        job,
    })
}

fn terminate_and_reap(process: HANDLE, thread: HANDLE) {
    // SAFETY: Both handles were returned by the create API and are still owned
    // by the failed spawn path.
    unsafe {
        let _ = TerminateProcess(process, 1);
        let _ = WaitForSingleObject(process, u32::MAX);
        let _ = CloseHandle(process);
        let _ = CloseHandle(thread);
    }
}

pub(crate) fn cleanup_process_info(process_information: &PROCESS_INFORMATION) {
    // SAFETY: Handles are only closed when the failed call populated them.
    unsafe {
        if !process_information.hProcess.is_null() {
            let _ = CloseHandle(process_information.hProcess);
        }
        if !process_information.hThread.is_null() {
            let _ = CloseHandle(process_information.hThread);
        }
    }
}

fn setup_stdio(stdin: Stdio, stdout: Stdio, stderr: Stdio) -> Result<StdioSetup, SandboxError> {
    let mut setup = StdioSetup {
        startup: unsafe { std::mem::zeroed() },
        stdin_parent: None,
        stdout_parent: None,
        stderr_parent: None,
        child_ends: Vec::new(),
        keepalive: Vec::new(),
    };
    setup.startup.cb = std::mem::size_of::<STARTUPINFOW>() as u32;

    let stdin_child = prepare_handle(stdin, IoSlot::Stdin, &mut setup)?;
    let stdout_child = prepare_handle(stdout, IoSlot::Stdout, &mut setup)?;
    let stderr_child = prepare_handle(stderr, IoSlot::Stderr, &mut setup)?;

    setup.startup.dwFlags = STARTF_USESTDHANDLES;
    setup.startup.hStdInput = stdin_child;
    setup.startup.hStdOutput = stdout_child;
    setup.startup.hStdError = stderr_child;
    Ok(setup)
}

/// Returns the child-side handle to put into STARTUPINFOW.
fn prepare_handle(
    mode: Stdio,
    slot: IoSlot,
    setup: &mut StdioSetup,
) -> Result<HANDLE, SandboxError> {
    match mode {
        Stdio::Inherit => {
            let handle = unsafe {
                GetStdHandle(match slot {
                    IoSlot::Stdin => STD_INPUT_HANDLE,
                    IoSlot::Stdout => STD_OUTPUT_HANDLE,
                    IoSlot::Stderr => STD_ERROR_HANDLE,
                })
            };
            if handle.is_null() {
                return Err(SandboxError::Io(io::Error::last_os_error()));
            }
            make_inheritable(handle)?;
            Ok(handle)
        }
        Stdio::Null => {
            let file = OpenOptions::new()
                .read(true)
                .write(true)
                .open("NUL")
                .map_err(SandboxError::Io)?;
            let handle = file.as_raw_handle();
            make_inheritable(handle)?;
            setup.keepalive.push(file);
            Ok(handle)
        }
        Stdio::Piped => {
            let attributes = SECURITY_ATTRIBUTES {
                nLength: std::mem::size_of::<SECURITY_ATTRIBUTES>() as u32,
                lpSecurityDescriptor: std::ptr::null_mut(),
                bInheritHandle: 1,
            };
            let mut read_end: HANDLE = std::ptr::null_mut();
            let mut write_end: HANDLE = std::ptr::null_mut();
            // SAFETY: Both out-parameters are valid, and the security
            // attributes structure is fully initialized.
            let ok = unsafe { CreatePipe(&mut read_end, &mut write_end, &attributes, 0) };
            if ok == 0 {
                return Err(SandboxError::Io(io::Error::last_os_error()));
            }

            let (child_end, parent_end) = match slot {
                IoSlot::Stdin => (write_end, read_end),
                IoSlot::Stdout | IoSlot::Stderr => (read_end, write_end),
            };
            // SAFETY: The parent end is a valid pipe handle created above and
            // ownership transfers to the returned File.
            let file = unsafe { File::from_raw_handle(parent_end as RawHandle) };
            make_inheritable(child_end)?;
            setup.child_ends.push(child_end);

            match slot {
                IoSlot::Stdin => setup.stdin_parent = Some(file),
                IoSlot::Stdout => setup.stdout_parent = Some(file),
                IoSlot::Stderr => setup.stderr_parent = Some(file),
            }
            Ok(child_end)
        }
    }
}

fn make_inheritable(handle: HANDLE) -> Result<(), SandboxError> {
    // SAFETY: `handle` is a valid handle owned by the caller; the flags only
    // adjust inheritance on that handle.
    let ok = unsafe { SetHandleInformation(handle, HANDLE_FLAG_INHERIT, HANDLE_FLAG_INHERIT) };
    if ok == 0 {
        return Err(SandboxError::Io(io::Error::last_os_error()));
    }
    Ok(())
}

fn close_child_ends(handles: &[HANDLE]) {
    for &handle in handles {
        if !handle.is_null() {
            // SAFETY: These are child-side pipe handles created by this spawn
            // and no longer needed after process creation.
            unsafe {
                let _ = CloseHandle(handle);
            }
        }
    }
}

fn build_command_line(program: &OsStr, args: &[OsString]) -> Result<Vec<u16>, SandboxError> {
    let mut parts = Vec::with_capacity(args.len() + 1);
    parts.push(quote_arg(program)?);
    parts.extend(
        args.iter()
            .map(|arg| quote_arg(arg.as_os_str()))
            .collect::<Result<Vec<_>, _>>()?,
    );
    Ok(to_wide(&parts.join(" ")))
}

fn quote_arg(arg: &OsStr) -> Result<String, SandboxError> {
    let value = arg.to_string_lossy();
    let mut quoted = String::with_capacity(value.len() + 2);
    quoted.push('"');
    let mut backslashes = 0usize;
    for ch in value.chars() {
        if ch == '\\' {
            backslashes += 1;
        } else if ch == '"' {
            for _ in 0..(2 * backslashes + 1) {
                quoted.push('\\');
            }
            quoted.push('"');
            backslashes = 0;
        } else {
            for _ in 0..backslashes {
                quoted.push('\\');
            }
            quoted.push(ch);
            backslashes = 0;
        }
    }
    for _ in 0..(2 * backslashes) {
        quoted.push('\\');
    }
    quoted.push('"');
    Ok(quoted)
}

pub(crate) fn build_env_block(
    env_clear: bool,
    envs: &BTreeMap<OsString, OsString>,
    removals: &[OsString],
) -> Option<Vec<u16>> {
    let mut entries: Vec<(String, String)> = if env_clear {
        Vec::new()
    } else {
        std::env::vars_os()
            .map(|(key, value)| {
                (
                    key.to_string_lossy().into_owned(),
                    value.to_string_lossy().into_owned(),
                )
            })
            .collect()
    };

    for removal in removals {
        let removed = removal.to_string_lossy().to_uppercase();
        entries.retain(|(key, _)| key.to_uppercase() != removed);
    }
    for (key, value) in envs {
        let key = key.to_string_lossy().into_owned();
        let upper = key.to_uppercase();
        entries.retain(|(existing, _)| existing.to_uppercase() != upper);
        entries.push((key, value.to_string_lossy().into_owned()));
    }

    if entries.is_empty() && !env_clear && envs.is_empty() && removals.is_empty() {
        return None;
    }

    entries.sort_by(|a, b| a.0.to_ascii_uppercase().cmp(&b.0.to_ascii_uppercase()));
    let mut block = Vec::new();
    for (key, value) in entries {
        block.extend(format!("{key}={value}").encode_utf16());
        block.push(0);
    }
    block.push(0);
    Some(block)
}

fn describe_win32_error(code: u32) -> String {
    match code {
        50 => "ERROR_NOT_SUPPORTED: the sandbox feature or requested policy is not supported on this build".into(),
        87 => "ERROR_INVALID_PARAMETER: a sandbox specification or parameter was invalid".into(),
        998 => "ERROR_NOACCESS: the engine rejected the sandbox specification buffer".into(),
        _ => format!("Win32 error {code}"),
    }
}
