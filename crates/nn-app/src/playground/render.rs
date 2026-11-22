//! Drawing the room.
//!
//! The editor draws in two dimensions with one another; this is the only part of
//! the program with a depth buffer. Three pipelines share one shader and one
//! instance layout: boxes for the furniture and the kitten, glyph cards for the
//! walls and the loose tokens, and camera-facing dots for dust and shadows. All
//! three are instanced, so a room of five hundred cards is three draw calls.
//!
//! Two decisions are worth the words. The first is that the cards are lit by the
//! absolute value of the dot product rather than by the plain one: a card is a
//! sheet of paper with a character printed on it, it looks the same from behind,
//! and the alternative — culling back faces and mirroring the walls that need it —
//! is more arithmetic for a worse answer. The second is that the fog colour is the
//! theme's own background: the far wall fades into the window rather than into a
//! grey chosen here, which is what makes the room look like it is in this editor
//! and not pasted onto it.
//!
//! Instances are sorted back to front before they are uploaded. Alpha blending
//! with a depth buffer sorts nothing, so the only thing keeping a token from being
//! painted over the kitten it is in front of is the order the CPU hands them over.

use wgpu::util::DeviceExt;

use nn_core::playground::camera::FOV;
use nn_core::playground::scene::{BACK, HALF_DEPTH, HALF_WIDTH, WALL_HEIGHT};
use nn_core::playground::{CardKind, Frame};

use crate::palette::Palette;
use crate::theme::{mix, Theme};

use super::glyphs::{GlyphAtlas, FIRST, LAST};
use super::mesh::{Mesh, Vertex};
use super::xform;

/// The depth format: the one every backend has, with no stencil to go wrong.
const DEPTH_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Depth32Float;
/// The near plane. Close enough that the camera can sit inside the room's front
/// edge and still see the floor under it.
const NEAR: f32 = 0.4;
/// The far plane: past the back wall by more than a room, so nothing pops.
const FAR: f32 = 64.0;
/// The corner radius of the boxes, in cell units.
const BOX_RADIUS: f32 = 0.2;
/// How many steps each face of a box is divided into.
const BOX_SEGMENTS: u32 = 3;
/// The direction the lamp's light travels: down, and out towards the room.
const LIGHT: [f32; 3] = [0.38, -0.86, 0.34];
/// The most glyph cards, boxes and dots the buffers start big enough for.
const MAX_GLYPHS: usize = 4096;
const MAX_BOXES: usize = 1024;
const MAX_DOTS: usize = 512;

const SHADER: &str = r#"
struct Globals {
    view_proj: mat4x4<f32>,
    eye: vec4<f32>,
    light: vec4<f32>,
    fog: vec4<f32>,
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
    return base * (0.52 + 0.48 * lambert + rim);
}

fn in_fog(colour: vec3<f32>, world: vec3<f32>) -> vec3<f32> {
    let far = globals.fog.y;
    let near = globals.fog.x;
    let distance = length(world - globals.eye.xyz);
    let t = clamp((distance - near) / max(far - near, 0.01), 0.0, 1.0);
    let flying = mix(colour, globals.tint.rgb, t * 0.55);
    return flying * (1.0 - globals.fog.z);
}

@fragment
fn fs_glyph(in: VsOut) -> @location(0) vec4<f32> {
    let coverage = textureSample(glyphs, glyph_sampler, in.uv).r;
    if (coverage < 0.02) {
        discard;
    }
    let colour = in_fog(lit(in.color.rgb, in.world, in.normal, 0.0), in.world);
    return vec4<f32>(colour, in.color.a * coverage);
}

@fragment
fn fs_box(in: VsOut) -> @location(0) vec4<f32> {
    let colour = in_fog(lit(in.color.rgb, in.world, in.normal, 0.22), in.world);
    return vec4<f32>(colour, in.color.a);
}

@fragment
fn fs_dot(in: VsOut) -> @location(0) vec4<f32> {
    let radius = length(in.local * 2.0 - 1.0);
    let falloff = clamp(1.0 - radius, 0.0, 1.0);
    let alpha = falloff * falloff * in.color.a;
    if (alpha < 0.004) {
        discard;
    }
    let colour = in_fog(in.color.rgb, in.world);
    return vec4<f32>(colour, alpha);
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
    fog: [f32; 4],
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

/// Everything the playground draws with.
pub struct Scene {
    glyph_pipeline: wgpu::RenderPipeline,
    box_pipeline: wgpu::RenderPipeline,
    dot_pipeline: wgpu::RenderPipeline,
    bind_group: wgpu::BindGroup,
    globals: wgpu::Buffer,
    glyphs: Batch3d,
    boxes: Batch3d,
    dots: Batch3d,
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

        let atlas_texture = device.create_texture(&wgpu::TextureDescriptor {
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
                texture: &atlas_texture,
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

        let pipeline = |label: &str, entry: &str, cull: Option<wgpu::Face>, depth: bool| {
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
                        blend: Some(wgpu::BlendState::ALPHA_BLENDING),
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

        let box_pipeline = pipeline("playground boxes", "fs_box", Some(wgpu::Face::Back), true);
        let glyph_pipeline = pipeline("playground glyphs", "fs_glyph", None, true);
        let dot_pipeline = pipeline("playground dots", "fs_dot", None, false);

        let uvs = (FIRST..=LAST)
            .filter_map(char::from_u32)
            .map(|ch| atlas.uv(ch))
            .collect();

        let (depth, depth_view) = Self::make_depth(device, 1, 1);

        Self {
            glyph_pipeline,
            box_pipeline,
            dot_pipeline,
            bind_group,
            globals,
            glyphs: Batch3d::new(device, MAX_GLYPHS, "playground glyph instances"),
            boxes: Batch3d::new(device, MAX_BOXES, "playground box instances"),
            dots: Batch3d::new(device, MAX_DOTS, "playground dot instances"),
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

        let camera = frame.camera;
        let aspect = width as f32 / height.max(1) as f32;
        let view = xform::look_at(camera.eye, camera.target, [0.0, 1.0, 0.0]);
        let projection = xform::perspective(FOV, aspect, NEAR, FAR);
        let view_proj = xform::mul(projection, view);

        // The room itself: a floor and three walls, so the cards stand on
        // something and the light has a surface to fall on.
        let floor = mix(theme.background, theme.foreground, 0.09).to_linear();
        let wall = mix(theme.background, theme.foreground, 0.05).to_linear();
        let reach = HALF_WIDTH + 0.2;
        self.boxes.instances.push(Instance::new(
            xform::block([0.0, -0.06, 0.0], [2.0 * reach, 0.12, 2.0 * HALF_DEPTH], 0.0),
            floor,
        ));
        self.boxes.instances.push(Instance::new(
            xform::block(
                [0.0, WALL_HEIGHT * 0.5, BACK - 0.08],
                [2.0 * reach, WALL_HEIGHT, 0.16],
                0.0,
            ),
            wall,
        ));
        for side in [-1.0f32, 1.0] {
            self.boxes.instances.push(Instance::new(
                xform::block(
                    [side * (HALF_WIDTH + 0.08), WALL_HEIGHT * 0.5, 0.0],
                    [0.16, WALL_HEIGHT, 2.0 * HALF_DEPTH],
                    0.0,
                ),
                wall,
            ));
        }

        let palette = Palette::new(*theme);
        for card in &frame.cards {
            if card.alpha <= 0.01 {
                continue;
            }
            match card.kind {
                CardKind::Glyph => {
                    let colour = palette
                        .color(card.style)
                        .to_linear_alpha(card.alpha);
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
                            super::GLYPH_WIDTH * card.scale,
                            card.scale,
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
                            card.scale,
                            card.scale,
                        ),
                        colour,
                    ));
                }
            }
        }

        for block in &frame.blocks {
            let colour = super::shade_colour(block.shade, theme).to_linear();
            self.boxes.instances.push(Instance::new(
                xform::block(block.pos, block.size, block.yaw),
                colour,
            ));
        }

        // Back to front, so the blended edges of two cards that overlap in
        // screen space come out in the order the room has them.
        let depth = |instance: &Instance| {
            let m = instance.model;
            let pos = [m[3][0], m[3][1], m[3][2]];
            camera.depth(pos)
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

        let eye = camera.eye;
        let globals = Globals {
            view_proj,
            eye: [eye[0], eye[1], eye[2], 1.0],
            light: [LIGHT[0], LIGHT[1], LIGHT[2], 0.0],
            fog: [6.0, 26.0, frame.dim, 0.0],
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
