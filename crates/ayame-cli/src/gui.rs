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
use tao::window::WindowBuilder;
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
        .with_title("Ayame — 菖蒲")
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
