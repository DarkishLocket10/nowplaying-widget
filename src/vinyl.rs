use std::{collections::HashMap, f32::consts::TAU, sync::Arc};

use eframe::egui::{Color32, ColorImage, Vec2};

use crate::config::VinylThumbnailConfig;

#[derive(Debug, Clone)]
pub struct VinylThumbnailOptions {
    pub swirl_strength: f32,
    pub label_ratio: f32,
    pub output_size: usize,
    pub groove_count: usize,
}

impl VinylThumbnailOptions {
    pub fn from_config(
        config: &VinylThumbnailConfig,
        source_width: usize,
        source_height: usize,
    ) -> Self {
        let max_dim = source_width.max(source_height).max(128);
        let mut output_size = max_dim.clamp(128, 1024);
        if output_size % 2 == 1 {
            output_size += 1;
        }
        Self {
            swirl_strength: config.swirl_strength(),
            label_ratio: config.label_ratio(),
            output_size,
            groove_count: 12,
        }
    }

    #[allow(dead_code)]
    pub fn cache_key(&self, hash: u64) -> VinylCacheKey {
        VinylCacheKey {
            hash,
            output_size: self.output_size as u32,
            swirl_strength: quantize(self.swirl_strength, 0.01),
            label_ratio: quantize(self.label_ratio, 0.001),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[allow(dead_code)]
pub struct VinylCacheKey {
    hash: u64,
    output_size: u32,
    swirl_strength: u16,
    label_ratio: u16,
}

#[derive(Default)]
#[allow(dead_code)]
pub struct VinylCache {
    entries: HashMap<VinylCacheKey, Arc<ColorImage>>,
}

impl VinylCache {
    #[allow(dead_code)]
    pub fn get_or_insert_with<F>(&mut self, key: VinylCacheKey, make: F) -> Arc<ColorImage>
    where
        F: FnOnce() -> ColorImage,
    {
        self.entries
            .entry(key)
            .or_insert_with(|| Arc::new(make()))
            .clone()
    }
}

#[derive(Debug, Clone)]
pub struct VinylSpin {
    angle: f32,
    speed: f32,
}

impl VinylSpin {
    pub fn new() -> Self {
        // 33 1/3 RPM ≈ 3.49 rad/s
        Self {
            angle: 0.0,
            speed: 3.49,
        }
    }

    pub fn advance(&mut self, dt: f32, spinning: bool) {
        if spinning && dt > 0.0 {
            self.angle = (self.angle + self.speed * dt).rem_euclid(TAU as f32);
        }
    }

    pub fn reset(&mut self) {
        self.angle = 0.0;
    }

    pub fn angle(&self) -> f32 {
        self.angle
    }
}

pub fn render_vinyl(image: &ColorImage, options: &VinylThumbnailOptions) -> ColorImage {
    let size = options.output_size;
    let mut output = ColorImage::new([size, size], vec![Color32::TRANSPARENT; size * size]);

    let radius_px = (size as f32) / 2.0;
    let inv_radius = 1.0 / radius_px;

    let src_width = image.size[0] as f32;
    let src_height = image.size[1] as f32;
    let src_min = src_width.min(src_height);
    let src_radius = src_min / 2.0;
    let src_center = Vec2::new(src_width / 2.0, src_height / 2.0);

    let swirl_strength = options.swirl_strength;
    let label_ratio = options.label_ratio.clamp(0.1, 0.6);
    let groove_count = options.groove_count.max(6);
    let groove_half_width = 0.015;
    let groove_intensity = 0.14;
    let label_ring_width = 0.015;
    let label_ring_highlight = 0.18;
    let edge_shadow_start = 0.82;
    let outer_vignette = 0.28;
    let sheen_angle = -0.35..=0.25;
    let sheen_strength = 0.22;
    let hole_radius_px = (size as f32 / 100.0).clamp(3.5, 7.5);
    let hole_ratio = hole_radius_px / radius_px;

    for y in 0..size {
        for x in 0..size {
            let fx = x as f32 + 0.5;
            let fy = y as f32 + 0.5;
            let dx = (fx - radius_px) * inv_radius;
            let dy = (fy - radius_px) * inv_radius;
            let r = (dx * dx + dy * dy).sqrt();

            let idx = y * size + x;

            if r >= 1.0 {
                output.pixels[idx] = Color32::TRANSPARENT;
                continue;
            }

            let base_angle = dy.atan2(dx);
            let mut sample_angle = base_angle;
            let mut sample_radius = r;

            if r <= label_ratio {
                if label_ratio > 0.0 {
                    sample_radius = (r / label_ratio).min(1.0);
                }
                sample_angle = base_angle;
            } else {
                // Increase perceptual impact of the swirl by using a steeper curve
                // and a modest multiplier. This keeps the configured `swirl_strength`
                // meaningful while making the outer distortion visually stronger.
                let normalized = ((r - label_ratio) / (1.0 - label_ratio)).clamp(0.0, 1.0);
                // Use a power curve to bias intensity toward the outer edge and
                // amplify by 1.4x for perceptual punch.
                let swirl = swirl_strength * normalized.powf(1.6) * 1.4;
                sample_angle += swirl;
            }

            let sample_px_radius = sample_radius * src_radius;
            let sample_x = src_center.x + sample_angle.cos() * sample_px_radius;
            let sample_y = src_center.y + sample_angle.sin() * sample_px_radius;
            let mut color = sample_bilinear(image, sample_x, sample_y);

            if r > label_ratio {
                let normalized = ((r - label_ratio) / (1.0 - label_ratio)).clamp(0.0, 1.0);
                let mut groove_shade = 0.0;
                for i in 1..=groove_count {
                    let ring_pos = i as f32 / (groove_count as f32 + 1.0);
                    let dist = (normalized - ring_pos).abs();
                    if dist < groove_half_width {
                        let t = 1.0 - (dist / groove_half_width);
                        groove_shade += t * t;
                    }
                }
                if groove_shade > 0.0 {
                    color = darken(color, groove_shade * groove_intensity);
                }
            }

            let ring_delta = (r - label_ratio).abs();
            if ring_delta < label_ring_width {
                let t = 1.0 - (ring_delta / label_ring_width);
                color = lighten(color, t * label_ring_highlight);
            }

            if r > edge_shadow_start {
                let t = ((r - edge_shadow_start) / (1.0 - edge_shadow_start)).clamp(0.0, 1.0);
                color = darken(color, t * (0.25 + outer_vignette));
            } else {
                let rim = (r / edge_shadow_start).clamp(0.0, 1.0);
                color = darken(color, rim.powf(2.4) * outer_vignette);
            }

            if r <= hole_ratio {
                color = Color32::from_rgba_unmultiplied(32, 32, 32, 255);
            } else if sheen_angle.contains(&base_angle) && r > label_ratio + 0.05 {
                let angle_t =
                    (base_angle - sheen_angle.start()) / (sheen_angle.end() - sheen_angle.start());
                let highlight = (1.0 - angle_t.clamp(0.0, 1.0)).powf(2.2);
                color = lighten(color, highlight * sheen_strength);
            }

            if r < label_ratio {
                let center_bright = (label_ratio - r) / label_ratio;
                color = lighten(color, center_bright * 0.08);
            }

            let alpha = if r > 0.995 {
                let t = ((1.0 - r) / (1.0 - 0.995)).clamp(0.0, 1.0);
                (t * 255.0) as u8
            } else {
                255
            };

            output.pixels[idx] =
                Color32::from_rgba_unmultiplied(color.r(), color.g(), color.b(), alpha);
        }
    }

    output
}

fn sample_bilinear(image: &ColorImage, x: f32, y: f32) -> Color32 {
    let width = image.size[0] as i32;
    let height = image.size[1] as i32;
    if width == 0 || height == 0 {
        return Color32::BLACK;
    }

    let clamped_x = x.clamp(0.0, (width - 1) as f32);
    let clamped_y = y.clamp(0.0, (height - 1) as f32);

    let x0 = clamped_x.floor() as i32;
    let y0 = clamped_y.floor() as i32;
    let x1 = (x0 + 1).min(width - 1);
    let y1 = (y0 + 1).min(height - 1);

    let tx = clamped_x - x0 as f32;
    let ty = clamped_y - y0 as f32;

    let c00 = image.pixels[(y0 as usize) * image.size[0] + x0 as usize];
    let c10 = image.pixels[(y0 as usize) * image.size[0] + x1 as usize];
    let c01 = image.pixels[(y1 as usize) * image.size[0] + x0 as usize];
    let c11 = image.pixels[(y1 as usize) * image.size[0] + x1 as usize];

    let top = lerp_color(c00, c10, tx);
    let bottom = lerp_color(c01, c11, tx);
    lerp_color(top, bottom, ty)
}

fn lerp_color(a: Color32, b: Color32, t: f32) -> Color32 {
    let t = t.clamp(0.0, 1.0);
    let inv = 1.0 - t;
    let ar = a.r() as f32;
    let ag = a.g() as f32;
    let ab = a.b() as f32;
    let aa = a.a() as f32;
    let br = b.r() as f32;
    let bg = b.g() as f32;
    let bb = b.b() as f32;
    let ba = b.a() as f32;
    Color32::from_rgba_unmultiplied(
        (ar * inv + br * t).round() as u8,
        (ag * inv + bg * t).round() as u8,
        (ab * inv + bb * t).round() as u8,
        (aa * inv + ba * t).round() as u8,
    )
}

fn darken(color: Color32, amount: f32) -> Color32 {
    let amount = amount.clamp(0.0, 1.0);
    let r = (color.r() as f32 * (1.0 - amount))
        .round()
        .clamp(0.0, 255.0) as u8;
    let g = (color.g() as f32 * (1.0 - amount))
        .round()
        .clamp(0.0, 255.0) as u8;
    let b = (color.b() as f32 * (1.0 - amount))
        .round()
        .clamp(0.0, 255.0) as u8;
    Color32::from_rgba_unmultiplied(r, g, b, color.a())
}

fn lighten(color: Color32, amount: f32) -> Color32 {
    let amount = amount.clamp(0.0, 1.0);
    let r = (color.r() as f32 + (255.0 - color.r() as f32) * amount)
        .round()
        .clamp(0.0, 255.0) as u8;
    let g = (color.g() as f32 + (255.0 - color.g() as f32) * amount)
        .round()
        .clamp(0.0, 255.0) as u8;
    let b = (color.b() as f32 + (255.0 - color.b() as f32) * amount)
        .round()
        .clamp(0.0, 255.0) as u8;
    Color32::from_rgba_unmultiplied(r, g, b, color.a())
}

#[allow(dead_code)]
fn quantize(value: f32, precision: f32) -> u16 {
    let scaled = (value / precision).round();
    scaled.clamp(0.0, u16::MAX as f32).round() as u16
}

#[cfg(test)]
mod tests {
    use super::*;

    fn solid_image(size: usize, color: Color32) -> ColorImage {
        ColorImage::new([size, size], vec![color; size * size])
    }

    #[test]
    fn cache_reuses_images() {
        let mut cache = VinylCache::default();
        let options = VinylThumbnailOptions {
            swirl_strength: 2.5,
            label_ratio: 0.35,
            output_size: 256,
            groove_count: 8,
        };
        let key = options.cache_key(123);
        let first = cache.get_or_insert_with(key, || solid_image(256, Color32::WHITE));
        let second = cache.get_or_insert_with(key, || solid_image(256, Color32::BLACK));
        assert!(Arc::ptr_eq(&first, &second));
    }

    #[test]
    fn spin_respects_toggle() {
        let mut spin = VinylSpin::new();
        spin.advance(1.0, true);
        let angle = spin.angle();
        spin.advance(1.0, false);
        assert_eq!(spin.angle(), angle);
    }

    #[test]
    fn render_produces_expected_size() {
        let image = solid_image(128, Color32::from_rgb(120, 60, 20));
        let opts = VinylThumbnailOptions {
            swirl_strength: 2.5,
            label_ratio: 0.35,
            output_size: 256,
            groove_count: 8,
        };
        let vinyl = render_vinyl(&image, &opts);
        assert_eq!(vinyl.size, [256, 256]);
    }

    #[test]
    fn render_supports_multiple_sizes() {
        let image = solid_image(64, Color32::from_rgb(200, 30, 30));
        let mut opts = VinylThumbnailOptions {
            swirl_strength: 2.5,
            label_ratio: 0.35,
            output_size: 128,
            groove_count: 8,
        };
        let small = render_vinyl(&image, &opts);
        assert_eq!(small.size, [128, 128]);

        opts.output_size = 512;
        let large = render_vinyl(&image, &opts);
        assert_eq!(large.size, [512, 512]);
    }
}
