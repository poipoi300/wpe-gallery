// Drawn interface icons, embedded as SVG. GPUI rasterizes an SVG once into its
// sprite atlas; a painted path would instead route the whole frame through a
// window-sized multisampled intermediate texture, which halved the frame rate.
use gpui::*;
use std::borrow::Cow;

const CHEVRON: &str = "chevron.svg";
const CHEVRON_SVG: &[u8] = br#"<svg xmlns="http://www.w3.org/2000/svg" width="12" height="12" viewBox="0 0 12 12"><path d="M4.5 2.5 8 6l-3.5 3.5" fill="none" stroke="black" stroke-width="1.5" stroke-linecap="round" stroke-linejoin="round"/></svg>"#;

pub struct Assets;
impl AssetSource for Assets {
    fn load(&self, path: &str) -> anyhow::Result<Option<Cow<'static, [u8]>>> {
        Ok((path == CHEVRON).then_some(Cow::Borrowed(CHEVRON_SVG)))
    }
    fn list(&self, _: &str) -> anyhow::Result<Vec<SharedString>> {
        Ok(vec![CHEVRON.into()])
    }
}

/// A disclosure chevron, pointing right or (when open) down.
pub fn chevron(open: bool, color: Hsla) -> impl IntoElement {
    let angle = if open {
        std::f32::consts::FRAC_PI_2
    } else {
        0.
    };
    svg()
        .path(CHEVRON)
        .size(px(12.))
        .flex_shrink_0()
        .text_color(color)
        .with_transformation(Transformation::rotate(radians(angle)))
}
