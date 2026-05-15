use anyhow::{Context, Result};
use serde::Deserialize;
use std::path::{Path, PathBuf};

#[derive(Debug, Deserialize, Clone)]
pub struct Config {
    /// Optional so the CLI can parse a config that predates the first
    /// `fetch-model` or `set` call, or a daemon-only config with no
    /// [inference] section.
    #[serde(default)]
    pub inference: Option<InferenceConfig>,
    /// Optional so the CLI can still parse a fresh config file before
    /// the wallpaper section has been written by the first `set` call.
    #[serde(default)]
    pub wallpaper: Option<WallpaperConfig>,
}

#[derive(Debug, Deserialize, Clone)]
pub struct InferenceConfig {
    pub model_path: PathBuf,
}

#[derive(Debug, Deserialize, Clone)]
#[allow(dead_code)] // populated for future slideshow / inspection commands
pub struct WallpaperConfig {
    pub color: PathBuf,
    #[serde(default)]
    pub depth: Option<PathBuf>,
}

pub fn config_path() -> PathBuf {
    if let Ok(xdg) = std::env::var("XDG_CONFIG_HOME") {
        PathBuf::from(xdg).join("shiftpaper/config.toml")
    } else if let Ok(home) = std::env::var("HOME") {
        PathBuf::from(home).join(".config/shiftpaper/config.toml")
    } else {
        PathBuf::from("config.toml")
    }
}

/// Load the CLI's view of the shared config.
/// Returns Ok(None) if the file doesn't exist; Err if it exists but is malformed.
pub fn try_load() -> Result<Option<Config>> {
    let path = config_path();
    let text = match std::fs::read_to_string(&path) {
        Ok(s) => s,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(e) => {
            return Err(anyhow::Error::new(e).context(format!("failed to read {}", path.display())));
        }
    };

    let mut cfg: Config =
        toml::from_str(&text).with_context(|| format!("failed to parse {}", path.display()))?;

    if let Some(ref mut i) = cfg.inference {
        i.model_path = expand_tilde(&i.model_path);
    }
    if let Some(ref mut w) = cfg.wallpaper {
        w.color = expand_tilde(&w.color);
        if let Some(ref mut d) = w.depth {
            *d = expand_tilde(d);
        }
    }

    Ok(Some(cfg))
}

fn expand_tilde(p: &Path) -> PathBuf {
    if let Ok(stripped) = p.strip_prefix("~")
        && let Ok(home) = std::env::var("HOME")
    {
        return PathBuf::from(home).join(stripped);
    }
    p.to_path_buf()
}
