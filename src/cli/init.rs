use crate::skills::marketplace;
use anyhow::{Context, Result};
use std::path::PathBuf;

pub async fn run(global: bool, root: Option<PathBuf>) -> Result<()> {
    let target_dir = if global {
        dirs::home_dir()
            .ok_or_else(|| anyhow::anyhow!("Could not find home directory"))?
            .join(".linggen/skills")
    } else {
        let ws = root
            .or_else(|| crate::workspace::resolve_workspace_root(None).ok())
            .ok_or_else(|| anyhow::anyhow!("Could not determine workspace root"))?;
        ws.join(".linggen/skills")
    };

    println!("Installing skills to {} ...", target_dir.display());

    let (owner, repo) = ("linggen", "skills");
    let zip_url = marketplace::build_github_zip_url(owner, repo, "main");
    let client = marketplace::http_client()?;
    let temp_zip = marketplace::download_to_temp(&client, &zip_url)
        .await
        .context("Failed to download skills repository")?;

    let installed = marketplace::extract_all_skills_from_zip(&temp_zip, &target_dir)
        .context("Failed to extract skills")?;

    let _ = std::fs::remove_file(&temp_zip);

    if installed.is_empty() {
        println!("No skills found in repository.");
    } else {
        println!("Installed {} skills:", installed.len());
        for name in &installed {
            println!("  - {}", name);
        }
    }

    Ok(())
}
