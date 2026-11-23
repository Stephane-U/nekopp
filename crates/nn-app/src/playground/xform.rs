// Neko++ -- a GPU-rendered text editor.
// Copyright (C) 2026 Stéphane Urdy
//
// This program is free software: you can redistribute it and/or modify it
// under the terms of the GNU General Public License as published by the Free
// Software Foundation, version 3 of the License.
//
// This program is distributed in the hope that it will be useful, but WITHOUT
// ANY WARRANTY; without even the implied warranty of MERCHANTABILITY or
// FITNESS FOR A PARTICULAR PURPOSE. See the GNU General Public License for
// more details.
//
// You should have received a copy of the GNU General Public License along with
// this program. If not, see <https://www.gnu.org/licenses/>.

//! The matrices the playground needs.
//!
//! Four by four, column-major, which is what WGSL's `mat4x4<f32>` is and what
//! `bytemuck` will hand to the shader without a transpose. There is no maths
//! library in this crate and no `glam` in the lock file, so the dozen operations
//! a page needs are here, each one tested against a point whose answer can be
//! written down.
//!
//! Two projections matter. The page is drawn with an orthographic camera, because
//! a page of text seen at a perspective would not be a page: the glyphs at the
//! edges would be a different size from the glyphs in the middle. The kitten is
//! drawn through the same camera and then sheared by [`oblique`], which is what
//! gives a box on a flat page a side and a top without tilting the text.
//!
//! Everything is right-handed with `-Z` forward, and the projection maps depth
//! into `0..1` rather than `-1..1`, because that is the range wgpu's depth buffer
//! and its `Less` comparison expect. Getting that wrong does not fail loudly: it
//! draws the far side of the cat in front of the near side.

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

/// A turn in the page's own plane, in radians: what a scattered letter does.
pub fn rotation_z(angle: f32) -> Mat4 {
    let (sin, cos) = angle.sin_cos();
    let mut out = identity();
    out[0][0] = cos;
    out[0][1] = sin;
    out[1][0] = -sin;
    out[1][1] = cos;
    out
}

/// The model matrix of one card: a quad facing `+Z`, sized, tipped, turned and
/// placed.
///
/// The quad is the unit square in the `XY` plane, so a card one line tall and
/// [`nn_core::playground::scene::GLYPH_WIDTH`] wide only has to be scaled. `roll`
/// is applied first, in the card's own plane, and `pitch` after it, so a letter
/// that has been knocked sideways still tips about the page's horizontal.
pub fn card(pos: [f32; 3], yaw: f32, pitch: f32, roll: f32, width: f32, height: f32) -> Mat4 {
    let placed = translation(pos);
    let turned = rotation_y(yaw);
    let tipped = rotation_x(pitch);
    let rolled = rotation_z(roll);
    let sized = scale([width, height, 1.0]);
    mul(
        mul(placed, turned),
        mul(tipped, mul(rolled, sized)),
    )
}

/// The model matrix of one box, centred on `pos`.
pub fn block(pos: [f32; 3], size: [f32; 3], yaw: f32) -> Mat4 {
    mul(mul(translation(pos), rotation_y(yaw)), scale(size))
}

/// A shear that turns a flat picture into a shallow box, in world space.
///
/// `out.x = x + kx * z`, `out.y = y + ky * z`. With `kx` positive and `ky`
/// negative the reader sees the left side and the top of whatever is drawn through
/// it, which is enough to make a box look like a box on a page that is otherwise
/// flat. Applied to the page's own cards it would tilt the text, so it is applied
/// to the kitten and nothing else.
pub fn oblique(kx: f32, ky: f32) -> Mat4 {
    let mut out = identity();
    out[2][0] = kx;
    out[2][1] = ky;
    out
}

/// An orthographic projection whose depth runs `0..1`.
pub fn ortho(left: f32, right: f32, bottom: f32, top: f32, near: f32, far: f32) -> Mat4 {
    let mut out = identity();
    out[0][0] = 2.0 / (right - left);
    out[1][1] = 2.0 / (top - bottom);
    out[2][2] = -1.0 / (far - near);
    out[3][0] = -(right + left) / (right - left);
    out[3][1] = -(top + bottom) / (top - bottom);
    out[3][2] = -near / (far - near);
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
    fn a_card_is_as_wide_and_as_tall_as_it_was_asked_to_be() {
        // The quad is the unit square centred on its own origin, so the corner
        // is half of the size away from the card's centre on each axis.
        let m = card([1.0, 2.0, 3.0], 0.0, 0.0, 0.0, 0.55, 1.0);
        assert_eq!(apply(m, [0.0, 0.0, 0.0]), [1.0, 2.0, 3.0, 1.0]);
        let corner = apply(m, [0.5, 0.5, 0.0]);
        assert!((corner[0] - 1.275).abs() < 1e-5, "{corner:?}");
        assert!((corner[1] - 2.5).abs() < 1e-5, "{corner:?}");
    }

    #[test]
    fn a_rolled_card_turns_in_the_page_but_stays_on_it() {
        let m = card([0.0, 0.0, 0.0], 0.0, 0.0, std::f32::consts::FRAC_PI_2, 0.55, 1.0);
        let corner = apply(m, [0.5, 0.0, 0.0]);
        // A quarter turn sends the right edge to the top edge, and the card keeps
        // its face on the page.
        assert!(corner[0].abs() < 1e-4, "{corner:?}");
        assert!((corner[1] - 0.275).abs() < 1e-4, "{corner:?}");
        assert!(corner[2].abs() < 1e-4, "{corner:?}");
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
    fn the_shear_leaves_the_page_alone_and_moves_what_stands_on_it() {
        let shear = oblique(0.4, -0.2);
        // A point on the page does not move.
        assert_eq!(apply(shear, [3.0, 4.0, 0.0]), [3.0, 4.0, 0.0, 1.0]);
        // A point in front of it moves right and down, which is a box seen from
        // slightly above and to the left.
        let front = apply(shear, [3.0, 4.0, 1.0]);
        assert!(front[0] > 3.0, "{front:?}");
        assert!(front[1] < 4.0, "{front:?}");
        assert_eq!(front[2], 1.0);
    }

    #[test]
    fn the_orthographic_camera_puts_the_page_where_it_belongs() {
        let view = look_at([0.0, 0.0, 6.0], [0.0, 0.0, 0.0], [0.0, 1.0, 0.0]);
        let proj = ortho(0.0, 66.0, 0.0, 40.0, 1.0, 16.0);
        let view_proj = mul(proj, view);
        // The bottom left of the page is the bottom left of the window.
        let corner = apply(view_proj, [0.0, 0.0, 0.0]);
        assert!((corner[0] + 1.0).abs() < 1e-5, "{corner:?}");
        assert!((corner[1] + 1.0).abs() < 1e-5, "{corner:?}");
        // The middle is the middle.
        let middle = apply(view_proj, [33.0, 20.0, 0.0]);
        assert!(middle[0].abs() < 1e-4 && middle[1].abs() < 1e-4, "{middle:?}");
        // Depth is in `0..1`, and nearer is smaller.
        let page = apply(view_proj, [33.0, 20.0, 0.0])[2];
        let kitten = apply(view_proj, [33.0, 20.0, 1.0])[2];
        assert!(page > 0.0 && page < 1.0, "{page}");
        assert!(kitten < page, "{kitten} {page}");
    }

    #[test]
    fn a_point_behind_the_camera_is_behind_it() {
        let view = look_at([0.0, 0.0, 6.0], [0.0, 0.0, 0.0], [0.0, 1.0, 0.0]);
        // The page is 6 units in front of the eye, so a point 20 units behind the
        // eye has a positive view z, which the depth range does not cover.
        let behind = apply(view, [0.0, 0.0, 20.0]);
        assert!(behind[2] > 0.0, "the view turns forward into +Z, so {behind:?}");
    }
}
