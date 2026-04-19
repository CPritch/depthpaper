mod cache;
mod config;
mod depth;

use anyhow::{Context, Result};
use clap::{Parser, Subcommand, ValueEnum};
use std::path::{Path, PathBuf};
use toml_edit::{value, DocumentMut};
use tracing::info;
use tracing_subscriber::EnvFilter;

/// Bake depth maps and configure the depthpaperd parallax wallpaper daemon.
#[derive(Parser)]
#[command(
    name = "depthpaper",
    version,
    about,
    long_about = "depthpaper is the command-line companion to the depthpaperd \
                  parallax wallpaper daemon. Use it to convert source images \
                  into the color + 16-bit depth pairs the daemon renders, set \
                  the active wallpaper, and switch cursor tracking modes.",
    propagate_version = true,
)]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    /// Bake a source image into a color + 16-bit depth PNG pair.
    ///
    /// Runs Depth Anything inference on the source image and writes a
    /// pair of files (`<hash>.color.png` and `<hash>.depth16.png`) to
    /// the cache directory or the directory specified by --out. Existing
    /// baked pairs for the same source content are reused — re-baking
    /// is essentially free.
    Bake {
        /// Source image (jpeg, png, or webp).
        input: PathBuf,
        /// Output directory. Defaults to the depthpaper cache directory
        /// at $XDG_CACHE_HOME/depthpaper/wallpapers.
        #[arg(short, long)]
        out: Option<PathBuf>,
        /// Path to the Depth Anything ONNX model. Falls back to
        /// $DEPTHPAPER_MODEL, then [inference] model_path in config.toml.
        #[arg(short, long, env = "DEPTHPAPER_MODEL")]
        model: Option<PathBuf>,
    },

    /// Bake an image and set it as the active wallpaper.
    ///
    /// Performs the same baking as `bake`, then updates the daemon's
    /// config.toml so the next daemon start uses this wallpaper. The
    /// resolved model path is also persisted to [inference] so future
    /// invocations don't need --model.
    Set {
        /// Source image (jpeg, png, or webp).
        input: PathBuf,
        /// Path to the Depth Anything ONNX model. Falls back to
        /// $DEPTHPAPER_MODEL, then [inference] model_path in config.toml.
        /// When provided, the resolved path is persisted to config.
        #[arg(short, long, env = "DEPTHPAPER_MODEL")]
        model: Option<PathBuf>,
    },

    /// Show or change the cursor tracking mode.
    ///
    /// Pointer mode (the default) uses Wayland's native pointer events.
    /// It works on any wlr-layer-shell compositor and is event-driven —
    /// the daemon stops rendering entirely when the cursor isn't over
    /// visible desktop, which saves significant battery.
    ///
    /// Hyprland mode reads the global cursor position from the Hyprland
    /// IPC socket. Parallax remains responsive even when windows cover
    /// the desktop, at the cost of being Hyprland-specific. Note that
    /// Hyprland mode lets the daemon observe cursor positions over
    /// arbitrary windows, which is a minor privacy consideration.
    Mode {
        /// Tracking mode to set. Omit to print the current value.
        #[arg(value_enum)]
        mode: Option<TrackingMode>,
    },
}

/// Cursor tracking mode. Mirrors `crate::config::TrackingMode` in the
/// daemon — kept here as a separate enum so the CLI can write the
/// string value via toml_edit without depending on the daemon crate.
#[derive(Copy, Clone, Debug, ValueEnum)]
enum TrackingMode {
    /// Wayland-native pointer events. Default. Works on any
    /// wlr-layer-shell compositor.
    Pointer,
    /// Hyprland IPC global cursor polling. Hyprland-only.
    Hyprland,
}

impl TrackingMode {
    fn as_str(self) -> &'static str {
        match self {
            Self::Pointer => "pointer",
            Self::Hyprland => "hyprland",
        }
    }
}

fn main() -> Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(
            EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| EnvFilter::new("depthpaper_cli=info")),
        )
        .init();

    let cli = Cli::parse();
    match cli.command {
        Command::Bake { input, out, model } => {
            let model = resolve_model(model)?;
            bake(&input, out.as_deref(), &model)?;
            Ok(())
        }
        Command::Set { input, model } => {
            let model = resolve_model(model)?;
            set(&input, &model)
        }
        Command::Mode { mode } => mode_cmd(mode),
    }
}

/// Resolve the model path. Clap merges --model and $DEPTHPAPER_MODEL into
/// `arg`, so by the time we get here a Some means flag-or-env. If still
/// None, fall through to config, then error.
fn resolve_model(arg: Option<PathBuf>) -> Result<PathBuf> {
    if let Some(p) = arg {
        return Ok(p);
    }
    if let Some(cfg) = config::try_load()? {
        return Ok(cfg.inference.model_path);
    }
    anyhow::bail!(
        "no model specified — pass --model, set DEPTHPAPER_MODEL, or add\n\
         \n\
         [inference]\n\
         model_path = \"/path/to/depth_anything_v2.onnx\"\n\
         \n\
         to {}",
        config::config_path().display()
    )
}

fn bake(input: &Path, out: Option<&Path>, model: &Path) -> Result<cache::BakedPaths> {
    let rgba = image::open(input)
        .with_context(|| format!("failed to open {}", input.display()))?
        .to_rgba8();

    let hash = cache::hash_source(&rgba);
    let out_dir = out
        .map(|p| p.to_path_buf())
        .unwrap_or_else(cache::cache_dir);
    let paths = cache::paths_for(&hash, &out_dir);

    if cache::cache_hit(&paths) {
        info!("cache hit, skipping inference");
        println!("{}", paths.color.display());
        println!("{}", paths.depth.display());
        return Ok(paths);
    }

    let depth_map = depth::estimate(&rgba, model)?;

    cache::write_color(&rgba, &paths.color)?;
    cache::write_depth(&depth_map, &paths.depth)?;

    info!("baked wallpaper");
    println!("{}", paths.color.display());
    println!("{}", paths.depth.display());

    Ok(paths)
}

fn set(input: &Path, model: &Path) -> Result<()> {
    let paths = bake(input, None, model)?;
    update_daemon_config(&paths, model)?;
    eprintln!();
    eprintln!("wallpaper set. reload the daemon to apply:");
    eprintln!("  systemctl --user reload depthpaperd");
    eprintln!("  # or: kill -HUP $(pidof depthpaperd)");
    Ok(())
}

fn mode_cmd(mode: Option<TrackingMode>) -> Result<()> {
    match mode {
        Some(m) => {
            update_tracking_mode(m)?;
            eprintln!("tracking mode set to {}", m.as_str());
            eprintln!();
            eprintln!("restart the daemon to apply:");
            eprintln!("  systemctl --user restart depthpaperd");
        }
        None => {
            let current = read_tracking_mode()?;
            match current.as_deref() {
                Some(m) => println!("{m}"),
                None => println!("pointer"),
            }
        }
    }
    Ok(())
}

fn read_tracking_mode() -> Result<Option<String>> {
    let path = config::config_path();
    let text = match std::fs::read_to_string(&path) {
        Ok(s) => s,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(e) => {
            return Err(anyhow::Error::new(e)
                .context(format!("failed to read {}", path.display())));
        }
    };

    let doc: DocumentMut = text
        .parse()
        .with_context(|| format!("failed to parse {}", path.display()))?;

    Ok(doc
        .get("daemon")
        .and_then(|d| d.get("tracking_mode"))
        .and_then(|m| m.as_str())
        .map(String::from))
}

fn update_tracking_mode(mode: TrackingMode) -> Result<()> {
    let path = config::config_path();

    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)
            .with_context(|| format!("failed to create {}", parent.display()))?;
    }

    let text = match std::fs::read_to_string(&path) {
        Ok(s) => s,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            info!(path = %path.display(), "config not found, creating");
            String::new()
        }
        Err(e) => {
            return Err(anyhow::Error::new(e)
                .context(format!("failed to read {}", path.display())));
        }
    };

    let mut doc: DocumentMut = text
        .parse()
        .with_context(|| format!("failed to parse {}", path.display()))?;

    ensure_table(&mut doc, "daemon");
    let daemon = doc["daemon"]
        .as_table_mut()
        .context("config [daemon] is not a table")?;
    daemon["tracking_mode"] = value(mode.as_str());

    std::fs::write(&path, doc.to_string())
        .with_context(|| format!("failed to write {}", path.display()))?;

    info!(path = %path.display(), "daemon config updated");
    Ok(())
}

fn update_daemon_config(paths: &cache::BakedPaths, model: &Path) -> Result<()> {
    let path = config::config_path();

    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)
            .with_context(|| format!("failed to create {}", parent.display()))?;
    }

    let text = match std::fs::read_to_string(&path) {
        Ok(s) => s,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            info!(path = %path.display(), "config not found, creating");
            String::new()
        }
        Err(e) => {
            return Err(anyhow::Error::new(e)
                .context(format!("failed to read {}", path.display())));
        }
    };

    let mut doc: DocumentMut = text
        .parse()
        .with_context(|| format!("failed to parse {}", path.display()))?;

    ensure_table(&mut doc, "inference");
    let inference = doc["inference"]
        .as_table_mut()
        .context("config [inference] is not a table")?;
    inference["model_path"] = value(model.to_string_lossy().into_owned());

    ensure_table(&mut doc, "wallpaper");
    let wallpaper = doc["wallpaper"]
        .as_table_mut()
        .context("config [wallpaper] is not a table")?;
    wallpaper["color"] = value(paths.color.to_string_lossy().into_owned());
    wallpaper["depth"] = value(paths.depth.to_string_lossy().into_owned());
    wallpaper.remove("path"); // legacy field from pre-workspace schema

    std::fs::write(&path, doc.to_string())
        .with_context(|| format!("failed to write {}", path.display()))?;

    info!(path = %path.display(), "daemon config updated");
    Ok(())
}

fn ensure_table(doc: &mut DocumentMut, key: &str) {
    if doc.get(key).is_none() {
        doc.insert(key, toml_edit::Item::Table(toml_edit::Table::new()));
    }
}