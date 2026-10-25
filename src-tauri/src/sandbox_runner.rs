use std::ffi::OsString;
use std::path::PathBuf;

#[cfg(any(target_os = "linux", target_os = "macos"))]
mod nanosandbox;
#[cfg(not(any(target_os = "windows", target_os = "linux", target_os = "macos")))]
mod unsupported;
#[cfg(target_os = "windows")]
mod windows;

/// Entry point used by the .NET sidecar to run a game without adding another bundled binary.
pub fn run() -> i32 {
    match run_inner(std::env::args_os().skip(2).collect()) {
        Ok(exit_code) => exit_code,
        Err(error) => {
            eprintln!("Sandbox failed to launch the game: {error}");
            1
        }
    }
}

pub(super) fn announce_ready() {
    println!("PUDU_SANDBOX_READY");
    use std::io::Write;
    let _ = std::io::stdout().flush();
}

pub(super) struct SandboxRequest {
    pub(super) memory_limit_mb: u64,
    pub(super) sandbox_backend: String,
    pub(super) read_only_dirs: Vec<PathBuf>,
    pub(super) game_data_dir: PathBuf,
    pub(super) working_dir: PathBuf,
    pub(super) program: PathBuf,
    pub(super) game_args: Vec<OsString>,
}

fn run_inner(args: Vec<OsString>) -> Result<i32, String> {
    let request = SandboxRequest::parse(args)?;

    #[cfg(target_os = "windows")]
    return windows::run(request);
    #[cfg(any(target_os = "linux", target_os = "macos"))]
    return nanosandbox::run(request);
    #[cfg(not(any(target_os = "windows", target_os = "linux", target_os = "macos")))]
    return unsupported::run(request);
}

impl SandboxRequest {
    fn parse(mut args: Vec<OsString>) -> Result<Self, String> {
        let mut memory_limit_mb = None;
        let mut sandbox_backend = None;
        let mut read_only_dirs = Vec::new();
        let mut game_data_dir = None;
        let mut working_dir = None;
        let mut program = None;
        let mut game_args = Vec::new();
        let mut iter = args.drain(..);

        while let Some(arg) = iter.next() {
            match arg.to_str() {
                Some("--") => {
                    game_args.extend(iter);
                    break;
                }
                Some("--memory-limit-mb") => {
                    memory_limit_mb = Some(parse_next(&mut iter, "memory limit")?)
                }
                Some("--sandbox-backend") => {
                    sandbox_backend = Some(parse_next(&mut iter, "sandbox backend")?)
                }
                Some("--read-only-dir") => {
                    read_only_dirs.push(path_next(&mut iter, "read-only directory")?)
                }
                Some("--game-data-dir") => {
                    game_data_dir = Some(path_next(&mut iter, "game data directory")?)
                }
                Some("--working-dir") => {
                    working_dir = Some(path_next(&mut iter, "working directory")?)
                }
                Some("--program") => program = Some(path_next(&mut iter, "game executable")?),
                _ => return Err("unrecognized sandbox runner argument".into()),
            }
        }

        let memory_limit_mb = memory_limit_mb.ok_or("missing memory limit")?;
        if !(512..=131_072).contains(&memory_limit_mb) {
            return Err("memory limit must be between 512 and 131072 MB".into());
        }
        let sandbox_backend = sandbox_backend.unwrap_or_else(|| "auto".to_owned());
        if !["auto", "windows-sandbox-api", "appcontainer"].contains(&sandbox_backend.as_str()) {
            return Err(
                "sandbox backend must be auto, windows-sandbox-api, or appcontainer".into(),
            );
        }
        if read_only_dirs.is_empty() {
            return Err("at least one read-only directory must be provided".into());
        }

        let request = Self {
            memory_limit_mb,
            sandbox_backend,
            read_only_dirs,
            game_data_dir: game_data_dir.ok_or("missing game data directory")?,
            working_dir: working_dir.ok_or("missing working directory")?,
            program: program.ok_or("missing game executable")?,
            game_args,
        };
        for directory in request
            .read_only_dirs
            .iter()
            .chain([&request.game_data_dir, &request.working_dir])
        {
            if !directory.is_dir() {
                return Err(format!(
                    "required sandbox directory does not exist: {}",
                    directory.display()
                ));
            }
        }
        if !request.program.is_file() {
            return Err(format!(
                "game executable does not exist: {}",
                request.program.display()
            ));
        }
        Ok(request)
    }
}

fn parse_next<T: std::str::FromStr>(
    iter: &mut impl Iterator<Item = OsString>,
    label: &str,
) -> Result<T, String> {
    iter.next()
        .and_then(|value| value.into_string().ok())
        .ok_or_else(|| format!("missing or invalid {label}"))?
        .parse()
        .map_err(|_| format!("invalid {label}"))
}

fn path_next(iter: &mut impl Iterator<Item = OsString>, label: &str) -> Result<PathBuf, String> {
    iter.next()
        .map(PathBuf::from)
        .ok_or_else(|| format!("missing {label}"))
}
