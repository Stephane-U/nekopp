//! Where the page sits in the window.
//!
//! The page is flat and the camera is orthographic, which is the whole difference
//! between this mode and a diorama: one cell is one line of text wherever it is on
//! the screen, so a glyph lands on whole pixels and the document reads like the
//! editor's own text. The pointer mapping follows from that -- a pixel is a cell,
//! so turning a position into a place on the page is a multiplication and not a
//! ray cast.

use super::scene::GLYPH_WIDTH;

/// The window, as a grid of character cells.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Camera {
    /// Cells across.
    pub columns: u32,
    /// Cells down.
    pub rows: u32,
}

impl Default for Camera {
    fn default() -> Self {
        Self::new(120, 40)
    }
}

impl Camera {
    /// A page of `columns` by `rows` cells.
    pub fn new(columns: u32, rows: u32) -> Self {
        Self {
            columns: columns.max(24),
            rows: rows.max(8),
        }
    }

    /// How wide the page is, in line heights.
    pub fn width(&self) -> f32 {
        self.columns as f32 * GLYPH_WIDTH
    }

    /// How tall the page is, in line heights.
    pub fn height(&self) -> f32 {
        self.rows as f32
    }

    /// Where a pointer at `ndc` -- `-1..1` on both axes, y up -- lands on the page.
    ///
    /// Outside the window there is no place on the page, which is what stops the
    /// kitten from chasing a mouse that has left.
    pub fn page(&self, ndc: Option<[f32; 2]>) -> Option<[f32; 3]> {
        let ndc = ndc?;
        if !(-1.0..=1.0).contains(&ndc[0]) || !(-1.0..=1.0).contains(&ndc[1]) {
            return None;
        }
        Some([
            (ndc[0] * 0.5 + 0.5) * self.width(),
            (ndc[1] * 0.5 + 0.5) * self.height(),
            0.0,
        ])
    }

    /// Where a point on the page lands in `ndc`, which is what a test uses to
    /// check the mapping in both directions.
    pub fn ndc(&self, point: [f32; 3]) -> [f32; 2] {
        [
            point[0] / self.width() * 2.0 - 1.0,
            point[1] / self.height() * 2.0 - 1.0,
        ]
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_middle_of_the_window_is_the_middle_of_the_page() {
        let camera = Camera::new(120, 40);
        let page = camera.page(Some([0.0, 0.0])).expect("the middle");
        assert!((page[0] - camera.width() * 0.5).abs() < 1e-4);
        assert!((page[1] - 20.0).abs() < 1e-4);
        assert_eq!(page[2], 0.0);
    }

    #[test]
    fn the_corners_of_the_window_are_the_corners_of_the_page() {
        let camera = Camera::new(80, 24);
        let bottom_left = camera.page(Some([-1.0, -1.0])).expect("a corner");
        assert_eq!(bottom_left, [0.0, 0.0, 0.0]);
        let top_right = camera.page(Some([1.0, 1.0])).expect("a corner");
        assert!((top_right[0] - camera.width()).abs() < 1e-4);
        assert!((top_right[1] - 24.0).abs() < 1e-4);
    }

    #[test]
    fn a_pixel_maps_to_a_place_and_back_again() {
        let camera = Camera::new(100, 30);
        for ndc in [[0.1, -0.4], [-0.7, 0.6], [0.0, 0.0]] {
            let page = camera.page(Some(ndc)).expect("on the page");
            let back = camera.ndc(page);
            assert!((back[0] - ndc[0]).abs() < 1e-4, "{back:?} {ndc:?}");
            assert!((back[1] - ndc[1]).abs() < 1e-4, "{back:?} {ndc:?}");
        }
    }

    #[test]
    fn a_pointer_outside_the_window_is_no_place_on_the_page() {
        let camera = Camera::new(120, 40);
        assert!(camera.page(None).is_none());
        assert!(camera.page(Some([1.4, 0.0])).is_none());
        assert!(camera.page(Some([0.0, -1.2])).is_none());
    }

    #[test]
    fn right_on_the_page_is_right_on_the_screen() {
        let camera = Camera::new(120, 40);
        let left = camera.page(Some([-0.6, 0.0])).expect("left");
        let right = camera.page(Some([0.6, 0.0])).expect("right");
        assert!(left[0] < right[0], "{left:?} {right:?}");
        // y is up on the page, so the top of the window is a higher row.
        let top = camera.page(Some([0.0, 0.8])).expect("top");
        let bottom = camera.page(Some([0.0, -0.8])).expect("bottom");
        assert!(top[1] > bottom[1], "{top:?} {bottom:?}");
    }

    #[test]
    fn a_tiny_grid_still_holds_a_page() {
        let camera = Camera::new(0, 0);
        assert!(camera.columns >= 24);
        assert!(camera.rows >= 8);
        assert!(camera.width() > 0.0);
    }
}
