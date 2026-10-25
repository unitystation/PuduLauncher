//! Sandboxed process creation through a regular AppContainer.
//!
//! `rappct` provides profile/ACL/capability helpers, but its published launch
//! helper has a known attribute-list/handle-list defect that fails with
//! ERROR_INVALID_PARAMETER on current Windows builds. This module implements
//! the launch itself with a single `STARTUPINFOEXW`: SECURITY_CAPABILITIES,
//! HANDLE_LIST, suspended creation, Job assignment before resume.

use std::collections::BTreeMap;
use std::ffi::{OsStr, OsString};
use std::fs::{File, OpenOptions};
use std::io;
use std::os::windows::io::{AsRawHandle, FromRawHandle, RawHandle};
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};

use windows_sys::Win32::Foundation::{
    CloseHandle, GetLastError, LocalFree, SetHandleInformation, ERROR_PIPE_CONNECTED, GENERIC_READ,
    GENERIC_WRITE, HANDLE, HANDLE_FLAG_INHERIT, HLOCAL, INVALID_HANDLE_VALUE,
};
use windows_sys::Win32::Security::Authorization::{
    ConvertSidToStringSidW, ConvertStringSecurityDescriptorToSecurityDescriptorW,
    ConvertStringSidToSidW,
};
use windows_sys::Win32::Security::SECURITY_ATTRIBUTES;
use windows_sys::Win32::Security::{
    GetTokenInformation, TokenUser, PSECURITY_DESCRIPTOR, PSID, SECURITY_CAPABILITIES,
    SID_AND_ATTRIBUTES, TOKEN_QUERY, TOKEN_USER,
};
use windows_sys::Win32::Storage::FileSystem::{
    CreateFileW, FILE_FLAG_FIRST_PIPE_INSTANCE, OPEN_EXISTING, PIPE_ACCESS_INBOUND,
    PIPE_ACCESS_OUTBOUND,
};
use windows_sys::Win32::System::Console::{
    GetStdHandle, STD_ERROR_HANDLE, STD_INPUT_HANDLE, STD_OUTPUT_HANDLE,
};
use windows_sys::Win32::System::Pipes::{
    ConnectNamedPipe, CreateNamedPipeW, PIPE_REJECT_REMOTE_CLIENTS, PIPE_TYPE_BYTE, PIPE_WAIT,
};
use windows_sys::Win32::System::Threading::{
    CreateProcessW, DeleteProcThreadAttributeList, GetCurrentProcess,
    InitializeProcThreadAttributeList, OpenProcessToken, ResumeThread, UpdateProcThreadAttribute,
    CREATE_NO_WINDOW, CREATE_SUSPENDED, CREATE_UNICODE_ENVIRONMENT, EXTENDED_STARTUPINFO_PRESENT,
    LPPROC_THREAD_ATTRIBUTE_LIST, PROCESS_INFORMATION, PROC_THREAD_ATTRIBUTE_HANDLE_LIST,
    PROC_THREAD_ATTRIBUTE_SECURITY_CAPABILITIES, STARTF_USESTDHANDLES, STARTUPINFOEXW,
};

use crate::child::SandboxChild;
use crate::job::Job;
use crate::sandbox::Sandbox;
use crate::{BackendKind, ResourceLimits, SandboxError, Stdio};

use super::AppContainerSandbox;

struct StdioSetup {
    startup: STARTUPINFOEXW,
    stdin_parent: Option<File>,
    stdout_parent: Option<File>,
    stderr_parent: Option<File>,
    child_handles: Vec<HANDLE>,
    owned_child_handles: Vec<HANDLE>,
    keepalive: Vec<File>,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum IoSlot {
    Stdin,
    Stdout,
    Stderr,
}

struct SecurityCaps {
    capabilities: SECURITY_CAPABILITIES,
    sid: HLOCAL,
}

/// Heap-backed descriptor used only while creating the private named pipes.
///
/// The descriptor grants the parent user and this exact AppContainer profile
/// access. The untrusted mandatory label permits the AppContainer's lower
/// integrity token to use the inherited pipe endpoints.
struct PipeSecurityDescriptor {
    descriptor: HLOCAL,
}

impl PipeSecurityDescriptor {
    fn new(appcontainer_sid: &str) -> Result<Self, SandboxError> {
        let user_sid = current_user_sid()?;
        let sddl =
            format!("D:P(A;;GA;;;{user_sid})(A;;GA;;;{appcontainer_sid})S:(ML;;NW;;;S-1-16-0)");
        let wide: Vec<u16> = sddl.encode_utf16().chain(std::iter::once(0)).collect();
        let mut descriptor: PSECURITY_DESCRIPTOR = std::ptr::null_mut();
        let mut descriptor_len = 0u32;
        // SAFETY: `wide` is a valid NUL-terminated SDDL string and the output
        // pointers are valid. The successful allocation is released by Drop.
        let ok = unsafe {
            ConvertStringSecurityDescriptorToSecurityDescriptorW(
                wide.as_ptr(),
                1,
                &mut descriptor,
                &mut descriptor_len,
            )
        };
        if ok == 0 || descriptor.is_null() {
            return Err(SandboxError::Io(io::Error::last_os_error()));
        }
        Ok(Self {
            descriptor: descriptor as HLOCAL,
        })
    }

    fn as_ptr(&self) -> PSECURITY_DESCRIPTOR {
        self.descriptor.cast()
    }
}

impl Drop for PipeSecurityDescriptor {
    fn drop(&mut self) {
        // SAFETY: the descriptor was allocated by the SDDL conversion API and
        // is freed exactly once after all pipe creation calls finish.
        unsafe {
            let _ = LocalFree(self.descriptor);
        }
    }
}

impl StdioSetup {
    fn close_owned_child_handles(&mut self) {
        for handle in self.owned_child_handles.drain(..) {
            if !handle.is_null() {
                // SAFETY: this is a child-side named-pipe handle created by
                // this setup and no longer needed once process creation ends.
                unsafe {
                    let _ = CloseHandle(handle);
                }
            }
        }
    }
}

impl Drop for StdioSetup {
    fn drop(&mut self) {
        // Close child endpoints on both success and failure paths. Inherited
        // console handles are deliberately not in this list.
        self.close_owned_child_handles();
    }
}

impl SecurityCaps {
    fn from_sddl(sddl: &str) -> Result<Self, SandboxError> {
        let wide: Vec<u16> = sddl.encode_utf16().chain(std::iter::once(0)).collect();
        let mut sid: PSID = std::ptr::null_mut();
        // SAFETY: `wide` is a valid null-terminated wide string and `sid` is a
        // valid out-parameter. The returned SID is owned by LocalAlloc.
        let ok = unsafe { ConvertStringSidToSidW(wide.as_ptr(), &mut sid) };
        if ok == 0 || sid.is_null() {
            return Err(SandboxError::PolicyCompileFailed {
                backend: BackendKind::AppContainer,
                reason: format!("ConvertStringSidToSidW failed for {sddl}"),
            });
        }
        let capabilities = SECURITY_CAPABILITIES {
            AppContainerSid: sid,
            Capabilities: std::ptr::null_mut::<SID_AND_ATTRIBUTES>(),
            CapabilityCount: 0,
            Reserved: 0,
        };
        Ok(Self {
            capabilities,
            sid: sid as HLOCAL,
        })
    }
}

impl Drop for SecurityCaps {
    fn drop(&mut self) {
        // SAFETY: `sid` was allocated by ConvertStringSidToSidW and is freed
        // exactly once after the launch call completes.
        unsafe {
            let _ = LocalFree(self.sid);
        }
    }
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
    let appcontainer = sandbox
        .appcontainer()
        .ok_or_else(|| SandboxError::PolicyCompileFailed {
            backend: BackendKind::AppContainer,
            reason: "AppContainer backend state was not initialized".into(),
        })?;
    let exe = resolve_program(&program)?;
    let command_line = build_command_line(&exe, &args)?;
    let env_block = build_env_block(env_clear, &envs, &removals);
    let current_dir = current_dir.as_deref().map(|path| {
        let value = path.to_str().ok_or_else(|| SandboxError::InvalidPath {
            path: path.to_path_buf(),
            reason: "working directory is not valid Unicode".into(),
        })?;
        Ok::<_, SandboxError>(
            value
                .encode_utf16()
                .chain(std::iter::once(0))
                .collect::<Vec<_>>(),
        )
    });
    let current_dir = current_dir.transpose()?;
    let mut stdio = setup_stdio(stdin, stdout, stderr, appcontainer.profile.sid.as_string())?;
    let limits = sandbox.limits();
    let timeout = sandbox.timeout();

    let spawned = launch(
        appcontainer,
        &exe,
        &command_line,
        current_dir.as_deref(),
        env_block.as_deref(),
        &stdio,
        limits,
    )?;

    // Keeping a parent copy of a child pipe endpoint suppresses EOF forever.
    // Close only the handles this setup created; never close inherited console
    // handles owned by the host process.
    stdio.close_owned_child_handles();

    Ok(SandboxChild::new(
        BackendKind::AppContainer,
        spawned.process,
        spawned.thread,
        spawned.pid,
        spawned.job,
        stdio.stdin_parent.take(),
        stdio.stdout_parent.take(),
        stdio.stderr_parent.take(),
        timeout,
    ))
}

pub(crate) struct SpawnedProcess {
    pub(crate) process: HANDLE,
    pub(crate) thread: HANDLE,
    pub(crate) pid: u32,
    pub(crate) job: Job,
}

fn launch(
    appcontainer: &AppContainerSandbox,
    exe: &std::path::Path,
    command_line: &[u16],
    current_dir: Option<&[u16]>,
    env_block: Option<&[u16]>,
    stdio: &StdioSetup,
    limits: ResourceLimits,
) -> Result<SpawnedProcess, SandboxError> {
    let caps = SecurityCaps::from_sddl(appcontainer.profile.sid.as_string())?;
    let mut command_line = command_line.to_vec();

    let attribute_count = 2u32;
    let mut needed = 0usize;
    // SAFETY: The first call intentionally passes NULL to query the required
    // buffer size; it fails with ERROR_INSUFFICIENT_BUFFER and fills `needed`.
    unsafe {
        InitializeProcThreadAttributeList(std::ptr::null_mut(), attribute_count, 0, &mut needed);
    }
    let mut attribute_buffer = vec![0u8; needed];
    let attribute_list = attribute_buffer.as_mut_ptr() as LPPROC_THREAD_ATTRIBUTE_LIST;
    // SAFETY: `attribute_buffer` is large enough and outlives the attribute
    // list; the list is initialized before use.
    let ok = unsafe {
        InitializeProcThreadAttributeList(attribute_list, attribute_count, 0, &mut needed)
    };
    if ok == 0 {
        return Err(SandboxError::PolicyCompileFailed {
            backend: BackendKind::AppContainer,
            reason: format!(
                "InitializeProcThreadAttributeList failed, Win32 error {}",
                unsafe { GetLastError() }
            ),
        });
    }

    // SAFETY: `caps` and `stdio.child_handles` outlive the CreateProcessW call;
    // the attribute list is valid until deleted below.
    let ok = unsafe {
        UpdateProcThreadAttribute(
            attribute_list,
            0,
            PROC_THREAD_ATTRIBUTE_SECURITY_CAPABILITIES as usize,
            (&raw const caps.capabilities).cast(),
            std::mem::size_of::<SECURITY_CAPABILITIES>(),
            std::ptr::null_mut(),
            std::ptr::null(),
        )
    };
    if ok == 0 {
        delete_attribute_list(attribute_list);
        return Err(SandboxError::PolicyCompileFailed {
            backend: BackendKind::AppContainer,
            reason: format!(
                "UpdateProcThreadAttribute(security) failed, Win32 error {}",
                unsafe { GetLastError() }
            ),
        });
    }
    let ok = unsafe {
        UpdateProcThreadAttribute(
            attribute_list,
            0,
            PROC_THREAD_ATTRIBUTE_HANDLE_LIST as usize,
            stdio.child_handles.as_ptr().cast(),
            std::mem::size_of::<HANDLE>() * stdio.child_handles.len(),
            std::ptr::null_mut(),
            std::ptr::null(),
        )
    };
    if ok == 0 {
        delete_attribute_list(attribute_list);
        return Err(SandboxError::PolicyCompileFailed {
            backend: BackendKind::AppContainer,
            reason: format!(
                "UpdateProcThreadAttribute(handles) failed, Win32 error {}",
                unsafe { GetLastError() }
            ),
        });
    }

    let mut startup = stdio.startup;
    startup.lpAttributeList = attribute_list;
    startup.StartupInfo.cb = std::mem::size_of::<STARTUPINFOEXW>() as u32;

    let exe_wide: Vec<u16> = exe
        .to_str()
        .ok_or_else(|| SandboxError::InvalidPath {
            path: exe.to_path_buf(),
            reason: "executable path is not valid Unicode".into(),
        })?
        .encode_utf16()
        .chain(std::iter::once(0))
        .collect();

    let mut process_information: PROCESS_INFORMATION = unsafe { std::mem::zeroed() };
    let creation_flags = EXTENDED_STARTUPINFO_PRESENT
        | CREATE_SUSPENDED
        | CREATE_NO_WINDOW
        | if env_block.is_some() {
            CREATE_UNICODE_ENVIRONMENT
        } else {
            0
        };

    // SAFETY: All pointers are valid and live through the call; the command
    // line buffer is mutable; reserved security attributes are NULL.
    let result = unsafe {
        CreateProcessW(
            exe_wide.as_ptr(),
            command_line.as_mut_ptr(),
            std::ptr::null(),
            std::ptr::null(),
            1,
            creation_flags,
            env_block
                .map(|block| block.as_ptr() as *const core::ffi::c_void)
                .unwrap_or(std::ptr::null()),
            current_dir
                .map(|dir| dir.as_ptr())
                .unwrap_or(std::ptr::null()),
            &mut startup as *mut STARTUPINFOEXW as *mut _,
            &mut process_information,
        )
    };
    delete_attribute_list(attribute_list);

    if result == 0 {
        let error = unsafe { GetLastError() };
        let env_diag = env_block
            .map(|block| {
                let first: Vec<String> = block
                    .split(|unit| *unit == 0)
                    .take(6)
                    .map(|part| String::from_utf16_lossy(part))
                    .collect();
                format!("env_len={} first={first:?}", block.len())
            })
            .unwrap_or_else(|| "env=none".into());
        return Err(SandboxError::ProcessCreationFailed {
            backend: BackendKind::AppContainer,
            win32_code: Some(error),
            message: format!("CreateProcessW failed: {error} ({env_diag})"),
        });
    }

    let job = Job::new(limits).map_err(|err| {
        terminate_and_reap(process_information.hProcess, process_information.hThread);
        err
    })?;
    if let Err(err) = job.assign_raw(process_information.hProcess) {
        let error = unsafe { GetLastError() };
        terminate_and_reap(process_information.hProcess, process_information.hThread);
        return Err(SandboxError::ProcessCreationFailed {
            backend: BackendKind::AppContainer,
            win32_code: Some(error),
            message: format!("job assignment failed: {err}"),
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
            backend: BackendKind::AppContainer,
            win32_code: Some(error),
            message: format!("ResumeThread failed: {error}"),
        });
    }

    Ok(SpawnedProcess {
        process: process_information.hProcess,
        thread: process_information.hThread,
        pid: process_information.dwProcessId,
        job,
    })
}

fn delete_attribute_list(attribute_list: LPPROC_THREAD_ATTRIBUTE_LIST) {
    // SAFETY: The list was initialized by InitializeProcThreadAttributeList and
    // is deleted exactly once after CreateProcessW.
    unsafe {
        DeleteProcThreadAttributeList(attribute_list);
    }
}

fn terminate_and_reap(process: HANDLE, thread: HANDLE) {
    // SAFETY: Both handles were returned by CreateProcessW and are still owned
    // by the failed spawn path.
    unsafe {
        let _ = windows_sys::Win32::System::Threading::TerminateProcess(process, 1);
        let _ = windows_sys::Win32::System::Threading::WaitForSingleObject(process, u32::MAX);
        let _ = CloseHandle(process);
        let _ = CloseHandle(thread);
    }
}

fn setup_stdio(
    stdin: Stdio,
    stdout: Stdio,
    stderr: Stdio,
    appcontainer_sid: &str,
) -> Result<StdioSetup, SandboxError> {
    let needs_pipe_security = matches!(stdin, Stdio::Piped)
        || matches!(stdout, Stdio::Piped)
        || matches!(stderr, Stdio::Piped);
    let pipe_security = needs_pipe_security
        .then(|| PipeSecurityDescriptor::new(appcontainer_sid))
        .transpose()?;
    let mut setup = StdioSetup {
        startup: unsafe { std::mem::zeroed() },
        stdin_parent: None,
        stdout_parent: None,
        stderr_parent: None,
        child_handles: Vec::new(),
        owned_child_handles: Vec::new(),
        keepalive: Vec::new(),
    };

    setup.startup.StartupInfo.cb = std::mem::size_of::<STARTUPINFOEXW>() as u32;
    let stdin_child = prepare_handle(stdin, IoSlot::Stdin, &mut setup, pipe_security.as_ref())?;
    let stdout_child = prepare_handle(stdout, IoSlot::Stdout, &mut setup, pipe_security.as_ref())?;
    let stderr_child = prepare_handle(stderr, IoSlot::Stderr, &mut setup, pipe_security.as_ref())?;

    setup.startup.StartupInfo.dwFlags = STARTF_USESTDHANDLES;
    setup.startup.StartupInfo.hStdInput = stdin_child;
    setup.startup.StartupInfo.hStdOutput = stdout_child;
    setup.startup.StartupInfo.hStdError = stderr_child;
    Ok(setup)
}

fn prepare_handle(
    mode: Stdio,
    slot: IoSlot,
    setup: &mut StdioSetup,
    pipe_security: Option<&PipeSecurityDescriptor>,
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
            setup.child_handles.push(handle);
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
            setup.child_handles.push(handle);
            setup.keepalive.push(file);
            Ok(handle)
        }
        Stdio::Piped => {
            let security = pipe_security.ok_or_else(|| SandboxError::PolicyCompileFailed {
                backend: BackendKind::AppContainer,
                reason: "missing named-pipe security descriptor".into(),
            })?;
            let (file, child_end) = create_piped_handle(slot, security)?;
            if let Err(err) = make_inheritable(child_end) {
                // SAFETY: `child_end` is owned by this setup until it is
                // successfully registered for launch below.
                unsafe {
                    let _ = CloseHandle(child_end);
                }
                return Err(err);
            }
            setup.child_handles.push(child_end);
            setup.owned_child_handles.push(child_end);
            match slot {
                IoSlot::Stdin => setup.stdin_parent = Some(file),
                IoSlot::Stdout => setup.stdout_parent = Some(file),
                IoSlot::Stderr => setup.stderr_parent = Some(file),
            }
            Ok(child_end)
        }
    }
}

fn create_piped_handle(
    slot: IoSlot,
    security: &PipeSecurityDescriptor,
) -> Result<(File, HANDLE), SandboxError> {
    let name = next_pipe_name(slot);
    let wide_name: Vec<u16> = name.encode_utf16().chain(std::iter::once(0)).collect();
    let server_access = match slot {
        // The host owns the server endpoint. It writes stdin and reads the
        // child's stdout/stderr; the inherited client end has the inverse.
        IoSlot::Stdin => PIPE_ACCESS_OUTBOUND,
        IoSlot::Stdout | IoSlot::Stderr => PIPE_ACCESS_INBOUND,
    };
    let client_access = match slot {
        IoSlot::Stdin => GENERIC_READ,
        IoSlot::Stdout | IoSlot::Stderr => GENERIC_WRITE,
    };
    let attributes = SECURITY_ATTRIBUTES {
        nLength: std::mem::size_of::<SECURITY_ATTRIBUTES>() as u32,
        lpSecurityDescriptor: security.as_ptr(),
        bInheritHandle: 0,
    };

    // Use a named pipe rather than CreatePipe so the full security descriptor,
    // including its mandatory integrity label, is applied to the pipe object.
    let server = unsafe {
        CreateNamedPipeW(
            wide_name.as_ptr(),
            server_access | FILE_FLAG_FIRST_PIPE_INSTANCE,
            PIPE_TYPE_BYTE | PIPE_WAIT | PIPE_REJECT_REMOTE_CLIENTS,
            1,
            64 * 1024,
            64 * 1024,
            0,
            &attributes,
        )
    };
    if server == INVALID_HANDLE_VALUE {
        return Err(SandboxError::Io(io::Error::last_os_error()));
    }

    // The parent opens the client endpoint before process creation, then the
    // handle list transfers it to the AppContainer child.
    let client = unsafe {
        CreateFileW(
            wide_name.as_ptr(),
            client_access,
            0,
            std::ptr::null(),
            OPEN_EXISTING,
            0,
            std::ptr::null_mut(),
        )
    };
    if client == INVALID_HANDLE_VALUE {
        let error = io::Error::last_os_error();
        unsafe {
            let _ = CloseHandle(server);
        }
        return Err(SandboxError::Io(error));
    }

    // Opening the client end races ConnectNamedPipe by design; a successful
    // connection is reported as ERROR_PIPE_CONNECTED in that case.
    let connected = unsafe { ConnectNamedPipe(server, std::ptr::null_mut()) };
    let connect_error = unsafe { GetLastError() };
    if connected == 0 && connect_error != ERROR_PIPE_CONNECTED {
        let error = io::Error::from_raw_os_error(connect_error as i32);
        unsafe {
            let _ = CloseHandle(client);
            let _ = CloseHandle(server);
        }
        return Err(SandboxError::Io(error));
    }

    // SAFETY: `server` is the parent endpoint and ownership transfers to the
    // returned File. `client` remains a raw child endpoint until launch ends.
    Ok((
        unsafe { File::from_raw_handle(server as RawHandle) },
        client,
    ))
}

fn next_pipe_name(slot: IoSlot) -> String {
    static NEXT_PIPE_ID: AtomicU64 = AtomicU64::new(0);
    let slot = match slot {
        IoSlot::Stdin => "stdin",
        IoSlot::Stdout => "stdout",
        IoSlot::Stderr => "stderr",
    };
    let sequence = NEXT_PIPE_ID.fetch_add(1, Ordering::Relaxed);
    let nonce = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|duration| duration.as_nanos())
        .unwrap_or_default();
    format!(
        r"\\.\pipe\sandboxrs-{}-{nonce:x}-{sequence:x}-{slot}",
        std::process::id()
    )
}

fn current_user_sid() -> Result<String, SandboxError> {
    let mut token = std::ptr::null_mut();
    // SAFETY: GetCurrentProcess returns a pseudo-handle valid for this call;
    // `token` is a valid out-parameter and is closed below.
    let opened = unsafe { OpenProcessToken(GetCurrentProcess(), TOKEN_QUERY, &mut token) };
    if opened == 0 {
        return Err(SandboxError::Io(io::Error::last_os_error()));
    }

    let result = (|| {
        let mut required = 0u32;
        // SAFETY: This sizing call intentionally has a null buffer.
        unsafe {
            let _ = GetTokenInformation(token, TokenUser, std::ptr::null_mut(), 0, &mut required);
        }
        if required == 0 {
            return Err(SandboxError::Io(io::Error::last_os_error()));
        }
        let mut buffer = vec![0u8; required as usize];
        // SAFETY: `buffer` is large enough according to the sizing call.
        let read = unsafe {
            GetTokenInformation(
                token,
                TokenUser,
                buffer.as_mut_ptr().cast(),
                required,
                &mut required,
            )
        };
        if read == 0 {
            return Err(SandboxError::Io(io::Error::last_os_error()));
        }
        // TOKEN_USER may not be aligned to Rust's stricter reference rules in
        // a Vec<u8>, so copy the C header with an unaligned read.
        let user = unsafe { std::ptr::read_unaligned(buffer.as_ptr().cast::<TOKEN_USER>()) };
        let mut string_sid = std::ptr::null_mut();
        // SAFETY: `user.User.Sid` points into the live token-information
        // buffer. The successful allocation is released before returning.
        let converted = unsafe { ConvertSidToStringSidW(user.User.Sid, &mut string_sid) };
        if converted == 0 || string_sid.is_null() {
            return Err(SandboxError::Io(io::Error::last_os_error()));
        }
        let mut len = 0usize;
        // SAFETY: ConvertSidToStringSidW returns a NUL-terminated wide string.
        unsafe {
            while *string_sid.add(len) != 0 {
                len += 1;
            }
        }
        let value =
            unsafe { String::from_utf16_lossy(std::slice::from_raw_parts(string_sid, len)) };
        unsafe {
            let _ = LocalFree(string_sid.cast());
        }
        Ok(value)
    })();

    // SAFETY: OpenProcessToken returned this owned token handle above.
    unsafe {
        let _ = CloseHandle(token);
    }
    result
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

fn resolve_program(program: &OsStr) -> Result<PathBuf, SandboxError> {
    let path = PathBuf::from(program);
    if path.is_absolute() {
        return Ok(path);
    }
    let extensions = ["", ".exe", ".com", ".bat", ".cmd"];
    let search = std::env::var_os("PATH")
        .map(|value| std::env::split_paths(&value).collect::<Vec<_>>())
        .unwrap_or_default();
    for dir in search {
        for extension in extensions {
            let candidate = dir.join(format!("{}{}", program.to_string_lossy(), extension));
            if candidate.is_file() {
                return Ok(candidate);
            }
        }
    }
    Err(SandboxError::ProcessCreationFailed {
        backend: BackendKind::AppContainer,
        win32_code: None,
        message: format!("could not resolve executable {:?} on PATH", program),
    })
}

fn build_command_line(exe: &std::path::Path, args: &[OsString]) -> Result<Vec<u16>, SandboxError> {
    let mut parts = vec![quote_arg(exe.as_os_str())?];
    parts.extend(
        args.iter()
            .map(|arg| quote_arg(arg.as_os_str()))
            .collect::<Result<Vec<_>, _>>()?,
    );
    Ok(parts
        .join(" ")
        .encode_utf16()
        .chain(std::iter::once(0))
        .collect())
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

fn build_env_block(
    env_clear: bool,
    envs: &BTreeMap<OsString, OsString>,
    removals: &[OsString],
) -> Option<Vec<u16>> {
    let mut entries = build_env_entries(env_clear, envs, removals);
    if entries.is_empty() {
        return None;
    }
    entries.sort_by(|a, b| {
        a.0.to_string_lossy()
            .to_ascii_uppercase()
            .cmp(&b.0.to_string_lossy().to_ascii_uppercase())
    });
    entries.dedup_by(|a, b| {
        a.0.to_string_lossy()
            .to_ascii_lowercase()
            .eq(&b.0.to_string_lossy().to_ascii_lowercase())
    });
    let mut block = Vec::new();
    for (key, value) in entries {
        block.extend(
            format!("{}={}", key.to_string_lossy(), value.to_string_lossy()).encode_utf16(),
        );
        block.push(0);
    }
    block.push(0);
    Some(block)
}

fn build_env_entries(
    env_clear: bool,
    envs: &BTreeMap<OsString, OsString>,
    removals: &[OsString],
) -> Vec<(OsString, OsString)> {
    let mut entries: Vec<(OsString, OsString)> = if env_clear {
        // A cleared environment still needs the parent's hidden drive
        // current-directory variables; CreateProcessW fails with error 203
        // when they are missing from a custom block.
        hidden_env_entries()
    } else {
        std::env::vars_os().collect()
    };
    for removal in removals {
        let removed = removal.to_string_lossy().to_uppercase();
        entries.retain(|(key, _)| key.to_string_lossy().to_uppercase() != removed);
    }
    for (key, value) in envs {
        let upper = key.to_string_lossy().to_uppercase();
        entries.retain(|(existing, _)| existing.to_string_lossy().to_uppercase() != upper);
        entries.push((key.clone(), value.clone()));
    }
    // Always include essential Windows variables so sandboxed children can
    // load when the parent environment is cleared.
    if entries.is_empty() {
        entries.extend(essential_env_entries());
    }
    // Some launch contexts (Start-Process -Environment) remove the hidden
    // drive current-directory variables from the process block entirely, so
    // GetEnvironmentStringsW cannot recover them. Enumerate the real drives
    // and synthesize the entries from the current directory.
    if env_clear {
        entries.extend(drive_current_dir_entries());
    }
    entries
}

#[cfg(windows)]
fn essential_env_entries() -> Vec<(OsString, OsString)> {
    let keys = [
        "SystemRoot",
        "windir",
        "SystemDrive",
        "ComSpec",
        "PATHEXT",
        "TEMP",
        "TMP",
        "PATH",
        "USERPROFILE",
        "HOMEDRIVE",
        "HOMEPATH",
        "ALLUSERSPROFILE",
        "APPDATA",
        "LOCALAPPDATA",
        "PROGRAMDATA",
        "PROGRAMFILES",
        "PROGRAMFILES(X86)",
        "CommonProgramFiles",
        "CommonProgramFiles(X86)",
        "OS",
        "NUMBER_OF_PROCESSORS",
        "PROCESSOR_ARCHITECTURE",
        "PROCESSOR_IDENTIFIER",
        "PROCESSOR_LEVEL",
        "PROCESSOR_REVISION",
        "PUBLIC",
        "COMPUTERNAME",
        "USERNAME",
        "USERDOMAIN",
        "SESSIONNAME",
    ];
    let mut entries = Vec::new();
    for key in keys {
        if let Some(value) = std::env::var_os(key) {
            entries.push((OsString::from(key), value));
        }
    }
    entries
}

#[cfg(windows)]
fn hidden_env_entries() -> Vec<(OsString, OsString)> {
    use windows_sys::Win32::System::Environment::{
        FreeEnvironmentStringsW, GetEnvironmentStringsW,
    };

    let mut entries = Vec::new();
    // SAFETY: GetEnvironmentStringsW returns a read-only double-null-terminated
    // block owned by the caller until FreeEnvironmentStringsW is called.
    let raw = unsafe { GetEnvironmentStringsW() };
    if raw.is_null() {
        return entries;
    }
    let mut cursor = raw;
    // SAFETY: `cursor` walks the returned NUL-separated block and never reads
    // past its terminating double NUL.
    unsafe {
        loop {
            let mut end = cursor;
            while *end != 0 {
                end = end.add(1);
            }
            if end == cursor {
                break;
            }
            let len = end.offset_from(cursor) as usize;
            let slice = std::slice::from_raw_parts(cursor, len);
            let entry = String::from_utf16_lossy(slice);
            if entry.starts_with('=') {
                if let Some((key, value)) = entry.split_once('=') {
                    entries.push((OsString::from(key), OsString::from(value)));
                }
            }
            cursor = end.add(1);
        }
        let _ = FreeEnvironmentStringsW(raw);
    }
    entries
}

#[cfg(windows)]
fn drive_current_dir_entries() -> Vec<(OsString, OsString)> {
    use windows_sys::Win32::Storage::FileSystem::GetLogicalDrives;
    use windows_sys::Win32::System::Environment::GetCurrentDirectoryW;

    let mut entries = Vec::new();
    let mut current_dir = [0u16; 32768];
    // SAFETY: `current_dir` is a valid writable buffer and GetCurrentDirectoryW
    // fills it with a NUL-terminated path.
    let len = unsafe { GetCurrentDirectoryW(current_dir.len() as u32, current_dir.as_mut_ptr()) };
    if len == 0 || len as usize >= current_dir.len() {
        return entries;
    }
    let current_dir = String::from_utf16_lossy(&current_dir[..len as usize]);
    let current_drive = current_dir
        .chars()
        .next()
        .map(|ch| ch.to_ascii_uppercase())
        .unwrap_or('C');

    // SAFETY: GetLogicalDrives returns a bitmask without side effects.
    let drives = unsafe { GetLogicalDrives() };
    for offset in 0..26 {
        if drives & (1u32 << offset) != 0 {
            let letter = (b'A' + offset) as char;
            let value = if letter == current_drive {
                current_dir.clone()
            } else {
                format!("{letter}:\\")
            };
            entries.push((OsString::from(format!("={letter}:")), OsString::from(value)));
        }
    }
    entries
}
