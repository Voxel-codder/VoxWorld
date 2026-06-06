#![deny(unsafe_code)]

use wasm_bindgen::{JsCast, prelude::*};
use web_sys::{Document, HtmlCanvasElement, Window};

const CANVAS_ID: &str = "voxworld-canvas";
const DETAIL_ID: &str = "voxworld-detail";
const STATUS_ID: &str = "voxworld-status";
const PROBE_SHADER: &str = r#"
struct VertexOut {
    @builtin(position) position: vec4<f32>,
    @location(0) color: vec3<f32>,
};

@vertex
fn vs_main(@builtin(vertex_index) vertex_index: u32) -> VertexOut {
    var positions = array<vec2<f32>, 3>(
        vec2<f32>(0.0, 0.72),
        vec2<f32>(-0.72, -0.56),
        vec2<f32>(0.72, -0.56),
    );
    var colors = array<vec3<f32>, 3>(
        vec3<f32>(0.16, 0.90, 0.76),
        vec3<f32>(0.28, 0.48, 1.00),
        vec3<f32>(1.00, 0.72, 0.24),
    );

    var out: VertexOut;
    out.position = vec4<f32>(positions[vertex_index], 0.0, 1.0);
    out.color = colors[vertex_index];
    return out;
}

@fragment
fn fs_main(in: VertexOut) -> @location(0) vec4<f32> {
    return vec4<f32>(in.color, 1.0);
}
"#;

#[wasm_bindgen(start)]
pub fn start() {
    console_error_panic_hook::set_once();
    set_status("Starting Voxygen WebGPU bootstrap...", StatusState::Loading);
    set_detail("Requesting browser WebGPU adapter...");

    wasm_bindgen_futures::spawn_local(async {
        match VoxygenWebBootstrap::new().await {
            Ok(bootstrap) => {
                bootstrap.render_probe_frame();
                set_status("WebGPU render probe is running.", StatusState::Ok);
                set_detail(
                    "The colored triangle is drawn by the browser GPU path. Original Voxygen \
                     scene/HUD integration is the next porting step.",
                );
                bootstrap.leak_for_browser_lifetime();
            },
            Err(error) => {
                set_status("Voxygen WebGPU bootstrap failed.", StatusState::Error);
                set_detail(&error);
            },
        }
    });
}

struct VoxygenWebBootstrap {
    canvas: HtmlCanvasElement,
    surface: wgpu::Surface<'static>,
    device: wgpu::Device,
    queue: wgpu::Queue,
    config: wgpu::SurfaceConfiguration,
    probe_pipeline: wgpu::RenderPipeline,
}

impl VoxygenWebBootstrap {
    async fn new() -> Result<Self, String> {
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
        let probe_pipeline = create_probe_pipeline(&device, config.format);

        Ok(Self {
            canvas,
            surface,
            device,
            queue,
            config,
            probe_pipeline,
        })
    }

    fn render_probe_frame(&self) {
        let Ok(frame) = self.surface.get_current_texture() else {
            return;
        };
        let view = frame
            .texture
            .create_view(&wgpu::TextureViewDescriptor::default());
        let mut encoder = self
            .device
            .create_command_encoder(&wgpu::CommandEncoderDescriptor {
                label: Some("Voxygen Web Probe Frame"),
            });

        {
            let mut render_pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("Voxygen Web Probe Pass"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: &view,
                    resolve_target: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Clear(wgpu::Color {
                            r: 0.055,
                            g: 0.155,
                            b: 0.255,
                            a: 1.0,
                        }),
                        store: wgpu::StoreOp::Store,
                    },
                    depth_slice: None,
                })],
                depth_stencil_attachment: None,
                timestamp_writes: None,
                occlusion_query_set: None,
            });
            render_pass.set_pipeline(&self.probe_pipeline);
            render_pass.draw(0..3, 0..1);
        }

        self.queue.submit(Some(encoder.finish()));
        frame.present();
    }

    fn leak_for_browser_lifetime(self) {
        let _ = self.canvas;
        let _ = self.config;
        Box::leak(Box::new(self));
    }
}

fn create_probe_pipeline(
    device: &wgpu::Device,
    format: wgpu::TextureFormat,
) -> wgpu::RenderPipeline {
    let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
        label: Some("Voxygen Web Probe Shader"),
        source: wgpu::ShaderSource::Wgsl(PROBE_SHADER.into()),
    });
    let layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
        label: Some("Voxygen Web Probe Pipeline Layout"),
        bind_group_layouts: &[],
        push_constant_ranges: &[],
    });

    device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
        label: Some("Voxygen Web Probe Pipeline"),
        layout: Some(&layout),
        vertex: wgpu::VertexState {
            module: &shader,
            entry_point: Some("vs_main"),
            buffers: &[],
            compilation_options: Default::default(),
        },
        primitive: wgpu::PrimitiveState {
            topology: wgpu::PrimitiveTopology::TriangleList,
            ..Default::default()
        },
        depth_stencil: None,
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
