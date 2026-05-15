use anyhow::{Context, Result};
use indicatif::{ProgressBar, ProgressStyle};
use std::path::{Path, PathBuf};
use toml_edit::{DocumentMut, value};
use tracing::info;

pub const DEFAULT_MODEL_URL: &str =
    "https://huggingface.co/onnx-community/depth-anything-v2-small/resolve/main/onnx/model.onnx";
const DEFAULT_MODEL_FILENAME: &str = "depth_anything_v2_small.onnx";

pub fn models_dir() -> PathBuf {
    directories::ProjectDirs::from("", "", "shiftpaper")
        .map(|d| d.data_local_dir().join("models"))
        .unwrap_or_else(|| {
            PathBuf::from(std::env::var("HOME").unwrap_or_default())
                .join(".local/share/shiftpaper/models")
        })
}

pub fn default_model_path() -> PathBuf {
    models_dir().join(DEFAULT_MODEL_FILENAME)
}

pub fn fetch_model(url: &str, dest: &Path, force: bool) -> Result<()> {
    if dest.exists() && !force {
        eprintln!("model already at {}", dest.display());
        eprintln!("use --force to re-download");
        return Ok(());
    }

    if let Some(parent) = dest.parent() {
        std::fs::create_dir_all(parent)
            .with_context(|| format!("failed to create {}", parent.display()))?;
    }

    eprintln!("downloading from {url}");

    let response = ureq::get(url)
        .call()
        .with_context(|| format!("HTTP GET failed for {url}"))?;

    let content_length: Option<u64> = response
        .headers()
        .get("content-length")
        .and_then(|v| v.to_str().ok())
        .and_then(|s| s.parse::<u64>().ok());

    let pb = match content_length {
        Some(n) => {
            let pb = ProgressBar::new(n);
            pb.set_style(
                ProgressStyle::with_template(
                    "{spinner:.green} [{elapsed_precise}] [{bar:40.cyan/blue}] \
                     {bytes}/{total_bytes} ({eta})",
                )
                .expect("valid template")
                .progress_chars("#>-"),
            );
            pb
        }
        None => {
            let pb = ProgressBar::new_spinner();
            pb.set_style(
                ProgressStyle::with_template(
                    "{spinner:.green} [{elapsed_precise}] {bytes} downloaded",
                )
                .expect("valid template"),
            );
            pb
        }
    };

    // Write to .part file first so a failed download doesn't leave a corrupt
    // .onnx that would pass the exists() check on the next invocation.
    let part_path = dest.with_extension("onnx.part");
    {
        let mut reader = pb.wrap_read(response.into_body().into_reader());
        let mut file = std::fs::File::create(&part_path)
            .with_context(|| format!("failed to create {}", part_path.display()))?;
        std::io::copy(&mut reader, &mut file).context("download failed mid-transfer")?;
    }
    pb.finish_with_message("done");

    std::fs::rename(&part_path, dest)
        .with_context(|| format!("failed to finalise {}", dest.display()))?;

    eprintln!("saved to {}", dest.display());
    Ok(())
}

pub fn persist_model_path(model_path: &Path) -> Result<()> {
    let config_path = crate::config::config_path();

    if let Some(parent) = config_path.parent() {
        std::fs::create_dir_all(parent)
            .with_context(|| format!("failed to create {}", parent.display()))?;
    }

    let text = match std::fs::read_to_string(&config_path) {
        Ok(s) => s,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => String::new(),
        Err(e) => {
            return Err(
                anyhow::Error::new(e).context(format!("failed to read {}", config_path.display()))
            );
        }
    };

    let mut doc: DocumentMut = text
        .parse()
        .with_context(|| format!("failed to parse {}", config_path.display()))?;

    if doc.get("inference").is_none() {
        doc.insert("inference", toml_edit::Item::Table(toml_edit::Table::new()));
    }
    let inference = doc["inference"]
        .as_table_mut()
        .context("config [inference] is not a table")?;
    inference["model_path"] = value(model_path.to_string_lossy().into_owned());

    std::fs::write(&config_path, doc.to_string())
        .with_context(|| format!("failed to write {}", config_path.display()))?;

    info!(path = %config_path.display(), "inference config updated");
    Ok(())
}
