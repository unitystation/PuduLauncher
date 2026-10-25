use super::{announce_ready, SandboxRequest};
use sandboxrs_windows::{BackendPreference, Sandbox, Stdio};

pub(super) fn run(request: SandboxRequest) -> Result<i32, String> {
    let preferred_backend = match request.sandbox_backend.as_str() {
        "auto" => BackendPreference::Auto,
        "windows-sandbox-api" => BackendPreference::WindowsSandboxApi,
        "appcontainer" => BackendPreference::AppContainer,
        _ => return Err("unsupported Windows sandbox backend".into()),
    };

    let memory_bytes = request
        .memory_limit_mb
        .checked_mul(1024 * 1024)
        .ok_or("memory limit is too large")?;
    eprintln!("Windows sandbox initializing");
    let mut builder = Sandbox::builder(&request.game_data_dir).max_memory(memory_bytes);
    for read_only_dir in &request.read_only_dirs {
        builder = builder.read_write(read_only_dir);
    }
    let sandbox = builder
        .preferred_backend(preferred_backend)
        .build()
        .map_err(|error| error.to_string())?;
    eprintln!("Windows sandbox backend initialized");

    eprintln!("starting game process");
    let mut child = sandbox
        .command(&request.program)
        .args(&request.game_args)
        .current_dir(&request.working_dir)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|error| error.to_string())?;
    eprintln!("game process started");

    announce_ready();

    let output = child
        .wait_with_output()
        .map_err(|error| error.to_string())?;
    use std::io::Write;
    let _ = std::io::stdout().write_all(&output.stdout);
    let _ = std::io::stderr().write_all(&output.stderr);
    Ok(output.status.code().unwrap_or(1))
}
