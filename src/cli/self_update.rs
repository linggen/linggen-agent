use anyhow::{Context, Result};
use serde::Deserialize;
use std::time::Duration;

#[derive(Deserialize)]
struct ReleaseManifest {
    version: String,
    #[serde(default)]
    assets: Vec<ReleaseAsset>,
}

#[derive(Deserialize)]
struct ReleaseAsset {
    name: String,
    url: String,
}

fn platform_asset_name() -> String {
    let arch = if cfg!(target_arch = "aarch64") {
        "aarch64"
    } else if cfg!(target_arch = "x86_64") {
        "x86_64"
    } else {
        "unknown"
    };

    let os = if cfg!(target_os = "macos") {
        "apple-darwin"
    } else if cfg!(target_os = "linux") {
        "unknown-linux-gnu"
    } else {
        "unknown"
    };

    format!("linggen-agent-{}-{}", arch, os)
}

pub async fn run() -> Result<()> {
    let current_version = env!("CARGO_PKG_VERSION");
    println!("Current version: v{}", current_version);

    let client = reqwest::Client::builder()
        .user_agent("linggen-agent")
        .timeout(Duration::from_secs(30))
        .build()
        .context("Failed to build HTTP client")?;

    let manifest_url =
        "https://github.com/linggen/linggen-agent/releases/latest/download/manifest.json";

    let manifest = match client.get(manifest_url).send().await {
        Ok(resp) if resp.status().is_success() => match resp.json::<ReleaseManifest>().await {
            Ok(m) => m,
            Err(e) => {
                println!("Failed to parse release manifest: {}", e);
                println!("No releases available yet.");
                return Ok(());
            }
        },
        Ok(resp) => {
            println!(
                "Release manifest returned HTTP {}. No releases available yet.",
                resp.status()
            );
            return Ok(());
        }
        Err(e) => {
            println!("Failed to fetch release manifest: {}", e);
            println!("No releases available yet.");
            return Ok(());
        }
    };

    if manifest.version == current_version {
        println!("Already up to date (v{}).", current_version);
        return Ok(());
    }

    let asset_name = platform_asset_name();
    let asset = manifest
        .assets
        .iter()
        .find(|a| a.name == asset_name)
        .ok_or_else(|| {
            anyhow::anyhow!(
                "No release asset found for platform '{}'. Available: {}",
                asset_name,
                manifest
                    .assets
                    .iter()
                    .map(|a| a.name.as_str())
                    .collect::<Vec<_>>()
                    .join(", ")
            )
        })?;

    println!("Downloading {} ...", asset.name);

    let resp = client
        .get(&asset.url)
        .send()
        .await
        .context("Failed to download release asset")?;

    if !resp.status().is_success() {
        anyhow::bail!("Download failed with HTTP {}", resp.status());
    }

    let bytes = resp.bytes().await.context("Failed to read download")?;

    let current_exe = std::env::current_exe().context("Failed to get current executable path")?;
    let parent = current_exe
        .parent()
        .ok_or_else(|| anyhow::anyhow!("Failed to get executable parent directory"))?;

    let temp_path = parent.join(format!(".linggen-agent-update-{}", std::process::id()));

    std::fs::write(&temp_path, &bytes).context("Failed to write temp binary")?;

    // Set executable permissions (Unix)
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&temp_path, std::fs::Permissions::from_mode(0o755))?;
    }

    // Atomic rename
    std::fs::rename(&temp_path, &current_exe).context("Failed to replace binary")?;

    println!(
        "Updated v{} -> v{}",
        current_version, manifest.version
    );

    Ok(())
}
