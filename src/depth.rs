use anyhow::Result;
use std::path::{Path, PathBuf};
use tracing::{debug, info};

/// Depth map at the wallpaper's native resolution.
/// Values are normalized to [0.0, 1.0] where 1.0 = closest to camera.
pub struct DepthMap {
    pub data: Vec<f32>,
    pub width: u32,
    pub height: u32,
}

/// Cache metadata stored alongside the raw depth data.
#[derive(serde::Serialize, serde::Deserialize)]
struct CacheMeta {
    width: u32,
    height: u32,
    model_version: String,
    source_hash: String,
}

const MODEL_VERSION: &str = "midas-v2.1-small";
const MIDAS_INPUT_SIZE: u32 = 256;

/// Produce a depth map for the given wallpaper image.
/// Checks the cache first; runs MiDaS inference on miss.
pub fn get_depth_map(wallpaper_path: &Path, model_path: &Path) -> Result<DepthMap> {
    let img = image::open(wallpaper_path)
        .map_err(|e| anyhow::anyhow!("failed to open {}: {e}", wallpaper_path.display()))?;
    let rgba = img.to_rgba8();
    let (orig_w, orig_h) = rgba.dimensions();

    let source_hash = hash_image_bytes(&rgba);

    // Check cache
    if let Some(cached) = load_from_cache(&source_hash, orig_w, orig_h) {
        info!(w = orig_w, h = orig_h, "loaded depth map from cache");
        return Ok(cached);
    }

    info!(w = orig_w, h = orig_h, "running MiDaS inference...");

    // Preprocess: resize to model input size
    let resized = image::imageops::resize(
        &rgba,
        MIDAS_INPUT_SIZE,
        MIDAS_INPUT_SIZE,
        image::imageops::FilterType::Lanczos3,
    );

    // Build CHW float tensor [1, 3, H, W] normalized to [0, 1]
    let npixels = (MIDAS_INPUT_SIZE * MIDAS_INPUT_SIZE) as usize;
    let mut input_tensor = vec![0.0f32; 3 * npixels];
    for y in 0..MIDAS_INPUT_SIZE {
        for x in 0..MIDAS_INPUT_SIZE {
            let pixel = resized.get_pixel(x, y);
            let idx = (y * MIDAS_INPUT_SIZE + x) as usize;
            input_tensor[idx] = pixel[0] as f32 / 255.0;              // R
            input_tensor[npixels + idx] = pixel[1] as f32 / 255.0;    // G
            input_tensor[2 * npixels + idx] = pixel[2] as f32 / 255.0; // B
        }
    }

    // Run ONNX inference
    let raw_depth = run_midas_onnx(model_path, &input_tensor)?;

    // Normalize to [0, 1] — MiDaS outputs inverse depth (higher = closer),
    // which is what we want for parallax (closer objects shift more).
    let (d_min, d_max) = raw_depth.iter().fold((f32::MAX, f32::MIN), |(mn, mx), &v| {
        (mn.min(v), mx.max(v))
    });
    let range = (d_max - d_min).max(1e-6);

    let normalized: Vec<f32> = raw_depth
        .iter()
        .map(|&v| (v - d_min) / range)
        .collect();

    // Resize to original image dimensions using bilinear interpolation
    let depth_data = resize_depth(
        &normalized,
        MIDAS_INPUT_SIZE,
        MIDAS_INPUT_SIZE,
        orig_w,
        orig_h,
    );

    let result = DepthMap {
        data: depth_data,
        width: orig_w,
        height: orig_h,
    };

    save_to_cache(&source_hash, &result);

    info!(w = orig_w, h = orig_h, "depth map ready");
    Ok(result)
}

fn run_midas_onnx(model_path: &Path, input: &[f32]) -> Result<Vec<f32>> {
    // ort 2.x: Error types don't impl std::error::Error + Send + Sync,
    // so we use map_err throughout instead of anyhow's .context().
    let mut session = ort::session::Session::builder()
        .map_err(|e| anyhow::anyhow!("failed to create ONNX session builder: {e}"))?
        .with_optimization_level(ort::session::builder::GraphOptimizationLevel::Level3)
        .map_err(|e| anyhow::anyhow!("failed to set optimization level: {e}"))?
        .commit_from_file(model_path)
        .map_err(|e| anyhow::anyhow!(
            "failed to load ONNX model from {}: {e}",
            model_path.display()
        ))?;

    let input_shape: Vec<i64> = vec![1, 3, MIDAS_INPUT_SIZE as i64, MIDAS_INPUT_SIZE as i64];
    let input_data: Box<[f32]> = input.to_vec().into_boxed_slice();
    let input_tensor = ort::value::Tensor::<f32>::from_array((input_shape, input_data))
        .map_err(|e| anyhow::anyhow!("failed to create input tensor: {e}"))?;

    let outputs = session
        .run(ort::inputs![input_tensor])
        .map_err(|e| anyhow::anyhow!("MiDaS inference failed: {e}"))?;

    // Access the first (and only) output tensor by index.
    // ort 2.x: try_extract_tensor returns Result<(&Shape, &[f32])>
    let output = &outputs[0];
    let (_shape, data) = output
        .try_extract_tensor::<f32>()
        .map_err(|e| anyhow::anyhow!("failed to extract f32 tensor from output: {e}"))?;

    Ok(data.to_vec())
}

/// Bilinear interpolation resize for a single-channel f32 buffer.
fn resize_depth(src: &[f32], src_w: u32, src_h: u32, dst_w: u32, dst_h: u32) -> Vec<f32> {
    let mut dst = vec![0.0f32; (dst_w * dst_h) as usize];

    for dy in 0..dst_h {
        for dx in 0..dst_w {
            let sx = (dx as f32 + 0.5) * (src_w as f32 / dst_w as f32) - 0.5;
            let sy = (dy as f32 + 0.5) * (src_h as f32 / dst_h as f32) - 0.5;

            let x0 = sx.floor().max(0.0) as u32;
            let y0 = sy.floor().max(0.0) as u32;
            let x1 = (x0 + 1).min(src_w - 1);
            let y1 = (y0 + 1).min(src_h - 1);

            let fx = sx - x0 as f32;
            let fy = sy - y0 as f32;

            let v00 = src[(y0 * src_w + x0) as usize];
            let v10 = src[(y0 * src_w + x1) as usize];
            let v01 = src[(y1 * src_w + x0) as usize];
            let v11 = src[(y1 * src_w + x1) as usize];

            let v = v00 * (1.0 - fx) * (1.0 - fy)
                + v10 * fx * (1.0 - fy)
                + v01 * (1.0 - fx) * fy
                + v11 * fx * fy;

            dst[(dy * dst_w + dx) as usize] = v;
        }
    }

    dst
}

fn hash_image_bytes(img: &image::RgbaImage) -> String {
    blake3::hash(img.as_raw()).to_hex().to_string()
}

fn cache_dir() -> PathBuf {
    directories::ProjectDirs::from("", "", "depthpaper")
        .map(|dirs| dirs.cache_dir().to_path_buf())
        .unwrap_or_else(|| PathBuf::from("/tmp/depthpaper-cache"))
}

fn cache_paths(source_hash: &str) -> (PathBuf, PathBuf) {
    let dir = cache_dir();
    (
        dir.join(format!("{source_hash}.depth")),
        dir.join(format!("{source_hash}.json")),
    )
}

fn load_from_cache(source_hash: &str, expected_w: u32, expected_h: u32) -> Option<DepthMap> {
    let (data_path, meta_path) = cache_paths(source_hash);

    let meta_bytes = std::fs::read(&meta_path).ok()?;
    let meta: CacheMeta = serde_json::from_slice(&meta_bytes).ok()?;

    if meta.width != expected_w || meta.height != expected_h || meta.model_version != MODEL_VERSION {
        debug!("cache miss: metadata mismatch");
        return None;
    }

    let data_bytes = std::fs::read(&data_path).ok()?;
    let expected_len = (expected_w * expected_h) as usize * std::mem::size_of::<f32>();
    if data_bytes.len() != expected_len {
        debug!("cache miss: data size mismatch");
        return None;
    }

    let data: Vec<f32> = data_bytes
        .chunks_exact(4)
        .map(|c| f32::from_le_bytes([c[0], c[1], c[2], c[3]]))
        .collect();

    Some(DepthMap {
        data,
        width: expected_w,
        height: expected_h,
    })
}

fn save_to_cache(source_hash: &str, depth: &DepthMap) {
    let (data_path, meta_path) = cache_paths(source_hash);

    if let Some(parent) = data_path.parent() {
        let _ = std::fs::create_dir_all(parent);
    }

    let meta = CacheMeta {
        width: depth.width,
        height: depth.height,
        model_version: MODEL_VERSION.to_string(),
        source_hash: source_hash.to_string(),
    };

    let data_bytes: Vec<u8> = depth.data.iter().flat_map(|v| v.to_le_bytes()).collect();

    if let Err(e) = std::fs::write(&data_path, &data_bytes) {
        tracing::warn!("failed to write depth cache: {e}");
        return;
    }

    match serde_json::to_vec_pretty(&meta) {
        Ok(json) => {
            if let Err(e) = std::fs::write(&meta_path, json) {
                tracing::warn!("failed to write depth cache meta: {e}");
            } else {
                debug!(path = %data_path.display(), "depth map cached");
            }
        }
        Err(e) => tracing::warn!("failed to serialize cache meta: {e}"),
    }
}