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

    // Bild der eigenen Oberflaeche:
    //   freeviewer --shot C:\bild.jpg [--view start|devices|settings] [--demo]
    // Die App zeichnet sich selbst und speichert das Ergebnis - das klappt auch
    // dann, wenn der Bildschirm gesperrt ist und ein Fensterfoto nur weiss waere.
    if let Some(i) = std::env::args().position(|a| a == "--shot") {
        let path = std::env::args()
            .nth(i + 1)
            .unwrap_or_else(|| "freeviewer-shot.jpg".to_string());
        let want = std::env::args().skip_while(|a| a != "--view").nth(1);
        let demo = std::env::args().any(|a| a == "--demo");
        let knock = std::env::args().any(|a| a == "--knock");
        let shot_shared = shared.clone();
        let options = eframe::NativeOptions {
            viewport: egui::ViewportBuilder::default()
                .with_inner_size([1180.0, 800.0])
                .with_visible(true)
                .with_icon(app_icon())
                .with_title("FreeViewer"),
            ..Default::default()
        };
        return eframe::run_native(
            "FreeViewer",
            options,
            Box::new(move |cc| {
                install_theme(&cc.egui_ctx);
                let mut app = App::new(shot_shared.clone(), false);
                if demo {
                    app.book.started("123456789", "geheim", true);
                    app.book.rename("123456789", "Buero-PC");
                    app.book.started("987654321", "", false);
                    app.selected = Some("123456789".to_string());
                    app.partner_id = "123456789".to_string();
                    *shot_shared.my_id.lock().unwrap() = "497628420".to_string();
                }
                if knock {
                    *shot_shared.knock.lock().unwrap() = Some(shared::Knock {
                        from: "Laptop von Justin".to_string(),
                        code: "1234".to_string(),
                        at: std::time::Instant::now(),
                    });
                }
                app.view = match want.as_deref() {
                    Some("devices") => View::Devices,
                    Some("settings") => View::Settings,
                    _ => View::Start,
                };
                app.shot = Some(std::path::PathBuf::from(path));
                Ok(Box::new(app))
            }),
        );
    }

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
            .with_icon(app_icon())
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
    /// --shot: wohin das Bild der eigenen Oberflaeche geschrieben wird.
    shot: Option<std::path::PathBuf>,
    /// Gezeichnete Frames seit dem Start im --shot-Modus.
    shot_n: u32,
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
            shot: None,
            shot_n: 0,
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

    /// Zeile fuer die Selbstaktualisierung.
    fn update_ui(&mut self, ui: &mut egui::Ui) {
        let pending = self.shared.update.lock().unwrap().clone();
        let status = self.shared.update_status.lock().unwrap().clone();
        ui.horizontal(|ui| {
            let mut auto = self.shared.auto_update.load(Ordering::Relaxed);
            if ui.checkbox(&mut auto, "Automatisch aktualisieren").changed() {
                self.shared.auto_update.store(auto, Ordering::Relaxed);
                ident::set_auto_update(auto);
            }
            if let Some(rel) = pending {
                if ghost_button(ui, &format!("Update {} installieren", rel.version)).clicked() {
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
        });
        if !status.is_empty() {
            ui.add_space(2.0);
            ui.label(egui::RichText::new(status).color(MUTED).size(11.5));
        }
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
    // ----------------------------------------------------------- Oberfläche

    /// Kopfzeile: Marke links, die drei Bereiche als Pillen, Status rechts.
    fn top_bar(&mut self, ui: &mut egui::Ui) {
        let my_id = self.shared.my_id.lock().unwrap().clone();
        let online = !my_id.is_empty();
        ui.horizontal(|ui| {
            ui.add_space(2.0);
            logo(ui);
            ui.add_space(2.0);
            ui.vertical(|ui| {
                ui.add_space(2.0);
                ui.label(egui::RichText::new("FreeViewer").size(17.0).strong());
                ui.label(
                    egui::RichText::new(format!("Version {}", update::VERSION))
                        .size(11.0)
                        .color(MUTED),
                );
            });
            ui.add_space(10.0);
            for (v, name) in [
                (View::Start, "Start"),
                (View::Devices, "Geräte"),
                (View::Settings, "Einstellungen"),
            ] {
                if tab(ui, name, self.view == v).clicked() {
                    self.view = v;
                }
            }
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                status_pill(ui, online, if online { "Bereit" } else { "Verbinde …" });
            });
        });
        ui.add_space(6.0);
        let line = ui.available_rect_before_wrap();
        ui.painter().hline(
            line.left()..=line.right(),
            line.top(),
            egui::Stroke::new(1.0, LINE),
        );
        ui.add_space(8.0);
    }

    fn home_ui(&mut self, ui: &mut egui::Ui) {
        self.top_bar(ui);
        match self.view {
            View::Start => self.start_view(ui),
            View::Devices => self.devices_view(ui),
            View::Settings => self.settings_view(ui),
        }
    }

    /// Startseite: links dieser PC, rechts der Weg nach draußen.
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
                label_small(ui, "Ihre ID");
                ui.horizontal(|ui| {
                    let id_text = if my_id.len() == 9 {
                        partners::pretty_id(&my_id)
                    } else {
                        "— — —".to_string()
                    };
                    ui.add(
                        egui::Label::new(
                            egui::RichText::new(id_text).size(27.0).strong().color(TEXT),
                        )
                        .selectable(true),
                    );
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        if !my_id.is_empty() && ghost_button(ui, "kopieren").clicked() {
                            ui.ctx().copy_text(partners::pretty_id(&my_id));
                            self.hint = "ID kopiert".to_string();
                        }
                    });
                });

                ui.add_space(6.0);
                label_small(ui, "Passwort");
                let mut changed = false;
                ui.horizontal(|ui| {
                    {
                        let mut pw = self.shared.password.lock().unwrap();
                        changed = ui
                            .add(
                                egui::TextEdit::singleline(&mut *pw)
                                    .font(egui::FontId::new(19.0, egui::FontFamily::Monospace))
                                    .desired_width(160.0)
                                    .margin(egui::Margin::symmetric(8, 4)),
                            )
                            .changed();
                    }
                    if ghost_button(ui, "kopieren").clicked() {
                        let pw = self.shared.password.lock().unwrap().clone();
                        ui.ctx().copy_text(pw);
                        self.hint = "Passwort kopiert".to_string();
                    }
                    if ghost_button(ui, "neu")
                        .on_hover_text("Neues Zufallspasswort erzeugen")
                        .clicked()
                    {
                        *self.shared.password.lock().unwrap() = ident::random_password();
                        changed = true;
                    }
                });
                ui.add_space(4.0);
                if ui
                    .checkbox(&mut self.pw_fixed, "Passwort behalten")
                    .on_hover_text(
                        "An: das Passwort bleibt nach einem Neustart gleich – nötig für unbeaufsichtigten Zugriff.\nAus: bei jedem Start ein neues.",
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

                ui.add_space(7.0);
                divider(ui);
                ui.add_space(6.0);
                ui.label(egui::RichText::new(host_status).size(12.5).color(MUTED));
                ui.label(egui::RichText::new(host_peer).size(12.5).color(MUTED));
            });

            ui.add_space(8.0);
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
                ui.add_space(2.0);
                let mut svc = self.service_on;
                if ui
                    .checkbox(&mut svc, "Einfachen Zugriff gewähren (Dienst)")
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
                        Ok(()) => self.hint = "Bitte die Windows-Abfrage bestätigen …".to_string(),
                        Err(e) => self.hint = format!("{}", e),
                    }
                }
                if self.service_on {
                    ui.add_space(4.0);
                    ui.label(
                        egui::RichText::new("Dienst läuft – auch ohne angemeldeten Benutzer")
                            .color(GREEN)
                            .size(11.5),
                    );
                }
            });

            // ---------------- rechts: Computer fernsteuern ----------------
            let ui = &mut cols[1];
            section(ui, "Computer fernsteuern");
            card(ui, |ui| {
                label_small(ui, "Partner-ID");
                ui.horizontal(|ui| {
                    ui.add(
                        egui::TextEdit::singleline(&mut self.partner_id)
                            .font(egui::FontId::new(19.0, egui::FontFamily::Monospace))
                            .desired_width(210.0)
                            .margin(egui::Margin::symmetric(8, 4))
                            .hint_text("123 456 789"),
                    );
                    let list = self.book.sorted();
                    if !list.is_empty() {
                        egui::ComboBox::from_id_salt("letzte_ids")
                            .selected_text("zuletzt")
                            .width(90.0)
                            .show_ui(ui, |ui| {
                                for p in list.iter().take(10) {
                                    if ui.selectable_label(false, p.label()).clicked() {
                                        self.partner_id = p.id.clone();
                                        if let Some(pw) = self.book.password(&p.id) {
                                            self.partner_pw = pw;
                                            self.remember_pw = true;
                                        }
                                    }
                                }
                            });
                    }
                });

                ui.add_space(7.0);
                label_small(ui, "Passwort");
                ui.add(
                    egui::TextEdit::singleline(&mut self.partner_pw)
                        .password(true)
                        .desired_width(210.0)
                        .margin(egui::Margin::symmetric(8, 4))
                        .hint_text("leer lassen für Anfrage"),
                );
                ui.add_space(2.0);
                ui.checkbox(&mut self.remember_pw, "Passwort merken");

                ui.add_space(8.0);
                if accent_button(ui, "Verbinden", !connecting).clicked() {
                    self.start_session();
                }
                ui.add_space(5.0);
                if ui
                    .add_enabled(!connecting, ghost(egui::vec2(200.0, 30.0), "Bestätigung anfordern"))
                    .on_hover_text(
                        "Ohne Passwort verbinden: die Person am anderen Rechner bekommt eine \
                         Anfrage und muss sie zulassen.",
                    )
                    .clicked()
                {
                    self.start_ask_session();
                }

                ui.add_space(7.0);
                let mut game = self.shared.mode.load(Ordering::Relaxed) == proto::MODE_GAME;
                if ui
                    .checkbox(&mut game, "Spielmodus (rohe Maus, ganze Tastatur)")
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
                if !viewer_status.is_empty() {
                    ui.add_space(6.0);
                    divider(ui);
                    ui.add_space(5.0);
                    ui.label(egui::RichText::new(viewer_status).size(12.5).color(MUTED));
                }
            });

            ui.add_space(8.0);
            section(ui, "Letzte Verbindungen");
            card(ui, |ui| {
                let list = self.book.sorted();
                if list.is_empty() {
                    ui.label(
                        egui::RichText::new("Noch niemand – oben eine ID eingeben.")
                            .color(MUTED)
                            .size(12.5),
                    );
                }
                let mut go: Option<String> = None;
                for (i, p) in list.iter().take(4).enumerate() {
                    if i > 0 {
                        ui.add_space(2.0);
                    }
                    let online = self.presence.online(&p.id);
                    ui.horizontal(|ui| {
                        dot(ui, online);
                        ui.add_space(2.0);
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
                            if ghost_button(ui, "verbinden").clicked() {
                                go = Some(p.id.clone());
                            }
                            ui.label(
                                egui::RichText::new(if online { "Online" } else { "Offline" })
                                    .color(if online { GREEN } else { MUTED })
                                    .size(11.5),
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
            ui.add_space(6.0);
            ui.label(egui::RichText::new(&self.hint).color(ACCENT).size(12.5));
        }
    }

    /// Geräteliste mit Detailspalte – der Aufbau aus TeamViewer.
    fn devices_view(&mut self, ui: &mut egui::Ui) {
        let connecting = self.shared.connecting.load(Ordering::Relaxed);
        let list = self.book.sorted();
        self.presence
            .watch(list.iter().map(|p| p.id.clone()).collect());

        let mut connect_now: Option<String> = None;
        let mut ask_now: Option<String> = None;
        let mut fav: Option<String> = None;
        let mut del: Option<String> = None;

        let detail_w = 340.0_f32.min(ui.available_width() * 0.42);
        let list_w = (ui.available_width() - detail_w - 20.0).max(240.0);

        ui.horizontal_top(|ui| {
            // ---------------- Liste ----------------
            ui.vertical(|ui| {
                ui.set_width(list_w);
                ui.add(
                    egui::TextEdit::singleline(&mut self.search)
                        .desired_width(list_w)
                        .margin(egui::Margin::symmetric(8, 5))
                        .hint_text("Suchen (Name oder ID)"),
                );
                ui.add_space(6.0);

                let needle = self.search.to_lowercase();
                let matches = |p: &partners::Partner| -> bool {
                    needle.is_empty()
                        || p.label().to_lowercase().contains(&needle)
                        || p.id.contains(&needle)
                };

                let recent: Vec<_> = list
                    .iter()
                    .filter(|p| p.last > 0 && matches(p))
                    .take(5)
                    .cloned()
                    .collect();
                let shown: std::collections::HashSet<String> =
                    recent.iter().map(|p| p.id.clone()).collect();
                let favs: Vec<_> = list
                    .iter()
                    .filter(|p| p.favorite && !shown.contains(&p.id) && matches(p))
                    .cloned()
                    .collect();
                let online: Vec<_> = list
                    .iter()
                    .filter(|p| {
                        !p.favorite
                            && !shown.contains(&p.id)
                            && self.presence.online(&p.id)
                            && matches(p)
                    })
                    .cloned()
                    .collect();
                let offline: Vec<_> = list
                    .iter()
                    .filter(|p| {
                        !p.favorite
                            && !shown.contains(&p.id)
                            && !self.presence.online(&p.id)
                            && matches(p)
                    })
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
                                egui::RichText::new(format!("{}  ·  {}", title, group.len()))
                                    .color(MUTED)
                                    .size(11.5)
                                    .strong(),
                            );
                            ui.add_space(2.0);
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
                                    .inner_margin(egui::Margin::symmetric(11, 6))
                                    .outer_margin(egui::Margin::symmetric(0, 2))
                                    .show(ui, |ui| {
                                        ui.set_width(ui.available_width());
                                        ui.horizontal(|ui| {
                                            dot(ui, online);
                                            ui.add_space(2.0);
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
                                                        .size(11.5),
                                                    );
                                                    ui.add_space(6.0);
                                                    ui.label(
                                                        egui::RichText::new(partners::pretty_id(
                                                            &p.id,
                                                        ))
                                                        .monospace()
                                                        .color(MUTED)
                                                        .size(12.5),
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
                        if recent.is_empty()
                            && favs.is_empty()
                            && online.is_empty()
                            && offline.is_empty()
                        {
                            ui.add_space(14.0);
                            ui.vertical_centered(|ui| {
                                ui.label(
                                    egui::RichText::new("Noch keine Geräte")
                                        .strong()
                                        .size(15.0),
                                );
                                ui.label(
                                    egui::RichText::new(
                                        "Verbinde dich einmal – danach steht der Partner hier.",
                                    )
                                    .color(MUTED)
                                    .size(12.5),
                                );
                            });
                        }
                    });
            });

            ui.add_space(8.0);

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
                            egui::RichText::new("Kein Gerät ausgewählt")
                                .strong()
                                .size(15.0),
                        );
                        ui.add_space(2.0);
                        ui.label(
                            egui::RichText::new("Links ein Gerät anklicken.")
                                .color(MUTED)
                                .size(12.5),
                        );
                    });
                    return;
                };
                let online = self.presence.online(&p.id);
                let info = self.presence.get(&p.id);

                card(ui, |ui| {
                    ui.horizontal(|ui| {
                        dot(ui, online);
                        ui.add_space(2.0);
                        ui.label(egui::RichText::new(p.label()).strong().size(17.0));
                    });
                    ui.label(
                        egui::RichText::new(partners::pretty_id(&p.id))
                            .monospace()
                            .size(13.0)
                            .color(MUTED),
                    );

                    ui.add_space(6.0);
                    label_small(ui, "Passwort");
                    ui.add(
                        egui::TextEdit::singleline(&mut self.partner_pw)
                            .password(true)
                            .desired_width(detail_w - 56.0)
                            .margin(egui::Margin::symmetric(8, 4)),
                    );
                    ui.add_space(2.0);
                    ui.checkbox(&mut self.remember_pw, "merken");

                    ui.add_space(6.0);
                    if accent_button(ui, "Verbinden", !connecting && online).clicked() {
                        connect_now = Some(p.id.clone());
                    }
                    ui.add_space(5.0);
                    if ui
                        .add_enabled(
                            !connecting && online,
                            ghost(egui::vec2(detail_w - 56.0, 28.0), "Bestätigung anfordern"),
                        )
                        .on_hover_text(
                            "Sendet eine Anfrage an den Benutzer am anderen Rechner – kein Passwort nötig.",
                        )
                        .clicked()
                    {
                        ask_now = Some(p.id.clone());
                    }
                    if !online {
                        ui.add_space(4.0);
                        ui.label(
                            egui::RichText::new("Gerät ist offline")
                                .color(MUTED)
                                .size(11.5),
                        );
                    }
                });

                ui.add_space(6.0);
                section(ui, "Geräteinformationen");
                card(ui, |ui| {
                    let relay_name = info.as_ref().map(|i| i.name.clone()).unwrap_or_default();
                    info_row(ui, "Name", &p.label());
                    if !relay_name.is_empty() && relay_name != p.label() {
                        info_row(ui, "Meldet sich als", &relay_name);
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
                            "—"
                        },
                    );
                });

                ui.add_space(6.0);
                card(ui, |ui| {
                    ui.horizontal_wrapped(|ui| {
                        if let Some((rid, buf)) = self.renaming.as_mut() {
                            if rid == &p.id {
                                let r = ui.add(
                                    egui::TextEdit::singleline(buf).desired_width(150.0),
                                );
                                let done = ghost_button(ui, "speichern").clicked()
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
                        if ghost_button(ui, "Umbenennen").clicked() {
                            self.renaming = Some((p.id.clone(), p.name.clone()));
                        }
                        if ghost_button(
                            ui,
                            if p.favorite {
                                "Nicht mehr anheften"
                            } else {
                                "Anheften"
                            },
                        )
                        .clicked()
                        {
                            fav = Some(p.id.clone());
                        }
                        if ghost_button(ui, "Entfernen").clicked() {
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
        ui.columns(2, |cols| {
            let ui = &mut cols[0];
            section(ui, "Dieser Computer");
            card(ui, |ui| {
                label_small(ui, "Name in fremden Listen");
                let mut name = self.shared.device_name.lock().unwrap().clone();
                if ui
                    .add(
                        egui::TextEdit::singleline(&mut name)
                            .desired_width(240.0)
                            .margin(egui::Margin::symmetric(8, 4))
                            .hint_text("z. B. Büro-PC"),
                    )
                    .changed()
                {
                    let clean = presence::clean(&name);
                    *self.shared.device_name.lock().unwrap() = clean.clone();
                    let _ = presence::save_device_name(&clean);
                }
                ui.add_space(6.0);
                divider(ui);
                ui.add_space(5.0);
                info_row(
                    ui,
                    "Einstellungen",
                    &ident::config_dir().display().to_string(),
                );
                info_row(ui, "Relay", &self.shared.relay_url);
                info_row(ui, "Version", update::VERSION);
            });

            let ui = &mut cols[1];
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
                ui.add_space(2.0);
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
                        Ok(()) => self.hint = "Bitte die Windows-Abfrage bestätigen …".to_string(),
                        Err(e) => self.hint = format!("{}", e),
                    }
                }
                ui.add_space(7.0);
                divider(ui);
                ui.add_space(5.0);
                self.update_ui(ui);
            });
        });

        if !self.hint.is_empty() {
            ui.add_space(6.0);
            ui.label(egui::RichText::new(&self.hint).color(ACCENT).size(12.5));
        }
    }

    /// Die Seite, die gefragt wird: „X möchte sich verbinden“.
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
            .frame(
                egui::Frame::group(&ctx.style())
                    .fill(CARD)
                    .stroke(egui::Stroke::new(1.0, ACCENT))
                    .corner_radius(14)
                    .inner_margin(egui::Margin::same(10)),
            )
            .show(ctx, |ui| {
                ui.set_min_width(340.0);
                ui.label(
                    egui::RichText::new(format!("„{}“ möchte sich verbinden", k.from))
                        .size(16.0)
                        .strong(),
                );
                ui.add_space(6.0);
                ui.horizontal(|ui| {
                    label_small(ui, "Sitzungscode");
                    ui.label(
                        egui::RichText::new(&k.code)
                            .monospace()
                            .color(ACCENT)
                            .size(22.0)
                            .strong(),
                    );
                });
                ui.label(
                    egui::RichText::new(
                        "Lass dir den Code nennen – stimmt er überein, redet ihr wirklich miteinander.",
                    )
                    .color(MUTED)
                    .size(11.5),
                );
                ui.add_space(6.0);
                ui.horizontal(|ui| {
                    if accent_button(ui, "Zulassen", true).clicked() {
                        self.shared.knock_answer.store(1, Ordering::Relaxed);
                    }
                    if ui.add(ghost(egui::vec2(110.0, 34.0), "Ablehnen")).clicked() {
                        self.shared.knock_answer.store(2, Ordering::Relaxed);
                    }
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        ui.label(
                            egui::RichText::new(format!("noch {} s", left))
                                .color(MUTED)
                                .size(11.5),
                        );
                    });
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
            paint_background(ctx);
            egui::CentralPanel::default()
                .frame(
                    egui::Frame::NONE
                        .fill(egui::Color32::TRANSPARENT)
                        .inner_margin(egui::Margin::symmetric(16, 10)),
                )
                .show(ctx, |ui| {
                    // Inhalt nicht ueber die ganze Breite ziehen
                    let full = ui.available_width();
                    let w = full.min(CONTENT_MAX);
                    let pad = ((full - w) / 2.0).max(0.0);
                    ui.horizontal_top(|ui| {
                        ui.add_space(pad);
                        ui.vertical(|ui| {
                            ui.set_width(w);
                            self.home_ui(ui);
                        });
                    });
                });
            ctx.request_repaint_after(Duration::from_millis(250));
        }

        // --shot: ein paar Frames zeichnen lassen, dann ein Bild anfordern
        if let Some(path) = self.shot.clone() {
            self.shot_n += 1;
            if self.shot_n == 6 {
                ctx.send_viewport_cmd(egui::ViewportCommand::Screenshot(
                    egui::UserData::default(),
                ));
            }
            if self.shot_n > 6 {
                let img = ctx.input(|i| {
                    i.events.iter().rev().find_map(|e| match e {
                        egui::Event::Screenshot { image, .. } => Some(image.clone()),
                        _ => None,
                    })
                });
                if let Some(img) = img {
                    save_shot(&img, &path);
                    std::process::exit(0);
                }
                if self.shot_n > 150 {
                    eprintln!("SHOT: kein Bild bekommen");
                    std::process::exit(2);
                }
            }
            ctx.request_repaint();
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

// Hausstil von fleitec: tiefer Grund, Panels darüber, ein Akzent.
const BG: egui::Color32 = egui::Color32::from_rgb(0x07, 0x09, 0x0f);
const CARD: egui::Color32 = egui::Color32::from_rgb(0x0e, 0x12, 0x20);
const CARD_HI: egui::Color32 = egui::Color32::from_rgb(0x12, 0x17, 0x28);
const ROW_SEL: egui::Color32 = egui::Color32::from_rgb(0x16, 0x1e, 0x36);
const FIELD: egui::Color32 = egui::Color32::from_rgb(0x0a, 0x0e, 0x1a);
const LINE: egui::Color32 = egui::Color32::from_rgb(0x1e, 0x25, 0x38);
const ACCENT: egui::Color32 = egui::Color32::from_rgb(0x38, 0xbd, 0xf8);
const VIOLET: egui::Color32 = egui::Color32::from_rgb(0x8b, 0x5c, 0xf6);
const GREEN: egui::Color32 = egui::Color32::from_rgb(0x22, 0xc5, 0x5e);
const MUTED: egui::Color32 = egui::Color32::from_rgb(0x8b, 0x95, 0xab);
const TEXT: egui::Color32 = egui::Color32::from_rgb(0xe7, 0xeb, 0xf3);

/// Breite, ab der der Inhalt nicht weiter auseinandergezogen wird.
const CONTENT_MAX: f32 = 1180.0;

/// Schriften: die von Windows mitgelieferte Segoe UI sieht auf einem
/// Windows-Rechner richtig aus - die eingebaute egui-Schrift wirkt fremd.
fn install_fonts(ctx: &egui::Context) {
    let mut fonts = egui::FontDefinitions::default();
    let mut body: Option<&str> = None;
    let mut head: Option<&str> = None;
    for (key, files) in [
        (
            "ui",
            [
                "C:/Windows/Fonts/segoeui.ttf",
                "C:/Windows/Fonts/SegUIVar.ttf",
            ],
        ),
        (
            "ui_bold",
            [
                "C:/Windows/Fonts/seguisb.ttf",
                "C:/Windows/Fonts/segoeuib.ttf",
            ],
        ),
    ] {
        for f in files {
            if let Ok(bytes) = std::fs::read(f) {
                fonts.font_data.insert(
                    key.to_owned(),
                    std::sync::Arc::new(egui::FontData::from_owned(bytes)),
                );
                if key == "ui" {
                    body = Some(key);
                } else {
                    head = Some(key);
                }
                break;
            }
        }
    }
    if let Some(b) = body {
        fonts
            .families
            .entry(egui::FontFamily::Proportional)
            .or_default()
            .insert(0, b.to_owned());
    }
    if let Some(h) = head {
        fonts
            .families
            .entry(egui::FontFamily::Name("head".into()))
            .or_default()
            .insert(0, h.to_owned());
    }
    ctx.set_fonts(fonts);
}

/// Kräftige Schrift für Überschriften, sonst die normale.
fn head_font(size: f32) -> egui::FontId {
    egui::FontId::new(size, egui::FontFamily::Name("head".into()))
}

fn install_theme(ctx: &egui::Context) {
    install_fonts(ctx);
    let mut style = (*ctx.style()).clone();
    {
        use egui::{FontFamily::Proportional, FontId, TextStyle};
        style.text_styles = [
            (TextStyle::Heading, FontId::new(20.0, Proportional)),
            (TextStyle::Body, FontId::new(14.0, Proportional)),
            (TextStyle::Button, FontId::new(14.0, Proportional)),
            (
                TextStyle::Monospace,
                FontId::new(14.0, egui::FontFamily::Monospace),
            ),
            (TextStyle::Small, FontId::new(11.5, Proportional)),
        ]
        .into();
    }
    {
        let v = &mut style.visuals;
        v.dark_mode = true;
        v.panel_fill = BG;
        v.window_fill = CARD;
        v.extreme_bg_color = FIELD;
        v.faint_bg_color = CARD_HI;
        v.override_text_color = Some(TEXT);
        v.hyperlink_color = ACCENT;
        v.selection.bg_fill = ACCENT.gamma_multiply(0.30);
        v.selection.stroke = egui::Stroke::new(1.0, TEXT);
        v.window_stroke = egui::Stroke::new(1.0, LINE);
        v.window_corner_radius = 14.into();
        v.widgets.noninteractive.bg_fill = CARD;
        v.widgets.noninteractive.bg_stroke = egui::Stroke::new(1.0, LINE);
        v.widgets.noninteractive.fg_stroke = egui::Stroke::new(1.0, TEXT);
        v.widgets.inactive.weak_bg_fill = CARD_HI;
        v.widgets.inactive.bg_stroke = egui::Stroke::new(1.0, LINE);
        v.widgets.inactive.fg_stroke = egui::Stroke::new(1.0, TEXT);
        v.widgets.inactive.corner_radius = 8.into();
        v.widgets.hovered.weak_bg_fill = ROW_SEL;
        v.widgets.hovered.bg_stroke = egui::Stroke::new(1.0, ACCENT.gamma_multiply(0.7));
        v.widgets.hovered.fg_stroke = egui::Stroke::new(1.0, TEXT);
        v.widgets.hovered.corner_radius = 8.into();
        v.widgets.active.weak_bg_fill = ROW_SEL;
        v.widgets.active.bg_stroke = egui::Stroke::new(1.0, ACCENT);
        v.widgets.active.fg_stroke = egui::Stroke::new(1.0, TEXT);
        v.widgets.active.corner_radius = 8.into();
        v.widgets.open.weak_bg_fill = ROW_SEL;
    }
    style.spacing.item_spacing = egui::vec2(6.0, 4.0);
    style.spacing.button_padding = egui::vec2(11.0, 5.0);
    style.spacing.interact_size.y = 24.0;
    style.spacing.window_margin = egui::Margin::same(10);
    ctx.set_style(style);
}

/// Farbnebel und feines Raster hinter allem - derselbe Trick wie auf
/// fleitec-Seiten, nur eben gemalt statt per CSS.
fn paint_background(ctx: &egui::Context) {
    let painter = ctx.layer_painter(egui::LayerId::background());
    let r = ctx.screen_rect();
    painter.rect_filled(r, 0.0, BG);

    // zwei weiche Farborbs (viele Kreise mit wenig Deckkraft = Verlauf)
    for (rel_x, rel_y, radius, color) in [
        (0.18, 0.05, 0.55, ACCENT),
        (0.92, 0.75, 0.50, VIOLET),
    ] {
        let center = egui::pos2(r.left() + r.width() * rel_x, r.top() + r.height() * rel_y);
        let max = r.width().max(r.height()) * radius;
        let steps = 14;
        for i in 0..steps {
            let f = 1.0 - i as f32 / steps as f32;
            painter.circle_filled(center, max * f, color.gamma_multiply(0.012));
        }
    }

    // 64px-Raster, das nach unten ausblendet
    let step = 64.0;
    let mut y = r.top();
    while y < r.bottom() {
        let fade = (1.0 - (y - r.top()) / r.height()).clamp(0.0, 1.0);
        painter.hline(
            r.left()..=r.right(),
            y,
            egui::Stroke::new(1.0, egui::Color32::from_white_alpha((10.0 * fade) as u8)),
        );
        y += step;
    }
    let mut x = r.left();
    while x < r.right() {
        painter.vline(
            x,
            r.top()..=r.bottom(),
            egui::Stroke::new(1.0, egui::Color32::from_white_alpha(5)),
        );
        x += step;
    }
}

/// Marke oben links: abgerundetes Quadrat mit Auge.
fn logo(ui: &mut egui::Ui) {
    let (rect, _) = ui.allocate_exact_size(egui::vec2(34.0, 34.0), egui::Sense::hover());
    let p = ui.painter();
    p.rect_filled(rect, 10.0, ACCENT.gamma_multiply(0.18));
    p.rect_stroke(
        rect,
        10.0,
        egui::Stroke::new(1.0, ACCENT.gamma_multiply(0.55)),
        egui::StrokeKind::Inside,
    );
    p.circle_stroke(rect.center(), 8.0, egui::Stroke::new(2.0, ACCENT));
    p.circle_filled(rect.center(), 3.0, ACCENT);
}

/// Reiter in der Kopfzeile.
fn tab(ui: &mut egui::Ui, text: &str, selected: bool) -> egui::Response {
    let label = egui::RichText::new(text)
        .size(13.5)
        .color(if selected { BG } else { MUTED });
    let btn = egui::Button::new(label)
        .fill(if selected {
            ACCENT
        } else {
            egui::Color32::TRANSPARENT
        })
        .stroke(egui::Stroke::NONE)
        .corner_radius(9)
        .min_size(egui::vec2(0.0, 24.0));
    ui.add(btn)
}

/// Überschrift über einer Karte.
fn section(ui: &mut egui::Ui, title: &str) {
    ui.label(
        egui::RichText::new(title)
            .font(head_font(15.5))
            .color(egui::Color32::from_rgb(0xd4, 0xdc, 0xea)),
    );
    ui.add_space(3.0);
}

/// Kleine graue Beschriftung über einem Feld.
fn label_small(ui: &mut egui::Ui, text: &str) {
    ui.label(egui::RichText::new(text).size(11.5).color(MUTED));
    ui.add_space(2.0);
}

/// Dünne Trennlinie innerhalb einer Karte.
fn divider(ui: &mut egui::Ui) {
    let r = ui.available_rect_before_wrap();
    ui.painter().hline(
        r.left()..=r.right(),
        r.top(),
        egui::Stroke::new(1.0, LINE),
    );
    ui.add_space(1.0);
}

/// Eine Karte im Hausstil.
fn card<R>(ui: &mut egui::Ui, add: impl FnOnce(&mut egui::Ui) -> R) -> R {
    egui::Frame::group(ui.style())
        .fill(CARD)
        .stroke(egui::Stroke::new(1.0, LINE))
        .corner_radius(13)
        .inner_margin(egui::Margin::same(10))
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            add(ui)
        })
        .inner
}

/// Der eine blaue Knopf.
fn accent_button(ui: &mut egui::Ui, text: &str, enabled: bool) -> egui::Response {
    let btn = egui::Button::new(
        egui::RichText::new(text)
            .size(14.5)
            .color(if enabled { BG } else { MUTED }),
    )
    .fill(if enabled { ACCENT } else { CARD_HI })
    .stroke(egui::Stroke::NONE)
    .corner_radius(10)
    .min_size(egui::vec2(200.0, 31.0));
    ui.add_enabled(enabled, btn)
}

/// Zweitrangiger Knopf mit Rand.
fn ghost(size: egui::Vec2, text: &str) -> egui::Button<'static> {
    egui::Button::new(egui::RichText::new(text).size(13.5).color(TEXT))
        .fill(CARD_HI)
        .stroke(egui::Stroke::new(1.0, LINE))
        .corner_radius(10)
        .min_size(size)
}

/// Kleiner zweitrangiger Knopf.
fn ghost_button(ui: &mut egui::Ui, text: &str) -> egui::Response {
    ui.add(
        egui::Button::new(egui::RichText::new(text).size(12.5).color(MUTED))
            .fill(CARD_HI)
            .stroke(egui::Stroke::new(1.0, LINE))
            .corner_radius(8),
    )
}

/// Grüner oder grauer Punkt vor einem Gerät.
fn dot(ui: &mut egui::Ui, online: bool) {
    let (rect, _) = ui.allocate_exact_size(egui::vec2(12.0, 12.0), egui::Sense::hover());
    let c = if online { GREEN } else { MUTED.gamma_multiply(0.55) };
    if online {
        ui.painter().circle_filled(rect.center(), 6.5, c.gamma_multiply(0.25));
    }
    ui.painter().circle_filled(rect.center(), 4.0, c);
}

/// Zustand des eigenen Rechners, oben rechts.
fn status_pill(ui: &mut egui::Ui, ok: bool, text: &str) {
    egui::Frame::group(ui.style())
        .fill(if ok {
            GREEN.gamma_multiply(0.14)
        } else {
            CARD_HI
        })
        .stroke(egui::Stroke::new(
            1.0,
            if ok { GREEN.gamma_multiply(0.6) } else { LINE },
        ))
        .corner_radius(12)
        .inner_margin(egui::Margin::symmetric(10, 3))
        .show(ui, |ui| {
            ui.label(
                egui::RichText::new(text)
                    .size(11.5)
                    .color(if ok { GREEN } else { MUTED }),
            );
        });
}

/// „Schlüssel   Wert“-Zeile in der Info-Karte.
fn info_row(ui: &mut egui::Ui, key: &str, value: &str) {
    ui.horizontal(|ui| {
        ui.label(egui::RichText::new(key).color(MUTED).size(11.5));
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            ui.add(
                egui::Label::new(egui::RichText::new(value).size(12.5))
                    .selectable(true)
                    .truncate(),
            );
        });
    });
    ui.add_space(2.0);
}

/// Fenstersymbol: dasselbe Auge wie im Infobereich, 32x32 RGBA.
fn app_icon() -> egui::IconData {
    const N: i32 = 32;
    let mut rgba = vec![0u8; (N * N * 4) as usize];
    let c = (N as f32 - 1.0) / 2.0;
    for y in 0..N {
        for x in 0..N {
            let (dx, dy) = (x as f32 - c, y as f32 - c);
            let d = (dx * dx + dy * dy).sqrt();
            if d > 15.0 {
                continue;
            }
            let (r, g, b) = if d <= 5.0 {
                (0xff, 0xff, 0xff)
            } else if d <= 8.5 {
                (0x07, 0x09, 0x0f)
            } else {
                (0x38, 0xbd, 0xf8)
            };
            let i = ((y * N + x) * 4) as usize;
            rgba[i] = r;
            rgba[i + 1] = g;
            rgba[i + 2] = b;
            rgba[i + 3] = 0xff;
        }
    }
    egui::IconData {
        rgba,
        width: N as u32,
        height: N as u32,
    }
}

/// Speichert ein Bild der Oberflaeche als JPEG (fuer --shot).
fn save_shot(img: &egui::ColorImage, path: &std::path::Path) {
    use image::ImageEncoder;
    let [w, h] = img.size;
    let mut rgb = Vec::with_capacity(w * h * 3);
    for p in &img.pixels {
        rgb.extend_from_slice(&[p.r(), p.g(), p.b()]);
    }
    match std::fs::File::create(path) {
        Ok(mut f) => {
            let enc = image::codecs::jpeg::JpegEncoder::new_with_quality(&mut f, 92);
            match enc.write_image(&rgb, w as u32, h as u32, image::ExtendedColorType::Rgb8) {
                Ok(()) => println!("SHOT {}x{} -> {}", w, h, path.display()),
                Err(e) => eprintln!("SHOT FAIL: {}", e),
            }
        }
        Err(e) => eprintln!("SHOT FAIL: {}", e),
    }
}
