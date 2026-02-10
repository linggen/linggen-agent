use anyhow::Result;
use serde::{Deserialize, Serialize};
use std::fs;
use std::path::{Path, PathBuf};

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct Config {
    #[serde(default)]
    pub models: Vec<ModelConfig>,
    pub server: ServerConfig,
    pub agent: AgentConfig,
    #[serde(default)]
    pub logging: LoggingConfig,
    #[serde(default)]
    pub agents: Vec<AgentSpecRef>,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct ModelConfig {
    pub id: String,
    pub provider: String, // "ollama" | "openai"
    pub url: String,
    pub model: String,
    pub api_key: Option<String>,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct AgentSpecRef {
    pub id: String,
    pub spec_path: String,
    pub model: Option<String>,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct AgentSpec {
    pub name: String,
    pub description: String,
    pub tools: Vec<String>,
    pub model: Option<String>,
    #[serde(default)]
    pub kind: AgentKind,
    #[serde(default)]
    pub work_globs: Vec<String>,
    #[serde(default)]
    pub default_lock_globs: Vec<String>,
}

#[derive(Debug, Serialize, Deserialize, Clone, Copy, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum AgentKind {
    Main,
    Subagent,
}

impl Default for AgentKind {
    fn default() -> Self {
        Self::Main
    }
}

impl AgentSpec {
    pub fn from_markdown(path: &Path) -> Result<(Self, String)> {
        let content = fs::read_to_string(path)?;
        if !content.starts_with("---") {
            anyhow::bail!(
                "Agent spec at {:?} must start with YAML frontmatter (---)",
                path
            );
        }
        let parts: Vec<&str> = content.splitn(3, "---").collect();
        if parts.len() < 3 {
            anyhow::bail!(
                "Agent spec at {:?} missing closing frontmatter delimiter (---)",
                path
            );
        }
        let spec: AgentSpec = serde_yaml::from_str(parts[1])?;
        let system_prompt = parts[2].trim().to_string();
        Ok((spec, system_prompt))
    }
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct ServerConfig {
    pub port: u16,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct AgentConfig {
    pub max_iters: usize,
}

#[derive(Debug, Serialize, Deserialize, Clone, Default)]
pub struct LoggingConfig {
    pub level: Option<String>,
    pub directory: Option<String>,
    pub rotation: Option<String>,
    pub retention_days: Option<u64>,
}

impl Config {
    pub fn load() -> Result<Self> {
        let (cfg, _path) = Self::load_with_path()?;
        Ok(cfg)
    }

    pub fn load_with_path() -> Result<(Self, Option<PathBuf>)> {
        let mut candidates = Vec::new();

        if let Ok(explicit) = std::env::var("LINGGEN_CONFIG") {
            candidates.push(PathBuf::from(explicit));
        }

        candidates.push(PathBuf::from("linggen-agent.toml"));

        if let Some(dir) = dirs::config_dir() {
            candidates.push(dir.join("linggen-agent").join("linggen-agent.toml"));
        }

        if let Some(dir) = dirs::data_dir() {
            candidates.push(dir.join("linggen-agent").join("linggen-agent.toml"));
        }

        for path in candidates {
            if path.exists() {
                let content = fs::read_to_string(&path)?;
                let config: Config = toml::from_str(&content)?;
                return Ok((config, Some(path)));
            }
        }

        Ok((Config::default(), None))
    }
}

impl Default for Config {
    fn default() -> Self {
        Self {
            models: vec![ModelConfig {
                id: "default".to_string(),
                provider: "ollama".to_string(),
                url: "http://127.0.0.1:11434".to_string(),
                model: "qwen3-coder".to_string(),
                api_key: None,
            }],
            server: ServerConfig { port: 8080 },
            agent: AgentConfig { max_iters: 10 },
            logging: LoggingConfig {
                level: None,
                directory: None,
                rotation: None,
                retention_days: None,
            },
            agents: Vec::new(),
        }
    }
}
