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
    #[allow(dead_code)] // Phase 3: idle detection
    pub idle_timeout_secs: u64,
}

#[derive(Debug, Deserialize, Clone)]
pub struct Wallpaper {
    pub color: PathBuf,
    #[serde(default)]
    pub depth: Option<PathBuf>,
}

#[derive(Debug, Deserialize, Clone)]
pub struct MonitorOverride {
    pub name: String,
    pub color: Option<PathBuf>,
    pub depth: Option<PathBuf>,
    pub parallax_intensity: Option<f32>,
}

impl Default for General {
    fn default() -> Self {
        Self {
            cursor_poll_hz: default_poll_hz(),
            parallax_intensity: default_intensity(),
            idle_timeout_secs: default_idle_timeout(),
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

        cfg.wallpaper.color = expand_tilde(&cfg.wallpaper.color);
        if let Some(ref mut p) = cfg.wallpaper.depth {
            *p = expand_tilde(p);
        }
        for m in &mut cfg.monitor {
            if let Some(ref mut p) = m.color {
                *p = expand_tilde(p);
            }
            if let Some(ref mut p) = m.depth {
                *p = expand_tilde(p);
            }
        }

        Ok(cfg)
    }

    pub fn color_for(&self, output_name: &str) -> &Path {
        self.monitor
            .iter()
            .find(|m| m.name == output_name)
            .and_then(|m| m.color.as_deref())
            .unwrap_or(&self.wallpaper.color)
    }

    /// Resolve the depth map path for an output. Per-monitor override wins,
    /// then top-level explicit depth, then sibling inference from color path.
    pub fn depth_for(&self, output_name: &str) -> PathBuf {
        if let Some(d) = self
            .monitor
            .iter()
            .find(|m| m.name == output_name)
            .and_then(|m| m.depth.as_deref())
        {
            return d.to_path_buf();
        }
        if let Some(d) = &self.wallpaper.depth {
            return d.clone();
        }
        infer_depth_path(self.color_for(output_name))
    }

    pub fn intensity_for(&self, output_name: &str) -> f32 {
        self.monitor
            .iter()
            .find(|m| m.name == output_name)
            .and_then(|m| m.parallax_intensity)
            .unwrap_or(self.general.parallax_intensity)
    }
}

/// `foo.color.png` → `foo.depth16.png`. Other filenames get `.depth16.png`
/// appended before the extension.
fn infer_depth_path(color: &Path) -> PathBuf {
    let s = color.to_string_lossy();
    if let Some(stripped) = s.strip_suffix(".color.png") {
        return PathBuf::from(format!("{stripped}.depth16.png"));
    }
    let mut p = color.to_path_buf();
    let stem = p
        .file_stem()
        .map(|s| s.to_string_lossy().into_owned())
        .unwrap_or_default();
    p.set_file_name(format!("{stem}.depth16.png"));
    p
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