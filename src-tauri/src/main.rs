// Prevents additional console window on Windows in release, DO NOT REMOVE!!
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

fn main() {
    // (Max): The AppContainer capability probe starts a copy of this executable as a minimal helper.
    //        Exit here instead of opening another launcher window.
    if std::env::var_os("SANDBOXRS_PROBE_SELF").is_some() {
        std::process::exit(0);
    }

    // The sandbox runner is a separate executable that is launched by the .NET sidecar to run the game in a sandbox.
    if std::env::args_os().nth(1).as_deref() == Some(std::ffi::OsStr::new("--pudu-sandbox-runner"))
    {
        std::process::exit(pudu_laucher_lib::run_sandbox_runner());
    }

    pudu_laucher_lib::run()
}
