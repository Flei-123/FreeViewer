#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

//! FreeViewer - open source remote desktop.
//!
//! One binary does both jobs, like TeamViewer: it registers this machine at the
//! relay (so others can connect to your ID) and it can connect to another ID.
//!
//! Relay can be overridden with the FV_RELAY environment variable,
//! the session password with FV_PASSWORD.

mod capture;
mod clip;
mod crypto;
mod encoder;
mod hostside;
mod ident;
mod input;
mod net;
mod proto;
mod selftest;
mod shared;
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
    if !viewer_only {
        let host_shared = shared.clone();
        let host_secret = secret.clone();
        rt().spawn(async move {
            hostside::run_host(host_shared, host_secret).await;
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
                println!(
                    "OK: {} Frames in {:.1}s, {:.0} fps, {:.0} kbit/s, {:.0} ms rtt, Cursor {:?}",
                    last,
                    start.elapsed().as_secs_f32(),
                    st.fps,
                    st.kbps,
                    st.latency_ms,
                    cur
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

    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_inner_size([1100.0, 720.0])
            .with_min_inner_size([700.0, 470.0])
            .with_title("FreeViewer"),
        ..Default::default()
    };

    eframe::run_native(
        "FreeViewer",
        options,
        Box::new(move |_cc| Ok(Box::new(App::new(shared)))),
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
}

impl App {
    fn new(shared: Arc<Shared>) -> Self {
        Self {
            shared,
            partner_id: String::new(),
            partner_pw: String::new(),
            tex: None,
            last_seq: 0,
            last_mods: egui::Modifiers::default(),
            viewer_task: None,
            hint: String::new(),
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
        self.shared.mode.store(proto::MODE_ADMIN, Ordering::Relaxed);
        self.viewer_task = Some(rt().spawn(async move {
            viewer::run_viewer(sh, id, pw).await;
        }));
    }

    fn stop_session(&mut self) {
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
                {
                    let mut pw = self.shared.password.lock().unwrap();
                    ui.add(
                        egui::TextEdit::singleline(&mut *pw)
                            .font(egui::TextStyle::Monospace)
                            .desired_width(180.0),
                    );
                }
                if ui.button("Neues Passwort").clicked() {
                    *self.shared.password.lock().unwrap() = ident::random_password();
                }
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
                ui.add_space(12.0);
                let btn = egui::Button::new(if connecting {
                    "Verbinde..."
                } else {
                    "Verbinden"
                });
                if ui.add_enabled(!connecting, btn).clicked() {
                    self.start_session();
                }
                ui.add_space(10.0);
                ui.label(egui::RichText::new(viewer_status).weak());
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
                ui.label(format!(
                    "{}x{}   {:.0} fps   {:.0} kbit/s   {:.0} ms",
                    rw, rh, stats.fps, stats.kbps, stats.latency_ms
                ));
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
        self.pull_frame(ctx);
        self.handle_drops(ctx);
        self.transfer_ui(ctx);

        if self.shared.connected.load(Ordering::Relaxed) {
            self.session_ui(ctx);
            ctx.request_repaint_after(Duration::from_millis(8));
        } else {
            if vinput::is_active() {
                vinput::set_active(false);
            }
            egui::CentralPanel::default().show(ctx, |ui| self.home_ui(ui));
            ctx.request_repaint_after(Duration::from_millis(250));
        }
    }
}
