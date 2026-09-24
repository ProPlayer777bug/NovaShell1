//! NovaShell GTK deep end: owns the window and the WebView, routes every
//! message between the JavaScript UI and the Rust core, and supervises game
//! launches so metadata keeps flowing when a title is running.

use std::cell::RefCell;
use std::rc::Rc;
use std::sync::mpsc;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use anyhow::{anyhow, Context, Result};
use gtk4 as gtk;
use gtk::glib;
use serde_json::{json, Value};
use webkit6::prelude::*;

use crate::controllers::Action;
use crate::games::{format_playtime, Game, Library};
use crate::launcher::Spec;
use crate::settings::Config;
use crate::system::{self, ControllerInfo, CpuSampler};
use crate::{integrations, ui, util};

#[derive(Clone, Debug, Default)]
pub struct RunOptions {
    pub fullscreen: bool,
    pub windowed: bool,
    pub debug: bool,
    pub watchdog: bool,
    pub help: bool,
}

/// Choose the graphics backend *before* GTK/WebKit initialize.
///
/// Under WSLg (`/mnt/wslg` plus `WSL_INTEROP`) the Mesa D3D12/EGL stack cannot
/// create a surfaceless EGL display, which is what GTK4's GSK renderer and
/// WebKitGTK probe at startup. That leaves the window mapped but blank.
/// Detect WSLg and fall back to a normal X11 window with GTK's cairo renderer
/// and WebKit's CPU (DMABUF-less) path — the combination verified to work.
/// On real desktops (no `/mnt/wslg`) we leave rendering alone (full GPU).
/// All settings respect an explicit user-supplied env override.
pub fn configure_graphics_backend() {
    if !std::path::Path::new("/mnt/wslg").exists()
        || std::env::var_os("WSL_INTEROP").is_none()
    {
        return;
    }
    eprintln!("NovaShell: WSLg detected — using X11 + software rendering fallback");
    for (key, value) in [
        ("GDK_BACKEND", "x11"),
        ("GSK_RENDERER", "cairo"),
        ("WEBKIT_DISABLE_DMABUF_RENDERER", "1"),
        ("WEBKIT_FORCE_SANDBOX", "0"),
    ] {
        if std::env::var_os(key).is_none() {
            std::env::set_var(key, value);
        }
    }
}

pub fn parse_args() -> RunOptions {
    let mut o = RunOptions {
        fullscreen: true,
        windowed: false,
        debug: false,
        watchdog: false,
        help: false,
    };
    for a in std::env::args().skip(1) {
        match a.as_str() {
            "-h" | "--help" => o.help = true,
            "--fullscreen" => o.fullscreen = true,
            "-w" | "--windowed" | "--dev" => {
                o.fullscreen = false;
                o.windowed = true;
            }
            "-d" | "--debug" => o.debug = true,
            "--watchdog" => o.watchdog = true,
            _ => {}
        }
    }
    let ui_override = std::env::var("NOVASHELL_UI").unwrap_or_default();
    if !ui_override.is_empty() {
        o.windowed = true;
        o.fullscreen = false;
    }
    o
}

// ---------------------------------------------------------------------------
// Logging
// ---------------------------------------------------------------------------

struct FileLogger {
    file: Mutex<Option<std::fs::File>>,
    level: log::LevelFilter,
}

impl log::Log for FileLogger {
    fn enabled(&self, metadata: &log::Metadata) -> bool {
        metadata.level() <= self.level
    }

    fn log(&self, record: &log::Record) {
        if !self.enabled(record.metadata()) {
            return;
        }
        let line = format!(
            "{} [{:<5}] {}",
            chrono::Local::now().format("%Y-%m-%d %H:%M:%S%.3f"),
            record.level(),
            record.args()
        );
        if let Ok(mut f) = self.file.lock() {
            use std::io::Write;
            if let Some(file) = f.as_mut() {
                let _ = writeln!(file, "{line}");
            }
        }
        if self.level == log::LevelFilter::Debug {
            eprintln!("{line}");
        }
    }

    fn flush(&self) {
        if let Ok(mut f) = self.file.lock() {
            if let Some(file) = f.as_mut() {
                use std::io::Write;
                let _ = file.flush();
            }
        }
    }
}

fn init_logging(debug: bool) {
    let _ = util::ensure_dirs();
    let level = if debug {
        log::LevelFilter::Debug
    } else {
        log::LevelFilter::Info
    };
    let path = util::logs_dir().join(format!(
        "nova-{}.log",
        chrono::Local::now().format("%Y%m%d")
    ));
    let file = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(&path)
        .ok();
    let logger = Box::leak(Box::new(FileLogger { file: Mutex::new(file), level }));
    let _ = log::set_logger(logger);
    log::set_max_level(level);
    log::info!("NovaShell {} starting right up", env!("CARGO_PKG_VERSION"));
    log::info!("log file: {}", path.display());
}

fn install_panic_hook() {
    std::panic::set_hook(Box::new(|info| {
        log::error!("panic: {info}");
        let marker = util::data_dir().join("crash.marker");
        let _ = std::fs::write(&marker, format!("{info}\n"));
    }));
}

// ---------------------------------------------------------------------------
// Watchdog
// ---------------------------------------------------------------------------

/// Run the shell as a child, restarting it if it crashes.
pub fn watchdog(opts: &RunOptions) -> Result<()> {
    configure_graphics_backend();
    init_logging(opts.debug);
    log::warn!("watchdog supervising the shell (max 3 restarts)");
    let exe = std::env::current_exe().context("resolving current executable")?;
    let original: Vec<String> = std::env::args()
        .skip(1)
        .filter(|a| a != "--watchdog")
        .collect();

    let mut restarts = 0;
    loop {
        let status = std::process::Command::new(&exe).args(&original).status();
        match status {
            Ok(s) if s.code() == Some(0) => {
                log::info!("shell exited cleanly; watchdog done");
                return Ok(());
            }
            Ok(s) => log::warn!("shell exited with {:?}; scheduling restart", s.code()),
            Err(e) => log::error!("failed to start shell: {e}"),
        }
        restarts += 1;
        if restarts > 3 {
            log::error!("shell failed {restarts} times in a row; giving up");
            return Err(anyhow!("no such luck: shell keeps failing"));
        }
        std::thread::sleep(Duration::from_secs(2));
    }
}

// ---------------------------------------------------------------------------
// Shell body
// ---------------------------------------------------------------------------

/// The live shell closure: everything one WebView message handler can touch.
pub struct RunningSession {
    id: String,
    title: String,
}

struct AppState {
    webview: webkit6::WebView,
    window: gtk::Window,
    config: Rc<RefCell<Config>>,
    library: Rc<RefCell<Library>>,
    cpu: Rc<RefCell<CpuSampler>>,
    controllers: Arc<Mutex<Vec<ControllerInfo>>>,
    sender: mpsc::Sender<String>,
    running: Rc<RefCell<Option<RunningSession>>>,
    main_loop: glib::MainLoop,
    opts: RunOptions,
}

pub fn run(opts: RunOptions) -> Result<()> {
    configure_graphics_backend();
    init_logging(opts.debug);
    install_panic_hook();
    util::ensure_dirs().context("creating data directories")?;

    if opts.help {
        print_help();
        return Ok(());
    }

    gtk::init().map_err(|e| anyhow!("GTK init failed: {e}"))?;
    let main_loop = glib::MainLoop::new(None, false);
    activate(&main_loop, &opts)?;
    main_loop.run();
    Ok(())
}

fn print_help() {
    eprintln!("NovaShell — console-style desktop shell for Ubuntu");
    eprintln!();
    eprintln!("  --fullscreen   start borderless fullscreen (default)");
    eprintln!("  --windowed     run in a windowed mode (dev)");
    eprintln!("  --dev          alias for --windowed");
    eprintln!("  --debug        verbose logging to stderr + log file");
    eprintln!("  --watchdog     supervise the shell and restart on crash");
}

fn activate(main_loop: &glib::MainLoop, opts: &RunOptions) -> Result<()> {
    log::debug!("building NovaShell UI…");
    let cfg = Config::load();

    // One-time full library scan; refresh happens on demand from the UI.
    let mut library = Library::load();
    {
        let found = crate::plugins::scan(&cfg);
        let seen: std::collections::HashSet<String> =
            found.iter().map(|g| g.id.clone()).collect();
        library.merge(found);
        library.prune_missing(&seen, true);
        let _ = library.save();
        log::info!(
            "library ready: {} entries",
            library.all_sorted().len()
        );
    }

    let ucm = webkit6::UserContentManager::new();
    let state_slot: Rc<RefCell<Option<AppState>>> = Rc::new(RefCell::new(None));

    // JS -> Rust messages land here.
    {
        let slot = state_slot.clone();
        ucm.connect_script_message_received(Some(ui::namespace()), move |_mg, value| {
            let st = slot.borrow();
            if let Some(state) = st.as_ref() {
                if let Some(msg) = ui::decode_message(value) {
                    if let Some(cmd) = msg.get("cmd").and_then(|c| c.as_str()) {
                        log::debug!("js -> rust: {cmd}");
                    }
                    route(state, msg);
                }
            }
        });
    }

    let webview = webkit6::WebView::builder()
        .user_content_manager(&ucm)
        .build();
    webview.set_background_color(&gtk::gdk::RGBA::new(0.016, 0.02, 0.04, 1.0));
    if opts.debug {
        if let Some(settings) = webkit6::prelude::WebViewExt::settings(&webview) {
            settings.set_enable_developer_extras(true);
        }
    }

    let window = gtk::Window::builder()
        .title("NovaShell")
        .default_width(1920)
        .default_height(1080)
        .build();
    window.set_child(Some(&webview));
    if opts.fullscreen {
        window.fullscreen();
    } else {
        window.maximize();
    }
window.present();
    {
        let close_loop = main_loop.clone();
        window.connect_close_request(move |_win| {
            close_loop.quit();
            glib::Propagation::Proceed
        });
    }

    // Core event bus (Rust threads -> main context -> UI).
    let (sender, receiver) = mpsc::channel::<String>();

    // Controller reader: updates the shared device list and forwards presses.
    let controllers: Arc<Mutex<Vec<ControllerInfo>>> = Arc::new(Mutex::new(Vec::new()));
    if cfg.controller.enabled {
        let ctrl_shared = controllers.clone();
        let ctl_cfg = cfg.controller.clone();
        let ctl_sender = sender.clone();
        std::thread::Builder::new()
            .name("novashell-input".into())
            .spawn(move || {
                let rx = crate::controllers::spawn(ctl_cfg);
                while let Ok(ev) = rx.recv() {
                    if ev.action == Action::None {
                        // Device connect / disconnect.
                        let mut list = ctrl_shared.lock().unwrap();
                        if ev.pressed {
                            if let Some(i) = list.iter_mut().find(|i| i.id == ev.pad) {
                                i.name = ev.name.clone();
                            } else {
                                list.push(ControllerInfo {
                                    id: ev.pad,
                                    name: ev.name.clone(),
                                    ..Default::default()
                                });
                            }
                        } else {
                            list.retain(|i| i.id != ev.pad);
                        }
                        continue;
                    }
                    let msg = json!({
                        "event": "controller",
                        "pad": ev.pad,
                        "action": ev.action.as_str(),
                        "pressed": ev.pressed,
                        "axis": ev.is_axis,
                    })
                    .to_string();
                    if ctl_sender.send(msg).is_err() {
                        break;
                    }
                }
            })?;
    }

    // Rust -> UI events (controller presses, game exits, live status).
    let slot = state_slot.clone();
    glib::timeout_add_local(Duration::from_millis(20), move || {
        if let Some(state) = slot.borrow().as_ref() {
            while let Ok(msg) = receiver.try_recv() {
                if let Ok(v) = serde_json::from_str::<Value>(&msg) {
                    handle_core_event(state, &v);
                }
            }
        }
        glib::ControlFlow::Continue
    });

    // Live status pushed to the status bar every few seconds.
    let slot = state_slot.clone();
    glib::timeout_add_local(Duration::from_secs(4), move || {
        if let Some(state) = slot.borrow().as_ref() {
            let status = status_payload(state);
            ui::dispatch(
                &state.webview,
                &json!({ "event": "status", "data": status }),
            );
        }
        glib::ControlFlow::Continue
    });

    let html = ui_html();
    webview.load_html(&html, Some("novashell://ui/"));

    *state_slot.borrow_mut() = Some(AppState {
        webview,
        window,
        config: Rc::new(RefCell::new(cfg)),
        library: Rc::new(RefCell::new(library)),
        cpu: Rc::new(RefCell::new(CpuSampler::new())),
        controllers,
        sender,
        running: Rc::new(RefCell::new(None)),
        main_loop: main_loop.clone(),
        opts: opts.clone(),
    });
    Ok(())
}

/// UI document from `ui/index.html`, overridable with `NOVASHELL_UI`.
fn ui_html() -> String {
    let embedded = include_str!("../../ui/index.html");
    if let Ok(path) = std::env::var("NOVASHELL_UI") {
        if let Ok(text) = std::fs::read_to_string(&path) {
            log::info!("loading custom UI from {path}");
            return text;
        }
        log::warn!("NOVASHELL_UI points to unreadable file {path}; using built-in UI");
    }
    embedded.to_string()
}

// ---------------------------------------------------------------------------
// Command routing (JS -> Rust)
// ---------------------------------------------------------------------------

fn route(state: &AppState, msg: Value) {
    let Some(cmd) = msg.get("cmd").and_then(|c| c.as_str()) else {
        log::warn!("js message without a cmd field: {msg}");
        return;
    };
    let id = msg.get("id").and_then(|i| i.as_str()).unwrap_or("");

    match cmd {
        "boot" => reply(state, id, boot_payload(state)),

        "library:refresh" => {
            let games = refresh_library(state);
            reply(state, id, json!({ "ok": true, "games": games }));
        }

        "library:favorite" => {
            let game_id = msg["game"].as_str().unwrap_or("");
            let fav = msg["favorite"].as_bool().unwrap_or(false);
            let res = state.library.borrow_mut().set_favorite(game_id, fav);
            reply(state, id, json!({ "ok": res.is_ok() }));
            send_event(state, json!({ "event": "library_changed" }));
        }

        "game:launch" => {
            let game_id = msg["id"].as_str().unwrap_or("");
            launch_by_id(state, game_id, id);
        }

        "app:launch" => {
            let program = msg["program"].as_str().unwrap_or("");
            let name = msg["name"].as_str().unwrap_or(program).to_string();
            let args: Vec<String> = msg["args"]
                .as_array()
                .map(|a| a.iter().filter_map(|x| x.as_str().map(String::from)).collect())
                .unwrap_or_default();
            let spec = Spec::new(name, program).args(args);
            match launch_spec(state, &spec, None) {
                Ok(()) => reply(state, id, json!({ "ok": true })),
                Err(e) => reply(state, id, json!({ "ok": false, "error": e.to_string() })),
            }
        }

        "settings:get" => reply(state, id, json!({ "config": state.config.borrow().clone() })),
        "settings:set" => settings_set(state, &msg),

        "quick:volume" => {
            let value = msg["value"].as_u64().map(|v| v as u8).unwrap_or(50);
            let result = system::set_audio_volume(value);
            let current = system::audio_volume();
            reply(
                state,
                id,
                json!({ "ok": result.is_ok(), "volume": current }),
            );
        }

        "quick:brightness" => {
            let value = msg["value"].as_u64().map(|v| v as u8).unwrap_or(50);
            let result = system::set_brightness(value);
            let current = system::brightness();
            reply(
                state,
                id,
                json!({ "ok": result.is_ok(), "brightness": current }),
            );
        }

        "power:action" => {
            let action = msg["action"].as_str().unwrap_or("");
            match action {
                "desktop" | "quit" => {
                    log::info!("returning to desktop (exiting shell)");
                    state.main_loop.quit();
                }
                _ => {
                    let res = system_power(action);
                    reply(state, id, json!({ "ok": res.is_ok() }));
                }
            }
        }

        "system:screenshot" => {
            let mode = msg["mode"].as_str().unwrap_or("auto");
            let path = system::take_screenshot(mode);
            reply(
                state,
                id,
                json!({
                    "ok": path.is_some(),
                    "path": path.as_ref().map(|p| p.to_string_lossy().to_string())
                }),
            );
        }

        "system:status" => reply(state, id, status_payload(state)),
        "controller:list" => {
            let ctrls: Vec<ControllerInfo> = state.controllers.lock().unwrap().clone();
            reply(state, id, json!({ "controllers": ctrls }));
        }
        "system:identity" => reply(
            state,
            id,
            json!({ "user": util::user_name(), "host": util::host_name() }),
        ),
        "echo" => reply(state, id, msg.clone()),
        "shutdown" => {
            // Dev/crash handling: clean exit so the watchdog can restart.
            state.main_loop.quit();
            state.window.close();
        }
        _ => log::warn!("unknown cmd '{cmd}'"),
    }
}

fn system_power(action: &str) -> Result<()> {
    match action {
        "suspend" => system::suspend(),
        "reboot" => system::reboot(),
        "poweroff" => system::poweroff(),
        "logout" => system::logout(),
        _ => Err(anyhow!("unknown power action '{action}'")),
    }
}

fn refresh_library(state: &AppState) -> Vec<Value> {
    let cfg = state.config.borrow().clone();
    let found = crate::plugins::scan(&cfg);
    let mut lib = state.library.borrow_mut();
    let seen: std::collections::HashSet<String> = found.iter().map(|g| g.id.clone()).collect();
    lib.merge(found);
    lib.prune_missing(&seen, true);
    let _ = lib.save();
    lib.all_sorted().iter().map(game_json).collect()
}

fn boot_payload(state: &AppState) -> Value {
    let games: Vec<Value> = state
        .library
        .borrow()
        .all_sorted()
        .iter()
        .map(game_json)
        .collect();
    json!({
        "ok": true,
        "version": env!("CARGO_PKG_VERSION"),
        "config": state.config.borrow().clone(),
        "library": { "games": games },
        "status": status_payload(state),
        "identity": {
            "user": util::user_name(),
            "host": util::host_name(),
        },
    })
}

fn status_payload(state: &AppState) -> Value {
    let mut cpu = state.cpu.borrow_mut();
    let ctrls: Vec<ControllerInfo> = state.controllers.lock().unwrap().clone();
    let snap = system::snapshot(&mut cpu, &ctrls);
    serde_json::to_value(snap).unwrap_or(Value::Null)
}

fn game_json(g: &Game) -> Value {
    json!({
        "id": g.id,
        "title": g.title,
        "source": g.source,
        "favicon": g.icon.as_ref().map(|p| p.to_string_lossy().to_string()),
        "artwork": g.artwork.as_ref().map(|p| p.to_string_lossy().to_string()),
        "last_played": g.last_played.filter(|t| *t > 0),
        "playtime": format_playtime(g.playtime_secs),
        "playtime_secs": g.playtime_secs,
        "favorite": g.favorite,
        "installed": g.installed,
    })
}

fn settings_set(state: &AppState, msg: &Value) {
    let mut cfg = state.config.borrow().clone();
    if let Some(v) = msg.get("accent").and_then(|x| x.as_str()) {
        if v.starts_with('#') && v.len() == 7 {
            cfg.accent = v.to_string();
        }
    }
    if let Some(v) = msg.get("theme").and_then(|x| x.as_str()) {
        if !v.is_empty() {
            cfg.theme = v.to_string();
        }
    }
    if let Some(v) = msg.get("ui_scale").and_then(|x| x.as_f64()) {
        cfg.ui_scale = v.clamp(0.5, 3.0);
    }
    if let Some(v) = msg.get("animations").and_then(|x| x.as_bool()) {
        cfg.animations = v;
    }
    if let Some(v) = msg.get("performance_mode").and_then(|x| x.as_bool()) {
        cfg.performance_mode = v;
    }
    if let Some(v) = msg.get("fullscreen").and_then(|x| x.as_bool()) {
        cfg.fullscreen = v;
    }
    if let Some(v) = msg.get("screenshot_mode").and_then(|x| x.as_str()) {
        if !v.is_empty() {
            cfg.screenshot_mode = v.to_string();
        }
    }
    if let Some(v) = msg.get("steam_enabled").and_then(|x| x.as_bool()) {
        cfg.launchers.steam_enabled = v;
    }
    if let Some(v) = msg.get("heroic_enabled").and_then(|x| x.as_bool()) {
        cfg.launchers.heroic_enabled = v;
    }
    if let Some(v) = msg.get("desktop_enabled").and_then(|x| x.as_bool()) {
        cfg.launchers.desktop_enabled = v;
    }
    if let Some(v) = msg.get("controller_enabled").and_then(|x| x.as_bool()) {
        cfg.controller.enabled = v;
    }
    if let Some(v) = msg.get("vibration").and_then(|x| x.as_bool()) {
        cfg.controller.vibration = v;
    }
    let saved = cfg.save();
    *state.config.borrow_mut() = cfg;
    if let Err(e) = &saved {
        log::warn!("failed to persist settings: {e}");
    }
    send_event(
        state,
        json!({
            "event": "settings_changed",
            "config": state.config.borrow().clone(),
        }),
    );
}

// ---------------------------------------------------------------------------
// Launches
// ---------------------------------------------------------------------------

fn launch_by_id(state: &AppState, game_id: &str, id: &str) {
    let game = match state.library.borrow().get(game_id) {
        Some(g) => g.clone(),
        None => {
            reply(state, id, json!({ "ok": false, "error": "game not found" }));
            return;
        }
    };
    if !game.installed {
        reply(state, id, json!({ "ok": false, "error": "not installed" }));
        return;
    }
    let steam_bin = integrations::steam::steam_binary();
    let Some(spec) = game.launch.to_spec(&game.title, steam_bin.as_deref()) else {
        reply(state, id, json!({ "ok": false, "error": "not launchable" }));
        return;
    };
    match launch_spec(state, &spec, Some(game_id)) {
        Ok(()) => {
            let _ = state.library.borrow_mut().record_launch(game_id);
            reply(state, id, json!({ "ok": true }));
        }
        Err(e) => reply(state, id, json!({ "ok": false, "error": e.to_string() })),
    }
}

/// Spawn a process, hide the shell while it's fullscreen, and report back.
fn launch_spec(state: &AppState, spec: &Spec, library_id: Option<&str>) -> Result<()> {
    log::info!("launching: {}", spec.name);
    let child = crate::launcher::spawn(spec)?;
    let title = spec.name.clone();
    {
        let mut running = state.running.borrow_mut();
        *running = Some(RunningSession {
            id: library_id.unwrap_or_default().to_string(),
            title: title.clone(),
        });
    }
    send_event(state, json!({ "event": "game_start", "title": title }));
    if state.opts.fullscreen && !state.opts.windowed {
        state.window.set_visible(false);
    }

    let sender = state.sender.clone();
    std::thread::spawn(move || {
        let start = Instant::now();
        let mut child = child;
        let _ = child.wait();
        let secs = start.elapsed().as_secs();
        let msg = json!({
            "event": "game_exit",
            "_internal": true,
            "title": title,
            "secs": secs,
        })
        .to_string();
        let _ = sender.send(msg);
    });
    Ok(())
}

// ---------------------------------------------------------------------------
// Rust -> UI events
// ---------------------------------------------------------------------------

fn handle_core_event(state: &AppState, v: &Value) {
    // Internal side effects first.
    if v.get("_internal").and_then(|x| x.as_bool()).unwrap_or(false)
        && v["event"].as_str() == Some("game_exit")
    {
        let secs = v["secs"].as_u64().unwrap_or(0);
        let running = state.running.borrow_mut().take();
        if let Some(r) = running {
            if !r.id.is_empty() {
                let close = state.library.borrow_mut().add_playtime(&r.id, secs);
                if let Err(e) = close {
                    log::warn!("failed to persist playtime: {e}");
                }
            }
            log::info!("{} closed after {secs}s", r.title);
            if state.opts.fullscreen && !state.opts.windowed {
                state.window.present();
                state.window.fullscreen();
            }
        }
    }
    ui::dispatch(&state.webview, v);
}

fn send_event(state: &AppState, payload: Value) {
    ui::dispatch(&state.webview, &payload);
}

fn reply(state: &AppState, id: &str, data: Value) {
    ui::dispatch(
        &state.webview,
        &json!({ "reply": true, "id": id, "data": data }),
    );
}