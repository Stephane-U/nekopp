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

//! Drawing the page.
//!
//! What the reader sees is a page of text: one cell is one cell wherever it sits
//! on screen, and a glyph lands on whole pixels. So the camera is orthographic and
//! the cards are drawn flat, straight through the projection, with no shear and no
//! depth sorting to get wrong — punctuation at the back of the page is the same
//! size as punctuation at the front, which is the whole point of a document.
//!
//! The kitten is the exception, and she is the reason the program still has a
//! depth buffer. Her boxes go through the same projection pre-multiplied by one
//! oblique shear, so a cube's side and top are visible while the page it stands on
//! stays square. Without the shear she would be a silhouette of rectangles on a
//! page; with it she has a far side and a near side, and the letters do not tilt.
//!
//! One more: text is not lit. A document whose syntax colours depended on where a
//! kitten's lamp happened to be would be a document that changes colour as a cat
//! walks past it. Only the boxes are lit, and the whole picture dims together when
//! the screensaver dims it.

use wgpu::util::DeviceExt;

use nn_core::playground::{CardKind, Frame, GlowKind};

use crate::palette::Palette;
use crate::theme::{Theme, ToGpu};

use super::glyphs::{GlyphAtlas, FIRST, LAST};
use super::mesh::{Mesh, Vertex};
use super::xform;

/// The depth format: the one every backend has, with no stencil to go wrong.
const DEPTH_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Depth32Float;
/// How far in front of the page the camera stands, in cells.
///
/// Far enough that the depth range holds the kitten's boxes and a lifted token,
/// and near enough that the numbers stay small. An orthographic camera's distance
/// changes nothing about the picture, only the depth values.
const CAMERA_Z: f32 = 6.0;
/// How far in front of the page the depth range starts.
const NEAR: f32 = 1.0;
/// How far the depth range reaches past the eye.
const FAR: f32 = 16.0;
/// How much a box is sheared to the right by its own depth.
const SHEAR_X: f32 = 0.26;
/// How much a box is sheared down by its own depth: the camera is a little above
/// the page, so what is nearer to the reader sits lower on it.
const SHEAR_Y: f32 = -0.13;
/// The corner radius of the boxes, in cell units.
const BOX_RADIUS: f32 = 0.2;
/// How many steps each face of a box is divided into.
const BOX_SEGMENTS: u32 = 3;
/// The direction the lamp's light travels: down, and towards the reader.
const LIGHT: [f32; 3] = [0.38, -0.86, 0.34];
/// The most glyph cards, boxes and dots the buffers start big enough for.
const MAX_GLYPHS: usize = 4096;
const MAX_BOXES: usize = 1024;
const MAX_DOTS: usize = 512;
const MAX_GLOWS: usize = 1024;

const SHADER: &str = r#"
struct Globals {
    view_proj: mat4x4<f32>,
    eye: vec4<f32>,
    light: vec4<f32>,
    /// The screensaver's dimming in `x`, and the mode's own clock in `y`, which the
    /// rainbow drifts with.
    dim: vec4<f32>,
    tint: vec4<f32>,
};

@group(0) @binding(0) var<uniform> globals: Globals;
@group(0) @binding(1) var glyphs: texture_2d<f32>;
@group(0) @binding(2) var glyph_sampler: sampler;

struct VertexIn {
    @location(0) pos: vec3<f32>,
    @location(1) normal: vec3<f32>,
    @location(2) uv: vec2<f32>,
};

struct Instance {
    @location(3) m0: vec4<f32>,
    @location(4) m1: vec4<f32>,
    @location(5) m2: vec4<f32>,
    @location(6) m3: vec4<f32>,
    @location(7) color: vec4<f32>,
    @location(8) uv: vec4<f32>,
};

struct VsOut {
    @builtin(position) position: vec4<f32>,
    @location(0) world: vec3<f32>,
    @location(1) normal: vec3<f32>,
    @location(2) uv: vec2<f32>,
    @location(3) local: vec2<f32>,
    @location(4) color: vec4<f32>,
};

@vertex
fn vs_main(vertex: VertexIn, inst: Instance) -> VsOut {
    let model = mat4x4<f32>(inst.m0, inst.m1, inst.m2, inst.m3);
    let world = model * vec4<f32>(vertex.pos, 1.0);
    var out: VsOut;
    out.position = globals.view_proj * world;
    out.world = world.xyz;
    out.normal = (model * vec4<f32>(vertex.normal, 0.0)).xyz;
    out.uv = mix(inst.uv.xy, inst.uv.zw, vertex.uv);
    out.local = vertex.uv;
    out.color = inst.color;
    return out;
}

fn lit(base: vec3<f32>, world: vec3<f32>, normal: vec3<f32>, rim_amount: f32) -> vec3<f32> {
    let n = normalize(normal);
    let lamp = normalize(-globals.light.xyz);
    let lambert = abs(dot(n, lamp));
    let to_eye = normalize(globals.eye.xyz - world);
    let rim = pow(1.0 - abs(dot(n, to_eye)), 2.0) * rim_amount;
    return base * (0.74 + 0.26 * lambert + rim);
}

/// The screensaver's dimming, and nothing else.
///
/// A page of code is not a lit surface: a document whose syntax colours depended
/// on where the kitten's lamp happened to be would be a document that changes
/// colour as a cat walks past it.
fn dimmed(colour: vec3<f32>) -> vec3<f32> {
    return colour * (1.0 - globals.dim.x);
}

@fragment
fn fs_glyph(in: VsOut) -> @location(0) vec4<f32> {
    let coverage = textureSample(glyphs, glyph_sampler, in.uv).r;
    if (coverage < 0.02) {
        discard;
    }
    return vec4<f32>(dimmed(in.color.rgb), in.color.a * coverage);
}

@fragment
fn fs_box(in: VsOut) -> @location(0) vec4<f32> {
    return vec4<f32>(dimmed(lit(in.color.rgb, in.world, in.normal, 0.16)), in.color.a);
}

@fragment
fn fs_dot(in: VsOut) -> @location(0) vec4<f32> {
    let radius = length(in.local * 2.0 - 1.0);
    let falloff = clamp(1.0 - radius, 0.0, 1.0);
    let alpha = falloff * falloff * in.color.a;
    if (alpha < 0.004) {
        discard;
    }
    return vec4<f32>(dimmed(in.color.rgb), alpha);
}

/// Hue to colour: one turn of the wheel, red at zero.
///
/// The simulation sends a number and knows nothing about colour; every rainbow in
/// the mode comes out of this function, which is why a trail turns smoothly rather
/// than stepping between a palette of six.
fn rainbow(hue: f32) -> vec3<f32> {
    let h = fract(hue) * 6.0;
    let r = clamp(abs(h - 3.0) - 1.0, 0.0, 1.0);
    let g = clamp(2.0 - abs(h - 2.0), 0.0, 1.0);
    let b = clamp(2.0 - abs(h - 4.0), 0.0, 1.0);
    return vec3<f32>(r, g, b);
}

/// A glowing thing, added to whatever is under it.
///
/// `color` carries the whole of it: `x` the hue, `y` the alpha, `z` which shape,
/// `w` a fixed seed per spark. Additive, so there is no order to get wrong and
/// overlapping sparks are brighter than one -- which is what makes a trail read as
/// light rather than as a row of pasted sprites.
///
/// The hue drifts with the clock and with distance from the middle, so a trail
/// shimmers as it fades and no two sparks are exactly one colour.
@fragment
fn fs_glow(in: VsOut) -> @location(0) vec4<f32> {
    let p = in.local * 2.0 - 1.0;
    let d = length(p);
    let kind = in.color.z;
    let hue = in.color.x + globals.dim.y * 0.12 + d * 0.10;
    var amount = 0.0;
    if (kind < 0.5) {
        // A ball of light with a hot middle: a piece of a trail, or confetti.
        let core = clamp(1.0 - d, 0.0, 1.0);
        amount = core * core * (1.35 - 0.35 * d);
    } else if (kind < 1.5) {
        // A ring on the paper: the wave of a landing, thin at the edge and thick
        // where it is brightest.
        let edge = abs(d - 0.82);
        amount = (1.0 - smoothstep(0.0, 0.22, edge)) * (1.0 - 0.25 * d);
    } else {
        // A star: six rays through a hot middle, turned by the spark's own roll.
        let a = atan2(p.y, p.x) + in.color.w * 6.2831855;
        let rays = pow(abs(cos(a * 3.0)), 12.0);
        let core = clamp(1.2 - d * 1.4, 0.0, 1.0);
        amount = core * core + rays * 0.7 * core;
    }
    if (amount < 0.004) {
        discard;
    }
    // Premultiplied: the pipeline adds `src` to `dst`, so the alpha channel carries
    // nothing and a glow never dims what is under it. It fades with the page, but
    // only a third as far: light outlasts the text it was thrown over, which is what
    // makes a page left alone look like a page with something still moving on it.
    let dark = 1.0 - 0.35 * globals.dim.x;
    return vec4<f32>(rainbow(hue) * amount * in.color.y * dark, 0.0);
}
"#;

/// One instance: where a piece of geometry goes, what colour it is, and — for a
/// glyph card — which cell of the atlas it samples.
#[repr(C)]
#[derive(Debug, Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
struct Instance {
    model: [[f32; 4]; 4],
    color: [f32; 4],
    uv: [f32; 4],
}

impl Instance {
    fn new(model: xform::Mat4, color: [f32; 4]) -> Self {
        Self {
            model,
            color,
            uv: [0.0, 0.0, 1.0, 1.0],
        }
    }
}

/// The uniform block, laid out the way the shader declares it.
#[repr(C)]
#[derive(Debug, Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
struct Globals {
    view_proj: xform::Mat4,
    eye: [f32; 4],
    light: [f32; 4],
    dim: [f32; 4],
    tint: [f32; 4],
}

/// A mesh that lives on the device.
struct Batch {
    vertices: wgpu::Buffer,
    indices: wgpu::Buffer,
    /// How many indices to draw.
    count: u32,
}

impl Batch {
    fn new(device: &wgpu::Device, mesh: &Mesh, label: &str) -> Self {
        Self {
            vertices: device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
                label: Some(label),
                contents: bytemuck::cast_slice(&mesh.vertices),
                usage: wgpu::BufferUsages::VERTEX,
            }),
            indices: device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
                label: Some(label),
                contents: bytemuck::cast_slice(&mesh.indices),
                usage: wgpu::BufferUsages::INDEX,
            }),
            count: mesh.indices.len() as u32,
        }
    }

    fn draw(&self, pass: &mut wgpu::RenderPass<'_>, instances: u32) {
        pass.set_vertex_buffer(0, self.vertices.slice(..));
        pass.set_index_buffer(self.indices.slice(..), wgpu::IndexFormat::Uint32);
        pass.draw_indexed(0..self.count, 0, 0..instances);
    }
}

/// An instance list and the buffer it is uploaded into.
struct Batch3d {
    instances: Vec<Instance>,
    buffer: wgpu::Buffer,
    capacity: usize,
}

impl Batch3d {
    fn new(device: &wgpu::Device, capacity: usize, label: &str) -> Self {
        let empty = vec![
            Instance::new(xform::identity(), [0.0; 4]);
            capacity
        ];
        Self {
            instances: Vec::with_capacity(capacity),
            buffer: device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
                label: Some(label),
                contents: bytemuck::cast_slice(&empty),
                usage: wgpu::BufferUsages::VERTEX | wgpu::BufferUsages::COPY_DST,
            }),
            capacity,
        }
    }

    fn begin(&mut self) {
        self.instances.clear();
    }

    fn upload(&mut self, device: &wgpu::Device, queue: &wgpu::Queue, label: &str) {
        if self.instances.len() > self.capacity {
            self.capacity = self.instances.len().next_power_of_two();
            self.buffer = device.create_buffer(&wgpu::BufferDescriptor {
                label: Some(label),
                size: (self.capacity * std::mem::size_of::<Instance>()) as u64,
                usage: wgpu::BufferUsages::VERTEX | wgpu::BufferUsages::COPY_DST,
                mapped_at_creation: false,
            });
        }
        if !self.instances.is_empty() {
            queue.write_buffer(&self.buffer, 0, bytemuck::cast_slice(&self.instances));
        }
    }

    fn draw(&self, pass: &mut wgpu::RenderPass<'_>, batch: &Batch, pipeline: &wgpu::RenderPipeline, bind_group: &wgpu::BindGroup) {
        if self.instances.is_empty() {
            return;
        }
        pass.set_pipeline(pipeline);
        pass.set_bind_group(0, bind_group, &[]);
        pass.set_vertex_buffer(1, self.buffer.slice(..));
        batch.draw(pass, self.instances.len() as u32);
    }
}

/// The atlas as a texture: one coverage value per pixel, uploaded whole.
fn upload_atlas(device: &wgpu::Device, queue: &wgpu::Queue, atlas: &GlyphAtlas) -> wgpu::Texture {
    let texture = device.create_texture(&wgpu::TextureDescriptor {
        label: Some("playground glyphs"),
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

/// Everything the playground draws with.
pub struct Scene {
    glyph_pipeline: wgpu::RenderPipeline,
    box_pipeline: wgpu::RenderPipeline,
    dot_pipeline: wgpu::RenderPipeline,
    glow_pipeline: wgpu::RenderPipeline,
    bind_group: wgpu::BindGroup,
    /// What the bind group is built from, kept so a new atlas can be given a new
    /// one without rebuilding the pipelines around it.
    layout: wgpu::BindGroupLayout,
    sampler: wgpu::Sampler,
    globals: wgpu::Buffer,
    glyphs: Batch3d,
    boxes: Batch3d,
    dots: Batch3d,
    glows: Batch3d,
    quad: Batch,
    cube: Batch,
    /// The texture coordinates of every atlas cell, by character.
    uvs: Vec<[f32; 4]>,
    depth: wgpu::Texture,
    depth_view: wgpu::TextureView,
}

impl Scene {
    /// Builds the pipelines, the meshes and the atlas texture.
    pub fn new(
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        format: wgpu::TextureFormat,
        atlas: &GlyphAtlas,
    ) -> Self {
        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("playground shader"),
            source: wgpu::ShaderSource::Wgsl(SHADER.into()),
        });

        let globals = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("playground globals"),
            size: std::mem::size_of::<Globals>() as u64,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });

        let layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("playground bind group layout"),
            entries: &[
                wgpu::BindGroupLayoutEntry {
                    binding: 0,
                    visibility: wgpu::ShaderStages::VERTEX_FRAGMENT,
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
            label: Some("playground sampler"),
            mag_filter: wgpu::FilterMode::Linear,
            min_filter: wgpu::FilterMode::Linear,
            address_mode_u: wgpu::AddressMode::ClampToEdge,
            address_mode_v: wgpu::AddressMode::ClampToEdge,
            ..Default::default()
        });

        let atlas_texture = upload_atlas(device, queue, atlas);
        let atlas_view = atlas_texture.create_view(&wgpu::TextureViewDescriptor::default());

        let bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("playground bind group"),
            layout: &layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: globals.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: wgpu::BindingResource::TextureView(&atlas_view),
                },
                wgpu::BindGroupEntry {
                    binding: 2,
                    resource: wgpu::BindingResource::Sampler(&sampler),
                },
            ],
        });

        let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("playground pipeline layout"),
            bind_group_layouts: &[Some(&layout)],
            immediate_size: 0,
        });

        let vertex_buffers = [
            Some(wgpu::VertexBufferLayout {
                array_stride: Vertex::STRIDE,
                step_mode: wgpu::VertexStepMode::Vertex,
                attributes: &[
                    wgpu::VertexAttribute {
                        offset: 0,
                        shader_location: 0,
                        format: wgpu::VertexFormat::Float32x3,
                    },
                    wgpu::VertexAttribute {
                        offset: 12,
                        shader_location: 1,
                        format: wgpu::VertexFormat::Float32x3,
                    },
                    wgpu::VertexAttribute {
                        offset: 24,
                        shader_location: 2,
                        format: wgpu::VertexFormat::Float32x2,
                    },
                ],
            }),
            Some(wgpu::VertexBufferLayout {
                array_stride: std::mem::size_of::<Instance>() as u64,
                step_mode: wgpu::VertexStepMode::Instance,
                attributes: &[
                    wgpu::VertexAttribute {
                        offset: 0,
                        shader_location: 3,
                        format: wgpu::VertexFormat::Float32x4,
                    },
                    wgpu::VertexAttribute {
                        offset: 16,
                        shader_location: 4,
                        format: wgpu::VertexFormat::Float32x4,
                    },
                    wgpu::VertexAttribute {
                        offset: 32,
                        shader_location: 5,
                        format: wgpu::VertexFormat::Float32x4,
                    },
                    wgpu::VertexAttribute {
                        offset: 48,
                        shader_location: 6,
                        format: wgpu::VertexFormat::Float32x4,
                    },
                    wgpu::VertexAttribute {
                        offset: 64,
                        shader_location: 7,
                        format: wgpu::VertexFormat::Float32x4,
                    },
                    wgpu::VertexAttribute {
                        offset: 80,
                        shader_location: 8,
                        format: wgpu::VertexFormat::Float32x4,
                    },
                ],
            }),
        ];

        let depth_state = |write: bool| {
            Some(wgpu::DepthStencilState {
                format: DEPTH_FORMAT,
                depth_write_enabled: Some(write),
                depth_compare: Some(wgpu::CompareFunction::Less),
                stencil: wgpu::StencilState::default(),
                bias: wgpu::DepthBiasState::default(),
            })
        };

        let pipeline = |label: &str,
                        entry: &str,
                        cull: Option<wgpu::Face>,
                        depth: bool,
                        blend: wgpu::BlendState| {
            device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
                label: Some(label),
                layout: Some(&pipeline_layout),
                vertex: wgpu::VertexState {
                    module: &shader,
                    entry_point: Some("vs_main"),
                    compilation_options: Default::default(),
                    buffers: &vertex_buffers,
                },
                fragment: Some(wgpu::FragmentState {
                    module: &shader,
                    entry_point: Some(entry),
                    compilation_options: Default::default(),
                    targets: &[Some(wgpu::ColorTargetState {
                        format,
                        blend: Some(blend),
                        write_mask: wgpu::ColorWrites::ALL,
                    })],
                }),
                primitive: wgpu::PrimitiveState {
                    cull_mode: cull,
                    ..Default::default()
                },
                depth_stencil: depth_state(depth),
                multisample: wgpu::MultisampleState::default(),
                multiview_mask: None,
                cache: None,
            })
        };

        // Light adds: a spark brightens the page and never draws a dark edge over
        // it, which is the difference between a glow and a sticker.
        let additive = wgpu::BlendState {
            color: wgpu::BlendComponent {
                src_factor: wgpu::BlendFactor::One,
                dst_factor: wgpu::BlendFactor::One,
                operation: wgpu::BlendOperation::Add,
            },
            alpha: wgpu::BlendComponent {
                src_factor: wgpu::BlendFactor::One,
                dst_factor: wgpu::BlendFactor::One,
                operation: wgpu::BlendOperation::Add,
            },
        };

        let box_pipeline = pipeline(
            "playground boxes",
            "fs_box",
            Some(wgpu::Face::Back),
            true,
            wgpu::BlendState::ALPHA_BLENDING,
        );
        let glyph_pipeline = pipeline(
            "playground glyphs",
            "fs_glyph",
            None,
            true,
            wgpu::BlendState::ALPHA_BLENDING,
        );
        let dot_pipeline = pipeline(
            "playground dots",
            "fs_dot",
            None,
            false,
            wgpu::BlendState::ALPHA_BLENDING,
        );
        // The sparks are drawn last, over the page and over the cat: light in front
        // of what it is shining on is what makes a trail read as in the air.
        let glow_pipeline = pipeline("playground glows", "fs_glow", None, false, additive);

        let uvs = (FIRST..=LAST)
            .filter_map(char::from_u32)
            .map(|ch| atlas.uv(ch))
            .collect();

        let (depth, depth_view) = Self::make_depth(device, 1, 1);

        Self {
            glyph_pipeline,
            box_pipeline,
            dot_pipeline,
            glow_pipeline,
            bind_group,
            layout,
            sampler,
            globals,
            glyphs: Batch3d::new(device, MAX_GLYPHS, "playground glyph instances"),
            boxes: Batch3d::new(device, MAX_BOXES, "playground box instances"),
            dots: Batch3d::new(device, MAX_DOTS, "playground dot instances"),
            glows: Batch3d::new(device, MAX_GLOWS, "playground glow instances"),
            quad: Batch::new(device, &Mesh::quad(), "playground quad"),
            cube: Batch::new(
                device,
                &Mesh::rounded_box(BOX_RADIUS, BOX_SEGMENTS),
                "playground box",
            ),
            uvs,
            // The texture itself is kept alive by the bind group that samples it.
            depth,
            depth_view,
        }
    }

    /// Replaces the glyphs.
    ///
    /// The atlas is rasterised at the size the cells are drawn at, so a window
    /// whose scale changed -- a display that reports another density, or the
    /// Window Scale row -- needs its glyphs again or they come out half the size
    /// of the page they are written on. Only the texture and the bind group that
    /// samples it are rebuilt: the pipelines describe the shape of a glyph, not
    /// how big it is.
    pub fn set_atlas(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        atlas: &GlyphAtlas,
    ) {
        let texture = upload_atlas(device, queue, atlas);
        let view = texture.create_view(&wgpu::TextureViewDescriptor::default());
        self.bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("playground bind group"),
            layout: &self.layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: self.globals.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: wgpu::BindingResource::TextureView(&view),
                },
                wgpu::BindGroupEntry {
                    binding: 2,
                    resource: wgpu::BindingResource::Sampler(&self.sampler),
                },
            ],
        });
        self.uvs = (FIRST..=LAST)
            .filter_map(char::from_u32)
            .map(|ch| atlas.uv(ch))
            .collect();
    }

    /// A depth texture of the target's size, and its view.
    fn make_depth(
        device: &wgpu::Device,
        width: u32,
        height: u32,
    ) -> (wgpu::Texture, wgpu::TextureView) {
        let texture = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("playground depth"),
            size: wgpu::Extent3d {
                width: width.max(1),
                height: height.max(1),
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: DEPTH_FORMAT,
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT,
            view_formats: &[],
        });
        let view = texture.create_view(&wgpu::TextureViewDescriptor::default());
        (texture, view)
    }

    /// The depth view the pass attaches, sized to the window.
    pub fn depth_view(&self) -> &wgpu::TextureView {
        &self.depth_view
    }

    /// Turns one frame of the simulation into instances and uploads them.
    #[allow(clippy::too_many_arguments)]
    pub fn prepare(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        frame: &Frame,
        theme: &Theme,
        width: u32,
        height: u32,
    ) {
        self.glyphs.begin();
        self.boxes.begin();
        self.dots.begin();
        self.glows.begin();

        let camera = frame.camera;
        // The page is flat and the camera looks straight at it, so the picture is
        // an orthographic window onto the grid: one cell is one cell wherever it
        // is on screen, and a glyph lands on whole pixels.
        let width_units = camera.width();
        let height_units = camera.height();
        let eye = [width_units * 0.5, height_units * 0.5, CAMERA_Z];
        let look = xform::look_at(eye, [eye[0], eye[1], 0.0], [0.0, 1.0, 0.0]);
        // `look_at` puts the middle of the page at the origin, so the window is
        // centred on it: passing `0..width` here would shift the whole page half a
        // viewport down and to the left.
        let (half_width, half_height) = (width_units * 0.5, height_units * 0.5);
        let projection = xform::ortho(-half_width, half_width, -half_height, half_height, NEAR, FAR);
        let view_proj = xform::mul(projection, look);
        // Everything on the page is drawn through `view_proj`; the kitten is
        // drawn through the same one, sheared, because a box has a side and a top
        // and a page of text does not.
        let sheared = xform::oblique(SHEAR_X, SHEAR_Y);

        let palette = Palette::new(*theme);
        for card in &frame.cards {
            if card.alpha <= 0.01 {
                continue;
            }
            match card.kind {
                CardKind::Glyph => {
                    let colour = palette.color(card.style).to_linear_alpha(card.alpha);
                    let uv = self
                        .uvs
                        .get(GlyphAtlas::index(card.ch) as usize)
                        .copied()
                        .unwrap_or([0.0, 0.0, 1.0, 1.0]);
                    self.glyphs.instances.push(Instance {
                        model: xform::card(
                            card.pos,
                            card.yaw,
                            card.pitch,
                            card.roll,
                            card.size[0],
                            card.size[1],
                        ),
                        color: colour,
                        uv,
                    });
                }
                CardKind::Dot => {
                    let colour = theme.background.to_linear_alpha(card.alpha);
                    self.dots.instances.push(Instance::new(
                        xform::card(
                            card.pos,
                            card.yaw,
                            card.pitch,
                            card.roll,
                            card.size[0],
                            card.size[1],
                        ),
                        colour,
                    ));
                }
            }
        }

        // The page's light: a trail behind a jump, a ring where she lands, the
        // confetti of a paw. One instance each, coloured by hue, added to whatever
        // they land on.
        for glow in &frame.glows {
            if glow.alpha <= 0.01 {
                continue;
            }
            let kind = match glow.kind {
                GlowKind::Blob => 0.0,
                GlowKind::Ring => 1.0,
                GlowKind::Star => 2.0,
            };
            self.glows.instances.push(Instance {
                model: xform::card(
                    glow.pos,
                    0.0,
                    0.0,
                    glow.roll,
                    glow.size * 2.0,
                    glow.size * 2.0,
                ),
                color: [glow.hue, glow.alpha, kind, glow.seed],
                uv: [0.0, 0.0, 1.0, 1.0],
            });
        }

        for block in &frame.blocks {
            let colour = super::shade_colour(block.shade, theme).to_linear();
            self.boxes.instances.push(Instance::new(
                xform::mul(sheared, xform::block(block.pos, block.size, block.yaw)),
                colour,
            ));
        }

        // Back to front, so the blended edges of two cards that overlap in
        // screen space come out in the order the page has them. Depth on a page is
        // `z` and nothing else: the text is all at zero and everything that stands
        // on it is in front.
        let depth = |instance: &Instance| {
            let m = instance.model;
            m[3][2]
        };
        self.glyphs
            .instances
            .sort_by(|a, b| depth(b).total_cmp(&depth(a)));
        self.boxes
            .instances
            .sort_by(|a, b| depth(b).total_cmp(&depth(a)));
        self.dots
            .instances
            .sort_by(|a, b| depth(b).total_cmp(&depth(a)));

        self.glyphs
            .upload(device, queue, "playground glyph instances");
        self.boxes
            .upload(device, queue, "playground box instances");
        self.dots.upload(device, queue, "playground dot instances");
        self.glows
            .upload(device, queue, "playground glow instances");

        let globals = Globals {
            view_proj,
            eye: [eye[0], eye[1], eye[2], 1.0],
            light: [LIGHT[0], LIGHT[1], LIGHT[2], 0.0],
            // The clock rides in `dim.y`: the rainbow drifts with it, so a still
            // frame at the same time is the same picture.
            dim: [frame.dim, frame.time, 0.0, 0.0],
            tint: [
                theme.background.to_linear()[0],
                theme.background.to_linear()[1],
                theme.background.to_linear()[2],
                1.0,
            ],
        };
        queue.write_buffer(&self.globals, 0, bytemuck::cast_slice(&[globals]));

        if self.depth.width() != width.max(1) || self.depth.height() != height.max(1) {
            let (depth, depth_view) = Self::make_depth(device, width, height);
            self.depth = depth;
            self.depth_view = depth_view;
        }

    }

    /// Draws the room into the pass.
    pub fn draw(&self, pass: &mut wgpu::RenderPass<'_>) {
        self.boxes
            .draw(pass, &self.cube, &self.box_pipeline, &self.bind_group);
        self.dots
            .draw(pass, &self.quad, &self.dot_pipeline, &self.bind_group);
        self.glyphs
            .draw(pass, &self.quad, &self.glyph_pipeline, &self.bind_group);
        // Light last, over everything: it is in front of the page it falls on.
        self.glows
            .draw(pass, &self.quad, &self.glow_pipeline, &self.bind_group);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use nn_core::playground::Playground;

    #[test]
    fn an_instance_is_sixteen_floats_of_matrix_and_two_of_colour() {
        assert_eq!(std::mem::size_of::<Instance>(), 96);
    }

    #[test]
    fn the_globals_are_what_the_shader_declares() {
        // One matrix and four vectors: 128 bytes, sixteen of alignment.
        assert_eq!(std::mem::size_of::<Globals>(), 128);
    }

    #[test]
    fn every_card_of_a_room_has_a_cell_in_the_atlas() {
        let mut font_system = crate::fonts::font_system();
        let atlas = GlyphAtlas::build(&mut font_system, 32);
        let uvs: Vec<[f32; 4]> = (FIRST..=LAST)
            .filter_map(char::from_u32)
            .map(|ch| atlas.uv(ch))
            .collect();
        assert_eq!(uvs.len(), 95);
        let playground = Playground::new(0x5EED);
        let frame = playground.frame(&Default::default());
        for card in &frame.cards {
            if card.kind != CardKind::Glyph {
                continue;
            }
            let index = GlyphAtlas::index(card.ch);
            assert!(
                (index as usize) < uvs.len(),
                "{:?} is outside the atlas",
                card.ch
            );
        }
    }

    #[test]
    fn a_room_draws_more_than_one_thing() {
        let playground = Playground::new(0x5EED);
        let frame = playground.frame(&Default::default());
        let glyphs = frame
            .cards
            .iter()
            .filter(|card| card.kind == CardKind::Glyph && card.alpha > 0.01)
            .count();
        let dots = frame
            .cards
            .iter()
            .filter(|card| card.kind == CardKind::Dot && card.alpha > 0.01)
            .count();
        assert!(glyphs > 200, "{glyphs} cards");
        assert!(dots > 8, "{dots} dots");
        assert!(frame.blocks.len() > 24, "{} blocks", frame.blocks.len());
    }
}
