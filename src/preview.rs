// A few thumbnail workers and one selection worker; no filesystem watchers.
use futures::channel::oneshot;
use image::{AnimationDecoder, Delay, Frame, ImageDecoder};
use std::{
    path::PathBuf,
    sync::{Arc, Mutex, mpsc},
    time::Duration,
};
/// Decoded frames, each its own single-frame image with its display time.
/// The gallery advances animated previews on a timer at the GIF's own rate;
/// a multi-frame `RenderImage` would make GPUI redraw every display refresh.
pub type Frames = Vec<PreviewFrame>;
#[derive(Clone)]
pub struct PreviewFrame {
    pub image: Arc<gpui::RenderImage>,
    pub reflection: Arc<gpui::RenderImage>,
    pub delay: Duration,
}

impl PreviewFrame {
    pub fn release(self, cx: &mut gpui::App) {
        cx.drop_image(self.image, None);
        cx.drop_image(self.reflection, None);
    }
}
type Reply = oneshot::Sender<Result<Frames, String>>;
struct Job {
    path: PathBuf,
    animate: bool,
    reply: Reply,
}
#[derive(Clone)]
pub struct Pool {
    thumbnails: mpsc::SyncSender<Job>,
    selected: mpsc::Sender<Job>,
}
impl Pool {
    pub fn new() -> Self {
        let (thumbnails, rx) = mpsc::sync_channel::<Job>(64);
        let (selected, selected_rx) = mpsc::channel::<Job>();
        // Thumbnail decodes (mostly large JPEGs) are the scroll-in bottleneck;
        // a few workers keep up without taking over the machine.
        let workers = std::thread::available_parallelism().map_or(1, |n| (n.get() / 4).clamp(1, 3));
        let rx = Arc::new(Mutex::new(rx));
        for _ in 0..workers {
            let rx = rx.clone();
            // The lock is held only while waiting, so jobs go out one at a time.
            std::thread::spawn(move || serve(|| rx.lock().unwrap().recv()));
        }
        std::thread::spawn(move || serve(|| selected_rx.recv()));
        Self {
            thumbnails,
            selected,
        }
    }
    pub fn request(
        &self,
        path: PathBuf,
        animate: bool,
    ) -> Option<oneshot::Receiver<Result<Frames, String>>> {
        let (reply, receiver) = oneshot::channel();
        let job = Job {
            path,
            animate,
            reply,
        };
        if animate {
            self.selected.send(job).ok()?;
        } else {
            self.thumbnails.try_send(job).ok()?;
        }
        Some(receiver)
    }
}
fn serve(mut next: impl FnMut() -> Result<Job, mpsc::RecvError>) {
    while let Ok(job) = next() {
        if job.reply.is_canceled() {
            continue;
        }
        let result = decode(&job.path, job.animate, &job.reply).map_err(|e| e.to_string());
        let _ = job.reply.send(result);
    }
}
fn decode(path: &PathBuf, animate: bool, reply: &Reply) -> anyhow::Result<Frames> {
    anyhow::ensure!(
        std::fs::metadata(path)?.len() <= 32 * 1024 * 1024,
        "Preview exceeds 32 MiB"
    );
    let mut frames = Vec::<Frame>::new();
    let max = if animate { 48 } else { 1 };
    use std::io::{Read, Seek};
    let mut file = std::io::BufReader::new(std::fs::File::open(path)?);
    let mut magic = [0u8; 4];
    let is_gif = file.read_exact(&mut magic).is_ok() && &magic == b"GIF8";
    file.rewind()?;
    if is_gif {
        let mut decoder = image::codecs::gif::GifDecoder::new(file)?;
        anyhow::ensure!(
            u64::from(decoder.dimensions().0) * u64::from(decoder.dimensions().1) <= 16_000_000,
            "Preview dimensions exceed limit"
        );
        decoder.set_limits(image::Limits::default())?;
        for frame in decoder.into_frames().take(max) {
            anyhow::ensure!(!reply.is_canceled(), "Preview superseded");
            let frame = frame?;
            let (n, d) = frame.delay().numer_denom_ms();
            let delay = Delay::from_numer_denom_ms((n / d.max(1)).max(66), 1);
            frames.push(convert(frame.into_buffer(), delay, animate));
        }
    } else {
        // Workshop previews are often mislabelled; trust content, not extension.
        let mut reader = image::ImageReader::new(file).with_guessed_format()?;
        reader.limits(image::Limits::default());
        frames.push(convert(
            reader.decode()?.into_rgba8(),
            Delay::from_numer_denom_ms(100, 1),
            animate,
        ));
    }
    anyhow::ensure!(!frames.is_empty(), "Empty preview");
    frames
        .into_iter()
        .map(|frame| {
            anyhow::ensure!(!reply.is_canceled(), "Preview superseded");
            let delay = Duration::from(frame.delay());
            let image = Arc::new(gpui::RenderImage::new(smallvec::smallvec![frame]));
            let reflection = if animate {
                crate::surface::surround(&image)
            } else {
                crate::surface::caption(&image)
            };
            Ok(PreviewFrame {
                image,
                reflection,
                delay,
            })
        })
        .collect()
}
fn convert(buffer: image::RgbaImage, delay: Delay, large: bool) -> Frame {
    let image = image::DynamicImage::ImageRgba8(buffer)
        .thumbnail(if large { 480 } else { 256 }, if large { 270 } else { 144 });
    let mut buffer = image.into_rgba8();
    for pixel in buffer.as_chunks_mut::<4>().0 {
        pixel.swap(0, 2);
    }
    Frame::from_parts(buffer, 0, 0, delay)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn gif_reflection_matches_each_displayed_frame() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("color-change.gif");
        let mut encoder =
            image::codecs::gif::GifEncoder::new(std::fs::File::create(&path).unwrap());
        for color in [[255, 0, 0, 255], [0, 0, 255, 255]] {
            encoder
                .encode_frame(Frame::new(image::RgbaImage::from_pixel(
                    4,
                    4,
                    image::Rgba(color),
                )))
                .unwrap();
        }
        drop(encoder);
        let (reply, _receiver) = oneshot::channel();
        let frames = decode(&path, true, &reply).unwrap();
        assert_eq!(frames.len(), 2);
        for frame in frames {
            let w = frame.reflection.size(0).width.0 as usize;
            let offset = 4 * (w * 3 + w / 2);
            let edge = &frame.reflection.as_bytes(0).unwrap()[offset..offset + 4];
            let art = &frame.image.as_bytes(0).unwrap()[..3];
            assert!(edge[3] > 0);
            // Gaussian resampling can round a channel by one level.
            assert!(edge[..3].iter().zip(art).all(|(a, b)| a.abs_diff(*b) <= 1));
        }
    }
}
