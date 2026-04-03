use anyhow::{Context, Result};
use serde::Deserialize;
use std::path::{Path, PathBuf};

#[derive(Debug, Deserialize, Clone)]
pub struct Config {
    #[serde(default)]
    pub general: General,
    pub wallpaper: Wallpaper,
    #[serde(default)]
    pub monitor: Vec<MonitorOverride>,
}

#[derive(Debug, Deserialize, Clone)]
pub struct General {
    #[serde(default = "default_poll_hz")]
    pub cursor_poll_hz: u32,
    #[serde(default = "default_intensity")]
    pub parallax_intensity: f32,
    #[serde(default = "default_idle_timeout")]
    #[allow(dead_code)] // Phase 4: idle detection
    pub idle_timeout_secs: u64,
    pub model_path: Option<PathBuf>,
}

#[derive(Debug, Deserialize, Clone)]
pub struct Wallpaper {
    pub path: PathBuf,
}

#[derive(Debug, Deserialize, Clone)]
pub struct MonitorOverride {
    pub name: String,
    pub wallpaper: Option<PathBuf>,
    pub parallax_intensity: Option<f32>,
}

impl Default for General {
    fn default() -> Self {
        Self {
            cursor_poll_hz: default_poll_hz(),
            parallax_intensity: default_intensity(),
            idle_timeout_secs: default_idle_timeout(),
            model_path: None,
        }
    }
}

fn default_poll_hz() -> u32 { 60 }
fn default_intensity() -> f32 { 0.025 }
fn default_idle_timeout() -> u64 { 5 }

impl Config {
    pub fn load() -> Result<Self> {
        let path = config_path();
        tracing::debug!(?path, "looking for config file");

        let text = std::fs::read_to_string(&path)
            .with_context(|| format!("failed to read config at {}", path.display()))?;

        let mut cfg: Config =
            toml::from_str(&text).with_context(|| "failed to parse config")?;

        // Expand ~ in paths
        cfg.wallpaper.path = expand_tilde(&cfg.wallpaper.path);
        if let Some(ref mut p) = cfg.general.model_path {
            *p = expand_tilde(p);
        }
        for m in &mut cfg.monitor {
            if let Some(ref mut p) = m.wallpaper {
                *p = expand_tilde(p);
            }
        }

        Ok(cfg)
    }

    /// Get the wallpaper path for a given output name, falling back to default.
    pub fn wallpaper_for(&self, output_name: &str) -> &Path {
        self.monitor
            .iter()
            .find(|m| m.name == output_name)
            .and_then(|m| m.wallpaper.as_deref())
            .unwrap_or(&self.wallpaper.path)
    }

    /// Get parallax intensity for a given output, falling back to default.
    pub fn intensity_for(&self, output_name: &str) -> f32 {
        self.monitor
            .iter()
            .find(|m| m.name == output_name)
            .and_then(|m| m.parallax_intensity)
            .unwrap_or(self.general.parallax_intensity)
    }
}

fn config_path() -> PathBuf {
    if let Ok(xdg) = std::env::var("XDG_CONFIG_HOME") {
        PathBuf::from(xdg).join("depthpaper/config.toml")
    } else if let Ok(home) = std::env::var("HOME") {
        PathBuf::from(home).join(".config/depthpaper/config.toml")
    } else {
        PathBuf::from("config.toml")
    }
}

fn expand_tilde(p: &Path) -> PathBuf {
    if let Ok(stripped) = p.strip_prefix("~") {
        if let Ok(home) = std::env::var("HOME") {
            return PathBuf::from(home).join(stripped);
        }
    }
    p.to_path_buf()
}