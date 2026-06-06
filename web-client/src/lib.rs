use std::{cell::RefCell, rc::Rc};

use js_sys::Reflect;
use wasm_bindgen::{JsCast, prelude::*};
use web_sys::{
    BinaryType, CanvasRenderingContext2d, CloseEvent, Document, Element, ErrorEvent, Event,
    HtmlButtonElement, HtmlCanvasElement, HtmlElement, HtmlInputElement, KeyboardEvent,
    MessageEvent, PointerEvent, WebSocket, Window,
};

thread_local! {
    static SOCKET: RefCell<Option<WebSocket>> = const { RefCell::new(None) };
    static INPUT: RefCell<InputState> = const { RefCell::new(InputState::new()) };
    static LAST_SNAPSHOT: RefCell<Option<SnapshotView>> = const { RefCell::new(None) };
    static CHAT_LOG: RefCell<Option<HtmlElement>> = const { RefCell::new(None) };
}

#[derive(Clone, Copy)]
struct InputState {
    forward: bool,
    back: bool,
    left: bool,
    right: bool,
    up: bool,
    down: bool,
    look_x: f32,
    look_y: f32,
    look_z: f32,
}

#[derive(Clone)]
struct SnapshotView {
    username: String,
    in_game: bool,
    position: Option<[f64; 3]>,
    health: Option<StatView>,
    energy: Option<StatView>,
    players_online: u32,
    entities: Vec<EntityView>,
}

#[derive(Clone)]
struct StatView {
    current: f64,
    maximum: f64,
    fraction: f64,
}

#[derive(Clone)]
struct EntityView {
    name: Option<String>,
    kind: String,
    is_self: bool,
    position: [f64; 3],
    health: Option<StatView>,
}

#[derive(Clone, Copy)]
enum ActionKind {
    Primary,
    Secondary,
    Block,
    Roll,
    Jump,
}

#[derive(Clone, Copy)]
enum ControlKind {
    Interact,
    Pickup,
    ToggleWield,
    SwapLoadout,
    Sneak,
    Sit,
    Respawn,
}

#[derive(Clone, Copy)]
enum MoveDirection {
    Forward,
    Back,
    Left,
    Right,
}

impl InputState {
    const fn new() -> Self {
        Self {
            forward: false,
            back: false,
            left: false,
            right: false,
            up: false,
            down: false,
            look_x: 0.0,
            look_y: 1.0,
            look_z: 0.0,
        }
    }

    fn movement(self) -> (f32, f32, f32) {
        let x = match (self.left, self.right) {
            (true, false) => -1.0,
            (false, true) => 1.0,
            _ => 0.0,
        };
        let y = match (self.forward, self.back) {
            (true, false) => 1.0,
            (false, true) => -1.0,
            _ => 0.0,
        };
        let z = match (self.up, self.down) {
            (true, false) => 1.0,
            (false, true) => -1.0,
            _ => 0.0,
        };

        (x, y, z)
    }

    fn look(self) -> (f32, f32, f32) { (self.look_x, self.look_y, self.look_z) }

    fn clear_controls(&mut self) -> bool {
        let had_controls =
            self.forward || self.back || self.left || self.right || self.up || self.down;
        self.forward = false;
        self.back = false;
        self.left = false;
        self.right = false;
        self.up = false;
        self.down = false;
        had_controls
    }
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
    let chat_log: HtmlElement = element_by_id(&document, "chat-log")?;
    let chat_form: HtmlElement = element_by_id(&document, "chat-form")?;
    let chat_input: HtmlInputElement = element_by_id(&document, "chat-input")?;
    let context = canvas_2d_context(&canvas)?;

    CHAT_LOG.with(|slot| {
        *slot.borrow_mut() = Some(chat_log);
    });

    resize_canvas(&canvas)?;
    install_resize_handler(&window, canvas.clone())?;
    install_pointer_handlers(&canvas)?;
    start_render_loop(&window, canvas, context)?;
    install_connect_handler(connect_button, server_url, status.clone())?;
    install_chat_handler(chat_form, chat_input, status)?;
    install_keyboard_handlers(&window)?;
    install_session_lifecycle_handlers(&window)?;
    install_touch_controls(&document)?;
    append_chat_line("system", "Session log ready");

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
        close_existing_socket("reconnect");

        match WebSocket::new(&url) {
            Ok(socket) => {
                socket.set_binary_type(BinaryType::Arraybuffer);
                attach_socket_handlers(&socket, status.clone());
                store_socket(socket);
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
        append_chat_line("system", "Connected");
    });
    socket.set_onopen(Some(on_open.as_ref().unchecked_ref()));
    on_open.forget();

    let message_status = status.clone();
    let on_message = Closure::<dyn FnMut(MessageEvent)>::new(move |event: MessageEvent| {
        message_status.set_text_content(Some(&summarize_server_message(&event.data())));
        web_sys::console::log_1(&event.data());
    });
    socket.set_onmessage(Some(on_message.as_ref().unchecked_ref()));
    on_message.forget();

    let error_status = status.clone();
    let on_error = Closure::<dyn FnMut(ErrorEvent)>::new(move |event: ErrorEvent| {
        error_status.set_text_content(Some("Connection error"));
        append_chat_line("error", "Connection error");
        web_sys::console::error_1(&event.into());
    });
    socket.set_onerror(Some(on_error.as_ref().unchecked_ref()));
    on_error.forget();

    let close_status = status.clone();
    let close_socket = socket.clone();
    let on_close = Closure::<dyn FnMut(CloseEvent)>::new(move |event: CloseEvent| {
        let reason = if event.reason().is_empty() {
            "Connection closed".to_owned()
        } else {
            format!("Connection closed: {}", event.reason())
        };
        close_status.set_text_content(Some(&reason));
        append_chat_line("system", &reason);
        clear_socket_if_current(&close_socket);
    });
    socket.set_onclose(Some(on_close.as_ref().unchecked_ref()));
    on_close.forget();
}

fn store_socket(socket: WebSocket) {
    SOCKET.with(|slot| {
        *slot.borrow_mut() = Some(socket);
    });
}

fn clear_socket_if_current(socket: &WebSocket) {
    SOCKET.with(|slot| {
        let should_clear = slot
            .borrow()
            .as_ref()
            .is_some_and(|current| js_sys::Object::is(current.as_ref(), socket.as_ref()));

        if should_clear {
            *slot.borrow_mut() = None;
        }
    });
}

fn close_existing_socket(reason: &str) {
    SOCKET.with(|slot| {
        let Some(socket) = slot.borrow_mut().take() else {
            return;
        };

        if socket.ready_state() != WebSocket::CLOSED {
            let _ = socket.close_with_code_and_reason(1000, reason);
        }
    });
}

fn summarize_server_message(data: &JsValue) -> String {
    let Some(text) = data.as_string() else {
        return "Binary message received from session".to_owned();
    };
    let Ok(value) = js_sys::JSON::parse(&text) else {
        return "Session message received".to_owned();
    };
    let message_type = string_property(&value, "type");

    match message_type.as_deref() {
        Some("stage") => {
            let stage = string_property(&value, "stage").unwrap_or_else(|| "unknown".to_owned());
            format!("Session stage: {stage}")
        },
        Some("snapshot") => {
            if let Some(snapshot) = parse_snapshot(&value) {
                LAST_SNAPSHOT.with(|last| {
                    *last.borrow_mut() = Some(snapshot.clone());
                });
            }

            let username = string_property(&value, "username").unwrap_or_else(|| "web".to_owned());
            let in_game = bool_property(&value, "in_game");
            let players = Reflect::get(&value, &JsValue::from_str("players_online"))
                .ok()
                .and_then(|players| players.dyn_into::<js_sys::Array>().ok())
                .map_or(0, |players| players.length());
            let state = if in_game { "in game" } else { "joining" };
            format!("{username}: {state}, {players} online")
        },
        Some("error") => {
            let message =
                string_property(&value, "message").unwrap_or_else(|| "unknown error".to_owned());
            append_chat_line("error", &message);
            format!("Session error: {message}")
        },
        Some("event") => {
            let message = string_property(&value, "message").unwrap_or_else(|| "event".to_owned());
            append_chat_line("event", &message);
            format!("Session event: {message}")
        },
        Some("chat") => {
            let line = summarize_chat_message(&value);
            append_chat_line("chat", &line);
            format!("Chat: {line}")
        },
        _ => "Session message received".to_owned(),
    }
}

fn summarize_chat_message(value: &JsValue) -> String {
    let scope = string_property(value, "scope").unwrap_or_else(|| "world".to_owned());
    let message = string_property(value, "message").unwrap_or_else(|| "message".to_owned());

    match string_property(value, "from").filter(|from| !from.is_empty()) {
        Some(from) => format!("[{scope}] {from}: {message}"),
        None => format!("[{scope}] {message}"),
    }
}

fn parse_snapshot(value: &JsValue) -> Option<SnapshotView> {
    let username = string_property(value, "username").unwrap_or_else(|| "web".to_owned());
    let in_game = bool_property(value, "in_game");
    let players_online = Reflect::get(value, &JsValue::from_str("players_online"))
        .ok()
        .and_then(|players| players.dyn_into::<js_sys::Array>().ok())
        .map_or(0, |players| players.length());
    let position = array3_property(value, "position");
    let health = stat_property(value, "health");
    let energy = stat_property(value, "energy");
    let entities = Reflect::get(value, &JsValue::from_str("entities"))
        .ok()
        .and_then(|entities| entities.dyn_into::<js_sys::Array>().ok())
        .map(|entities| {
            entities
                .iter()
                .filter_map(|entity| {
                    Some(EntityView {
                        name: string_property(&entity, "name"),
                        kind: string_property(&entity, "kind").unwrap_or_else(|| "entity".into()),
                        is_self: bool_property(&entity, "is_self"),
                        position: array3_property(&entity, "position")?,
                        health: stat_property(&entity, "health"),
                    })
                })
                .collect()
        })
        .unwrap_or_default();

    Some(SnapshotView {
        username,
        in_game,
        position,
        health,
        energy,
        players_online,
        entities,
    })
}

fn array3_property(value: &JsValue, key: &str) -> Option<[f64; 3]> {
    let array = Reflect::get(value, &JsValue::from_str(key))
        .ok()?
        .dyn_into::<js_sys::Array>()
        .ok()?;

    Some([
        array.get(0).as_f64()?,
        array.get(1).as_f64()?,
        array.get(2).as_f64()?,
    ])
}

fn stat_property(value: &JsValue, key: &str) -> Option<StatView> {
    let stat = Reflect::get(value, &JsValue::from_str(key)).ok()?;
    if stat.is_null() || stat.is_undefined() {
        return None;
    }

    Some(StatView {
        current: number_property(&stat, "current")?,
        maximum: number_property(&stat, "maximum")?,
        fraction: number_property(&stat, "fraction")?.clamp(0.0, 1.0),
    })
}

fn string_property(value: &JsValue, key: &str) -> Option<String> {
    Reflect::get(value, &JsValue::from_str(key))
        .ok()
        .and_then(|value| value.as_string())
}

fn number_property(value: &JsValue, key: &str) -> Option<f64> {
    Reflect::get(value, &JsValue::from_str(key))
        .ok()
        .and_then(|value| value.as_f64())
}

fn bool_property(value: &JsValue, key: &str) -> bool {
    Reflect::get(value, &JsValue::from_str(key))
        .ok()
        .and_then(|value| value.as_bool())
        .unwrap_or(false)
}

fn install_chat_handler(
    form: HtmlElement,
    input: HtmlInputElement,
    status: HtmlElement,
) -> Result<(), JsValue> {
    let on_submit = Closure::<dyn FnMut(Event)>::new(move |event: Event| {
        event.prevent_default();
        send_chat_message(&input, &status);
    });

    form.add_event_listener_with_callback("submit", on_submit.as_ref().unchecked_ref())?;
    on_submit.forget();
    Ok(())
}

fn send_chat_message(input: &HtmlInputElement, status: &HtmlElement) {
    let message = input.value().trim().to_owned();
    if message.is_empty() {
        return;
    }

    let payload = format!(r#"{{"type":"chat","message":{}}}"#, json_string(&message));
    let sent = send_socket_message(&payload);

    if sent {
        input.set_value("");
        append_chat_line("outbound", &format!("You: {message}"));
    } else {
        status.set_text_content(Some("Chat unavailable"));
        append_chat_line("error", "Chat unavailable");
    }
}

fn json_string(value: &str) -> String {
    let mut output = String::with_capacity(value.len() + 2);
    output.push('"');
    for character in value.chars() {
        match character {
            '"' => output.push_str("\\\""),
            '\\' => output.push_str("\\\\"),
            '\n' => output.push_str("\\n"),
            '\r' => output.push_str("\\r"),
            '\t' => output.push_str("\\t"),
            '\u{08}' => output.push_str("\\b"),
            '\u{0c}' => output.push_str("\\f"),
            character if character < ' ' => {
                output.push_str(&format!("\\u{:04x}", character as u32));
            },
            character => output.push(character),
        }
    }
    output.push('"');
    output
}

fn append_chat_line(kind: &str, text: &str) {
    CHAT_LOG.with(|slot| {
        let Some(log) = slot.borrow().as_ref().cloned() else {
            return;
        };
        let Some(document) = log.owner_document() else {
            return;
        };
        let Ok(line) = document.create_element("div") else {
            return;
        };

        line.set_class_name(&format!("chat-line chat-line-{kind}"));
        line.set_text_content(Some(text));

        if let Err(error) = log.append_child(&line) {
            web_sys::console::error_1(&error);
            return;
        }

        while log.child_element_count() > 80 {
            let Some(first) = log.first_element_child() else {
                break;
            };
            if let Err(error) = log.remove_child(&first) {
                web_sys::console::error_1(&error);
                break;
            }
        }

        log.set_scroll_top(log.scroll_height());
    });
}

fn install_keyboard_handlers(window: &Window) -> Result<(), JsValue> {
    let on_keydown = Closure::<dyn FnMut(KeyboardEvent)>::new(move |event: KeyboardEvent| {
        if focused_text_field() {
            return;
        }
        if update_key(event.key().as_str(), true) {
            event.prevent_default();
            send_input_state();
        } else if let Some(action) = action_for_key(event.key().as_str()) {
            event.prevent_default();
            if !event.repeat() {
                send_action(action, true);
            }
        } else if let Some(control) = control_for_key(event.key().as_str()) {
            event.prevent_default();
            if !event.repeat() {
                send_control(control);
            }
        }
    });
    window.add_event_listener_with_callback("keydown", on_keydown.as_ref().unchecked_ref())?;
    on_keydown.forget();

    let on_keyup = Closure::<dyn FnMut(KeyboardEvent)>::new(move |event: KeyboardEvent| {
        if focused_text_field() {
            return;
        }
        if update_key(event.key().as_str(), false) {
            event.prevent_default();
            send_input_state();
        } else if let Some(action) = action_for_key(event.key().as_str()) {
            event.prevent_default();
            send_action(action, false);
        }
    });
    window.add_event_listener_with_callback("keyup", on_keyup.as_ref().unchecked_ref())?;
    on_keyup.forget();

    Ok(())
}

fn install_session_lifecycle_handlers(window: &Window) -> Result<(), JsValue> {
    let on_blur = Closure::<dyn FnMut(Event)>::new(move |_| {
        release_controls();
    });
    window.add_event_listener_with_callback("blur", on_blur.as_ref().unchecked_ref())?;
    on_blur.forget();

    let on_pagehide = Closure::<dyn FnMut(Event)>::new(move |_| {
        release_controls();
        close_existing_socket("page hidden");
    });
    window.add_event_listener_with_callback("pagehide", on_pagehide.as_ref().unchecked_ref())?;
    on_pagehide.forget();

    if let Some(document) = window.document() {
        let on_visibility_change = Closure::<dyn FnMut(Event)>::new(move |_| {
            release_controls();
        });
        document.add_event_listener_with_callback(
            "visibilitychange",
            on_visibility_change.as_ref().unchecked_ref(),
        )?;
        on_visibility_change.forget();
    }

    Ok(())
}

fn focused_text_field() -> bool {
    web_window()
        .ok()
        .and_then(|window| window.document())
        .and_then(|document| document.active_element())
        .is_some_and(|element| {
            element.dyn_ref::<HtmlInputElement>().is_some()
                || element
                    .dyn_ref::<Element>()
                    .is_some_and(|element| element.has_attribute("contenteditable"))
        })
}

fn update_key(key: &str, pressed: bool) -> bool {
    match key {
        "w" | "W" | "ArrowUp" => set_movement(MoveDirection::Forward, pressed),
        "s" | "S" | "ArrowDown" => set_movement(MoveDirection::Back, pressed),
        "a" | "A" | "ArrowLeft" => set_movement(MoveDirection::Left, pressed),
        "d" | "D" | "ArrowRight" => set_movement(MoveDirection::Right, pressed),
        "PageUp" => {
            INPUT.with(|input| input.borrow_mut().up = pressed);
            true
        },
        "PageDown" => {
            INPUT.with(|input| input.borrow_mut().down = pressed);
            true
        },
        _ => false,
    }
}

fn action_for_key(key: &str) -> Option<ActionKind> {
    match key {
        " " => Some(ActionKind::Jump),
        "Shift" => Some(ActionKind::Roll),
        "f" | "F" => Some(ActionKind::Block),
        "j" | "J" => Some(ActionKind::Primary),
        "k" | "K" => Some(ActionKind::Secondary),
        _ => None,
    }
}

fn control_for_key(key: &str) -> Option<ControlKind> {
    match key {
        "e" | "E" => Some(ControlKind::Interact),
        "g" | "G" => Some(ControlKind::Pickup),
        "r" | "R" => Some(ControlKind::ToggleWield),
        "Tab" => Some(ControlKind::SwapLoadout),
        "c" | "C" => Some(ControlKind::Sneak),
        "x" | "X" => Some(ControlKind::Sit),
        "l" | "L" => Some(ControlKind::Respawn),
        _ => None,
    }
}

fn install_touch_controls(document: &Document) -> Result<(), JsValue> {
    install_movement_button(
        element_by_id(document, "touch-forward")?,
        MoveDirection::Forward,
    )?;
    install_movement_button(element_by_id(document, "touch-back")?, MoveDirection::Back)?;
    install_movement_button(element_by_id(document, "touch-left")?, MoveDirection::Left)?;
    install_movement_button(
        element_by_id(document, "touch-right")?,
        MoveDirection::Right,
    )?;

    install_action_button(
        element_by_id(document, "touch-primary")?,
        ActionKind::Primary,
    )?;
    install_action_button(
        element_by_id(document, "touch-secondary")?,
        ActionKind::Secondary,
    )?;
    install_action_button(element_by_id(document, "touch-jump")?, ActionKind::Jump)?;
    install_action_button(element_by_id(document, "touch-roll")?, ActionKind::Roll)?;
    install_action_button(element_by_id(document, "touch-block")?, ActionKind::Block)?;
    install_control_button(
        element_by_id(document, "touch-interact")?,
        ControlKind::Interact,
    )?;
    install_control_button(
        element_by_id(document, "touch-wield")?,
        ControlKind::ToggleWield,
    )?;
    install_control_button(
        element_by_id(document, "touch-swap-loadout")?,
        ControlKind::SwapLoadout,
    )?;
    Ok(())
}

fn install_movement_button(button: HtmlElement, direction: MoveDirection) -> Result<(), JsValue> {
    let on_down = Closure::<dyn FnMut(PointerEvent)>::new(move |event: PointerEvent| {
        event.prevent_default();
        if set_movement(direction, true) {
            send_input_state();
        }
    });
    button.add_event_listener_with_callback("pointerdown", on_down.as_ref().unchecked_ref())?;
    on_down.forget();

    for event_name in ["pointerup", "pointercancel", "pointerleave"] {
        let on_up = Closure::<dyn FnMut(PointerEvent)>::new(move |event: PointerEvent| {
            event.prevent_default();
            if set_movement(direction, false) {
                send_input_state();
            }
        });
        button.add_event_listener_with_callback(event_name, on_up.as_ref().unchecked_ref())?;
        on_up.forget();
    }

    Ok(())
}

fn install_action_button(button: HtmlElement, action: ActionKind) -> Result<(), JsValue> {
    let on_down = Closure::<dyn FnMut(PointerEvent)>::new(move |event: PointerEvent| {
        event.prevent_default();
        send_action(action, true);
    });
    button.add_event_listener_with_callback("pointerdown", on_down.as_ref().unchecked_ref())?;
    on_down.forget();

    for event_name in ["pointerup", "pointercancel", "pointerleave"] {
        let on_up = Closure::<dyn FnMut(PointerEvent)>::new(move |event: PointerEvent| {
            event.prevent_default();
            send_action(action, false);
        });
        button.add_event_listener_with_callback(event_name, on_up.as_ref().unchecked_ref())?;
        on_up.forget();
    }

    Ok(())
}

fn install_control_button(button: HtmlElement, control: ControlKind) -> Result<(), JsValue> {
    let on_down = Closure::<dyn FnMut(PointerEvent)>::new(move |event: PointerEvent| {
        event.prevent_default();
        send_control(control);
    });
    button.add_event_listener_with_callback("pointerdown", on_down.as_ref().unchecked_ref())?;
    on_down.forget();
    Ok(())
}

fn set_movement(direction: MoveDirection, pressed: bool) -> bool {
    INPUT.with(|input| {
        let mut input = input.borrow_mut();
        match direction {
            MoveDirection::Forward => input.forward = pressed,
            MoveDirection::Back => input.back = pressed,
            MoveDirection::Left => input.left = pressed,
            MoveDirection::Right => input.right = pressed,
        }
    });
    true
}

fn release_controls() {
    let changed = INPUT.with(|input| input.borrow_mut().clear_controls());
    if changed {
        send_input_state();
    }

    for action in ActionKind::all() {
        send_action(action, false);
    }
}

fn install_pointer_handlers(canvas: &HtmlCanvasElement) -> Result<(), JsValue> {
    let move_canvas = canvas.clone();
    let on_pointer_move = Closure::<dyn FnMut(PointerEvent)>::new(move |event: PointerEvent| {
        event.prevent_default();
        update_look_from_pointer(&move_canvas, event.client_x(), event.client_y());
        send_input_state();
    });
    canvas.add_event_listener_with_callback(
        "pointermove",
        on_pointer_move.as_ref().unchecked_ref(),
    )?;
    on_pointer_move.forget();

    let down_canvas = canvas.clone();
    let on_pointer_down = Closure::<dyn FnMut(PointerEvent)>::new(move |event: PointerEvent| {
        event.prevent_default();
        update_look_from_pointer(&down_canvas, event.client_x(), event.client_y());
        send_input_state();
        if event.pointer_type() == "mouse"
            && let Some(action) = action_for_pointer_button(event.button())
        {
            send_action(action, true);
        }
    });
    canvas.add_event_listener_with_callback(
        "pointerdown",
        on_pointer_down.as_ref().unchecked_ref(),
    )?;
    on_pointer_down.forget();

    let up_canvas = canvas.clone();
    let on_pointer_up = Closure::<dyn FnMut(PointerEvent)>::new(move |event: PointerEvent| {
        event.prevent_default();
        update_look_from_pointer(&up_canvas, event.client_x(), event.client_y());
        send_input_state();
        if event.pointer_type() == "mouse"
            && let Some(action) = action_for_pointer_button(event.button())
        {
            send_action(action, false);
        }
    });
    canvas.add_event_listener_with_callback("pointerup", on_pointer_up.as_ref().unchecked_ref())?;
    on_pointer_up.forget();

    let on_context_menu = Closure::<dyn FnMut(Event)>::new(move |event: Event| {
        event.prevent_default();
    });
    canvas.add_event_listener_with_callback(
        "contextmenu",
        on_context_menu.as_ref().unchecked_ref(),
    )?;
    on_context_menu.forget();
    Ok(())
}

fn action_for_pointer_button(button: i16) -> Option<ActionKind> {
    match button {
        0 => Some(ActionKind::Primary),
        2 => Some(ActionKind::Secondary),
        _ => None,
    }
}

fn update_look_from_pointer(canvas: &HtmlCanvasElement, client_x: i32, client_y: i32) {
    let element: &Element = canvas.unchecked_ref();
    let rect = element.get_bounding_client_rect();
    let center_x = rect.left() + rect.width() * 0.5;
    let center_y = rect.top() + rect.height() * 0.5;
    let dx = f64::from(client_x) - center_x;
    let dy = center_y - f64::from(client_y);
    let magnitude = (dx * dx + dy * dy).sqrt();

    if !magnitude.is_finite() || magnitude < 8.0 {
        return;
    }

    let look_x = (dx / magnitude) as f32;
    let look_y = (dy / magnitude) as f32;

    INPUT.with(|input| {
        let mut input = input.borrow_mut();
        input.look_x = look_x;
        input.look_y = look_y;
        input.look_z = 0.0;
    });
}

fn send_action(action: ActionKind, pressed: bool) {
    let message = format!(
        r#"{{"type":"action","action":"{}","pressed":{pressed}}}"#,
        action.as_str()
    );

    let _ = send_socket_message(&message);
}

fn send_control(control: ControlKind) {
    let message = format!(r#"{{"type":"control","control":"{}"}}"#, control.as_str());

    let _ = send_socket_message(&message);
}

fn send_input_state() {
    let (move_x, move_y, move_z) = INPUT.with(|input| input.borrow().movement());
    let (look_x, look_y, look_z) = INPUT.with(|input| input.borrow().look());
    let message = format!(
        r#"{{"type":"input","move_x":{move_x},"move_y":{move_y},"move_z":{move_z},"look_x":{look_x},"look_y":{look_y},"look_z":{look_z}}}"#
    );

    let _ = send_socket_message(&message);
}

fn send_socket_message(message: &str) -> bool {
    SOCKET.with(|slot| {
        let socket = slot.borrow();
        let Some(socket) = socket.as_ref() else {
            return false;
        };
        if socket.ready_state() != WebSocket::OPEN {
            return false;
        }
        match socket.send_with_str(message) {
            Ok(()) => true,
            Err(error) => {
                web_sys::console::error_1(&error);
                false
            },
        }
    })
}

impl ActionKind {
    const fn all() -> [Self; 5] {
        [
            Self::Primary,
            Self::Secondary,
            Self::Block,
            Self::Roll,
            Self::Jump,
        ]
    }

    fn as_str(self) -> &'static str {
        match self {
            Self::Primary => "primary",
            Self::Secondary => "secondary",
            Self::Block => "block",
            Self::Roll => "roll",
            Self::Jump => "jump",
        }
    }
}

impl ControlKind {
    fn as_str(self) -> &'static str {
        match self {
            Self::Interact => "interact",
            Self::Pickup => "pickup",
            Self::ToggleWield => "toggle_wield",
            Self::SwapLoadout => "swap_loadout",
            Self::Sneak => "sneak",
            Self::Sit => "sit",
            Self::Respawn => "respawn",
        }
    }
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
        draw_world(&context, &canvas, time);

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
fn draw_world(context: &CanvasRenderingContext2d, canvas: &HtmlCanvasElement, time: f64) {
    let width = f64::from(canvas.width());
    let height = f64::from(canvas.height());
    let snapshot = LAST_SNAPSHOT.with(|last| last.borrow().clone());

    if let Some(snapshot) = snapshot
        && snapshot.position.is_some()
    {
        draw_snapshot(context, width, height, &snapshot);
        return;
    }

    draw_placeholder(context, width, height, time);
}

#[allow(deprecated)]
fn draw_placeholder(context: &CanvasRenderingContext2d, width: f64, height: f64, time: f64) {
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
    let _ = context.fill_text("Waiting for the world session...", 32.0, 76.0);
}

#[allow(deprecated)]
fn draw_snapshot(
    context: &CanvasRenderingContext2d,
    width: f64,
    height: f64,
    snapshot: &SnapshotView,
) {
    let [origin_x, origin_y, origin_z] = snapshot.position.unwrap_or([0.0, 0.0, 0.0]);
    let center_x = width * 0.5;
    let center_y = height * 0.5;
    let scale = 4.0;
    let (look_x, look_y, _) = INPUT.with(|input| input.borrow().look());

    context.set_fill_style(&JsValue::from_str("#07111f"));
    context.fill_rect(0.0, 0.0, width, height);

    context.set_stroke_style(&JsValue::from_str("rgba(156, 220, 205, 0.16)"));
    context.set_line_width(1.0);
    for i in -8..=8 {
        let offset = f64::from(i) * 64.0 * scale;
        context.begin_path();
        context.move_to(center_x + offset, 0.0);
        context.line_to(center_x + offset, height);
        context.stroke();

        context.begin_path();
        context.move_to(0.0, center_y + offset);
        context.line_to(width, center_y + offset);
        context.stroke();
    }

    for entity in &snapshot.entities {
        let dx = (entity.position[0] - origin_x) * scale;
        let dy = (entity.position[1] - origin_y) * scale;
        let x = center_x + dx;
        let y = center_y - dy;

        if x < -20.0 || x > width + 20.0 || y < -20.0 || y > height + 20.0 {
            continue;
        }

        let (radius, color) = if entity.is_self {
            (8.0, "#e7f7ff")
        } else if entity.kind == "player" {
            (6.0, "#3ab49b")
        } else {
            (4.0, "#d8b15f")
        };

        context.begin_path();
        context.set_fill_style(&JsValue::from_str(color));
        let _ = context.arc(x, y, radius, 0.0, std::f64::consts::TAU);
        context.fill();

        if !entity.is_self
            && let Some(health) = &entity.health
        {
            draw_entity_health_bar(context, x, y - radius - 10.0, health);
        }

        if entity.is_self || entity.kind == "player" {
            if let Some(name) = &entity.name {
                context.set_fill_style(&JsValue::from_str("rgba(231, 247, 255, 0.82)"));
                context.set_font("12px system-ui, sans-serif");
                let _ = context.fill_text(name, x + 10.0, y - 10.0);
            }
        }
    }

    context.set_stroke_style(&JsValue::from_str("rgba(231, 247, 255, 0.86)"));
    context.set_line_width(2.0);
    context.begin_path();
    context.move_to(center_x, center_y);
    context.line_to(
        center_x + f64::from(look_x) * 34.0,
        center_y - f64::from(look_y) * 34.0,
    );
    context.stroke();

    context.set_fill_style(&JsValue::from_str("#e7f7ff"));
    context.set_font("700 20px system-ui, sans-serif");
    let state = if snapshot.in_game {
        "in game"
    } else {
        "joining"
    };
    let _ = context.fill_text(&format!("{} - {state}", snapshot.username), 28.0, 38.0);

    context.set_fill_style(&JsValue::from_str("rgba(231, 247, 255, 0.72)"));
    context.set_font("13px system-ui, sans-serif");
    let _ = context.fill_text(
        &format!(
            "{} online | {:.1}, {:.1}, {:.1}",
            snapshot.players_online, origin_x, origin_y, origin_z
        ),
        28.0,
        62.0,
    );

    if let Some(health) = &snapshot.health {
        draw_stat_bar(context, 28.0, 82.0, 184.0, "Health", health, "#d95f5f");
    }

    if let Some(energy) = &snapshot.energy {
        draw_stat_bar(context, 28.0, 108.0, 184.0, "Energy", energy, "#d8b15f");
    }
}

#[allow(deprecated)]
fn draw_stat_bar(
    context: &CanvasRenderingContext2d,
    x: f64,
    y: f64,
    width: f64,
    label: &str,
    stat: &StatView,
    color: &str,
) {
    let height = 12.0;
    context.set_fill_style(&JsValue::from_str("rgba(0, 0, 0, 0.36)"));
    context.fill_rect(x, y, width, height);

    context.set_fill_style(&JsValue::from_str(color));
    context.fill_rect(x, y, width * stat.fraction, height);

    context.set_stroke_style(&JsValue::from_str("rgba(231, 247, 255, 0.34)"));
    context.set_line_width(1.0);
    context.stroke_rect(x, y, width, height);

    context.set_fill_style(&JsValue::from_str("rgba(231, 247, 255, 0.86)"));
    context.set_font("12px system-ui, sans-serif");
    let _ = context.fill_text(
        &format!(
            "{label} {:.0}/{:.0}",
            stat.current.max(0.0),
            stat.maximum.max(0.0)
        ),
        x + width + 10.0,
        y + 10.0,
    );
}

#[allow(deprecated)]
fn draw_entity_health_bar(
    context: &CanvasRenderingContext2d,
    center_x: f64,
    y: f64,
    health: &StatView,
) {
    let width = 34.0;
    let height = 4.0;
    let x = center_x - width * 0.5;

    context.set_fill_style(&JsValue::from_str("rgba(0, 0, 0, 0.48)"));
    context.fill_rect(x, y, width, height);
    context.set_fill_style(&JsValue::from_str("#d95f5f"));
    context.fill_rect(x, y, width * health.fraction, height);
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
