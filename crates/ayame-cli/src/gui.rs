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
use tao::event_loop::{ControlFlow, EventLoopBuilder, EventLoopProxy};
use tao::window::{Icon, Window, WindowBuilder};
use wry::{http::Request, DragDropEvent, WebContext, WebView, WebViewBuilder};

use crate::{has_flag, parse_for};

pub fn cmd_gui(args: &[String]) -> Result<()> {
    // Same file-opening options as `serve`; the window opens empty if no FILE.
    // `--recover` is internal: a window spawned by a dirty-tab handoff (issue
    // #35) passes it so the page replays the detached tab's crash log without
    // the crash-recovery prompt.
    let (pos, opts, flags) = parse_for("gui", args)?;
    let recover_pending = has_flag(&flags, &["--recover"]);
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

    // Keep a handle for exit cleanup: spawn_background moves `state` into the
    // server thread, but the window's event loop must drop this session's
    // scratch/WAL on close (#138).
    let cleanup_state = state.clone();
    // Bring the editor up behind the window and learn its loopback address.
    let addr = crate::serve::spawn_background(state)?;
    let url = format!("http://{addr}/");
    eprintln!("ayame: native window → {url}");

    let event_loop = EventLoopBuilder::<GuiEvent>::with_user_event().build();
    let proxy = event_loop.create_proxy();
    let ipc_proxy = proxy.clone();
    // Created hidden: the page reveals it with a "ready" IPC event (fallback timer
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
            let recover_js = if recover_pending {
                "window.__ayamePendingRecover = true;"
            } else {
                ""
            };
            format!(
                "window.__ayamePendingOpen = {};{recover_js}",
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
        .with_ipc_handler(
            move |req: Request<String>| match decode_ipc_message(req.body()) {
                Ok(Some(event)) => {
                    let _ = ipc_proxy.send_event(event);
                }
                Ok(None) => {}
                Err(error) => eprintln!("ayame: invalid native IPC message: {error}"),
            },
        );
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
    let mut macos_menu = setup_macos_menu(&proxy, None);

    // Keep the webview alive for the lifetime of the window.
    let mut close_pending = false;
    let mut close_deadline: Option<Instant> = None;
    let mut shown = false;
    let mut update_check_enabled: Option<bool> = None;
    let mut update_check_started = false;
    let mut update_installing = false;
    // Set when the user answered "restart now" to an installed update: the
    // successor process is spawned once this one has finished exiting.
    let mut restart_after_exit = false;
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
            Event::NewEvents(StartCause::ResumeTimeReached { .. }) if !shown => {
                reveal_window(
                    &window,
                    &proxy,
                    &mut shown,
                    start_maximized,
                    update_check_enabled,
                    &mut update_check_started,
                );
            }
            Event::WindowEvent {
                event: WindowEvent::Moved(_) | WindowEvent::Resized(_),
                ..
            } if !window.is_maximized() => {
                // Track the un-maximized geometry as it changes; maximized
                // bounds are useless for restore and are skipped.
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
            Event::WindowEvent {
                event: WindowEvent::CloseRequested,
                ..
            } => {
                request_close(
                    &webview,
                    &window,
                    last_normal.as_ref(),
                    &mut close_pending,
                    &mut close_deadline,
                    close_timeout,
                    control_flow,
                );
            }
            Event::UserEvent(GuiEvent::CloseConfirmed) => {
                save_window_state(&window, last_normal.as_ref());
                *control_flow = ControlFlow::Exit;
            }
            Event::UserEvent(GuiEvent::CloseCanceled) => {
                close_pending = false;
                close_deadline = None;
                // The page kept the window open (unsaved work). A later,
                // unrelated close must not surprise the user with a restart.
                restart_after_exit = false;
            }
            Event::UserEvent(GuiEvent::SetTitle(title)) => {
                window.set_title(&title);
            }
            Event::UserEvent(GuiEvent::Ready) => {
                reveal_window(
                    &window,
                    &proxy,
                    &mut shown,
                    start_maximized,
                    update_check_enabled,
                    &mut update_check_started,
                );
            }
            Event::UserEvent(GuiEvent::UpdateCheckStartup(enabled)) => {
                update_check_enabled = Some(enabled);
                maybe_start_startup_update_check(
                    &proxy,
                    shown,
                    update_check_enabled,
                    &mut update_check_started,
                );
            }
            Event::UserEvent(GuiEvent::UpdateAvailable(info)) => {
                if update_check_enabled != Some(true)
                    || update_installing
                    || !confirm_update_dialog(&window, &info)
                {
                    return;
                }
                update_installing = true;
                spawn_update_install(proxy.clone());
            }
            Event::UserEvent(GuiEvent::UpdateInstalled(report)) => {
                update_installing = false;
                if show_update_installed_dialog(&window, &report) {
                    // Take the ordinary close path so unsaved work still gets
                    // its confirmation; the replacement process is spawned
                    // from LoopDestroyed, once this one really is leaving.
                    restart_after_exit = true;
                    request_close(
                        &webview,
                        &window,
                        last_normal.as_ref(),
                        &mut close_pending,
                        &mut close_deadline,
                        close_timeout,
                        control_flow,
                    );
                }
            }
            Event::UserEvent(GuiEvent::UpdateInstallFailed(message)) => {
                update_installing = false;
                show_update_failed_dialog(&window, &message);
            }
            #[cfg(target_os = "macos")]
            Event::UserEvent(GuiEvent::Menu(id)) => {
                if id == "quit" {
                    // Cmd+Q takes the same path as the window close button so
                    // unsaved changes get the same confirmation dialog.
                    request_close(
                        &webview,
                        &window,
                        last_normal.as_ref(),
                        &mut close_pending,
                        &mut close_deadline,
                        close_timeout,
                        control_flow,
                    );
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
            Event::UserEvent(GuiEvent::NewWindowPath(path)) => {
                spawn_new_window_with_path(&path, false);
            }
            Event::UserEvent(GuiEvent::NewWindowPathRecover(path)) => {
                spawn_new_window_with_path(&path, true);
            }
            // The OS dialogs run modally on this (the UI) thread — required on
            // macOS, standard on Windows; the dialog pumps its own events.
            Event::UserEvent(GuiEvent::PickSave(req)) => {
                let picked = file_dialog(&req.dir)
                    .set_file_name(req.name.trim())
                    .save_file();
                let result = picked.as_deref().map(crate::serve::workspace::display_path);
                let json = serde_json::to_string(&result).unwrap_or_else(|_| "null".into());
                let _ = webview.evaluate_script(&format!(
                    "window.__ayameSaveDialogDone && window.__ayameSaveDialogDone({json});"
                ));
            }
            Event::UserEvent(GuiEvent::PickOpen(req)) => {
                let picked: Vec<String> = file_dialog(&req.dir)
                    .pick_files()
                    .unwrap_or_default()
                    .iter()
                    .map(|p| crate::serve::workspace::display_path(p))
                    .collect();
                let json = serde_json::to_string(&picked).unwrap_or_else(|_| "[]".into());
                let _ = webview.evaluate_script(&format!(
                    "window.__ayameOpenDialogDone && window.__ayameOpenDialogDone({json});"
                ));
            }
            #[cfg(target_os = "macos")]
            Event::UserEvent(GuiEvent::MenuConfig(config)) => {
                macos_menu = setup_macos_menu(&proxy, Some(&config));
            }
            #[cfg(not(target_os = "macos"))]
            Event::UserEvent(GuiEvent::MenuConfig(config)) => {
                let _ = config;
            }
            // Fires once, after any `ControlFlow::Exit`, before the loop tears
            // the process down — the single choke point where every close path
            // (window button, menu quit, close-confirm timeout) converges, so
            // this session's scratch/WAL/aside files are dropped exactly here
            // instead of leaking every session (#138).
            Event::LoopDestroyed => {
                crate::serve::cleanup_session(&cleanup_state);
                // After cleanup, so the successor never races this session's
                // scratch/WAL teardown. It starts the freshly installed build
                // and restores the session like any manual restart would.
                if restart_after_exit {
                    spawn_new_window();
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
    UpdateCheckStartup(bool),
    UpdateAvailable(crate::UpdateInfo),
    UpdateInstalled(crate::UpdateInstallReport),
    UpdateInstallFailed(String),
    OpenPaths(Vec<String>),
    /// The page (Ctrl+Shift+N, rebindable) asked for a fresh window.
    NewWindow,
    NewWindowPath(String),
    /// A dirty-tab handoff: open `path` in a fresh window and auto-replay its
    /// detached crash log (issue #35).
    NewWindowPathRecover(String),
    /// The page asked for the OS save dialog (名前を付けて保存).
    PickSave(PickSaveRequest),
    /// The page asked for the OS open dialog (ファイルを開く).
    PickOpen(PickOpenRequest),
    /// Current web-localized labels and user key bindings for the macOS menu.
    MenuConfig(NativeMenuConfig),
    /// A native menu item was activated; carries the muda item id, which is
    /// the frozen action name understood by `window.__ayameMenu` in the page.
    #[cfg(target_os = "macos")]
    Menu(String),
}

#[derive(Debug, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
enum IpcMessage {
    CloseConfirmed,
    CloseCanceled,
    Ready,
    SetTitle { title: String },
    UpdateCheckStartup { enabled: bool },
    NewWindow,
    NewWindowPath { path: String, recover: bool },
    PickSave { dir: String, name: String },
    PickOpen { dir: String },
    MenuConfig { items: Vec<NativeMenuItemConfig> },
}

const MAX_NATIVE_MENU_ITEMS: usize = 128;
const MAX_NATIVE_MENU_ID_CHARS: usize = 64;
const MAX_NATIVE_MENU_LABEL_CHARS: usize = 160;
const MAX_NATIVE_MENU_SHORTCUT_CHARS: usize = 64;

#[derive(Debug, Clone, Deserialize, PartialEq, Eq)]
struct NativeMenuItemConfig {
    id: String,
    label: String,
    #[serde(default)]
    shortcut: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct NativeMenuConfig {
    items: Vec<NativeMenuItemConfig>,
}

impl NativeMenuConfig {
    #[cfg(any(target_os = "macos", test))]
    fn item(&self, id: &str) -> Option<&NativeMenuItemConfig> {
        self.items.iter().find(|item| item.id == id)
    }
}

#[cfg(any(target_os = "macos", test))]
fn configured_menu_label<'a>(
    config: Option<&'a NativeMenuConfig>,
    id: &str,
    fallback: &'a str,
) -> &'a str {
    config
        .and_then(|config| config.item(id))
        .map(|item| item.label.as_str())
        .unwrap_or(fallback)
}

fn normalize_menu_config(items: Vec<NativeMenuItemConfig>) -> NativeMenuConfig {
    let items = items
        .into_iter()
        .take(MAX_NATIVE_MENU_ITEMS)
        .filter_map(|item| {
            let raw_id = item.id.trim();
            if raw_id.is_empty()
                || raw_id.chars().count() > MAX_NATIVE_MENU_ID_CHARS
                || !raw_id
                    .chars()
                    .all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '_' | '-'))
            {
                return None;
            }
            let id = raw_id.to_string();
            let label: String = item
                .label
                .chars()
                .filter(|c| !c.is_control())
                .take(MAX_NATIVE_MENU_LABEL_CHARS)
                .collect::<String>()
                .trim()
                .to_string();
            if label.is_empty() {
                return None;
            }
            let shortcut = item
                .shortcut
                .chars()
                .filter(|c| !c.is_control())
                .take(MAX_NATIVE_MENU_SHORTCUT_CHARS)
                .collect::<String>()
                .trim()
                .to_string();
            Some(NativeMenuItemConfig {
                id,
                label,
                shortcut,
            })
        })
        .collect();
    NativeMenuConfig { items }
}

#[cfg(any(target_os = "macos", test))]
fn macos_accelerator_spelling(shortcut: &str) -> Option<String> {
    let shortcut = shortcut.trim();
    if shortcut.is_empty() {
        return None;
    }
    Some(match shortcut.strip_prefix("Ctrl+") {
        Some(rest) => format!("Command+{rest}"),
        None => shortcut.to_string(),
    })
}

/// What the page suggests for the OS save dialog: a starting folder and a
/// pre-filled file name. Empty strings let the OS choose its last-used values.
#[derive(Debug)]
struct PickSaveRequest {
    dir: String,
    name: String,
}

#[derive(Debug)]
struct PickOpenRequest {
    dir: String,
}

fn decode_ipc_message(body: &str) -> serde_json::Result<Option<GuiEvent>> {
    let event = match serde_json::from_str::<IpcMessage>(body)? {
        IpcMessage::CloseConfirmed => GuiEvent::CloseConfirmed,
        IpcMessage::CloseCanceled => GuiEvent::CloseCanceled,
        IpcMessage::Ready => GuiEvent::Ready,
        IpcMessage::SetTitle { title } => GuiEvent::SetTitle(clean_window_title(&title)),
        IpcMessage::UpdateCheckStartup { enabled } => GuiEvent::UpdateCheckStartup(enabled),
        IpcMessage::NewWindow => GuiEvent::NewWindow,
        IpcMessage::NewWindowPath { path, recover } if recover => {
            GuiEvent::NewWindowPathRecover(path)
        }
        IpcMessage::NewWindowPath { path, .. } => GuiEvent::NewWindowPath(path),
        IpcMessage::PickSave { dir, name } => GuiEvent::PickSave(PickSaveRequest { dir, name }),
        IpcMessage::PickOpen { dir } => GuiEvent::PickOpen(PickOpenRequest { dir }),
        IpcMessage::MenuConfig { items } => GuiEvent::MenuConfig(normalize_menu_config(items)),
    };
    Ok(Some(event))
}

fn reveal_window(
    window: &Window,
    proxy: &EventLoopProxy<GuiEvent>,
    shown: &mut bool,
    start_maximized: bool,
    update_check_enabled: Option<bool>,
    update_check_started: &mut bool,
) {
    if !*shown {
        *shown = true;
        window.set_visible(true);
        if start_maximized {
            window.set_maximized(true);
        }
    }
    maybe_start_startup_update_check(proxy, *shown, update_check_enabled, update_check_started);
}

fn request_close(
    webview: &WebView,
    window: &Window,
    last_normal: Option<&WindowState>,
    close_pending: &mut bool,
    close_deadline: &mut Option<Instant>,
    close_timeout: Duration,
    control_flow: &mut ControlFlow,
) {
    if *close_pending {
        return;
    }
    *close_pending = true;
    *close_deadline = Some(Instant::now() + close_timeout);
    if webview.evaluate_script(NATIVE_CLOSE_SCRIPT).is_err() {
        save_window_state(window, last_normal);
        *control_flow = ControlFlow::Exit;
    }
}

#[cfg(test)]
mod pure_tests {
    use super::*;

    #[test]
    fn the_window_title_names_the_file_being_opened() {
        assert_eq!(
            initial_window_title(&["/logs/app.log".to_string()]),
            "app.log - Ayame Editor"
        );
        assert_eq!(initial_window_title(&[]), "Ayame Editor");
    }

    #[test]
    fn a_path_shows_its_file_name_or_falls_back_to_itself() {
        assert_eq!(path_display_name("/a/b/notes.txt"), "notes.txt");
        assert_eq!(path_display_name("notes.txt"), "notes.txt");
        // No file-name component: the path itself is the best label there is.
        assert_eq!(path_display_name("/"), "/");
        assert_eq!(path_display_name(""), "");
    }

    /// The page sets the title, so it is untrusted input: control characters
    /// could rewrite the titlebar and an unbounded string could stall it.
    #[test]
    fn a_title_from_the_page_is_stripped_and_bounded() {
        assert_eq!(clean_window_title("notes.txt"), "notes.txt");
        assert_eq!(clean_window_title("a\u{0}b\nc"), "abc");
        assert_eq!(clean_window_title("").len(), "Ayame Editor".len());
        assert_eq!(clean_window_title("   "), "Ayame Editor");
        assert_eq!(clean_window_title(&"x".repeat(1000)).chars().count(), 256);
    }

    #[test]
    fn env_flags_accept_the_spellings_people_actually_use() {
        for on in ["1", "true", "yes", "on", "TRUE", " On "] {
            assert_eq!(parse_env_bool(on), Some(true), "{on:?}");
        }
        for off in ["0", "false", "no", "off", "OFF"] {
            assert_eq!(parse_env_bool(off), Some(false), "{off:?}");
        }
        // Anything else is not an answer, so the caller's default stands.
        for junk in ["", "maybe", "2"] {
            assert_eq!(parse_env_bool(junk), None, "{junk:?}");
        }
    }

    /// Restored geometry is clamped so a stale or hand-edited file cannot
    /// produce a window nobody can use or find.
    #[test]
    fn restored_geometry_is_clamped_to_something_usable() {
        let tiny = WindowState {
            x: Some(10),
            y: Some(10),
            width: 1,
            height: 1,
            maximized: false,
        }
        .sanitized();
        assert_eq!((tiny.width, tiny.height), (900, 560));

        let huge = WindowState {
            x: Some(10),
            y: Some(10),
            width: 99_999,
            height: 99_999,
            maximized: false,
        }
        .sanitized();
        assert_eq!((huge.width, huge.height), (8192, 8192));
    }

    #[test]
    fn an_offscreen_position_is_dropped_so_the_window_is_placed_by_the_os() {
        let lost = WindowState {
            x: Some(999_999),
            y: Some(999_999),
            ..WindowState::default()
        }
        .sanitized();
        assert_eq!((lost.x, lost.y), (None, None));

        // A monitor left of the primary has a genuinely negative origin, so
        // that must survive.
        let left_monitor = WindowState {
            x: Some(-2560),
            y: Some(0),
            ..WindowState::default()
        }
        .sanitized();
        assert_eq!((left_monitor.x, left_monitor.y), (Some(-2560), Some(0)));
    }

    /// The persistence round trip: what a close writes is what the next start
    /// restores.
    #[test]
    fn window_state_survives_a_save_and_load() {
        let saved = next_window_state(false, Some((120, 80)), (1600, 900), None);
        let json = serde_json::to_vec(&saved).unwrap();

        let restored = parse_window_state(&json);

        assert_eq!(restored.x, Some(120));
        assert_eq!(restored.y, Some(80));
        assert_eq!((restored.width, restored.height), (1600, 900));
        assert!(!restored.maximized);
    }

    /// Quitting maximized keeps the last un-maximized geometry: the maximized
    /// bounds would otherwise become the restore size forever after.
    #[test]
    fn quitting_maximized_remembers_the_geometry_underneath() {
        let previous = WindowState {
            x: Some(50),
            y: Some(60),
            width: 1000,
            height: 700,
            maximized: false,
        };

        let saved = next_window_state(true, Some((0, 0)), (3840, 2160), Some(&previous));

        assert!(saved.maximized);
        assert_eq!((saved.x, saved.y), (Some(50), Some(60)));
        assert_eq!((saved.width, saved.height), (1000, 700));
    }

    #[test]
    fn quitting_maximized_with_nothing_remembered_falls_back_to_defaults() {
        let saved = next_window_state(true, None, (3840, 2160), None);
        assert!(saved.maximized);
        assert_eq!(
            (saved.width, saved.height),
            (WindowState::default().width, WindowState::default().height)
        );
    }

    /// A corrupt or hand-edited file costs the geometry, not the window.
    #[test]
    fn unparseable_state_falls_back_to_the_defaults() {
        let restored = parse_window_state(b"{not json");
        let default = WindowState::default().sanitized();
        assert_eq!(
            (restored.width, restored.height),
            (default.width, default.height)
        );
        assert_eq!((restored.x, restored.y), (default.x, default.y));
    }

    #[test]
    fn web_shortcuts_map_control_to_the_macos_command_key() {
        assert_eq!(
            macos_accelerator_spelling("Ctrl+Shift+F").as_deref(),
            Some("Command+Shift+F")
        );
        assert_eq!(
            macos_accelerator_spelling("Ctrl++").as_deref(),
            Some("Command++")
        );
        assert_eq!(
            macos_accelerator_spelling("Alt+ArrowUp").as_deref(),
            Some("Alt+ArrowUp")
        );
        assert_eq!(macos_accelerator_spelling(""), None);
    }

    #[test]
    fn configured_menu_labels_override_only_their_fallback_item() {
        let config = NativeMenuConfig {
            items: vec![NativeMenuItemConfig {
                id: "find".into(),
                label: "Search Logs".into(),
                shortcut: String::new(),
            }],
        };
        assert_eq!(
            configured_menu_label(Some(&config), "find", "Find"),
            "Search Logs"
        );
        assert_eq!(
            configured_menu_label(Some(&config), "saveFile", "Save"),
            "Save"
        );
    }
}

#[cfg(test)]
mod ipc_tests {
    use super::*;

    fn event(body: &str) -> GuiEvent {
        decode_ipc_message(body)
            .expect("valid native IPC message")
            .expect("message maps to an event on this platform")
    }

    #[test]
    fn decodes_control_messages() {
        assert!(matches!(
            event(r#"{"type":"close_confirmed"}"#),
            GuiEvent::CloseConfirmed
        ));
        assert!(matches!(
            event(r#"{"type":"close_canceled"}"#),
            GuiEvent::CloseCanceled
        ));
        assert!(matches!(event(r#"{"type":"ready"}"#), GuiEvent::Ready));
        assert!(matches!(
            event(r#"{"type":"new_window"}"#),
            GuiEvent::NewWindow
        ));
        assert!(matches!(
            event(r#"{"type":"update_check_startup","enabled":true}"#),
            GuiEvent::UpdateCheckStartup(true)
        ));
    }

    #[test]
    fn decodes_structured_payloads_without_delimiter_ambiguity() {
        match event(r#"{"type":"new_window_path","path":"C:\\tmp:a.txt","recover":true}"#) {
            GuiEvent::NewWindowPathRecover(path) => assert_eq!(path, r"C:\tmp:a.txt"),
            other => panic!("unexpected event: {other:?}"),
        }
        match event(r#"{"type":"pick_save","dir":"/tmp:a","name":"draft.json"}"#) {
            GuiEvent::PickSave(request) => {
                assert_eq!(request.dir, "/tmp:a");
                assert_eq!(request.name, "draft.json");
            }
            other => panic!("unexpected event: {other:?}"),
        }
        match event(r#"{"type":"pick_open","dir":"/tmp:b"}"#) {
            GuiEvent::PickOpen(request) => assert_eq!(request.dir, "/tmp:b"),
            other => panic!("unexpected event: {other:?}"),
        }
        match event(r#"{"type":"set_title","title":"draft\u0000"}"#) {
            GuiEvent::SetTitle(title) => assert_eq!(title, "draft"),
            other => panic!("unexpected event: {other:?}"),
        }
        match event(
            r#"{"type":"menu_config","items":[{"id":"find","label":"Find\u0000","shortcut":"Ctrl+Shift+F"},{"id":"***","label":"bad","shortcut":""},{"id":"empty","label":"\u0000","shortcut":""}]}"#,
        ) {
            GuiEvent::MenuConfig(config) => {
                assert_eq!(
                    config.items,
                    vec![NativeMenuItemConfig {
                        id: "find".into(),
                        label: "Find".into(),
                        shortcut: "Ctrl+Shift+F".into(),
                    }]
                );
            }
            other => panic!("unexpected event: {other:?}"),
        }
    }

    #[test]
    fn rejects_malformed_or_unknown_payloads() {
        assert!(decode_ipc_message("not json").is_err());
        assert!(decode_ipc_message(r#"{"type":"pick_save","dir":""}"#).is_err());
        assert!(
            decode_ipc_message(r#"{"type":"new_window_path","path":7,"recover":false}"#).is_err()
        );
        assert!(decode_ipc_message(r#"{"type":"menu_config","items":{}}"#).is_err());
        assert!(decode_ipc_message(r#"{"type":"not_a_message"}"#).is_err());
    }
}

fn maybe_start_startup_update_check(
    proxy: &EventLoopProxy<GuiEvent>,
    shown: bool,
    enabled: Option<bool>,
    started: &mut bool,
) {
    if *started || !shown || enabled != Some(true) || !startup_update_check_allowed() {
        return;
    }
    *started = true;
    let proxy = proxy.clone();
    std::thread::spawn(move || match crate::check_latest_update() {
        Ok(Some(info)) => {
            let _ = proxy.send_event(GuiEvent::UpdateAvailable(info));
        }
        Ok(None) => {}
        Err(e) => eprintln!("ayame: startup update check failed: {e:#}"),
    });
}

fn startup_update_check_allowed() -> bool {
    !env_bool("AYAME_NO_UPDATE_CHECK").unwrap_or(false)
}

fn env_bool(name: &str) -> Option<bool> {
    parse_env_bool(&std::env::var(name).ok()?)
}

/// The spellings an environment flag accepts. Split from [`env_bool`] so it is
/// testable without mutating process-global state from parallel tests (#187).
fn parse_env_bool(value: &str) -> Option<bool> {
    match value.trim().to_ascii_lowercase().as_str() {
        "1" | "true" | "yes" | "on" => Some(true),
        "0" | "false" | "no" | "off" => Some(false),
        _ => None,
    }
}

fn confirm_update_dialog(window: &Window, info: &crate::UpdateInfo) -> bool {
    let description = format!(
        "Ayame {} is available.\n\nCurrent version: {}\nInstall target: {}\n\nUpdate now?",
        info.release_version, info.current_version, info.install_target
    );
    rfd::MessageDialog::new()
        .set_parent(window)
        .set_level(rfd::MessageLevel::Info)
        .set_title("Ayame update available")
        .set_description(description)
        .set_buttons(rfd::MessageButtons::YesNo)
        .show()
        == rfd::MessageDialogResult::Yes
}

fn spawn_update_install(proxy: EventLoopProxy<GuiEvent>) {
    std::thread::spawn(move || match crate::install_latest_update() {
        Ok(report) => {
            let _ = proxy.send_event(GuiEvent::UpdateInstalled(report));
        }
        Err(e) => {
            let _ = proxy.send_event(GuiEvent::UpdateInstallFailed(format!("{e:#}")));
        }
    });
}

/// Report the finished install and offer to act on it. Returns `true` when the
/// user asked to restart now — until this window actually exits, the running
/// process is older than the binary it was loaded from, so every op worker it
/// would spawn is refused (#137). Restarting is the fix, so the dialog offers
/// it instead of only describing it.
fn show_update_installed_dialog(window: &Window, report: &crate::UpdateInstallReport) -> bool {
    // A deferred install is finished by a helper that waits for *this* process
    // to exit before copying the new binary in. A successor started from here
    // would therefore launch the old build and then have it replaced
    // underneath itself — exactly the hazard the restart exists to end — so
    // this case stays a plain notice.
    if report.deferred {
        let _ = rfd::MessageDialog::new()
            .set_parent(window)
            .set_level(rfd::MessageLevel::Info)
            .set_title("Update will finish after quit")
            .set_description(format!(
                "Ayame {} was downloaded.\n\nClose Ayame, then open it again to finish installing the update.",
                report.release_version
            ))
            .set_buttons(rfd::MessageButtons::Ok)
            .show();
        return false;
    }
    rfd::MessageDialog::new()
        .set_parent(window)
        .set_level(rfd::MessageLevel::Info)
        .set_title("Update installed")
        .set_description(format!(
            "Ayame {} was installed to:\n{}\n\nSearch, sort, replace and the other file operations need the new version. Restart Ayame now?",
            report.release_version, report.destination
        ))
        .set_buttons(rfd::MessageButtons::YesNo)
        .show()
        == rfd::MessageDialogResult::Yes
}

fn show_update_failed_dialog(window: &Window, message: &str) {
    let _ = rfd::MessageDialog::new()
        .set_parent(window)
        .set_level(rfd::MessageLevel::Error)
        .set_title("Update failed")
        .set_description(format!("Ayame could not install the update.\n\n{message}"))
        .set_buttons(rfd::MessageButtons::Ok)
        .show();
}

/// Base OS file dialog, starting in `dir` when the page suggested one that
/// actually exists (otherwise the OS picks — typically the last-used folder).
fn file_dialog(dir: &str) -> rfd::FileDialog {
    let mut dlg = rfd::FileDialog::new();
    let dir = dir.trim();
    if !dir.is_empty() && Path::new(dir).is_dir() {
        dlg = dlg.set_directory(dir);
    }
    dlg
}

/// Open a new editor window: spawn a fresh, detached `<installed-exe> gui`
/// process. Each window is its own process + server by design — no state is
/// shared, so a crash in one window can never take another down. Failures are
/// logged and swallowed: the running window must never break over this.
///
/// The program comes from [`crate::worker::installed_program`] so a window
/// opened after a self-update starts the newly installed build instead of
/// failing on Linux's `… (deleted)` path (#137).
fn spawn_new_window() {
    match crate::worker::installed_program() {
        Ok(exe) => {
            if let Err(e) = std::process::Command::new(exe).arg("gui").spawn() {
                eprintln!("ayame: opening a new window failed: {e}");
            }
        }
        Err(e) => eprintln!("ayame: opening a new window failed (current_exe): {e}"),
    }
}

fn spawn_new_window_with_path(path: &str, recover: bool) {
    let clean: String = path
        .chars()
        .filter(|c| !c.is_control())
        .take(4096)
        .collect();
    if clean.trim().is_empty() {
        spawn_new_window();
        return;
    }
    match crate::worker::installed_program() {
        Ok(exe) => {
            let mut cmd = std::process::Command::new(exe);
            cmd.arg("gui").arg(clean);
            if recover {
                cmd.arg("--recover");
            }
            if let Err(e) = cmd.spawn() {
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
#[derive(Clone, Copy)]
enum UiLocale {
    Ja,
    En,
}

#[cfg(target_os = "macos")]
impl UiLocale {
    fn from_env() -> UiLocale {
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
}

#[cfg(target_os = "macos")]
fn setup_macos_menu(
    proxy: &tao::event_loop::EventLoopProxy<GuiEvent>,
    config: Option<&NativeMenuConfig>,
) -> Option<muda::Menu> {
    let proxy = proxy.clone();
    muda::MenuEvent::set_event_handler(Some(move |event: muda::MenuEvent| {
        let _ = proxy.send_event(GuiEvent::Menu(event.id.0));
    }));
    let menu = build_macos_menu(UiLocale::from_env(), config)?;
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
fn build_macos_menu(locale: UiLocale, config: Option<&NativeMenuConfig>) -> Option<muda::Menu> {
    use muda::accelerator::{Accelerator, Code, Modifiers};
    use muda::{AboutMetadata, Menu, MenuItem, PredefinedMenuItem, Submenu};

    let cmd = Modifiers::SUPER;
    let shift_cmd = Modifiers::SUPER | Modifiers::SHIFT;
    let key = |mods: Modifiers, code: Code| Some(Accelerator::new(Some(mods), code));
    let fallback_label = |ja: &'static str, en: &'static str| match locale {
        UiLocale::Ja => ja,
        UiLocale::En => en,
    };
    let label = |id: &str, ja: &'static str, en: &'static str| {
        configured_menu_label(config, id, fallback_label(ja, en))
    };
    let accelerator = |id: &str, fallback: Option<Accelerator>| {
        let Some(item) = config.and_then(|config| config.item(id)) else {
            return fallback;
        };
        macos_accelerator_spelling(&item.shortcut).and_then(|value| value.parse().ok())
    };
    // Ids are the frozen `window.__ayameMenu` action names, except "quit"
    // which is intercepted natively in the event loop. Once the page boots,
    // both label and accelerator come from its i18n/keymap configuration.
    let item = |id: &str, ja: &'static str, en: &'static str, fallback| {
        MenuItem::with_id(id, label(id, ja, en), true, accelerator(id, fallback))
    };

    let app = Submenu::with_items(
        "Ayame Editor",
        true,
        &[
            &PredefinedMenuItem::about(
                Some(label(
                    "about",
                    "Ayame Editor について",
                    "About Ayame Editor",
                )),
                Some(AboutMetadata {
                    name: Some("Ayame Editor".into()),
                    version: Some(env!("CARGO_PKG_VERSION").into()),
                    ..Default::default()
                }),
            ),
            &PredefinedMenuItem::separator(),
            &item("settings", "設定…", "Settings...", key(cmd, Code::Comma)),
            &PredefinedMenuItem::separator(),
            // Not PredefinedMenuItem::quit: quitting must go through the same
            // unsaved-changes confirmation as closing the window.
            &item(
                "quit",
                "Ayame Editor を終了",
                "Quit Ayame Editor",
                key(cmd, Code::KeyQ),
            ),
        ],
    )
    .ok()?;

    let file = Submenu::with_items(
        label("section.file", "ファイル", "File"),
        true,
        &[
            &item("newFile", "新規ファイル", "New File", key(cmd, Code::KeyN)),
            // Handled natively in the event loop (like "quit"): a new window
            // is a new process, not a page action.
            &item(
                "newWindow",
                "新規ウィンドウ",
                "New Window",
                key(shift_cmd, Code::KeyN),
            ),
            &item("openFile", "開く", "Open", key(cmd, Code::KeyO)),
            &PredefinedMenuItem::separator(),
            &item("saveFile", "保存", "Save", key(cmd, Code::KeyS)),
            &item(
                "saveAs",
                "名前を付けて保存",
                "Save As",
                key(shift_cmd, Code::KeyS),
            ),
            &item(
                "encoding",
                "文字コード / 改行コード…",
                "Encoding / Line Endings...",
                None,
            ),
            &PredefinedMenuItem::separator(),
            &item(
                "closeTab",
                "タブを閉じる",
                "Close Tab",
                key(cmd, Code::KeyW),
            ),
        ],
    )
    .ok()?;

    let edit = Submenu::with_items(
        label("section.edit", "編集", "Edit"),
        true,
        &[
            &item("undo", "元に戻す", "Undo", key(cmd, Code::KeyZ)),
            &item("redo", "やり直す", "Redo", key(shift_cmd, Code::KeyZ)),
            &PredefinedMenuItem::separator(),
            &item("cut", "切り取り", "Cut", key(cmd, Code::KeyX)),
            &item("copy", "コピー", "Copy", key(cmd, Code::KeyC)),
            // Paste stays a native selector so the DOM paste event (the only
            // sanctioned clipboard-read path) reaches the hidden textarea. Its
            // label is configured, while AppKit intentionally owns Cmd+V.
            &PredefinedMenuItem::paste(Some(label("paste", "貼り付け", "Paste"))),
            &item(
                "selectAll",
                "すべて選択",
                "Select All",
                key(cmd, Code::KeyA),
            ),
            &PredefinedMenuItem::separator(),
            &item("find", "検索", "Find", key(cmd, Code::KeyF)),
            &item("replace", "置換", "Replace", None),
            &item("gotoLine", "行へ移動", "Go to Line", key(cmd, Code::KeyG)),
            &PredefinedMenuItem::separator(),
            &item(
                "duplicateLine",
                "行を複製",
                "Duplicate Line",
                key(shift_cmd, Code::KeyD),
            ),
            &item("deleteLine", "行を削除", "Delete Line", None),
        ],
    )
    .ok()?;

    let selection = Submenu::with_items(
        label("section.selection", "選択", "Selection"),
        true,
        &[
            &item(
                "selectNextOccurrence",
                "次の一致を選択",
                "Select Next Occurrence",
                key(cmd, Code::KeyD),
            ),
            &PredefinedMenuItem::separator(),
            &item(
                "addCursorAbove",
                "カーソルを上に追加",
                "Add Cursor Above",
                None,
            ),
            &item(
                "addCursorBelow",
                "カーソルを下に追加",
                "Add Cursor Below",
                None,
            ),
        ],
    )
    .ok()?;

    let view = Submenu::with_items(
        label("section.view", "表示", "View"),
        true,
        &[
            &item(
                "commandPalette",
                "コマンドパレット",
                "Command Palette",
                key(shift_cmd, Code::KeyP),
            ),
            &PredefinedMenuItem::separator(),
            &item(
                "toggleWhitespace",
                "空白・改行を表示",
                "Show Whitespace",
                None,
            ),
            &item(
                "toggleZenkakuUnderline",
                "全角空白を下線で表示",
                "Underline Full-width Spaces",
                None,
            ),
            &item("toggleWordWrap", "折り返し", "Word Wrap", None),
            &item("toggleFollowTail", "末尾に追従", "Follow Tail", None),
        ],
    )
    .ok()?;

    let help = Submenu::with_items(
        label("section.help", "ヘルプ", "Help"),
        true,
        &[
            &item("help", "Ayame Editor ヘルプ", "Ayame Editor Help", None),
            &item(
                "keymap",
                "キーボードショートカット",
                "Keyboard Shortcuts",
                None,
            ),
            &item(
                "commandPalette",
                "コマンドパレット",
                "Command Palette",
                key(shift_cmd, Code::KeyP),
            ),
        ],
    )
    .ok()?;

    let tools = Submenu::with_items(
        label("section.tools", "ツール", "Tools"),
        true,
        &[
            &item("sortSave", "ソート", "Sort", None),
            &item("splitFile", "ファイルを分割", "Split File", None),
            &item("grepFolder", "フォルダ内検索", "Grep Folder", None),
            &item("grepSave", "grep して保存", "Grep to File", None),
            &PredefinedMenuItem::separator(),
            &item("caseUpper", "大文字に変換", "Uppercase", None),
            &item("caseLower", "小文字に変換", "Lowercase", None),
            &item("caseCamel", "camelCase に変換", "camelCase", None),
            &item("casePascal", "PascalCase に変換", "PascalCase", None),
            &item("caseSnake", "snake_case に変換", "snake_case", None),
            &item("caseKebab", "kebab-case に変換", "kebab-case", None),
            &item(
                "caseConstant",
                "CONSTANT_CASE に変換",
                "CONSTANT_CASE",
                None,
            ),
        ],
    )
    .ok()?;

    let window = Submenu::with_items(
        label("section.window", "ウインドウ", "Window"),
        true,
        &[
            &PredefinedMenuItem::minimize(Some(label("window.minimize", "しまう", "Minimize"))),
            &PredefinedMenuItem::maximize(Some(label("window.zoom", "拡大/縮小", "Zoom"))),
        ],
    )
    .ok()?;
    // Let AppKit append the standard window list to this submenu.
    window.set_as_windows_menu_for_nsapp();

    Menu::with_items(&[
        &app, &file, &edit, &selection, &view, &tools, &window, &help,
    ])
    .ok()
}

const NATIVE_CLOSE_SCRIPT: &str = r#"
if (window.__ayameNativeCloseRequested) {
  window.__ayameNativeCloseRequested();
} else if (window.ipc && window.ipc.postMessage) {
  window.ipc.postMessage(JSON.stringify({ type: "close_confirmed" }));
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
    Some(parse_window_state(&bytes))
}

/// A stored state, clamped. Unparseable JSON falls back to the defaults rather
/// than being an error: a corrupt or hand-edited file must cost the geometry,
/// not the window.
fn parse_window_state(bytes: &[u8]) -> WindowState {
    serde_json::from_slice::<WindowState>(bytes)
        .unwrap_or_default()
        .sanitized()
}

/// What a close should persist, given the window's live geometry.
///
/// Split out from [`save_window_state`] because it is the whole decision and
/// the rest is file I/O: a maximized window's bounds are useless for restore,
/// so the last known un-maximized geometry is kept and only the flag moves
/// (#187). Without that, maximizing once and quitting would restore to a
/// full-screen-sized *un-maximized* window forever after.
fn next_window_state(
    maximized: bool,
    position: Option<(i32, i32)>,
    size: (u32, u32),
    previous: Option<&WindowState>,
) -> WindowState {
    if maximized {
        let mut state = previous.cloned().unwrap_or_default();
        state.maximized = true;
        return state;
    }
    WindowState {
        x: position.map(|(x, _)| x),
        y: position.map(|(_, y)| y),
        width: size.0,
        height: size.1,
        maximized: false,
    }
}

/// Best-effort save on close. Failures are silently ignored: the close path
/// must never break over a full disk or read-only cache directory.
fn save_window_state(window: &Window, previous: Option<&WindowState>) {
    let Some(path) = window_state_path() else {
        return;
    };
    let pos = window.outer_position().ok().map(|p| (p.x, p.y));
    let size = window.inner_size();
    let state = next_window_state(
        window.is_maximized(),
        pos,
        (size.width, size.height),
        previous,
    );
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

/// The window/taskbar icon: the Ayame flower mark painted by `crate::icon`
/// (kept there, GUI-free, so its near-square aspect ratio stays unit-tested —
/// the shape itself regressed once into a vertically-stretched titlebar icon,
/// issue #51).
fn app_icon() -> Option<Icon> {
    let n = crate::icon::ICON_SIZE;
    Icon::from_rgba(crate::icon::app_icon_rgba(), n, n).ok()
}
