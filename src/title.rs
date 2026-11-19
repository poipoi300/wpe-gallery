use gpui::{prelude::*, *};
use gpui_component::{label::Label, v_flex};
use std::time::{Duration, Instant};

const FONT_SIZE: f32 = 14.;
const MARQUEE_SPEED: f32 = 40.;
const MARQUEE_HOLD: f32 = 0.9;

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
/// Only the hovered title needs its untruncated width. Move the Kit label
/// inside a clipped viewport, sleeping through the holds at either end.
pub fn caption(
    text: &str,
    width: f32,
    since: Option<Instant>,
    wake: &Wake,
    window: &mut Window,
    cx: &mut App,
) -> Div {
    let text: SharedString = text.replace('\n', " ").into();
    let label = Label::new(text.clone())
        .text_size(px(FONT_SIZE))
        .line_height(px(20.));
    let mut viewport = v_flex()
        .relative()
        .min_w_0()
        .flex_1()
        .h(px(20.))
        .overflow_hidden();
    if let Some(since) = since {
        let run = TextRun {
            len: text.len(),
            font: font("Inter"),
            color: white(),
            background_color: None,
            underline: None,
            strikethrough: None,
        };
        let line = window
            .text_system()
            .shape_line(text, px(FONT_SIZE), &[run], None);
        let overflow = f32::from(line.width) - width;
        if overflow > 0. {
            let (offset, hold) = marquee(overflow, since.elapsed());
            if let Some(remaining) = hold {
                *wake.borrow_mut() = Some(window.spawn(cx, async move |cx| {
                    cx.background_executor().timer(remaining).await;
                    _ = cx.update(|window, _| window.refresh());
                }));
            } else {
                window.request_animation_frame();
            }
            viewport = viewport.child(label.absolute().left(px(-offset)).whitespace_nowrap());
            return viewport;
        }
    }
    viewport.child(label.w(px(width)).truncate())
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
