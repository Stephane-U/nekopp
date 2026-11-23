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

//! Drawing the vector icons.
//!
//! The icons are paths, but a GPU draws triangles and textures, so the paths are
//! rasterised once into an alpha atlas and every icon is then one textured
//! rectangle. The atlas is rebuilt when the icon size changes — a window moving
//! to a high-density display, or the toolbar growing with the font — so the
//! drawing is always sampled at the size it is shown at rather than scaled up
//! from a fixed bitmap.
//!
//! Two cells per icon: the body and the accent. They are separate because they
//! are tinted with different colours, and keeping them apart means the tint is a
//! shader constant rather than a second texture lookup with a mask.

use wgpu::util::DeviceExt;

use crate::icons::{self, ICONS};
use crate::vector;

/// How many cells fit across the atlas.
const COLUMNS: u32 = 8;

/// The rasterised icons, as one single-channel texture.
pub struct IconAtlas {
    /// The side of one cell, in pixels.
    pub size: u32,
    /// Cells across.
    pub cols: u32,
    /// Cells down.
    pub rows: u32,
    /// `cols * size` by `rows * size` coverage values, row-major.
    pub pixels: Vec<u8>,
}

impl IconAtlas {
    /// Rasterises every icon at `size` pixels square.
    ///
    /// The work is a few hundred thousand arithmetic operations for the whole
    /// set, which is why it is done on the spot rather than cached to disk.
    pub fn build(size: u32) -> Self {
        let size = size.max(8);
        let cells = ICONS.len() as u32 * 2;
        let cols = COLUMNS;
        let rows = cells.div_ceil(cols);
        let (w, h) = (cols * size, rows * size);
        let mut pixels = vec![0u8; (w * h) as usize];
        for (index, icon) in ICONS.iter().enumerate() {
            for (half, parts) in [icon.ink, icon.accent].into_iter().enumerate() {
                let cell = index as u32 * 2 + half as u32;
                let (cx, cy) = ((cell % cols) * size, (cell / cols) * size);
                let mask = vector::render_parts(parts, icons::VIEW, size, size);
                for y in 0..size {
                    let src = (y * size) as usize;
                    let dst = ((cy + y) * w + cx) as usize;
                    pixels[dst..dst + size as usize]
                        .copy_from_slice(&mask[src..src + size as usize]);
                }
            }
        }
        Self {
            size,
            cols,
            rows,
            pixels,
        }
    }

    /// The texture coordinates of one half of one icon.
    ///
    /// A half-pixel inset keeps a linear sample from picking up the neighbouring
    /// cell along the edge of a quad.
    pub fn uv(&self, icon: usize, accent: bool) -> [f32; 4] {
        let cell = icon as u32 * 2 + accent as u32;
        let (cx, cy) = ((cell % self.cols) * self.size, (cell / self.cols) * self.size);
        let (w, h) = (self.cols * self.size, self.rows * self.size);
        let inset = 0.5;
        [
            (cx as f32 + inset) / w as f32,
            (cy as f32 + inset) / h as f32,
            (cx as f32 + self.size as f32 - inset) / w as f32,
            (cy as f32 + self.size as f32 - inset) / h as f32,
        ]
    }

    /// The width of the atlas texture.
    pub fn width(&self) -> u32 {
        self.cols * self.size
    }

    /// The height of the atlas texture.
    pub fn height(&self) -> u32 {
        self.rows * self.size
    }
}

/// One icon rectangle.
#[repr(C)]
#[derive(Debug, Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
struct Instance {
    /// `x`, `y`, `width`, `height` in pixels.
    rect: [f32; 4],
    /// `u0`, `v0`, `u1`, `v1`.
    uv: [f32; 4],
    /// Linear RGBA.
    color: [f32; 4],
}

const SHADER: &str = r#"
struct Globals {
    resolution: vec2<f32>,
    _pad: vec2<f32>,
};

@group(0) @binding(0) var<uniform> globals: Globals;
@group(0) @binding(1) var atlas: texture_2d<f32>;
@group(0) @binding(2) var atlas_sampler: sampler;

struct Instance {
    @location(0) rect: vec4<f32>,
    @location(1) uv: vec4<f32>,
    @location(2) color: vec4<f32>,
};

struct VsOut {
    @builtin(position) position: vec4<f32>,
    @location(0) uv: vec2<f32>,
    @location(1) color: vec4<f32>,
};

@vertex
fn vs_main(@builtin(vertex_index) vi: u32, inst: Instance) -> VsOut {
    var corners = array<vec2<f32>, 6>(
        vec2<f32>(0.0, 0.0),
        vec2<f32>(1.0, 0.0),
        vec2<f32>(0.0, 1.0),
        vec2<f32>(1.0, 0.0),
        vec2<f32>(1.0, 1.0),
        vec2<f32>(0.0, 1.0),
    );
    let corner = corners[vi];
    let pixel = inst.rect.xy + corner * inst.rect.zw;
    let ndc = vec2<f32>(
        pixel.x / globals.resolution.x * 2.0 - 1.0,
        1.0 - pixel.y / globals.resolution.y * 2.0,
    );
    var out: VsOut;
    out.position = vec4<f32>(ndc, 0.0, 1.0);
    out.uv = mix(inst.uv.xy, inst.uv.zw, corner);
    out.color = inst.color;
    return out;
}

@fragment
fn fs_main(in: VsOut) -> @location(0) vec4<f32> {
    let coverage = textureSample(atlas, atlas_sampler, in.uv).r;
    return vec4<f32>(in.color.rgb, in.color.a * coverage);
}
"#;

/// Draws batches of tinted icons from the atlas.
pub struct IconRenderer {
    pipeline: wgpu::RenderPipeline,
    layout: wgpu::BindGroupLayout,
    bind_group: wgpu::BindGroup,
    texture: wgpu::Texture,
    sampler: wgpu::Sampler,
    resolution: wgpu::Buffer,
    instances: wgpu::Buffer,
    data: Vec<Instance>,
    capacity: usize,
    /// The cell size the atlas was built at, so a resize can be detected.
    atlas_size: u32,
    /// The texture coordinates of every cell, two per icon, in the order the
    /// icon table lists them. Computed once when the atlas is built: rebuilding
    /// the atlas on every queued icon would rasterise the whole set per button.
    uvs: Vec<[f32; 4]>,
}

impl IconRenderer {
    /// Builds the pipeline and an atlas at `size` pixels per icon.
    pub fn new(device: &wgpu::Device, queue: &wgpu::Queue, format: wgpu::TextureFormat, size: u32) -> Self {
        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("icon shader"),
            source: wgpu::ShaderSource::Wgsl(SHADER.into()),
        });

        let resolution = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("icon resolution"),
            size: 16,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });

        let layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("icon bind group layout"),
            entries: &[
                wgpu::BindGroupLayoutEntry {
                    binding: 0,
                    visibility: wgpu::ShaderStages::VERTEX,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Uniform,
                        has_dynamic_offset: false,
                        min_binding_size: None,
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 1,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Texture {
                        sample_type: wgpu::TextureSampleType::Float { filterable: true },
                        view_dimension: wgpu::TextureViewDimension::D2,
                        multisampled: false,
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 2,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
                    count: None,
                },
            ],
        });

        let sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("icon sampler"),
            mag_filter: wgpu::FilterMode::Linear,
            min_filter: wgpu::FilterMode::Linear,
            address_mode_u: wgpu::AddressMode::ClampToEdge,
            address_mode_v: wgpu::AddressMode::ClampToEdge,
            ..Default::default()
        });

        let atlas = IconAtlas::build(size);
        let uvs = Self::uvs(&atlas);
        let texture = Self::make_texture(device, queue, &atlas);
        let bind_group = Self::make_bind_group(device, &layout, &texture, &sampler, &resolution);

        let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("icon pipeline layout"),
            bind_group_layouts: &[Some(&layout)],
            immediate_size: 0,
        });

        let pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("icon pipeline"),
            layout: Some(&pipeline_layout),
            vertex: wgpu::VertexState {
                module: &shader,
                entry_point: Some("vs_main"),
                compilation_options: Default::default(),
                buffers: &[Some(wgpu::VertexBufferLayout {
                    array_stride: std::mem::size_of::<Instance>() as u64,
                    step_mode: wgpu::VertexStepMode::Instance,
                    attributes: &[
                        wgpu::VertexAttribute {
                            offset: 0,
                            shader_location: 0,
                            format: wgpu::VertexFormat::Float32x4,
                        },
                        wgpu::VertexAttribute {
                            offset: 16,
                            shader_location: 1,
                            format: wgpu::VertexFormat::Float32x4,
                        },
                        wgpu::VertexAttribute {
                            offset: 32,
                            shader_location: 2,
                            format: wgpu::VertexFormat::Float32x4,
                        },
                    ],
                })],
            },
            fragment: Some(wgpu::FragmentState {
                module: &shader,
                entry_point: Some("fs_main"),
                compilation_options: Default::default(),
                targets: &[Some(wgpu::ColorTargetState {
                    format,
                    blend: Some(wgpu::BlendState::ALPHA_BLENDING),
                    write_mask: wgpu::ColorWrites::ALL,
                })],
            }),
            primitive: wgpu::PrimitiveState::default(),
            depth_stencil: None,
            multisample: wgpu::MultisampleState::default(),
            multiview_mask: None,
            cache: None,
        });

        let capacity = 256;
        let instances = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("icon instances"),
            contents: bytemuck::cast_slice(&vec![
                Instance {
                    rect: [0.0; 4],
                    uv: [0.0; 4],
                    color: [0.0; 4],
                };
                capacity
            ]),
            usage: wgpu::BufferUsages::VERTEX | wgpu::BufferUsages::COPY_DST,
        });

        Self {
            pipeline,
            layout,
            bind_group,
            texture,
            sampler,
            resolution,
            instances,
            data: Vec::with_capacity(capacity),
            capacity,
            atlas_size: size.max(8),
            uvs,
        }
    }

    /// The texture coordinates of every cell, in icon order.
    fn uvs(atlas: &IconAtlas) -> Vec<[f32; 4]> {
        let mut out = Vec::with_capacity(ICONS.len() * 2);
        for index in 0..ICONS.len() {
            out.push(atlas.uv(index, false));
            out.push(atlas.uv(index, true));
        }
        out
    }

    fn make_texture(device: &wgpu::Device, queue: &wgpu::Queue, atlas: &IconAtlas) -> wgpu::Texture {
        let texture = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("icon atlas"),
            size: wgpu::Extent3d {
                width: atlas.width(),
                height: atlas.height(),
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::R8Unorm,
            usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
            view_formats: &[],
        });
        queue.write_texture(
            wgpu::TexelCopyTextureInfo {
                texture: &texture,
                mip_level: 0,
                origin: wgpu::Origin3d::ZERO,
                aspect: wgpu::TextureAspect::All,
            },
            &atlas.pixels,
            wgpu::TexelCopyBufferLayout {
                offset: 0,
                bytes_per_row: Some(atlas.width()),
                rows_per_image: Some(atlas.height()),
            },
            wgpu::Extent3d {
                width: atlas.width(),
                height: atlas.height(),
                depth_or_array_layers: 1,
            },
        );
        texture
    }

    fn make_bind_group(
        device: &wgpu::Device,
        layout: &wgpu::BindGroupLayout,
        texture: &wgpu::Texture,
        sampler: &wgpu::Sampler,
        resolution: &wgpu::Buffer,
    ) -> wgpu::BindGroup {
        let view = texture.create_view(&wgpu::TextureViewDescriptor::default());
        device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("icon bind group"),
            layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: resolution.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: wgpu::BindingResource::TextureView(&view),
                },
                wgpu::BindGroupEntry {
                    binding: 2,
                    resource: wgpu::BindingResource::Sampler(sampler),
                },
            ],
        })
    }

    /// Rebuilds the atlas when the icons are being drawn at a different size.
    ///
    /// Returns true when it did, so the caller can skip the work on every other
    /// frame.
    pub fn set_size(&mut self, device: &wgpu::Device, queue: &wgpu::Queue, size: u32) -> bool {
        let size = size.max(8);
        if size == self.atlas_size {
            return false;
        }
        self.atlas_size = size;
        let atlas = IconAtlas::build(size);
        self.uvs = Self::uvs(&atlas);
        self.texture = Self::make_texture(device, queue, &atlas);
        self.bind_group =
            Self::make_bind_group(device, &self.layout, &self.texture, &self.sampler, &self.resolution);
        true
    }

    /// The cell size the atlas is currently built at.
    pub fn atlas_size(&self) -> u32 {
        self.atlas_size
    }

    /// Discards the previous frame's icons.
    pub fn begin(&mut self) {
        self.data.clear();
    }

    /// Queues one icon.
    ///
    /// `icon` is an index into [`crate::icons::ICONS`]; `accent` picks the half
    /// of the cell to draw.
    ///
    /// Seven arguments is one over the lint's limit, and the alternative — a
    /// struct for the rectangle — would make every call site longer than the
    /// call it replaces. The order is the same as [`QuadRenderer::push`], so a
    /// reader who knows one knows the other.
    #[allow(clippy::too_many_arguments)]
    pub fn push(
        &mut self,
        icon: usize,
        accent: bool,
        x: f32,
        y: f32,
        width: f32,
        height: f32,
        color: [f32; 4],
    ) {
        if width <= 0.0 || height <= 0.0 {
            return;
        }
        let Some(uv) = self.uvs.get(icon * 2 + accent as usize).copied() else {
            return;
        };
        self.data.push(Instance {
            rect: [x, y, width, height],
            uv,
            color,
        });
    }

    /// Uploads the queued icons and the target resolution.
    pub fn upload(&mut self, device: &wgpu::Device, queue: &wgpu::Queue, width: f32, height: f32) {
        if self.data.len() > self.capacity {
            self.capacity = self.data.len().next_power_of_two();
            self.instances = device.create_buffer(&wgpu::BufferDescriptor {
                label: Some("icon instances"),
                size: (self.capacity * std::mem::size_of::<Instance>()) as u64,
                usage: wgpu::BufferUsages::VERTEX | wgpu::BufferUsages::COPY_DST,
                mapped_at_creation: false,
            });
        }
        if !self.data.is_empty() {
            queue.write_buffer(&self.instances, 0, bytemuck::cast_slice(&self.data));
        }
        queue.write_buffer(
            &self.resolution,
            0,
            bytemuck::cast_slice(&[width, height, 0.0, 0.0]),
        );
    }

    /// Draws every queued icon.
    pub fn draw(&self, pass: &mut wgpu::RenderPass<'_>) {
        if self.data.is_empty() {
            return;
        }
        pass.set_pipeline(&self.pipeline);
        pass.set_bind_group(0, &self.bind_group, &[]);
        pass.set_vertex_buffer(0, self.instances.slice(..));
        pass.draw(0..6, 0..self.data.len() as u32);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_atlas_holds_two_cells_per_icon() {
        let atlas = IconAtlas::build(16);
        assert_eq!(atlas.cols, COLUMNS);
        assert_eq!(atlas.rows, (ICONS.len() as u32 * 2).div_ceil(COLUMNS));
        assert_eq!(atlas.pixels.len(), (atlas.width() * atlas.height()) as usize);
    }

    #[test]
    fn every_icon_has_coverage_in_at_least_one_half() {
        let atlas = IconAtlas::build(24);
        for (index, icon) in ICONS.iter().enumerate() {
            let mut total = 0;
            for accent in [false, true] {
                let cell = index as u32 * 2 + accent as u32;
                let (cx, cy) = ((cell % atlas.cols) * atlas.size, (cell / atlas.cols) * atlas.size);
                for y in 0..atlas.size {
                    for x in 0..atlas.size {
                        let i = ((cy + y) * atlas.width() + cx + x) as usize;
                        if atlas.pixels[i] > 8 {
                            total += 1;
                        }
                    }
                }
            }
            // `open` is drawn entirely in the accent colour, so the check is on
            // the pair rather than on the body alone.
            assert!(total > 20, "{} drew nothing", icon.name);
        }
    }

    #[test]
    fn texture_coordinates_stay_inside_the_atlas() {
        let atlas = IconAtlas::build(20);
        for index in 0..ICONS.len() {
            for accent in [false, true] {
                let uv = atlas.uv(index, accent);
                assert!(uv[0] >= 0.0 && uv[2] <= 1.0, "{uv:?}");
                assert!(uv[1] >= 0.0 && uv[3] <= 1.0, "{uv:?}");
                assert!(uv[0] < uv[2] && uv[1] < uv[3], "{uv:?}");
            }
        }
    }

    #[test]
    fn two_icons_never_share_a_cell() {
        let atlas = IconAtlas::build(20);
        let mut seen: Vec<[f32; 4]> = Vec::new();
        for index in 0..ICONS.len() {
            for accent in [false, true] {
                let uv = atlas.uv(index, accent);
                assert!(!seen.contains(&uv), "cell reused at {index}");
                seen.push(uv);
            }
        }
    }

    #[test]
    fn a_tiny_size_is_clamped_rather_than_dividing_by_zero() {
        let atlas = IconAtlas::build(0);
        assert!(atlas.size >= 8);
        assert!(!atlas.pixels.is_empty());
    }
}
