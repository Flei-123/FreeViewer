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
mod presence;
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

    // Draws every page once without a window:  freeviewer --uitest
    // egui can run headless, so a broken layout or a panic in the GUI shows
    // up in a build step instead of in front of the user.
    if std::env::args().any(|a| a == "--uitest") {
        let tmp = std::env::temp_dir().join("fv-uitest");
        let _ = std::fs::create_dir_all(&tmp);
        std::env::set_var("FV_CONFIG", &tmp);
        let ctx = egui::Context::default();
        install_theme(&ctx);
        let mut app = App::new(shared.clone(), false);
        app.book.started("123456789", "geheim", true);
        app.book.rename("123456789", "Test-PC");
        app.book.started("987654321", "", false);
        app.selected = Some("123456789".to_string());
        app.partner_id = "123456789".to_string();
        *shared.my_id.lock().unwrap() = "497628420".to_string();
        *shared.knock.lock().unwrap() = Some(shared::Knock {
            from: "Pait Laptop".to_string(),
            code: "1234".to_string(),
            at: std::time::Instant::now(),
        });
        let mut ok = true;
        for (name, view) in [
            ("Start", View::Start),
            ("Geraete", View::Devices),
            ("Einstellungen", View::Settings),
        ] {
            app.view = view;
            let input = egui::RawInput {
                screen_rect: Some(egui::Rect::from_min_size(
                    egui::pos2(0.0, 0.0),
                    egui::vec2(1100.0, 720.0),
                )),
                ..Default::default()
            };
            let out = ctx.run(input, |ctx| {
                app.knock_ui(ctx);
                egui::CentralPanel::default().show(ctx, |ui| app.home_ui(ui));
            });
            let shapes = out.shapes.len();
            println!("{}: {} Formen gezeichnet", name, shapes);
            if shapes < 10 {
                ok = false;
            }
        }
        let _ = std::fs::remove_dir_all(&tmp);
        println!("{}", if ok { "UITEST OK" } else { "UITEST FAIL" });
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
    let viewer_only = std::env::args()
        .any(|a| a == "--connect" || a == "--inputtest" || a == "--ask");
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

    // scripted "please confirm" test:  freeviewer --ask <id> [frames]
    if let Some(pos) = argv.iter().position(|a| a == "--ask") {
        let id = argv.get(pos + 1).cloned().unwrap_or_default();
        let want: u64 = argv
            .get(pos + 2)
            .and_then(|s| s.parse().ok())
            .unwrap_or(10);
        let sh = shared.clone();
        let idc = id.clone();
        rt().spawn(async move { viewer::run_viewer_ask(sh, idc).await });
        let start = std::time::Instant::now();
        let mut last = 0u64;
        loop {
            std::thread::sleep(Duration::from_millis(200));
            let seq = shared
                .frame
                .lock()
                .unwrap()
                .as_ref()
                .map(|f| f.seq)
                .unwrap_or(0);
            if seq != last {
                last = seq;
                println!("frame {}", seq);
            }
            if last >= want {
                println!(
                    "OK: {} Frames in {:.1}s, Code {}",
                    last,
                    start.elapsed().as_secs_f32(),
                    shared.session_code.lock().unwrap()
                );
                std::process::exit(0);
            }
            let status = shared.viewer_status.lock().unwrap().clone();
            if status.starts_with("Fehler") {
                println!("FAIL: {}", status);
                std::process::exit(1);
            }
            if start.elapsed() > Duration::from_secs(45) {
                println!("FAIL: Zeitueberschreitung - {}", status);
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
        Box::new(move |cc| {
            install_theme(&cc.egui_ctx);
            Ok(Box::new(App::new(shared, start_hidden)))
        }),
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
    /// Which of the three pages is on screen.
    view: View,
    /// Filter of the device list.
    search: String,
    /// Device shown in the detail column.
    selected: Option<String>,
    /// Who is online right now (asked at the relay).
    presence: Arc<presence::Watch>,
}

impl App {
    fn new(shared: Arc<Shared>, start_hidden: bool) -> Self {
        let watch = presence::Watch::new(shared.relay_url.clone());
        watch.start();
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
            view: View::Start,
            search: String::new(),
            selected: None,
            presence: watch,
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

    /// Connect without a password: the other side gets a question and has to
    /// allow the session by hand.
    fn start_ask_session(&mut self) {
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
        self.tex = None;
        self.last_seq = 0;
        self.book.started(&id, "", false);
        self.session = Some((id.clone(), std::time::Instant::now()));
        self.hint_until = Some(std::time::Instant::now() + Duration::from_secs(8));
        self.shared.mode.store(proto::MODE_ADMIN, Ordering::Relaxed);
        self.viewer_task = Some(rt().spawn(async move {
            viewer::run_viewer_ask(sh, id).await;
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

    // ----------------------------------------------------------- Oberflaeche

    /// Kopfzeile mit den drei Bereichen.
    fn top_bar(&mut self, ui: &mut egui::Ui) {
        let my_id = self.shared.my_id.lock().unwrap().clone();
        let online = !my_id.is_empty();
        ui.horizontal(|ui| {
            ui.label(
                egui::RichText::new("FreeViewer")
                    .size(20.0)
                    .strong()
                    .color(ACCENT),
            );
            ui.label(
                egui::RichText::new(format!("v{}", update::VERSION))
                    .size(11.0)
                    .color(MUTED),
            );
            ui.add_space(12.0);
            for (v, name) in [
                (View::Start, "Start"),
                (View::Devices, "Geraete"),
                (View::Settings, "Einstellungen"),
            ] {
                let sel = self.view == v;
                let text = egui::RichText::new(name).color(if sel { TEXT } else { MUTED });
                if ui.selectable_label(sel, text).clicked() {
                    self.view = v;
                }
            }
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                status_pill(ui, online, if online { "Bereit" } else { "Verbinde..." });
            });
        });
        ui.add_space(6.0);
        ui.separator();
        ui.add_space(10.0);
    }

    fn home_ui(&mut self, ui: &mut egui::Ui) {
        self.top_bar(ui);
        match self.view {
            View::Start => self.start_view(ui),
            View::Devices => self.devices_view(ui),
            View::Settings => self.settings_view(ui),
        }
    }

    /// Startseite: links dieser PC, rechts der Weg nach draussen.
    fn start_view(&mut self, ui: &mut egui::Ui) {
        let my_id = self.shared.my_id.lock().unwrap().clone();
        let host_status = self.shared.host_status.lock().unwrap().clone();
        let host_peer = self.shared.host_peer.lock().unwrap().clone();
        let viewer_status = self.shared.viewer_status.lock().unwrap().clone();
        let connecting = self.shared.connecting.load(Ordering::Relaxed);

        ui.columns(2, |cols| {
            // ---------------- links: Fernsteuerung zulassen ----------------
            let ui = &mut cols[0];
            section(ui, "Fernsteuerung zulassen");
            card(ui, |ui| {
                ui.label(egui::RichText::new("Ihre ID").color(MUTED).size(12.0));
                ui.horizontal(|ui| {
                    let id_text = if my_id.len() == 9 {
                        partners::pretty_id(&my_id)
                    } else {
                        "--- --- ---".to_string()
                    };
                    ui.add(
                        egui::Label::new(
                            egui::RichText::new(id_text).monospace().size(30.0).strong(),
                        )
                        .selectable(true),
                    );
                    if !my_id.is_empty() && ui.small_button("kopieren").clicked() {
                        ui.ctx().copy_text(partners::pretty_id(&my_id));
                        self.hint = "ID kopiert".to_string();
                    }
                });
                ui.add_space(10.0);
                ui.label(egui::RichText::new("Passwort").color(MUTED).size(12.0));
                let mut changed = false;
                ui.horizontal(|ui| {
                    {
                        let mut pw = self.shared.password.lock().unwrap();
                        changed = ui
                            .add(
                                egui::TextEdit::singleline(&mut *pw)
                                    .font(egui::TextStyle::Monospace)
                                    .desired_width(150.0),
                            )
                            .changed();
                    }
                    if ui.small_button("kopieren").clicked() {
                        let pw = self.shared.password.lock().unwrap().clone();
                        ui.ctx().copy_text(pw);
                        self.hint = "Passwort kopiert".to_string();
                    }
                    if ui
                        .small_button("neu")
                        .on_hover_text("Neues Zufallspasswort erzeugen")
                        .clicked()
                    {
                        *self.shared.password.lock().unwrap() = ident::random_password();
                        changed = true;
                    }
                });
                if ui
                    .checkbox(&mut self.pw_fixed, "Passwort behalten")
                    .on_hover_text(
                        "An: das Passwort bleibt nach einem Neustart gleich - noetig fuer unbeaufsichtigten Zugriff.\nAus: bei jedem Start ein neues.",
                    )
                    .changed()
                {
                    changed = true;
                }
                if changed {
                    let pw = self.shared.password.lock().unwrap().clone();
                    let store = if self.pw_fixed { Some(pw.as_str()) } else { None };
                    if let Err(e) = ident::set_fixed_password(store) {
                        self.hint = format!("Passwort nicht gespeichert: {}", e);
                    }
                }
                ui.add_space(8.0);
                ui.label(egui::RichText::new(host_status).color(MUTED).size(12.0));
                ui.label(egui::RichText::new(host_peer).color(MUTED).size(12.0));
            });

            ui.add_space(14.0);
            section(ui, "Unbeaufsichtigter Zugriff");
            card(ui, |ui| {
                let mut boot = self.autostart;
                if ui
                    .checkbox(&mut boot, "FreeViewer mit Windows starten")
                    .on_hover_text("Startet unsichtbar in den Infobereich.")
                    .changed()
                {
                    match autostart::set(boot) {
                        Ok(()) => self.autostart = boot,
                        Err(e) => self.hint = format!("Autostart ging nicht: {}", e),
                    }
                }
                let mut svc = self.service_on;
                if ui
                    .checkbox(&mut svc, "Einfachen Zugriff gewaehren (Dienst)")
                    .on_hover_text(
                        "Windows-Dienst: dieser PC ist auch am Sperr- und Anmeldebildschirm erreichbar, \
                         schon bevor sich jemand anmeldet. Fragt nach Administrator-Rechten.",
                    )
                    .changed()
                {
                    let flag = if svc {
                        "--install-service"
                    } else {
                        "--uninstall-service"
                    };
                    match service::elevate(flag) {
                        Ok(()) => self.hint = "Bitte die Windows-Abfrage bestaetigen...".to_string(),
                        Err(e) => self.hint = format!("{}", e),
                    }
                }
                if self.service_on {
                    ui.label(
                        egui::RichText::new("Dienst laeuft - auch ohne angemeldeten Benutzer")
                            .color(GREEN)
                            .size(11.0),
                    );
                }
            });

            // ---------------- rechts: Computer fernsteuern ----------------
            let ui = &mut cols[1];
            section(ui, "Computer fernsteuern");
            card(ui, |ui| {
                ui.label(egui::RichText::new("Partner-ID").color(MUTED).size(12.0));
                ui.horizontal(|ui| {
                    ui.add(
                        egui::TextEdit::singleline(&mut self.partner_id)
                            .font(egui::TextStyle::Monospace)
                            .desired_width(190.0)
                            .hint_text("123 456 789"),
                    );
                    egui::ComboBox::from_id_salt("letzte_ids")
                        .selected_text("")
                        .width(28.0)
                        .show_ui(ui, |ui| {
                            for p in self.book.sorted().iter().take(10) {
                                if ui.selectable_label(false, p.label()).clicked() {
                                    self.partner_id = p.id.clone();
                                    if let Some(pw) = self.book.password(&p.id) {
                                        self.partner_pw = pw;
                                        self.remember_pw = true;
                                    }
                                }
                            }
                        });
                });
                ui.add_space(8.0);
                ui.label(egui::RichText::new("Passwort").color(MUTED).size(12.0));
                ui.add(
                    egui::TextEdit::singleline(&mut self.partner_pw)
                        .password(true)
                        .desired_width(190.0)
                        .hint_text("leer lassen fuer Anfrage"),
                );
                ui.checkbox(&mut self.remember_pw, "Passwort merken");
                ui.add_space(12.0);
                ui.horizontal(|ui| {
                    if accent_button(ui, "Verbinden", !connecting).clicked() {
                        self.start_session();
                    }
                    if ui
                        .add_enabled(!connecting, egui::Button::new("Bestaetigung anfordern"))
                        .on_hover_text(
                            "Ohne Passwort verbinden: die Person am anderen Rechner bekommt eine \
                             Anfrage und muss sie zulassen.",
                        )
                        .clicked()
                    {
                        self.start_ask_session();
                    }
                });
                ui.add_space(6.0);
                ui.horizontal(|ui| {
                    let mut game = self.shared.mode.load(Ordering::Relaxed) == proto::MODE_GAME;
                    if ui
                        .checkbox(&mut game, "Spielmodus")
                        .on_hover_text(
                            "Rohe relative Maus, ganze Tastatur, 60 fps - fuer Spiele. \
                             Sonst Fernwartung mit scharfem Bild.",
                        )
                        .changed()
                    {
                        self.shared.mode.store(
                            if game {
                                proto::MODE_GAME
                            } else {
                                proto::MODE_ADMIN
                            },
                            Ordering::Relaxed,
                        );
                    }
                });
                if !viewer_status.is_empty() {
                    ui.add_space(8.0);
                    ui.label(egui::RichText::new(viewer_status).color(MUTED).size(12.0));
                }
            });

            ui.add_space(14.0);
            section(ui, "Letzte Verbindungen");
            card(ui, |ui| {
                let list = self.book.sorted();
                if list.is_empty() {
                    ui.label(
                        egui::RichText::new("Noch niemand - oben eine ID eingeben.")
                            .color(MUTED)
                            .size(12.0),
                    );
                }
                let mut go: Option<String> = None;
                for p in list.iter().take(4) {
                    let online = self.presence.online(&p.id);
                    ui.horizontal(|ui| {
                        dot(ui, online);
                        let label = ui.add(
                            egui::Label::new(egui::RichText::new(p.label()).strong())
                                .sense(egui::Sense::click()),
                        );
                        if label.clicked() {
                            self.partner_id = p.id.clone();
                            if let Some(pw) = self.book.password(&p.id) {
                                self.partner_pw = pw;
                                self.remember_pw = true;
                            }
                        }
                        if label.double_clicked() {
                            go = Some(p.id.clone());
                        }
                        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                            if ui.small_button("verbinden").clicked() {
                                go = Some(p.id.clone());
                            }
                            ui.label(
                                egui::RichText::new(if online { "Online" } else { "Offline" })
                                    .color(if online { GREEN } else { MUTED })
                                    .size(11.0),
                            );
                        });
                    });
                }
                if let Some(id) = go {
                    self.partner_id = id.clone();
                    if let Some(pw) = self.book.password(&id) {
                        self.partner_pw = pw;
                        self.remember_pw = true;
                    }
                    if !connecting {
                        self.start_session();
                    }
                }
            });
        });

        if !self.hint.is_empty() {
            ui.add_space(10.0);
            ui.label(egui::RichText::new(&self.hint).color(ACCENT).size(12.0));
        }
    }

    /// Geraeteliste mit Detailspalte - der Aufbau aus TeamViewer.
    fn devices_view(&mut self, ui: &mut egui::Ui) {
        let connecting = self.shared.connecting.load(Ordering::Relaxed);
        let list = self.book.sorted();
        self.presence
            .watch(list.iter().map(|p| p.id.clone()).collect());

        let mut connect_now: Option<String> = None;
        let mut ask_now: Option<String> = None;
        let mut fav: Option<String> = None;
        let mut del: Option<String> = None;

        let detail_w = 320.0_f32.min(ui.available_width() * 0.45);
        let list_w = (ui.available_width() - detail_w - 16.0).max(220.0);

        ui.horizontal_top(|ui| {
            // ---------------- Liste ----------------
            ui.vertical(|ui| {
                ui.set_width(list_w);
                ui.horizontal(|ui| {
                    ui.add(
                        egui::TextEdit::singleline(&mut self.search)
                            .desired_width(list_w - 90.0)
                            .hint_text("Suchen (Name oder ID)"),
                    );
                    if ui.small_button("neu").on_hover_text("ID von Hand eintragen").clicked() {
                        self.view = View::Start;
                    }
                });
                ui.add_space(6.0);

                let needle = self.search.to_lowercase();
                let matches = |p: &partners::Partner| -> bool {
                    needle.is_empty()
                        || p.label().to_lowercase().contains(&needle)
                        || p.id.contains(&needle)
                };
                let now_online = |id: &str| self.presence.online(id);

                let recent: Vec<_> = list
                    .iter()
                    .filter(|p| p.last > 0 && matches(p))
                    .take(5)
                    .cloned()
                    .collect();
                let favs: Vec<_> = list
                    .iter()
                    .filter(|p| p.favorite && matches(p))
                    .cloned()
                    .collect();
                let online: Vec<_> = list
                    .iter()
                    .filter(|p| !p.favorite && now_online(&p.id) && matches(p))
                    .cloned()
                    .collect();
                let offline: Vec<_> = list
                    .iter()
                    .filter(|p| !p.favorite && !now_online(&p.id) && matches(p))
                    .cloned()
                    .collect();

                egui::ScrollArea::vertical()
                    .auto_shrink([false, false])
                    .show(ui, |ui| {
                        for (title, group) in [
                            ("Letzte Verbindungen", &recent),
                            ("Meine Computer", &favs),
                            ("Online", &online),
                            ("Offline", &offline),
                        ] {
                            if group.is_empty() {
                                continue;
                            }
                            ui.add_space(6.0);
                            ui.label(
                                egui::RichText::new(format!("{}  ({})", title, group.len()))
                                    .color(MUTED)
                                    .size(12.0)
                                    .strong(),
                            );
                            for p in group.iter() {
                                let selected = self.selected.as_deref() == Some(p.id.as_str());
                                let online = self.presence.online(&p.id);
                                let row = egui::Frame::group(ui.style())
                                    .fill(if selected { ROW_SEL } else { CARD })
                                    .stroke(egui::Stroke::new(
                                        1.0,
                                        if selected { ACCENT } else { LINE },
                                    ))
                                    .corner_radius(10)
                                    .inner_margin(egui::Margin::symmetric(10, 7))
                                    .show(ui, |ui| {
                                        ui.horizontal(|ui| {
                                            dot(ui, online);
                                            ui.label(egui::RichText::new(p.label()).strong());
                                            ui.with_layout(
                                                egui::Layout::right_to_left(egui::Align::Center),
                                                |ui| {
                                                    ui.label(
                                                        egui::RichText::new(if online {
                                                            "Online"
                                                        } else {
                                                            "Offline"
                                                        })
                                                        .color(if online { GREEN } else { MUTED })
                                                        .size(11.0),
                                                    );
                                                    ui.label(
                                                        egui::RichText::new(partners::pretty_id(
                                                            &p.id,
                                                        ))
                                                        .monospace()
                                                        .color(MUTED)
                                                        .size(12.0),
                                                    );
                                                },
                                            );
                                        });
                                    });
                                let resp = row.response.interact(egui::Sense::click());
                                if resp.clicked() {
                                    self.selected = Some(p.id.clone());
                                    self.partner_id = p.id.clone();
                                    if let Some(pw) = self.book.password(&p.id) {
                                        self.partner_pw = pw;
                                        self.remember_pw = true;
                                    }
                                }
                                if resp.double_clicked() {
                                    connect_now = Some(p.id.clone());
                                }
                            }
                        }
                        if recent.is_empty() && favs.is_empty() && online.is_empty() && offline.is_empty()
                        {
                            ui.add_space(20.0);
                            ui.label(
                                egui::RichText::new(
                                    "Noch keine Geraete. Verbinde dich einmal, dann steht der Partner hier.",
                                )
                                .color(MUTED),
                            );
                        }
                    });
            });

            ui.add_space(12.0);

            // ---------------- Detailspalte ----------------
            ui.vertical(|ui| {
                ui.set_width(detail_w);
                let sel = self
                    .selected
                    .clone()
                    .and_then(|id| self.book.get(&id).cloned());
                let Some(p) = sel else {
                    card(ui, |ui| {
                        ui.label(
                            egui::RichText::new("Kein Geraet ausgewaehlt")
                                .strong()
                                .size(15.0),
                        );
                        ui.label(
                            egui::RichText::new("Links ein Geraet anklicken.")
                                .color(MUTED)
                                .size(12.0),
                        );
                    });
                    return;
                };
                let online = self.presence.online(&p.id);
                let info = self.presence.get(&p.id);

                card(ui, |ui| {
                    ui.horizontal(|ui| {
                        dot(ui, online);
                        ui.label(egui::RichText::new(p.label()).strong().size(17.0));
                    });
                    ui.label(
                        egui::RichText::new(partners::pretty_id(&p.id))
                            .monospace()
                            .color(MUTED),
                    );
                    ui.add_space(10.0);
                    ui.label(egui::RichText::new("Passwort").color(MUTED).size(12.0));
                    ui.add(
                        egui::TextEdit::singleline(&mut self.partner_pw)
                            .password(true)
                            .desired_width(detail_w - 40.0),
                    );
                    ui.checkbox(&mut self.remember_pw, "merken");
                    ui.add_space(10.0);
                    if accent_button(ui, "Verbinden", !connecting && online).clicked() {
                        connect_now = Some(p.id.clone());
                    }
                    ui.add_space(4.0);
                    if ui
                        .add_enabled(
                            !connecting && online,
                            egui::Button::new("Bestaetigung anfordern"),
                        )
                        .on_hover_text(
                            "Sendet eine Anfrage an den Benutzer am anderen Rechner - kein Passwort noetig.",
                        )
                        .clicked()
                    {
                        ask_now = Some(p.id.clone());
                    }
                    if !online {
                        ui.label(
                            egui::RichText::new("Geraet ist offline")
                                .color(MUTED)
                                .size(11.0),
                        );
                    }
                });

                ui.add_space(10.0);
                section(ui, "Geraeteinformationen");
                card(ui, |ui| {
                    let name_from_relay = info
                        .as_ref()
                        .map(|i| i.name.clone())
                        .unwrap_or_default();
                    info_row(ui, "Name", &p.label());
                    if !name_from_relay.is_empty() && name_from_relay != p.label() {
                        info_row(ui, "Meldet sich als", &name_from_relay);
                    }
                    info_row(ui, "FreeViewer-ID", &partners::pretty_id(&p.id));
                    info_row(
                        ui,
                        "Zuletzt online",
                        &info
                            .as_ref()
                            .map(|i| presence::ago_ms(i.seen))
                            .unwrap_or_else(|| "unbekannt".to_string()),
                    );
                    info_row(ui, "Zuletzt verbunden", &p.ago());
                    info_row(ui, "Verbindungen", &format!("{}", p.count));
                    info_row(ui, "Gesamtdauer", &p.total());
                    info_row(
                        ui,
                        "Passwort",
                        if p.secret.is_some() {
                            "gespeichert"
                        } else {
                            "--"
                        },
                    );
                });

                ui.add_space(10.0);
                card(ui, |ui| {
                    ui.horizontal(|ui| {
                        if let Some((rid, buf)) = self.renaming.as_mut() {
                            if rid == &p.id {
                                let r = ui.add(
                                    egui::TextEdit::singleline(buf).desired_width(160.0),
                                );
                                let done = ui.small_button("speichern").clicked()
                                    || (r.lost_focus()
                                        && ui.input(|i| i.key_pressed(egui::Key::Enter)));
                                if done {
                                    let (id, name) = (rid.clone(), buf.clone());
                                    self.book.rename(&id, &name);
                                    self.renaming = None;
                                }
                                return;
                            }
                        }
                        if ui.button("Umbenennen").clicked() {
                            self.renaming = Some((p.id.clone(), p.name.clone()));
                        }
                        if ui
                            .button(if p.favorite {
                                "Nicht mehr anheften"
                            } else {
                                "Anheften"
                            })
                            .clicked()
                        {
                            fav = Some(p.id.clone());
                        }
                        if ui.button("Entfernen").clicked() {
                            del = Some(p.id.clone());
                        }
                    });
                });
            });
        });

        if let Some(id) = fav {
            self.book.toggle_favorite(&id);
        }
        if let Some(id) = del {
            self.book.remove(&id);
            if self.selected.as_deref() == Some(id.as_str()) {
                self.selected = None;
            }
        }
        if let Some(id) = connect_now {
            self.partner_id = id.clone();
            if let Some(pw) = self.book.password(&id) {
                self.partner_pw = pw;
                self.remember_pw = true;
            }
            if !connecting {
                self.start_session();
            }
        }
        if let Some(id) = ask_now {
            self.partner_id = id;
            if !connecting {
                self.start_ask_session();
            }
        }
    }

    fn settings_view(&mut self, ui: &mut egui::Ui) {
        section(ui, "Dieser Computer");
        card(ui, |ui| {
            ui.horizontal(|ui| {
                ui.label(egui::RichText::new("Name").color(MUTED).size(12.0));
                let mut name = self.shared.device_name.lock().unwrap().clone();
                if ui
                    .add(
                        egui::TextEdit::singleline(&mut name)
                            .desired_width(220.0)
                            .hint_text("wie dieser PC in fremden Listen heisst"),
                    )
                    .changed()
                {
                    let clean = presence::clean(&name);
                    *self.shared.device_name.lock().unwrap() = clean.clone();
                    let _ = presence::save_device_name(&clean);
                }
            });
            info_row(ui, "Konfiguration", &ident::config_dir().display().to_string());
            info_row(ui, "Relay", &self.shared.relay_url);
            info_row(ui, "Version", update::VERSION);
        });

        ui.add_space(12.0);
        section(ui, "Start und Zugriff");
        card(ui, |ui| {
            let mut boot = self.autostart;
            if ui
                .checkbox(&mut boot, "Mit Windows starten (unsichtbar im Infobereich)")
                .changed()
            {
                match autostart::set(boot) {
                    Ok(()) => self.autostart = boot,
                    Err(e) => self.hint = format!("Autostart ging nicht: {}", e),
                }
            }
            let mut svc = self.service_on;
            if ui
                .checkbox(
                    &mut svc,
                    "Dienst: auch am Sperr- und Anmeldebildschirm erreichbar",
                )
                .changed()
            {
                let flag = if svc {
                    "--install-service"
                } else {
                    "--uninstall-service"
                };
                match service::elevate(flag) {
                    Ok(()) => self.hint = "Bitte die Windows-Abfrage bestaetigen...".to_string(),
                    Err(e) => self.hint = format!("{}", e),
                }
            }
        });

        ui.add_space(12.0);
        section(ui, "Aktualisierung");
        card(ui, |ui| {
            self.update_ui(ui);
        });

        if !self.hint.is_empty() {
            ui.add_space(10.0);
            ui.label(egui::RichText::new(&self.hint).color(ACCENT).size(12.0));
        }
    }

    /// Der Dialog, den TeamViewer "Bestaetigung anfordern" nennt - hier die
    /// Seite, die gefragt wird.
    fn knock_ui(&mut self, ctx: &egui::Context) {
        let knock = self.shared.knock.lock().unwrap().clone();
        let Some(k) = knock else {
            return;
        };
        let left = 60_u64.saturating_sub(k.at.elapsed().as_secs());
        egui::Window::new("Verbindungsanfrage")
            .collapsible(false)
            .resizable(false)
            .anchor(egui::Align2::CENTER_CENTER, [0.0, 0.0])
            .show(ctx, |ui| {
                ui.add_space(4.0);
                ui.label(
                    egui::RichText::new(format!("\"{}\" moechte sich verbinden.", k.from))
                        .size(15.0)
                        .strong(),
                );
                ui.add_space(6.0);
                ui.label(
                    egui::RichText::new(format!("Sitzungscode {}", k.code))
                        .monospace()
                        .color(ACCENT)
                        .size(18.0),
                );
                ui.label(
                    egui::RichText::new(
                        "Lass dir den Code nennen - stimmt er ueberein, redet ihr wirklich miteinander.",
                    )
                    .color(MUTED)
                    .size(11.0),
                );
                ui.add_space(10.0);
                ui.horizontal(|ui| {
                    if accent_button(ui, "Zulassen", true).clicked() {
                        self.shared.knock_answer.store(1, Ordering::Relaxed);
                    }
                    if ui.button("Ablehnen").clicked() {
                        self.shared.knock_answer.store(2, Ordering::Relaxed);
                    }
                    ui.label(
                        egui::RichText::new(format!("noch {} s", left))
                            .color(MUTED)
                            .size(11.0),
                    );
                });
            });
        ctx.request_repaint_after(Duration::from_millis(250));
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

    /// The duplication API does not paint the cursor into the frame. In game
    /// mode the local pointer is locked and invisible, so the remote one has
    /// to be drawn - in remote maintenance the local pointer sits exactly
    /// where the remote one is and a second arrow only confuses.
    fn draw_remote_cursor(&self, ctx: &egui::Context, rect: egui::Rect, game: bool) {
        if !game {
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
        self.knock_ui(ctx);
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

// --------------------------------------------------------------- Aussehen --

/// Which page the window shows.
#[derive(Clone, Copy, PartialEq, Eq)]
enum View {
    Start,
    Devices,
    Settings,
}

const BG: egui::Color32 = egui::Color32::from_rgb(0x0b, 0x0f, 0x1a);
const CARD: egui::Color32 = egui::Color32::from_rgb(0x14, 0x1b, 0x2c);
const ROW_SEL: egui::Color32 = egui::Color32::from_rgb(0x1b, 0x26, 0x3d);
const FIELD: egui::Color32 = egui::Color32::from_rgb(0x0e, 0x14, 0x22);
const LINE: egui::Color32 = egui::Color32::from_rgb(0x25, 0x2e, 0x45);
const ACCENT: egui::Color32 = egui::Color32::from_rgb(0x38, 0xbd, 0xf8);
const GREEN: egui::Color32 = egui::Color32::from_rgb(0x22, 0xc5, 0x5e);
const MUTED: egui::Color32 = egui::Color32::from_rgb(0x8b, 0x95, 0xab);
const TEXT: egui::Color32 = egui::Color32::from_rgb(0xe7, 0xeb, 0xf3);

/// Dark FreeViewer look: same palette as the product page.
fn install_theme(ctx: &egui::Context) {
    let mut style = (*ctx.style()).clone();
    {
        let v = &mut style.visuals;
        v.dark_mode = true;
        v.panel_fill = BG;
        v.window_fill = CARD;
        v.extreme_bg_color = FIELD;
        v.faint_bg_color = CARD;
        v.override_text_color = Some(TEXT);
        v.hyperlink_color = ACCENT;
        v.selection.bg_fill = ACCENT.gamma_multiply(0.35);
        v.selection.stroke = egui::Stroke::new(1.0, TEXT);
        v.window_stroke = egui::Stroke::new(1.0, LINE);
        v.widgets.noninteractive.bg_fill = CARD;
        v.widgets.noninteractive.bg_stroke = egui::Stroke::new(1.0, LINE);
        v.widgets.inactive.weak_bg_fill = CARD;
        v.widgets.inactive.bg_stroke = egui::Stroke::new(1.0, LINE);
        v.widgets.hovered.weak_bg_fill = ROW_SEL;
        v.widgets.hovered.bg_stroke = egui::Stroke::new(1.0, ACCENT);
        v.widgets.active.weak_bg_fill = ROW_SEL;
        v.widgets.active.bg_stroke = egui::Stroke::new(1.0, ACCENT);
    }
    style.spacing.item_spacing = egui::vec2(8.0, 8.0);
    style.spacing.button_padding = egui::vec2(12.0, 6.0);
    style.spacing.interact_size.y = 26.0;
    ctx.set_style(style);
}

/// Small headline above a card.
fn section(ui: &mut egui::Ui, title: &str) {
    ui.label(
        egui::RichText::new(title)
            .size(15.0)
            .strong()
            .color(egui::Color32::from_rgb(0xc9, 0xd2, 0xe3)),
    );
    ui.add_space(4.0);
}

/// One rounded panel.
fn card<R>(ui: &mut egui::Ui, add: impl FnOnce(&mut egui::Ui) -> R) -> R {
    egui::Frame::group(ui.style())
        .fill(CARD)
        .stroke(egui::Stroke::new(1.0, LINE))
        .corner_radius(12)
        .inner_margin(egui::Margin::same(14))
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            add(ui)
        })
        .inner
}

/// The one blue button per card.
fn accent_button(ui: &mut egui::Ui, text: &str, enabled: bool) -> egui::Response {
    let btn = egui::Button::new(egui::RichText::new(text).strong().color(BG))
        .fill(if enabled { ACCENT } else { LINE })
        .corner_radius(8)
        .min_size(egui::vec2(120.0, 30.0));
    ui.add_enabled(enabled, btn)
}

/// Green or grey dot in front of a device.
fn dot(ui: &mut egui::Ui, online: bool) {
    let (rect, _) = ui.allocate_exact_size(egui::vec2(10.0, 10.0), egui::Sense::hover());
    ui.painter().circle_filled(
        rect.center(),
        4.5,
        if online { GREEN } else { MUTED.gamma_multiply(0.6) },
    );
}

/// State of our own machine, top right.
fn status_pill(ui: &mut egui::Ui, ok: bool, text: &str) {
    egui::Frame::group(ui.style())
        .fill(if ok {
            GREEN.gamma_multiply(0.15)
        } else {
            LINE
        })
        .stroke(egui::Stroke::new(1.0, if ok { GREEN } else { LINE }))
        .corner_radius(10)
        .inner_margin(egui::Margin::symmetric(10, 3))
        .show(ui, |ui| {
            ui.label(
                egui::RichText::new(text)
                    .size(11.0)
                    .color(if ok { GREEN } else { MUTED }),
            );
        });
}

/// "Schluessel   Wert" line inside the info card.
fn info_row(ui: &mut egui::Ui, key: &str, value: &str) {
    ui.horizontal(|ui| {
        ui.label(egui::RichText::new(key).color(MUTED).size(11.0));
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            ui.add(
                egui::Label::new(egui::RichText::new(value).size(12.0))
                    .selectable(true)
                    .truncate(),
            );
        });
    });
}