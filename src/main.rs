#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

//! FreeViewer - open source remote desktop.
//!
//! One binary does both jobs, like TeamViewer: it registers this machine at the
//! relay (so others can connect to your ID) and it can connect to another ID.
//!
//! Relay can be overridden with the FV_RELAY environment variable,
//! the session password with FV_PASSWORD.

mod crypto;
mod encoder;
mod hostside;
mod ident;
mod net;
mod proto;
mod shared;
mod viewer;

use std::sync::atomic::Ordering;
use std::sync::{Arc, OnceLock};
use std::time::Duration;

use proto::Msg;
use shared::Shared;

const DEFAULT_RELAY: &str = "wss://jarvis.fleitec.com/fv/ws";

static RT: OnceLock<tokio::runtime::Runtime> = OnceLock::new();

fn rt() -> &'static tokio::runtime::Runtime {
    RT.get().expect("tokio runtime")
}

fn main() -> eframe::Result<()> {
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

    // capture self test:  freeviewer --captest   (writes <config>/captest.txt)
    if std::env::args().any(|a| a == "--captest") {
        let report = hostside::capture_selftest(8);
        let path = ident::config_dir().join("captest.txt");
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
    let viewer_only = std::env::args().any(|a| a == "--connect");
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

    // headless viewer mode for testing:  freeviewer --connect <id> <password> [frames]
    let argv: Vec<String> = std::env::args().collect();
    if let Some(pos) = argv.iter().position(|a| a == "--connect") {
        let id = argv.get(pos + 1).cloned().unwrap_or_default();
        let pw = argv.get(pos + 2).cloned().unwrap_or_default();
        let want: u64 = argv
            .get(pos + 3)
            .and_then(|s| s.parse().ok())
            .unwrap_or(10);
        let sh = shared.clone();
        let idc = id.clone();
        rt().spawn(async move { viewer::run_viewer(sh, idc, pw).await });
        let start = std::time::Instant::now();
        let mut last = 0u64;
        loop {
            std::thread::sleep(Duration::from_millis(200));
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
                println!(
                    "OK: {} Frames in {:.1}s, {:.0} fps, {:.0} kbit/s, {:.0} ms rtt",
                    last,
                    start.elapsed().as_secs_f32(),
                    st.fps,
                    st.kbps,
                    st.latency_ms
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
        self.viewer_task = Some(rt().spawn(async move {
            viewer::run_viewer(sh, id, pw).await;
        }));
    }

    fn stop_session(&mut self) {
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
            ui.label(egui::RichText::new("v0.3 - frei, verschluesselt, ohne Konto").weak());
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

        ui.add_space(14.0);
        ui.label(
            egui::RichText::new(format!("Relay: {}", self.shared.relay_url))
                .weak()
                .size(11.0),
        );
    }

    fn session_ui(&mut self, ctx: &egui::Context) {
        let stats = *self.shared.stats.lock().unwrap();
        let mut disconnect = false;

        egui::TopBottomPanel::top("session_bar").show(ctx, |ui| {
            ui.horizontal(|ui| {
                if ui.button("Trennen").clicked() {
                    disconnect = true;
                }
                ui.separator();
                let (rw, rh) = *self.shared.remote_size.lock().unwrap();
                ui.label(format!(
                    "{}x{}   {:.0} fps   {:.0} kbit/s   {:.0} ms",
                    rw, rh, stats.fps, stats.kbps, stats.latency_ms
                ));
            });
        });

        let mut image_rect: Option<egui::Rect> = None;
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
                        image_rect = Some(resp.rect);
                    });
                } else {
                    ui.centered_and_justified(|ui| {
                        ui.label("Warte auf Bild...");
                    });
                }
            });

        if let Some(rect) = image_rect {
            self.forward_input(ctx, rect);
        }

        if disconnect {
            self.stop_session();
        }
    }

    fn forward_input(&mut self, ctx: &egui::Context, rect: egui::Rect) {
        // modifier keys are not part of egui's Key enum, so we diff the state
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
                    if let Some((x, y)) = norm(pos) {
                        self.shared.send_input(Msg::MouseMove { x, y });
                    }
                }
                egui::Event::PointerButton {
                    pos,
                    button,
                    pressed,
                    ..
                } => {
                    if let Some((x, y)) = norm(pos) {
                        self.shared.send_input(Msg::MouseMove { x, y });
                        let b = match button {
                            egui::PointerButton::Secondary => 1u8,
                            egui::PointerButton::Middle => 2u8,
                            _ => 0u8,
                        };
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
                    if !repeat {
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

        if self.shared.connected.load(Ordering::Relaxed) {
            self.session_ui(ctx);
            ctx.request_repaint_after(Duration::from_millis(16));
        } else {
            egui::CentralPanel::default().show(ctx, |ui| self.home_ui(ui));
            ctx.request_repaint_after(Duration::from_millis(250));
        }
    }
}
