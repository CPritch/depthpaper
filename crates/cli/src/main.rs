mod cache;
mod depth;

use anyhow::{Context, Result};
use clap::{Parser, Subcommand};
use std::path::{Path, PathBuf};
use toml_edit::{value, DocumentMut};
use tracing::info;
use tracing_subscriber::EnvFilter;

#[derive(Parser)]
#[command(name = "depthpaper-cli", version, about = "Bake depth maps and configure depthpaper")]
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
        /// Path to the Depth Anything ONNX model.
        #[arg(short, long, env = "DEPTHPAPER_MODEL")]
        model: PathBuf,
    },
    /// Bake and set as the active wallpaper in the daemon's config.
    Set {
        /// Source image.
        input: PathBuf,
        /// Path to the Depth Anything ONNX model.
        #[arg(short, long, env = "DEPTHPAPER_MODEL")]
        model: PathBuf,
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
            bake(&input, out.as_deref(), &model)?;
            Ok(())
        }
        Command::Set { input, model } => set(&input, &model),
    }
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
    update_daemon_config(&paths.color)?;
    eprintln!();
    eprintln!("wallpaper set. restart the daemon to apply:");
    eprintln!("  systemctl --user restart depthpaper");
    Ok(())
}

fn daemon_config_path() -> PathBuf {
    if let Ok(xdg) = std::env::var("XDG_CONFIG_HOME") {
        PathBuf::from(xdg).join("depthpaper/config.toml")
    } else if let Ok(home) = std::env::var("HOME") {
        PathBuf::from(home).join(".config/depthpaper/config.toml")
    } else {
        PathBuf::from("config.toml")
    }
}

fn update_daemon_config(color_path: &Path) -> Result<()> {
    let path = daemon_config_path();
    let text = std::fs::read_to_string(&path)
        .with_context(|| format!("failed to read {}", path.display()))?;

    let mut doc: DocumentMut = text
        .parse()
        .with_context(|| format!("failed to parse {}", path.display()))?;

    if doc.get("wallpaper").is_none() {
        doc.insert("wallpaper", toml_edit::Item::Table(toml_edit::Table::new()));
    }

    let wallpaper = doc["wallpaper"]
        .as_table_mut()
        .context("config [wallpaper] is not a table")?;

    wallpaper["color"] = value(color_path.to_string_lossy().into_owned());
    // Clear stale/legacy fields so sibling inference resolves the new depth path
    wallpaper.remove("depth");
    wallpaper.remove("path");

    std::fs::write(&path, doc.to_string())
        .with_context(|| format!("failed to write {}", path.display()))?;

    info!(path = %path.display(), "daemon config updated");
    Ok(())
}