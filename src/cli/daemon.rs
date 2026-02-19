use crate::config::Config;
use anyhow::{Context, Result};
use std::fs;
use std::path::{Path, PathBuf};
use std::time::Duration;

#[cfg(unix)]
use std::os::unix::process::CommandExt;

// ---------------------------------------------------------------------------
// Path helpers
// ---------------------------------------------------------------------------

fn agent_pid_file() -> PathBuf {
    crate::paths::linggen_home().join("ling.pid")
}

fn agent_log_file() -> PathBuf {
    crate::paths::linggen_home().join("ling.log")
}

fn memory_pid_file() -> PathBuf {
    crate::paths::linggen_home().join("ling-mem.pid")
}

fn memory_log_file() -> PathBuf {
    crate::paths::linggen_home().join("ling-mem.log")
}

/// Find the ling-mem server binary in common locations.
pub fn find_memory_binary() -> Option<String> {
    let base_name = "ling-mem";

    // 1. On macOS, check user-local install location.
    #[cfg(target_os = "macos")]
    {
        if let Some(home) = dirs::home_dir() {
            let user_local = home.join("Library/Application Support/Linggen/bin").join(base_name);
            if user_local.exists() {
                return Some(user_local.to_string_lossy().to_string());
            }
        }
    }

    // 2. Check PATH
    if let Some(paths) = std::env::var_os("PATH") {
        for dir in std::env::split_paths(&paths) {
            if dir.join(base_name).exists() {
                return Some(base_name.to_string());
            }
        }
    }

    // 3. Check alongside the current executable
    if let Ok(exe_path) = std::env::current_exe() {
        if let Some(parent) = exe_path.parent() {
            let alongside = parent.join(base_name);
            if alongside.exists() {
                return Some(alongside.to_string_lossy().to_string());
            }
        }
    }

    None
}

// ---------------------------------------------------------------------------
// Process helpers
// ---------------------------------------------------------------------------

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

async fn stop_process_by_pid_file(pid_path: &Path, label: &str) -> Result<()> {
    let pid = match fs::read_to_string(pid_path)
        .ok()
        .and_then(|s| s.trim().parse::<u32>().ok())
    {
        Some(p) => p,
        None => {
            println!("{}: no PID file found; may not be running.", label);
            return Ok(());
        }
    };

    if !is_process_running(pid) {
        println!("{}: process {} is not running. Cleaning up PID file.", label, pid);
        let _ = fs::remove_file(pid_path);
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

    let _ = fs::remove_file(pid_path);
    println!("{} stopped (PID {})", label, pid);
    Ok(())
}

fn print_port_status(label: &str, port: u16, listening: bool, pid: Option<u32>) {
    print!("  {:<12} port {}", format!("{}:", label), port);
    match (listening, pid) {
        (true, Some(pid)) => println!("  running (PID {})", pid),
        (true, None) => println!("  running"),
        (false, Some(pid)) => {
            if is_process_running(pid) {
                println!("  process alive (PID {}) but port not listening", pid);
            } else {
                println!("  not running (stale PID)");
            }
        }
        (false, None) => println!("  not running"),
    }
}

// ---------------------------------------------------------------------------
// Agent daemon
// ---------------------------------------------------------------------------

pub async fn start_agent(
    config: &Config,
    port_override: Option<u16>,
    root: Option<PathBuf>,
) -> Result<()> {
    let port = port_override.unwrap_or(config.server.port);

    if is_port_listening(port).await {
        println!("Agent server already running on port {}", port);
        return Ok(());
    }

    let pid_path = agent_pid_file();
    if let Some(parent) = pid_path.parent() {
        fs::create_dir_all(parent)?;
    }

    let exe = std::env::current_exe().context("Failed to get current executable path")?;
    let log = agent_log_file();

    let mut args = vec!["--web".to_string(), "--port".to_string(), port.to_string()];
    if let Some(ref r) = root {
        args.push("--root".to_string());
        args.push(r.display().to_string());
    }

    let log_out = fs::File::create(&log).context("Failed to create daemon log file")?;
    let log_err = log_out.try_clone()?;

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
        println!("Agent server started on http://localhost:{} (PID {})", port, pid);
    } else {
        println!(
            "Agent server spawned (PID {}) but not yet reachable on port {}",
            pid, port
        );
        println!("Check logs at {}", log.display());
    }

    Ok(())
}

pub async fn stop_agent() -> Result<()> {
    stop_process_by_pid_file(&agent_pid_file(), "Agent server").await
}

// ---------------------------------------------------------------------------
// Memory server
// ---------------------------------------------------------------------------

pub async fn start_memory(config: &Config) -> Result<()> {
    let port = config.memory.server_port;

    if is_port_listening(port).await {
        println!("Memory server already running on port {}", port);
        return Ok(());
    }

    let memory_bin = match find_memory_binary() {
        Some(bin) => bin,
        None => {
            println!("Memory server binary (ling-mem) not found.");
            println!("Install it with: ling install --memory");
            return Ok(());
        }
    };

    let pid_path = memory_pid_file();
    if let Some(parent) = pid_path.parent() {
        fs::create_dir_all(parent)?;
    }

    let log = memory_log_file();
    let log_out = fs::File::create(&log).context("Failed to create memory server log file")?;
    let log_err = log_out.try_clone()?;

    let child = std::process::Command::new(&memory_bin)
        .arg("--port")
        .arg(port.to_string())
        .stdout(log_out)
        .stderr(log_err)
        .stdin(std::process::Stdio::null())
        .process_group(0)
        .spawn()
        .context("Failed to spawn memory server process")?;

    let pid = child.id();
    fs::write(&pid_path, pid.to_string())?;

    // Poll until ready
    let mut ready = false;
    for _ in 0..30 {
        tokio::time::sleep(Duration::from_millis(100)).await;
        if is_port_listening(port).await {
            ready = true;
            break;
        }
    }

    if ready {
        println!("Memory server started on http://localhost:{} (PID {})", port, pid);
    } else {
        println!(
            "Memory server spawned (PID {}) but not yet reachable on port {}",
            pid, port
        );
        println!("Check logs at {}", log.display());
    }

    Ok(())
}

pub async fn stop_memory() -> Result<()> {
    stop_process_by_pid_file(&memory_pid_file(), "Memory server").await
}

pub async fn memory_status(config: &Config) -> Result<()> {
    let port = config.memory.server_port;
    let listening = is_port_listening(port).await;
    let pid = fs::read_to_string(memory_pid_file())
        .ok()
        .and_then(|s| s.trim().parse::<u32>().ok());

    println!("ling memory status\n");
    print_port_status("Memory", port, listening, pid);

    if listening {
        // Try to get server info
        if let Ok(client) = reqwest::Client::builder()
            .timeout(Duration::from_secs(2))
            .build()
        {
            let url = format!("http://127.0.0.1:{}/api/status", port);
            if let Ok(resp) = client.get(&url).send().await {
                if let Ok(text) = resp.text().await {
                    println!("  Status:      {}", text.trim());
                }
            }
        }
    }

    println!();
    Ok(())
}

// ---------------------------------------------------------------------------
// Combined status (agent + memory)
// ---------------------------------------------------------------------------

pub async fn status(config: &Config, config_path: Option<&Path>) -> Result<()> {
    println!("ling status\n");

    // Version
    println!("  Version:     v{}", env!("CARGO_PKG_VERSION"));

    // Config
    match config_path {
        Some(p) => println!("  Config:      {}", p.display()),
        None => println!("  Config:      (default)"),
    }

    // Agent server
    let port = config.server.port;
    let listening = is_port_listening(port).await;
    let pid = fs::read_to_string(agent_pid_file())
        .ok()
        .and_then(|s| s.trim().parse::<u32>().ok());
    print_port_status("Agent", port, listening, pid);

    // Memory server
    let mem_port = config.memory.server_port;
    let mem_listening = is_port_listening(mem_port).await;
    let mem_pid = fs::read_to_string(memory_pid_file())
        .ok()
        .and_then(|s| s.trim().parse::<u32>().ok());
    print_port_status("Memory", mem_port, mem_listening, mem_pid);

    // Memory binary
    match find_memory_binary() {
        Some(bin) => println!("  Memory bin:  {}", bin),
        None => println!("  Memory bin:  not found (install with: ling install --memory)"),
    }

    // Workspace
    match crate::workspace::resolve_workspace_root(None) {
        Ok(ws) => println!("  Workspace:   {}", ws.display()),
        Err(_) => println!("  Workspace:   none"),
    }

    // Models and agents counts
    println!("  Models:      {}", config.models.len());

    // Count agents from both global and project directories (dedup by filename stem)
    let mut agent_ids = std::collections::HashSet::new();
    let count_md = |dir: &Path, seen: &mut std::collections::HashSet<String>| {
        if dir.exists() {
            if let Ok(entries) = fs::read_dir(dir) {
                for e in entries.filter_map(|e| e.ok()) {
                    let p = e.path();
                    if p.extension().map_or(false, |ext| ext == "md") {
                        if let Some(stem) = p.file_stem().and_then(|s| s.to_str()) {
                            seen.insert(stem.to_lowercase());
                        }
                    }
                }
            }
        }
    };
    count_md(&crate::paths::global_agents_dir(), &mut agent_ids);
    count_md(&PathBuf::from("agents"), &mut agent_ids);
    println!("  Agents:      {}", agent_ids.len());

    println!();
    Ok(())
}
