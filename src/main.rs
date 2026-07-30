#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

//! FreeViewer - open source remote desktop.
//!
//! One binary does both jobs, like TeamViewer: it registers this machine at the
//! relay (so others can connect to your ID) and it can connect to another ID.
//!
//! Relay can be overridden with the FV_RELAY environment variable,
//! the session password with FV_PASSWORD.

mod autostart;
mod capture;
mod clip;
mod crypto;
mod encoder;
mod h264;
mod hostside;
mod ident;
mod input;
mod net;
mod p2p;
mod partners;
mod proto;
mod selftest;
mod service;
mod shared;
mod tray;
mod update;
mod viewer;
mod vinput;
mod xfer;

use std::sync::atomic::Ordering;
use std::sync::{Arc, OnceLock};
use std::time::Duration;

use proto::Msg;
use shared::Shared;

const DEFAULT_RELAY: &str = "wss://jarvis.fleitec.com/fv/ws";

static RT: OnceLock<tokio::runtime::Runtime> = OnceLock::new();

/// Diagnostic line into <config>/debug.log. The release build is a GUI binary,
/// so println! is invisible unless somebody redirected stdout.
pub fn dbg_line(s: &str) {
    use std::io::Write;
    println!("{}", s);
    let path = ident::config_dir().join("debug.log");
    if let Ok(mut f) = std::fs::OpenOptions::new().create(true).append(true).open(path) {
        let _ = writeln!(f, "{}", s);
    }
}

fn rt() -> &'static tokio::runtime::Runtime {
    RT.get().expect("tokio runtime")
}

/// Capture and input work in physical pixels, so the process must not be
/// scaled by Windows.
#[cfg(windows)]
fn make_dpi_aware() {
    use windows::Win32::UI::HiDpi::{
        SetProcessDpiAwarenessContext, DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2,
    };
    unsafe {
        let _ = SetProcessDpiAwarenessContext(DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2);
    }
}
#[cfg(not(windows))]
fn make_dpi_aware() {}

fn main() -> eframe::Result<()> {
    make_dpi_aware();
    let _ = rustls::crypto::ring::default_provider().install_default();

    let relay = std::env::var("FV_RELAY").unwrap_or_else(|_| DEFAULT_RELAY.to_string());
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
        .expect("tokio runtime");
    let _ = RT.set(runtime);

    let secret = ident::load_or_create_secret();
    let password = std::env::var("FV_PASSWORD")
        .ok()
        .or_else(ident::fixed_password)
        .unwrap_or_else(ident::random_password);
    let shared = Arc::new(Shared::new(relay, password));

    // print the version:  freeviewer --version
    if std::env::args().any(|a| a == "--version" || a == "-V") {
        println!("FreeViewer {}", update::VERSION);
        return Ok(());
    }

    // ---- the Windows service -------------------------------------------
    // "freeviewer --service" is what the service control manager starts.
    if std::env::args().any(|a| a == "--service") {
        if let Err(e) = service::run() {
            service::log(&format!("Dienst konnte nicht starten: {}", e));
        }
        return Ok(());
    }
    // install / remove, asking for admin rights when we do not have them yet
    for (flag, install) in [("--install-service", true), ("--uninstall-service", false)] {
        if std::env::args().any(|a| a == flag) {
            if !service::is_elevated() {
                println!("Administrator-Rechte anfordern...");
                match service::elevate(flag) {
                    Ok(()) => println!("Windows fragt jetzt nach der Bestaetigung."),
                    Err(e) => println!("FAIL: {}", e),
                }
                return Ok(());
            }
            let r = if install {
                service::install()
            } else {
                service::uninstall()
            };
            match r {
                Ok(()) => println!(
                    "{}",
                    if install {
                        "Dienst installiert und gestartet."
                    } else {
                        "Dienst gestoppt und entfernt."
                    }
                ),
                Err(e) => println!("FAIL: {}", e),
            }
            return Ok(());
        }
    }
    // state of service and autostart:  freeviewer --status
    if std::env::args().any(|a| a == "--status") {
        println!("Version:    {}", update::VERSION);
        println!("Config:     {}", ident::config_dir().display());
        println!("Autostart:  {}", match autostart::current() {
            Some(c) => c,
            None => "aus".to_string(),
        });
        println!("Dienst:     installiert={} laeuft={}", service::installed(), service::running());
        match service::published() {
            Some(p) => println!("Agent:      ID {} Passwort {} (vor {} s, Desktop {})",
                p.id, p.password,
                std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH)
                    .map(|d| d.as_secs()).unwrap_or(0).saturating_sub(p.at),
                p.desktop),
            None => println!("Agent:      nichts veroeffentlicht"),
        }
        return Ok(());
    }
    // switch autostart from a script:  freeviewer --autostart on|off
    if let Some(pos) = std::env::args().position(|a| a == "--autostart") {
        let arg: Vec<String> = std::env::args().collect();
        match arg.get(pos + 1).map(|s| s.as_str()) {
            Some("on") | Some("off") => {
                let on = arg[pos + 1] == "on";
                match autostart::set(on) {
                    Ok(()) => println!("Autostart {}", if on { "an" } else { "aus" }),
                    Err(e) => println!("FAIL: {}", e),
                }
            }
            _ => {}
        }
        println!("Autostart jetzt: {:?} (zeigt hierher: {})", autostart::current(), autostart::points_here());
        return Ok(());
    }

    // update check/installation from the command line:  freeviewer --update
    if std::env::args().any(|a| a == "--update") {
        update::cleanup();
        println!("lokal: {}", update::VERSION);
        match update::check() {
            Ok(rel) => {
                println!(
                    "relay: {} ({} Bytes, {}...)",
                    rel.version,
                    rel.size,
                    &rel.sha256[..12]
                );
                if update::newer(&rel.version, update::VERSION) {
                    println!("installiere...");
                    match update::download(&rel) {
                        Ok(tmp) => {
                            println!("geladen + Pruefsumme ok: {}", tmp.display());
                            if std::env::args().any(|a| a == "--dry") {
                                let _ = std::fs::remove_file(&tmp);
                                println!("DRY: nicht ersetzt");
                                return Ok(());
                            }
                            match update::swap(&tmp) {
                                Ok(p) => println!("UPDATE OK -> {}", p.display()),
                                Err(e) => println!("FAIL: {}", e),
                            }
                        }
                        Err(e) => println!("FAIL: {}", e),
                    }
                } else {
                    println!("schon aktuell");
                }
            }
            Err(e) => println!("FAIL: {}", e),
        }
        return Ok(());
    }

    // which screens can this machine share?   freeviewer --monitors
    if std::env::args().any(|a| a == "--monitors") {
        for (i, m) in hostside::monitor_list(true).iter().enumerate() {
            println!(
                "{}: {} {}x{}{}",
                i,
                m.name,
                m.w,
                m.h,
                if m.primary { "  (primaer)" } else { "" }
            );
        }
        return Ok(());
    }

    // capture self test:  freeviewer --captest   (writes <config>/captest.txt)
    if std::env::args().any(|a| a == "--captest") {
        let report = hostside::capture_selftest(20);
        let path = ident::config_dir().join("captest.txt");
        let _ = std::fs::write(&path, &report);
        println!("{}", report);
        return Ok(());
    }

    // GPU vs CPU scaling:  freeviewer --gputest [rounds]
    if std::env::args().any(|a| a == "--gputest") {
        let rounds: u32 = std::env::args()
            .skip_while(|a| a != "--gputest")
            .nth(1)
            .and_then(|s| s.parse().ok())
            .unwrap_or(60);
        let report = hostside::gpu_selftest(rounds);
        let path = ident::config_dir().join("gputest.txt");
        let _ = std::fs::write(&path, &report);
        println!("{}", report);
        return Ok(());
    }

    // H.264 codec self test:  freeviewer --h264test [rounds]
    if std::env::args().any(|a| a == "--h264test") {
        let rounds: u32 = std::env::args()
            .skip_while(|a| a != "--h264test")
            .nth(1)
            .and_then(|s| s.parse().ok())
            .unwrap_or(60);
        let report = h264::selftest(rounds);
        let path = ident::config_dir().join("h264test.txt");
        let _ = std::fs::write(&path, &report);
        println!("{}", report);
        return Ok(());
    }

    // live H.264 vs JPEG on the real screen:  freeviewer --videotest [rounds]
    if std::env::args().any(|a| a == "--videotest") {
        let rounds: u32 = std::env::args()
            .skip_while(|a| a != "--videotest")
            .nth(1)
            .and_then(|s| s.parse().ok())
            .unwrap_or(120);
        let report = hostside::video_selftest(rounds);
        let path = ident::config_dir().join("videotest.txt");
        let _ = std::fs::write(&path, &report);
        println!("{}", report);
        return Ok(());
    }

    // can we punch a hole?  freeviewer --p2ptest
    if std::env::args().any(|a| a == "--p2ptest") {
        let report = p2p::selftest();
        let path = ident::config_dir().join("p2ptest.txt");
        let _ = std::fs::write(&path, &report);
        println!("{}", report);
        return Ok(());
    }

    // delta/full-frame benchmark:  freeviewer --deltatest [rounds]
    if std::env::args().any(|a| a == "--deltatest") {
        let rounds: u32 = std::env::args()
            .skip_while(|a| a != "--deltatest")
            .nth(1)
            .and_then(|s| s.parse().ok())
            .unwrap_or(30);
        let report = hostside::delta_selftest(rounds);
        let path = ident::config_dir().join("deltatest.txt");
        let _ = std::fs::write(&path, &report);
        println!("{}", report);
        return Ok(());
    }
    // in --connect test mode we only act as a viewer (otherwise this process
    // would register the same machine identity and kick the real host offline)
    let viewer_only = std::env::args().any(|a| a == "--connect" || a == "--inputtest");
    // Started by the service? Then we ARE the host of this machine.
    let is_agent = std::env::args().any(|a| a == "--agent");
    // Two processes with the same identity would kick each other off the
    // relay, so the GUI keeps its hands off while the service does the job.
    let service_owns_host = !is_agent && service::running();
    if !viewer_only && !service_owns_host {
        let host_shared = shared.clone();
        let host_secret = secret.clone();
        rt().spawn(async move {
            hostside::run_host(host_shared, host_secret).await;
        });
    }
    if is_agent {
        // let the user's GUI show ID and password of this host
        service::publish_loop(shared.clone());
        service::log(&format!("Agent laeuft: {}", service::desktop_report()));
        // follow the input desktop (lock screen, UAC prompt, ...)
        service::watch_desktop();
    }
    if service_owns_host {
        shared.set_host_status("Der Dienst betreibt den Host - auch am Anmeldebildschirm");
        let sh = shared.clone();
        std::thread::spawn(move || loop {
            if let Some(p) = service::published() {
                *sh.my_id.lock().unwrap() = p.id;
                *sh.password.lock().unwrap() = p.password;
            }
            std::thread::sleep(Duration::from_secs(2));
        });
    }

    // headless host mode (no window) - handy for servers and for testing
    if std::env::args().any(|a| a == "--headless") {
        println!("FreeViewer headless host, relay = {}", shared.relay_url);
        println!("password = {}", shared.password.lock().unwrap());
        loop {
            std::thread::sleep(Duration::from_secs(2));
            let id = shared.my_id.lock().unwrap().clone();
            if !id.is_empty() {
                println!(
                    "ID {} | {} | {}",
                    id,
                    shared.host_status.lock().unwrap(),
                    shared.host_peer.lock().unwrap()
                );
            }
        }
    }

    // headless viewer mode for testing:
    //   freeviewer --connect <id> <password> [frames] [--game]
    let argv: Vec<String> = std::env::args().collect();
    if let Some(pos) = argv.iter().position(|a| a == "--connect") {
        let id = argv.get(pos + 1).cloned().unwrap_or_default();
        let pw = argv.get(pos + 2).cloned().unwrap_or_default();
        let want: u64 = argv
            .get(pos + 3)
            .and_then(|s| s.parse().ok())
            .unwrap_or(10);
        let game = argv.iter().any(|a| a == "--game");
        // extra checks for this scripted session
        let mon_arg: Option<u8> = argv
            .iter()
            .position(|a| a == "--monitor")
            .and_then(|i| argv.get(i + 1))
            .and_then(|s| s.parse().ok());
        let file_arg: Option<String> = argv
            .iter()
            .position(|a| a == "--sendfile")
            .and_then(|i| argv.get(i + 1))
            .cloned();
        let sh = shared.clone();
        let idc = id.clone();
        rt().spawn(async move { viewer::run_viewer(sh, idc, pw).await });
        let start = std::time::Instant::now();
        let mut last = 0u64;
        let mut mode_sent = false;
        let mut extras_done = false;
        let mut mon_switched = false;
        let mut mons_printed = false;
        loop {
            std::thread::sleep(Duration::from_millis(200));
            if !mons_printed {
                let mons = shared.monitors.lock().unwrap().clone();
                if !mons.is_empty() {
                    mons_printed = true;
                    for (i, m) in mons.iter().enumerate() {
                        println!("monitor {}: {} {}x{}", i, m.name, m.w, m.h);
                    }
                }
            }
            if !extras_done && shared.connected.load(Ordering::Relaxed) {
                extras_done = true;
                if let Some(idx) = mon_arg {
                    println!("switching to monitor {}", idx);
                    shared.send_input(Msg::SetMonitor { index: idx });
                }
                if let Some(f) = file_arg.clone() {
                    let path = std::path::PathBuf::from(&f);
                    match shared.xfer.lock().unwrap().as_mut() {
                        Some(x) => {
                            println!("sending file {}", f);
                            x.send_path(path);
                        }
                        None => println!("FAIL: kein Transfer-Modul"),
                    }
                }
            }
            if mon_arg.is_some() && !mon_switched && shared.connected.load(Ordering::Relaxed) {
                let act = shared.active_monitor.load(Ordering::Relaxed);
                if Some(act) == mon_arg {
                    let (rw, rh) = *shared.remote_size.lock().unwrap();
                    println!("monitor active {} -> {}x{}", act, rw, rh);
                    mon_switched = true;
                }
            }
            if file_arg.is_some() {
                let done = shared
                    .xfers
                    .lock()
                    .unwrap()
                    .iter()
                    .find(|x| !x.incoming && x.finished)
                    .cloned();
                if let Some(p) = done {
                    if p.error.is_empty() {
                        println!("FILE OK: {} ({} Bytes)", p.name, p.done);
                        std::process::exit(0);
                    }
                    println!("FILE FAIL: {} - {}", p.name, p.error);
                    std::process::exit(1);
                }
                if start.elapsed() > Duration::from_secs(120) {
                    println!("FILE FAIL: Zeitueberschreitung");
                    std::process::exit(1);
                }
                continue;
            }
            if game && !mode_sent && shared.connected.load(Ordering::Relaxed) {
                shared.send_input(Msg::SetMode {
                    mode: proto::MODE_GAME,
                });
                shared.mode.store(proto::MODE_GAME, Ordering::Relaxed);
                mode_sent = true;
                println!("mode -> game");
            }
            let seq = shared
                .frame
                .lock()
                .unwrap()
                .as_ref()
                .map(|f| (f.seq, f.width, f.height))
                .unwrap_or((0, 0, 0));
            if seq.0 != last {
                last = seq.0;
                println!("frame {} {}x{}", seq.0, seq.1, seq.2);
            }
            if last >= want {
                let st = *shared.stats.lock().unwrap();
                let cur = *shared.remote_cursor.lock().unwrap();
                let udp = shared.udp_frames.load(Ordering::Relaxed);
                println!(
                    "OK: {} Frames in {:.1}s, {:.0} fps, {:.0} kbit/s, {:.0} ms rtt, Cursor {:?}",
                    last,
                    start.elapsed().as_secs_f32(),
                    st.fps,
                    st.kbps,
                    st.latency_ms,
                    cur
                );
                println!(
                    "Transport: {} ({} Bilder direkt per UDP, Rest ueber den Relay)",
                    if shared.direct.load(Ordering::Relaxed) {
                        "direkt (P2P)"
                    } else {
                        "Relay"
                    },
                    udp
                );
                std::process::exit(0);
            }
            let status = shared.viewer_status.lock().unwrap().clone();
            if status.starts_with("Fehler") {
                println!("FAIL: {}", status);
                std::process::exit(1);
            }
            if start.elapsed() > Duration::from_secs(30) {
                println!("FAIL: {}", shared.viewer_status.lock().unwrap());
                std::process::exit(1);
            }
        }
    }

    // scripted input self test:  freeviewer --inputtest <id> <password>
    if let Some(pos) = argv.iter().position(|a| a == "--inputtest") {
        selftest::input_selftest(shared.clone(), &argv, pos);
    }

    vinput::init(shared.clone());
    update::watcher(shared.clone());

    // Started by the autostart entry (or by the service): no window in the
    // user's face, only the tray icon. The host runs either way.
    let start_hidden = std::env::args().any(|a| a == "--tray" || a == "--background");
    // A second window of the same user is pointless - bring the first one to
    // the front instead.
    if !tray::claim_single_instance() {
        println!("FreeViewer laeuft bereits - Fenster nach vorne geholt.");
        return Ok(());
    }
    autostart::refresh();
    tray::start(shared.clone());

    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_inner_size([1100.0, 720.0])
            .with_min_inner_size([700.0, 470.0])
            .with_visible(!start_hidden)
            .with_title("FreeViewer"),
        ..Default::default()
    };

    eframe::run_native(
        "FreeViewer",
        options,
        Box::new(move |_cc| Ok(Box::new(App::new(shared, start_hidden)))),
    )
}

struct App {
    shared: Arc<Shared>,
    partner_id: String,
    partner_pw: String,
    tex: Option<egui::TextureHandle>,
    last_seq: u64,
    last_mods: egui::Modifiers,
    viewer_task: Option<tokio::task::JoinHandle<()>>,
    hint: String,
    /// Is a permanent password stored for this machine?
    pw_fixed: bool,
    /// Everyone we connected to before.
    book: partners::Book,
    /// Store the password of the next connection?
    remember_pw: bool,
    /// Entry currently being renamed: (id, buffer).
    renaming: Option<(String, String)>,
    /// Running session: partner id + when it started.
    session: Option<(String, std::time::Instant)>,
    /// Until when the "how do I get out" hint stays on screen.
    hint_until: Option<std::time::Instant>,
    /// Started into the tray, so the window has to disappear once winit has
    /// created it.
    start_hidden: bool,
    first_frame: bool,
    /// Does the Run key point at us? Re-read every few seconds, because the
    /// tray menu can change it behind the GUI's back.
    autostart: bool,
    autostart_checked: std::time::Instant,
    /// The "I am still running down there" balloon is shown only once.
    told_about_tray: bool,
    /// Is the Windows service installed and running?
    service_on: bool,
    /// Until when a start into the tray keeps forcing the window away.
    hide_until: Option<std::time::Instant>,
}

impl App {
    fn new(shared: Arc<Shared>, start_hidden: bool) -> Self {
        Self {
            shared,
            partner_id: String::new(),
            partner_pw: String::new(),
            tex: None,
            last_seq: 0,
            last_mods: egui::Modifiers::default(),
            viewer_task: None,
            hint: String::new(),
            pw_fixed: ident::has_fixed_password(),
            book: partners::Book::load(),
            remember_pw: false,
            renaming: None,
            session: None,
            hint_until: None,
            start_hidden,
            first_frame: true,
            autostart: autostart::enabled(),
            autostart_checked: std::time::Instant::now(),
            told_about_tray: false,
            service_on: service::running(),
            hide_until: None,
        }
    }

    /// Everything around the tray icon: hide instead of quit, pick up what
    /// the menu did, keep the checkbox in sync.
    fn tray_ui(&mut self, ctx: &egui::Context) {
        if self.first_frame {
            self.first_frame = false;
            if self.start_hidden {
                ctx.send_viewport_cmd(egui::ViewportCommand::Visible(false));
                tray::hide_window();
                self.hide_until =
                    Some(std::time::Instant::now() + Duration::from_secs(3));
            }
        }
        // eframe pushes the window back on screen during the first frames, so
        // insist for a moment - but only for that moment, otherwise the tray
        // menu could never open the window again.
        if let Some(until) = self.hide_until {
            if std::time::Instant::now() < until {
                if tray::is_hidden() {
                    tray::hide_window();
                }
            } else {
                self.hide_until = None;
            }
        }
        if let Some(err) = tray::take_error() {
            self.shared.set_update_status(err);
        }
        if self.autostart_checked.elapsed() > Duration::from_secs(2) {
            self.autostart_checked = std::time::Instant::now();
            self.autostart = autostart::enabled();
            self.service_on = service::running();
        }
        // The X button folds the window away instead of killing the host -
        // that is what "runs in the background" means. Quitting for real is
        // in the tray menu.
        if ctx.input(|i| i.viewport().close_requested()) && tray::is_running() {
            ctx.send_viewport_cmd(egui::ViewportCommand::CancelClose);
            tray::hide_window();
            if !self.told_about_tray {
                self.told_about_tray = true;
                tray::balloon(
                    "FreeViewer laeuft weiter",
                    "Das Fenster ist nur zugeklappt. Ueber das Symbol im Infobereich kommst du zurueck - oder beendest FreeViewer ganz.",
                );
            }
        }
        if tray::is_hidden() {
            // no picture to draw, but the menu must stay responsive
            ctx.request_repaint_after(Duration::from_millis(400));
        }
    }

    fn start_session(&mut self) {
        let id: String = self
            .partner_id
            .chars()
            .filter(|c| c.is_ascii_digit())
            .collect();
        if id.len() < 9 {
            self.shared
                .set_viewer_status("Bitte 9-stellige Partner-ID eingeben");
            return;
        }
        let sh = self.shared.clone();
        let pw = self.partner_pw.clone();
        self.tex = None;
        self.last_seq = 0;
        self.book.started(&id, &pw, self.remember_pw);
        self.session = Some((id.clone(), std::time::Instant::now()));
        self.hint_until = Some(std::time::Instant::now() + Duration::from_secs(8));
        self.shared.mode.store(proto::MODE_ADMIN, Ordering::Relaxed);
        self.viewer_task = Some(rt().spawn(async move {
            viewer::run_viewer(sh, id, pw).await;
        }));
    }

    /// Books the time of a finished session into the address book.
    fn close_session(&mut self) {
        if let Some((id, started)) = self.session.take() {
            self.book.ended(&id, started.elapsed().as_secs());
        }
        self.hint_until = None;
    }

    fn stop_session(&mut self) {
        self.close_session();
        vinput::set_active(false);
        // release modifiers that might still be held down on the remote side
        for code in [proto::KEY_SHIFT, proto::KEY_CTRL, proto::KEY_ALT] {
            self.shared.send_input(Msg::Key {
                code,
                named: true,
                down: false,
            });
        }
        if let Some(t) = self.viewer_task.take() {
            t.abort();
        }
        self.shared.connected.store(false, Ordering::Relaxed);
        self.shared.connecting.store(false, Ordering::Relaxed);
        *self.shared.input_tx.lock().unwrap() = None;
        *self.shared.frame.lock().unwrap() = None;
        self.shared.set_viewer_status("Getrennt");
        self.tex = None;
        self.last_seq = 0;
        self.last_mods = egui::Modifiers::default();
    }

    fn set_mode(&mut self, mode: u8) {
        self.shared.mode.store(mode, Ordering::Relaxed);
        self.shared.send_input(Msg::SetMode { mode });
        if mode == proto::MODE_GAME {
            vinput::set_active(true);
            self.hint =
                "Spielmodus: Maus + Tastatur werden komplett uebertragen. Rechte Strg = freigeben."
                    .to_string();
        } else {
            vinput::set_active(false);
            self.hint = "Fernwartung: scharfes Bild, absolute Maus.".to_string();
        }
    }

    /// Files dropped onto the window go to the other side of the session.
    fn handle_drops(&mut self, ctx: &egui::Context) {
        let files: Vec<std::path::PathBuf> = ctx.input(|i| {
            i.raw
                .dropped_files
                .iter()
                .filter_map(|f| f.path.clone())
                .collect()
        });
        if files.is_empty() {
            return;
        }
        let mut guard = self.shared.xfer.lock().unwrap();
        match guard.as_mut() {
            Some(x) => {
                let n = files.len();
                for p in files {
                    x.send_path(p);
                }
                self.hint = format!("{} Datei(en) werden uebertragen", n);
            }
            None => {
                self.hint = "Keine aktive Sitzung - Datei nicht gesendet".to_string();
            }
        }
    }

    fn pick_and_send(&mut self) {
        let files = rfd::FileDialog::new()
            .set_title("Datei(en) an die Gegenstelle senden")
            .pick_files();
        if let Some(files) = files {
            let mut guard = self.shared.xfer.lock().unwrap();
            if let Some(x) = guard.as_mut() {
                for p in files {
                    x.send_path(p);
                }
            } else {
                self.hint = "Keine aktive Sitzung".to_string();
            }
        }
    }

    fn open_drop_dir(&self) {
        let dir = self.shared.drop_dir.lock().unwrap().clone();
        let _ = std::fs::create_dir_all(&dir);
        #[cfg(windows)]
        {
            let _ = std::process::Command::new("explorer").arg(&dir).spawn();
        }
    }

    /// Bottom bar with every running/finished transfer of this session.
    fn transfer_ui(&mut self, ctx: &egui::Context) {
        let list = self.shared.xfers.lock().unwrap().clone();
        if list.is_empty() {
            return;
        }
        egui::TopBottomPanel::bottom("xfer_bar").show(ctx, |ui| {
            let mut clear = false;
            ui.horizontal(|ui| {
                ui.label(egui::RichText::new("Dateien").strong());
                if ui.small_button("Ordner oeffnen").clicked() {
                    self.open_drop_dir();
                }
                if ui.small_button("Liste leeren").clicked() {
                    clear = true;
                }
            });
            egui::ScrollArea::vertical()
                .max_height(96.0)
                .show(ui, |ui| {
                    for p in &list {
                        ui.horizontal(|ui| {
                            ui.label(if p.incoming { "<-" } else { "->" });
                            ui.label(&p.name);
                            if !p.error.is_empty() {
                                ui.colored_label(
                                    egui::Color32::from_rgb(230, 120, 120),
                                    &p.error,
                                );
                            } else if p.finished {
                                ui.colored_label(
                                    egui::Color32::from_rgb(120, 220, 120),
                                    "fertig",
                                );
                            } else {
                                ui.add(
                                    egui::ProgressBar::new(p.percent())
                                        .desired_width(160.0)
                                        .show_percentage(),
                                );
                                ui.label(format!(
                                    "{:.1}/{:.1} MB",
                                    p.done as f32 / 1_048_576.0,
                                    p.size as f32 / 1_048_576.0
                                ));
                            }
                        });
                    }
                });
            if clear {
                self.shared
                    .xfers
                    .lock()
                    .unwrap()
                    .retain(|p| !p.finished && p.error.is_empty());
            }
        });
    }

    /// Update line at the bottom of the home screen.
    fn update_ui(&mut self, ui: &mut egui::Ui) {
        let pending = self.shared.update.lock().unwrap().clone();
        let status = self.shared.update_status.lock().unwrap().clone();
        ui.horizontal(|ui| {
            let mut auto = self.shared.auto_update.load(Ordering::Relaxed);
            if ui.checkbox(&mut auto, "Automatisch aktualisieren").changed() {
                self.shared.auto_update.store(auto, Ordering::Relaxed);
                ident::set_auto_update(auto);
            }
            let mut boot = self.autostart;
            if ui
                .checkbox(&mut boot, "Mit Windows starten")
                .on_hover_text(
                    "Startet FreeViewer bei der Anmeldung unsichtbar in den Infobereich",
                )
                .changed()
            {
                match autostart::set(boot) {
                    Ok(()) => self.autostart = boot,
                    Err(e) => self
                        .shared
                        .set_update_status(format!("Autostart ging nicht: {}", e)),
                }
            }
            let mut svc = self.service_on;
            if ui
                .checkbox(&mut svc, "Auch am Anmeldebildschirm (Dienst)")
                .on_hover_text(
                    "Installiert einen Windows-Dienst, der FreeViewer schon vor der Anmeldung \
                     bereithaelt und den Sperrbildschirm zeigen kann. Fragt nach \
                     Administrator-Rechten.",
                )
                .changed()
            {
                let flag = if svc {
                    "--install-service"
                } else {
                    "--uninstall-service"
                };
                match service::elevate(flag) {
                    Ok(()) => self
                        .shared
                        .set_update_status("Bitte die Windows-Abfrage bestaetigen..."),
                    Err(e) => self.shared.set_update_status(format!("{}", e)),
                }
            }
            if let Some(rel) = pending {
                if ui
                    .button(format!("Update {} jetzt installieren", rel.version))
                    .clicked()
                {
                    self.shared
                        .set_update_status(format!("Installiere {} ...", rel.version));
                    let sh = self.shared.clone();
                    std::thread::spawn(move || {
                        if let Err(e) = update::install(&rel) {
                            sh.set_update_status(format!("Update fehlgeschlagen: {}", e));
                        }
                    });
                }
            }
            if !status.is_empty() {
                ui.label(egui::RichText::new(status).weak().size(11.0));
            }
        });
    }

    fn pull_frame(&mut self, ctx: &egui::Context) {
        let img = {
            let guard = self.shared.frame.lock().unwrap();
            match guard.as_ref() {
                Some(f) if f.seq != self.last_seq => {
                    self.last_seq = f.seq;
                    Some(egui::ColorImage::from_rgba_unmultiplied(
                        [f.width as usize, f.height as usize],
                        &f.rgba,
                    ))
                }
                _ => None,
            }
        };
        if let Some(ci) = img {
            match self.tex.as_mut() {
                Some(t) => t.set(ci, egui::TextureOptions::LINEAR),
                None => {
                    self.tex = Some(ctx.load_texture("remote", ci, egui::TextureOptions::LINEAR))
                }
            }
        }
    }

    fn home_ui(&mut self, ui: &mut egui::Ui) {
        ui.add_space(6.0);
        ui.horizontal(|ui| {
            ui.heading("FreeViewer");
            ui.label(
                egui::RichText::new(format!(
                    "v{} - frei, verschluesselt, ohne Konto",
                    update::VERSION
                ))
                .weak(),
            );
        });
        ui.separator();
        ui.add_space(10.0);

        let my_id = self.shared.my_id.lock().unwrap().clone();
        let host_status = self.shared.host_status.lock().unwrap().clone();
        let host_peer = self.shared.host_peer.lock().unwrap().clone();
        let viewer_status = self.shared.viewer_status.lock().unwrap().clone();
        let connecting = self.shared.connecting.load(Ordering::Relaxed);

        ui.columns(2, |cols| {
            // ---- left: this machine ----
            cols[0].group(|ui| {
                ui.set_min_height(230.0);
                ui.label(egui::RichText::new("Dieser PC").strong().size(17.0));
                ui.add_space(8.0);
                ui.label("Deine ID");
                let id_text = if my_id.len() == 9 {
                    format!("{} {} {}", &my_id[0..3], &my_id[3..6], &my_id[6..9])
                } else {
                    "--- --- ---".to_string()
                };
                ui.add(
                    egui::Label::new(egui::RichText::new(id_text).monospace().size(28.0))
                        .selectable(true),
                );
                ui.add_space(8.0);
                ui.label("Passwort");
                let mut changed;
                {
                    let mut pw = self.shared.password.lock().unwrap();
                    changed = ui
                        .add(
                            egui::TextEdit::singleline(&mut *pw)
                                .font(egui::TextStyle::Monospace)
                                .desired_width(180.0),
                        )
                        .changed();
                }
                ui.horizontal(|ui| {
                    if ui.button("Neues Passwort").clicked() {
                        *self.shared.password.lock().unwrap() = ident::random_password();
                        changed = true;
                    }
                    if ui
                        .checkbox(&mut self.pw_fixed, "merken")
                        .on_hover_text(
                            "An: dieses Passwort bleibt nach einem Neustart gleich (unbeaufsichtigter Zugriff).\nAus: bei jedem Start ein neues Zufallspasswort.\nGilt nur fuer DIESEN PC.",
                        )
                        .changed()
                    {
                        changed = true;
                    }
                });
                if changed {
                    let pw = self.shared.password.lock().unwrap().clone();
                    let store = if self.pw_fixed { Some(pw.as_str()) } else { None };
                    if let Err(e) = ident::set_fixed_password(store) {
                        self.hint = format!("Passwort nicht gespeichert: {}", e);
                    }
                }
                ui.label(
                    egui::RichText::new(if self.pw_fixed {
                        "festes Passwort - ueberlebt Neustarts"
                    } else {
                        "Sitzungspasswort - bei jedem Start neu"
                    })
                    .weak()
                    .size(11.0),
                );
                ui.add_space(10.0);
                ui.label(egui::RichText::new(host_status).weak());
                ui.label(egui::RichText::new(host_peer).weak());
                if self.shared.xfer.lock().unwrap().is_some() {
                    ui.add_space(4.0);
                    ui.label(
                        egui::RichText::new(
                            "Dateien fuer die Gegenstelle einfach hier ins Fenster ziehen",
                        )
                        .weak()
                        .size(11.0),
                    );
                }
            });

            // ---- right: connect to someone ----
            cols[1].group(|ui| {
                ui.set_min_height(230.0);
                ui.label(egui::RichText::new("Anderen PC steuern").strong().size(17.0));
                ui.add_space(8.0);
                ui.label("Partner-ID");
                ui.add(
                    egui::TextEdit::singleline(&mut self.partner_id)
                        .font(egui::TextStyle::Monospace)
                        .desired_width(200.0)
                        .hint_text("123456789"),
                );
                ui.add_space(6.0);
                ui.label("Passwort");
                ui.add(
                    egui::TextEdit::singleline(&mut self.partner_pw)
                        .password(true)
                        .desired_width(200.0),
                );
                ui.horizontal(|ui| {
                    ui.checkbox(&mut self.remember_pw, "Passwort merken")
                        .on_hover_text(
                            "Wird verschluesselt neben der Geraete-Kennung abgelegt und nur auf diesem PC entschluesselbar.",
                        );
                });
                ui.add_space(10.0);
                let btn = egui::Button::new(if connecting {
                    "Verbinde..."
                } else {
                    "Verbinden"
                });
                if ui.add_enabled(!connecting, btn).clicked() {
                    self.start_session();
                }
                ui.add_space(8.0);
                ui.label(egui::RichText::new(viewer_status).weak());
                ui.add_space(6.0);
                self.partner_list(ui, connecting);
            });
        });

        ui.add_space(12.0);
        self.update_ui(ui);

        ui.add_space(8.0);
        ui.label(
            egui::RichText::new(format!("Relay: {}", self.shared.relay_url))
                .weak()
                .size(11.0),
        );
    }

    /// "Zuletzt verbunden" - the address book. Favourites first, one click
    /// fills the form, one more connects.
    fn partner_list(&mut self, ui: &mut egui::Ui, connecting: bool) {
        let list = self.book.sorted();
        ui.separator();
        ui.horizontal(|ui| {
            ui.label(egui::RichText::new("Zuletzt verbunden").strong());
            if list.is_empty() {
                ui.label(
                    egui::RichText::new("- noch niemand -")
                        .weak()
                        .size(11.0),
                );
            }
        });

        let mut connect_now: Option<String> = None;
        let mut fill: Option<String> = None;
        let mut fav: Option<String> = None;
        let mut del: Option<String> = None;
        let mut rename_done: Option<(String, String)> = None;
        let mut rename_start: Option<String> = None;

        egui::ScrollArea::vertical()
            .max_height(150.0)
            .show(ui, |ui| {
                for p in &list {
                    ui.horizontal(|ui| {
                        let star = if p.favorite { "*" } else { "-" };
                        if ui
                            .small_button(star)
                            .on_hover_text("Anheften / loesen")
                            .clicked()
                        {
                            fav = Some(p.id.clone());
                        }

                        if let Some((rid, buf)) = self.renaming.as_mut() {
                            if rid == &p.id {
                                let r = ui.add(
                                    egui::TextEdit::singleline(buf).desired_width(120.0),
                                );
                                if r.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter)) {
                                    rename_done = Some((p.id.clone(), buf.clone()));
                                }
                                if ui.small_button("ok").clicked() {
                                    rename_done = Some((p.id.clone(), buf.clone()));
                                }
                                return;
                            }
                        }

                        let label = ui.add(
                            egui::Label::new(egui::RichText::new(p.label()).strong())
                                .sense(egui::Sense::click()),
                        );
                        if label.clicked() {
                            fill = Some(p.id.clone());
                        }
                        if label.double_clicked() {
                            connect_now = Some(p.id.clone());
                        }
                        label.on_hover_text(format!(
                            "{}\n{} Verbindungen, insgesamt {}\nzuletzt {}",
                            partners::pretty_id(&p.id),
                            p.count,
                            p.total(),
                            p.ago()
                        ));

                        ui.label(egui::RichText::new(p.ago()).weak().size(11.0));
                        if p.secret.is_some() {
                            ui.label(
                                egui::RichText::new("Passwort gespeichert")
                                    .weak()
                                    .size(10.0),
                            );
                        }

                        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                            if ui.small_button("x").on_hover_text("Aus der Liste entfernen").clicked() {
                                del = Some(p.id.clone());
                            }
                            if ui.small_button("umbenennen").clicked() {
                                rename_start = Some(p.id.clone());
                            }
                            if ui
                                .add_enabled(!connecting, egui::Button::new("verbinden").small())
                                .clicked()
                            {
                                connect_now = Some(p.id.clone());
                            }
                        });
                    });
                }
            });

        if let Some(id) = fav {
            self.book.toggle_favorite(&id);
        }
        if let Some(id) = del {
            self.book.remove(&id);
        }
        if let Some(id) = rename_start {
            let cur = self
                .book
                .get(&id)
                .map(|p| p.name.clone())
                .unwrap_or_default();
            self.renaming = Some((id, cur));
        }
        if let Some((id, name)) = rename_done {
            self.book.rename(&id, &name);
            self.renaming = None;
        }
        if let Some(id) = fill.or(connect_now.clone()) {
            self.partner_id = id.clone();
            if let Some(pw) = self.book.password(&id) {
                self.partner_pw = pw;
                self.remember_pw = true;
            }
        }
        if connect_now.is_some() && !connecting {
            self.start_session();
        }
    }

    fn session_ui(&mut self, ctx: &egui::Context) {
        let stats = *self.shared.stats.lock().unwrap();
        let game = self.shared.game_mode();
        let mut disconnect = false;
        let mut want_mode: Option<u8> = None;
        let mut want_mon: Option<u8> = None;
        let mut special: Option<u8> = None;
        let mut pick = false;
        let mut open_dir = false;

        egui::TopBottomPanel::top("session_bar").show(ctx, |ui| {
            ui.horizontal(|ui| {
                if ui.button("Trennen").clicked() {
                    disconnect = true;
                }
                ui.separator();

                ui.label("Modus:");
                if ui
                    .selectable_label(!game, "Fernwartung")
                    .on_hover_text("Scharfes Bild, absolute Maus, Tastatur nur im Fenster")
                    .clicked()
                    && game
                {
                    want_mode = Some(proto::MODE_ADMIN);
                }
                if ui
                    .selectable_label(game, "Spiel")
                    .on_hover_text(
                        "Relative Maus fuer Ingame-Kameras, komplette Tastatur (Win, Alt+Tab), mehr fps",
                    )
                    .clicked()
                    && !game
                {
                    want_mode = Some(proto::MODE_GAME);
                }

                let mons = self.shared.monitors.lock().unwrap().clone();
                if mons.len() > 1 {
                    ui.separator();
                    let act = self.shared.active_monitor.load(Ordering::Relaxed) as usize;
                    let label = |i: usize, m: &proto::MonitorInfo| {
                        format!("{}. {} ({}x{})", i + 1, m.name, m.w, m.h)
                    };
                    let cur = mons
                        .get(act)
                        .map(|m| label(act, m))
                        .unwrap_or_else(|| "Bildschirm".to_string());
                    egui::ComboBox::from_id_salt("monitor_pick")
                        .selected_text(cur)
                        .show_ui(ui, |ui| {
                            for (i, m) in mons.iter().enumerate() {
                                if ui.selectable_label(i == act, label(i, m)).clicked() {
                                    want_mon = Some(i as u8);
                                }
                            }
                        });
                }

                ui.separator();
                let mut want_pick = false;
                let mut want_open = false;
                ui.menu_button("Dateien", |ui| {
                    if ui.button("Datei senden...").clicked() {
                        want_pick = true;
                        ui.close();
                    }
                    if ui.button("Empfangsordner oeffnen").clicked() {
                        want_open = true;
                        ui.close();
                    }
                    ui.label(
                        egui::RichText::new("Tipp: Dateien einfach ins Fenster ziehen")
                            .weak()
                            .size(11.0),
                    );
                });
                if want_pick {
                    pick = true;
                }
                if want_open {
                    open_dir = true;
                }

                ui.separator();
                ui.menu_button("Tasten senden", |ui| {
                    if ui.button("Strg+Alt+Entf").clicked() {
                        special = Some(proto::SPECIAL_CAD);
                        ui.close();
                    }
                    if ui.button("Task-Manager (Strg+Shift+Esc)").clicked() {
                        special = Some(proto::SPECIAL_TASKMGR);
                        ui.close();
                    }
                    if ui.button("Windows-Taste").clicked() {
                        special = Some(proto::SPECIAL_WIN);
                        ui.close();
                    }
                    if ui.button("Alt+Tab").clicked() {
                        special = Some(proto::SPECIAL_ALTTAB);
                        ui.close();
                    }
                    if ui.button("Sperren (Win+L)").clicked() {
                        special = Some(proto::SPECIAL_LOCK);
                        ui.close();
                    }
                });

                ui.separator();
                let (rw, rh) = *self.shared.remote_size.lock().unwrap();
                let direct = self.shared.direct.load(Ordering::Relaxed);
                ui.label(format!(
                    "{}x{}   {:.0} fps   {:.0} kbit/s   {:.0} ms   {}",
                    rw,
                    rh,
                    stats.fps,
                    stats.kbps,
                    stats.latency_ms,
                    if direct { "direkt" } else { "ueber Relay" }
                ));
                ui.separator();
                ui.label(
                    egui::RichText::new("rechte Strg = raus")
                        .weak()
                        .size(11.0),
                )
                .on_hover_text(
                    "Einmal druecken gibt Maus und Tastatur wieder an diesen PC zurueck.\nDreimal schnell hintereinander beendet die Sitzung.",
                );
                if game {
                    ui.separator();
                    if vinput::is_active() {
                        ui.colored_label(
                            egui::Color32::from_rgb(120, 220, 120),
                            "Eingabe gegriffen - rechte Strg loest",
                        );
                    } else {
                        ui.colored_label(
                            egui::Color32::from_rgb(230, 190, 90),
                            "frei - ins Bild klicken zum Greifen",
                        );
                    }
                }
            });
        });

        if let Some(until) = self.hint_until {
            if std::time::Instant::now() < until {
                let text = if self.hint.is_empty() {
                    "Rechte Strg = Eingabe freigeben  |  3x rechte Strg = Sitzung beenden"
                        .to_string()
                } else {
                    self.hint.clone()
                };
                egui::TopBottomPanel::top("escape_hint").show(ctx, |ui| {
                    ui.horizontal(|ui| {
                        ui.label(
                            egui::RichText::new(text)
                                .color(egui::Color32::from_rgb(240, 220, 120)),
                        );
                    });
                });
            } else {
                self.hint_until = None;
                self.hint.clear();
            }
        }

        let mut image_rect: Option<egui::Rect> = None;
        let mut clicked_image = false;
        egui::CentralPanel::default()
            .frame(egui::Frame::NONE.fill(egui::Color32::from_rgb(18, 18, 18)))
            .show(ctx, |ui| {
                if let Some(tex) = &self.tex {
                    let size_px = tex.size();
                    let (iw, ih) = (size_px[0] as f32, size_px[1] as f32);
                    let avail = ui.available_size();
                    let scale = (avail.x / iw).min(avail.y / ih).max(0.05);
                    let size = egui::vec2(iw * scale, ih * scale);
                    ui.vertical_centered(|ui| {
                        let resp = ui.add(
                            egui::Image::new(egui::load::SizedTexture::new(tex.id(), size))
                                .sense(egui::Sense::click_and_drag()),
                        );
                        clicked_image = resp.clicked();
                        image_rect = Some(resp.rect);
                    });
                } else {
                    ui.centered_and_justified(|ui| {
                        ui.label("Warte auf Bild...");
                    });
                }
            });

        if let Some(rect) = image_rect {
            self.draw_remote_cursor(ctx, rect, game);
            self.update_grab(ctx, rect, game, clicked_image);
            self.forward_input(ctx, rect, game);
        }

        if let Some(m) = want_mode {
            self.set_mode(m);
        }
        if let Some(i) = want_mon {
            self.shared.active_monitor.store(i, Ordering::Relaxed);
            self.shared.send_input(Msg::SetMonitor { index: i });
            self.tex = None;
            self.last_seq = 0;
        }
        if let Some(code) = special {
            self.shared.send_input(Msg::Special { code });
        }
        if pick {
            self.pick_and_send();
        }
        if open_dir {
            self.open_drop_dir();
        }
        if disconnect {
            self.stop_session();
        }
    }

    /// The duplication API does not paint the cursor into the frame, so the
    /// viewer draws the remote pointer itself.
    fn draw_remote_cursor(&self, ctx: &egui::Context, rect: egui::Rect, game: bool) {
        if game {
            return;
        }
        let (x, y, visible) = *self.shared.remote_cursor.lock().unwrap();
        if !visible {
            return;
        }
        let p = egui::pos2(
            rect.left() + rect.width() * (x as f32 / 10000.0).clamp(0.0, 1.0),
            rect.top() + rect.height() * (y as f32 / 10000.0).clamp(0.0, 1.0),
        );
        let painter = ctx.layer_painter(egui::LayerId::new(
            egui::Order::Foreground,
            egui::Id::new("remote_cursor"),
        ));
        let pts = vec![
            p,
            egui::pos2(p.x, p.y + 17.0),
            egui::pos2(p.x + 4.5, p.y + 12.5),
            egui::pos2(p.x + 10.5, p.y + 12.0),
        ];
        painter.add(egui::Shape::convex_polygon(
            pts,
            egui::Color32::WHITE,
            egui::Stroke::new(1.2, egui::Color32::BLACK),
        ));
    }

    /// Keeps the pointer lock in sync with focus and clicks.
    fn update_grab(&mut self, ctx: &egui::Context, rect: egui::Rect, game: bool, clicked: bool) {
        if !game {
            return;
        }
        let focused = ctx.input(|i| i.viewport().focused.unwrap_or(true));
        if !focused {
            vinput::set_active(false);
            return;
        }
        // center of the picture in physical screen pixels
        let ppp = ctx.pixels_per_point();
        let origin = ctx
            .input(|i| i.viewport().inner_rect)
            .map(|r| r.min)
            .unwrap_or(egui::Pos2::ZERO);
        let c = rect.center();
        vinput::set_center(
            ((origin.x + c.x) * ppp) as i32,
            ((origin.y + c.y) * ppp) as i32,
        );
        if clicked && !vinput::is_active() {
            vinput::set_active(true);
        }
        if vinput::is_active() {
            ctx.set_cursor_icon(egui::CursorIcon::None);
        }
    }

    fn forward_input(&mut self, ctx: &egui::Context, rect: egui::Rect, game: bool) {
        let grabbed = game && vinput::is_active();

        // modifier keys are not part of egui's Key enum, so we diff the state.
        // In game mode the low level hook already forwards everything.
        if !grabbed {
            let mods = ctx.input(|i| i.modifiers);
            if mods.shift != self.last_mods.shift {
                self.shared.send_input(Msg::Key {
                    code: proto::KEY_SHIFT,
                    named: true,
                    down: mods.shift,
                });
            }
            if mods.ctrl != self.last_mods.ctrl {
                self.shared.send_input(Msg::Key {
                    code: proto::KEY_CTRL,
                    named: true,
                    down: mods.ctrl,
                });
            }
            if mods.alt != self.last_mods.alt {
                self.shared.send_input(Msg::Key {
                    code: proto::KEY_ALT,
                    named: true,
                    down: mods.alt,
                });
            }
            self.last_mods = mods;
        }

        let norm = |pos: egui::Pos2| -> Option<(i32, i32)> {
            if rect.width() < 1.0 || rect.height() < 1.0 || !rect.contains(pos) {
                return None;
            }
            let x = ((pos.x - rect.left()) / rect.width() * 10000.0).clamp(0.0, 10000.0) as i32;
            let y = ((pos.y - rect.top()) / rect.height() * 10000.0).clamp(0.0, 10000.0) as i32;
            Some((x, y))
        };

        let events = ctx.input(|i| i.events.clone());
        for ev in events {
            match ev {
                egui::Event::PointerMoved(pos) => {
                    // in game mode the raw delta path owns the mouse
                    if !grabbed {
                        if let Some((x, y)) = norm(pos) {
                            self.shared.send_input(Msg::MouseMove { x, y });
                        }
                    }
                }
                egui::Event::PointerButton {
                    pos,
                    button,
                    pressed,
                    ..
                } => {
                    let b = match button {
                        egui::PointerButton::Secondary => 1u8,
                        egui::PointerButton::Middle => 2u8,
                        egui::PointerButton::Extra1 => 3u8,
                        egui::PointerButton::Extra2 => 4u8,
                        _ => 0u8,
                    };
                    if grabbed {
                        self.shared.send_input(Msg::MouseButton {
                            button: b,
                            down: pressed,
                        });
                    } else if let Some((x, y)) = norm(pos) {
                        self.shared.send_input(Msg::MouseMove { x, y });
                        self.shared.send_input(Msg::MouseButton {
                            button: b,
                            down: pressed,
                        });
                    }
                }
                egui::Event::MouseWheel { unit, delta, .. } => {
                    let lines = match unit {
                        egui::MouseWheelUnit::Line => delta.y,
                        egui::MouseWheelUnit::Point => delta.y / 50.0,
                        egui::MouseWheelUnit::Page => delta.y * 3.0,
                    };
                    let l = lines.round() as i32;
                    if l != 0 {
                        self.shared.send_input(Msg::Wheel { lines: l });
                    }
                }
                egui::Event::Key {
                    key,
                    pressed,
                    repeat,
                    ..
                } => {
                    if !repeat && !grabbed {
                        if let Some((code, named)) = map_key(key) {
                            self.shared.send_input(Msg::Key {
                                code,
                                named,
                                down: pressed,
                            });
                        }
                    }
                }
                _ => {}
            }
        }
    }
}

fn map_key(k: egui::Key) -> Option<(u32, bool)> {
    use egui::Key as K;
    use proto as P;

    let named = match k {
        K::Backspace => Some(P::KEY_BACKSPACE),
        K::Enter => Some(P::KEY_ENTER),
        K::Tab => Some(P::KEY_TAB),
        K::Escape => Some(P::KEY_ESCAPE),
        K::ArrowLeft => Some(P::KEY_LEFT),
        K::ArrowRight => Some(P::KEY_RIGHT),
        K::ArrowUp => Some(P::KEY_UP),
        K::ArrowDown => Some(P::KEY_DOWN),
        K::Delete => Some(P::KEY_DELETE),
        K::Home => Some(P::KEY_HOME),
        K::End => Some(P::KEY_END),
        K::PageUp => Some(P::KEY_PAGEUP),
        K::PageDown => Some(P::KEY_PAGEDOWN),
        K::Insert => Some(P::KEY_INSERT),
        K::Space => Some(P::KEY_SPACE),
        _ => None,
    };
    if let Some(code) = named {
        return Some((code, true));
    }

    let name = k.name();
    // single letters: "A".."Z"
    if name.len() == 1 {
        let c = name.chars().next().unwrap().to_ascii_lowercase();
        return Some((c as u32, false));
    }
    if let Some(d) = name.strip_prefix("Num") {
        if d.len() == 1 {
            return Some((d.chars().next().unwrap() as u32, false));
        }
    }
    if let Some(f) = name.strip_prefix('F') {
        if let Ok(n) = f.parse::<u32>() {
            if (1..=12).contains(&n) {
                return Some((P::KEY_F1 + n - 1, true));
            }
        }
    }

    let ch = match k {
        K::Minus => '-',
        K::Plus => '+',
        K::Equals => '=',
        K::Comma => ',',
        K::Period => '.',
        K::Slash => '/',
        K::Backslash => '\\',
        K::Semicolon => ';',
        K::Colon => ':',
        K::Quote => '\'',
        K::Backtick => '`',
        K::OpenBracket => '[',
        K::CloseBracket => ']',
        _ => return None,
    };
    Some((ch as u32, false))
}

impl eframe::App for App {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        self.tray_ui(ctx);
        self.pull_frame(ctx);
        self.handle_drops(ctx);
        self.transfer_ui(ctx);

        // right Ctrl: 1x hands the input back, 3x leaves the session
        match self.shared.escape.swap(0, Ordering::Relaxed) {
            1 => {
                self.hint = "Eingabe freigegeben - ins Bild klicken uebernimmt wieder".to_string();
                self.hint_until = Some(std::time::Instant::now() + Duration::from_secs(4));
            }
            2 => {
                self.stop_session();
                self.hint = "Sitzung ueber die Host-Taste beendet".to_string();
            }
            _ => {}
        }

        if self.shared.connected.load(Ordering::Relaxed) {
            self.session_ui(ctx);
            ctx.request_repaint_after(Duration::from_millis(8));
        } else {
            if self.session.is_some() && !self.shared.connecting.load(Ordering::Relaxed) {
                // the session died on its own - still book the time
                self.close_session();
            }
            if vinput::is_active() {
                vinput::set_active(false);
            }
            egui::CentralPanel::default().show(ctx, |ui| self.home_ui(ui));
            ctx.request_repaint_after(Duration::from_millis(250));
        }
    }
}
