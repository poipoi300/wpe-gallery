//! Bounded, spatially varying artwork reflections. All image buffers here use
//! the same BGRA channel order as the GPUI preview they are sampled from.
use gpui::RenderImage;
use image::{Frame, Rgba, RgbaImage, imageops};
use std::sync::{Arc, OnceLock};

const SIDE: u32 = 32;
const PAD: u32 = 4;
pub const SPILL_FRACTION: f32 = PAD as f32 / SIDE as f32;

fn sample(image: &RenderImage, u: f32, v: f32) -> Rgba<u8> {
    let size = image.size(0);
    let w = size.width.0 as usize;
    let h = size.height.0 as usize;
    let x = ((u * w as f32) as usize).min(w - 1);
    let y = ((v * h as f32) as usize).min(h - 1);
    let i = 4 * (y * w + x);
    let bytes = &image.as_bytes(0).expect("decoded first frame")[i..i + 4];
    // Transparent pixels contribute no reflected color.
    let alpha = u16::from(bytes[3]);
    let [b, g, r] = [0, 1, 2].map(|i| (u16::from(bytes[i]) * alpha / 255) as u8);
    Rgba([b, g, r, 255])
}

fn texture(pixels: RgbaImage) -> Arc<RenderImage> {
    Arc::new(RenderImage::new(smallvec::smallvec![Frame::new(pixels)]))
}

/// Reflect the bottom edge of the actual 16:9 cover crop into its caption.
pub fn caption(image: &RenderImage) -> Arc<RenderImage> {
    let size = image.size(0);
    let aspect = size.width.0 as f32 / size.height.0 as f32;
    let crop_w = (16. / 9. / aspect).min(1.);
    let crop_h = (aspect / (16. / 9.)).min(1.);
    let edge = RgbaImage::from_fn(SIDE, 5, |x, y| {
        let u = 0.5 + ((x as f32 + 0.5) / SIDE as f32 - 0.5) * crop_w;
        let v = 0.5 + (0.43 + 0.015 * y as f32) * crop_h;
        sample(image, u, v)
    });
    let soft = imageops::blur(&edge, 2.);
    texture(RgbaImage::from_fn(SIDE, 12, |x, y| {
        let mut color = *soft.get_pixel(x, 2);
        color[3] = (64. * (1. - y as f32 / 12.).powi(2)) as u8;
        color
    }))
}

/// Extend each edge of the selected image into the glass and fade it to zero
/// at the perimeter. The caller positions the inner square on the fitted art.
pub fn surround(image: &RenderImage) -> Arc<RenderImage> {
    let reduced = RgbaImage::from_fn(SIDE, SIDE, |x, y| {
        sample(
            image,
            (x as f32 + 0.5) / SIDE as f32,
            (y as f32 + 0.5) / SIDE as f32,
        )
    });
    let soft = imageops::blur(&reduced, 2.);
    texture(RgbaImage::from_fn(
        SIDE + 2 * PAD,
        SIDE + 2 * PAD,
        |x, y| {
            let sx = x.clamp(PAD, PAD + SIDE - 1);
            let sy = y.clamp(PAD, PAD + SIDE - 1);
            let distance = (x.abs_diff(sx) as f32).hypot(y.abs_diff(sy) as f32);
            let mut color = *soft.get_pixel(sx - PAD, sy - PAD);
            color[3] = (105. * (1. - distance / PAD as f32).max(0.).powi(2)) as u8;
            color
        },
    ))
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
    use super::*;

    #[test]
    fn reflections_follow_local_edges_and_fade_to_transparent() {
        let image = texture(RgbaImage::from_fn(80, 80, |x, y| match (x < 40, y < 40) {
            (true, true) => Rgba([255, 0, 0, 255]),
            (false, true) => Rgba([0, 0, 255, 255]),
            (true, false) => Rgba([0, 255, 0, 255]),
            (false, false) => Rgba([125, 0, 125, 255]),
        }));
        let spill = surround(&image);
        let pixel = |x: usize, y: usize| {
            let i = (y * (SIDE + 2 * PAD) as usize + x) * 4;
            &spill.as_bytes(0).unwrap()[i..i + 4]
        };
        assert_eq!(&pixel(10, 3)[..3], &[255, 0, 0]);
        assert_eq!(&pixel(30, 3)[..3], &[0, 0, 255]);
        assert_eq!(&pixel(10, 36)[..3], &[0, 255, 0]);
        assert!(pixel(10, 3)[3] > pixel(10, 2)[3]);
        assert_eq!(pixel(10, 0)[3], 0);
        let caption = caption(&image);
        let bytes = caption.as_bytes(0).unwrap();
        assert_eq!(&bytes[..3], &[0, 255, 0]);
        assert!(bytes[3] > bytes[bytes.len() - 1]);
    }
}
