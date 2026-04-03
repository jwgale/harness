use crate::xdg;

pub fn run(action: &str) -> Result<(), String> {
    xdg::ensure_dirs()?;
    let pid_file = xdg::data_dir().join("daemon.pid");

    match action {
        "start" => {
            if pid_file.exists() {
                let pid = std::fs::read_to_string(&pid_file).unwrap_or_default();
                // Check if process is actually running
                let pid_path = format!("/proc/{}", pid.trim());
                if std::path::Path::new(&pid_path).exists() {
                    println!("Daemon is already running (PID {}).", pid.trim());
                    return Ok(());
                }
            }
            println!("harness daemon: not yet implemented.");
            println!();
            println!("This will become the persistent agent runner (OpenClaw-style).");
            println!("Planned features:");
            println!("  - Background process watching .harness/ for triggers");
            println!("  - Workspace-based agent sessions");
            println!("  - Plugin hook execution");
            println!("  - Systemd user service integration");
            println!();
            println!("For now, use `harness run` for interactive orchestration.");
            Ok(())
        }
        "stop" => {
            if pid_file.exists() {
                let pid = std::fs::read_to_string(&pid_file).unwrap_or_default();
                println!("Would stop daemon (PID {}).", pid.trim());
                println!("Daemon not yet implemented — nothing to stop.");
            } else {
                println!("No daemon running.");
            }
            Ok(())
        }
        "status" => {
            if pid_file.exists() {
                let pid = std::fs::read_to_string(&pid_file).unwrap_or_default();
                let pid_path = format!("/proc/{}", pid.trim());
                if std::path::Path::new(&pid_path).exists() {
                    println!("Daemon is running (PID {}).", pid.trim());
                } else {
                    println!("Daemon PID file exists but process is not running.");
                    println!("Stale PID file: {}", pid_file.display());
                }
            } else {
                println!("Daemon is not running.");
            }
            println!();
            println!("XDG directories:");
            println!("  config: {}", xdg::config_dir().display());
            println!("  data:   {}", xdg::data_dir().display());
            println!("  cache:  {}", xdg::cache_dir().display());
            println!("  plugins: {}", xdg::plugins_dir().display());
            Ok(())
        }
        _ => Err(format!("Unknown daemon action: {action}. Use start, stop, or status.")),
    }
}
