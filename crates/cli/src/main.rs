mod cache;
mod config;
mod depth;

use anyhow::{Context, Result};
use clap::{Parser, Subcommand};
use std::path::{Path, PathBuf};
use toml_edit::{value, DocumentMut};
use tracing::info;
use tracing_subscriber::EnvFilter;

#[derive(Parser)]
#[command(name = "depthpaper", version, about = "Bake depth maps and configure depthpaperd")]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    /// Bake a source image into a color + 16-bit depth PNG pair.
    Bake {
        /// Source image (jpeg, png, webp).
        input: PathBuf,
        /// Output directory. Defaults to the depthpaper cache directory.
        #[arg(short, long)]
        out: Option<PathBuf>,
        /// Path to the Depth Anything ONNX model. Falls back to
        /// $DEPTHPAPER_MODEL, then [inference] model_path in config.toml.
        #[arg(short, long, env = "DEPTHPAPER_MODEL")]
        model: Option<PathBuf>,
    },
    /// Bake and set as the active wallpaper in the daemon's config.
    Set {
        /// Source image.
        input: PathBuf,
        /// Path to the Depth Anything ONNX model. Falls back to
        /// $DEPTHPAPER_MODEL, then [inference] model_path in config.toml.
        /// When provided, the resolved path is persisted to config.
        #[arg(short, long, env = "DEPTHPAPER_MODEL")]
        model: Option<PathBuf>,
    },
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
    eprintln!("wallpaper set. restart the daemon to apply:");
    eprintln!("  systemctl --user restart depthpaperd");
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