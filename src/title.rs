// Tile titles are shaped once per item and painted from that cache. GPUI keeps
// a shaped line only while consecutive frames use it, so rows scrolling into
// view used to reshape their titles (font fallback and shaping-plan setup made
// that the main cause of dropped frames). Hovered overflowing titles scroll.
use gpui::*;
use std::{
    collections::HashMap,
    time::{Duration, Instant},
};

/// Tile title line height in logical pixels.
pub const LINE_HEIGHT: f32 = 18.;
/// Marquee speed in logical pixels per second.
const MARQUEE_SPEED: f32 = 40.;
/// Pause at each end of the marquee, which also delays its start.
const MARQUEE_HOLD: f32 = 0.9;
const FONT: &str = "sans-serif";

#[derive(Default)]
pub struct Titles {
    lines: HashMap<usize, ShapedLine>,
    ellipsis: Option<ShapedLine>,
    font_size: Pixels,
    /// Next item index the idle prewarm pass will shape.
    prewarmed: usize,
}
impl Titles {
    fn shape(text: SharedString, font_size: Pixels, color: Hsla, window: &Window) -> ShapedLine {
        let run = TextRun {
            len: text.len(),
            font: font(FONT),
            color,
            background_color: None,
            underline: None,
            strikethrough: None,
        };
        window
            .text_system()
            .shape_line(text, font_size, &[run], None)
    }
    /// Drop every cached line if the rem size (and so the font size) changed.
    fn sync_font_size(&mut self, window: &Window) {
        let font_size = rems(0.875).to_pixels(window.rem_size());
        if font_size != self.font_size {
            *self = Self {
                font_size,
                ..Self::default()
            };
        }
    }
    /// The shaped title of item `index`, shaping it on first use.
    pub fn line(&mut self, index: usize, title: &str, window: &Window) -> ShapedLine {
        self.sync_font_size(window);
        let font_size = self.font_size;
        self.lines
            .entry(index)
            .or_insert_with(|| {
                Self::shape(
                    SharedString::from(title.replace('\n', " ")),
                    font_size,
                    rgb(crate::FOREGROUND).into(),
                    window,
                )
            })
            .clone()
    }
    pub fn ellipsis(&mut self, window: &Window) -> ShapedLine {
        self.sync_font_size(window);
        let font_size = self.font_size;
        self.ellipsis
            .get_or_insert_with(|| {
                Self::shape("…".into(), font_size, rgb(crate::FOREGROUND).into(), window)
            })
            .clone()
    }
    /// Shape titles not yet cached, in item order, until `budget` is spent.
    /// Returns true once every title is cached.
    pub fn prewarm<'a>(
        &mut self,
        titles: impl IntoIterator<Item = &'a str>,
        window: &Window,
        budget: Duration,
    ) -> bool {
        let started = Instant::now();
        for (index, title) in titles.into_iter().enumerate().skip(self.prewarmed) {
            if started.elapsed() >= budget {
                return false;
            }
            self.line(index, title, window);
            self.prewarmed = index + 1;
        }
        true
    }
}

/// Horizontal offset of a title overflowing its tile by `overflow` pixels,
/// `elapsed` after the pointer entered: hold, scroll to the end, hold,
/// scroll back, repeat. During a hold, also the time until it ends, so the
/// caller can sleep instead of repainting an unchanged title. Titles that fit
/// never move.
pub fn marquee(overflow: f32, elapsed: Duration) -> (f32, Option<Duration>) {
    if overflow <= 0. {
        return (0., None);
    }
    let travel = overflow / MARQUEE_SPEED;
    let t = elapsed.as_secs_f32() % (2. * (MARQUEE_HOLD + travel));
    let hold = |end: f32| Some(Duration::from_secs_f32(end - t));
    let (offset, hold) = if t < MARQUEE_HOLD {
        (0., hold(MARQUEE_HOLD))
    } else if t < MARQUEE_HOLD + travel {
        ((t - MARQUEE_HOLD) * MARQUEE_SPEED, None)
    } else if t < 2. * MARQUEE_HOLD + travel {
        (overflow, hold(2. * MARQUEE_HOLD + travel))
    } else {
        (
            overflow - (t - 2. * MARQUEE_HOLD - travel) * MARQUEE_SPEED,
            None,
        )
    };
    (offset.clamp(0., overflow), hold)
}

/// Slot for the single pending repaint that ends a marquee pause; storing a
/// new task drops (cancels) the previous one.
pub type Wake = std::rc::Rc<std::cell::RefCell<Option<Task<()>>>>;
/// Title banner height: the line plus 3 logical pixels above and below.
pub const BANNER_HEIGHT: f32 = LINE_HEIGHT + 6.;
const INSET_X: f32 = 8.;
const INSET_BOTTOM: f32 = 3.;

/// A tile title, laid over the whole tile (inside its border) and painted in
/// the banner at its bottom: static and ellipsized, or scrolling while
/// `hovered_since` is set, the pointer is over the tile and the title does
/// not fit. GPUI reports hover changes only on pointer motion, so tiles that
/// scroll under a still pointer keep stale hover state; testing the pointer
/// against the tile at paint time keeps the marquee on the tile actually
/// under it. Only that title schedules further frames.
pub fn title(
    line: ShapedLine,
    ellipsis: ShapedLine,
    hovered_since: Option<Instant>,
    wake: Wake,
) -> Canvas<()> {
    canvas(
        |_, _, _| {},
        move |tile, _, window, cx| {
            let height = px(LINE_HEIGHT);
            let bounds = Bounds::new(
                point(
                    tile.left() + px(INSET_X),
                    tile.bottom() - px(INSET_BOTTOM + LINE_HEIGHT),
                ),
                size(tile.size.width - px(2. * INSET_X), height),
            );
            let overflow = f32::from(line.width - bounds.size.width);
            if overflow <= 0. {
                _ = line.paint(bounds.origin, height, window, cx);
                return;
            }
            let hovered_since = hovered_since
                .filter(|_| window.is_window_hovered() && tile.contains(&window.mouse_position()));
            window.with_content_mask(Some(ContentMask { bounds }), |window| {
                if let Some(since) = hovered_since {
                    let (offset, hold) = marquee(overflow, since.elapsed());
                    _ = line.paint(
                        bounds.origin - point(px(offset), px(0.)),
                        height,
                        window,
                        cx,
                    );
                    match hold {
                        Some(remaining) => {
                            *wake.borrow_mut() = Some(window.spawn(cx, async move |cx| {
                                cx.background_executor().timer(remaining).await;
                                _ = cx.update(|window, _| window.refresh());
                            }));
                        }
                        None => window.request_animation_frame(),
                    }
                    return;
                }
                let limit = bounds.size.width - ellipsis.width;
                let cut = line
                    .index_for_x(limit)
                    .map_or(limit, |index| line.x_for_index(index));
                let clip = Bounds::new(bounds.origin, size(cut, bounds.size.height));
                window.with_content_mask(Some(ContentMask { bounds: clip }), |window| {
                    _ = line.paint(bounds.origin, height, window, cx);
                });
                _ = ellipsis.paint(bounds.origin + point(cut, px(0.)), height, window, cx);
            });
        },
    )
}

#[cfg(test)]
mod tests {
    // Not `super::*`: GPUI's glob export includes a `test` macro.
    use super::{Duration, marquee};
    fn marquee_offset(overflow: f32, elapsed: Duration) -> f32 {
        marquee(overflow, elapsed).0
    }
    fn at(seconds: f32) -> Duration {
        Duration::from_secs_f32(seconds)
    }
    #[test]
    fn fitting_titles_never_move() {
        assert_eq!(marquee_offset(0., at(5.)), 0.);
        assert_eq!(marquee_offset(-30., at(5.)), 0.);
    }
    #[test]
    fn overflowing_titles_hold_scroll_hold_and_return() {
        let overflow = 80.; // 2 s of travel at 40 px/s.
        assert_eq!(marquee_offset(overflow, at(0.)), 0.);
        assert_eq!(marquee_offset(overflow, at(0.8)), 0.);
        assert!((marquee_offset(overflow, at(1.9)) - 40.).abs() < 0.01);
        assert_eq!(marquee_offset(overflow, at(3.0)), overflow);
        assert_eq!(marquee_offset(overflow, at(3.7)), overflow);
        assert!((marquee_offset(overflow, at(4.8)) - 40.).abs() < 0.01);
        // One full cycle is 2 × (hold + travel) = 5.8 s.
        assert_eq!(marquee_offset(overflow, at(5.8 + 0.5)), 0.);
        // Holds report when they end; scrolling phases do not.
        let hold = marquee(overflow, at(0.4)).1.unwrap();
        assert!((hold.as_secs_f32() - 0.5).abs() < 0.001);
        assert!(marquee(overflow, at(3.5)).1.is_some());
        assert!(marquee(overflow, at(1.9)).1.is_none());
        assert!(marquee(overflow, at(4.8)).1.is_none());
        for step in 0..600 {
            let offset = marquee_offset(overflow, at(step as f32 * 0.01));
            assert!((0. ..=overflow).contains(&offset));
        }
    }
}
