//! Compile a `FilesystemPlan` into a `SandboxSpec` FlatBuffer.
//!
//! Field order, defaults, and the "SBOX" file identifier must match
//! `SandboxSpec.fbs` exactly. The schema is experimental; it is isolated here
//! so a schema change only affects this module.

use flatbuffers::{FlatBufferBuilder, ForwardsUOffset, Vector, WIPOffset};

use crate::filesystem::{Access, FilesystemPlan};
use crate::SandboxError;

/// Schema version accepted by the current Windows engine.
pub(crate) const SANDBOX_SPEC_VERSION: &str = "0.1.0";

pub(crate) fn build_sandbox_spec(plan: &FilesystemPlan) -> Result<Vec<u8>, SandboxError> {
    let mut read_write: Vec<String> = Vec::new();
    let mut read_only: Vec<String> = Vec::new();

    for rule in plan.rules() {
        let path = rule
            .path()
            .to_str()
            .ok_or_else(|| SandboxError::InvalidPath {
                path: rule.path().to_path_buf(),
                reason: "path cannot be represented in the UTF-16 sandbox specification".into(),
            })?
            .to_string();
        match rule.access() {
            Access::ReadWrite => read_write.push(path),
            Access::ReadOnly => read_only.push(path),
            Access::Hidden => {}
        }
    }

    let mut builder = FlatBufferBuilder::with_capacity(1024);
    let version = builder.create_string(SANDBOX_SPEC_VERSION);

    let read_write = build_string_vector(&mut builder, &read_write)?;
    let read_only = build_string_vector(&mut builder, &read_only)?;

    // Slot indices are the SandboxSpec.fbs field order:
    // 0 version, 1 app_container, 2 integrity_level (deprecated),
    // 3 disallow_win32k_system_calls, 4 ui_restrictions, 5 least_privilege,
    // 6 capabilities, 7 fs_read_write, 8 fs_read_only, 9 network_policy,
    // 10 integrity, 11 fs_deny.
    let table = builder.start_table();
    builder.push_slot_always::<WIPOffset<&str>>(0, version);
    builder.push_slot::<bool>(1, true, false);
    if let Some(vector) = read_write {
        builder.push_slot_always::<WIPOffset<_>>(7, vector);
    }
    if let Some(vector) = read_only {
        builder.push_slot_always::<WIPOffset<_>>(8, vector);
    }
    let root = builder.end_table(table);
    builder.finish(root, Some("SBOX"));

    Ok(builder.finished_data().to_vec())
}

fn build_string_vector<'a>(
    builder: &mut FlatBufferBuilder<'a>,
    values: &[String],
) -> Result<Option<WIPOffset<Vector<'a, ForwardsUOffset<&'a str>>>>, SandboxError> {
    if values.is_empty() {
        return Ok(None);
    }
    let offsets: Vec<WIPOffset<&str>> = values
        .iter()
        .map(|value| builder.create_string(value))
        .collect();
    Ok(Some(builder.create_vector(&offsets)))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::{Path, PathBuf};

    #[cfg(windows)]
    #[test]
    fn spec_starts_with_root_and_sbox_identifier() {
        let plan = FilesystemPlan::compile(
            Path::new(r"C:\workspace"),
            &[PathBuf::from(r"C:\workspace\.readonly")],
            &[],
        )
        .unwrap();
        let bytes = build_sandbox_spec(&plan).unwrap();
        assert_eq!(&bytes[4..8], b"SBOX");
        assert!(bytes.len() > 8);
    }

    #[cfg(windows)]
    #[test]
    fn probe_policy_compiles() {
        let plan = FilesystemPlan::compile(
            Path::new(r"C:\temp\probe"),
            &[PathBuf::from(r"C:\Windows")],
            &[],
        )
        .unwrap();
        let bytes = build_sandbox_spec(&plan).unwrap();
        assert_eq!(&bytes[4..8], b"SBOX");
    }
}
