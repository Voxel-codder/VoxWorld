#![deny(unsafe_code)]

mod world_preview;

use std::{cell::RefCell, rc::Rc};
use vek::Vec2;
use wasm_bindgen::{JsCast, closure::Closure, prelude::*};
use web_sys::{Document, HtmlCanvasElement, KeyboardEvent, Window};
use world_preview::{
    FLOATS_PER_VERTEX, OriginalEntityMarker, OriginalEntityMarkerShape, OriginalWorldMesh,
    OriginalWorldPreview,
};

const CANVAS_ID: &str = "voxworld-canvas";
const DETAIL_ID: &str = "voxworld-detail";
const STATUS_ID: &str = "voxworld-status";
const DEPTH_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Depth24Plus;
const PLAYER_SPEED_BLOCKS_PER_SECOND: f32 = 14.0;
const MAX_FRAME_DELTA_SECONDS: f32 = 0.05;
const THIRD_PERSON_CAMERA_DISTANCE: f32 = 34.0;
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
                let client = Rc::new(RefCell::new(client));
                if let Err(error) = VoxygenWebClient::install_input_controls(Rc::clone(&client)) {
                    set_status("Original WorldSim terrain is rendering.", StatusState::Ok);
                    set_detail(&format!(
                        "{} Keyboard controls failed to attach: {error}",
                        client.borrow().scene_summary()
                    ));
                } else {
                    set_detail(&client.borrow().scene_summary());
                }
                if let Err(error) = VoxygenWebClient::install_animation_loop(Rc::clone(&client)) {
                    set_status("Original WorldSim terrain is rendering.", StatusState::Ok);
                    set_detail(&format!(
                        "{} Animation loop failed to start: {error}",
                        client.borrow().scene_summary()
                    ));
                }
                client.borrow().render_frame();
                set_status("Original WorldSim terrain is rendering.", StatusState::Ok);
                std::mem::forget(client);
            },
            Err(error) => {
                set_status("Voxygen web scene failed.", StatusState::Error);
                set_detail(&error);
            },
        }
    });
}

struct VoxygenWebClient {
    _canvas: HtmlCanvasElement,
    surface: wgpu::Surface<'static>,
    device: wgpu::Device,
    queue: wgpu::Queue,
    _config: wgpu::SurfaceConfiguration,
    depth_view: wgpu::TextureView,
    terrain_pipeline: wgpu::RenderPipeline,
    terrain_vertex_buffer: wgpu::Buffer,
    terrain_index_buffer: wgpu::Buffer,
    terrain_index_count: u32,
    entity_marker_vertex_buffer: wgpu::Buffer,
    entity_marker_index_buffer: wgpu::Buffer,
    entity_marker_index_count: u32,
    player_marker_vertex_buffer: wgpu::Buffer,
    player_marker_index_buffer: wgpu::Buffer,
    player_marker_index_count: u32,
    camera_buffer: wgpu::Buffer,
    camera_bind_group: wgpu::BindGroup,
    world_preview: OriginalWorldPreview,
    world_mesh: OriginalWorldMesh,
    player: PlayerState,
    keydown_listener: Option<Closure<dyn FnMut(KeyboardEvent)>>,
    keyup_listener: Option<Closure<dyn FnMut(KeyboardEvent)>>,
}

struct PlayerState {
    wpos: Vec2<f32>,
    center_chunk_pos: Vec2<i32>,
    aspect: f32,
    input: InputState,
    last_frame_ms: Option<f64>,
}

impl PlayerState {
    fn new(wpos: Vec2<f32>, center_chunk_pos: Vec2<i32>, aspect: f32) -> Self {
        Self {
            wpos,
            center_chunk_pos,
            aspect,
            input: InputState::default(),
            last_frame_ms: None,
        }
    }

    fn frame_delta(&mut self, timestamp_ms: f64) -> f32 {
        let dt = self
            .last_frame_ms
            .map(|last_frame_ms| ((timestamp_ms - last_frame_ms) / 1000.0) as f32)
            .unwrap_or(0.0)
            .clamp(0.0, MAX_FRAME_DELTA_SECONDS);
        self.last_frame_ms = Some(timestamp_ms);
        dt
    }
}

#[derive(Default)]
struct InputState {
    forward: bool,
    backward: bool,
    left: bool,
    right: bool,
}

impl InputState {
    fn set_key_state(&mut self, key: &str, pressed: bool) -> bool {
        match key {
            "ArrowUp" | "w" | "W" => self.forward = pressed,
            "ArrowDown" | "s" | "S" => self.backward = pressed,
            "ArrowLeft" | "a" | "A" => self.left = pressed,
            "ArrowRight" | "d" | "D" => self.right = pressed,
            _ => return false,
        }
        true
    }

    fn direction(&self) -> Option<Vec2<f32>> {
        let x = i32::from(self.right) - i32::from(self.left);
        let y = i32::from(self.backward) - i32::from(self.forward);
        if x == 0 && y == 0 {
            return None;
        }

        let direction = Vec2::new(x as f32, y as f32);
        let magnitude = (direction.x * direction.x + direction.y * direction.y).sqrt();
        Some(direction / magnitude.max(f32::EPSILON))
    }
}

impl VoxygenWebClient {
    async fn new() -> Result<Self, String> {
        let world_preview = world_preview::build_original_world_preview()?;
        let center_chunk_pos = world_preview.initial_center_chunk_pos();
        let player_wpos = world_preview.initial_player_wpos();
        let world_mesh = world_preview.generate_mesh(center_chunk_pos)?;
        set_detail(
            "Original WorldSim terrain patch generated. Requesting browser WebGPU adapter...",
        );

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
        let aspect = width as f32 / height.max(1) as f32;
        let player_render_pos = world_preview.player_render_position(
            player_wpos,
            center_chunk_pos,
            world_mesh.vertical_origin,
        );
        let camera_matrix = camera_view_projection(aspect, &world_mesh, player_render_pos);
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
        let (entity_marker_vertex_buffer, entity_marker_index_buffer, entity_marker_index_count) =
            create_marker_buffers(&device, &world_mesh.entity_markers, "Entity");
        let player_marker = player_marker(world_preview.player_render_position(
            player_wpos,
            center_chunk_pos,
            world_mesh.vertical_origin,
        ));
        let (player_marker_vertex_buffer, player_marker_index_buffer, player_marker_index_count) =
            create_marker_buffers(&device, &[player_marker], "Player");

        Ok(Self {
            _canvas: canvas,
            surface,
            device,
            queue,
            _config: config,
            depth_view,
            terrain_pipeline,
            terrain_vertex_buffer,
            terrain_index_buffer,
            terrain_index_count,
            entity_marker_vertex_buffer,
            entity_marker_index_buffer,
            entity_marker_index_count,
            player_marker_vertex_buffer,
            player_marker_index_buffer,
            player_marker_index_count,
            camera_buffer,
            camera_bind_group,
            world_preview,
            world_mesh,
            player: PlayerState::new(player_wpos, center_chunk_pos, aspect),
            keydown_listener: None,
            keyup_listener: None,
        })
    }

    fn install_input_controls(client: Rc<RefCell<Self>>) -> Result<(), String> {
        let window = web_window()?;

        let keydown_client = Rc::clone(&client);
        let keydown_listener = Closure::wrap(Box::new(move |event: KeyboardEvent| {
            if keydown_client
                .borrow_mut()
                .set_key_state(&event.key(), true)
            {
                event.prevent_default();
            }
        }) as Box<dyn FnMut(KeyboardEvent)>);
        window
            .add_event_listener_with_callback("keydown", keydown_listener.as_ref().unchecked_ref())
            .map_err(|_| "failed to attach keydown controls".to_owned())?;

        let keyup_client = Rc::clone(&client);
        let keyup_listener = Closure::wrap(Box::new(move |event: KeyboardEvent| {
            if keyup_client.borrow_mut().set_key_state(&event.key(), false) {
                event.prevent_default();
            }
        }) as Box<dyn FnMut(KeyboardEvent)>);
        window
            .add_event_listener_with_callback("keyup", keyup_listener.as_ref().unchecked_ref())
            .map_err(|_| "failed to attach keyup controls".to_owned())?;

        let mut client = client.borrow_mut();
        client.keydown_listener = Some(keydown_listener);
        client.keyup_listener = Some(keyup_listener);
        Ok(())
    }

    fn install_animation_loop(client: Rc<RefCell<Self>>) -> Result<(), String> {
        let window = web_window()?;
        let frame_callback: Rc<RefCell<Option<Closure<dyn FnMut(f64)>>>> =
            Rc::new(RefCell::new(None));
        let frame_callback_handle = Rc::clone(&frame_callback);
        let frame_client = Rc::clone(&client);

        *frame_callback.borrow_mut() = Some(Closure::wrap(Box::new(move |timestamp_ms: f64| {
            if let Err(error) = frame_client.borrow_mut().tick(timestamp_ms) {
                set_status("Voxygen web scene failed.", StatusState::Error);
                set_detail(&error);
                return;
            }
            if let Some(window) = web_sys::window()
                && let Some(callback) = frame_callback_handle.borrow().as_ref()
            {
                let _ = window.request_animation_frame(callback.as_ref().unchecked_ref());
            }
        }) as Box<dyn FnMut(f64)>));

        if let Some(callback) = frame_callback.borrow().as_ref() {
            window
                .request_animation_frame(callback.as_ref().unchecked_ref())
                .map_err(|_| "failed to request animation frame".to_owned())?;
        }
        Ok(())
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

            render_pass.set_vertex_buffer(0, self.entity_marker_vertex_buffer.slice(..));
            render_pass.set_index_buffer(
                self.entity_marker_index_buffer.slice(..),
                wgpu::IndexFormat::Uint32,
            );
            render_pass.draw_indexed(0..self.entity_marker_index_count, 0, 0..1);

            render_pass.set_vertex_buffer(0, self.player_marker_vertex_buffer.slice(..));
            render_pass.set_index_buffer(
                self.player_marker_index_buffer.slice(..),
                wgpu::IndexFormat::Uint32,
            );
            render_pass.draw_indexed(0..self.player_marker_index_count, 0, 0..1);
        }

        self.queue.submit(Some(encoder.finish()));
        frame.present();
    }

    fn upload_world_mesh(&mut self, world_mesh: OriginalWorldMesh) -> Result<(), String> {
        let terrain_vertex_buffer = create_buffer_with_data(
            &self.device,
            &f32_bytes(&world_mesh.vertices),
            wgpu::BufferUsages::VERTEX | wgpu::BufferUsages::COPY_DST,
            "Voxygen Web Terrain Vertex Buffer",
        );
        let terrain_index_buffer = create_buffer_with_data(
            &self.device,
            &u32_bytes(&world_mesh.indices),
            wgpu::BufferUsages::INDEX | wgpu::BufferUsages::COPY_DST,
            "Voxygen Web Terrain Index Buffer",
        );
        let terrain_index_count = world_mesh
            .indices
            .len()
            .try_into()
            .map_err(|_| "terrain index count overflowed u32".to_owned())?;
        let (entity_marker_vertex_buffer, entity_marker_index_buffer, entity_marker_index_count) =
            create_marker_buffers(&self.device, &world_mesh.entity_markers, "Entity");

        self.terrain_vertex_buffer = terrain_vertex_buffer;
        self.terrain_index_buffer = terrain_index_buffer;
        self.terrain_index_count = terrain_index_count;
        self.entity_marker_vertex_buffer = entity_marker_vertex_buffer;
        self.entity_marker_index_buffer = entity_marker_index_buffer;
        self.entity_marker_index_count = entity_marker_index_count;
        self.world_mesh = world_mesh;
        Ok(())
    }

    fn set_key_state(&mut self, key: &str, pressed: bool) -> bool {
        self.player.input.set_key_state(key, pressed)
    }

    fn tick(&mut self, timestamp_ms: f64) -> Result<(), String> {
        let dt = self.player.frame_delta(timestamp_ms);
        let Some(direction) = self.player.input.direction() else {
            return Ok(());
        };
        if dt <= 0.0 {
            return Ok(());
        }

        self.player.wpos += direction * (PLAYER_SPEED_BLOCKS_PER_SECOND * dt);
        self.player.wpos = self.world_preview.clamp_player_wpos(self.player.wpos);

        let next_center = self.world_preview.center_chunk_for_wpos(self.player.wpos);
        if next_center != self.player.center_chunk_pos {
            set_status(
                "Generating original WorldSim terrain chunks...",
                StatusState::Loading,
            );
            let world_mesh = self.world_preview.generate_mesh(next_center)?;
            self.upload_world_mesh(world_mesh)?;
            self.player.center_chunk_pos = next_center;
        }

        self.update_camera();
        self.update_player_marker();
        self.render_frame();
        set_status("Original WorldSim terrain is rendering.", StatusState::Ok);
        set_detail(&self.scene_summary());
        Ok(())
    }

    fn update_camera(&self) {
        let player_render_pos = self.world_preview.player_render_position(
            self.player.wpos,
            self.player.center_chunk_pos,
            self.world_mesh.vertical_origin,
        );
        let camera_matrix =
            camera_view_projection(self.player.aspect, &self.world_mesh, player_render_pos);
        self.queue
            .write_buffer(&self.camera_buffer, 0, &matrix_bytes(&camera_matrix));
    }

    fn update_player_marker(&mut self) {
        let player_render_pos = self.world_preview.player_render_position(
            self.player.wpos,
            self.player.center_chunk_pos,
            self.world_mesh.vertical_origin,
        );
        let marker = player_marker(player_render_pos);
        let (vertex_buffer, index_buffer, index_count) =
            create_marker_buffers(&self.device, &[marker], "Player");
        self.player_marker_vertex_buffer = vertex_buffer;
        self.player_marker_index_buffer = index_buffer;
        self.player_marker_index_count = index_count;
    }

    fn scene_summary(&self) -> String {
        let (chunks_x, chunks_y) = self.world_mesh.chunk_dimensions;
        let (patch_x, patch_y) = self.world_mesh.chunk_patch;
        format!(
            "Seed {} generated {} original TerrainChunks in a {}x{} patch around {:?} inside a \
             {}x{} WorldSim. Player block position: ({:.1}, {:.1}). WebGPU block faces: {}. \
             Filled blocks: {}. Liquid blocks: {}. Visible entity markers: {}. Entity spawns: {}. \
             World features loaded: {}. Wildlife spawn manifests: {}.",
            self.world_mesh.seed,
            self.world_mesh.generated_chunks,
            patch_x,
            patch_y,
            self.world_mesh.center_chunk_pos,
            chunks_x,
            chunks_y,
            self.player.wpos.x,
            self.player.wpos.y,
            self.world_mesh.terrain_faces,
            self.world_mesh.filled_blocks,
            self.world_mesh.liquid_blocks,
            self.world_mesh.entity_markers.len(),
            self.world_mesh.generated_entity_spawns,
            self.world_mesh.enabled_world_features,
            self.world_mesh.wildlife_spawn_manifests
        )
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

fn create_marker_buffers(
    device: &wgpu::Device,
    markers: &[OriginalEntityMarker],
    label_prefix: &'static str,
) -> (wgpu::Buffer, wgpu::Buffer, u32) {
    let (vertices, indices) = marker_mesh(markers);
    let vertex_buffer = create_buffer_with_data(
        device,
        &f32_bytes(&vertices),
        wgpu::BufferUsages::VERTEX | wgpu::BufferUsages::COPY_DST,
        marker_buffer_label(label_prefix, "Vertex"),
    );
    let index_buffer = create_buffer_with_data(
        device,
        &u32_bytes(&indices),
        wgpu::BufferUsages::INDEX | wgpu::BufferUsages::COPY_DST,
        marker_buffer_label(label_prefix, "Index"),
    );
    (vertex_buffer, index_buffer, indices.len() as u32)
}

fn marker_buffer_label(prefix: &'static str, kind: &'static str) -> &'static str {
    match (prefix, kind) {
        ("Player", "Vertex") => "Voxygen Web Player Marker Vertex Buffer",
        ("Player", "Index") => "Voxygen Web Player Marker Index Buffer",
        ("Entity", "Vertex") => "Voxygen Web Entity Marker Vertex Buffer",
        ("Entity", "Index") => "Voxygen Web Entity Marker Index Buffer",
        _ => "Voxygen Web Marker Buffer",
    }
}

fn player_marker(render_pos: [f32; 3]) -> OriginalEntityMarker {
    OriginalEntityMarker {
        render_pos: [render_pos[0], render_pos[1] + 0.18, render_pos[2]],
        radius: 0.78,
        height: 2.55,
        color: [1.0, 0.96, 0.22],
        shape: OriginalEntityMarkerShape::Humanoid,
    }
}

fn marker_mesh(markers: &[OriginalEntityMarker]) -> (Vec<f32>, Vec<u32>) {
    let mut vertices = Vec::new();
    let mut indices = Vec::new();
    for marker in markers {
        add_marker_shape(&mut vertices, &mut indices, *marker);
    }
    (vertices, indices)
}

fn add_marker_shape(vertices: &mut Vec<f32>, indices: &mut Vec<u32>, marker: OriginalEntityMarker) {
    match marker.shape {
        OriginalEntityMarkerShape::Humanoid => add_humanoid_marker(vertices, indices, marker),
        OriginalEntityMarkerShape::Quadruped => add_quadruped_marker(vertices, indices, marker),
        OriginalEntityMarkerShape::Flyer => add_flyer_marker(vertices, indices, marker),
        OriginalEntityMarkerShape::Fish => add_fish_marker(vertices, indices, marker),
        OriginalEntityMarkerShape::Large => add_large_marker(vertices, indices, marker),
        OriginalEntityMarkerShape::Object => add_object_marker(vertices, indices, marker),
    }
}

fn add_humanoid_marker(
    vertices: &mut Vec<f32>,
    indices: &mut Vec<u32>,
    marker: OriginalEntityMarker,
) {
    let [x, y, z] = marker.render_pos;
    let r = marker.radius;
    let h = marker.height;
    add_marker_cuboid(
        vertices,
        indices,
        [x, y + h * 0.38, z],
        [r * 0.48, h * 0.30, r * 0.34],
        marker.color,
    );
    add_marker_cuboid(
        vertices,
        indices,
        [x, y + h * 0.78, z],
        [r * 0.34, h * 0.13, r * 0.34],
        tint_marker_color(marker.color, 1.08),
    );
    for x_offset in [-r * 0.23, r * 0.23] {
        add_marker_cuboid(
            vertices,
            indices,
            [x + x_offset, y + h * 0.13, z],
            [r * 0.13, h * 0.17, r * 0.14],
            tint_marker_color(marker.color, 0.86),
        );
    }
    for x_offset in [-r * 0.60, r * 0.60] {
        add_marker_cuboid(
            vertices,
            indices,
            [x + x_offset, y + h * 0.42, z],
            [r * 0.11, h * 0.24, r * 0.11],
            tint_marker_color(marker.color, 0.92),
        );
    }
}

fn add_quadruped_marker(
    vertices: &mut Vec<f32>,
    indices: &mut Vec<u32>,
    marker: OriginalEntityMarker,
) {
    let [x, y, z] = marker.render_pos;
    let r = marker.radius;
    let h = marker.height;
    add_marker_cuboid(
        vertices,
        indices,
        [x, y + h * 0.43, z],
        [r * 1.12, h * 0.23, r * 0.46],
        marker.color,
    );
    add_marker_cuboid(
        vertices,
        indices,
        [x + r * 1.28, y + h * 0.55, z],
        [r * 0.36, h * 0.18, r * 0.32],
        tint_marker_color(marker.color, 1.06),
    );
    add_marker_cuboid(
        vertices,
        indices,
        [x - r * 1.34, y + h * 0.48, z],
        [r * 0.22, h * 0.06, r * 0.16],
        tint_marker_color(marker.color, 0.78),
    );
    for x_offset in [-r * 0.70, r * 0.70] {
        for z_offset in [-r * 0.30, r * 0.30] {
            add_marker_cuboid(
                vertices,
                indices,
                [x + x_offset, y + h * 0.15, z + z_offset],
                [r * 0.10, h * 0.16, r * 0.09],
                tint_marker_color(marker.color, 0.84),
            );
        }
    }
}

fn add_flyer_marker(vertices: &mut Vec<f32>, indices: &mut Vec<u32>, marker: OriginalEntityMarker) {
    let [x, y, z] = marker.render_pos;
    let r = marker.radius;
    let h = marker.height;
    add_marker_cuboid(
        vertices,
        indices,
        [x, y + h * 0.50, z],
        [r * 0.62, h * 0.20, r * 0.36],
        marker.color,
    );
    for z_offset in [-r * 0.98, r * 0.98] {
        add_marker_cuboid(
            vertices,
            indices,
            [x - r * 0.10, y + h * 0.52, z + z_offset],
            [r * 0.44, h * 0.05, r * 0.70],
            tint_marker_color(marker.color, 0.90),
        );
    }
    add_marker_cuboid(
        vertices,
        indices,
        [x + r * 0.78, y + h * 0.58, z],
        [r * 0.28, h * 0.12, r * 0.24],
        tint_marker_color(marker.color, 1.08),
    );
}

fn add_fish_marker(vertices: &mut Vec<f32>, indices: &mut Vec<u32>, marker: OriginalEntityMarker) {
    let [x, y, z] = marker.render_pos;
    let r = marker.radius;
    let h = marker.height;
    add_marker_cuboid(
        vertices,
        indices,
        [x, y + h * 0.46, z],
        [r * 1.02, h * 0.20, r * 0.32],
        marker.color,
    );
    add_marker_cuboid(
        vertices,
        indices,
        [x - r * 1.16, y + h * 0.46, z],
        [r * 0.20, h * 0.30, r * 0.11],
        tint_marker_color(marker.color, 0.92),
    );
    add_marker_cuboid(
        vertices,
        indices,
        [x + r * 0.95, y + h * 0.48, z],
        [r * 0.18, h * 0.13, r * 0.20],
        tint_marker_color(marker.color, 1.06),
    );
}

fn add_large_marker(vertices: &mut Vec<f32>, indices: &mut Vec<u32>, marker: OriginalEntityMarker) {
    let [x, y, z] = marker.render_pos;
    let r = marker.radius;
    let h = marker.height;
    add_marker_cuboid(
        vertices,
        indices,
        [x, y + h * 0.38, z],
        [r * 0.78, h * 0.34, r * 0.62],
        marker.color,
    );
    add_marker_cuboid(
        vertices,
        indices,
        [x, y + h * 0.82, z],
        [r * 0.48, h * 0.15, r * 0.45],
        tint_marker_color(marker.color, 1.05),
    );
    for x_offset in [-r * 0.52, r * 0.52] {
        add_marker_cuboid(
            vertices,
            indices,
            [x + x_offset, y + h * 0.12, z],
            [r * 0.16, h * 0.18, r * 0.18],
            tint_marker_color(marker.color, 0.84),
        );
    }
}

fn add_object_marker(
    vertices: &mut Vec<f32>,
    indices: &mut Vec<u32>,
    marker: OriginalEntityMarker,
) {
    let [x, y, z] = marker.render_pos;
    add_marker_cuboid(
        vertices,
        indices,
        [x, y + marker.height * 0.5, z],
        [marker.radius, marker.height * 0.5, marker.radius],
        marker.color,
    );
}

fn add_marker_cuboid(
    vertices: &mut Vec<f32>,
    indices: &mut Vec<u32>,
    center: [f32; 3],
    half_extents: [f32; 3],
    color: [f32; 3],
) {
    let [x, y, z] = center;
    let [hx, hy, hz] = half_extents.map(|extent| extent.max(0.01));
    let corners = [
        [x - hx, y - hy, z - hz],
        [x + hx, y - hy, z - hz],
        [x + hx, y - hy, z + hz],
        [x - hx, y - hy, z + hz],
        [x - hx, y + hy, z - hz],
        [x + hx, y + hy, z - hz],
        [x + hx, y + hy, z + hz],
        [x - hx, y + hy, z + hz],
    ];
    const FACES: [[usize; 4]; 6] = [
        [0, 1, 2, 3],
        [4, 7, 6, 5],
        [0, 4, 5, 1],
        [1, 5, 6, 2],
        [2, 6, 7, 3],
        [3, 7, 4, 0],
    ];

    for (face_index, face) in FACES.iter().enumerate() {
        let base = (vertices.len() / FLOATS_PER_VERTEX) as u32;
        let color = shade_marker_color(color, face_index);
        for corner_index in face {
            let [vx, vy, vz] = corners[*corner_index];
            vertices.extend_from_slice(&[vx, vy, vz, color[0], color[1], color[2]]);
        }
        indices.extend_from_slice(&[base, base + 1, base + 2, base, base + 2, base + 3]);
    }
}

fn tint_marker_color(color: [f32; 3], factor: f32) -> [f32; 3] {
    color.map(|channel| (channel * factor).min(1.0))
}

fn shade_marker_color(color: [f32; 3], face_index: usize) -> [f32; 3] {
    let shade = match face_index {
        1 => 1.12,
        0 => 0.58,
        2 | 3 => 0.88,
        _ => 0.74,
    };
    color.map(|channel| (channel * shade).min(1.0))
}

fn camera_view_projection(
    aspect: f32,
    _world_mesh: &OriginalWorldMesh,
    player_render_pos: [f32; 3],
) -> [f32; 16] {
    let target = [
        player_render_pos[0],
        player_render_pos[1] + 3.6,
        player_render_pos[2],
    ];
    let eye = [
        target[0] + THIRD_PERSON_CAMERA_DISTANCE * 0.58,
        target[1] + THIRD_PERSON_CAMERA_DISTANCE * 0.44,
        target[2] + THIRD_PERSON_CAMERA_DISTANCE * 0.72,
    ];
    let up = [0.0, 1.0, 0.0];
    let view = look_at_rh(eye, target, up);
    let projection = perspective_rh(58.0_f32.to_radians(), aspect.max(0.1), 0.1, 220.0);
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
