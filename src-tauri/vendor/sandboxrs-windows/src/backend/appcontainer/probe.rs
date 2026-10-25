//! M3 probe: real AppContainer launch and outside-write denial without admin.
//!
//! The probe exercises the same `spawn` path used by the library, not a
//! separate launch helper.

#[cfg(windows)]
pub(crate) fn launch_probe() -> Result<(), String> {
    use std::fs;
    use std::path::PathBuf;
    use std::time::SystemTime;

    use crate::filesystem::FilesystemPlan;
    use crate::sandbox::Sandbox;
    use crate::{BackendKind, ResourceLimits, Stdio};

    use super::{build_state, spawn};

    let nonce = SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_err(|err| err.to_string())?
        .as_nanos() as u64;
    let identity = format!("sandbox-probe-{nonce:016x}");
    let temp = std::env::temp_dir();
    let workspace = temp.join(format!("{identity}-workspace"));
    let outside = temp.join(format!("{identity}-outside.txt"));
    let helper = std::env::var("SANDBOXRS_ATTACKER")
        .ok()
        .map(PathBuf::from)
        .or_else(|| std::env::current_exe().ok());

    let result = (|| {
        fs::create_dir_all(&workspace).map_err(|err| err.to_string())?;
        let helper = helper
            .clone()
            .ok_or_else(|| "no probe helper executable".to_string())?;
        let helper_copy = workspace.join("sandboxrs-probe-helper.exe");
        fs::copy(&helper, &helper_copy)
            .map_err(|err| format!("copy helper into workspace failed: {err}"))?;
        let plan = FilesystemPlan::compile(&workspace, &[], &[]).map_err(|err| err.to_string())?;
        let appcontainer = build_state(&identity, &plan).map_err(|err| err.to_string())?;
        let sandbox = Sandbox::new(
            workspace.clone(),
            plan,
            BackendKind::AppContainer,
            identity,
            ResourceLimits::default(),
            None,
            Some(appcontainer),
        );

        let attacker_mode = std::env::var("SANDBOXRS_ATTACKER").is_ok();
        if !attacker_mode {
            std::env::set_var("SANDBOXRS_PROBE_SELF", "1");
        }
        let mut success = if attacker_mode {
            spawn::spawn(
                &sandbox,
                helper_copy.clone().into(),
                vec!["sleep".into(), "1".into()],
                false,
                Default::default(),
                Vec::new(),
                Some(workspace.clone()),
                Stdio::Piped,
                Stdio::Piped,
                Stdio::Piped,
            )
        } else {
            spawn::spawn(
                &sandbox,
                helper_copy.clone().into(),
                Vec::new(),
                false,
                Default::default(),
                Vec::new(),
                Some(workspace.clone()),
                Stdio::Piped,
                Stdio::Piped,
                Stdio::Piped,
            )
        }
        .map_err(|err| err.to_string())?;
        let output = success.wait_with_output().map_err(|err| err.to_string())?;
        if !output.status.success() {
            return Err(format!(
                "probe launch exited with {:?}: stderr={}",
                output.status,
                String::from_utf8_lossy(&output.stderr)
            ));
        }

        if attacker_mode {
            let mut denied = spawn::spawn(
                &sandbox,
                helper_copy.clone().into(),
                vec!["write".into(), outside.display().to_string().into()],
                false,
                Default::default(),
                Vec::new(),
                Some(workspace.clone()),
                Stdio::Piped,
                Stdio::Piped,
                Stdio::Piped,
            )
            .map_err(|err| err.to_string())?;
            let denied_status = denied.wait().map_err(|err| err.to_string())?;
            if denied_status.success() || outside.exists() {
                return Err(format!(
                    "outside-write was not denied (status {denied_status:?}, file existed={})",
                    outside.exists()
                ));
            }
        }
        Ok(())
    })();

    let _ = fs::remove_dir_all(&workspace);
    let _ = fs::remove_file(&outside);
    result
}

#[cfg(not(windows))]
#[allow(dead_code)]
pub(crate) fn launch_probe() -> Result<(), String> {
    Err("AppContainer probe requires Windows".into())
}
