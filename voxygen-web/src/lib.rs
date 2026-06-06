#![deny(unsafe_code)]

mod world_preview;

use wasm_bindgen::{JsCast, prelude::*};
use web_sys::{Document, HtmlCanvasElement, Window};
use world_preview::{FLOATS_PER_VERTEX, OriginalWorldMesh};

const CANVAS_ID: &str = "voxworld-canvas";
const DETAIL_ID: &str = "voxworld-detail";
const STATUS_ID: &str = "voxworld-status";
const DEPTH_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Depth24Plus;
const TERRAIN_SHADER: &str = r#"
struct Camera {
    view_proj: mat4x4<f32>,
};

struct VertexIn {
    @location(0) position: vec3<f32>,
    @location(1) color: vec3<f32>,
};

struct VertexOut {
    @builtin(position) position: vec4<f32>,
    @location(0) color: vec3<f32>,
};

@group(0) @binding(0)
var<uniform> camera: Camera;

@vertex
fn vs_main(in: VertexIn) -> VertexOut {
    var out: VertexOut;
    out.position = camera.view_proj * vec4<f32>(in.position, 1.0);
    out.color = in.color;
    return out;
}

@fragment
fn fs_main(in: VertexOut) -> @location(0) vec4<f32> {
    let light = vec3<f32>(0.10, 0.11, 0.13);
    return vec4<f32>(in.color + light, 1.0);
}
"#;

#[wasm_bindgen(start)]
pub fn start() {
    console_error_panic_hook::set_once();
    set_status(
        "Starting original Voxygen web scene...",
        StatusState::Loading,
    );
    set_detail("Generating original WorldSim terrain chunks...");

    wasm_bindgen_futures::spawn_local(async {
        match VoxygenWebClient::new().await {
            Ok(client) => {
                client.render_frame();
                set_status("Original WorldSim terrain is rendering.", StatusState::Ok);
                set_detail(&client.scene_summary());
                client.leak_for_browser_lifetime();
            },
            Err(error) => {
                set_status("Voxygen web scene failed.", StatusState::Error);
                set_detail(&error);
            },
        }
    });
}

struct VoxygenWebClient {
    canvas: HtmlCanvasElement,
    surface: wgpu::Surface<'static>,
    device: wgpu::Device,
    queue: wgpu::Queue,
    config: wgpu::SurfaceConfiguration,
    depth_view: wgpu::TextureView,
    terrain_pipeline: wgpu::RenderPipeline,
    terrain_vertex_buffer: wgpu::Buffer,
    terrain_index_buffer: wgpu::Buffer,
    terrain_index_count: u32,
    camera_buffer: wgpu::Buffer,
    camera_bind_group: wgpu::BindGroup,
    world_mesh: OriginalWorldMesh,
}

impl VoxygenWebClient {
    async fn new() -> Result<Self, String> {
        let world_mesh = world_preview::build_original_world_mesh()?;
        set_detail("Original WorldSim mesh generated. Requesting browser WebGPU adapter...");

        let window = web_window()?;
        let document = web_document(&window)?;
        let canvas = canvas_element(&document)?;
        let (width, height) = resize_canvas_to_window(&canvas, &window)?;

        let instance = wgpu::Instance::new(&wgpu::InstanceDescriptor {
            backends: wgpu::Backends::BROWSER_WEBGPU,
            ..Default::default()
        });
        let surface = instance
            .create_surface(wgpu::SurfaceTarget::Canvas(canvas.clone()))
            .map_err(|error| format!("failed to create WebGPU surface: {error}"))?;
        let adapter = instance
            .request_adapter(&wgpu::RequestAdapterOptions {
                power_preference: wgpu::PowerPreference::HighPerformance,
                compatible_surface: Some(&surface),
                force_fallback_adapter: false,
            })
            .await
            .map_err(|error| format!("failed to request WebGPU adapter: {error}"))?;
        let (device, queue) = adapter
            .request_device(&wgpu::DeviceDescriptor {
                label: Some("Voxygen Web Device"),
                required_features: wgpu::Features::empty(),
                required_limits: wgpu::Limits::default(),
                ..Default::default()
            })
            .await
            .map_err(|error| format!("failed to request WebGPU device: {error}"))?;
        let config = surface
            .get_default_config(&adapter, width, height)
            .ok_or_else(|| "WebGPU surface has no compatible default config".to_owned())?;
        surface.configure(&device, &config);

        let depth_view = create_depth_view(&device, &config);
        let camera_matrix =
            camera_view_projection(width as f32 / height.max(1) as f32, &world_mesh);
        let camera_buffer = create_buffer_with_data(
            &device,
            &matrix_bytes(&camera_matrix),
            wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            "Voxygen Web Camera Buffer",
        );
        let camera_bind_group_layout =
            device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
                label: Some("Voxygen Web Camera Bind Group Layout"),
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
        let camera_bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("Voxygen Web Camera Bind Group"),
            layout: &camera_bind_group_layout,
            entries: &[wgpu::BindGroupEntry {
                binding: 0,
                resource: camera_buffer.as_entire_binding(),
            }],
        });
        let terrain_pipeline =
            create_terrain_pipeline(&device, config.format, &camera_bind_group_layout);
        let terrain_vertex_buffer = create_buffer_with_data(
            &device,
            &f32_bytes(&world_mesh.vertices),
            wgpu::BufferUsages::VERTEX | wgpu::BufferUsages::COPY_DST,
            "Voxygen Web Terrain Vertex Buffer",
        );
        let terrain_index_buffer = create_buffer_with_data(
            &device,
            &u32_bytes(&world_mesh.indices),
            wgpu::BufferUsages::INDEX | wgpu::BufferUsages::COPY_DST,
            "Voxygen Web Terrain Index Buffer",
        );
        let terrain_index_count = world_mesh
            .indices
            .len()
            .try_into()
            .map_err(|_| "terrain index count overflowed u32".to_owned())?;

        Ok(Self {
            canvas,
            surface,
            device,
            queue,
            config,
            depth_view,
            terrain_pipeline,
            terrain_vertex_buffer,
            terrain_index_buffer,
            terrain_index_count,
            camera_buffer,
            camera_bind_group,
            world_mesh,
        })
    }

    fn render_frame(&self) {
        let Ok(frame) = self.surface.get_current_texture() else {
            return;
        };
        let view = frame
            .texture
            .create_view(&wgpu::TextureViewDescriptor::default());
        let mut encoder = self
            .device
            .create_command_encoder(&wgpu::CommandEncoderDescriptor {
                label: Some("Voxygen Web Terrain Frame"),
            });

        {
            let mut render_pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("Voxygen Web Terrain Pass"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: &view,
                    resolve_target: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Clear(wgpu::Color {
                            r: 0.045,
                            g: 0.075,
                            b: 0.105,
                            a: 1.0,
                        }),
                        store: wgpu::StoreOp::Store,
                    },
                    depth_slice: None,
                })],
                depth_stencil_attachment: Some(wgpu::RenderPassDepthStencilAttachment {
                    view: &self.depth_view,
                    depth_ops: Some(wgpu::Operations {
                        load: wgpu::LoadOp::Clear(1.0),
                        store: wgpu::StoreOp::Store,
                    }),
                    stencil_ops: None,
                }),
                timestamp_writes: None,
                occlusion_query_set: None,
            });
            render_pass.set_pipeline(&self.terrain_pipeline);
            render_pass.set_bind_group(0, &self.camera_bind_group, &[]);
            render_pass.set_vertex_buffer(0, self.terrain_vertex_buffer.slice(..));
            render_pass.set_index_buffer(
                self.terrain_index_buffer.slice(..),
                wgpu::IndexFormat::Uint32,
            );
            render_pass.draw_indexed(0..self.terrain_index_count, 0, 0..1);
        }

        self.queue.submit(Some(encoder.finish()));
        frame.present();
    }

    fn scene_summary(&self) -> String {
        let (chunks_x, chunks_y) = self.world_mesh.chunk_dimensions;
        let (patch_x, patch_y) = self.world_mesh.chunk_patch;
        format!(
            "Seed {} generated {} original TerrainChunks in a {}x{} patch around {:?} inside a \
             {}x{} WorldSim. WebGPU block faces: {}. Filled blocks: {}. Liquid blocks: {}. Entity \
             spawns: {}. World features loaded: {}. Wildlife spawn manifests: {}.",
            self.world_mesh.seed,
            self.world_mesh.generated_chunks,
            patch_x,
            patch_y,
            self.world_mesh.center_chunk_pos,
            chunks_x,
            chunks_y,
            self.world_mesh.terrain_faces,
            self.world_mesh.filled_blocks,
            self.world_mesh.liquid_blocks,
            self.world_mesh.generated_entity_spawns,
            self.world_mesh.enabled_world_features,
            self.world_mesh.wildlife_spawn_manifests
        )
    }

    fn leak_for_browser_lifetime(self) {
        let _ = self.canvas;
        let _ = self.config;
        let _ = self.camera_buffer;
        Box::leak(Box::new(self));
    }
}

fn create_terrain_pipeline(
    device: &wgpu::Device,
    format: wgpu::TextureFormat,
    camera_bind_group_layout: &wgpu::BindGroupLayout,
) -> wgpu::RenderPipeline {
    let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
        label: Some("Voxygen Web Terrain Shader"),
        source: wgpu::ShaderSource::Wgsl(TERRAIN_SHADER.into()),
    });
    let layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
        label: Some("Voxygen Web Terrain Pipeline Layout"),
        bind_group_layouts: &[camera_bind_group_layout],
        push_constant_ranges: &[],
    });

    device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
        label: Some("Voxygen Web Terrain Pipeline"),
        layout: Some(&layout),
        vertex: wgpu::VertexState {
            module: &shader,
            entry_point: Some("vs_main"),
            buffers: &[wgpu::VertexBufferLayout {
                array_stride: (FLOATS_PER_VERTEX * size_of::<f32>()) as u64,
                step_mode: wgpu::VertexStepMode::Vertex,
                attributes: &[
                    wgpu::VertexAttribute {
                        format: wgpu::VertexFormat::Float32x3,
                        offset: 0,
                        shader_location: 0,
                    },
                    wgpu::VertexAttribute {
                        format: wgpu::VertexFormat::Float32x3,
                        offset: (3 * size_of::<f32>()) as u64,
                        shader_location: 1,
                    },
                ],
            }],
            compilation_options: Default::default(),
        },
        primitive: wgpu::PrimitiveState {
            topology: wgpu::PrimitiveTopology::TriangleList,
            cull_mode: None,
            ..Default::default()
        },
        depth_stencil: Some(wgpu::DepthStencilState {
            format: DEPTH_FORMAT,
            depth_write_enabled: true,
            depth_compare: wgpu::CompareFunction::LessEqual,
            stencil: wgpu::StencilState::default(),
            bias: wgpu::DepthBiasState::default(),
        }),
        multisample: wgpu::MultisampleState::default(),
        fragment: Some(wgpu::FragmentState {
            module: &shader,
            entry_point: Some("fs_main"),
            targets: &[Some(wgpu::ColorTargetState {
                format,
                blend: Some(wgpu::BlendState::REPLACE),
                write_mask: wgpu::ColorWrites::ALL,
            })],
            compilation_options: Default::default(),
        }),
        multiview: None,
        cache: None,
    })
}

fn create_depth_view(
    device: &wgpu::Device,
    config: &wgpu::SurfaceConfiguration,
) -> wgpu::TextureView {
    let texture = device.create_texture(&wgpu::TextureDescriptor {
        label: Some("Voxygen Web Depth Texture"),
        size: wgpu::Extent3d {
            width: config.width,
            height: config.height,
            depth_or_array_layers: 1,
        },
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format: DEPTH_FORMAT,
        usage: wgpu::TextureUsages::RENDER_ATTACHMENT,
        view_formats: &[],
    });
    texture.create_view(&wgpu::TextureViewDescriptor::default())
}

fn create_buffer_with_data(
    device: &wgpu::Device,
    bytes: &[u8],
    usage: wgpu::BufferUsages,
    label: &'static str,
) -> wgpu::Buffer {
    let buffer = device.create_buffer(&wgpu::BufferDescriptor {
        label: Some(label),
        size: bytes.len().max(4) as u64,
        usage,
        mapped_at_creation: true,
    });
    {
        let mut view = buffer.slice(..).get_mapped_range_mut();
        view[..bytes.len()].copy_from_slice(bytes);
    }
    buffer.unmap();
    buffer
}

fn camera_view_projection(aspect: f32, world_mesh: &OriginalWorldMesh) -> [f32; 16] {
    let patch_width = world_mesh.chunk_patch.0.max(world_mesh.chunk_patch.1) as f32;
    let eye_distance = (patch_width * 28.0).max(42.0);
    let eye = [
        eye_distance * 0.72,
        eye_distance * 0.46,
        eye_distance * 0.86,
    ];
    let target = [0.0, 4.0, 0.0];
    let up = [0.0, 1.0, 0.0];
    let view = look_at_rh(eye, target, up);
    let projection = perspective_rh(50.0_f32.to_radians(), aspect.max(0.1), 0.1, 360.0);
    mul_mat4(projection, view)
}

fn look_at_rh(eye: [f32; 3], target: [f32; 3], up: [f32; 3]) -> [f32; 16] {
    let f = normalize3(sub3(target, eye));
    let s = normalize3(cross3(f, up));
    let u = cross3(s, f);

    [
        s[0],
        u[0],
        -f[0],
        0.0,
        s[1],
        u[1],
        -f[1],
        0.0,
        s[2],
        u[2],
        -f[2],
        0.0,
        -dot3(s, eye),
        -dot3(u, eye),
        dot3(f, eye),
        1.0,
    ]
}

fn perspective_rh(fovy_radians: f32, aspect: f32, near: f32, far: f32) -> [f32; 16] {
    let f = 1.0 / (fovy_radians * 0.5).tan();
    [
        f / aspect,
        0.0,
        0.0,
        0.0,
        0.0,
        f,
        0.0,
        0.0,
        0.0,
        0.0,
        far / (near - far),
        -1.0,
        0.0,
        0.0,
        (far * near) / (near - far),
        0.0,
    ]
}

fn mul_mat4(a: [f32; 16], b: [f32; 16]) -> [f32; 16] {
    let mut out = [0.0; 16];
    for col in 0..4 {
        for row in 0..4 {
            out[col * 4 + row] = a[row] * b[col * 4]
                + a[4 + row] * b[col * 4 + 1]
                + a[8 + row] * b[col * 4 + 2]
                + a[12 + row] * b[col * 4 + 3];
        }
    }
    out
}

fn sub3(a: [f32; 3], b: [f32; 3]) -> [f32; 3] { [a[0] - b[0], a[1] - b[1], a[2] - b[2]] }

fn cross3(a: [f32; 3], b: [f32; 3]) -> [f32; 3] {
    [
        a[1] * b[2] - a[2] * b[1],
        a[2] * b[0] - a[0] * b[2],
        a[0] * b[1] - a[1] * b[0],
    ]
}

fn dot3(a: [f32; 3], b: [f32; 3]) -> f32 { a[0] * b[0] + a[1] * b[1] + a[2] * b[2] }

fn normalize3(v: [f32; 3]) -> [f32; 3] {
    let len = dot3(v, v).sqrt().max(f32::EPSILON);
    [v[0] / len, v[1] / len, v[2] / len]
}

fn f32_bytes(values: &[f32]) -> Vec<u8> {
    let mut bytes = Vec::with_capacity(std::mem::size_of_val(values));
    for value in values {
        bytes.extend_from_slice(&value.to_le_bytes());
    }
    bytes
}

fn u32_bytes(values: &[u32]) -> Vec<u8> {
    let mut bytes = Vec::with_capacity(std::mem::size_of_val(values));
    for value in values {
        bytes.extend_from_slice(&value.to_le_bytes());
    }
    bytes
}

fn matrix_bytes(matrix: &[f32; 16]) -> Vec<u8> { f32_bytes(matrix) }

fn web_window() -> Result<Window, String> {
    web_sys::window().ok_or_else(|| "missing browser window".to_owned())
}

fn web_document(window: &Window) -> Result<Document, String> {
    window
        .document()
        .ok_or_else(|| "missing browser document".to_owned())
}

fn canvas_element(document: &Document) -> Result<HtmlCanvasElement, String> {
    document
        .get_element_by_id(CANVAS_ID)
        .ok_or_else(|| format!("missing canvas element #{CANVAS_ID}"))?
        .dyn_into::<HtmlCanvasElement>()
        .map_err(|_| format!("element #{CANVAS_ID} is not a canvas"))
}

fn resize_canvas_to_window(
    canvas: &HtmlCanvasElement,
    window: &Window,
) -> Result<(u32, u32), String> {
    let device_pixel_ratio = window.device_pixel_ratio().max(1.0);
    let width = window
        .inner_width()
        .map_err(|_| "failed to read window width".to_owned())?
        .as_f64()
        .unwrap_or(1280.0);
    let height = window
        .inner_height()
        .map_err(|_| "failed to read window height".to_owned())?
        .as_f64()
        .unwrap_or(720.0);

    let physical_width = (width * device_pixel_ratio).round().max(1.0) as u32;
    let physical_height = (height * device_pixel_ratio).round().max(1.0) as u32;
    canvas.set_width(physical_width);
    canvas.set_height(physical_height);
    canvas
        .style()
        .set_property("width", "100vw")
        .map_err(|_| "failed to size canvas width".to_owned())?;
    canvas
        .style()
        .set_property("height", "100vh")
        .map_err(|_| "failed to size canvas height".to_owned())?;

    Ok((physical_width, physical_height))
}

enum StatusState {
    Loading,
    Ok,
    Error,
}

fn set_status(message: &str, state: StatusState) {
    if let Some(document) = web_sys::window().and_then(|window| window.document())
        && let Some(element) = document.get_element_by_id(STATUS_ID)
    {
        element.set_text_content(Some(message));
        let class_name = match state {
            StatusState::Loading => "status status-loading",
            StatusState::Ok => "status status-ok",
            StatusState::Error => "status status-error",
        };
        element.set_class_name(class_name);
    }
}

fn set_detail(message: &str) {
    if let Some(document) = web_sys::window().and_then(|window| window.document())
        && let Some(element) = document.get_element_by_id(DETAIL_ID)
    {
        element.set_text_content(Some(message));
    }
}
