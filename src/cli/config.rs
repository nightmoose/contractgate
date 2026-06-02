use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

/// Resolved `.contractgate.yml` configuration.
#[derive(Debug, Clone, Deserialize, Serialize, Default)]
pub struct CliConfig {
    #[serde(default = "default_version")]
    pub version: String,
    #[serde(default)]
    pub gateway: GatewayConfig,
    #[serde(default)]
    pub contracts: ContractsConfig,
    #[serde(default)]
    pub defaults: DefaultsConfig,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct GatewayConfig {
    pub url: String,
}

impl Default for GatewayConfig {
    fn default() -> Self {
        Self {
            url: "http://localhost:3000".into(),
        }
    }
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct ContractsConfig {
    pub dir: PathBuf,
    pub pattern: String,
}

impl Default for ContractsConfig {
    fn default() -> Self {
        Self {
            dir: PathBuf::from("./contracts"),
            pattern: "*.yaml".into(),
        }
    }
}

#[derive(Debug, Clone, Deserialize, Serialize, Default)]
pub struct DefaultsConfig {
    #[serde(default)]
    pub format: OutputFormat,
}

#[derive(Debug, Clone, Deserialize, Serialize, Default, PartialEq)]
#[serde(rename_all = "lowercase")]
pub enum OutputFormat {
    #[default]
    Human,
    Json,
}

fn default_version() -> String {
    "1.0".into()
}

impl CliConfig {
    /// Walk up from `start` looking for `.contractgate.yml`, stopping at the
    /// git root (directory containing `.git`) or the filesystem root.
    pub fn discover(start: &Path) -> Result<Self> {
        let path = Self::find_config_file(start)?;
        match path {
            Some(p) => Self::load(&p),
            None => Ok(Self::default()),
        }
    }

    /// Load config from an explicit path.
    pub fn load(path: &Path) -> Result<Self> {
        let src = std::fs::read_to_string(path)
            .with_context(|| format!("reading config file: {}", path.display()))?;
        let cfg: Self = serde_yaml::from_str(&src)
            .with_context(|| format!("parsing config file: {}", path.display()))?;
        Ok(cfg)
    }

    fn find_config_file(start: &Path) -> Result<Option<PathBuf>> {
        let mut current = start.to_path_buf();
        loop {
            let candidate = current.join(".contractgate.yml");
            if candidate.exists() {
                return Ok(Some(candidate));
            }
            // Stop at git root.
            if current.join(".git").exists() {
                break;
            }
            match current.parent() {
                Some(p) => current = p.to_path_buf(),
                None => break,
            }
        }
        Ok(None)
    }
}
