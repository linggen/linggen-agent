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
    #[serde(default)]
    pub routing: RoutingConfig,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct ModelConfig {
    pub id: String,
    pub provider: String, // "ollama" | "openai"
    pub url: String,
    pub model: String,
    pub api_key: Option<String>,
    pub keep_alive: Option<String>,
    /// Manual context window override (tokens). Used when the provider API
    /// does not report context size (e.g. Ollama cloud/remote models).
    #[serde(default)]
    pub context_window: Option<usize>,
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
    pub skills: Vec<String>,
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
    #[serde(default = "default_max_delegation_depth")]
    pub max_delegation_depth: usize,
}

fn default_max_delegation_depth() -> usize {
    2
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
    pub retention_days: Option<u64>,
}

#[derive(Debug, Serialize, Deserialize, Clone, Default)]
pub struct RoutingConfig {
    #[serde(default)]
    pub default_policy: Option<String>,
    #[serde(default)]
    pub policies: Vec<RoutingPolicy>,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct RoutingPolicy {
    pub name: String,
    #[serde(default)]
    pub rules: Vec<RoutingRule>,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct RoutingRule {
    pub model: String,
    #[serde(default)]
    pub priority: u32,
    #[serde(default)]
    pub min_complexity: Option<ComplexityLevel>,
    #[serde(default)]
    pub max_complexity: Option<ComplexityLevel>,
}

#[derive(Debug, Serialize, Deserialize, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
#[serde(rename_all = "snake_case")]
pub enum ComplexityLevel {
    Low,
    Medium,
    High,
}

impl Config {
    pub fn load_with_path() -> Result<(Self, Option<PathBuf>)> {
        let mut candidates = Vec::new();

        if let Ok(explicit) = std::env::var("LINGGEN_CONFIG") {
            candidates.push(PathBuf::from(explicit));
        }

        // Check runtime.toml alongside the base config in each search location
        candidates.push(PathBuf::from("linggen-agent.runtime.toml"));
        candidates.push(PathBuf::from("linggen-agent.toml"));

        if let Some(dir) = dirs::config_dir() {
            let cfg_dir = dir.join("linggen-agent");
            candidates.push(cfg_dir.join("linggen-agent.runtime.toml"));
            candidates.push(cfg_dir.join("linggen-agent.toml"));
        }

        if let Some(dir) = dirs::data_dir() {
            let data_dir = dir.join("linggen-agent");
            candidates.push(data_dir.join("linggen-agent.runtime.toml"));
            candidates.push(data_dir.join("linggen-agent.toml"));
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

    pub fn runtime_config_path(config_dir: Option<&Path>) -> PathBuf {
        if let Some(dir) = config_dir {
            return dir.join("linggen-agent.runtime.toml");
        }
        if let Some(dir) = dirs::data_dir() {
            return dir
                .join("linggen-agent")
                .join("linggen-agent.runtime.toml");
        }
        PathBuf::from("linggen-agent.runtime.toml")
    }

    pub fn save_runtime(&self, config_dir: Option<&Path>) -> Result<PathBuf> {
        let path = Self::runtime_config_path(config_dir);
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)?;
        }
        let content = toml::to_string_pretty(self)?;
        fs::write(&path, content)?;
        Ok(path)
    }

    pub fn validate(&self) -> Result<()> {
        if self.models.is_empty() {
            anyhow::bail!("At least one model must be configured");
        }
        let mut seen_ids = std::collections::HashSet::new();
        for model in &self.models {
            if model.id.trim().is_empty() {
                anyhow::bail!("Model ID cannot be empty");
            }
            if !seen_ids.insert(&model.id) {
                anyhow::bail!("Duplicate model ID: {}", model.id);
            }
            // Validate provider is known.
            let known_providers = ["ollama", "openai"];
            if !known_providers.contains(&model.provider.as_str()) {
                anyhow::bail!(
                    "Model '{}' has unknown provider '{}'. Known providers: {}",
                    model.id,
                    model.provider,
                    known_providers.join(", ")
                );
            }
            // Validate model URL scheme to prevent SSRF.
            let url_lower = model.url.trim().to_lowercase();
            if !url_lower.starts_with("http://") && !url_lower.starts_with("https://") {
                anyhow::bail!(
                    "Model '{}' URL must start with http:// or https://, got: {}",
                    model.id,
                    model.url
                );
            }
        }
        if self.server.port == 0 {
            anyhow::bail!("Server port must be greater than 0");
        }
        if self.agent.max_iters == 0 {
            anyhow::bail!("Agent max_iters must be greater than 0");
        }
        if self.agent.max_iters > 1000 {
            anyhow::bail!("Agent max_iters must not exceed 1000");
        }
        Ok(())
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
                context_window: None,
            }],
            server: ServerConfig { port: 8080 },
            agent: AgentConfig {
                max_iters: 10,
                write_safety_mode: WriteSafetyMode::default(),
                prompt_loop_breaker: None,
                max_delegation_depth: default_max_delegation_depth(),
            },
            logging: LoggingConfig {
                level: None,
                directory: None,
                retention_days: None,
            },
            agents: Vec::new(),
            routing: RoutingConfig::default(),
        }
    }
}
