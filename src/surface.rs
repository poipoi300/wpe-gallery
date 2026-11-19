//! Static matte grain and light reflected by the two sides of a preview.
use gpui::{Background, Hsla, RenderImage, linear_color_stop, linear_gradient, rgb};
use image::{Frame, Rgba, RgbaImage};
use std::sync::{Arc, OnceLock};

#[derive(Clone, Copy)]
pub struct EdgeLight {
    left: Hsla,
    right: Hsla,
}

impl Default for EdgeLight {
    fn default() -> Self {
        Self {
            left: rgb(0x414441).into(),
            right: rgb(0x373838).into(),
        }
    }
}

impl EdgeLight {
    /// Sample the decoded BGRA thumbnail once on admission to the cache.
    /// A fixed sample grid bounds the work for every aspect ratio.
    pub fn sample(image: &RenderImage) -> Self {
        let Some(pixels) = image.as_bytes(0) else {
            return Self::default();
        };
        let width = image.size(0).width.0 as usize;
        if width == 0 {
            return Self::default();
        }
        let height = pixels.len() / (4 * width);
        if height == 0 {
            return Self::default();
        }
        let half = |side: usize| {
            let mut sum = [0.; 3];
            let mut weights = 0.;
            for y in 0..6 {
                for x in 0..4 {
                    let sx = (side * 8 + x * 2 + 1) * width / 16;
                    let sy = (2 * y + 1) * height / 12;
                    let p = &pixels[4 * (sy * width + sx)..][..4];
                    let brightness = *p[..3].iter().max().unwrap() as f32 / 255.;
                    let weight = (0.15 + brightness) * f32::from(p[3]) / 255.;
                    weights += weight;
                    for (channel, total) in sum.iter_mut().enumerate() {
                        *total += f32::from(p[2 - channel]) * weight;
                    }
                }
            }
            let [r, g, b] = sum.map(|n| (n / weights.max(1.)) as u32);
            Hsla::from(rgb(r << 16 | g << 8 | b))
        };
        Self {
            left: half(0),
            right: half(1),
        }
    }

    pub fn wash(&self, opacity: f32) -> Background {
        linear_gradient(
            95.,
            linear_color_stop(self.left.opacity(opacity), 0.),
            linear_color_stop(self.right.opacity(opacity), 1.),
        )
    }

    pub fn halo(&self) -> Hsla {
        self.left.opacity(0.18)
    }
}

/// One grain image shared by the whole window. Geometry can change without
/// regenerating it, and a static surface schedules no frames.
pub fn grain() -> Arc<RenderImage> {
    static GRAIN: OnceLock<Arc<RenderImage>> = OnceLock::new();
    GRAIN
        .get_or_init(|| {
            let mut state = 0x8b5a_421du32;
            let pixels = RgbaImage::from_fn(768, 512, |_, _| {
                state ^= state << 13;
                state ^= state >> 17;
                state ^= state << 5;
                let v = ((state >> 28) as u8) / 3 + 18;
                Rgba([v, v, v, 255])
            });
            Arc::new(RenderImage::new(smallvec::smallvec![Frame::new(pixels)]))
        })
        .clone()
}

#[cfg(test)]
mod tests {
    use super::{EdgeLight, Frame, Hsla, RenderImage, Rgba, RgbaImage, rgb};

    #[test]
    fn glass_tints_follow_the_two_sides_of_bgra_artwork() {
        let pixels = RgbaImage::from_fn(2, 1, |x, _| {
            if x == 0 {
                Rgba([0, 0, 255, 255])
            } else {
                Rgba([255, 0, 0, 255])
            }
        });
        let image = RenderImage::new(smallvec::smallvec![Frame::new(pixels)]);
        let light = EdgeLight::sample(&image);
        assert_eq!(light.left, Hsla::from(rgb(0xff0000)));
        assert_eq!(light.right, Hsla::from(rgb(0x0000ff)));
    }
}
