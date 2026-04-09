use anyhow::Result;
use std::path::Path;
use tracing::{debug, info};

pub struct DepthMap {
    pub data: Vec<f32>,
    pub width: u32,
    pub height: u32,
}

const MODEL_INPUT_SIZE: u32 = 518;

// ImageNet normalization constants used by Depth Anything V2/V3
const MEAN: [f32; 3] = [0.485, 0.456, 0.406];
const STD: [f32; 3] = [0.229, 0.224, 0.225];

/// Run depth estimation on an already-loaded RGBA image.
/// Returns a normalized [0, 1] depth map at the source image's resolution
/// where 1.0 = closest to the camera.
pub fn estimate(rgba: &image::RgbaImage, model_path: &Path) -> Result<DepthMap> {
    let (orig_w, orig_h) = rgba.dimensions();

    info!(w = orig_w, h = orig_h, "running depth estimation");

    let resized = image::imageops::resize(
        rgba,
        MODEL_INPUT_SIZE,
        MODEL_INPUT_SIZE,
        image::imageops::FilterType::Lanczos3,
    );

    let npixels = (MODEL_INPUT_SIZE * MODEL_INPUT_SIZE) as usize;
    let mut input_data = vec![0.0f32; 3 * npixels];
    for y in 0..MODEL_INPUT_SIZE {
        for x in 0..MODEL_INPUT_SIZE {
            let pixel = resized.get_pixel(x, y);
            let idx = (y * MODEL_INPUT_SIZE + x) as usize;
            input_data[idx] = (pixel[0] as f32 / 255.0 - MEAN[0]) / STD[0];
            input_data[npixels + idx] = (pixel[1] as f32 / 255.0 - MEAN[1]) / STD[1];
            input_data[2 * npixels + idx] = (pixel[2] as f32 / 255.0 - MEAN[2]) / STD[2];
        }
    }

    let raw_depth = run_inference(model_path, &input_data)?;

    // Normalize to [0, 1] with 1.0 = closest.
    // DA2 outputs inverse depth (higher = closer) — already correct.
    // DA3 outputs direct depth (higher = farther) — needs inversion.
    let (d_min, d_max) = raw_depth.data.iter().fold((f32::MAX, f32::MIN), |(mn, mx), &v| {
        (mn.min(v), mx.max(v))
    });
    let range = (d_max - d_min).max(1e-6);

    let normalized: Vec<f32> = if raw_depth.invert {
        raw_depth.data.iter().map(|&v| 1.0 - (v - d_min) / range).collect()
    } else {
        raw_depth.data.iter().map(|&v| (v - d_min) / range).collect()
    };

    let data = resize_depth(
        &normalized,
        MODEL_INPUT_SIZE,
        MODEL_INPUT_SIZE,
        orig_w,
        orig_h,
    );

    Ok(DepthMap { data, width: orig_w, height: orig_h })
}

struct InferenceResult {
    data: Vec<f32>,
    /// True if output is direct depth (DA3), false if inverse depth (DA2).
    invert: bool,
}

fn run_inference(model_path: &Path, input: &[f32]) -> Result<InferenceResult> {
    let mut session = ort::session::Session::builder()
        .map_err(|e| anyhow::anyhow!("failed to create ONNX session builder: {e}"))?
        .with_optimization_level(ort::session::builder::GraphOptimizationLevel::Level3)
        .map_err(|e| anyhow::anyhow!("failed to set optimization level: {e}"))?
        .with_execution_providers([ort::ep::CUDA::default().build()])
        .map_err(|e| anyhow::anyhow!("failed to set execution providers: {e}"))?
        .commit_from_file(model_path)
        .map_err(|e| anyhow::anyhow!(
            "failed to load ONNX model from {}: {e}",
            model_path.display()
        ))?;

    let sz = MODEL_INPUT_SIZE as i64;

    // Try DA3 (rank 5: [batch, views, C, H, W]) first, fall back to DA2 (rank 4).
    let result: Result<InferenceResult> = {
        let data: Box<[f32]> = input.to_vec().into_boxed_slice();
        let tensor = ort::value::Tensor::<f32>::from_array((vec![1i64, 1, 3, sz, sz], data))
            .map_err(|e| anyhow::anyhow!("failed to create input tensor: {e}"))?;
        match session.run(ort::inputs![tensor]) {
            Ok(outputs) => {
                debug!("inference succeeded with rank-5 input (DA3)");
                let (_shape, d) = outputs[0]
                    .try_extract_tensor::<f32>()
                    .map_err(|e| anyhow::anyhow!("failed to extract output: {e}"))?;
                Ok(InferenceResult { data: d.to_vec(), invert: true })
            }
            Err(_) => {
                debug!("rank-5 failed, retrying with rank-4 input (DA2)");
                Err(anyhow::anyhow!("rank-5 failed"))
            }
        }
    };

    if let Ok(data) = result {
        return Ok(data);
    }

    let data: Box<[f32]> = input.to_vec().into_boxed_slice();
    let tensor = ort::value::Tensor::<f32>::from_array((vec![1i64, 3, sz, sz], data))
        .map_err(|e| anyhow::anyhow!("failed to create input tensor: {e}"))?;
    let outputs = session.run(ort::inputs![tensor])
        .map_err(|e| anyhow::anyhow!("inference failed with both rank-5 and rank-4: {e}"))?;

    let (_shape, data) = outputs[0]
        .try_extract_tensor::<f32>()
        .map_err(|e| anyhow::anyhow!("failed to extract output: {e}"))?;

    Ok(InferenceResult { data: data.to_vec(), invert: false })
}

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