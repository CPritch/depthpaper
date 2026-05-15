use anyhow::{Context, Result};
use std::path::Path;

/// Depth map at the wallpaper's native resolution.
/// Values are u16 normalized, the GPU treats them as [0.0, 1.0] via R16Unorm.
pub struct DepthMap {
    pub data: Vec<u16>,
    pub width: u32,
    pub height: u32,
}

/// Load a 16-bit grayscale PNG produced by `shiftpaper-cli bake`.
pub fn load_depth_map(path: &Path) -> Result<DepthMap> {
    let img = image::open(path)
        .with_context(|| format!("failed to open depth map: {}", path.display()))?;
    let luma = img.to_luma16();
    let (width, height) = luma.dimensions();
    let data = luma.into_raw();

    Ok(DepthMap { data, width, height })
}