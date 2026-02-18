use crate::config::Config;
use anyhow::{Context, Result};
use std::fs;
use std::path::{Path, PathBuf};
use std::time::Duration;

#[cfg(unix)]
use std::os::unix::process::CommandExt;

fn pid_file() -> PathBuf {
    dirs::home_dir()
        .unwrap_or_else(|| PathBuf::from("."))
        .join(".linggen/linggen-agent.pid")
}

fn log_file() -> PathBuf {
    dirs::home_dir()
        .unwrap_or_else(|| PathBuf::from("."))
        .join(".linggen/linggen-agent.log")
}

fn read_pid() -> Option<u32> {
    let path = pid_file();
    fs::read_to_string(&path)
        .ok()
        .and_then(|s| s.trim().parse::<u32>().ok())
}

fn is_process_running(pid: u32) -> bool {
    std::process::Command::new("kill")
        .args(["-0", &pid.to_string()])
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status()
        .map(|s| s.success())
        .unwrap_or(false)
}

async fn is_port_listening(port: u16) -> bool {
    tokio::time::timeout(
        Duration::from_secs(1),
        tokio::net::TcpStream::connect(format!("127.0.0.1:{}", port)),
    )
    .await
    .map(|r| r.is_ok())
    .unwrap_or(false)
}

pub async fn start(config: &Config, port_override: Option<u16>, root: Option<PathBuf>) -> Result<()> {
    let port = port_override.unwrap_or(config.server.port);

    // Check if already running
    if is_port_listening(port).await {
        println!("Server already running on port {}", port);
        return Ok(());
    }

    // Ensure ~/.linggen/ exists
    let pid_path = pid_file();
    if let Some(parent) = pid_path.parent() {
        fs::create_dir_all(parent)?;
    }

    let exe = std::env::current_exe().context("Failed to get current executable path")?;
    let log = log_file();

    let mut args = vec!["serve".to_string(), "--port".to_string(), port.to_string()];
    if let Some(ref r) = root {
        args.push("--root".to_string());
        args.push(r.display().to_string());
    }

    let log_out = fs::File::create(&log).context("Failed to create daemon log file")?;
    let log_err = log_out.try_clone()?;

    // Spawn detached child
    let child = std::process::Command::new(&exe)
        .args(&args)
        .stdout(log_out)
        .stderr(log_err)
        .stdin(std::process::Stdio::null())
        .process_group(0)
        .spawn()
        .context("Failed to spawn daemon process")?;

    let pid = child.id();
    fs::write(&pid_path, pid.to_string())?;

    // Poll until ready (30 x 100ms = 3s)
    let mut ready = false;
    for _ in 0..30 {
        tokio::time::sleep(Duration::from_millis(100)).await;
        if is_port_listening(port).await {
            ready = true;
            break;
        }
    }

    if ready {
        println!("Server started on http://localhost:{} (PID {})", port, pid);
    } else {
        println!(
            "Server spawned (PID {}) but not yet reachable on port {}",
            pid, port
        );
        println!("Check logs at {}", log.display());
    }

    Ok(())
}

pub async fn stop() -> Result<()> {
    let pid_path = pid_file();
    let pid = match read_pid() {
        Some(p) => p,
        None => {
            println!("No PID file found; server may not be running.");
            return Ok(());
        }
    };

    if !is_process_running(pid) {
        println!("Process {} is not running. Cleaning up PID file.", pid);
        let _ = fs::remove_file(&pid_path);
        return Ok(());
    }

    // Send SIGTERM
    let _ = std::process::Command::new("kill")
        .args(["-TERM", &pid.to_string()])
        .status();

    tokio::time::sleep(Duration::from_millis(500)).await;

    if is_process_running(pid) {
        // Force kill
        let _ = std::process::Command::new("kill")
            .args(["-9", &pid.to_string()])
            .status();
        tokio::time::sleep(Duration::from_millis(200)).await;
    }

    let _ = fs::remove_file(&pid_path);
    println!("Server stopped (PID {})", pid);
    Ok(())
}

pub async fn status(config: &Config, config_path: Option<&Path>) -> Result<()> {
    println!("linggen-agent status\n");

    // Config
    match config_path {
        Some(p) => println!("  Config:    {}", p.display()),
        None => println!("  Config:    (default)"),
    }

    let port = config.server.port;
    println!("  Port:      {}", port);

    // Running state
    let listening = is_port_listening(port).await;
    let pid = read_pid();

    match (listening, pid) {
        (true, Some(pid)) => println!("  Status:    running (PID {})", pid),
        (true, None) => println!("  Status:    running (no PID file)"),
        (false, Some(pid)) => {
            if is_process_running(pid) {
                println!("  Status:    process alive (PID {}) but port not listening", pid);
            } else {
                println!("  Status:    not running (stale PID file)");
            }
        }
        (false, None) => println!("  Status:    not running"),
    }

    // Workspace
    match crate::workspace::resolve_workspace_root(None) {
        Ok(ws) => println!("  Workspace: {}", ws.display()),
        Err(_) => println!("  Workspace: none"),
    }

    // Models and agents counts
    println!("  Models:    {}", config.models.len());

    let agents_dir = PathBuf::from("agents");
    let agent_count = if agents_dir.exists() {
        fs::read_dir(&agents_dir)
            .map(|entries| {
                entries
                    .filter_map(|e| e.ok())
                    .filter(|e| e.path().extension().map_or(false, |ext| ext == "md"))
                    .count()
            })
            .unwrap_or(0)
    } else {
        0
    };
    println!("  Agents:    {}", agent_count);

    println!();
    Ok(())
}
