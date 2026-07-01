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

/// The window/taskbar icon, drawn to RGBA in code so it needs no image crate or
/// bundled asset. It mirrors `web/favicon.svg`: a rounded blue tile with three
/// white "text" bars and a pink edit caret. (Used on Windows/Linux; the macOS
/// dock icon comes from the .app bundle's .icns instead.)
fn app_icon() -> Option<Icon> {
    const N: u32 = 64;
    const R: f32 = 13.0; // corner radius, in the 64px grid
    let mut px = vec![0u8; (N * N * 4) as usize];

    // Rounded-rect membership: inside [4,60), with rounded corners of radius R.
    let inside_tile = |x: f32, y: f32| -> bool {
        if !(4.0..60.0).contains(&x) || !(4.0..60.0).contains(&y) {
            return false;
        }
        let cx = x.clamp(4.0 + R, 60.0 - R);
        let cy = y.clamp(4.0 + R, 60.0 - R);
        let (dx, dy) = (x - cx, y - cy);
        dx * dx + dy * dy <= R * R
    };
    // Filled-rect helper for the bars/caret.
    let in_rect = |x: f32, y: f32, x0: f32, y0: f32, x1: f32, y1: f32| {
        (x0..x1).contains(&x) && (y0..y1).contains(&y)
    };

    for y in 0..N {
        for x in 0..N {
            let (fx, fy) = (x as f32 + 0.5, y as f32 + 0.5);
            if !inside_tile(fx, fy) {
                continue; // transparent outside the tile
            }
            // Vertical gradient #2a7fc0 → #0e639c.
            let t = ((fy - 4.0) / 56.0).clamp(0.0, 1.0);
            let lerp = |a: u8, b: u8| (a as f32 + (b as f32 - a as f32) * t) as u8;
            let mut rgb = [lerp(0x2a, 0x0e), lerp(0x7f, 0x63), lerp(0xc0, 0x9c)];
            // Pink caret bar.
            if in_rect(fx, fy, 44.0, 16.0, 48.0, 48.0) {
                rgb = [0xe8, 0xa0, 0xbf];
            }
            // Three white text bars.
            else if in_rect(fx, fy, 15.0, 20.0, 39.0, 24.5)
                || in_rect(fx, fy, 15.0, 30.0, 47.0, 34.5)
                || in_rect(fx, fy, 15.0, 40.0, 34.0, 44.5)
            {
                rgb = [0xff, 0xff, 0xff];
            }
            let i = ((y * N + x) * 4) as usize;
            px[i] = rgb[0];
            px[i + 1] = rgb[1];
            px[i + 2] = rgb[2];
            px[i + 3] = 0xff;
        }
    }
    Icon::from_rgba(px, N, N).ok()
}
