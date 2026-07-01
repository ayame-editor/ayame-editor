//! `ayame gui` — the native desktop window.
//!
//! There is no second UI here: we start the same local editor server used by
//! `ayame serve` (on an ephemeral loopback port, in a background thread) and
//! point an OS-native webview at it. The window is WKWebView on macOS, WebView2
//! on Windows, and WebKitGTK on Linux — the platform's own engine, not a
//! bundled browser — so the desktop app stays small and reuses the entire web
//! front-end and every `/api/*` endpoint unchanged.

use std::sync::Arc;

use anyhow::{Context, Result};
use tao::dpi::LogicalSize;
use tao::event::{Event, WindowEvent};
use tao::event_loop::{ControlFlow, EventLoop};
use tao::window::{Icon, WindowBuilder};
use wry::WebViewBuilder;

use crate::parse;

pub fn cmd_gui(args: &[String]) -> Result<()> {
    // Same file-opening options as `serve`; the window opens empty if no FILE.
    let (pos, opts, flags) = parse(args, &["--encoding", "--stride", "--cache-dir"]);
    let state = Arc::new(crate::serve::build_state(&pos, &opts, &flags)?);

    // Bring the editor up behind the window and learn its loopback address.
    let addr = crate::serve::spawn_background(state)?;
    let url = format!("http://{addr}/");
    eprintln!("ayame: native window → {url}");

    let event_loop = EventLoop::new();
    let window = WindowBuilder::new()
        .with_title("Ayame")
        .with_window_icon(app_icon())
        .with_inner_size(LogicalSize::new(1280.0, 800.0))
        .build(&event_loop)
        .context("creating window")?;

    let builder = WebViewBuilder::new().with_url(&url);
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
    event_loop.run(move |event, _, control_flow| {
        *control_flow = ControlFlow::Wait;
        let _ = &webview;
        if let Event::WindowEvent {
            event: WindowEvent::CloseRequested,
            ..
        } = event
        {
            *control_flow = ControlFlow::Exit;
        }
    })
}

/// The window/taskbar icon: a flat, single-colour iris (菖蒲), drawn to RGBA in
/// code so it needs no image crate or bundled asset — no gradient, no tile,
/// transparent background. Mirrors `web/favicon.svg`. (Windows/Linux window
/// icon; the macOS dock icon comes from the .app bundle's .icns instead.)
fn app_icon() -> Option<Icon> {
    const N: u32 = 64;
    let mut px = vec![0u8; (N * N * 4) as usize];

    // Iris petals as filled ellipses (cx, cy, rx, ry) in the 64px grid:
    // three upright "standards" on top, three drooping "falls" below.
    let petals: [(f32, f32, f32, f32); 6] = [
        (32.0, 20.0, 6.5, 14.0),
        (22.5, 23.0, 5.5, 12.0),
        (41.5, 23.0, 5.5, 12.0),
        (32.0, 42.0, 7.0, 13.0),
        (21.0, 40.0, 5.5, 11.0),
        (43.0, 40.0, 5.5, 11.0),
    ];
    let in_ellipse = |x: f32, y: f32, cx: f32, cy: f32, rx: f32, ry: f32| {
        let (dx, dy) = ((x - cx) / rx, (y - cy) / ry);
        dx * dx + dy * dy <= 1.0
    };

    for y in 0..N {
        for x in 0..N {
            let (fx, fy) = (x as f32 + 0.5, y as f32 + 0.5);
            let hit = petals
                .iter()
                .any(|&(cx, cy, rx, ry)| in_ellipse(fx, fy, cx, cy, rx, ry));
            if !hit {
                continue; // transparent
            }
            let i = ((y * N + x) * 4) as usize;
            px[i] = 0x7A; // flat iris purple #7A5CC0
            px[i + 1] = 0x5C;
            px[i + 2] = 0xC0;
            px[i + 3] = 0xff;
        }
    }
    Icon::from_rgba(px, N, N).ok()
}
