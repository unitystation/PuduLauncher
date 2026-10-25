//! M0 capability probe: a real, minimal sandbox launch with no admin.

use std::collections::BTreeMap;
use std::ffi::OsString;
use std::fs;
use std::path::PathBuf;
use std::time::SystemTime;

use windows_sys::Win32::Foundation::{CloseHandle, GetLastError};
use windows_sys::Win32::System::Threading::{
    GetExitCodeProcess, WaitForSingleObject, STARTUPINFOW,
};

use crate::filesystem::FilesystemPlan;
use crate::ResourceLimits;

use super::ffi::to_wide;
use super::spawn::{build_env_block, launch};
use super::spec::build_sandbox_spec;

pub(crate) fn launch_probe() -> Result<(), String> {
    let nonce = SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_err(|err| err.to_string())?
        .as_nanos() as u64;

    let temp = std::env::temp_dir();
    let workspace = temp.join(format!("sandboxrs-probe-{nonce:x}"));
    let outside_file = temp.join(format!("sandboxrs-probe-out-{nonce:x}.txt"));
    let system_root = std::env::var("SystemRoot").unwrap_or_else(|_| r"C:\Windows".to_string());

    let probe_result = (|| {
        fs::create_dir_all(&workspace).map_err(|err| err.to_string())?;

        let plan = FilesystemPlan::compile(&workspace, &[PathBuf::from(system_root)], &[])
            .map_err(|err| err.to_string())?;
        let spec = build_sandbox_spec(&plan).map_err(|err| err.to_string())?;
        let identity = format!("sandbox-{nonce:016x}");

        let mut startup: STARTUPINFOW = unsafe { std::mem::zeroed() };
        startup.cb = std::mem::size_of::<STARTUPINFOW>() as u32;

        // Launch success path: cmd exits 0 from the granted workspace.
        let command_line = to_wide("cmd /c exit 0");
        let spawned = launch(
            &command_line,
            None,
            None,
            &startup,
            &to_wide(&identity),
            &spec,
            ResourceLimits::default(),
        )
        .map_err(|err| err.to_string())?;
        let success_code = wait_exit_code(spawned.process);
        close_spawned(spawned.process, spawned.thread);
        if success_code != 0 {
            return Err(format!(
                "minimal launch returned exit code {success_code} instead of 0"
            ));
        }

        // Denial path: writing outside the granted workspace must fail.
        let mut envs = BTreeMap::new();
        envs.insert(
            OsString::from("SANDBOXRS_PROBE_OUT"),
            OsString::from(outside_file.to_string_lossy().into_owned()),
        );
        let env_block = build_env_block(true, &envs, &[]).unwrap_or_default();
        let denied_command = to_wide("cmd /c type nul > \"%SANDBOXRS_PROBE_OUT%\"");
        let current_dir = to_wide(&temp.to_string_lossy());
        let denied = launch(
            &denied_command,
            Some(&current_dir),
            Some(&env_block),
            &startup,
            &to_wide(&identity),
            &spec,
            ResourceLimits::default(),
        )
        .map_err(|err| err.to_string())?;
        let denied_code = wait_exit_code(denied.process);
        close_spawned(denied.process, denied.thread);

        if denied_code == 0 || outside_file.exists() {
            return Err(format!(
                "outside-write was not denied (exit {denied_code}, file existed={})",
                outside_file.exists()
            ));
        }
        Ok(())
    })();

    let _ = fs::remove_dir_all(&workspace);
    let _ = fs::remove_file(&outside_file);
    probe_result
}

fn wait_exit_code(process: windows_sys::Win32::Foundation::HANDLE) -> i32 {
    // SAFETY: `process` is a live process handle owned by the probe.
    unsafe {
        let _ = WaitForSingleObject(process, u32::MAX);
        let mut code = 0u32;
        if GetExitCodeProcess(process, &mut code) == 0 {
            let _ = GetLastError();
        }
        code as i32
    }
}

fn close_spawned(
    process: windows_sys::Win32::Foundation::HANDLE,
    thread: windows_sys::Win32::Foundation::HANDLE,
) {
    // SAFETY: The probe owns both handles and no longer needs them.
    unsafe {
        let _ = CloseHandle(process);
        let _ = CloseHandle(thread);
    }
}
