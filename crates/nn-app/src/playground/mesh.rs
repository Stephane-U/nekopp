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

//! The two shapes a playground frame is made of.
//!
//! Everything in the room is either a quad — a glyph card, a dust mote, the
//! shadow under a token — or a box — a kitten limb, a table, a wall panel. Rather
//! than one mesh per object, both are built once at unit size and placed by an
//! instance matrix, so a room of five hundred cards is one draw call.
//!
//! The boxes are rounded. A cube with sharp edges reads as a diagram of a box
//! rather than as a thing; the corner radius is what makes a stack of six of them
//! look like a cat, and it costs nothing at draw time because the rounding is
//! baked into the vertices. The rounding is the standard clamp-and-push: take the
//! point on the face, clamp it into the inner box, and push it out along the
//! direction it was clamped from by the radius. Flat faces stay flat, because
//! there the point and its clamp are the same place.

/// One vertex. Position and normal in the mesh's own space, texture coordinates
/// in `0..1` across the face.
#[repr(C)]
#[derive(Debug, Clone, Copy, PartialEq, bytemuck::Pod, bytemuck::Zeroable)]
pub struct Vertex {
    pub pos: [f32; 3],
    pub normal: [f32; 3],
    pub uv: [f32; 2],
}

impl Vertex {
    /// How many bytes one vertex is, for the vertex buffer layout.
    pub const STRIDE: u64 = std::mem::size_of::<Self>() as u64;
}

/// A mesh, ready to hand to the device.
#[derive(Debug, Clone, PartialEq)]
pub struct Mesh {
    pub vertices: Vec<Vertex>,
    pub indices: Vec<u32>,
}

impl Mesh {
    /// The unit square in the `XY` plane, centred on the origin, facing `+Z`.
    ///
    /// Texture coordinates are `0..1` from the bottom left, so the atlas cell is
    /// upright on the card.
    pub fn quad() -> Self {
        let corners = [
            ([-0.5, -0.5], [0.0, 0.0]),
            ([0.5, -0.5], [1.0, 0.0]),
            ([0.5, 0.5], [1.0, 1.0]),
            ([-0.5, 0.5], [0.0, 1.0]),
        ];
        let vertices = corners
            .iter()
            .map(|(pos, uv)| Vertex {
                pos: [pos[0], pos[1], 0.0],
                normal: [0.0, 0.0, 1.0],
                uv: *uv,
            })
            .collect();
        Self {
            vertices,
            indices: vec![0, 1, 2, 0, 2, 3],
        }
    }

    /// A box of side one, centred on the origin, with its edges rounded by
    /// `radius` and `segments` steps across each face.
    ///
    /// `radius` is in the same units as the box, so `0.5` is a sphere and `0.0` a
    /// cube. `segments` of one leaves the faces as two triangles each; four is
    /// where a corner stops looking like a chamfer.
    pub fn rounded_box(radius: f32, segments: u32) -> Self {
        let radius = radius.clamp(0.0, 0.5);
        let segments = segments.max(1);
        let inner = 0.5 - radius;
        let mut vertices = Vec::new();
        let mut indices = Vec::new();

        // Six faces: which axis is fixed, which way it faces, and the two axes
        // that spread across it.
        let faces: [(usize, f32); 6] = [
            (0, 1.0),
            (0, -1.0),
            (1, 1.0),
            (1, -1.0),
            (2, 1.0),
            (2, -1.0),
        ];
        for (axis, sign) in faces {
            let (u_axis, v_axis) = match axis {
                0 => (1, 2),
                1 => (0, 2),
                _ => (0, 1),
            };
            let base = vertices.len() as u32;
            for v in 0..=segments {
                for u in 0..=segments {
                    let fu = u as f32 / segments as f32;
                    let fv = v as f32 / segments as f32;
                    let mut pos = [0.0f32; 3];
                    pos[axis] = sign * 0.5;
                    pos[u_axis] = fu - 0.5;
                    pos[v_axis] = fv - 0.5;
                    // Clamped into the inner box, then pushed back out.
                    let clamped = [
                        pos[0].clamp(-inner, inner),
                        pos[1].clamp(-inner, inner),
                        pos[2].clamp(-inner, inner),
                    ];
                    let mut away = [
                        pos[0] - clamped[0],
                        pos[1] - clamped[1],
                        pos[2] - clamped[2],
                    ];
                    let length = (away[0] * away[0] + away[1] * away[1] + away[2] * away[2])
                        .sqrt();
                    let normal = if length > 1e-6 {
                        away = [away[0] / length, away[1] / length, away[2] / length];
                        away
                    } else {
                        let mut n = [0.0f32; 3];
                        n[axis] = sign;
                        n
                    };
                    vertices.push(Vertex {
                        pos: [
                            clamped[0] + normal[0] * radius,
                            clamped[1] + normal[1] * radius,
                            clamped[2] + normal[2] * radius,
                        ],
                        normal,
                        uv: [fu, fv],
                    });
                }
            }
            let row = segments + 1;
            // Which way a triangle wound `u` then `v` faces: the cross product of
            // the two axes the face spreads along. The face's own normal decides
            // whether that is the outside or the inside.
            let mut u_vec = [0.0f32; 3];
            let mut v_vec = [0.0f32; 3];
            u_vec[u_axis] = 1.0;
            v_vec[v_axis] = 1.0;
            let wind = [
                u_vec[1] * v_vec[2] - u_vec[2] * v_vec[1],
                u_vec[2] * v_vec[0] - u_vec[0] * v_vec[2],
                u_vec[0] * v_vec[1] - u_vec[1] * v_vec[0],
            ];
            let outward = wind[axis] * sign > 0.0;
            for v in 0..segments {
                for u in 0..segments {
                    let corner = base + v * row + u;
                    let right = corner + 1;
                    let down = corner + row;
                    let diagonal = down + 1;
                    if outward {
                        indices.extend_from_slice(&[corner, right, diagonal]);
                        indices.extend_from_slice(&[corner, diagonal, down]);
                    } else {
                        indices.extend_from_slice(&[corner, diagonal, right]);
                        indices.extend_from_slice(&[corner, down, diagonal]);
                    }
                }
            }
        }

        Self { vertices, indices }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_quad_is_four_corners_and_two_triangles() {
        let quad = Mesh::quad();
        assert_eq!(quad.vertices.len(), 4);
        assert_eq!(quad.indices.len(), 6);
        assert!(quad.vertices.iter().all(|v| v.normal == [0.0, 0.0, 1.0]));
        // The corners are the unit square, centred.
        assert!(quad
            .vertices
            .iter()
            .all(|v| v.pos[0].abs() == 0.5 && v.pos[1].abs() == 0.5 && v.pos[2] == 0.0));
    }

    #[test]
    fn a_box_is_six_faces_and_stays_inside_the_unit_cube() {
        let mesh = Mesh::rounded_box(0.2, 3);
        assert_eq!(mesh.vertices.len(), 6 * 16);
        assert_eq!(mesh.indices.len(), 6 * 9 * 6);
        for vertex in &mesh.vertices {
            for axis in 0..3 {
                assert!(vertex.pos[axis].abs() <= 0.5 + 1e-5, "{:?}", vertex.pos);
            }
            let n = vertex.normal;
            let length = (n[0] * n[0] + n[1] * n[1] + n[2] * n[2]).sqrt();
            assert!((length - 1.0).abs() < 1e-4, "{n:?}");
        }
    }

    #[test]
    fn rounding_pulls_the_corners_in() {
        let sharp = Mesh::rounded_box(0.0, 4);
        let round = Mesh::rounded_box(0.4, 4);
        // The distance from the centre to the farthest vertex is the diagonal of
        // the half-cube when sharp, and less than that once rounded.
        let reach = |mesh: &Mesh| {
            mesh.vertices
                .iter()
                .map(|v| (v.pos[0] * v.pos[0] + v.pos[1] * v.pos[1] + v.pos[2] * v.pos[2]).sqrt())
                .fold(0.0f32, f32::max)
        };
        let (sharp, round) = (reach(&sharp), reach(&round));
        assert!(sharp > 0.86, "{sharp}");
        assert!(round < sharp - 0.05, "{round} {sharp}");
    }

    #[test]
    fn a_flat_face_keeps_its_normal_and_its_place() {
        // The middle of a face is not near a corner, so nothing moves it.
        let mesh = Mesh::rounded_box(0.25, 4);
        let middle = mesh
            .vertices
            .iter()
            .find(|v| v.pos[0] > 0.49 && v.pos[1].abs() < 0.01 && v.pos[2].abs() < 0.01)
            .expect("the middle of the +X face");
        assert!((middle.pos[0] - 0.5).abs() < 1e-5, "{:?}", middle.pos);
        assert_eq!(middle.normal, [1.0, 0.0, 0.0]);
    }

    #[test]
    fn every_triangle_winds_outwards() {
        // A triangle whose corners are in the wrong order is invisible with the
        // back faces culled, and the missing face is always the one behind the
        // kitten: this is what catches it.
        let mesh = Mesh::rounded_box(0.25, 3);
        for triangle in mesh.indices.chunks(3) {
            let [a, b, c] = [
                mesh.vertices[triangle[0] as usize].pos,
                mesh.vertices[triangle[1] as usize].pos,
                mesh.vertices[triangle[2] as usize].pos,
            ];
            let e1 = [b[0] - a[0], b[1] - a[1], b[2] - a[2]];
            let e2 = [c[0] - a[0], c[1] - a[1], c[2] - a[2]];
            let normal = [
                e1[1] * e2[2] - e1[2] * e2[1],
                e1[2] * e2[0] - e1[0] * e2[2],
                e1[0] * e2[1] - e1[1] * e2[0],
            ];
            let middle = [
                (a[0] + b[0] + c[0]) / 3.0,
                (a[1] + b[1] + c[1]) / 3.0,
                (a[2] + b[2] + c[2]) / 3.0,
            ];
            let out = normal[0] * middle[0] + normal[1] * middle[1] + normal[2] * middle[2];
            assert!(out > 0.0, "inward triangle at {middle:?}");
        }
    }

    #[test]
    fn one_segment_is_a_cube() {
        let mesh = Mesh::rounded_box(0.0, 1);
        assert_eq!(mesh.vertices.len(), 24);
        assert_eq!(mesh.indices.len(), 36);
    }

    #[test]
    fn every_vertex_is_packed_for_the_gpu() {
        assert_eq!(Vertex::STRIDE, 32);
    }
}
