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
    pub keep_alive: Option<String>,
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
    #[serde(default)]
    pub policy: AgentPolicy,
}

#[derive(Debug, Serialize, Deserialize, Clone, Copy, PartialEq, Eq, Hash)]
pub enum AgentPolicyCapability {
    #[serde(alias = "patch", alias = "PATCH")]
    Patch,
    #[serde(alias = "finalize", alias = "FINALIZE", alias = "FinalizeTask")]
    Finalize,
    #[serde(alias = "delegate", alias = "DELEGATE")]
    Delegate,
}

#[derive(Debug, Serialize, Deserialize, Clone, Default)]
struct AgentPolicyObject {
    #[serde(default)]
    allow: Vec<AgentPolicyCapability>,
    #[serde(default)]
    delegate_targets: Vec<String>,
}

#[derive(Debug, Clone, Default, Serialize)]
pub struct AgentPolicy {
    pub allow: Vec<AgentPolicyCapability>,
    pub delegate_targets: Vec<String>,
}

#[derive(Debug, Deserialize)]
#[serde(untagged)]
enum AgentPolicyWire {
    List(Vec<AgentPolicyCapability>),
    Object(AgentPolicyObject),
}

impl<'de> Deserialize<'de> for AgentPolicy {
    fn deserialize<D>(deserializer: D) -> std::result::Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        let wire = AgentPolicyWire::deserialize(deserializer)?;
        let mut policy = match wire {
            AgentPolicyWire::List(allow) => AgentPolicy {
                allow,
                delegate_targets: Vec::new(),
            },
            AgentPolicyWire::Object(obj) => AgentPolicy {
                allow: obj.allow,
                delegate_targets: obj.delegate_targets,
            },
        };
        policy.normalize();
        Ok(policy)
    }
}

impl AgentPolicy {
    fn normalize(&mut self) {
        let mut seen_allow = std::collections::HashSet::new();
        self.allow.retain(|cap| seen_allow.insert(*cap));

        let mut seen_targets = std::collections::HashSet::new();
        self.delegate_targets = self
            .delegate_targets
            .iter()
            .map(|target| target.trim().to_lowercase())
            .filter(|target| !target.is_empty())
            .filter(|target| seen_targets.insert(target.clone()))
            .collect();
    }

    pub fn allows(&self, capability: AgentPolicyCapability) -> bool {
        self.allow.contains(&capability)
    }

    pub fn allows_delegate_target(&self, target_agent_id: &str) -> bool {
        if self.delegate_targets.is_empty() {
            return true;
        }
        let target = target_agent_id.trim().to_lowercase();
        self.delegate_targets.iter().any(|item| item == &target)
    }
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
    pub fn from_markdown_content(content: &str) -> Result<(Self, String)> {
        if !content.starts_with("---") {
            anyhow::bail!("Agent spec must start with YAML frontmatter (---)");
        }
        let parts: Vec<&str> = content.splitn(3, "---").collect();
        if parts.len() < 3 {
            anyhow::bail!("Agent spec missing closing frontmatter delimiter (---)");
        }
        let spec: AgentSpec = serde_yaml::from_str(parts[1])?;
        let system_prompt = parts[2].trim().to_string();
        Ok((spec, system_prompt))
    }

    pub fn from_markdown(path: &Path) -> Result<(Self, String)> {
        let content = fs::read_to_string(path)?;
        Self::from_markdown_content(&content)
            .map_err(|e| anyhow::anyhow!("Agent spec at {:?} is invalid: {}", path, e))
    }

    pub fn allows_policy(&self, capability: AgentPolicyCapability) -> bool {
        self.policy.allows(capability)
    }
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct ServerConfig {
    pub port: u16,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct AgentConfig {
    pub max_iters: usize,
    #[serde(default)]
    pub write_safety_mode: WriteSafetyMode,
    #[serde(default)]
    pub prompt_loop_breaker: Option<String>,
}

#[derive(Debug, Serialize, Deserialize, Clone, Copy, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum WriteSafetyMode {
    Strict,
    Warn,
    Off,
}

impl Default for WriteSafetyMode {
    fn default() -> Self {
        // User-selected default for this repo: warn (allow write, but emit warnings).
        WriteSafetyMode::Warn
    }
}

#[derive(Debug, Serialize, Deserialize, Clone, Default)]
pub struct LoggingConfig {
    pub level: Option<String>,
    pub directory: Option<String>,
    pub rotation: Option<String>,
    pub retention_days: Option<u64>,
}

impl Config {
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
                keep_alive: None,
            }],
            server: ServerConfig { port: 8080 },
            agent: AgentConfig {
                max_iters: 10,
                write_safety_mode: WriteSafetyMode::default(),
                prompt_loop_breaker: None,
            },
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
