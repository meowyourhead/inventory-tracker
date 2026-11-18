//! Inventory — portable desktop application.
//!
//! A 100% Rust host process (no Node, no Electron, no console window) around
//! the original HTML user interface. The UI is served from a private custom
//! protocol and talks to the Rust backend through the same `window.desk` API
//! that the Electron preload script exposed, so every existing feature works
//! unchanged.

#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod state;
mod update;
mod xlsm;

use std::{
    borrow::Cow,
    fs,
    path::PathBuf,
    sync::Arc,
    thread,
    time::Duration,
};

use base64::Engine;
use serde_json::{json, Value};
use tao::{
    dpi::LogicalSize,
    event::{Event, WindowEvent},
    event_loop::{ControlFlow, EventLoopBuilder},
    window::{Icon, ResizeDirection, Window, WindowBuilder},
};
use wry::{
    http::{header::CONTENT_TYPE, Request, Response},
    RequestAsyncResponder, WebContext, WebViewBuilder,
};

use state::{AppEvent, AppState, BASE_H, BASE_W, MIN_H, MIN_W};

#[cfg(windows)]
use tao::platform::windows::{WindowBuilderExtWindows, WindowExtWindows};

const INDEX_HTML: &[u8] = include_bytes!("../ui/index.html");
const ICON_PNG: &[u8] = include_bytes!("../assets/icon.png");
const CYBERPUNK_LOGO: &[u8] = include_bytes!("../assets/cyberpunk-logo.svg");
const GENSHIN_LOGO: &[u8] = include_bytes!("../assets/genshin-logo.svg");
const ZZZ_LOGO: &[u8] = include_bytes!("../assets/zzz-logo.png");
const UI_FONT: &[u8] = include_bytes!("../zh-cn.ttf");
const RAJDHANI_500: &[u8] = include_bytes!("../fonts/rajdhani-latin-500-normal.woff2");
const RAJDHANI_600: &[u8] = include_bytes!("../fonts/rajdhani-latin-600-normal.woff2");
const RAJDHANI_700: &[u8] = include_bytes!("../fonts/rajdhani-latin-700-normal.woff2");
const SHARE_TECH_MONO: &[u8] = include_bytes!("../fonts/share-tech-mono-latin-400-normal.woff2");
const ALEGREYA_400: &[u8] = include_bytes!("../fonts/alegreya-sans-latin-400-normal.woff2");
const ALEGREYA_500: &[u8] = include_bytes!("../fonts/alegreya-sans-latin-500-normal.woff2");
const ALEGREYA_700: &[u8] = include_bytes!("../fonts/alegreya-sans-latin-700-normal.woff2");
// Claude theme: open (OFL) stand-ins for claude.ai's proprietary Anthropic
// Sans / Anthropic Serif. Both are variable fonts (wght, plus opsz for serif).
const HANKEN_GROTESK: &[u8] = include_bytes!("../fonts/hanken-grotesk-latin-wght-normal.woff2");
const SOURCE_SERIF_4: &[u8] = include_bytes!("../fonts/source-serif-4-latin-opsz-normal.woff2");
const CURSOR_DEFAULT: &[u8] = include_bytes!("../cursors/gi_arrow.cur");
const CURSOR_LINK: &[u8] = include_bytes!("../cursors/gi_link.cur");
const CURSOR_BUSY: &[u8] = include_bytes!("../cursors/gi_busy.cur");
const CURSOR_WORKING: &[u8] = include_bytes!("../cursors/gi_working.cur");

fn main() {
    if let Err(error) = run() {
        show_error(&error);
        std::process::exit(1);
    }
}

/// `--updated-from <pid> <version>`: passed by the updater when it relaunches
/// the app. Waits for the old process to exit before opening the window.
fn updated_from_args() -> Option<String> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let [flag, pid, version, ..] = args.as_slice() else {
        return None;
    };
    if flag != "--updated-from" {
        return None;
    }
    if let Ok(pid) = pid.parse::<u32>() {
        update::wait_for_process_exit(pid, Duration::from_secs(15));
    }
    update::Version::parse(version).map(|v| v.to_string())
}

/// Relaunches the (just replaced) executable.
fn relaunch() {
    let Ok(exe) = std::env::current_exe() else {
        return;
    };
    let mut command = std::process::Command::new(&exe);
    command
        .arg("--updated-from")
        .arg(std::process::id().to_string())
        .arg(update::CURRENT);
    if let Some(dir) = exe.parent() {
        command.current_dir(dir);
    }
    if let Err(e) = command.spawn() {
        show_error(&format!(
            "Inventory was updated but could not restart automatically.\n\nPlease start it again.\n\n{e}"
        ));
    }
}

fn run() -> Result<(), String> {
    let updated_from = updated_from_args();

    let mut event_loop_builder = EventLoopBuilder::<AppEvent>::with_user_event();
    let event_loop = event_loop_builder.build();

    let state = Arc::new(AppState::new(event_loop.create_proxy()));
    *state
        .updated_from
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner) = updated_from;
    state.ensure_ready()?;
    state.prewarm();

    let webview_data_dir = webview_data_dir();
    let mut web_context = WebContext::new(Some(webview_data_dir));

    // Decode the PNG once; it is used for both the window and taskbar icon.
    let icon = load_icon();
    let mut window_builder = WindowBuilder::new()
        .with_title("Inventory")
        .with_inner_size(LogicalSize::new(BASE_W, BASE_H))
        .with_min_inner_size(LogicalSize::new(MIN_W, MIN_H))
        .with_maximized(true)
        .with_decorations(false)
        .with_visible(false)
        .with_window_icon(icon.clone());
    // Keep the drop shadow for borderless windows on Windows.
    #[cfg(windows)]
    {
        // `with_window_icon` only sets the small (title-bar) icon in tao; the
        // taskbar uses the big icon. Set it explicitly so other PCs do not
        // fall back to a cached/generic taskbar icon.
        window_builder = window_builder
            .with_taskbar_icon(icon)
            .with_undecorated_shadow(true);
    }
    let window = window_builder
        .build(&event_loop)
        .map_err(|e| format!("Cannot create the application window:\n{e}"))?;

    state.set_hwnd(window.hwnd());

    let protocol_state = Arc::clone(&state);
    let show_state = Arc::clone(&state);
    let devtools = std::env::var("INVENTORY_DEVTOOLS")
        .map(|v| v == "1")
        .unwrap_or(false);

    let builder = WebViewBuilder::new_with_web_context(&mut web_context)
        .with_background_color((245, 247, 250, 255))
        .with_asynchronous_custom_protocol(
            "inventory".into(),
            move |_webview_id, request, responder| {
                serve(Arc::clone(&protocol_state), request, responder);
            },
        )
        .with_url("inventory://localhost/index.html")
        // Never navigate away from the app or open popups: any page that
        // got loaded here could otherwise call the local /api/ backend.
        .with_navigation_handler(|url| is_app_url(&url))
        .with_new_window_req_handler(|_url, _features| wry::NewWindowResponse::Deny)
        .with_devtools(devtools)
        .with_on_page_load_handler(move |event, _url| {
            if matches!(event, wry::PageLoadEvent::Finished) {
                show_state.show();
            }
        });

    let webview = builder.build(&window).map_err(|e| {
        format!(
            "Cannot start the embedded browser (Microsoft Edge WebView2).\n\n{e}\n\n\
             Install the Microsoft Edge WebView2 Runtime and try again."
        )
    })?;

    state.start_watch();

    // Safety net: never leave the user with an invisible window if the page
    // load event is missed for any reason.
    {
        let state = Arc::clone(&state);
        thread::spawn(move || {
            thread::sleep(Duration::from_secs(6));
            state.show();
        });
    }

    event_loop.run(move |event, _target, control_flow| {
        *control_flow = ControlFlow::Wait;
        match event {
            Event::WindowEvent {
                event: WindowEvent::CloseRequested,
                ..
            } => {
                state.cleanup();
                *control_flow = ControlFlow::Exit;
            }
            // Sent by the page-load handler and by the 6 s safety net; only
            // the first one may maximize/focus, otherwise the safety net
            // would undo a restore-down or steal focus a few seconds later.
            Event::UserEvent(AppEvent::Show) if !window.is_visible() => {
                window.set_maximized(true);
                window.set_visible(true);
                window.set_focus();
            }
            Event::UserEvent(AppEvent::Restart) => {
                // Same shutdown path as closing the window (auto-backup,
                // lock release), then start the updated executable.
                state.cleanup();
                relaunch();
                *control_flow = ControlFlow::Exit;
            }
            Event::UserEvent(AppEvent::Resize { factor, reply }) => {
                let _ = reply.send(resize_window(&window, factor));
            }
            Event::UserEvent(AppEvent::Window { command, reply }) => {
                let mut maximized = window.is_maximized();
                match command {
                    "minimize" => window.set_minimized(true),
                    "toggle-maximize" => {
                        maximized = !maximized;
                        window.set_maximized(maximized);
                    }
                    "drag" => {
                        let _ = window.drag_window();
                        maximized = window.is_maximized();
                    }
                    "close" => {
                        state.cleanup();
                        if let Some(reply) = reply {
                            let _ = reply.send(json!({ "ok": true }));
                        }
                        // Give the HTTP responder a moment to flush before exit.
                        thread::sleep(Duration::from_millis(80));
                        *control_flow = ControlFlow::Exit;
                        return;
                    }
                    _ => {}
                }
                if let Some(reply) = reply {
                    let _ = reply.send(json!({ "ok": true, "maximized": maximized }));
                }
            }
            Event::UserEvent(AppEvent::WindowResize { direction, reply }) => {
                if !window.is_maximized() {
                    let _ = window.drag_resize_window(direction);
                }
                let maximized = window.is_maximized();
                let _ = reply.send(json!({ "ok": true, "maximized": maximized }));
            }
            Event::UserEvent(AppEvent::Push { channel, payload }) => {
                let channel = serde_json::to_string(channel).unwrap_or_else(|_| "\"\"".into());
                let payload = serde_json::to_string(&payload).unwrap_or_else(|_| "null".into());
                let script =
                    format!("window.__deskPush&&window.__deskPush({channel},{payload});");
                let _ = webview.evaluate_script(&script);
            }
            _ => {}
        }
        // Keep the WebView2 context alive for as long as the event loop runs.
        let _ = &web_context;
    });
}

// -------------------------------------------------------------- protocol --

/// Embedded files served from the private `inventory://` origin.
fn static_asset(path: &str) -> Option<(&'static [u8], &'static str)> {
    Some(match path {
        "/build/icon.png" => (ICON_PNG, "image/png"),
        "/theme/cyberpunk-logo.svg" => (CYBERPUNK_LOGO, "image/svg+xml"),
        "/theme/genshin-logo.svg" => (GENSHIN_LOGO, "image/svg+xml"),
        "/theme/zzz-logo.png" => (ZZZ_LOGO, "image/png"),
        "/fonts/zh-cn.ttf" => (UI_FONT, "font/ttf"),
        "/fonts/rajdhani-500.woff2" => (RAJDHANI_500, "font/woff2"),
        "/fonts/rajdhani-600.woff2" => (RAJDHANI_600, "font/woff2"),
        "/fonts/rajdhani-700.woff2" => (RAJDHANI_700, "font/woff2"),
        "/fonts/share-tech-mono-400.woff2" => (SHARE_TECH_MONO, "font/woff2"),
        "/fonts/alegreya-sans-400.woff2" => (ALEGREYA_400, "font/woff2"),
        "/fonts/alegreya-sans-500.woff2" => (ALEGREYA_500, "font/woff2"),
        "/fonts/alegreya-sans-700.woff2" => (ALEGREYA_700, "font/woff2"),
        "/fonts/hanken-grotesk.woff2" => (HANKEN_GROTESK, "font/woff2"),
        "/fonts/source-serif-4.woff2" => (SOURCE_SERIF_4, "font/woff2"),
        "/cursors/gi_arrow.cur" => (CURSOR_DEFAULT, "image/x-icon"),
        "/cursors/gi_link.cur" => (CURSOR_LINK, "image/x-icon"),
        "/cursors/gi_busy.cur" => (CURSOR_BUSY, "image/x-icon"),
        "/cursors/gi_working.cur" => (CURSOR_WORKING, "image/x-icon"),
        _ => return None,
    })
}

/// Only the app's own private origin may be shown in the WebView. On Windows
/// wry maps `inventory://localhost/` to `http://inventory.localhost/`.
fn is_app_url(url: &str) -> bool {
    url.starts_with("inventory://localhost/")
        || url.starts_with("http://inventory.localhost/")
        || url.starts_with("https://inventory.localhost/")
}

fn serve(state: Arc<AppState>, request: Request<Vec<u8>>, responder: RequestAsyncResponder) {
    let path = request.uri().path();

    if matches!(path, "/" | "/index.html") {
        responder.respond(html_response());
        return;
    }
    if let Some((bytes, content_type)) = static_asset(path) {
        responder.respond(static_response(bytes, content_type));
        return;
    }

    let Some(method) = path.strip_prefix("/api/").map(str::to_string) else {
        responder.respond(not_found_response());
        return;
    };
    // The API changes data and the lock: accept only POSTs (what the UI
    // bridge sends), never a plain navigation or <img src> GET.
    if request.method() != wry::http::Method::POST {
        responder.respond(not_found_response());
        return;
    }

    let body = request.into_body();
    thread::spawn(move || {
        let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            dispatch(&state, &method, &body)
        }))
        .unwrap_or_else(|_| encode(&json!({ "ok": false, "error": "Internal backend error" })));
        responder.respond(json_response(result));
    });
}

fn encode(value: &Value) -> Vec<u8> {
    serde_json::to_vec(value)
        .unwrap_or_else(|_| br#"{"ok":false,"error":"Could not encode response"}"#.to_vec())
}

fn dispatch(state: &Arc<AppState>, method: &str, body: &[u8]) -> Vec<u8> {
    // `load` returns the whole inventory: serialize it without first
    // copying it into a temporary `Value`.
    if method == "load" {
        return state.load_response_bytes();
    }
    encode(&dispatch_value(state, method, body))
}

fn dispatch_value(state: &Arc<AppState>, method: &str, body: &[u8]) -> Value {
    match method {
        "save" => match serde_json::from_slice::<Value>(body) {
            Ok(data) => state.save(data),
            Err(e) => json!({ "ok": false, "error": e.to_string() }),
        },
        "activity" => state.activity(),
        "lock" => state.acquire_lock(),
        "unlock" => state.unlock(),
        "backup" => state.backup(),
        "backup-status" => state.backup_status(),
        "choose-backup-dir" => state.choose_backup_dir(),
        "open-backup-dir" => state.open_backup_dir(),
        "restore" => state.restore(),
        "resize" => {
            let factor = serde_json::from_slice::<Value>(body)
                .ok()
                .and_then(|value| value.get("factor").and_then(Value::as_f64))
                .unwrap_or(1.0);
            state.request_resize(factor)
        }
        "window-state" => state.window_state(),
        "window-minimize" => state.window_minimize(),
        "window-toggle-maximize" => state.window_toggle_maximize(),
        "window-close" => state.window_close(),
        "window-drag" => {
            state.window_drag();
            json!({ "ok": true })
        }
        "window-resize" => {
            let direction = serde_json::from_slice::<Value>(body)
                .ok()
                .and_then(|value| value.get("direction").and_then(Value::as_str).and_then(resize_direction));
            match direction {
                Some(direction) => state.window_resize(direction),
                None => json!({ "ok": false, "error": "Unknown resize direction" }),
            }
        }
        "read-xlsm" => read_xlsm(body),
        "update-info" => state.update_info(),
        "update-check" => state.update_check(),
        "update-install" => state.update_install(),
        _ => json!({ "ok": false, "error": "Unknown API method" }),
    }
}

fn read_xlsm(body: &[u8]) -> Value {
    let parsed: Value = match serde_json::from_slice(body) {
        Ok(value) => value,
        Err(e) => return json!({ "ok": false, "error": e.to_string() }),
    };
    let Some(encoded) = parsed.get("data").and_then(Value::as_str) else {
        return json!({ "ok": false, "error": "Missing workbook data" });
    };
    let bytes = match base64::engine::general_purpose::STANDARD.decode(encoded) {
        Ok(bytes) => bytes,
        Err(e) => return json!({ "ok": false, "error": e.to_string() }),
    };
    match xlsm::read_workbook(bytes) {
        Ok(workbook) => json!({ "ok": true, "workbook": workbook }),
        Err(e) => json!({ "ok": false, "error": e }),
    }
}

/// Defense in depth for the single-file UI: no remote scripts, frames,
/// plugins or form posts, and `fetch` may only reach the app's own origin.
/// Inline script/style stay allowed because the UI is one self-contained file.
const CSP: &str = "default-src 'self'; \
    script-src 'self' 'unsafe-inline'; \
    style-src 'self' 'unsafe-inline'; \
    img-src 'self' data: blob:; \
    font-src 'self' data:; \
    connect-src 'self'; \
    object-src 'none'; frame-src 'none'; base-uri 'none'; form-action 'none'";

fn html_response() -> Response<Cow<'static, [u8]>> {
    Response::builder()
        .status(200)
        .header(CONTENT_TYPE, "text/html; charset=utf-8")
        .header("Cache-Control", "no-store")
        .header("Content-Security-Policy", CSP)
        .header("Content-Length", INDEX_HTML.len().to_string())
        .body(Cow::Borrowed(INDEX_HTML))
        .expect("valid HTML response")
}

/// Embedded bytes are handed to WebView2 without copying (the 7 MB UI font
/// used to be duplicated on every page load).
fn static_response(bytes: &'static [u8], content_type: &'static str) -> Response<Cow<'static, [u8]>> {
    Response::builder()
        .status(200)
        .header(CONTENT_TYPE, content_type)
        .header("Cache-Control", "no-store")
        .header("Content-Length", bytes.len().to_string())
        .body(Cow::Borrowed(bytes))
        .expect("valid static response")
}

fn json_response(body: Vec<u8>) -> Response<Vec<u8>> {
    Response::builder()
        .status(200)
        .header(CONTENT_TYPE, "application/json; charset=utf-8")
        .header("Cache-Control", "no-store")
        .header("Content-Length", body.len().to_string())
        .body(body)
        .expect("valid JSON response")
}

fn not_found_response() -> Response<&'static [u8]> {
    Response::builder()
        .status(404)
        .header(CONTENT_TYPE, "text/plain; charset=utf-8")
        .body(&b"Not found"[..])
        .expect("valid 404 response")
}

// ---------------------------------------------------------------- window --

fn resize_direction(name: &str) -> Option<ResizeDirection> {
    Some(match name {
        "e" => ResizeDirection::East,
        "n" => ResizeDirection::North,
        "ne" => ResizeDirection::NorthEast,
        "nw" => ResizeDirection::NorthWest,
        "s" => ResizeDirection::South,
        "se" => ResizeDirection::SouthEast,
        "sw" => ResizeDirection::SouthWest,
        "w" => ResizeDirection::West,
        _ => return None,
    })
}

fn resize_window(window: &Window, factor: f64) -> Value {
    let factor = factor.clamp(0.85, 1.3);
    let scale = window.scale_factor();

    let (work_width, work_height) = monitor_work_area(window)
        .map(|(w, h)| (w as f64 / scale, h as f64 / scale))
        .or_else(|| {
            window.current_monitor().map(|monitor| {
                let size = monitor.size();
                (
                    size.width as f64 / monitor.scale_factor(),
                    size.height as f64 / monitor.scale_factor(),
                )
            })
        })
        .unwrap_or((BASE_W, BASE_H));

    let width = (BASE_W * factor).max(MIN_W).min(work_width).round() as u32;
    let height = (BASE_H * factor).max(MIN_H).min(work_height).round() as u32;

    window.set_inner_size(LogicalSize::new(width, height));
    json!({ "ok": true, "width": width, "height": height })
}

#[cfg(windows)]
fn monitor_work_area(window: &Window) -> Option<(u32, u32)> {
    use tao::platform::windows::WindowExtWindows;
    use windows_sys::Win32::Graphics::Gdi::{
        GetMonitorInfoW, MonitorFromWindow, MONITORINFO, MONITOR_DEFAULTTONEAREST,
    };

    let hwnd = window.hwnd();
    // SAFETY: MONITORINFO is a plain C struct (integers and RECTs); all-zero
    // is a valid bit pattern.
    let mut info: MONITORINFO = unsafe { std::mem::zeroed() };
    info.cbSize = u32::try_from(std::mem::size_of::<MONITORINFO>()).ok()?;
    // SAFETY: `hwnd` is the live main window owned by this event loop.
    let monitor = unsafe { MonitorFromWindow(hwnd as _, MONITOR_DEFAULTTONEAREST) };
    if monitor.is_null() {
        return None;
    }
    // SAFETY: `monitor` is a valid HMONITOR and `info` is a correctly sized,
    // writable MONITORINFO that outlives the call.
    if unsafe { GetMonitorInfoW(monitor, &raw mut info) } == 0 {
        return None;
    }
    let width = info.rcWork.right.saturating_sub(info.rcWork.left);
    let height = info.rcWork.bottom.saturating_sub(info.rcWork.top);
    Some((u32::try_from(width).ok()?, u32::try_from(height).ok()?))
}

#[cfg(not(windows))]
fn monitor_work_area(_window: &Window) -> Option<(u32, u32)> {
    None
}

fn load_icon() -> Option<Icon> {
    let image = image::load_from_memory_with_format(ICON_PNG, image::ImageFormat::Png).ok()?;
    let image = image.to_rgba8();
    let (width, height) = image.dimensions();
    Icon::from_rgba(image.into_raw(), width, height).ok()
}

fn webview_data_dir() -> PathBuf {
    let base = std::env::var_os("LOCALAPPDATA")
        .map(PathBuf::from)
        .unwrap_or_else(std::env::temp_dir);
    let dir = base.join("Inventory").join("WebView2");
    let _ = fs::create_dir_all(&dir);
    dir
}

fn show_error(message: &str) {
    eprintln!("{message}");
    let _ = rfd::MessageDialog::new()
        .set_title("Inventory")
        .set_description(message)
        .set_level(rfd::MessageLevel::Error)
        .show();
}
