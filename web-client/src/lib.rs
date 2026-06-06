use std::{cell::RefCell, rc::Rc};

use wasm_bindgen::{JsCast, prelude::*};
use web_sys::{
    BinaryType, CanvasRenderingContext2d, CloseEvent, Document, ErrorEvent, Event,
    HtmlButtonElement, HtmlCanvasElement, HtmlElement, HtmlInputElement, MessageEvent, WebSocket,
    Window,
};

thread_local! {
    static SOCKET: RefCell<Option<WebSocket>> = const { RefCell::new(None) };
}

#[wasm_bindgen(start)]
pub fn start() -> Result<(), JsValue> {
    console_error_panic_hook::set_once();

    let window = web_window()?;
    let document = web_document(&window)?;
    let canvas: HtmlCanvasElement = element_by_id(&document, "world-canvas")?;
    let status: HtmlElement = element_by_id(&document, "connection-status")?;
    let server_url: HtmlInputElement = element_by_id(&document, "server-url")?;
    let connect_button: HtmlButtonElement = element_by_id(&document, "connect-button")?;
    let context = canvas_2d_context(&canvas)?;

    resize_canvas(&canvas)?;
    install_resize_handler(&window, canvas.clone())?;
    start_render_loop(&window, canvas, context)?;
    install_connect_handler(connect_button, server_url, status)?;

    Ok(())
}

fn install_connect_handler(
    button: HtmlButtonElement,
    server_url: HtmlInputElement,
    status: HtmlElement,
) -> Result<(), JsValue> {
    let on_click = Closure::<dyn FnMut(Event)>::new(move |_| {
        let url = server_url.value();
        status.set_text_content(Some("Connecting..."));

        match WebSocket::new(&url) {
            Ok(socket) => {
                socket.set_binary_type(BinaryType::Arraybuffer);
                attach_socket_handlers(&socket, status.clone());
                SOCKET.with(|slot| {
                    *slot.borrow_mut() = Some(socket);
                });
            },
            Err(error) => {
                status.set_text_content(Some("Connection failed before opening"));
                web_sys::console::error_1(&error);
            },
        }
    });

    button.add_event_listener_with_callback("click", on_click.as_ref().unchecked_ref())?;
    on_click.forget();
    Ok(())
}

fn attach_socket_handlers(socket: &WebSocket, status: HtmlElement) {
    let open_status = status.clone();
    let on_open = Closure::<dyn FnMut(Event)>::new(move |_| {
        open_status.set_text_content(Some("Connected"));
    });
    socket.set_onopen(Some(on_open.as_ref().unchecked_ref()));
    on_open.forget();

    let message_status = status.clone();
    let on_message = Closure::<dyn FnMut(MessageEvent)>::new(move |event: MessageEvent| {
        message_status.set_text_content(Some("Message received from server"));
        web_sys::console::log_1(&event.data());
    });
    socket.set_onmessage(Some(on_message.as_ref().unchecked_ref()));
    on_message.forget();

    let error_status = status.clone();
    let on_error = Closure::<dyn FnMut(ErrorEvent)>::new(move |event: ErrorEvent| {
        error_status.set_text_content(Some("Connection error"));
        web_sys::console::error_1(&event.into());
    });
    socket.set_onerror(Some(on_error.as_ref().unchecked_ref()));
    on_error.forget();

    let close_status = status.clone();
    let on_close = Closure::<dyn FnMut(CloseEvent)>::new(move |event: CloseEvent| {
        let reason = if event.reason().is_empty() {
            "Connection closed".to_owned()
        } else {
            format!("Connection closed: {}", event.reason())
        };
        close_status.set_text_content(Some(&reason));
        SOCKET.with(|slot| {
            *slot.borrow_mut() = None;
        });
    });
    socket.set_onclose(Some(on_close.as_ref().unchecked_ref()));
    on_close.forget();
}

fn install_resize_handler(window: &Window, canvas: HtmlCanvasElement) -> Result<(), JsValue> {
    let on_resize = Closure::<dyn FnMut(Event)>::new(move |_| {
        if let Err(error) = resize_canvas(&canvas) {
            web_sys::console::error_1(&error);
        }
    });

    window.add_event_listener_with_callback("resize", on_resize.as_ref().unchecked_ref())?;
    on_resize.forget();
    Ok(())
}

fn start_render_loop(
    window: &Window,
    canvas: HtmlCanvasElement,
    context: CanvasRenderingContext2d,
) -> Result<(), JsValue> {
    let frame = Rc::new(RefCell::new(None::<Closure<dyn FnMut(f64)>>));
    let frame_ref = Rc::clone(&frame);
    let animation_window = window.clone();

    *frame_ref.borrow_mut() = Some(Closure::<dyn FnMut(f64)>::new(move |time| {
        draw_placeholder(&context, &canvas, time);

        if let Some(callback) = frame.borrow().as_ref() {
            let _ = animation_window.request_animation_frame(callback.as_ref().unchecked_ref());
        }
    }));

    if let Some(callback) = frame_ref.borrow().as_ref() {
        window.request_animation_frame(callback.as_ref().unchecked_ref())?;
    }

    Ok(())
}

#[allow(deprecated)]
fn draw_placeholder(context: &CanvasRenderingContext2d, canvas: &HtmlCanvasElement, time: f64) {
    let width = f64::from(canvas.width());
    let height = f64::from(canvas.height());
    let cell = 28.0;
    let pulse = ((time / 700.0).sin() + 1.0) * 0.5;

    context.set_fill_style(&JsValue::from_str("#08111f"));
    context.fill_rect(0.0, 0.0, width, height);

    let cols = (width / cell).ceil() as i32;
    let rows = (height / cell).ceil() as i32;

    for y in 0..rows {
        for x in 0..cols {
            let wave = (((x + y) as f64 * 0.32) + time / 900.0).sin();
            let alpha = 0.08 + 0.14 * wave.max(0.0) + 0.08 * pulse;
            context.set_fill_style(&JsValue::from_str(&format!(
                "rgba(58, 180, 155, {alpha:.3})"
            )));
            context.fill_rect(
                f64::from(x) * cell + 1.0,
                f64::from(y) * cell + 1.0,
                cell - 2.0,
                cell - 2.0,
            );
        }
    }

    context.set_fill_style(&JsValue::from_str("#e7f7ff"));
    context.set_font("700 24px system-ui, sans-serif");
    let _ = context.fill_text("Vox World Web Client", 32.0, 48.0);

    context.set_fill_style(&JsValue::from_str("rgba(231, 247, 255, 0.72)"));
    context.set_font("15px system-ui, sans-serif");
    let _ = context.fill_text(
        "WASM boot path online. Renderer and transport come next.",
        32.0,
        76.0,
    );
}

fn resize_canvas(canvas: &HtmlCanvasElement) -> Result<(), JsValue> {
    let window = web_window()?;
    let scale = window.device_pixel_ratio();
    let width = window
        .inner_width()?
        .as_f64()
        .ok_or_else(|| JsValue::from_str("window.innerWidth was not a number"))?;
    let height = window
        .inner_height()?
        .as_f64()
        .ok_or_else(|| JsValue::from_str("window.innerHeight was not a number"))?;

    canvas.set_width((width * scale).round() as u32);
    canvas.set_height((height * scale).round() as u32);

    let element: &HtmlElement = canvas.unchecked_ref();
    let style = element.style();
    style.set_property("width", &format!("{width}px"))?;
    style.set_property("height", &format!("{height}px"))?;
    Ok(())
}

fn canvas_2d_context(canvas: &HtmlCanvasElement) -> Result<CanvasRenderingContext2d, JsValue> {
    Ok(canvas
        .get_context("2d")?
        .ok_or_else(|| JsValue::from_str("2d canvas context is unavailable"))?
        .dyn_into::<CanvasRenderingContext2d>()?)
}

fn element_by_id<T>(document: &Document, id: &str) -> Result<T, JsValue>
where
    T: JsCast,
{
    document
        .get_element_by_id(id)
        .ok_or_else(|| JsValue::from_str(&format!("missing element #{id}")))?
        .dyn_into::<T>()
        .map_err(|_| JsValue::from_str(&format!("element #{id} has the wrong type")))
}

fn web_window() -> Result<Window, JsValue> {
    web_sys::window().ok_or_else(|| JsValue::from_str("window is unavailable"))
}

fn web_document(window: &Window) -> Result<Document, JsValue> {
    window
        .document()
        .ok_or_else(|| JsValue::from_str("document is unavailable"))
}
