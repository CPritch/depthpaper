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
    directories::ProjectDirs::from("", "", "depthpaper")
        .map(|dirs| dirs.config_dir().join("config.toml"))
        .unwrap_or_else(|| PathBuf::from("config.toml"))
}

// TODO: This could be improved a bit to better handle ~
fn expand_tilde(p: &Path) -> PathBuf {
    if let Ok(stripped) = p.strip_prefix("~") {
        if let Ok(home) = std::env::var("HOME") {
            return PathBuf::from(home).join(stripped);
        }
    }
    p.to_path_buf()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::Path;

    #[test]
    fn test_default_deserialization() {
        let toml_str = r#"
            [wallpaper]
            path = "/tmp/bg.jpg"
        "#;

        let cfg: Config = toml::from_str(toml_str).unwrap();
        
        // Check required fields
        assert_eq!(cfg.wallpaper.path, Path::new("/tmp/bg.jpg"));
        
        // Check defaults
        assert_eq!(cfg.general.cursor_poll_hz, 60);
        assert_eq!(cfg.general.parallax_intensity, 0.025);
        assert_eq!(cfg.monitor.len(), 0);
    }

    #[test]
    fn test_monitor_overrides() {
        let toml_str = r#"
            [wallpaper]
            path = "/tmp/default.jpg"

            [[monitor]]
            name = "DP-1"
            wallpaper = "/tmp/dp1.jpg"
            parallax_intensity = 0.05
            
            [[monitor]]
            name = "HDMI-A-1"
        "#;

        let cfg: Config = toml::from_str(toml_str).unwrap();

        // Test explicit override
        assert_eq!(cfg.wallpaper_for("DP-1"), Path::new("/tmp/dp1.jpg"));
        assert_eq!(cfg.intensity_for("DP-1"), 0.05);

        // Test partial override (falls back to default wallpaper/intensity)
        assert_eq!(cfg.wallpaper_for("HDMI-A-1"), Path::new("/tmp/default.jpg"));
        assert_eq!(cfg.intensity_for("HDMI-A-1"), 0.025);

        // Test unknown monitor (falls back to default)
        assert_eq!(cfg.wallpaper_for("UNKNOWN"), Path::new("/tmp/default.jpg"));
    }
}