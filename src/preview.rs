// Exactly two workers. No timers, filesystem watchers, or background animation.
use futures::channel::oneshot;
use image::{AnimationDecoder, Delay, Frame, ImageDecoder};
use std::{
    path::PathBuf,
    sync::{Arc, mpsc},
};
type Reply = oneshot::Sender<Result<Arc<gpui::RenderImage>, String>>;
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
        for receiver in [rx, selected_rx] {
            std::thread::spawn(move || {
                while let Ok(job) = receiver.recv() {
                    if job.reply.is_canceled() {
                        continue;
                    }
                    let result =
                        decode(&job.path, job.animate, &job.reply).map_err(|e| e.to_string());
                    let _ = job.reply.send(result);
                }
            });
        }
        Self {
            thumbnails,
            selected,
        }
    }
    pub fn request(
        &self,
        path: PathBuf,
        animate: bool,
    ) -> Option<oneshot::Receiver<Result<Arc<gpui::RenderImage>, String>>> {
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
fn decode(path: &PathBuf, animate: bool, reply: &Reply) -> anyhow::Result<Arc<gpui::RenderImage>> {
    anyhow::ensure!(
        std::fs::metadata(path)?.len() <= 32 * 1024 * 1024,
        "Preview exceeds 32 MiB"
    );
    let mut frames = smallvec::SmallVec::<[Frame; 1]>::new();
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
    Ok(Arc::new(gpui::RenderImage::new(frames)))
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
