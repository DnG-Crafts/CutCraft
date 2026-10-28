//! Keeping the canvas picture on the graphics card. Handing egui a big mesh
//! means egui copies it and sends it to the graphics card again every frame
//! (tens of milliseconds for a large trace). Instead each mesh of the
//! picture is uploaded once, the first time it's drawn, and each frame only
//! a small paint callback says where to draw it - so panning, hovering and
//! redrawing cost next to nothing.
//!
//! Works with both of eframe's renderers: wgpu (DirectX, Metal, Vulkan) and
//! OpenGL. Buffers not drawn for a few frames are freed.

use egui::Mesh;
use std::collections::HashMap;
use std::sync::{Arc, Mutex};

/// Which renderer is drawing the window.
pub enum Gpu {
    Wgpu,
    Glow(Arc<Mutex<GlScene>>),
    /// Neither (draw through egui's meshes).
    None,
}

impl Gpu {
    /// Sets up for whichever renderer eframe started with.
    pub fn new(cc: &eframe::CreationContext<'_>) -> Gpu {
        if let Some(rs) = &cc.wgpu_render_state {
            let res = WgpuScene::new(&rs.device, rs.target_format);
            rs.renderer.write().callback_resources.insert(res);
            return Gpu::Wgpu;
        }
        if cc.gl.is_some() {
            return Gpu::Glow(Arc::new(Mutex::new(GlScene::default())));
        }
        Gpu::None
    }

    pub fn available(&self) -> bool {
        !matches!(self, Gpu::None)
    }

    /// A shape that draws mesh `id` (uploading it the first time), moved by
    /// `shift` points, inside `rect` (the canvas). `frame` counts up each
    /// time the canvas is drawn.
    pub fn draw(&self, id: u64, mesh: &Arc<Mesh>, shift: egui::Vec2, rect: egui::Rect, frame: u64) -> egui::Shape {
        // canvas points → the callback's own coordinates
        let uniform = [1.0, 1.0, shift.x - rect.min.x, shift.y - rect.min.y, rect.width(), rect.height(), 0.0, 0.0];
        let draw = Draw { id, mesh: mesh.clone(), uniform, frame };
        match self {
            Gpu::Wgpu => egui::Shape::Callback(eframe::egui_wgpu::Callback::new_paint_callback(rect, draw)),
            Gpu::Glow(scene) => {
                let scene = scene.clone();
                let cb = eframe::egui_glow::CallbackFn::new(move |_info, painter| {
                    if let Ok(mut s) = scene.lock() {
                        s.paint(painter.gl(), &draw);
                    }
                });
                egui::Shape::Callback(egui::PaintCallback { rect, callback: Arc::new(cb) })
            }
            Gpu::None => egui::Shape::Noop,
        }
    }
}

/// One mesh to draw this frame.
struct Draw {
    id: u64,
    mesh: Arc<Mesh>,
    /// [scale x, scale y, add x, add y, width, height, linear output, 0]
    uniform: [f32; 8],
    frame: u64,
}

/// How many frames an unused mesh stays on the graphics card.
const KEEP_FRAMES: u64 = 3;

// ---- wgpu ----

use eframe::egui_wgpu::wgpu;

const WGSL: &str = r#"
struct U { a: vec4<f32>, b: vec4<f32> };
@group(0) @binding(0) var<uniform> u: U;
struct Out { @builtin(position) pos: vec4<f32>, @location(0) color: vec4<f32> };

@vertex
fn vs_main(@location(0) p: vec2<f32>, @location(1) c: vec4<f32>) -> Out {
    let q = p * u.a.xy + u.a.zw;
    var o: Out;
    o.pos = vec4<f32>(2.0 * q.x / u.b.x - 1.0, 1.0 - 2.0 * q.y / u.b.y, 0.0, 1.0);
    o.color = c;
    return o;
}

fn linear_from_gamma(s: vec3<f32>) -> vec3<f32> {
    let lower = s / vec3<f32>(12.92);
    let higher = pow((s + vec3<f32>(0.055)) / vec3<f32>(1.055), vec3<f32>(2.4));
    return select(higher, lower, s < vec3<f32>(0.04045));
}

@fragment
fn fs_main(i: Out) -> @location(0) vec4<f32> {
    if (u.b.z > 0.5) {
        return vec4<f32>(linear_from_gamma(i.color.rgb), i.color.a);
    }
    return i.color;
}
"#;

struct WgpuMesh {
    vertices: wgpu::Buffer,
    indices: wgpu::Buffer,
    count: u32,
    uniform: wgpu::Buffer,
    bind: wgpu::BindGroup,
    last: u64,
}

struct WgpuScene {
    pipeline: wgpu::RenderPipeline,
    layout: wgpu::BindGroupLayout,
    linear: bool,
    meshes: HashMap<u64, WgpuMesh>,
}

impl WgpuScene {
    fn new(device: &wgpu::Device, format: wgpu::TextureFormat) -> WgpuScene {
        let module = device.create_shader_module(wgpu::ShaderModuleDescriptor { label: Some("cutcraft_scene"), source: wgpu::ShaderSource::Wgsl(WGSL.into()) });
        let layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("cutcraft_scene_uniform"),
            entries: &[wgpu::BindGroupLayoutEntry {
                binding: 0,
                visibility: wgpu::ShaderStages::VERTEX | wgpu::ShaderStages::FRAGMENT,
                ty: wgpu::BindingType::Buffer { ty: wgpu::BufferBindingType::Uniform, has_dynamic_offset: false, min_binding_size: None },
                count: None,
            }],
        });
        let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor { label: Some("cutcraft_scene"), bind_group_layouts: &[Some(&layout)], immediate_size: 0 });
        let pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("cutcraft_scene"),
            layout: Some(&pipeline_layout),
            vertex: wgpu::VertexState {
                module: &module,
                entry_point: Some("vs_main"),
                // egui's vertex: position, texture coordinates (unused), colour
                buffers: &[Some(wgpu::VertexBufferLayout {
                    array_stride: 5 * 4,
                    step_mode: wgpu::VertexStepMode::Vertex,
                    attributes: &[
                        wgpu::VertexAttribute { format: wgpu::VertexFormat::Float32x2, offset: 0, shader_location: 0 },
                        wgpu::VertexAttribute { format: wgpu::VertexFormat::Unorm8x4, offset: 16, shader_location: 1 },
                    ],
                })],
                compilation_options: wgpu::PipelineCompilationOptions::default(),
            },
            primitive: wgpu::PrimitiveState { topology: wgpu::PrimitiveTopology::TriangleList, cull_mode: None, ..Default::default() },
            depth_stencil: None,
            multisample: wgpu::MultisampleState { count: 1, mask: !0, alpha_to_coverage_enabled: false },
            // the same blending as egui (premultiplied alpha)
            fragment: Some(wgpu::FragmentState {
                module: &module,
                entry_point: Some("fs_main"),
                targets: &[Some(wgpu::ColorTargetState {
                    format,
                    blend: Some(wgpu::BlendState {
                        color: wgpu::BlendComponent { src_factor: wgpu::BlendFactor::One, dst_factor: wgpu::BlendFactor::OneMinusSrcAlpha, operation: wgpu::BlendOperation::Add },
                        alpha: wgpu::BlendComponent { src_factor: wgpu::BlendFactor::OneMinusDstAlpha, dst_factor: wgpu::BlendFactor::One, operation: wgpu::BlendOperation::Add },
                    }),
                    write_mask: wgpu::ColorWrites::ALL,
                })],
                compilation_options: wgpu::PipelineCompilationOptions::default(),
            }),
            multiview_mask: None,
            cache: None,
        });
        WgpuScene { pipeline, layout, linear: format.is_srgb(), meshes: HashMap::new() }
    }
}

impl eframe::egui_wgpu::CallbackTrait for Draw {
    fn prepare(
        &self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        _screen: &eframe::egui_wgpu::ScreenDescriptor,
        _encoder: &mut wgpu::CommandEncoder,
        resources: &mut eframe::egui_wgpu::CallbackResources,
    ) -> Vec<wgpu::CommandBuffer> {
        use wgpu::util::DeviceExt;
        let Some(scene) = resources.get_mut::<WgpuScene>() else { return vec![] };
        scene.meshes.retain(|_, m| m.last + KEEP_FRAMES >= self.frame);
        let layout = &scene.layout;
        let m = scene.meshes.entry(self.id).or_insert_with(|| {
            let vertices = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
                label: Some("cutcraft_scene_vertices"),
                contents: bytemuck::cast_slice(&self.mesh.vertices),
                usage: wgpu::BufferUsages::VERTEX,
            });
            let indices = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
                label: Some("cutcraft_scene_indices"),
                contents: bytemuck::cast_slice(&self.mesh.indices),
                usage: wgpu::BufferUsages::INDEX,
            });
            let uniform = device.create_buffer(&wgpu::BufferDescriptor {
                label: Some("cutcraft_scene_uniform"),
                size: 32,
                usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
                mapped_at_creation: false,
            });
            let bind = device.create_bind_group(&wgpu::BindGroupDescriptor {
                label: Some("cutcraft_scene_uniform"),
                layout,
                entries: &[wgpu::BindGroupEntry { binding: 0, resource: uniform.as_entire_binding() }],
            });
            WgpuMesh { vertices, indices, count: self.mesh.indices.len() as u32, uniform, bind, last: self.frame }
        });
        m.last = self.frame;
        let mut u = self.uniform;
        u[6] = if scene.linear { 1.0 } else { 0.0 };
        queue.write_buffer(&m.uniform, 0, bytemuck::cast_slice(&u));
        vec![]
    }

    fn paint(&self, _info: egui::PaintCallbackInfo, pass: &mut wgpu::RenderPass<'static>, resources: &eframe::egui_wgpu::CallbackResources) {
        let Some(scene) = resources.get::<WgpuScene>() else { return };
        let Some(m) = scene.meshes.get(&self.id) else { return };
        pass.set_pipeline(&scene.pipeline);
        pass.set_bind_group(0, &m.bind, &[]);
        pass.set_vertex_buffer(0, m.vertices.slice(..));
        pass.set_index_buffer(m.indices.slice(..), wgpu::IndexFormat::Uint32);
        pass.draw_indexed(0..m.count, 0, 0..1);
    }
}

// ---- OpenGL ----

use eframe::glow::{self, HasContext};

struct GlMesh {
    vertices: glow::Buffer,
    indices: glow::Buffer,
    vao: Option<glow::VertexArray>,
    count: i32,
    last: u64,
}

struct GlProgram {
    program: glow::Program,
    pos: u32,
    color: u32,
    a: Option<glow::UniformLocation>,
    b: Option<glow::UniformLocation>,
}

#[derive(Default)]
pub struct GlScene {
    program: Option<GlProgram>,
    /// The shaders didn't build: draw nothing rather than retry every frame.
    failed: bool,
    meshes: HashMap<u64, GlMesh>,
}

const GL_VERTEX: &str = r#"
#ifdef NEW_SHADER_INTERFACE
#define I in
#define O out
#else
#define I attribute
#define O varying
#endif
#ifdef GL_ES
precision highp float;
#endif
uniform vec4 u_a;
uniform vec4 u_b;
I vec2 a_pos;
I vec4 a_color;
O vec4 v_color;
void main() {
    vec2 q = a_pos * u_a.xy + u_a.zw;
    gl_Position = vec4(2.0 * q.x / u_b.x - 1.0, 1.0 - 2.0 * q.y / u_b.y, 0.0, 1.0);
    v_color = a_color;
}
"#;

const GL_FRAGMENT: &str = r#"
#ifdef GL_ES
precision mediump float;
#endif
#ifdef NEW_SHADER_INTERFACE
in vec4 v_color;
out vec4 f_color;
#define FRAG f_color
#else
varying vec4 v_color;
#define FRAG gl_FragColor
#endif
void main() {
    FRAG = v_color;
}
"#;

impl GlScene {
    /// SAFETY (for the unsafe GL calls below): called from egui_glow's paint
    /// callback, on the thread that owns `gl`, which egui_glow restores
    /// afterwards.
    fn build_program(gl: &glow::Context) -> Option<GlProgram> {
        let version = eframe::egui_glow::ShaderVersion::get(gl);
        let header = format!("{}{}", version.version_declaration(), if version.is_new_shader_interface() { "#define NEW_SHADER_INTERFACE\n" } else { "" });
        unsafe {
            let program = gl.create_program().ok()?;
            let mut shaders = vec![];
            for (kind, src) in [(glow::VERTEX_SHADER, GL_VERTEX), (glow::FRAGMENT_SHADER, GL_FRAGMENT)] {
                let s = gl.create_shader(kind).ok()?;
                gl.shader_source(s, &format!("{header}{src}"));
                gl.compile_shader(s);
                if !gl.get_shader_compile_status(s) {
                    eprintln!("canvas shader: {}", gl.get_shader_info_log(s));
                    return None;
                }
                gl.attach_shader(program, s);
                shaders.push(s);
            }
            gl.link_program(program);
            for s in shaders {
                gl.detach_shader(program, s);
                gl.delete_shader(s);
            }
            if !gl.get_program_link_status(program) {
                eprintln!("canvas shader: {}", gl.get_program_info_log(program));
                return None;
            }
            Some(GlProgram {
                pos: gl.get_attrib_location(program, "a_pos")?,
                color: gl.get_attrib_location(program, "a_color")?,
                a: gl.get_uniform_location(program, "u_a"),
                b: gl.get_uniform_location(program, "u_b"),
                program,
            })
        }
    }

    fn paint(&mut self, gl: &glow::Context, d: &Draw) {
        if self.program.is_none() && !self.failed {
            self.program = Self::build_program(gl);
            self.failed = self.program.is_none();
        }
        let Some(p) = &self.program else { return };
        // SAFETY: see build_program.
        unsafe {
            self.meshes.retain(|_, m| {
                let keep = m.last + KEEP_FRAMES >= d.frame;
                if !keep {
                    gl.delete_buffer(m.vertices);
                    gl.delete_buffer(m.indices);
                    if let Some(v) = m.vao {
                        gl.delete_vertex_array(v);
                    }
                }
                keep
            });
            let bind_attributes = |gl: &glow::Context| {
                gl.enable_vertex_attrib_array(p.pos);
                gl.vertex_attrib_pointer_f32(p.pos, 2, glow::FLOAT, false, 20, 0);
                gl.enable_vertex_attrib_array(p.color);
                gl.vertex_attrib_pointer_f32(p.color, 4, glow::UNSIGNED_BYTE, true, 20, 16);
            };
            if !self.meshes.contains_key(&d.id) {
                let (Ok(vertices), Ok(indices)) = (gl.create_buffer(), gl.create_buffer()) else { return };
                let vao = gl.create_vertex_array().ok();
                if let Some(v) = vao {
                    gl.bind_vertex_array(Some(v));
                }
                gl.bind_buffer(glow::ARRAY_BUFFER, Some(vertices));
                gl.buffer_data_u8_slice(glow::ARRAY_BUFFER, bytemuck::cast_slice(&d.mesh.vertices), glow::STATIC_DRAW);
                gl.bind_buffer(glow::ELEMENT_ARRAY_BUFFER, Some(indices));
                gl.buffer_data_u8_slice(glow::ELEMENT_ARRAY_BUFFER, bytemuck::cast_slice(&d.mesh.indices), glow::STATIC_DRAW);
                if vao.is_some() {
                    bind_attributes(gl);
                }
                self.meshes.insert(d.id, GlMesh { vertices, indices, vao, count: d.mesh.indices.len() as i32, last: d.frame });
            }
            let Some(m) = self.meshes.get_mut(&d.id) else { return };
            m.last = d.frame;
            gl.use_program(Some(p.program));
            gl.uniform_4_f32(p.a.as_ref(), d.uniform[0], d.uniform[1], d.uniform[2], d.uniform[3]);
            gl.uniform_4_f32(p.b.as_ref(), d.uniform[4], d.uniform[5], 0.0, 0.0);
            match m.vao {
                Some(v) => gl.bind_vertex_array(Some(v)),
                None => {
                    gl.bind_buffer(glow::ARRAY_BUFFER, Some(m.vertices));
                    gl.bind_buffer(glow::ELEMENT_ARRAY_BUFFER, Some(m.indices));
                    bind_attributes(gl);
                }
            }
            gl.draw_elements(glow::TRIANGLES, m.count, glow::UNSIGNED_INT, 0);
            if m.vao.is_some() {
                gl.bind_vertex_array(None);
            }
        }
    }
}
