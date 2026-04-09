use anyhow::{Context, Result};
use std::path::Path;

/// Depth map at the wallpaper's native resolution.
/// Values are normalized to [0.0, 1.0] where 1.0 = closest to camera.
pub struct DepthMap {
    pub data: Vec<f32>,
    pub width: u32,
    pub height: u32,
}

/// Load a 16-bit grayscale PNG produced by `depthpaper-cli bake` and
/// convert it to normalized f32 values for GPU upload.
pub fn load_depth_map(path: &Path) -> Result<DepthMap> {
    let img = image::open(path)
        .with_context(|| format!("failed to open depth map: {}", path.display()))?;
    let luma = img.to_luma16();
    let (width, height) = luma.dimensions();

    let data: Vec<f32> = luma
        .pixels()
        .map(|p| p.0[0] as f32 / 65535.0)
        .collect();

    Ok(DepthMap { data, width, height })
}