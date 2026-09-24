use anyhow::{Context, Result, bail};
use image::{
    DynamicImage, ImageDecoder, ImageReader, Rgba, RgbaImage, imageops, imageops::FilterType,
    metadata::Orientation,
};

use crate::project::{EffectMode, Project};

pub fn load_assets(project: &Project) -> Result<(Vec<RgbaImage>, Vec<String>)> {
    let mut assets = Vec::with_capacity(project.images.len());
    let mut warnings = Vec::new();

    for path in &project.images {
        let mut decoder = ImageReader::open(path)
            .with_context(|| format!("無法開啟圖片：{}", path.display()))?
            .into_decoder()
            .with_context(|| format!("無法讀取圖片中繼資料：{}", path.display()))?;
        let orientation = decoder.orientation().unwrap_or(Orientation::NoTransforms);
        let mut oriented = DynamicImage::from_decoder(decoder)
            .with_context(|| format!("無法解碼圖片：{}", path.display()))?;
        oriented.apply_orientation(orientation);
        let decoded = oriented.to_rgba8();

        let mut alpha_min = u8::MAX;
        let mut alpha_max = 0_u8;
        for pixel in decoded.pixels() {
            alpha_min = alpha_min.min(pixel.0[3]);
            alpha_max = alpha_max.max(pixel.0[3]);
        }
        if alpha_max == 0 {
            bail!("圖片完全透明，無法產生內容：{}", path.display());
        }
        if alpha_min == 255 {
            warnings.push(format!(
                "{} 沒有透明區域；工具不會自動去背。",
                path.file_name().unwrap_or_default().to_string_lossy()
            ));
        }

        let max_width = (project.width as f32 * 0.64).floor().max(1.0);
        let max_height = (project.height as f32 * 0.64).floor().max(1.0);
        let scale = (max_width / decoded.width() as f32).min(max_height / decoded.height() as f32);
        let target_width = (decoded.width() as f32 * scale).round().max(1.0) as u32;
        let target_height = (decoded.height() as f32 * scale).round().max(1.0) as u32;
        let mut asset =
            imageops::resize(&decoded, target_width, target_height, FilterType::Lanczos3);

        if project.mode == EffectMode::Spin {
            let diagonal = (asset.width() as f32).hypot(asset.height() as f32);
            let max_diagonal = project.width.min(project.height) as f32 * 0.88;
            if diagonal > max_diagonal {
                let shrink = max_diagonal / diagonal;
                asset = imageops::resize(
                    &asset,
                    (asset.width() as f32 * shrink).round().max(1.0) as u32,
                    (asset.height() as f32 * shrink).round().max(1.0) as u32,
                    FilterType::Lanczos3,
                );
            }
        }
        assets.push(asset);
    }

    Ok((assets, warnings))
}

pub fn render_frame(index: usize, assets: &[RgbaImage], project: &Project) -> Result<RgbaImage> {
    if assets.is_empty() {
        bail!("沒有可供預覽的圖片。");
    }
    let width = project.width;
    let height = project.height;
    let slot_frames = project.slot_frames()?;
    let total_frames = project.total_frames()?.max(1);

    match project.mode {
        EffectMode::Carousel => {
            let frame = index % total_frames;
            let slot = frame / slot_frames;
            let local = frame % slot_frames;
            let transition_frames = project.transition_frames()?;
            let steady_frames = slot_frames - transition_frames;
            if local < steady_frames {
                return Ok(place(&assets[slot], width, height, 0.0, 0.0));
            }

            let amount = (local - steady_frames) as f32 / transition_frames as f32;
            let eased = amount * amount * (3.0 - 2.0 * amount);
            let shift = width as f32 * 0.075;
            let outgoing = place(&assets[slot], width, height, -shift * eased, 0.0);
            let incoming = place(
                &assets[(slot + 1) % assets.len()],
                width,
                height,
                shift * (1.0 - eased),
                0.0,
            );
            Ok(crossfade(&outgoing, &incoming, eased))
        }
        EffectMode::Float => {
            let phase = std::f32::consts::TAU * (index % total_frames) as f32 / total_frames as f32;
            let dx = width as f32 * 0.045 * phase.sin();
            let dy = -(height as f32) * 0.025 * (phase * 2.0).sin();
            Ok(place(&assets[0], width, height, dx, dy))
        }
        EffectMode::Spin => {
            let angle =
                -std::f32::consts::TAU * (index % total_frames) as f32 / total_frames as f32;
            let rotated = rotate_bicubic(&assets[0], angle);
            Ok(place(&rotated, width, height, 0.0, 0.0))
        }
    }
}

pub fn checkerboard(width: u32, height: u32) -> RgbaImage {
    let tile = (height / 12).max(12);
    RgbaImage::from_fn(width, height, |x, y| {
        let alternate = ((x / tile) + (y / tile)) % 2 == 1;
        if alternate {
            Rgba([91, 99, 112, 255])
        } else {
            Rgba([58, 64, 76, 255])
        }
    })
}

fn place(source: &RgbaImage, width: u32, height: u32, dx: f32, dy: f32) -> RgbaImage {
    let mut canvas = RgbaImage::new(width, height);
    let left = ((width as f32 - source.width() as f32) / 2.0 + dx).round() as i32;
    let top = ((height as f32 - source.height() as f32) / 2.0 + dy).round() as i32;

    for (x, y, pixel) in source.enumerate_pixels() {
        let target_x = left + x as i32;
        let target_y = top + y as i32;
        if target_x >= 0 && target_y >= 0 && target_x < width as i32 && target_y < height as i32 {
            canvas.put_pixel(target_x as u32, target_y as u32, *pixel);
        }
    }
    canvas
}

fn crossfade(a: &RgbaImage, b: &RgbaImage, amount: f32) -> RgbaImage {
    RgbaImage::from_fn(a.width(), a.height(), |x, y| {
        let left = a.get_pixel(x, y).0;
        let right = b.get_pixel(x, y).0;
        let alpha_a = left[3] as f32 / 255.0;
        let alpha_b = right[3] as f32 / 255.0;
        let alpha = alpha_a * (1.0 - amount) + alpha_b * amount;
        if alpha <= f32::EPSILON {
            return Rgba([0, 0, 0, 0]);
        }

        let mut rgba = [0; 4];
        for channel in 0..3 {
            let premultiplied = left[channel] as f32 / 255.0 * alpha_a * (1.0 - amount)
                + right[channel] as f32 / 255.0 * alpha_b * amount;
            rgba[channel] = ((premultiplied / alpha).clamp(0.0, 1.0) * 255.0).round() as u8;
        }
        rgba[3] = (alpha.clamp(0.0, 1.0) * 255.0).round() as u8;
        Rgba(rgba)
    })
}

fn rotate_bicubic(source: &RgbaImage, angle: f32) -> RgbaImage {
    let bound = (source.width() as f32).hypot(source.height() as f32).ceil() as u32;
    let output_width = bound.max(1);
    let output_height = bound.max(1);
    let center_x = (output_width as f32 - 1.0) / 2.0;
    let center_y = (output_height as f32 - 1.0) / 2.0;
    let source_center_x = (source.width() as f32 - 1.0) / 2.0;
    let source_center_y = (source.height() as f32 - 1.0) / 2.0;
    let cos = angle.cos();
    let sin = angle.sin();

    RgbaImage::from_fn(output_width, output_height, |x, y| {
        let target_x = x as f32 - center_x;
        let target_y = y as f32 - center_y;
        let source_x = cos * target_x + sin * target_y + source_center_x;
        let source_y = -sin * target_x + cos * target_y + source_center_y;
        sample_cubic(source, source_x, source_y)
    })
}

fn sample_cubic(source: &RgbaImage, x: f32, y: f32) -> Rgba<u8> {
    if x < -1.0 || y < -1.0 || x > source.width() as f32 || y > source.height() as f32 {
        return Rgba([0, 0, 0, 0]);
    }
    let base_x = x.floor() as i32;
    let base_y = y.floor() as i32;
    let mut accum = [0.0_f32; 4];
    for offset_y in -1..=2 {
        for offset_x in -1..=2 {
            let sample_x = base_x + offset_x;
            let sample_y = base_y + offset_y;
            if sample_x < 0
                || sample_y < 0
                || sample_x >= source.width() as i32
                || sample_y >= source.height() as i32
            {
                continue;
            }
            let weight = cubic_weight(x - sample_x as f32) * cubic_weight(y - sample_y as f32);
            let pixel = source.get_pixel(sample_x as u32, sample_y as u32).0;
            for channel in 0..4 {
                accum[channel] += pixel[channel] as f32 * weight;
            }
        }
    }
    Rgba(accum.map(|value| value.clamp(0.0, 255.0).round() as u8))
}

fn cubic_weight(distance: f32) -> f32 {
    let x = distance.abs();
    if x <= 1.0 {
        1.5 * x.powi(3) - 2.5 * x.powi(2) + 1.0
    } else if x < 2.0 {
        -0.5 * x.powi(3) + 2.5 * x.powi(2) - 4.0 * x + 2.0
    } else {
        0.0
    }
}
