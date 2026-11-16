// Geometry is recomputed from the configured client viewport each render.
pub const ROW_HEIGHT: f32 = 164.;
pub struct GridLayout {
    pub columns: usize,
    pub tile_width: f32,
    pub cache_limit: usize,
}
impl GridLayout {
    pub fn new(width: f32, height: f32) -> Self {
        let columns = (((width - 388.) / 190.).floor() as usize).clamp(1, 12);
        let tile_width = ((width - 372. - 12. * (columns - 1) as f32) / columns as f32).max(80.);
        // About four viewports of thumbnails (256×144 RGBA, ~144 KiB each):
        // scrolling back and forth reuses decodes instead of evicting them.
        let viewport = columns * ((height / ROW_HEIGHT).ceil() as usize + 2);
        let cache_limit = (viewport * 4).max(256);
        Self {
            columns,
            tile_width,
            cache_limit,
        }
    }
}
/// A keyboard move through the grid.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Step {
    Previous,
    Next,
    Up,
    Down,
}
/// The position (in the visible list) reached from `position` by `step` in a
/// grid of `len` items laid out in `columns` columns.
///
/// Left/Right run through the list and wrap across row ends; Up/Down keep the
/// column. Down from the row above a shorter last row lands on the last item;
/// moves past either end stay put.
pub fn step(position: usize, len: usize, columns: usize, step: Step) -> usize {
    if len == 0 {
        return 0;
    }
    let columns = columns.max(1);
    match step {
        Step::Previous => position.saturating_sub(1),
        Step::Next => (position + 1).min(len - 1),
        Step::Up => position.checked_sub(columns).unwrap_or(position),
        Step::Down if position + columns < len => position + columns,
        Step::Down if position / columns < (len - 1) / columns => len - 1,
        Step::Down => position,
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn first_configure_and_tiled_resize_recompute_finite_geometry() {
        let initial = GridLayout::new(0., 0.);
        assert_eq!(initial.columns, 1);
        let tiled = GridLayout::new(1331., 1107.);
        assert_eq!(tiled.columns, 4);
        assert!(tiled.tile_width.is_finite() && tiled.tile_width > 80.);
        assert!(
            tiled.columns as f32 * tiled.tile_width + 12. * (tiled.columns - 1) as f32
                <= 1331. - 372. + 0.01
        );
        let resized = GridLayout::new(1336., 1107.);
        assert_eq!(tiled.columns, resized.columns);
        assert_ne!(tiled.tile_width, resized.tile_width);
        let fullscreen = GridLayout::new(4096., 1152.);
        assert_eq!(fullscreen.columns, 12);
        // Four viewports of 12 columns × (8 visible + 2) rows.
        assert_eq!(fullscreen.cache_limit, 480);
    }
    #[test]
    fn horizontal_steps_wrap_rows_and_clamp_at_ends() {
        assert_eq!(step(3, 10, 4, Step::Next), 4);
        assert_eq!(step(4, 10, 4, Step::Previous), 3);
        assert_eq!(step(0, 10, 4, Step::Previous), 0);
        assert_eq!(step(9, 10, 4, Step::Next), 9);
    }
    #[test]
    fn vertical_steps_keep_the_column() {
        // 10 items, 4 columns: rows [0..4), [4..8), [8..10).
        assert_eq!(step(1, 10, 4, Step::Down), 5);
        assert_eq!(step(5, 10, 4, Step::Up), 1);
        assert_eq!(step(5, 10, 4, Step::Down), 9);
        assert_eq!(step(8, 10, 4, Step::Up), 4);
    }
    #[test]
    fn vertical_steps_clamp_sensibly_at_the_ends() {
        // Top row stays put going up.
        assert_eq!(step(2, 10, 4, Step::Up), 2);
        // Above an empty cell of the short last row: land on the last item.
        assert_eq!(step(7, 10, 4, Step::Down), 9);
        // Already on the last row: stay.
        assert_eq!(step(9, 10, 4, Step::Down), 9);
        assert_eq!(step(8, 10, 4, Step::Down), 8);
        // Single row and degenerate inputs.
        assert_eq!(step(1, 3, 4, Step::Down), 1);
        assert_eq!(step(0, 0, 4, Step::Down), 0);
        assert_eq!(step(5, 6, 0, Step::Up), 4);
    }
}
