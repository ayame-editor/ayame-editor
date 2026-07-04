//! `ayame gui` — the native desktop window.
//!
//! There is no second UI here: we start the same local editor server used by
//! `ayame serve` (on an ephemeral loopback port, in a background thread) and
//! point an OS-native webview at it. The window is WKWebView on macOS, WebView2
//! on Windows, and WebKitGTK on Linux — the platform's own engine, not a
//! bundled browser — so the desktop app stays small and reuses the entire web
//! front-end and every `/api/*` endpoint unchanged.

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, Instant};

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use tao::dpi::{LogicalSize, PhysicalPosition, PhysicalSize};
use tao::event::{Event, StartCause, WindowEvent};
use tao::event_loop::{ControlFlow, EventLoopBuilder};
use tao::window::{Icon, Window, WindowBuilder};
use wry::{http::Request, DragDropEvent, WebContext, WebViewBuilder};

use crate::parse_checked;

pub fn cmd_gui(args: &[String]) -> Result<()> {
    // Same file-opening options as `serve`; the window opens empty if no FILE.
    let (pos, opts, flags) = parse_checked(
        args,
        &["--encoding", "--stride", "--cache-dir"],
        &["--no-cache"],
    )?;
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
    let saved_state = load_window_state();
    let start_maximized = saved_state.as_ref().is_some_and(|s| s.maximized);
    let builder = WindowBuilder::new()
        .with_title(title)
        .with_window_icon(app_icon())
        .with_min_inner_size(LogicalSize::new(900.0, 560.0))
        .with_visible(false);
    // Restore last session's geometry. Saved bounds are physical pixels (the
    // same units `save_window_state` captures), already sanity-clamped on load.
    let builder = match &saved_state {
        Some(s) => {
            let b = builder.with_inner_size(PhysicalSize::new(s.width, s.height));
            match (s.x, s.y) {
                (Some(x), Some(y)) => b.with_position(PhysicalPosition::new(x, y)),
                _ => b,
            }
        }
        None => builder.with_inner_size(LogicalSize::new(1280.0, 800.0)),
    };
    let window = builder.build(&event_loop).context("creating window")?;

    let init_script = match &pending_open {
        Some(p) => {
            // Absolute-ize so the server resolves the path regardless of cwd.
            // The Windows verbatim prefix canonicalize adds is stripped so it
            // never surfaces in tab tooltips or save dialogs.
            let abs = std::fs::canonicalize(p)
                .map(|x| crate::serve::workspace::display_path(&x))
                .unwrap_or_else(|_| p.clone());
            format!(
                "window.__ayamePendingOpen = {};",
                serde_json::to_string(&abs).unwrap_or_else(|_| "\"\"".into())
            )
        }
        None => String::new(),
    };

    // Pin the webview's profile data to our platform cache directory. Without
    // this, WebView2 on Windows drops an `ayame.exe.WebView2` folder next to
    // the executable; macOS (WKWebView -> ~/Library) and Linux (WebKitGTK ->
    // XDG dirs) already use system locations, and this keeps them under one
    // predictable `ayame` directory too. Must stay alive as long as the
    // webview, hence the binding out here.
    let web_data_dir = crate::default_cache_dir().map(|d| d.join("webview"));
    if let Some(dir) = &web_data_dir {
        let _ = std::fs::create_dir_all(dir);
    }
    let mut web_context = WebContext::new(web_data_dir);

    let dnd_proxy = proxy.clone();
    let builder = WebViewBuilder::new_with_web_context(&mut web_context)
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
            "ayame:new-window" => {
                let _ = ipc_proxy.send_event(GuiEvent::NewWindow);
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

    // The native menu bar must exist before the loop runs and must stay alive
    // for the whole app lifetime, so it is created here and captured below.
    #[cfg(target_os = "macos")]
    let macos_menu = setup_macos_menu(&proxy);

    // Keep the webview alive for the lifetime of the window.
    let mut close_pending = false;
    let mut close_deadline: Option<Instant> = None;
    let mut shown = false;
    // Latest NON-maximized geometry seen this session, so quitting while
    // maximized still remembers where the normal window lived (the loaded
    // state would otherwise be written back, losing this session's moves).
    let mut last_normal: Option<WindowState> = saved_state.clone();
    // Show even if the page never reports ready (server error, slow webview),
    // so the user always gets a window to look at.
    let show_deadline = Instant::now() + Duration::from_millis(2000);
    let close_timeout = Duration::from_secs(5);
    event_loop.run(move |event, _, control_flow| {
        let now = Instant::now();
        if close_pending && close_deadline.is_some_and(|deadline| now >= deadline) {
            eprintln!("ayame: close confirmation timed out; exiting");
            save_window_state(&window, last_normal.as_ref());
            *control_flow = ControlFlow::Exit;
            return;
        }
        *control_flow = if let Some(deadline) = close_deadline {
            ControlFlow::WaitUntil(deadline)
        } else if shown {
            ControlFlow::Wait
        } else {
            ControlFlow::WaitUntil(show_deadline)
        };
        let _ = &webview;
        let _ = &web_context; // keep the webview profile directory binding alive
        #[cfg(target_os = "macos")]
        let _ = &macos_menu;
        match event {
            Event::NewEvents(StartCause::ResumeTimeReached { .. }) => {
                if !shown {
                    shown = true;
                    window.set_visible(true);
                    if start_maximized {
                        window.set_maximized(true);
                    }
                }
            }
            Event::WindowEvent {
                event: WindowEvent::Moved(_) | WindowEvent::Resized(_),
                ..
            } => {
                // Track the un-maximized geometry as it changes; maximized
                // bounds are useless for restore and are skipped.
                if !window.is_maximized() {
                    let size = window.inner_size();
                    if size.width > 0 && size.height > 0 {
                        let pos = window.outer_position().ok();
                        last_normal = Some(WindowState {
                            x: pos.map(|p| p.x),
                            y: pos.map(|p| p.y),
                            width: size.width,
                            height: size.height,
                            maximized: false,
                        });
                    }
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
                close_deadline = Some(Instant::now() + close_timeout);
                if webview.evaluate_script(NATIVE_CLOSE_SCRIPT).is_err() {
                    save_window_state(&window, last_normal.as_ref());
                    *control_flow = ControlFlow::Exit;
                }
            }
            Event::UserEvent(GuiEvent::CloseConfirmed) => {
                save_window_state(&window, last_normal.as_ref());
                *control_flow = ControlFlow::Exit;
            }
            Event::UserEvent(GuiEvent::CloseCanceled) => {
                close_pending = false;
                close_deadline = None;
            }
            Event::UserEvent(GuiEvent::SetTitle(title)) => {
                window.set_title(&title);
            }
            Event::UserEvent(GuiEvent::Ready) => {
                if !shown {
                    shown = true;
                    window.set_visible(true);
                    if start_maximized {
                        window.set_maximized(true);
                    }
                }
            }
            #[cfg(target_os = "macos")]
            Event::UserEvent(GuiEvent::Menu(id)) => {
                if id == "quit" {
                    // Cmd+Q takes the same path as the window close button so
                    // unsaved changes get the same confirmation dialog.
                    if close_pending {
                        return;
                    }
                    close_pending = true;
                    close_deadline = Some(Instant::now() + close_timeout);
                    if webview.evaluate_script(NATIVE_CLOSE_SCRIPT).is_err() {
                        save_window_state(&window, last_normal.as_ref());
                        *control_flow = ControlFlow::Exit;
                    }
                } else if id == "newWindow" {
                    // Same native path as the IPC request — a new window is a
                    // new process, never a round-trip through __ayameMenu.
                    spawn_new_window();
                } else if let Ok(id_json) = serde_json::to_string(&id) {
                    let js = format!("window.__ayameMenu && window.__ayameMenu({id_json});");
                    let _ = webview.evaluate_script(&js);
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
            Event::UserEvent(GuiEvent::NewWindow) => {
                spawn_new_window();
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
    /// The page (Ctrl+Shift+N, rebindable) asked for a fresh window.
    NewWindow,
    /// A native menu item was activated; carries the muda item id, which is
    /// the frozen action name understood by `window.__ayameMenu` in the page.
    #[cfg(target_os = "macos")]
    Menu(String),
}

/// Open a new editor window: spawn a fresh, detached `<current-exe> gui`
/// process. Each window is its own process + server by design — no state is
/// shared, so a crash in one window can never take another down. Failures are
/// logged and swallowed: the running window must never break over this.
fn spawn_new_window() {
    match std::env::current_exe() {
        Ok(exe) => {
            if let Err(e) = std::process::Command::new(exe).arg("gui").spawn() {
                eprintln!("ayame: opening a new window failed: {e}");
            }
        }
        Err(e) => eprintln!("ayame: opening a new window failed (current_exe): {e}"),
    }
}

/// Registers the muda event forwarder and attaches the menu bar to NSApp.
///
/// Returns the menu so the caller keeps it (and every item hanging off it)
/// alive for the app's lifetime; muda items are refcounted and dropping the
/// root would tear the bar down. `None` (menu construction failed) simply
/// leaves the app without a menu bar — everything else still works.
#[cfg(target_os = "macos")]
fn setup_macos_menu(proxy: &tao::event_loop::EventLoopProxy<GuiEvent>) -> Option<muda::Menu> {
    let proxy = proxy.clone();
    muda::MenuEvent::set_event_handler(Some(move |event: muda::MenuEvent| {
        let _ = proxy.send_event(GuiEvent::Menu(event.id.0));
    }));
    let menu = build_macos_menu()?;
    // tao's macOS event loop drives NSApp itself, so attaching here — on the
    // main thread, after the event loop exists and before `run` — is the
    // whole interop story.
    menu.init_for_nsapp();
    Some(menu)
}

/// The native macOS menu bar. Beyond convention, this is what makes
/// Cmd+C/V/X/A work inside WKWebView: AppKit only routes the standard edit
/// selectors to the focused view when NSMenu items carry those key
/// equivalents. Windows/Linux use the in-page menubar instead.
#[cfg(target_os = "macos")]
fn build_macos_menu() -> Option<muda::Menu> {
    use muda::accelerator::{Accelerator, Code, Modifiers};
    use muda::{AboutMetadata, Menu, MenuItem, PredefinedMenuItem, Submenu};

    #[derive(Clone, Copy)]
    enum UiLocale {
        Ja,
        En,
    }

    fn ui_locale() -> UiLocale {
        let lang = ["AYAME_LANG", "LANGUAGE", "LC_ALL", "LC_MESSAGES", "LANG"]
            .iter()
            .filter_map(|key| std::env::var(key).ok())
            .find(|value| !value.trim().is_empty())
            .unwrap_or_default()
            .to_ascii_lowercase();
        if lang.starts_with("ja") {
            UiLocale::Ja
        } else {
            UiLocale::En
        }
    }

    let cmd = Modifiers::SUPER;
    let shift_cmd = Modifiers::SUPER | Modifiers::SHIFT;
    let key = |mods: Modifiers, code: Code| Some(Accelerator::new(Some(mods), code));
    // Ids are the frozen `window.__ayameMenu` action names, except "quit"
    // which is intercepted natively in the event loop.
    let item = |id: &str, text: &str, accel| MenuItem::with_id(id, text, true, accel);
    let locale = ui_locale();
    let label = |ja: &'static str, en: &'static str| match locale {
        UiLocale::Ja => ja,
        UiLocale::En => en,
    };

    let app = Submenu::with_items(
        "Ayame Editor",
        true,
        &[
            &PredefinedMenuItem::about(
                Some(label("Ayame Editor について", "About Ayame Editor")),
                Some(AboutMetadata {
                    name: Some("Ayame Editor".into()),
                    version: Some(env!("CARGO_PKG_VERSION").into()),
                    ..Default::default()
                }),
            ),
            &PredefinedMenuItem::separator(),
            &item("settings", label("設定…", "Settings..."), key(cmd, Code::Comma)),
            &PredefinedMenuItem::separator(),
            // Not PredefinedMenuItem::quit: quitting must go through the same
            // unsaved-changes confirmation as closing the window.
            &item("quit", label("Ayame Editor を終了", "Quit Ayame Editor"), key(cmd, Code::KeyQ)),
        ],
    )
    .ok()?;

    let file = Submenu::with_items(
        label("ファイル", "File"),
        true,
        &[
            &item("newFile", label("新規ファイル", "New File"), key(cmd, Code::KeyN)),
            // Handled natively in the event loop (like "quit"): a new window
            // is a new process, not a page action.
            &item("newWindow", label("新規ウィンドウ", "New Window"), key(shift_cmd, Code::KeyN)),
            &item("openFile", label("開く", "Open"), key(cmd, Code::KeyO)),
            &PredefinedMenuItem::separator(),
            &item("saveFile", label("保存", "Save"), key(cmd, Code::KeyS)),
            &item("saveAs", label("名前を付けて保存", "Save As"), key(shift_cmd, Code::KeyS)),
            &PredefinedMenuItem::separator(),
            &item("closeTab", label("タブを閉じる", "Close Tab"), key(cmd, Code::KeyW)),
        ],
    )
    .ok()?;

    let edit = Submenu::with_items(
        label("編集", "Edit"),
        true,
        &[
            &item("undo", label("元に戻す", "Undo"), key(cmd, Code::KeyZ)),
            &item("redo", label("やり直す", "Redo"), key(shift_cmd, Code::KeyZ)),
            &PredefinedMenuItem::separator(),
            &item("cut", label("切り取り", "Cut"), key(cmd, Code::KeyX)),
            &item("copy", label("コピー", "Copy"), key(cmd, Code::KeyC)),
            // Paste stays a native selector so the DOM paste event (the only
            // sanctioned clipboard-read path) reaches the hidden textarea.
            &PredefinedMenuItem::paste(Some(label("貼り付け", "Paste"))),
            &item("selectAll", label("すべて選択", "Select All"), key(cmd, Code::KeyA)),
            &PredefinedMenuItem::separator(),
            &item("find", label("検索", "Find"), key(cmd, Code::KeyF)),
            &item("replace", label("置換", "Replace"), None),
        ],
    )
    .ok()?;

    let window = Submenu::with_items(
        label("ウインドウ", "Window"),
        true,
        &[
            &PredefinedMenuItem::minimize(Some(label("しまう", "Minimize"))),
            &PredefinedMenuItem::maximize(Some(label("拡大/縮小", "Zoom"))),
        ],
    )
    .ok()?;
    // Let AppKit append the standard window list to this submenu.
    window.set_as_windows_menu_for_nsapp();

    Menu::with_items(&[&app, &file, &edit, &window]).ok()
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

/// Last session's window geometry, persisted in the index-cache directory.
/// Everything is optional-with-defaults so a corrupt or hand-edited file
/// degrades to "open like a fresh install" instead of failing.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
struct WindowState {
    x: Option<i32>,
    y: Option<i32>,
    width: u32,
    height: u32,
    maximized: bool,
}

impl Default for WindowState {
    fn default() -> Self {
        Self {
            x: None,
            y: None,
            width: 1280,
            height: 800,
            maximized: false,
        }
    }
}

impl WindowState {
    /// Clamp restored bounds so a stale or corrupt file can never produce an
    /// unusable window: size within [min window size, 8192] and a position
    /// that is at least partially reachable on a plausible monitor layout.
    /// The x lower bound is generous (-32768) because monitors placed left of
    /// the primary have genuinely negative origins (e.g. x = -2560).
    fn sanitized(mut self) -> Self {
        self.width = self.width.clamp(900, 8192);
        self.height = self.height.clamp(560, 8192);
        let pos_ok = matches!(
            (self.x, self.y),
            (Some(x), Some(y)) if (-32_768..=20_000).contains(&x) && (-200..=20_000).contains(&y)
        );
        if !pos_ok {
            self.x = None;
            self.y = None;
        }
        self
    }
}

fn window_state_path() -> Option<PathBuf> {
    crate::default_cache_dir().map(|dir| dir.join("window-state.json"))
}

/// `None` when there is nothing to restore (first run, unreadable cache dir);
/// unparseable JSON falls back to the defaults rather than being an error.
fn load_window_state() -> Option<WindowState> {
    let bytes = std::fs::read(window_state_path()?).ok()?;
    let state: WindowState = serde_json::from_slice(&bytes).unwrap_or_default();
    Some(state.sanitized())
}

/// Best-effort save on close. Failures are silently ignored: the close path
/// must never break over a full disk or read-only cache directory.
fn save_window_state(window: &Window, previous: Option<&WindowState>) {
    let Some(path) = window_state_path() else {
        return;
    };
    let state = if window.is_maximized() {
        // Maximized bounds are useless for restore; keep the last known
        // un-maximized geometry and only remember the maximized flag.
        let mut state = previous.cloned().unwrap_or_default();
        state.maximized = true;
        state
    } else {
        let pos = window.outer_position().ok();
        let size = window.inner_size();
        WindowState {
            x: pos.map(|p| p.x),
            y: pos.map(|p| p.y),
            width: size.width,
            height: size.height,
            maximized: false,
        }
    };
    let Ok(json) = serde_json::to_vec(&state) else {
        return;
    };
    if let Some(dir) = path.parent() {
        let _ = std::fs::create_dir_all(dir);
    }
    // Write-then-rename so a crash mid-write can't leave a truncated file.
    let tmp = path.with_extension("json.tmp");
    if std::fs::write(&tmp, &json).is_ok() {
        let _ = std::fs::rename(&tmp, &path);
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
