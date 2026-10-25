use super::SandboxRequest;

pub(super) fn run(_request: SandboxRequest) -> Result<i32, String> {
    Err("sandboxing is not supported on this platform".into())
}
