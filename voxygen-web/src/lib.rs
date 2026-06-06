#![deny(unsafe_code)]

use wasm_bindgen::{JsCast, prelude::*};
use web_sys::{Document, HtmlCanvasElement, Window};

const CANVAS_ID: &str = "voxworld-canvas";
const STATUS_ID: &str = "voxworld-status";

#[wasm_bindgen(start)]
pub fn start() {
    console_error_panic_hook::set_once();
    set_status("Starting Voxygen WebGPU bootstrap...");

    wasm_bindgen_futures::spawn_local(async {
        match VoxygenWebBootstrap::new().await {
            Ok(bootstrap) => {
                bootstrap.render_probe_frame();
                set_status(
                    "Voxygen WebGPU bootstrap is running. Next step: attach the original \
                     scene/HUD.",
                );
                bootstrap.leak_for_browser_lifetime();
            },
            Err(error) => set_status(&format!("Voxygen WebGPU bootstrap failed: {error}")),
        }
    });
}

struct VoxygenWebBootstrap {
    canvas: HtmlCanvasElement,
    surface: wgpu::Surface<'static>,
    device: wgpu::Device,
    queue: wgpu::Queue,
    config: wgpu::SurfaceConfiguration,
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

        Ok(Self {
            canvas,
            surface,
            device,
            queue,
            config,
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
            let _render_pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("Voxygen Web Probe Pass"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: &view,
                    resolve_target: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Clear(wgpu::Color {
                            r: 0.045,
                            g: 0.055,
                            b: 0.075,
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

fn set_status(message: &str) {
    if let Some(document) = web_sys::window().and_then(|window| window.document())
        && let Some(element) = document.get_element_by_id(STATUS_ID)
    {
        element.set_text_content(Some(message));
    }
}
