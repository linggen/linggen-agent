use crate::config::Config;
use anyhow::Result;
use std::path::{Path, PathBuf};
use std::time::Duration;

const GREEN: &str = "\x1b[32m";
const RED: &str = "\x1b[31m";
const CYAN: &str = "\x1b[36m";
const RESET: &str = "\x1b[0m";

fn ok(label: &str, detail: &str) {
    println!("  {GREEN}[OK]{RESET}   {label}: {detail}");
}

fn fail(label: &str, detail: &str) {
    println!("  {RED}[FAIL]{RESET} {label}: {detail}");
}

fn info(label: &str, detail: &str) {
    println!("  {CYAN}[INFO]{RESET} {label}: {detail}");
}

pub async fn run(config: &Config, config_path: Option<&Path>) -> Result<()> {
    println!("ling doctor\n");

    // 1. Binary version
    let version = env!("CARGO_PKG_VERSION");
    ok("Version", version);

    // 2. Config file
    match config_path {
        Some(p) => ok("Config", &p.display().to_string()),
        None => info("Config", "(default)"),
    }

    // 3. Workspace
    match crate::workspace::resolve_workspace_root(None) {
        Ok(ws) => ok("Workspace", &ws.display().to_string()),
        Err(_) => info("Workspace", "none"),
    }

    // 4. Agent server port
    let port = config.server.port;
    match check_tcp_port(port).await {
        true => ok("Agent server", &format!("port {} is listening", port)),
        false => info("Agent server", &format!("port {} not reachable", port)),
    }

    // 5. Memory server
    let mem_port = config.memory.server_port;
    match check_tcp_port(mem_port).await {
        true => ok("Memory server", &format!("port {} is listening", mem_port)),
        false => info("Memory server", &format!("port {} not reachable", mem_port)),
    }

    // 5b. Memory binary
    match crate::cli::daemon::find_memory_binary() {
        Some(bin) => ok("Memory binary", &bin),
        None => info("Memory binary", "not found (install with: ling install --memory)"),
    }

    // 6. Models
    check_models(config).await;

    // 7. Skills dirs
    check_skills_dirs();

    // 8. Agents dir
    check_agents_dir();

    // 9. Log directory
    check_log_dir(config);

    println!();
    Ok(())
}

async fn check_tcp_port(port: u16) -> bool {
    tokio::time::timeout(
        Duration::from_secs(1),
        tokio::net::TcpStream::connect(format!("127.0.0.1:{}", port)),
    )
    .await
    .map(|r| r.is_ok())
    .unwrap_or(false)
}

async fn check_models(config: &Config) {
    if config.models.is_empty() {
        info("Models", "none configured");
        return;
    }

    let client = match reqwest::Client::builder()
        .timeout(Duration::from_secs(5))
        .build()
    {
        Ok(c) => c,
        Err(_) => {
            fail("Models", "failed to build HTTP client");
            return;
        }
    };

    for m in &config.models {
        let label = format!("Model [{}]", m.id);
        let check_url = match m.provider.as_str() {
            "ollama" => format!("{}/api/tags", m.url.trim_end_matches('/')),
            "openai" => format!("{}/models", m.url.trim_end_matches('/')),
            _ => {
                info(&label, &format!("unknown provider '{}'", m.provider));
                continue;
            }
        };

        let mut req = client.get(&check_url);
        if let Some(key) = &m.api_key {
            req = req.header("Authorization", format!("Bearer {}", key));
        }

        match req.send().await {
            Ok(resp) if resp.status().is_success() => {
                ok(&label, &format!("{} @ {} (reachable)", m.model, m.url));
            }
            Ok(resp) => {
                fail(
                    &label,
                    &format!("{} @ {} (HTTP {})", m.model, m.url, resp.status()),
                );
            }
            Err(e) => {
                fail(&label, &format!("{} @ {} ({})", m.model, m.url, e));
            }
        }
    }
}

fn check_skills_dirs() {
    let home = dirs::home_dir();
    let dirs_to_check: Vec<(PathBuf, &str)> = [
        home.as_ref().map(|h| (h.join(".linggen/skills"), "global")),
        Some((PathBuf::from(".linggen/skills"), "project")),
    ]
    .into_iter()
    .flatten()
    .collect();

    for (dir, scope) in dirs_to_check {
        if dir.exists() {
            let count = std::fs::read_dir(&dir)
                .map(|entries| entries.filter_map(|e| e.ok()).count())
                .unwrap_or(0);
            ok(
                &format!("Skills ({})", scope),
                &format!("{} entries in {}", count, dir.display()),
            );
        } else {
            info(
                &format!("Skills ({})", scope),
                &format!("{} (not found)", dir.display()),
            );
        }
    }
}

fn check_agents_dir() {
    let agents_dir = PathBuf::from("agents");
    if agents_dir.exists() {
        let count = std::fs::read_dir(&agents_dir)
            .map(|entries| {
                entries
                    .filter_map(|e| e.ok())
                    .filter(|e| {
                        e.path()
                            .extension()
                            .map_or(false, |ext| ext == "md")
                    })
                    .count()
            })
            .unwrap_or(0);
        ok("Agents", &format!("{} agent files", count));
    } else {
        info("Agents", "agents/ directory not found");
    }
}

fn check_log_dir(config: &Config) {
    let log_dir = config
        .logging
        .directory
        .as_deref()
        .map(PathBuf::from)
        .or_else(|| {
            dirs::data_dir().map(|d| d.join("linggen-agent").join("logs"))
        });

    match log_dir {
        Some(dir) if dir.exists() => {
            // Check writable by trying to create a temp file
            let test_path = dir.join(".doctor-check");
            match std::fs::write(&test_path, "") {
                Ok(_) => {
                    let _ = std::fs::remove_file(&test_path);
                    ok("Logs", &dir.display().to_string());
                }
                Err(_) => fail("Logs", &format!("{} (not writable)", dir.display())),
            }
        }
        Some(dir) => info("Logs", &format!("{} (not found)", dir.display())),
        None => info("Logs", "could not determine log directory"),
    }
}
