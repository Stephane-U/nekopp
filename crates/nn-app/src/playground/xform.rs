//! The matrices the playground needs.
//!
//! Four by four, column-major, which is what WGSL's `mat4x4<f32>` is and what
//! `bytemuck` will hand to the shader without a transpose. There is no maths
//! library in this crate and no `glam` in the lock file, so the dozen operations
//! a room needs are here, each one tested against a point whose answer can be
//! written down.
//!
//! Everything is right-handed with `-Z` forward, and the projection maps depth
//! into `0..1` rather than `-1..1`, because that is the range wgpu's depth buffer
//! and its `Less` comparison expect. Getting that wrong does not fail loudly: it
//! draws the far wall in front of the kitten.

/// A column-major four by four matrix.
pub type Mat4 = [[f32; 4]; 4];

/// The identity.
pub fn identity() -> Mat4 {
    [
        [1.0, 0.0, 0.0, 0.0],
        [0.0, 1.0, 0.0, 0.0],
        [0.0, 0.0, 1.0, 0.0],
        [0.0, 0.0, 0.0, 1.0],
    ]
}

/// `a * b`: `b` is applied first.
pub fn mul(a: Mat4, b: Mat4) -> Mat4 {
    let mut out = [[0.0; 4]; 4];
    for (column, column_out) in out.iter_mut().enumerate() {
        for (row, value) in column_out.iter_mut().enumerate() {
            let mut sum = 0.0;
            for k in 0..4 {
                sum += a[k][row] * b[column][k];
            }
            *value = sum;
        }
    }
    out
}

/// A translation.
pub fn translation(at: [f32; 3]) -> Mat4 {
    let mut out = identity();
    out[3] = [at[0], at[1], at[2], 1.0];
    out
}

/// A scale, per axis.
pub fn scale(by: [f32; 3]) -> Mat4 {
    let mut out = identity();
    out[0][0] = by[0];
    out[1][1] = by[1];
    out[2][2] = by[2];
    out
}

/// A turn about the vertical axis, in radians.
pub fn rotation_y(angle: f32) -> Mat4 {
    let (sin, cos) = angle.sin_cos();
    let mut out = identity();
    out[0][0] = cos;
    out[0][2] = -sin;
    out[2][0] = sin;
    out[2][2] = cos;
    out
}

/// A tip about the horizontal axis, in radians.
pub fn rotation_x(angle: f32) -> Mat4 {
    let (sin, cos) = angle.sin_cos();
    let mut out = identity();
    out[1][1] = cos;
    out[1][2] = sin;
    out[2][1] = -sin;
    out[2][2] = cos;
    out
}

/// The model matrix of one card: a quad facing `+Z`, sized, turned and placed.
///
/// The quad is the unit square in the `XY` plane, so a card one line tall and
/// [`nn_core::playground::scene::GLYPH_WIDTH`] wide only has to be scaled. The
/// tip is applied before the turn, which is what makes a card lying on the floor
/// still spin about the world's vertical axis.
pub fn card(pos: [f32; 3], yaw: f32, pitch: f32, width: f32, height: f32) -> Mat4 {
    let placed = translation(pos);
    let turned = rotation_y(yaw);
    let tipped = rotation_x(pitch);
    let sized = scale([width, height, 1.0]);
    mul(mul(placed, turned), mul(tipped, sized))
}

/// The model matrix of one box, centred on `pos`.
pub fn block(pos: [f32; 3], size: [f32; 3], yaw: f32) -> Mat4 {
    mul(mul(translation(pos), rotation_y(yaw)), scale(size))
}

/// A perspective projection whose depth runs `0..1`.
pub fn perspective(fov_y: f32, aspect: f32, near: f32, far: f32) -> Mat4 {
    let f = 1.0 / (fov_y * 0.5).tan();
    let mut out = [[0.0; 4]; 4];
    out[0][0] = f / aspect.max(1e-3);
    out[1][1] = f;
    out[2][2] = far / (near - far);
    out[2][3] = -1.0;
    out[3][2] = near * far / (near - far);
    out
}

/// A view matrix looking from `eye` at `target`.
pub fn look_at(eye: [f32; 3], target: [f32; 3], up: [f32; 3]) -> Mat4 {
    let forward = normalise(sub(target, eye));
    let right = normalise(cross(forward, up));
    let up = cross(right, forward);
    let mut out = identity();
    out[0] = [right[0], up[0], -forward[0], 0.0];
    out[1] = [right[1], up[1], -forward[1], 0.0];
    out[2] = [right[2], up[2], -forward[2], 0.0];
    out[3] = [-dot(right, eye), -dot(up, eye), dot(forward, eye), 1.0];
    out
}

/// `m * [x, y, z, 1]`, as `x`, `y`, `z` and `w` out of the four components.
///
/// The renderer hands matrices to the GPU and never projects a point itself, so
/// this exists for the tests that pin down what those matrices mean.
#[cfg(test)]
pub fn apply(m: Mat4, point: [f32; 3]) -> [f32; 4] {
    let mut out = [0.0; 4];
    for (row, value) in out.iter_mut().enumerate() {
        *value = m[0][row] * point[0]
            + m[1][row] * point[1]
            + m[2][row] * point[2]
            + m[3][row];
    }
    out
}

fn sub(a: [f32; 3], b: [f32; 3]) -> [f32; 3] {
    [a[0] - b[0], a[1] - b[1], a[2] - b[2]]
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
    [a[0] / length, a[1] / length, a[2] / length]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_identity_leaves_a_point_alone() {
        let p = apply(identity(), [1.0, 2.0, 3.0]);
        assert_eq!(p, [1.0, 2.0, 3.0, 1.0]);
    }

    #[test]
    fn a_translation_after_a_scale_keeps_the_translation() {
        let m = mul(translation([5.0, 0.0, 0.0]), scale([2.0, 2.0, 2.0]));
        assert_eq!(apply(m, [1.0, 0.0, 0.0]), [7.0, 0.0, 0.0, 1.0]);
    }

    #[test]
    fn a_turn_about_y_takes_positive_z_to_positive_x() {
        let m = rotation_y(std::f32::consts::FRAC_PI_2);
        let p = apply(m, [0.0, 0.0, 1.0]);
        assert!((p[0] - 1.0).abs() < 1e-5, "{p:?}");
        assert!(p[2].abs() < 1e-5, "{p:?}");
    }

    #[test]
    fn a_tip_of_minus_a_quarter_turn_lays_a_card_flat_with_its_face_up() {
        // The card's normal is +Z; tipped by -90 degrees it must point at the
        // ceiling, which is what every floor card and every shadow relies on.
        let m = card([0.0, 0.01, 0.0], 0.0, -std::f32::consts::FRAC_PI_2, 0.55, 1.0);
        let normal = apply(m, [0.0, 0.0, 1.0]);
        let origin = apply(m, [0.0, 0.0, 0.0]);
        let n = [normal[0] - origin[0], normal[1] - origin[1], normal[2] - origin[2]];
        assert!(n[1] > 0.9, "{n:?}");
        assert!(n[2].abs() < 1e-5, "{n:?}");
    }

    #[test]
    fn a_card_is_as_wide_and_as_tall_as_it_was_asked_to_be() {
        // The quad is the unit square centred on its own origin, so the corner
        // is half of the size away from the card's centre on each axis.
        let m = card([1.0, 2.0, 3.0], 0.0, 0.0, 0.55, 1.0);
        assert_eq!(apply(m, [0.0, 0.0, 0.0]), [1.0, 2.0, 3.0, 1.0]);
        let corner = apply(m, [0.5, 0.5, 0.0]);
        assert!((corner[0] - 1.275).abs() < 1e-5, "{corner:?}");
        assert!((corner[1] - 2.5).abs() < 1e-5, "{corner:?}");
    }

    #[test]
    fn a_box_is_centred_and_sized() {
        let m = block([0.0, 1.0, 0.0], [2.0, 0.5, 4.0], 0.0);
        let corner = apply(m, [1.0, 1.0, 1.0]);
        assert!((corner[0] - 2.0).abs() < 1e-5, "{corner:?}");
        assert!((corner[1] - 1.5).abs() < 1e-5, "{corner:?}");
        assert!((corner[2] - 4.0).abs() < 1e-5, "{corner:?}");
    }

    #[test]
    fn the_point_a_camera_looks_at_lands_in_the_middle_of_the_screen() {
        let view = look_at([0.0, 0.0, 5.0], [0.0, 0.0, 0.0], [0.0, 1.0, 0.0]);
        let proj = perspective(1.0, 1.6, 0.1, 100.0);
        let p = apply(mul(proj, view), [0.0, 0.0, 0.0]);
        assert!(p[3] > 0.0, "behind the camera: {p:?}");
        assert!(p[0].abs() < 1e-5 && p[1].abs() < 1e-5, "{p:?}");
        // Depth in `0..1`, nearer to zero for a nearer point.
        let depth = p[2] / p[3];
        assert!(depth > 0.0 && depth < 1.0, "{depth}");
    }

    #[test]
    fn a_nearer_point_has_a_smaller_projected_depth() {
        let view = look_at([0.0, 0.0, 5.0], [0.0, 0.0, 0.0], [0.0, 1.0, 0.0]);
        let proj = perspective(1.0, 1.6, 0.1, 100.0);
        let near = apply(mul(proj, view), [0.0, 0.0, 2.0]);
        let far = apply(mul(proj, view), [0.0, 0.0, -8.0]);
        assert!(
            near[2] / near[3] < far[2] / far[3],
            "{} {}",
            near[2] / near[3],
            far[2] / far[3]
        );
    }

    #[test]
    fn a_point_behind_the_camera_is_behind_it() {
        let view = look_at([0.0, 0.0, 5.0], [0.0, 0.0, 0.0], [0.0, 1.0, 0.0]);
        let p = apply(view, [0.0, 0.0, 20.0]);
        assert!(p[2] > 0.0, "the view turns forward into +Z, so {p:?}");
        let proj = perspective(1.0, 1.6, 0.1, 100.0);
        let clip = apply(proj, [p[0], p[1], p[2]]);
        assert!(clip[3] < 0.0, "{clip:?}");
    }
}
