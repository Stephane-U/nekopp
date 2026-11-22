//! The camera that looks into the room.
//!
//! A 3/4 view from just above the kitten's head, with a little parallax when the
//! pointer moves: enough that the room feels like a place rather than a picture,
//! and little enough that a room which pans under the cursor does not make anyone
//! seasick. The pointer also has to become a position on the floor, because that
//! is what the kitten chases, so the ray a pixel stands for is part of this
//! module rather than of the renderer.

use super::scene::{BACK, HALF_DEPTH, HALF_WIDTH};

/// The camera's lens: 40 degrees, which fits a room a few characters wide without
/// the distortion a wide lens would give the walls.
pub const FOV: f32 = 40.0 * std::f32::consts::PI / 180.0;

/// A camera looking into the room.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Camera {
    pub eye: [f32; 3],
    pub target: [f32; 3],
    /// Where the pointer is, in `-1..1` on both axes, for the parallax.
    pub pointer: [f32; 2],
}

impl Default for Camera {
    fn default() -> Self {
        Self::still()
    }
}

impl Camera {
    /// Looking straight down the room, no parallax.
    pub fn still() -> Self {
        Self {
            eye: [0.0, 3.4, 8.2],
            target: [0.0, 2.1, -6.0],
            pointer: [0.0, 0.0],
        }
    }

    /// The same camera with the pointer's parallax applied.
    ///
    /// The eye moves and the target stays, which is what gives the walls a little
    /// shift against each other. The offset is deliberately small: a few tenths
    /// of a character, not a walk.
    pub fn with_pointer(mut self, pointer: Option<[f32; 2]>) -> Self {
        if let Some(pointer) = pointer {
            self.pointer = [pointer[0].clamp(-1.0, 1.0), pointer[1].clamp(-1.0, 1.0)];
        }
        self.eye[0] += self.pointer[0] * 0.55;
        self.eye[1] -= self.pointer[1] * 0.28;
        self
    }

    /// Right, up and forward, each a unit vector.
    pub fn basis(&self) -> ([f32; 3], [f32; 3], [f32; 3]) {
        let forward = normalise(sub(self.target, self.eye));
        // The world's up, unless the camera were ever to look straight up.
        let right = normalise(cross(forward, [0.0, 1.0, 0.0]));
        let up = cross(right, forward);
        (right, up, forward)
    }

    /// The ray a pixel at `ndc` stands for.
    ///
    /// `ndc` is `-1..1` on both axes, y up, which is what the pointer position
    /// arrives as once it has been through the window's size.
    pub fn ray(&self, ndc: [f32; 2], aspect: f32) -> ([f32; 3], [f32; 3]) {
        let (right, up, forward) = self.basis();
        let half = (FOV * 0.5).tan();
        let direction = normalise(add(
            forward,
            add(
                scale(right, ndc[0] * half * aspect),
                scale(up, ndc[1] * half),
            ),
        ));
        (self.eye, direction)
    }

    /// Where the pointer meets the floor, if it meets it inside the room.
    pub fn floor(&self, ndc: Option<[f32; 2]>, aspect: f32) -> Option<[f32; 3]> {
        let (origin, direction) = self.ray(ndc?, aspect);
        if direction[1] >= -0.001 {
            return None;
        }
        let distance = -origin[1] / direction[1];
        let point = add(origin, scale(direction, distance));
        if point[0].abs() > HALF_WIDTH - 0.4 || point[2] < BACK + 0.3 || point[2] > 2.4 {
            return None;
        }
        Some([point[0], 0.0, point[2]])
    }

    /// How far into the room `point` is, in characters, for the fog.
    pub fn depth(&self, point: [f32; 3]) -> f32 {
        let (_, _, forward) = self.basis();
        dot(sub(point, self.eye), forward)
    }
}

fn add(a: [f32; 3], b: [f32; 3]) -> [f32; 3] {
    [a[0] + b[0], a[1] + b[1], a[2] + b[2]]
}

fn sub(a: [f32; 3], b: [f32; 3]) -> [f32; 3] {
    [a[0] - b[0], a[1] - b[1], a[2] - b[2]]
}

fn scale(a: [f32; 3], by: f32) -> [f32; 3] {
    [a[0] * by, a[1] * by, a[2] * by]
}

fn dot(a: [f32; 3], b: [f32; 3]) -> f32 {
    a[0] * b[0] + a[1] * b[1] + a[2] * b[2]
}

fn cross(a: [f32; 3], b: [f32; 3]) -> [f32; 3] {
    [
        a[1] * b[2] - a[2] * b[1],
        a[2] * b[0] - a[0] * b[2],
        a[0] * b[1] - a[1] * b[0],
    ]
}

fn normalise(a: [f32; 3]) -> [f32; 3] {
    let length = dot(a, a).sqrt().max(1e-6);
    scale(a, 1.0 / length)
}

/// The depth of the back wall from the camera, so the fog can be tuned to it.
pub fn far_depth() -> f32 {
    8.2 - BACK
}

/// The room's own extent, for a renderer that wants to draw a backdrop.
pub fn extent() -> ([f32; 3], [f32; 3]) {
    ([-HALF_WIDTH, 0.0, BACK], [HALF_WIDTH, 0.0, -HALF_DEPTH])
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_pointer_in_the_middle_looks_straight_ahead() {
        let camera = Camera::still();
        let (origin, direction) = camera.ray([0.0, 0.0], 1.6);
        assert_eq!(origin, camera.eye);
        let straight = normalise(sub(camera.target, camera.eye));
        for axis in 0..3 {
            assert!((direction[axis] - straight[axis]).abs() < 1e-3);
        }
    }

    #[test]
    fn a_pointer_at_the_bottom_of_the_window_meets_the_floor() {
        let camera = Camera::still();
        let point = camera.floor(Some([0.0, -0.9]), 1.6).expect("the floor");
        assert_eq!(point[1], 0.0);
        // Below the middle of the window is nearer the camera.
        assert!(point[2] > -6.0, "{point:?}");
        assert!(point[0].abs() < 1.0, "{point:?}");
    }

    #[test]
    fn a_pointer_at_the_top_of_the_window_hits_the_wall_and_not_the_floor() {
        let camera = Camera::still();
        assert!(camera.floor(Some([0.0, 0.95]), 1.6).is_none());
    }

    #[test]
    fn a_pointer_points_left_and_right_the_right_way_round() {
        let camera = Camera::still();
        let left = camera.floor(Some([-0.6, -0.9]), 1.6).expect("floor");
        let right = camera.floor(Some([0.6, -0.9]), 1.6).expect("floor");
        assert!(left[0] < right[0], "{left:?} {right:?}");
    }

    #[test]
    fn parallax_moves_the_eye_a_little_and_not_the_target() {
        let camera = Camera::still().with_pointer(Some([1.0, 0.0]));
        assert!(camera.eye[0] > Camera::still().eye[0]);
        assert_eq!(camera.target, Camera::still().target);
        // A little: less than a character, or the room would swim.
        assert!(camera.eye[0] - Camera::still().eye[0] < 1.0);

        let clamped = Camera::still().with_pointer(Some([9.0, 9.0]));
        let limit = Camera::still().with_pointer(Some([1.0, 1.0]));
        assert_eq!(clamped, limit);
    }

    #[test]
    fn the_back_wall_is_further_away_than_the_kitten() {
        let camera = Camera::still();
        let wall = camera.depth([0.0, 3.0, BACK]);
        let kitten = camera.depth([0.0, 1.0, -2.0]);
        assert!(wall > kitten, "{wall} {kitten}");
        assert!((wall - far_depth()).abs() < 1.0);
    }

    #[test]
    fn no_pointer_is_no_floor_point() {
        assert!(Camera::still().floor(None, 1.6).is_none());
    }
}
