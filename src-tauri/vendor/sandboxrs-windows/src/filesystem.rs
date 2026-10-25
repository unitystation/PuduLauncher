use std::path::{Component, Path, PathBuf};

use crate::error::SandboxError;

/// Authority level granted to a filesystem root.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Access {
    /// Unspecified resources are not granted by default.
    Hidden,
    ReadOnly,
    ReadWrite,
}

impl Access {
    #[allow(dead_code)]
    pub(crate) fn is_readable(self) -> bool {
        matches!(self, Self::ReadOnly | Self::ReadWrite)
    }

    pub(crate) fn is_writable(self) -> bool {
        matches!(self, Self::ReadWrite)
    }
}

#[derive(Debug, Clone)]
pub(crate) struct PathRule {
    path: PathBuf,
    access: Access,
}

impl PathRule {
    pub(crate) fn path(&self) -> &Path {
        &self.path
    }

    pub(crate) fn access(&self) -> Access {
        self.access
    }
}

/// Normalized, conflict-checked authority boundary.
///
/// Raw builder input never reaches a backend. The plan resolves duplicate
/// roots and applies the "more specific path wins" overlap rule before any
/// backend compiles it into OS enforcement.
#[derive(Debug, Clone)]
pub(crate) struct FilesystemPlan {
    rules: Vec<PathRule>,
}

impl FilesystemPlan {
    pub(crate) fn compile(
        workspace: &Path,
        read_only: &[PathBuf],
        read_write: &[PathBuf],
    ) -> Result<Self, SandboxError> {
        let workspace = normalize_root(workspace)?;
        let mut merged: Vec<PathRule> = Vec::new();
        push_rule(
            &mut merged,
            PathRule {
                path: workspace,
                access: Access::ReadWrite,
            },
        )?;

        for path in read_only {
            let path = normalize_root(path)?;
            push_rule(
                &mut merged,
                PathRule {
                    path,
                    access: Access::ReadOnly,
                },
            )?;
        }
        for path in read_write {
            let path = normalize_root(path)?;
            push_rule(
                &mut merged,
                PathRule {
                    path,
                    access: Access::ReadWrite,
                },
            )?;
        }

        merged.sort_by_key(|rule| std::cmp::Reverse(rule.path.components().count()));
        Ok(Self { rules: merged })
    }

    pub(crate) fn rules(&self) -> &[PathRule] {
        &self.rules
    }

    #[allow(dead_code)]
    pub(crate) fn access_for(&self, path: &Path) -> Access {
        let normalized = normalize_root(path).unwrap_or_else(|_| path.to_path_buf());
        for rule in &self.rules {
            if path_is_within(&rule.path, &normalized) {
                return rule.access;
            }
        }
        Access::Hidden
    }
}

fn push_rule(rules: &mut Vec<PathRule>, rule: PathRule) -> Result<(), SandboxError> {
    if let Some(existing) = rules
        .iter_mut()
        .find(|existing| paths_equal(&existing.path, &rule.path))
    {
        if existing.access != rule.access {
            return Err(SandboxError::PolicyCompileFailed {
                backend: crate::BackendKind::AppContainer,
                reason: format!(
                    "conflicting access for {} ({} and {})",
                    rule.path.display(),
                    access_name(existing.access),
                    access_name(rule.access)
                ),
            });
        }
        return Ok(());
    }
    rules.push(rule);
    Ok(())
}

fn access_name(access: Access) -> &'static str {
    match access {
        Access::Hidden => "hidden",
        Access::ReadOnly => "read-only",
        Access::ReadWrite => "read-write",
    }
}

fn normalize_root(path: &Path) -> Result<PathBuf, SandboxError> {
    let invalid = |reason: &str| SandboxError::InvalidPath {
        path: path.to_path_buf(),
        reason: reason.to_string(),
    };

    if path.as_os_str().is_empty() {
        return Err(invalid("path is empty"));
    }
    if path.to_string_lossy().contains('\0') {
        return Err(invalid("path contains a NUL byte"));
    }
    if !path.is_absolute() {
        return Err(invalid("sandbox roots must be absolute"));
    }
    for component in path.components() {
        if matches!(component, Component::ParentDir) {
            return Err(invalid("sandbox roots must not contain '..'"));
        }
    }

    let original = path.to_string_lossy().into_owned();
    let trimmed = original
        .trim_end_matches(std::path::MAIN_SEPARATOR)
        .to_owned();
    let normalized = if !trimmed.is_empty() && Path::new(&trimmed).is_absolute() {
        trimmed
    } else {
        original
    };

    #[cfg(windows)]
    let normalized = normalized.replace('/', "\\");

    Ok(PathBuf::from(normalized))
}

fn paths_equal(a: &Path, b: &Path) -> bool {
    #[cfg(windows)]
    {
        a.to_string_lossy().to_lowercase() == b.to_string_lossy().to_lowercase()
    }
    #[cfg(not(windows))]
    {
        a == b
    }
}

#[allow(dead_code)]
fn path_is_within(parent: &Path, child: &Path) -> bool {
    let parent_parts: Vec<_> = parent.components().collect();
    let child_parts: Vec<_> = child.components().collect();
    if child_parts.len() < parent_parts.len() {
        return false;
    }
    parent_parts
        .iter()
        .zip(&child_parts)
        .all(|(a, b)| component_equal(a, b))
}

#[allow(dead_code)]
fn component_equal(a: &Component<'_>, b: &Component<'_>) -> bool {
    let a = a.as_os_str();
    let b = b.as_os_str();
    #[cfg(windows)]
    {
        a.to_string_lossy().to_lowercase() == b.to_string_lossy().to_lowercase()
    }
    #[cfg(not(windows))]
    {
        a == b
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn abs(path: &str) -> PathBuf {
        #[cfg(windows)]
        {
            PathBuf::from(format!("C:{path}"))
        }
        #[cfg(not(windows))]
        {
            PathBuf::from(path)
        }
    }

    #[test]
    fn rejects_relative_and_parent_components() {
        assert!(normalize_root(Path::new("relative/path")).is_err());
        assert!(normalize_root(&abs("/tmp/../escape")).is_err());
    }

    #[test]
    fn more_specific_rule_wins() {
        let plan = FilesystemPlan::compile(&abs("/workspace"), &[abs("/workspace/.readonly")], &[])
            .unwrap();

        assert!(plan.access_for(&abs("/workspace/file.txt")).is_writable());
        assert!(!plan
            .access_for(&abs("/workspace/.readonly/secret.txt"))
            .is_writable());
        assert!(plan
            .access_for(&abs("/workspace/.readonly/secret.txt"))
            .is_readable());
        assert_eq!(plan.access_for(&abs("/outside/file.txt")), Access::Hidden);
    }

    #[test]
    fn duplicate_conflicting_rules_fail() {
        let err = FilesystemPlan::compile(
            &abs("/workspace"),
            &[abs("/workspace")],
            &[abs("/workspace")],
        )
        .unwrap_err();
        assert!(matches!(err, SandboxError::PolicyCompileFailed { .. }));
    }
}
