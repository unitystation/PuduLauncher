use super::{announce_ready, SandboxRequest};
use nanosandbox::{Sandbox, MB};

pub(super) fn run(request: SandboxRequest) -> Result<i32, String> {
    let program = request
        .program
        .to_str()
        .ok_or("game executable path is not valid UTF-8")?;
    let owned_args = request
        .game_args
        .iter()
        .map(|arg| {
            arg.to_str()
                .map(str::to_owned)
                .ok_or("game argument is not valid UTF-8")
        })
        .collect::<Result<Vec<_>, _>>()?;
    let borrowed_args = owned_args.iter().map(String::as_str).collect::<Vec<_>>();

    let mut builder = Sandbox::builder();
    for read_only_dir in &request.read_only_dirs {
        builder = builder.read_only(read_only_dir);
    }
    builder = builder
        .writable(&request.game_data_dir)
        .hide_home()
        .working_dir(&request.working_dir)
        .memory_limit(request.memory_limit_mb * MB)
        .private_tmp(1024 * MB)
        .host_network();

    #[cfg(target_os = "macos")]
    {
        builder = builder.shared_tmp(false);
    }

    let forwarded_environment = [
        "DISPLAY",
        "WAYLAND_DISPLAY",
        "XDG_RUNTIME_DIR",
        "XDG_SESSION_TYPE",
        "XDG_SESSION_DESKTOP",
        "XDG_CURRENT_DESKTOP",
        "DESKTOP_SESSION",
        "GDK_BACKEND",
        "QT_QPA_PLATFORM",
        "SDL_VIDEODRIVER",
        "DBUS_SESSION_BUS_ADDRESS",
        "XAUTHORITY",
        "PULSE_SERVER",
        "PULSE_LATENCY_MSEC",
        "PATH",
        "LD_LIBRARY_PATH",
        "DYLD_LIBRARY_PATH",
        "LANG",
        "LC_ALL",
        "HOME",
    ]
    .into_iter()
    .filter_map(|key| std::env::var(key).ok().map(|value| (key.to_owned(), value)));
    builder = builder.envs(forwarded_environment);

    let sandbox = builder.build().map_err(|error| error.to_string())?;
    announce_ready();

    let result = sandbox
        .run(program, &borrowed_args)
        .map_err(|error| error.to_string())?;
    print!("{}", result.stdout);
    eprint!("{}", result.stderr);
    if result.killed_by_oom {
        eprintln!("Sandbox terminated the game after it exceeded its memory limit.");
    }
    if result.output_truncated {
        eprintln!("Sandbox truncated game output after its output buffer limit.");
    }
    Ok(result.exit_code)
}

