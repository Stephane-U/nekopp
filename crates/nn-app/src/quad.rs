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

//! Instanced rectangle rendering.
//!
//! Everything in the editor that is not a glyph is a rectangle: the caret, the
//! selection, the current-line band, the gutter, and later the fold markers and
//! the minimap. They all go through one instanced draw call, so adding a hundred
//! highlights costs one buffer write and no extra state changes.

use wgpu::util::DeviceExt;

/// One rectangle, in physical pixels, with a linear RGBA colour.
#[repr(C)]
#[derive(Debug, Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
pub struct QuadInstance {
    /// `x`, `y`, `width`, `height` in pixels, origin at the top left.
    pub rect: [f32; 4],
    /// Linear RGBA.
    pub color: [f32; 4],
}

const SHADER: &str = r#"
struct Globals {
    resolution: vec2<f32>,
    _pad: vec2<f32>,
};

@group(0) @binding(0) var<uniform> globals: Globals;

struct Instance {
    @location(0) rect: vec4<f32>,
    @location(1) color: vec4<f32>,
};

struct VsOut {
    @builtin(position) position: vec4<f32>,
    @location(0) color: vec4<f32>,
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
    out.color = inst.color;
    return out;
}

@fragment
fn fs_main(in: VsOut) -> @location(0) vec4<f32> {
    return in.color;
}
"#;

/// Draws batches of coloured rectangles.
pub struct QuadRenderer {
    pipeline: wgpu::RenderPipeline,
    bind_group: wgpu::BindGroup,
    resolution: wgpu::Buffer,
    instances: wgpu::Buffer,
    data: Vec<QuadInstance>,
    capacity: usize,
}

impl QuadRenderer {
    /// Builds the pipeline for a surface of `format`.
    pub fn new(device: &wgpu::Device, format: wgpu::TextureFormat) -> Self {
        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("quad shader"),
            source: wgpu::ShaderSource::Wgsl(SHADER.into()),
        });

        let resolution = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("quad resolution"),
            size: 16,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });

        let bind_group_layout =
            device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
                label: Some("quad bind group layout"),
                entries: &[wgpu::BindGroupLayoutEntry {
                    binding: 0,
                    visibility: wgpu::ShaderStages::VERTEX,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Uniform,
                        has_dynamic_offset: false,
                        min_binding_size: None,
                    },
                    count: None,
                }],
            });

        let bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("quad bind group"),
            layout: &bind_group_layout,
            entries: &[wgpu::BindGroupEntry {
                binding: 0,
                resource: resolution.as_entire_binding(),
            }],
        });

        let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("quad pipeline layout"),
            bind_group_layouts: &[Some(&bind_group_layout)],
            immediate_size: 0,
        });

        let pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("quad pipeline"),
            layout: Some(&pipeline_layout),
            vertex: wgpu::VertexState {
                module: &shader,
                entry_point: Some("vs_main"),
                compilation_options: Default::default(),
                buffers: &[Some(wgpu::VertexBufferLayout {
                    array_stride: std::mem::size_of::<QuadInstance>() as u64,
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

        let capacity = 1024;
        let instances = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("quad instances"),
            contents: bytemuck::cast_slice(&vec![QuadInstance {
                rect: [0.0; 4],
                color: [0.0; 4],
            }; capacity]),
            usage: wgpu::BufferUsages::VERTEX | wgpu::BufferUsages::COPY_DST,
        });

        Self {
            pipeline,
            bind_group,
            resolution,
            instances,
            data: Vec::with_capacity(capacity),
            capacity,
        }
    }

    /// Discards the previous frame's rectangles.
    pub fn begin(&mut self) {
        self.data.clear();
    }

    /// Queues a rectangle. Zero-area rectangles are dropped.
    pub fn push(&mut self, x: f32, y: f32, width: f32, height: f32, color: [f32; 4]) {
        if width <= 0.0 || height <= 0.0 {
            return;
        }
        self.data.push(QuadInstance {
            rect: [x, y, width, height],
            color,
        });
    }

    /// Uploads the queued rectangles and the target resolution.
    pub fn upload(&mut self, device: &wgpu::Device, queue: &wgpu::Queue, width: f32, height: f32) {
        if self.data.len() > self.capacity {
            self.capacity = self.data.len().next_power_of_two();
            self.instances = device.create_buffer(&wgpu::BufferDescriptor {
                label: Some("quad instances"),
                size: (self.capacity * std::mem::size_of::<QuadInstance>()) as u64,
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

    /// Draws every queued rectangle.
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
