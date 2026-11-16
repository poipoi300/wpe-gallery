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
        let cache_limit = (columns * ((height / ROW_HEIGHT).ceil() as usize + 3) + 64).max(128);
        Self {
            columns,
            tile_width,
            cache_limit,
        }
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
        assert!(fullscreen.cache_limit >= 12 * 8);
    }
}
