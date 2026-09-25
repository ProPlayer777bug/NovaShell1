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
use crate::games::{format_playtime, Game, Launch, Library};
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
    /// `novashell --play <file>`: play a game file headlessly and exit.
    pub play: Option<String>,
    /// `novashell --win-file <file>`: a Windows file was opened from the file
    /// manager. It is inspected and queued for the user to confirm; it is never
    /// executed by this process.
    pub win_file: Option<String>,
    /// `novashell --install-associations`: register Nova as the handler for
    /// Windows file types. Run from the install script and the setup screen.
    pub install_associations: bool,
}

/// Normalise `$HOME` so the data dirs (library, config, logs) are the same
/// no matter how the shell is launched. `wsl -u root -c` exports a mangled
/// `HOME=C:Usersdell`, which would split the library between two directories.
fn normalize_home() {
    let Some(h) = std::env::var_os("HOME") else {
        return;
    };
    let hs = h.to_string_lossy();
    if hs.starts_with('/') {
        return;
    }
    let whoami = util::user_name();
    let home = if whoami.is_empty() || whoami == "root" {
        "/root".to_string()
    } else {
        format!("/home/{whoami}")
    };
    std::env::set_var("HOME", &home);
    eprintln!("NovaShell: HOME was '{hs}'; normalized to {home}");
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
    let is_wsl = std::path::Path::new("/mnt/wslg").exists()
        || std::env::var_os("WSL_DISTRO_NAME").is_some()
        || std::env::var_os("WSL_INTEROP").is_some();
    if !is_wsl {
        return;
    }
    // WSLg is up whenever /mnt/wslg is mounted; some shells/services do not
    // export DISPLAY even though the socket exists.
    if std::env::var_os("DISPLAY").is_none() {
        let x0 = std::path::Path::new("/tmp/.X11-unix/X0");
        if std::path::Path::new("/mnt/wslg").exists() || x0.exists() {
            std::env::set_var("DISPLAY", ":0");
        }
    }
    eprintln!("NovaShell: WSL detected — using X11 + software rendering fallback");
    // Game packages install into /usr/games, which desktop sessions often omit
    // from PATH; add the usual extras so emulators resolve (and so the apps we
    // launch inherit a working PATH).
    if let Some(have) = std::env::var_os("PATH") {
        let mut parts: Vec<std::ffi::OsString> = std::env::split_paths(&have)
            .map(|p| p.into_os_string())
            .collect();
        for extra in ["/usr/games", "/usr/local/games", "/snap/bin"] {
            let p = std::ffi::OsString::from(extra);
            if !parts.contains(&p) {
                parts.push(p);
            }
        }
        if let Ok(joined) = std::env::join_paths(parts) {
            std::env::set_var("PATH", joined);
        }
    }
    for (key, value) in [
        ("GDK_BACKEND", "x11"),
        // WSLg has no session bus/type, and GNOME apps (Nautilus) refuse to
        // start without these: "Failed to initialize display server connection:
        // Unsupported or missing session type ''".
        ("XDG_SESSION_TYPE", "x11"),
        ("XDG_SESSION_CLASS", "x11"),
        ("XDG_CURRENT_DESKTOP", "GNOME"),
        // No dconf/dbus in this session: keep settings in-process so apps
        // don't stall on a settings bus that never answers.
        ("GSETTINGS_BACKEND", "memory"),
        ("NO_AT_BRIDGE", "1"),
        ("GSK_RENDERER", "cairo"),
        ("WEBKIT_DISABLE_DMABUF_RENDERER", "1"),
        ("WEBKIT_FORCE_SANDBOX", "0"),
        // Qt-based titles (PCSX2, Dolphin) try a wayland platform plugin that
        // does not exist here; point them at the X11 backend we actually run.
        ("QT_QPA_PLATFORM", "xcb"),
        // No desktop cues a cursor theme; give WebKit/GTK one so the pointer
        // is actually visible in the WSLg window.
        ("XCURSOR_THEME", "Adwaita"),
        ("XCURSOR_SIZE", "24"),
        // Force mesa onto llvmpipe so the d3d12/zink paths can never crash.
        ("LIBGL_ALWAYS_SOFTWARE", "1"),
        ("GALLIUM_DRIVER", "llvmpipe"),
    ] {
        if std::env::var_os(key).is_none() {
            std::env::set_var(key, value);
        }
    }
    // Without an X settings daemon, X11 apps (Brave, Qt emulators) fall back
    // to an empty cursor unless the search path is explicit.
    if std::env::var_os("XCURSOR_PATH").is_none() {
        let icons_home = dirs::home_dir()
            .map(|h| h.join(".icons").to_string_lossy().into_owned())
            .unwrap_or_default();
        std::env::set_var(
            "XCURSOR_PATH",
            format!("/usr/share/icons:{icons_home}:/usr/share/pixmaps:"),
        );
    }
}

pub fn parse_args() -> RunOptions {
    let mut o = RunOptions {
        fullscreen: true,
        windowed: false,
        debug: false,
        watchdog: false,
        help: false,
        play: None,
        win_file: None,
        install_associations: false,
    };
    let mut args = std::env::args().skip(1);
    while let Some(a) = args.next() {
        match a.as_str() {
            "-h" | "--help" => o.help = true,
            "--fullscreen" => o.fullscreen = true,
            "-w" | "--windowed" | "--dev" => {
                o.fullscreen = false;
                o.windowed = true;
            }
            "-d" | "--debug" => o.debug = true,
            "--watchdog" => o.watchdog = true,
            // `novashell --play <game.iso>`: headless "open this game with the
            // right emulator". This is what the file manager runs when a game
            // file is double-clicked, so the selection is handed to the shell
            // instead of the desktop trying to mount the disc image.
            "--play" => o.play = args.next(),
            other if other.starts_with("--play=") => {
                o.play = Some(other["--play=".len()..].to_string())
            }
            // File-manager handler for Windows files: inspect and queue for
            // confirmation. This mode never executes the file.
            "--win-file" => o.win_file = args.next(),
            other if other.starts_with("--win-file=") => {
                o.win_file = Some(other["--win-file=".len()..].to_string())
            }
            // `novashell --install-associations`: register the .exe/.msi
            // handlers so the file manager routes them through this shell.
            "--install-associations" => o.install_associations = true,
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
    /// PID of the launched process group leader, so closing the shell can take
    /// the running app down with it instead of leaving it orphaned.
    pid: u32,
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
    /// Helper windows opened by the shell (file manager, chooser) shown in the
    /// taskbar so the user can switch back to them or close them.
    helpers: Rc<RefCell<Vec<HelperWindow>>>,
    /// Base URL of the loopback server that fronts the UI document and icons.
    ui_base: String,
    /// Icon/artwork path map served by that server, kept fresh on refresh.
    ui_icons: Arc<Mutex<std::collections::HashMap<String, std::path::PathBuf>>>,
    /// Every process the Runtime Manager has launched, keyed by pid. The
    /// `running` field above remains the single foreground session that drives
    /// the taskbar and playtime accounting.
    procs: crate::runtime::procs::ProcessManager,
    /// Installed Windows applications/games (metadata catalogue).
    win_apps: Arc<Mutex<Vec<crate::runtime::apps::WindowsApp>>>,
    main_loop: glib::MainLoop,
    opts: RunOptions,
}

/// A helper window the shell opened on the user's behalf.
#[derive(Clone)]
struct HelperWindow {
    pid: u32,
    title: String,
}

/// Wait until the display reports a usable monitor size.
///
/// When the shell starts as a service during WSLg start-up, the display can
/// briefly report a 0x0 monitor; sizing to that produced a tiny 640x480 window.
fn wait_for_monitor(max_wait: Duration) -> Option<(i32, i32)> {
    let deadline = Instant::now() + max_wait;
    loop {
        if let Some(size) = current_monitor_size() {
            if size.0 >= 640 && size.1 >= 480 {
                return Some(size);
            }
        }
        if Instant::now() >= deadline {
            return current_monitor_size();
        }
        std::thread::sleep(Duration::from_millis(250));
    }
}

/// The primary monitor's size, or `None` while the display is not ready.
fn current_monitor_size() -> Option<(i32, i32)> {
    let disp = gtk::gdk::Display::default()?;
    use gtk::gdk::prelude::MonitorExt;
    use gtk::gio::prelude::ListModelExt;
    let mon = disp
        .monitors()
        .item(0)
        .and_then(|o| o.downcast::<gtk::gdk::Monitor>().ok())?;
    let g = MonitorExt::geometry(&mon);
    Some((g.width(), g.height()))
}

/// Window size for windowed mode. Clamped to the monitor minus frame headroom
/// and to 1440x860 absolute: the Windows desktop here is 1536x960 logical
/// (1920x1200 physical at 125% scaling) while WSLg's X root reports
/// 1920x1200, so a window sized to the full X root can land outside the
/// visible desktop and show nothing but a taskbar entry.
fn fitted_window_size() -> (i32, i32) {
    let (w, h) = match current_monitor_size() {
        // A degenerate (0x0) report means the display is not up yet: use a
        // sensible default rather than shrinking to the minimum.
        Some((mw, mh)) if mw >= 640 && mh >= 480 => (
            (mw - 80).max(640),
            (mh - 120).max(480),
        ),
        _ => (1280, 720),
    };
    (w.min(1440), h.min(860))
}

/// Terminate the app this shell launched (if any). Launched apps get their own
/// process group, so signalling the negated pid reaches every child process
/// (a browser's renderer/helper processes included). Without this, quitting
/// NovaShell left Brave (and its whole process tree) running forever.
fn terminate_running(state: &AppState) {
    let running = state.running.borrow_mut().take();
    if let Some(r) = running {
        if r.pid == 0 {
            return;
        }
        log::info!("closing launched app: {} (pid {})", r.title, r.pid);
        #[cfg(unix)]
        {
            let _ = std::process::Command::new("kill")
                .arg(format!("-{}", r.pid))
                .stdout(std::process::Stdio::null())
                .stderr(std::process::Stdio::null())
                .status();
        }
    }
}

/// Bring the shell window back on screen, re-applying the fitted size so it
/// never comes back larger than the display after an app was hidden.
fn show_shell(state: &AppState) {
    if state.opts.fullscreen && !state.opts.windowed {
        state.window.present();
        state.window.fullscreen();
    } else {
        let (w, h) = fitted_window_size();
        state.window.set_default_size(w, h);
        state.window.present();
    }
}

pub fn run(opts: RunOptions) -> Result<()> {
    normalize_home();
    configure_graphics_backend();
    eprintln!(
        "NovaShell: DISPLAY={:?} wslg={} x11sock={}",
        std::env::var_os("DISPLAY"),
        std::path::Path::new("/mnt/wslg").exists(),
        std::path::Path::new("/tmp/.X11-unix/X0").exists()
    );
    init_logging(opts.debug);
    install_panic_hook();
    util::ensure_dirs().context("creating data directories")?;

    if opts.help {
        print_help();
        return Ok(());
    }

    // Headless "play this game file" mode: used as the file manager's handler
    // for disc images, so double-clicking a game in Files starts it.
    if let Some(path) = opts.play.clone() {
        normalize_home();
        configure_graphics_backend();
        return play_game_file(&path);
    }

    // Registering the Windows file associations is a one-shot system change,
    // so it is an explicit flag rather than something a normal launch does.
    if opts.install_associations {
        let home = dirs::home_dir().unwrap_or_default();
        crate::runtime::apps::install_file_associations(&home)?;
        eprintln!("novashell: registered .exe/.msi handlers");
        return Ok(());
    }

    // A Windows file was opened from the file manager. Inspect it, remember a
    // previous decision if the user made one, and queue it for confirmation.
    // Nothing is executed here.
    if let Some(path) = opts.win_file.clone() {
        normalize_home();
        let file = std::path::PathBuf::from(&path);
        if !file.is_file() {
            eprintln!("novashell: not a file: {path}");
            std::process::exit(1);
        }
        use crate::runtime::decisions::PendingAction;
        let guess = crate::runtime::apps::guess_if_game(&file);
        let remembered = crate::runtime::decisions::Decisions::load().get(&file).map(|s| s.to_string());
        let action = match remembered.as_deref() {
            Some("wine") => PendingAction::Wine,
            Some("proton") => PendingAction::Proton,
            Some("never") => PendingAction::InspectOnly,
            _ => PendingAction::Ask,
        };
        crate::runtime::decisions::Decisions::default().add_pending(
            crate::runtime::decisions::PendingFile {
                path: file,
                action,
                is_game: guess.is_game,
                is_installer: crate::runtime::apps::is_installer(std::path::Path::new(&path)),
                reasons: guess.reasons,
                created_at: util::unix_now(),
            },
        );
        // If the shell is already running it picks the file up through the
        // pending list; otherwise the next launch shows it in the UI.
        eprintln!("novashell: queued {path} for confirmation in Nova Shell");
        return Ok(());
    }


    gtk::init().map_err(|e| anyhow!("GTK init failed (is a display available?): {e}"))?;
    eprintln!("NovaShell: gtk init ok");
    let main_loop = glib::MainLoop::new(None, false);
    let state_slot = activate(&main_loop, &opts)?;
    eprintln!("NovaShell: window presented, entering main loop");
    main_loop.run();
    // Quitting the shell must not leave a launched app (and its helper
    // processes) running; take the whole process group down with us.
    if let Some(state) = state_slot.borrow().as_ref() {
        terminate_running(state);
    }
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

/// A tiny loopback HTTP server for the UI document and its icons.
///
/// This WebKit build refuses to render both `data:` and `file://` images when
/// the page itself is loaded from a custom scheme, which is why app icons came
/// out as black boxes. Serving the UI over `http://127.0.0.1` gives the page a
/// normal same-origin context, so icons load.
struct UiServer {
    base: String,
    icons: Arc<Mutex<std::collections::HashMap<String, std::path::PathBuf>>>,
}

fn start_ui_server(
    html: String,
    icons: std::collections::HashMap<String, std::path::PathBuf>,
) -> Result<UiServer> {
    use std::io::{BufRead, BufReader, Read, Write};
    let listener = std::net::TcpListener::bind("127.0.0.1:0")?;
    let port = listener.local_addr()?.port();
    let base = format!("http://127.0.0.1:{port}/");
    let icons = Arc::new(Mutex::new(icons));
    let icon_map = icons.clone();

    std::thread::Builder::new()
        .name("novashell-uiserver".into())
        .spawn(move || {
            for stream in listener.incoming() {
                let Ok(mut stream) = stream else { continue };
                let icons = icon_map.clone();
                let html = html.clone();
                std::thread::spawn(move || {
                    let mut reader = BufReader::new(match stream.try_clone() {
                        Ok(s) => s,
                        Err(_) => return,
                    });
                    let mut request_line = String::new();
                    if reader.read_line(&mut request_line).is_err() {
                        return;
                    }
                    // Drain headers.
                    let mut line = String::new();
                    while let Ok(n) = reader.read_line(&mut line) {
                        if n == 0 || line == "\r\n" || line == "\n" {
                            break;
                        }
                        line.clear();
                    }
                    let path = request_line
                        .split_whitespace()
                        .nth(1)
                        .unwrap_or("/")
                        .to_string();

                    let (ctype, body): (&str, Vec<u8>) = if path == "/" || path.starts_with("/?") {
                        ("text/html; charset=utf-8", html.into_bytes())
                    } else if path.starts_with("/icon/") || path.starts_with("/art/") {
                        let want_art = path.starts_with("/art/");
                        let raw_id = path
                            .trim_start_matches("/icon/")
                            .trim_start_matches("/art/")
                            .split('?')
                            .next()
                            .unwrap_or("");
                        let key = if want_art {
                            format!("art:{}", percent_decode(raw_id))
                        } else {
                            percent_decode(raw_id)
                        };
                        let found = icons.lock().ok().and_then(|m| m.get(&key).cloned());
                        match found {
                            Some(file) => {
                                let mime = match file
                                    .extension()
                                    .map(|e| e.to_string_lossy().to_lowercase())
                                    .unwrap_or_default()
                                    .as_str()
                                {
                                    "svg" => "image/svg+xml",
                                    "jpg" | "jpeg" => "image/jpeg",
                                    "webp" => "image/webp",
                                    "xpm" => "image/x-xpixmap",
                                    _ => "image/png",
                                };
                                match std::fs::read(&file) {
                                    Ok(bytes) => (mime, bytes),
                                    Err(_) => ("text/plain", b"unreadable".to_vec()),
                                }
                            }
                            None => ("text/plain", b"no image".to_vec()),
                        }
                    } else {
                        ("text/plain", b"not found".to_vec())
                    };

                    let header = format!(
                        "HTTP/1.1 200 OK\r\nContent-Type: {ctype}\r\nContent-Length: {}\r\nCache-Control: no-store\r\nConnection: close\r\n\r\n",
                        body.len()
                    );
                    let _ = stream.write_all(header.as_bytes());
                    let _ = stream.write_all(&body);
                    let _ = stream.flush();
                    let _ = (&mut stream).read(&mut []);
                });
            }
        })?;
    Ok(UiServer { base, icons })
}

/// Decode `%XX` escapes in a URL path segment.
fn percent_decode(raw: &str) -> String {
    let bytes = raw.as_bytes();
    let mut out: Vec<u8> = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%' && i + 2 < bytes.len() {
            let hex = std::str::from_utf8(&bytes[i + 1..i + 3]).unwrap_or("");
            if let Ok(byte) = u8::from_str_radix(hex, 16) {
                out.push(byte);
                i += 3;
                continue;
            }
        }
        out.push(bytes[i]);
        i += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}

fn activate(
    main_loop: &glib::MainLoop,
    opts: &RunOptions,
) -> Result<Rc<RefCell<Option<AppState>>>> {
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

    // Serve the UI over loopback HTTP: this WebKit build refuses to render
    // data: and file: images on a custom-scheme page, which left every app icon
    // as an empty box. Same-origin http makes icons (and artwork) load.
    let mut icon_map: std::collections::HashMap<String, std::path::PathBuf> =
        Default::default();
    for g in library.all_sorted() {
        if let Some(p) = &g.icon {
            icon_map.insert(g.id.clone(), p.clone());
        }
        if let Some(p) = &g.artwork {
            icon_map.insert(format!("art:{}", g.id), p.clone());
        }
    }
    let server = start_ui_server(ui_html(), icon_map)?;
    let ui_base = server.base.clone();
    let ui_icons = server.icons.clone();
    log::info!("UI served from {ui_base}");

    let ucm = webkit6::UserContentManager::new();
    if !ucm.register_script_message_handler(ui::namespace(), None) {
        log::warn!("could not register script message handler '{}'", ui::namespace());
    }
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
    if let Some(settings) = webkit6::prelude::WebViewExt::settings(&webview) {
        settings.set_enable_write_console_messages_to_stdout(true);
    }
    if opts.debug {
        if let Some(settings) = webkit6::prelude::WebViewExt::settings(&webview) {
            settings.set_enable_developer_extras(true);
        }
    }

    let window = gtk::Window::builder()
        .title("NovaShell")
        .build();
    window.set_child(Some(&webview));
    if opts.fullscreen {
        window.fullscreen();
    } else {
        // The display may still be coming up when we start as a service; sizing
        // to a 0x0 monitor would give a tiny window.
        let _ = wait_for_monitor(Duration::from_secs(10));
        let (w, h) = fitted_window_size();
        log::debug!("windowed size {w}x{h}");
        window.set_default_size(w, h);
    }
    window.present();
    // If the monitor only reported its real size after we mapped, re-apply the
    // fitted size so the window is never left tiny or off-screen.
    if !(opts.fullscreen && !opts.windowed) {
        let win = window.clone();
        glib::timeout_add_local(Duration::from_millis(2500), move || {
            let (w, h) = fitted_window_size();
            win.set_default_size(w, h);
            glib::ControlFlow::Break
        });
    }
    // Under WSLg the surface does not always get keyboard focus on map;
    // force it onto the WebView right away and re-assert it for a few
    // seconds in case the compositor hands focus back to the terminal.
    webview.grab_focus();
    {
        let wv = webview.clone();
        let mut rounds = 0u32;
        glib::timeout_add_local(Duration::from_millis(300), move || {
            rounds += 1;
            wv.grab_focus();
            if rounds >= 12 {
                glib::ControlFlow::Break
            } else {
                glib::ControlFlow::Continue
            }
        });
    }
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

    // Load over loopback HTTP so same-origin icon requests succeed.
    webview.load_uri(&ui_base);

    *state_slot.borrow_mut() = Some(AppState {
        webview,
        window,
        config: Rc::new(RefCell::new(cfg)),
        library: Rc::new(RefCell::new(library)),
        cpu: Rc::new(RefCell::new(CpuSampler::new())),
        controllers,
        sender,
        running: Rc::new(RefCell::new(None)),
        helpers: Rc::new(RefCell::new(Vec::new())),
        ui_base,
        ui_icons,
        procs: crate::runtime::procs::ProcessManager::new(),
        win_apps: Arc::new(Mutex::new(
            crate::runtime::apps::WindowsApp::load_all(),
        )),
        main_loop: main_loop.clone(),
        opts: opts.clone(),
    });
    Ok(state_slot)
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
            let rom = msg["rom"].as_str();
            launch_by_id(state, game_id, rom, id);
        }

        "game:install" => {
            let game_id = msg["id"].as_str().unwrap_or("").to_string();
            let reply_id = id.to_string();
            let title = state
                .library
                .borrow()
                .get(&game_id)
                .map(|g| g.title.clone())
                .unwrap_or_default();
            let pkgs: Option<Vec<String>> = state.library.borrow().get(&game_id).and_then(|g| {
                match &g.launch {
                    Launch::Program { program, .. } => integrations::apps::apt_packages_for_program(program)
                        .map(|p| p.iter().map(|s| s.to_string()).collect()),
                    _ => None,
                }
            });
            match pkgs {
                Some(p) if !p.is_empty() => {
                    reply(state, &reply_id, json!({ "ok": true, "status": "installing" }));
                    let tx = state.sender.clone();
                    std::thread::Builder::new()
                        .name("novashell-install".into())
                        .spawn(move || {
                            let result = run_apt_install(&p);
                            let msg = json!({
                                "event": "_install_done",
                                "ok": result.is_ok(),
                                "error": result
                                    .as_ref()
                                    .err()
                                    .map(|e| e.to_string()),
                                "title": title,
                            })
                            .to_string();
                            let _ = tx.send(msg);
                        })
                        .ok();
                }
                _ => reply(
                    state,
                    &reply_id,
                    json!({
                        "ok": false,
                        "error": "No package available — this one needs a third-party repo.",
                    }),
                ),
            }
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

        // Taskbar contents: the shell plus anything it has open. WSLg gives
        // every app its own window, so the taskbar is how the user switches
        // between them.
        "windows:list" => {
            let mut items: Vec<Value> = vec![json!({
                "id": "shell",
                "title": "NovaShell",
                "kind": "shell",
                "pid": 0,
                "active": true,
            })];
            if let Some(r) = state.running.borrow().as_ref() {
                items.push(json!({
                    "id": format!("app-{}", r.pid),
                    "title": r.title,
                    "kind": "app",
                    "pid": r.pid,
                    "active": false,
                }));
            }
            for h in state.helpers.borrow().iter() {
                items.push(json!({
                    "id": format!("helper-{}", h.pid),
                    "title": h.title,
                    "kind": "helper",
                    "pid": h.pid,
                    "active": false,
                }));
            }
            reply(state, id, json!({ "ok": true, "windows": items }));
        }

        // Raise/focus an open window. Under WSLg the compositor owns focus, so
        // this is best effort — clicking the taskbar item should still bring
        // the window forward on compositors that honour _NET_ACTIVE_WINDOW.
        "windows:focus" => {
            let pid = msg["pid"].as_i64().unwrap_or(0);
            if pid > 0 {
                let _ = std::process::Command::new("xdotool")
                    .args([
                        "search",
                        "--pid",
                        &pid.to_string(),
                        "windowactivate",
                        "--sync",
                    ])
                    .stdout(std::process::Stdio::null())
                    .stderr(std::process::Stdio::null())
                    .status();
            } else {
                show_shell(state);
            }
            reply(state, id, json!({ "ok": true }));
        }

        // Close something the shell opened (app or helper window).
        "windows:close" => {
            let pid = msg["pid"].as_i64().unwrap_or(0);
            if pid > 0 {
                state.helpers.borrow_mut().retain(|h| h.pid as i64 != pid);
                let _ = std::process::Command::new("kill")
                    .arg(format!("-{pid}"))
                    .stdout(std::process::Stdio::null())
                    .stderr(std::process::Stdio::null())
                    .status();
                if let Some(r) = state.running.borrow().as_ref() {
                    if r.pid as i64 == pid {
                        state.running.borrow_mut().take();
                    }
                }
                show_shell(state);
            }
            reply(state, id, json!({ "ok": true }));
        }

        // Native file chooser for a game file — the same dialog a browser
        // shows for "upload file". The chosen path comes back to the UI as
        // `_rom_chosen`, which then boots the emulator with it.
        // ---------------- Runtime Manager ----------------
        "runtimes:list" => {
            let runtimes = crate::runtime::detect_all();
            reply(
                state,
                id,
                json!({ "ok": true, "runtimes": serde_json::to_value(runtimes).unwrap_or(Value::Null) }),
            );
        }

        "runtimes:detect" => {
            let runtimes = crate::runtime::detect_all();
            send_event(state, json!({ "event": "runtime.detected", "runtimes": runtimes }));
            reply(
                state,
                id,
                json!({ "ok": true, "runtimes": serde_json::to_value(runtimes).unwrap_or(Value::Null) }),
            );
        }

        "runtimes:validate" => {
            let runtime_id = msg["runtime"].as_str().unwrap_or("").to_string();
            let mut target = crate::runtime::LaunchTarget::default();
            if let Some(p) = msg["executable"].as_str() {
                target.executable = Some(std::path::PathBuf::from(p));
            }
            if let Some(p) = msg["prefix"].as_str() {
                target.prefix = Some(std::path::PathBuf::from(p));
            }
            let checks = crate::runtime::validate_runtime(&runtime_id, &target);
            reply(state, id, json!({ "ok": true, "checks": checks }));
        }

        "prefixes:list" => {
            reply(
                state,
                id,
                json!({ "ok": true, "prefixes": crate::runtime::prefixes::list() }),
            );
        }

        "prefixes:create" => {
            let app_id = msg["id"].as_str().unwrap_or("").to_string();
            let kind = match msg["kind"].as_str().unwrap_or("wine") {
                "proton" => crate::runtime::prefixes::PrefixKind::Proton,
                _ => crate::runtime::prefixes::PrefixKind::Wine,
            };
            // Long operation: create on a worker so the UI thread never blocks.
            let sender = state.sender.clone();
            let rid = id.to_string();
            std::thread::Builder::new()
                .name("novashell-prefix".into())
                .spawn(move || {
                    let result = crate::runtime::prefixes::create(kind, &app_id);
                    let payload = match result {
                        Ok(info) => json!({ "reply": true, "id": rid, "data": { "ok": true, "prefix": info } }),
                        Err(e) => json!({ "reply": true, "id": rid, "data": { "ok": false, "error": e.to_string() } }),
                    };
                    let _ = sender.send(payload.to_string());
                    let _ = sender.send(
                        json!({ "event": "prefix.created", "id": app_id, "kind": kind_dir(kind) })
                            .to_string(),
                    );
                })
                .ok();
        }

        "prefixes:delete" => {
            let app_id = msg["id"].as_str().unwrap_or("").to_string();
            let kind = match msg["kind"].as_str().unwrap_or("wine") {
                "proton" => crate::runtime::prefixes::PrefixKind::Proton,
                _ => crate::runtime::prefixes::PrefixKind::Wine,
            };
            match crate::runtime::prefixes::delete(kind, &app_id) {
                Ok(()) => {
                    send_event(state, json!({ "event": "prefix.deleted", "id": app_id }));
                    reply(state, id, json!({ "ok": true }));
                }
                Err(e) => reply(state, id, json!({ "ok": false, "error": e.to_string() })),
            }
        }

        "prefixes:repair" => {
            let app_id = msg["id"].as_str().unwrap_or("").to_string();
            let kind = match msg["kind"].as_str().unwrap_or("wine") {
                "proton" => crate::runtime::prefixes::PrefixKind::Proton,
                _ => crate::runtime::prefixes::PrefixKind::Wine,
            };
            match crate::runtime::prefixes::repair(kind, &app_id) {
                Ok(info) => reply(state, id, json!({ "ok": true, "prefix": info })),
                Err(e) => reply(state, id, json!({ "ok": false, "error": e.to_string() })),
            }
        }

        "procs:list" => {
            reply(
                state,
                id,
                json!({ "ok": true, "processes": state.procs.list() }),
            );
        }

        "procs:stop" | "procs:kill" => {
            let pid = msg["pid"].as_u64().unwrap_or(0) as u32;
            let outcome = if id.ends_with("kill") || msg["force"].as_bool().unwrap_or(false) {
                state.procs.kill(pid)
            } else {
                state.procs.stop(pid)
            };
            match outcome {
                Ok(()) => reply(state, id, json!({ "ok": true })),
                Err(e) => reply(state, id, json!({ "ok": false, "error": e.to_string() })),
            }
        }

        "runtime:logs" => {
            let app_id = msg["id"].as_str().unwrap_or("").to_string();
            let lines = msg["lines"].as_u64().unwrap_or(200).min(2000) as usize;
            reply(
                state,
                id,
                json!({ "ok": true, "lines": crate::runtime::logs::tail(&app_id, lines) }),
            );
        }

        // Windows application catalogue.
        "windows:apps" => {
            let apps = state.win_apps.lock().map(|g| g.clone()).unwrap_or_default();
            reply(state, id, json!({ "ok": true, "apps": apps }));
        }

        // Inspect a file the user picked: is it a Windows binary, an installer,
        // and does it look like a game? Nothing is executed here.
        "windows:inspect" => {
            let path = msg["path"].as_str().unwrap_or("").to_string();
            let p = std::path::Path::new(&path);
            let guess = crate::runtime::apps::guess_if_game(p);
            reply(
                state,
                id,
                json!({
                    "ok": true,
                    "path": path,
                    "exists": p.is_file(),
                    "is_windows_executable": crate::runtime::apps::is_windows_executable(p),
                    "is_installer": crate::runtime::apps::is_installer(p),
                    "guess_game": guess.is_game,
                    "reasons": guess.reasons,
                    "size": std::fs::metadata(p).map(|m| m.len()).unwrap_or(0),
                }),
            );
        }

        "windows:app:add" => {
            let app = match windows_app_from_msg(&msg) {
                Ok(a) => a,
                Err(e) => {
                    reply(state, id, json!({ "ok": false, "error": e.to_string() }));
                    return;
                }
            };
            let saved = {
                let mut apps = match state.win_apps.lock() {
                    Ok(g) => g,
                    Err(_) => {
                        reply(state, id, json!({ "ok": false, "error": "state unavailable" }));
                        return;
                    }
                };
                crate::runtime::apps::WindowsApp::upsert(&mut apps, app.clone());
                crate::runtime::apps::WindowsApp::save_all(&apps)
            };
            match saved {
                Ok(()) => {
                    // Make it visible in the existing Nova library.
                    if let Err(e) = add_windows_app_to_library(state, &app) {
                        log::warn!("could not add {} to library: {e}", app.name);
                    }
                    send_event(state, json!({ "event": "application.installed", "app": app }));
                    reply(state, id, json!({ "ok": true, "app": app }));
                }
                Err(e) => reply(state, id, json!({ "ok": false, "error": e.to_string() })),
            }
        }

        "windows:app:remove" => {
            let app_id = msg["id"].as_str().unwrap_or("").to_string();
            let result = {
                let mut apps = match state.win_apps.lock() {
                    Ok(g) => g,
                    Err(e) => {
                        reply(state, id, json!({ "ok": false, "error": e.to_string() }));
                        return;
                    }
                };
                apps.retain(|a| a.id != app_id);
                crate::runtime::apps::WindowsApp::save_all(&apps)
            };
            let _ = state.library.borrow_mut().delete_key(&app_id);
            let _ = state.library.borrow_mut().save();
            send_event(state, json!({ "event": "library_changed" }));
            match result {
                Ok(()) => reply(state, id, json!({ "ok": true })),
                Err(e) => reply(state, id, json!({ "ok": false, "error": e.to_string() })),
            }
        }

        // Launch a stored Windows application through its runtime.
        "windows:launch" => {
            let app_id = msg["id"].as_str().unwrap_or("").to_string();
            let app = state
                .win_apps
                .lock()
                .ok()
                .and_then(|apps| apps.iter().find(|a| a.id == app_id).cloned());
            match app {
                Some(app) => match launch_windows_app(state, &app) {
                    Ok(()) => reply(state, id, json!({ "ok": true, "pid": true })),
                    Err(e) => reply(state, id, json!({ "ok": false, "error": e.to_string() })),
                },
                None => reply(state, id, json!({ "ok": false, "error": "app not found" })),
            }
        }

        // Run a Windows installer in its own prefix. The installer runs
        // asynchronously: a cold `wineboot` is slow, and the UI must not block.
        "windows:install" => {
            let installer = msg["path"].as_str().unwrap_or("").to_string();
            let app_id = msg["id"].as_str().unwrap_or("").to_string();
            let name = msg["name"].as_str().unwrap_or(&app_id).to_string();
            let safe_id = match crate::runtime::security::safe_segment(&app_id) {
                Ok(v) => v,
                Err(e) => {
                    reply(state, id, json!({ "ok": false, "error": e.to_string() }));
                    return;
                }
            };
            if let Err(e) = crate::runtime::security::ensure_not_root() {
                reply(state, id, json!({ "ok": false, "error": e.to_string() }));
                return;
            }
            let installer_path = std::path::PathBuf::from(&installer);
            if !installer_path.is_file() {
                reply(state, id, json!({ "ok": false, "error": "installer not found" }));
                return;
            }
            if !crate::runtime::apps::is_windows_executable(&installer_path) {
                reply(
                    state,
                    id,
                    json!({ "ok": false, "error": "not a Windows executable or installer" }),
                );
                return;
            }
            // Always a fresh, per-application prefix, owned by the user who
            // will run it (Wine refuses prefixes owned by anyone else).
            let prefix = match crate::runtime::wine::WineRuntime::resolve_owned_prefix(&safe_id) {
                Ok(p) => p,
                Err(e) => {
                    reply(state, id, json!({ "ok": false, "error": e.to_string() }));
                    return;
                }
            };
            let rid = id.to_string();
            let reply_sender = state.sender.clone();
            let event_sender = state.sender.clone();
            std::thread::Builder::new()
                .name("novashell-wineinstall".into())
                .spawn(move || {
                    use crate::runtime::Runtime as _;
                    let mut rt = crate::runtime::wine::WineRuntime::new();
                    let _ = rt.detect();
                    let outcome = rt.init_prefix(&prefix, 300).and_then(|_| {
                        let mut target = crate::runtime::LaunchTarget::new(&safe_id, &name)
                            .with_executable(installer_path.clone());
                        target.prefix = Some(prefix.clone());
                        let spec = rt.build_spec(&target)?;
                        crate::launcher::spawn(&spec)?;
                        Ok::<(), anyhow::Error>(())
                    });
                    match outcome {
                        Ok(()) => {
                            let _ = reply_sender.send(
                                json!({
                                    "reply": true,
                                    "id": rid,
                                    "data": { "ok": true, "prefix": prefix, "started": true },
                                })
                                .to_string(),
                            );
                            let _ = event_sender.send(
                                json!({ "event": "application.installation.started", "id": safe_id })
                                    .to_string(),
                            );
                        }
                        Err(e) => {
                            let _ = reply_sender.send(
                                json!({
                                    "reply": true,
                                    "id": rid,
                                    "data": { "ok": false, "error": e.to_string() },
                                })
                                .to_string(),
                            );
                            let _ = event_sender.send(
                                json!({
                                    "event": "application.installation.failed",
                                    "id": safe_id,
                                    "error": e.to_string(),
                                })
                                .to_string(),
                            );
                        }
                    }
                })
                .ok();
        }

        // Files opened from the file manager that need a decision.
        "windows:pending" => {
            let pending = crate::runtime::decisions::Decisions::load_pending();
            reply(state, id, json!({ "ok": true, "pending": pending }));
        }

        // Remember how this file should be handled next time.
        "windows:decide" => {
            let path = msg["path"].as_str().unwrap_or("").to_string();
            let runtime = msg["runtime"].as_str().unwrap_or("never").to_string();
            if path.is_empty() {
                reply(state, id, json!({ "ok": false, "error": "no path" }));
                return;
            }
            let mut decisions = crate::runtime::decisions::Decisions::load();
            decisions.remember(std::path::Path::new(&path), &runtime);
            crate::runtime::decisions::Decisions::clear_pending(std::path::Path::new(&path));
            send_event(state, json!({ "event": "windows.file_decided", "path": path, "runtime": runtime }));
            reply(state, id, json!({ "ok": true }));
        }

        // Install the .exe/.msi file associations so the file manager routes
        // them through the Runtime Manager instead of executing them.
        "windows:associations" => {
            let home = dirs::home_dir().unwrap_or_default();
            match crate::runtime::apps::install_file_associations(&home) {
                Ok(()) => reply(state, id, json!({ "ok": true })),
                Err(e) => reply(state, id, json!({ "ok": false, "error": e.to_string() })),
            }
        }

        // ---------------- ROM routes (existing) ----------------
        "roms:choose" => {
            let game_id = msg["id"].as_str().unwrap_or("").to_string();
            match state.library.borrow().get(&game_id) {
                Some(g) => {
                    let title = g.title.clone();
                    let exts = g.rom_exts.clone();
                    let folder = g
                        .rom_dir
                        .as_ref()
                        .map(|d| dirs::home_dir().unwrap_or_default().join(d));
                    if let Some(f) = &folder {
                        let _ = std::fs::create_dir_all(f);
                    }
                    let sender = state.sender.clone();
                    open_rom_chooser(&state.window, &title, &exts, folder, move |chosen| {
                        let payload = match chosen {
                            Some(path) => json!({
                                "event": "_rom_chosen",
                                "id": game_id,
                                "path": path,
                            }),
                            None => json!({ "event": "_rom_cancelled" }),
                        };
                        let _ = sender.send(payload.to_string());
                    });
                    reply(state, id, json!({ "ok": true }));
                }
                None => reply(state, id, json!({ "ok": false, "error": "game not found" })),
            }
        }

        // Pick a game in the real Files window: open it on the console folder,
        // hide the shell so Files is the visible/focused window, and watch for
        // the user opening a game file. Everything is done on a worker thread
        // because the shell window is hidden (and its JS timers throttled)
        // while the file manager is up.
        "roms:pick" => {
            let game_id = msg["id"].as_str().unwrap_or("").to_string();
            log::info!("roms:pick requested for {game_id}");
            let game = state.library.borrow().get(&game_id).cloned();
            match game {
                Some(g) => {
                    let dir = console_rom_dir(&g)
                        .unwrap_or_else(|| dirs::home_dir().unwrap_or_default());
                    let _ = std::fs::create_dir_all(&dir);
                    match integrations::apps::first_installed_file_manager() {
                        Some(program) => {
                            log::info!("roms:pick opening Files ({program}) on {}", dir.display());
                            let spec = Spec::new("Files", program)
                                .arg(dir.to_string_lossy().to_string());
                            match spawn_background(&spec) {
                                Ok(pid) => state
                                    .helpers
                                    .borrow_mut()
                                    .push(HelperWindow { pid, title: "Files".into() }),
                                Err(e) => log::error!("roms:pick could not start Files: {e}"),
                            }
                        }
                        None => log::error!("roms:pick: no file manager installed"),
                    }
                    // The shell stays open: Files opens as a window on top of
                    // it, like any other app, and the taskbar can switch back.
                    let sender = state.sender.clone();
                    let exts = g.rom_exts.clone();
                    std::thread::Builder::new()
                        .name("novashell-romwatch".into())
                        .spawn(move || {
                            // Prime, then poll for a game the user opens.
                            let _ = watch_opened_games(&exts, &game_id);
                            for _ in 0..150 {
                                std::thread::sleep(Duration::from_secs(2));
                                if let Some(found) =
                                    watch_opened_games(&exts, &game_id).pop()
                                {
                                    if let Some(path) = found["path"].as_str() {
                                        let msg = json!({
                                            "event": "_rom_opened",
                                            "id": game_id,
                                            "path": path,
                                            "_internal": true,
                                        })
                                        .to_string();
                                        let _ = sender.send(msg);
                                        return;
                                    }
                                }
                            }
                            let _ = sender.send(
                                json!({ "event": "_rom_pick_timeout", "_internal": true })
                                    .to_string(),
                            );
                        })
                        .ok();
                    reply(state, id, json!({ "ok": true }));
                }
                None => reply(state, id, json!({ "ok": false, "error": "game not found" })),
            }
        }

        // Report a game file the user just opened in the file manager, so the
        // shell can copy it into the console folder and boot it.
        "roms:watch" => {
            let game_id = msg["id"].as_str().unwrap_or("").to_string();
            let exts = state
                .library
                .borrow()
                .get(&game_id)
                .map(|g| g.rom_exts.clone())
                .unwrap_or_default();
            let opened = watch_opened_games(&exts, &game_id);
            reply(state, id, json!({ "ok": true, "opened": opened }));
        }

        // Look for game files matching this console's extensions in the usual
        // places, so a game the user opened/copied in the file manager shows
        // up as "Play" without hunting for it.
        "roms:scan" => {
            let game_id = msg["id"].as_str().unwrap_or("").to_string();
            let game = state.library.borrow().get(&game_id).cloned();
            match game {
                Some(g) => {
                    let exts: Vec<String> = g.rom_exts.iter().map(|e| e.to_lowercase()).collect();
                    let lib = crate::roms::RomLibrary::load();
                    let known = lib.games_for(&game_id);
                    let mut found: Vec<Value> = Vec::new();
                    for dir in game_search_dirs() {
                        let Ok(entries) = std::fs::read_dir(&dir) else {
                            continue;
                        };
                        for e in entries.flatten() {
                            let p = e.path();
                            if !p.is_file() {
                                continue;
                            }
                            let ext = p
                                .extension()
                                .map(|x| x.to_string_lossy().to_lowercase())
                                .unwrap_or_default();
                            if !exts.contains(&ext) {
                                continue;
                            }
                            let path = p.to_string_lossy().to_string();
                            let size = e.metadata().map(|m| m.len()).unwrap_or(0);
                            found.push(json!({
                                "name": p.file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_default(),
                                "path": path,
                                "size": size,
                                "known": known.contains(&path),
                            }));
                        }
                        if found.len() >= 200 {
                            break;
                        }
                    }
                    reply(state, id, json!({ "ok": true, "games": found }));
                }
                None => reply(state, id, json!({ "ok": false, "error": "game not found" })),
            }
        }

        "files:list" => {
            let path = msg["path"].as_str().unwrap_or("").to_string();
            let out = list_folder(&path);
            match out {
                Ok(v) => reply(state, id, json!({ "ok": true, "folder": v })),
                Err(e) => reply(state, id, json!({ "ok": false, "error": e.to_string() })),
            }
        }

        // Games previously booted with an emulator, newest first.
        "roms:list" => {
            let emulator = msg["id"].as_str().unwrap_or("");
            let lib = crate::roms::RomLibrary::load();
            reply(state, id, json!({ "ok": true, "games": lib.games_for(emulator) }));
        }

        // Remember a game so the next launch of this emulator can offer it.
        "roms:remember" => {
            let emulator = msg["id"].as_str().unwrap_or("").to_string();
            let path = msg["path"].as_str().unwrap_or("").to_string();
            if emulator.is_empty() || path.is_empty() {
                reply(state, id, json!({ "ok": false, "error": "missing id or path" }));
            } else {
                let mut lib = crate::roms::RomLibrary::load();
                lib.remember(&emulator, &path);
                reply(state, id, json!({ "ok": true }));
            }
        }

        // Open the desktop file manager on a folder (used by the ROM picker
        // so the user can copy game files in before selecting one).
        "files:reveal" => {
            let path = msg["path"].as_str().unwrap_or("");
            let dir = if path.is_empty() {
                std::path::PathBuf::from("/")
            } else {
                std::path::PathBuf::from(path)
            };
            let dir = if dir.is_dir() { dir } else {
                dir.parent().map(|p| p.to_path_buf()).unwrap_or(dir)
            };
            let program = integrations::apps::first_installed_file_manager()
                .unwrap_or_else(|| "nautilus".to_string());
            let spec = Spec::new("Files", program).arg(dir.to_string_lossy().to_string());
            match spawn_background(&spec) {
                Ok(pid) => {
                    state.helpers.borrow_mut().push(HelperWindow { pid, title: "Files".into() });
                    reply(state, id, json!({ "ok": true, "pid": pid }));
                }
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
    let base = state.ui_base.clone();
    // Keep the served icon map in step with the library.
    if let Ok(mut map) = state.ui_icons.lock() {
        map.clear();
        for g in lib.all_sorted() {
            if let Some(p) = &g.icon {
                map.insert(g.id.clone(), p.clone());
            }
            if let Some(p) = &g.artwork {
                map.insert(format!("art:{}", g.id), p.clone());
            }
        }
    }
    lib.all_sorted()
        .iter()
        .map(|g| game_json(g, &base))
        .collect()
}

fn boot_payload(state: &AppState) -> Value {
    let base = state.ui_base.clone();
    let games: Vec<Value> = state
        .library
        .borrow()
        .all_sorted()
        .iter()
        .map(|g| game_json(g, &base))
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

/// Serialize a game for the UI.
///
/// `icon_base` is the loopback HTTP server that fronts the UI: this WebKit
/// build blocks both `data:` and `file://` images on a custom-scheme page, so
/// icons are served over the same `http://127.0.0.1` origin as the document.
fn game_json(g: &Game, icon_base: &str) -> Value {
    let icon = g
        .icon
        .as_ref()
        .map(|_| format!("{icon_base}icon/{}", url_encode(&g.id)));
    let artwork = g
        .artwork
        .as_ref()
        .map(|_| format!("{icon_base}art/{}", url_encode(&g.id)));
    json!({
        "id": g.id,
        "title": g.title,
        "source": g.source,
        "platform": g.platform,
        "rom_exts": g.rom_exts,
        // The UI renders g.icon; keep favicon as an alias for older payloads.
        "icon": icon,
        "favicon": icon,
        "artwork": artwork,
        "last_played": g.last_played.filter(|t| *t > 0),
        "playtime": format_playtime(g.playtime_secs),
        "playtime_secs": g.playtime_secs,
        "favorite": g.favorite,
        "installed": g.installed,
    })
}

/// Percent-encode the characters that can appear in a library id, so it is
/// safe to use as a URL path segment.
fn url_encode(raw: &str) -> String {
    let mut out = String::with_capacity(raw.len());
    for b in raw.bytes() {
        match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                out.push(b as char)
            }
            _ => out.push_str(&format!("%{b:02X}")),
        }
    }
    out
}


/// Directory name for a prefix kind, for events and the UI.
fn kind_dir(kind: crate::runtime::prefixes::PrefixKind) -> &'static str {
    kind.dir_name()
}

/// Build a `WindowsApp` from a bridge message, validating every path first.
fn windows_app_from_msg(
    msg: &Value,
) -> anyhow::Result<crate::runtime::apps::WindowsApp> {
    use crate::runtime::apps::{AppKind, WindowsApp};
    use crate::runtime::prefixes::PrefixKind;

    let id = msg["id"].as_str().unwrap_or("").to_string();
    // Validated: the id becomes a directory name.
    let safe_id = crate::runtime::security::safe_segment(&id)?;
    let name = msg["name"].as_str().unwrap_or(&id).to_string();
    let kind = match msg["kind"].as_str().unwrap_or("windows-app") {
        "windows-game" | "game" => AppKind::WindowsGame,
        _ => AppKind::WindowsApp,
    };
    let runtime_id = msg["runtime"].as_str().unwrap_or("wine").to_string();
    let executable = msg["executable"].as_str().unwrap_or("").to_string();
    if executable.is_empty() {
        anyhow::bail!("no executable given");
    }
    let exe_path = std::path::PathBuf::from(&executable);
    if !exe_path.is_file() {
        anyhow::bail!("executable not found: {executable}");
    }
    // Windows executables must never be launched as root.
    crate::runtime::security::ensure_not_root()?;

    let prefix_kind = match kind {
        AppKind::WindowsGame => PrefixKind::Proton,
        AppKind::WindowsApp => PrefixKind::Wine,
    };
    let prefix = match msg["prefix"].as_str() {
        Some(p) if !p.is_empty() => std::path::PathBuf::from(p),
        // Wine prefixes must belong to the user running them.
        _ if kind == AppKind::WindowsApp => {
            crate::runtime::wine::WineRuntime::resolve_owned_prefix(&safe_id)?
        }
        _ => crate::runtime::prefixes::prefix_path(prefix_kind, &safe_id)?,
    };

    let mut app = WindowsApp::new(&safe_id, &name, kind, &runtime_id);
    app.prefix = prefix;
    app.executable = exe_path;
    app.working_directory = msg["workingDirectory"]
        .as_str()
        .map(std::path::PathBuf::from);
    app.arguments = msg
        .get("arguments")
        .and_then(|a| a.as_array())
        .map(|a| {
            a.iter()
                .filter_map(|v| v.as_str().map(String::from))
                .collect()
        })
        .unwrap_or_default();
    // Environment is filtered through the allowlist, never taken verbatim.
    if let Some(env) = msg.get("environment").and_then(|e| e.as_object()) {
        let mut map = std::collections::BTreeMap::new();
        for (k, v) in env {
            if let Some(s) = v.as_str() {
                map.insert(k.clone(), s.to_string());
            }
        }
        app.environment = crate::runtime::security::filter_env(&map);
    }
    Ok(app)
}

/// Add an installed Windows application to the existing Nova library so it
/// shows up on the home/library screens like any other title.
fn add_windows_app_to_library(
    state: &AppState,
    app: &crate::runtime::apps::WindowsApp,
) -> anyhow::Result<()> {
    let game = Game {
        id: app.id.clone(),
        title: app.name.clone(),
        source: app.kind.as_str().to_string(),
        launch: Launch::Program {
            program: app.executable.to_string_lossy().to_string(),
            args: app.arguments.clone(),
        },
        icon: app.icon.clone(),
        artwork: None,
        last_played: app.last_played,
        playtime_secs: app.play_time_secs,
        favorite: false,
        installed: true,
        platform: Some("Windows".to_string()),
        rom_exts: Vec::new(),
        rom_dir: None,
    };
    let mut lib = state.library.borrow_mut();
    lib.set(game)?;
    Ok(())
}

/// Spawn a process without waiting for it, returning its pid.
fn spawn_detached(spec: &Spec) -> Result<u32> {
    let mut child = crate::launcher::spawn(spec)?;
    let pid = child.id();
    std::thread::spawn(move || {
        let _ = child.wait();
    });
    Ok(pid)
}

/// Launch a stored Windows application through its runtime.
fn launch_windows_app(state: &AppState, app: &crate::runtime::apps::WindowsApp) -> Result<()> {
    crate::runtime::security::ensure_not_root()?;
    if !app.executable.is_file() {
        anyhow::bail!("executable is missing: {}", app.executable.display());
    }

    let runtimes = crate::runtime::registry();
    let mut target = crate::runtime::LaunchTarget::new(&app.id, &app.name)
        .with_executable(app.executable.clone())
        .with_game(matches!(app.kind, crate::runtime::apps::AppKind::WindowsGame));
    target.prefix = Some(app.prefix.clone());
    target.args = app.arguments.clone();
    target.env = app.environment.clone();
    target.working_dir = app.working_directory.clone();
    target.runtime_override = Some(app.runtime.clone());

    let runtime = crate::runtime::select(&runtimes, &target)
        .ok_or_else(|| anyhow!("no runtime available for {}", app.runtime))?;
    let checks = runtime.validate(&target);
    if let Some(bad) = checks.iter().find(|c| !c.ok) {
        log::warn!("pre-flight failed: {} ({:?})", bad.label, bad.detail);
    }
    let spec = runtime.build_spec(&target)?;
    let log = crate::runtime::logs::LaunchLog::create(&app.id)?;
    log.write_launch(
        &spec.program,
        &spec.args,
        &spec.env.iter().map(|(k, _)| k.as_str()).collect::<Vec<_>>(),
    );

    let pid = spawn_detached(&spec)?;
    let mut info = crate::runtime::procs::describe(
        &spec,
        &app.id,
        &app.runtime,
        app.prefix.to_str(),
    );
    info.pid = pid;
    info.state = crate::runtime::procs::ProcessState::Running;
    state.procs.insert(info);
    send_event(state, json!({ "event": "application.started", "id": app.id, "pid": pid }));
    Ok(())
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

/// List a directory for the UI's ROM/file browser. Defaults to the user's
/// home dir when `path` is empty or missing.
fn list_folder(path: &str) -> Result<Value> {
    // "~/PS2" and a bare console folder name both resolve under the home dir,
    // so the ROM picker can just ask for the tile's `rom_dir`.
    let raw = path.trim();
    let expanded = if raw == "~" || raw.starts_with("~/") {
        util::expand_tilde(raw)
    } else if !raw.is_empty() && !raw.starts_with('/') {
        dirs::home_dir().unwrap_or_default().join(raw)
    } else {
        std::path::PathBuf::from(raw)
    };
    let start = if expanded.as_os_str().is_empty() {
        dirs::home_dir().unwrap_or_else(|| std::path::PathBuf::from("/"))
    } else {
        expanded
    };
    // A console folder that does not exist yet is created on demand: it is the
    // place the user is told to drop game files.
    if !start.exists() && start.parent().map(|p| p.is_dir()).unwrap_or(false) {
        let _ = std::fs::create_dir_all(&start);
    }
    let start = start.canonicalize().map_err(|e| anyhow!("bad folder: {e}"))?;
    if !start.is_dir() {
        return Err(anyhow!("not a directory"));
    }
    let parent = start
        .parent()
        .map(|p| p.to_string_lossy().to_string())
        .unwrap_or_default();
    let read = std::fs::read_dir(&start).map_err(|e| anyhow!("cannot read: {e}"))?;

    let mut dirs: Vec<Value> = Vec::new();
    let mut files: Vec<Value> = Vec::new();
    for entry in read.flatten() {
        let name = entry.file_name().to_string_lossy().to_string();
        if name.starts_with('.') {
            continue; // skip hidden
        }
        let p = entry.path();
        let is_dir = p.is_dir();
        if !is_dir && !p.is_file() {
            continue;
        }
        let item = json!({
            "name": name,
            "path": p.to_string_lossy().to_string(),
            "is_dir": is_dir,
        });
        if is_dir {
            dirs.push(item);
        } else {
            files.push(item);
        }
    }
    dirs.sort_by(|a, b| a["name"].as_str().cmp(&b["name"].as_str()));
    files.sort_by(|a, b| a["name"].as_str().cmp(&b["name"].as_str()));
    dirs.extend(files);

    Ok(json!({
        "path": start.to_string_lossy().to_string(),
        "parent": parent,
        "entries": dirs,
    }))
}

fn launch_by_id(state: &AppState, game_id: &str, rom: Option<&str>, id: &str) {
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
    let mut spec = match game.launch.to_spec(&game.title, steam_bin.as_deref()) {
        Some(s) => s,
        None => {
            reply(state, id, json!({ "ok": false, "error": "not launchable" }));
            return;
        }
    };
    let flags = console_polish(&spec.program);
    if !flags.is_empty() {
        let mut all = flags;
        all.extend(spec.args.clone());
        spec.args = all;
    }
    if let Some(rom) = rom {
        if !rom.is_empty() {
            let src = std::path::Path::new(rom);
            if !src.exists() {
                reply(state, id, json!({ "ok": false, "error": "ROM not found" }));
                return;
            }
            // Keep every game inside its console folder: a file picked
            // elsewhere (Desktop, Downloads, a Windows drive) is copied in
            // first, so the library always points at ~/PS2, ~/Wii, ...
            let rom_path = match console_rom_dir(&game) {
                Some(dir) if !src.starts_with(&dir) => {
                    let _ = std::fs::create_dir_all(&dir);
                    match copy_into(src, &dir) {
                        Ok(dest) => dest,
                        Err(e) => {
                            log::warn!("could not copy game into console folder: {e}");
                            src.to_path_buf()
                        }
                    }
                }
                _ => src.to_path_buf(),
            };
            let rom_arg = rom_path.to_string_lossy().to_string();
            spec = spec.arg(&rom_arg);
            // Remember it so this emulator can list its games next time.
            let mut lib = crate::roms::RomLibrary::load();
            lib.remember(game_id, &rom_arg);
            log::info!("{} (rom: {rom_arg})", game.title);
        }
    }
    match launch_spec(state, &spec, Some(game_id)) {
        Ok(()) => {
            let _ = state.library.borrow_mut().record_launch(game_id);
            reply(state, id, json!({ "ok": true }));
        }
        Err(e) => reply(state, id, json!({ "ok": false, "error": e.to_string() })),
    }
}

/// Spawn `apt-get install` for the curated package list. Runs on a worker
/// thread; the UI is told via `_install_done` when it finishes.
fn run_apt_install(pkgs: &[String]) -> Result<()> {
    log::info!("installing via apt: {}", pkgs.join(" "));
    for pkg in pkgs {
        if crate::integrations::apps::needs_repo(pkg) {
            crate::integrations::apps::ensure_third_party_repo(pkg).map_err(|e| anyhow!(e))?;
        }
    }
    let status = std::process::Command::new("apt-get")
        .args(["install", "-y", "--no-install-recommends"])
        .args(pkgs)
        .env("DEBIAN_FRONTEND", "noninteractive")
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .status();
    match status {
        Ok(s) if s.success() => Ok(()),
        Ok(s) => Err(anyhow!("apt-get exited with {}", s.code().unwrap_or(-1))),
        Err(e) => Err(anyhow!("could not run apt-get: {e}")),
    }
}

/// Spawn a process, hide the shell while it's fullscreen, and report back.
/// Open the native GTK file chooser for a game file. `on_done` receives the
/// chosen path, or `None` when the dialog is cancelled. The dialog is kept
/// alive in a thread-local until it closes, because dropping the last
/// reference would close it immediately.
#[allow(deprecated)]
fn open_rom_chooser<F>(
    parent: &gtk::Window,
    title: &str,
    exts: &[String],
    folder: Option<std::path::PathBuf>,
    on_done: F,
) where
    F: Fn(Option<String>) + 'static,
{
    use gtk::prelude::*;

    // A plain GTK file-chooser dialog: `FileChooserNative` would need an
    // xdg-desktop-portal, which does not exist in this session, and fails
    // silently there. This renders in-process and still looks/behaves like the
    // system "open file" dialog.
    thread_local! {
        static OPEN_CHOOSERS: RefCell<Vec<gtk::FileChooserDialog>> =
            const { RefCell::new(Vec::new()) };
    }

    let dialog = gtk::FileChooserDialog::builder()
        .title(format!("Select a game file for {title}"))
        .transient_for(parent)
        .modal(true)
        .action(gtk::FileChooserAction::Open)
        .build();

    if let Some(dir) = folder {
        let file = gtk::gio::File::for_path(dir);
        let _ = dialog.set_current_folder(Some(&file));
    }
    if !exts.is_empty() {
        let filter = gtk::FileFilter::new();
        filter.set_name(Some("Game files"));
        for ext in exts {
            filter.add_pattern(&format!("*.{ext}"));
        }
        let all = gtk::FileFilter::new();
        all.set_name(Some("All files"));
        all.add_pattern("*");
        dialog.add_filter(&filter);
        dialog.add_filter(&all);
        dialog.set_filter(&filter);
    }

    let cb = on_done;
    dialog.connect_response(move |d, response| {
        let chosen = if response == gtk::ResponseType::Accept {
            d.file()
                .and_then(|f| f.path())
                .map(|p| p.to_string_lossy().to_string())
        } else {
            None
        };
        OPEN_CHOOSERS.with(|c| {
            c.borrow_mut().retain(|x| x != d);
        });
        d.destroy();
        cb(chosen);
    });

    OPEN_CHOOSERS.with(|c| c.borrow_mut().push(dialog.clone()));
    log::info!("opening game file chooser for {title}");
    dialog.show();
}

/// Console-style launch flags, keyed by binary name. Emulators get forced
/// fullscreen so they take the whole screen like a console app; browsers get
/// a dedicated fullscreen window on a fresh profile so launching them never
/// just drops a tab into an already-running instance.
fn console_polish(program: &str) -> Vec<String> {
    use std::path::Path;
    let bin = Path::new(program)
        .file_name()
        .map(|n| n.to_string_lossy().to_string())
        .unwrap_or_else(|| program.to_string());
    let fs_profile = |name: &str| {
        let dir = crate::util::data_dir()
            .join("browsers")
            .join(format!("{name}-profile"))
            .to_string_lossy()
            .to_string();
        vec![
            // Kiosk = no tab strip, no address bar, no chrome: a dedicated
            // full-screen "app" window, the way a console opens a title.
            "--kiosk".into(),
            "--start-fullscreen".into(),
            "--no-first-run".into(),
            "--no-default-browser-check".into(),
            // A private profile guarantees its own window instead of opening
            // a tab inside an already-running browser instance.
            format!("--user-data-dir={dir}"),
        ]
    };
    match bin.as_str() {
        "brave-browser" | "brave-browser-stable" | "brave" => fs_profile("brave"),
        "google-chrome" | "google-chrome-stable" | "chromium" | "chromium-browser" => {
            fs_profile("chrome")
        }
        "retroarch" => vec!["-f".into()],
        // PCSX2 and Dolphin reject unknown command-line switches outright
        // ("unknown parameter -fullscreen", exit 1) and both already start
        // fullscreen from their own settings, so they get no extra flags.
        "dolphin-emu" | "dolphin" | "pcsx2" | "pcsx2-qt" => vec![],
        _ => vec![],
    }
}

/// Play a game file without the GUI: find the emulator for its extension, keep
/// a copy in the console's folder, remember it, and boot it. This is what the
/// desktop file manager runs when a disc image is opened.
fn play_game_file(path: &str) -> Result<()> {
    let src = std::path::Path::new(path);
    if !src.is_file() {
        anyhow::bail!("not a game file: {path}");
    }
    let ext = src
        .extension()
        .map(|e| e.to_string_lossy().to_lowercase())
        .unwrap_or_default();

    // Pick the console. The folder the file sits in is the strongest signal:
    // a game in ~/PS2 is a PS2 game even though ".iso" is also a PSX/PS3/Wii
    // extension, and choosing by extension alone sent PS2 games to RPCS3.
    let folder = src
        .parent()
        .and_then(|p| p.file_name())
        .map(|n| n.to_string_lossy().to_string())
        .unwrap_or_default();
    let by_folder = |want: &str| {
        integrations::apps::EMULATORS
            .iter()
            .find(|d| d.rom_dir.eq_ignore_ascii_case(want))
    };
    let matching = integrations::apps::EMULATORS
        .iter()
        .filter(|d| d.exts.iter().any(|e| e.eq_ignore_ascii_case(&ext)))
        .collect::<Vec<_>>();
    let chosen = by_folder(&folder)
        .filter(|d| d.bins.iter().any(|b| integrations::apps::find_bin(b).is_some()))
        .or_else(|| {
            matching
                .iter()
                .find(|d| d.bins.iter().any(|b| integrations::apps::find_bin(b).is_some()))
                .copied()
        })
        .or_else(|| matching.first().copied())
        .ok_or_else(|| anyhow!("no emulator handles '.{ext}' files"))?;
    let def = chosen;
    let bin = def
        .bins
        .iter()
        .find_map(|b| integrations::apps::find_bin(b))
        .ok_or_else(|| anyhow!("{} is not installed", def.pretty))?;

    // Keep the master copy in the console folder and play that.
    let dir = dirs::home_dir()
        .unwrap_or_default()
        .join(def.rom_dir.trim_start_matches("~/"));
    let _ = std::fs::create_dir_all(&dir);
    let game = if src.starts_with(&dir) {
        src.to_path_buf()
    } else {
        match copy_into(src, &dir) {
            Ok(d) => d,
            Err(e) => {
                eprintln!("novashell: could not copy into {}: {e}", dir.display());
                src.to_path_buf()
            }
        }
    };

    let mut lib = crate::roms::RomLibrary::load();
    let bin_name = std::path::Path::new(&bin)
        .file_name()
        .map(|n| n.to_string_lossy().to_string())
        .unwrap_or_else(|| bin.clone());
    lib.remember(&format!("emulator-{bin_name}"), &game.to_string_lossy());

    let mut spec = Spec::new(def.pretty, bin).arg(game.to_string_lossy().to_string());
    let flags = console_polish(&spec.program);
    if !flags.is_empty() {
        let mut all = flags;
        all.extend(spec.args.clone());
        spec.args = all;
    }
    let mut child = crate::launcher::spawn(&spec)?;
    eprintln!("novashell: playing {} with {}", game.display(), def.pretty);
    let _ = child.wait();
    Ok(())
}

/// The console's own game folder (e.g. `~/PS2`) for an emulator tile.
fn console_rom_dir(game: &Game) -> Option<std::path::PathBuf> {
    let dir = game.rom_dir.as_ref()?;
    Some(dirs::home_dir().unwrap_or_default().join(dir))
}

/// Copy `src` into `dir`, never overwriting: "Game.iso", "Game (1).iso", ...
fn copy_into(src: &std::path::Path, dir: &std::path::Path) -> Result<std::path::PathBuf> {
    let name = src
        .file_name()
        .map(|n| n.to_os_string())
        .ok_or_else(|| anyhow!("game file has no name"))?;
    let size = std::fs::metadata(src).map(|m| m.len()).unwrap_or(0);

    // Already present with the same size: reuse it instead of making another
    // multi-gigabyte duplicate.
    let existing = dir.join(&name);
    if existing.is_file() && std::fs::metadata(&existing).map(|m| m.len()).ok() == Some(size) {
        log::info!("game already in {}: {}", dir.display(), name.to_string_lossy());
        return Ok(existing);
    }

    let mut dest = dir.join(&name);
    if dest.exists() {
        let stem = src
            .file_stem()
            .map(|s| s.to_string_lossy().to_string())
            .unwrap_or_else(|| "game".into());
        let ext = src
            .extension()
            .map(|e| format!(".{}", e.to_string_lossy()))
            .unwrap_or_default();
        for n in 1..1000 {
            dest = dir.join(format!("{stem} ({n}){ext}"));
            if !dest.exists() {
                break;
            }
        }
    }
    log::info!("copying game into {}", dir.display());
    // Copy to a .part file and rename only when complete, so an interrupted copy
    // can never be left behind looking like a playable game.
    let part = dest.with_file_name(format!(
        "{}.part",
        dest.file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_default()
    ));
    let _ = std::fs::remove_file(&part);
    if let Err(e) = std::fs::copy(src, &part) {
        let _ = std::fs::remove_file(&part);
        return Err(anyhow!("copy failed: {e}"));
    }
    std::fs::rename(&part, &dest)?;
    Ok(dest)
}

/// Folders searched for game files when the user picks one in the file
/// manager: the console folder plus the usual download spots, including the
/// Windows drives mounted into WSL.
fn game_search_dirs() -> Vec<std::path::PathBuf> {
    let home = dirs::home_dir().unwrap_or_default();
    let mut dirs = vec![
        home.join("PS2"),
        home.join("PSX"),
        home.join("PS3"),
        home.join("Xbox"),
        home.join("Wii"),
        home.join("Nintendo"),
        home.join("Desktop"),
        home.join("Downloads"),
        home.join("Documents"),
        home.join("Videos"),
    ];
    for extra in [
        "/mnt/c/Users/dell/Desktop",
        "/mnt/c/Users/dell/Downloads",
        "/mnt/c/Users/dell/Videos",
        "/mnt/d/Games",
        "/mnt/d/Downloads",
        "/mnt/e",
    ] {
        let p = std::path::PathBuf::from(extra);
        if p.is_dir() {
            dirs.push(p);
        }
    }
    dirs
}

/// Watch the game folders and report a game file the user has just *opened* in
/// the file manager. Nautilus cannot hand a selection back to us, so we notice
/// by access time: selecting/opening a file reads it, which updates its atime
/// on the filesystems we care about. A first call primes the snapshot and
/// arms the watch; later calls report anything opened since.
fn watch_opened_games(exts: &[String], game_id: &str) -> Vec<Value> {
    let exts: Vec<String> = exts.iter().map(|e| e.to_lowercase()).collect();
    let mut snapshot: std::collections::HashMap<String, std::time::SystemTime> =
        Default::default();
    let mut all: Vec<Value> = Vec::new();

    for dir in game_search_dirs() {
        let Ok(entries) = std::fs::read_dir(&dir) else {
            continue;
        };
        for e in entries.flatten() {
            let p = e.path();
            if !p.is_file() {
                continue;
            }
            let ext = p
                .extension()
                .map(|x| x.to_string_lossy().to_lowercase())
                .unwrap_or_default();
            if !exts.contains(&ext) {
                continue;
            }
            let path = p.to_string_lossy().to_string();
            let Ok(meta) = e.metadata() else { continue };
            let accessed = meta.accessed().unwrap_or(std::time::SystemTime::UNIX_EPOCH);
            snapshot.insert(path.clone(), accessed);
            all.push(json!({
                "name": p.file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_default(),
                "path": path,
                "size": meta.len(),
            }));
        }
    }

    // Compare against the previous snapshot for this console.
    let store = rom_watch_store();
    let mut guard = store.lock().unwrap_or_else(|e| e.into_inner());
    match guard.get(game_id) {
        None => {
            // First call: remember the current state, report nothing yet.
            guard.insert(game_id.to_string(), snapshot);
            Vec::new()
        }
        Some(previous) => {
            let mut result = Vec::new();
            for (path, atime) in &snapshot {
                match previous.get(path) {
                    Some(before) if before >= atime => {}
                    _ => {
                        if let Some(item) = all.iter().find(|v| v["path"] == path.as_str()) {
                            result.push(item.clone());
                        }
                    }
                }
            }
            guard.insert(game_id.to_string(), snapshot);
            result
        }
    }
}

/// Process-wide map of console id -> last seen file access times.
fn rom_watch_store() -> &'static std::sync::Mutex<
    std::collections::HashMap<String, std::collections::HashMap<String, std::time::SystemTime>>,
> {
    use std::sync::OnceLock;
    static STORE: OnceLock<
        std::sync::Mutex<
            std::collections::HashMap<String, std::collections::HashMap<String, std::time::SystemTime>>,
        >,
    > = OnceLock::new();
    STORE.get_or_init(|| std::sync::Mutex::new(Default::default()))
}

/// Launch a helper app (e.g. the file manager) *beside* the shell: the shell
/// window stays visible, no "now playing" state, and quitting the shell does
/// not take the helper down (it is the user's own file manager window).
fn spawn_background(spec: &Spec) -> Result<u32> {
    log::info!("opening helper: {} ({})", spec.name, spec.program);
    let mut child = crate::launcher::spawn(spec)?;
    let pid = child.id();
    // A freshly mapped window can land behind the shell under WSLg, which
    // looks like "nothing opened". Raise it once it is on screen.
    let program = std::path::Path::new(&spec.program)
        .file_name()
        .map(|n| n.to_string_lossy().to_string())
        .unwrap_or_default();
    std::thread::spawn(move || {
        std::thread::sleep(Duration::from_millis(1800));
        let _ = std::process::Command::new("xdotool")
            .args(["search", "--class", &program, "windowactivate", "--sync"])
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .status();
        let _ = child.wait();
    });
    Ok(pid)
}

fn launch_spec(state: &AppState, spec: &Spec, library_id: Option<&str>) -> Result<()> {
    log::info!("launching: {}", spec.name);
    let child = crate::launcher::spawn(spec)?;
    let title = spec.name.clone();
    let pid = child.id();
    // Register with the Runtime Manager's process table so stop/kill and the
    // taskbar can see it alongside Wine/Proton sessions.
    {
        let mut info = crate::runtime::procs::describe(
            spec,
            library_id.unwrap_or_default(),
            "shell",
            None,
        );
        info.pid = pid;
        info.state = crate::runtime::procs::ProcessState::Running;
        state.procs.insert(info);
    }
    {
        let mut running = state.running.borrow_mut();
        *running = Some(RunningSession {
            id: library_id.unwrap_or_default().to_string(),
            title: title.clone(),
            pid,
        });
    }
    send_event(state, json!({ "event": "game_start", "title": title }));

    let sender = state.sender.clone();
    // Do NOT hide the shell: launched apps open as normal windows on top of
    // NovaShell (the way Chrome opens over a desktop), and the taskbar in the
    // shell is used to switch between them or close them.
    let hide_mode = false;

    // Wait for the child. If it dies within the first ~1.4s (e.g. a Qt app
    // that cannot find its backend), tell the UI it failed so the shell comes
    // straight back instead of leaving a blank fullscreen.
    std::thread::spawn(move || {
        let start = Instant::now();
        let mut child = child;
        std::thread::sleep(Duration::from_millis(1400));
        let took = start.elapsed().as_secs();
        let early_exit = if let Ok(Some(status)) = child.try_wait() {
            Some(status.code())
        } else {
            None
        };
        if let Some(code) = early_exit {
            if hide_mode {
                let _ = sender.send(
                    json!({ "event": "_launch_failed", "_internal": true, "title": title, "code": code })
                        .to_string(),
                );
            }
        } else if hide_mode {
            let _ = sender.send(
                json!({ "event": "_launch_hide", "_internal": true, "title": title }).to_string(),
            );
        }
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
        let _ = took;
    });
    Ok(())
}

// ---------------------------------------------------------------------------
// Rust -> UI events
// ---------------------------------------------------------------------------

fn handle_core_event(state: &AppState, v: &Value) {
    // Internal side effects first.
    if v.get("_internal").and_then(|x| x.as_bool()).unwrap_or(false)
        && v["event"].as_str() == Some("_rom_opened")
    {
        // The user opened a game in the file manager: copy it into the console
        // folder, remember it and boot the emulator.
        let gid = v["id"].as_str().unwrap_or_default().to_string();
        let path = v["path"].as_str().unwrap_or_default().to_string();
        state.window.set_visible(true);
        launch_by_id(state, &gid, Some(&path), "");
    }
    if v.get("_internal").and_then(|x| x.as_bool()).unwrap_or(false)
        && v["event"].as_str() == Some("_rom_pick_timeout")
    {
        // Nothing was chosen in the file manager: bring the shell back.
        show_shell(state);
    }
    if v.get("_internal").and_then(|x| x.as_bool()).unwrap_or(false)
        && v["event"].as_str() == Some("_launch_hide")
    {
        state.window.set_visible(false);
    }
    if v.get("_internal").and_then(|x| x.as_bool()).unwrap_or(false)
        && v["event"].as_str() == Some("_launch_failed")
    {
        show_shell(state);
        log::warn!("{} exited on start (code {:?})", v["title"].as_str().unwrap_or("?"), v["code"].as_i64());
    }
    if v.get("_internal").and_then(|x| x.as_bool()).unwrap_or(false)
        && v["event"].as_str() == Some("_install_done")
        && v["ok"].as_bool().unwrap_or(false)
    {
        log::info!("install finished; refreshing library");
        refresh_library(state);
    }
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
            show_shell(state);
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
