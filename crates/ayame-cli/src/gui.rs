//! `ayame gui` — the native desktop window.
//!
//! There is no second UI here: we start the same local editor server used by
//! `ayame serve` (on an ephemeral loopback port, in a background thread) and
//! point an OS-native webview at it. The window is WKWebView on macOS, WebView2
//! on Windows, and WebKitGTK on Linux — the platform's own engine, not a
//! bundled browser — so the desktop app stays small and reuses the entire web
//! front-end and every `/api/*` endpoint unchanged.

use std::path::Path;
use std::sync::Arc;
use std::time::{Duration, Instant};

use anyhow::{Context, Result};
use tao::dpi::LogicalSize;
use tao::event::{Event, StartCause, WindowEvent};
use tao::event_loop::{ControlFlow, EventLoopBuilder};
use tao::window::{Icon, WindowBuilder};
use wry::{http::Request, DragDropEvent, WebViewBuilder};

use crate::parse;

pub fn cmd_gui(args: &[String]) -> Result<()> {
    // Same file-opening options as `serve`; the window opens empty if no FILE.
    let (pos, opts, flags) = parse(args, &["--encoding", "--stride", "--cache-dir"]);
    let title = initial_window_title(&pos);

    // A plain FILE argument is opened asynchronously from the page (via
    // /api/open) so the window appears immediately instead of after the first
    // index build of a huge file. Explicit open options (--encoding etc.)
    // still take the synchronous path because /api/open does not carry them.
    let async_open = !pos.is_empty() && opts.is_empty();
    let state = if async_open {
        let no_file: Vec<String> = Vec::new();
        Arc::new(crate::serve::build_state(&no_file, &opts, &flags)?)
    } else {
        Arc::new(crate::serve::build_state(&pos, &opts, &flags)?)
    };
    let pending_open = if async_open {
        pos.first().cloned()
    } else {
        None
    };

    // Bring the editor up behind the window and learn its loopback address.
    let addr = crate::serve::spawn_background(state)?;
    let url = format!("http://{addr}/");
    eprintln!("ayame: native window → {url}");

    let event_loop = EventLoopBuilder::<GuiEvent>::with_user_event().build();
    let proxy = event_loop.create_proxy();
    let ipc_proxy = proxy.clone();
    // Created hidden: the page reveals it with "ayame:ready" (fallback timer
    // below), which removes the white flash before first paint.
    let window = WindowBuilder::new()
        .with_title(title)
        .with_window_icon(app_icon())
        .with_inner_size(LogicalSize::new(1280.0, 800.0))
        .with_min_inner_size(LogicalSize::new(900.0, 560.0))
        .with_visible(false)
        .build(&event_loop)
        .context("creating window")?;

    let init_script = match &pending_open {
        Some(p) => {
            // Absolute-ize so the server resolves the path regardless of cwd.
            let abs = std::fs::canonicalize(p)
                .map(|x| x.to_string_lossy().into_owned())
                .unwrap_or_else(|_| p.clone());
            format!(
                "window.__ayamePendingOpen = {};",
                serde_json::to_string(&abs).unwrap_or_else(|_| "\"\"".into())
            )
        }
        None => String::new(),
    };

    let dnd_proxy = proxy.clone();
    let builder = WebViewBuilder::new()
        .with_url(&url)
        // Paper-tone backdrop while the page loads (matches the default theme).
        .with_background_color((251, 248, 241, 255))
        .with_initialization_script(&init_script)
        // Native drops hand the editor real paths: the file is mmap'd in place
        // instead of the DOM fallback that uploads a full temp copy.
        .with_drag_drop_handler(move |event| {
            if let DragDropEvent::Drop { paths, .. } = event {
                let paths: Vec<String> = paths
                    .iter()
                    .map(|p| p.to_string_lossy().into_owned())
                    .collect();
                if !paths.is_empty() {
                    let _ = dnd_proxy.send_event(GuiEvent::OpenPaths(paths));
                }
            }
            true // consume all drag events: never fall back to the upload path
        })
        .with_ipc_handler(move |req: Request<String>| match req.body().as_str() {
            "ayame:close-ok" => {
                let _ = ipc_proxy.send_event(GuiEvent::CloseConfirmed);
            }
            "ayame:close-cancel" => {
                let _ = ipc_proxy.send_event(GuiEvent::CloseCanceled);
            }
            "ayame:ready" => {
                let _ = ipc_proxy.send_event(GuiEvent::Ready);
            }
            msg => {
                if let Some(title) = msg.strip_prefix("ayame:title:") {
                    let _ = ipc_proxy.send_event(GuiEvent::SetTitle(clean_window_title(title)));
                }
            }
        });
    // On macOS/Windows the webview attaches to the native window handle; on
    // Linux the webview must live inside the window's GTK container.
    #[cfg(not(target_os = "linux"))]
    let webview = builder.build(&window).context("creating webview")?;
    #[cfg(target_os = "linux")]
    let webview = {
        use tao::platform::unix::WindowExtUnix;
        use wry::WebViewBuilderExtUnix;
        let vbox = window
            .default_vbox()
            .context("window has no GTK container")?;
        builder.build_gtk(vbox).context("creating webview")?
    };

    // Keep the webview alive for the lifetime of the window.
    let mut close_pending = false;
    let mut shown = false;
    // Show even if the page never reports ready (server error, slow webview),
    // so the user always gets a window to look at.
    let show_deadline = Instant::now() + Duration::from_millis(2000);
    event_loop.run(move |event, _, control_flow| {
        *control_flow = if shown {
            ControlFlow::Wait
        } else {
            ControlFlow::WaitUntil(show_deadline)
        };
        let _ = &webview;
        match event {
            Event::NewEvents(StartCause::ResumeTimeReached { .. }) => {
                if !shown {
                    shown = true;
                    window.set_visible(true);
                }
            }
            Event::WindowEvent {
                event: WindowEvent::CloseRequested,
                ..
            } => {
                if close_pending {
                    return;
                }
                close_pending = true;
                if webview.evaluate_script(NATIVE_CLOSE_SCRIPT).is_err() {
                    *control_flow = ControlFlow::Exit;
                }
            }
            Event::UserEvent(GuiEvent::CloseConfirmed) => {
                *control_flow = ControlFlow::Exit;
            }
            Event::UserEvent(GuiEvent::CloseCanceled) => {
                close_pending = false;
            }
            Event::UserEvent(GuiEvent::SetTitle(title)) => {
                window.set_title(&title);
            }
            Event::UserEvent(GuiEvent::Ready) => {
                if !shown {
                    shown = true;
                    window.set_visible(true);
                }
            }
            Event::UserEvent(GuiEvent::OpenPaths(paths)) => {
                if let Ok(json) = serde_json::to_string(&paths) {
                    let js = format!(
                        "window.__ayameOpenNativePaths && window.__ayameOpenNativePaths({json});"
                    );
                    let _ = webview.evaluate_script(&js);
                }
            }
            _ => {}
        }
    })
}

#[derive(Debug)]
enum GuiEvent {
    CloseConfirmed,
    CloseCanceled,
    SetTitle(String),
    Ready,
    OpenPaths(Vec<String>),
}

const NATIVE_CLOSE_SCRIPT: &str = r#"
if (window.__ayameNativeCloseRequested) {
  window.__ayameNativeCloseRequested();
} else if (window.ipc && window.ipc.postMessage) {
  window.ipc.postMessage("ayame:close-ok");
}
"#;

fn initial_window_title(pos: &[String]) -> String {
    match pos.first() {
        Some(path) => format!("{} - Ayame Editor", path_display_name(path)),
        None => "Ayame Editor".to_string(),
    }
}

fn path_display_name(path: &str) -> String {
    Path::new(path)
        .file_name()
        .map(|s| s.to_string_lossy().to_string())
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| path.to_string())
}

fn clean_window_title(title: &str) -> String {
    let clean: String = title
        .chars()
        .filter(|c| !c.is_control())
        .take(256)
        .collect();
    if clean.trim().is_empty() {
        "Ayame Editor".to_string()
    } else {
        clean
    }
}

/// One petal ellipse: (cx, cy, rx, ry, rotation radians, r, g, b).
type Petal = (f32, f32, f32, f32, f32, u8, u8, u8);

/// The window/taskbar icon mirrors `web/favicon.svg`: the purple Ayame Editor
/// flower mark on a transparent background. It is drawn to RGBA so the native
/// GUI stays dependency-free.
fn app_icon() -> Option<Icon> {
    const N: u32 = 64;
    let mut px = vec![0u8; (N * N * 4) as usize];

    // Ordered back-to-front.
    let petals: [Petal; 6] = [
        (32.0, 18.0, 6.0, 13.2, 0.0, 0xA9, 0x92, 0xE0),
        (23.0, 27.0, 5.8, 13.0, -0.58, 0x9B, 0x82, 0xD8),
        (41.0, 27.0, 5.8, 13.0, 0.58, 0x79, 0x5F, 0xC3),
        (24.0, 43.0, 6.2, 13.5, 0.47, 0x8E, 0x73, 0xCF),
        (40.0, 43.0, 6.2, 13.5, -0.47, 0x6F, 0x56, 0xB8),
        (32.0, 38.0, 6.4, 11.8, 0.0, 0x67, 0x4F, 0xAF),
    ];

    let in_ellipse = |x: f32, y: f32, cx: f32, cy: f32, rx: f32, ry: f32, rot: f32| {
        let (sin, cos) = rot.sin_cos();
        let dx0 = x - cx;
        let dy0 = y - cy;
        let dx = (dx0 * cos + dy0 * sin) / rx;
        let dy = (-dx0 * sin + dy0 * cos) / ry;
        dx * dx + dy * dy <= 1.0
    };

    for y in 0..N {
        for x in 0..N {
            let mut dst = [0.0f32; 4];
            for &(cx, cy, rx, ry, rot, r, g, b) in &petals {
                let mut hits = 0.0;
                for sy in 0..4 {
                    for sx in 0..4 {
                        let fx = x as f32 + (sx as f32 + 0.5) / 4.0;
                        let fy = y as f32 + (sy as f32 + 0.5) / 4.0;
                        if in_ellipse(fx, fy, cx, cy, rx, ry, rot) {
                            hits += 1.0;
                        }
                    }
                }
                let src_a = hits / 16.0;
                if src_a <= 0.0 {
                    continue;
                }
                let src = [r as f32 / 255.0, g as f32 / 255.0, b as f32 / 255.0];
                let out_a = src_a + dst[3] * (1.0 - src_a);
                for c in 0..3 {
                    dst[c] = (src[c] * src_a + dst[c] * dst[3] * (1.0 - src_a)) / out_a;
                }
                dst[3] = out_a;
            }

            let i = ((y * N + x) * 4) as usize;
            px[i] = (dst[0] * 255.0).round() as u8;
            px[i + 1] = (dst[1] * 255.0).round() as u8;
            px[i + 2] = (dst[2] * 255.0).round() as u8;
            px[i + 3] = (dst[3] * 255.0).round() as u8;
        }
    }
    Icon::from_rgba(px, N, N).ok()
}
