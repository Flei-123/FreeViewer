//! Split out of main.rs (r35): mechanical move, behaviour unchanged.
use crate::lockutil::LockExt;
use super::*;

pub(crate) fn map_key(k: egui::Key) -> Option<(u32, bool)> {
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
        // Sign of life for the watchdog: the UI is drawing.
        IMAGE_DRAWN.store(true, Ordering::Relaxed);
        if let Some(n) = self.meet_demo {
            self.meet_demo_ui(ctx, n);
            self.shot_ui(ctx);
            return;
        }
        // Without constant redrawing in the parent window the
        // meeting window gets no frames - and --meetshot waits forever.
        // For the image the meeting window is drawn into the MAIN window.
        // A separate child window returns nothing on the image command -
        // measured: the screenshot event never arrived there.
        if self.meet_shot.is_some() {
            self.meet_win_ui(ctx);
            self.shot_ui(ctx);
            return;
        }
        if self.caption_tick.elapsed() > Duration::from_secs(2) {
            self.caption_tick = std::time::Instant::now();
            chrome::paint_from_theme();
        }
        self.tray_ui(ctx);
        // Setup job from the browser download: as soon as the host
        // has its ID from the relay, everything happens by itself (no pairing).
        if let Some((code, name)) = self.auto_setup.clone() {
            let ready_id = !self.shared.my_id.lock_ok().is_empty();
            if ready_id {
                self.auto_setup = None;
                self.setup_dlg = Some(SetupDlg {
                    code,
                    name: if name.trim().is_empty() {
                        presence::machine_name()
                    } else {
                        name
                    },
                    auto: true,
                    ..Default::default()
                });
            }
        }
        self.update_modal(ctx);
        self.uninstall_modal(ctx);
        #[cfg(feature = "license")]
        self.license_modal(ctx);
        self.knock_ui(ctx);
        self.pw_overwrite_ui(ctx);
        self.pw_ask_ui(ctx);
        self.setup_dialog_ui(ctx);
        self.pull_frame(ctx);
        // eframe imposes its own look at start (and when Windows switches between
        // light and dark) - then, for example, the borders of the input fields
        // are missing. So check and, if necessary, put our
        // look back on.
        if !theme::style_is_ours(ctx) {
            let look = self.look.clone();
            theme::apply(ctx, &look);
        }
        self.handle_drops(ctx);
        self.handle_link(ctx);
        // Account: collect results and quietly sync every two minutes
        self.take_sync_result();
        if self.acc.is_some()
            && !self.acc_busy.load(Ordering::Relaxed)
            && std::time::Instant::now() >= self.acc_next
        {
            self.start_sync(false);
        }
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

        let connected = self.shared.connected.load(Ordering::Relaxed);
        if !connected {
            if self.session.is_some() && !self.shared.connecting.load(Ordering::Relaxed) {
                // the session died on its own - still book the time
                self.close_session();
            }
            if vinput::is_active() {
                vinput::set_active(false);
            }
            // Fullscreen belonged to the session window that is just closing -
            // only reset the marker here, the main window was never
            // in fullscreen.
            self.full = false;
        }

        // The main window always stays the main window: devices, Meet and
        // settings are usable even during a running session.
        paint_background(ctx);
        self.rail(ctx);
        self.header(ctx);
        egui::CentralPanel::default()
            .frame(
                egui::Frame::NONE
                    .fill(egui::Color32::TRANSPARENT)
                    .inner_margin(egui::Margin::symmetric(16, 10)),
            )
            .show(ctx, |ui| {
                // Do not stretch the content across the whole width
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
        self.add_dev_ui(ctx);
        ctx.request_repaint_after(Duration::from_millis(250));

        // The session runs in its own operating system window. This way
        // the main window stays usable (open a second session, search a
        // file), and fullscreen only affects the remote screen.
        if connected {
            let title = match self.shared.host_peer.lock_ok().clone() {
                s if s.is_empty() => i18n::t("sess.window").to_string(),
                _ => {
                    let id = self.partner_id.clone();
                    if id.is_empty() {
                        i18n::t("sess.window").to_string()
                    } else {
                        format!("{} - {}", i18n::t("sess.window"), partners::pretty_id(&id))
                    }
                }
            };
            let mut closed = false;
            ctx.show_viewport_immediate(
                egui::ViewportId::from_hash_of("fv_session"),
                egui::ViewportBuilder::default()
                    .with_title(title)
                    .with_inner_size([1280.0, 800.0])
                    .with_min_inner_size([480.0, 320.0]),
                |vctx, _class| {
                    // Dropping must be read HERE - the session window
                    // has its own input, the main window never got to
                    // see the files.
                    self.handle_drops(vctx);
                    self.session_ui(vctx);
                    if vctx.input(|i| i.viewport().close_requested()) {
                        closed = true;
                    }
                    vctx.request_repaint_after(Duration::from_millis(8));
                },
            );
            if closed {
                self.stop_session();
            }
        }

        // Like the remote session, the meeting gets its own window:
        // preparation, invitation and participant list run next to the
        // main window, not as a tab inside it.
        if self.meet_win.is_open {
            let mut closed = false;
            ctx.show_viewport_immediate(
                egui::ViewportId::from_hash_of("fv_meet"),
                egui::ViewportBuilder::default()
                    .with_title(i18n::t("meet.window"))
                    .with_inner_size([920.0, 600.0])
                    .with_min_inner_size([560.0, 430.0]),
                |vctx, _class| {
                    let in_conversation = self.native_meet.is_some();
                    self.meet_win_ui(vctx);
                    if vctx.input(|i| i.viewport().close_requested()) {
                        closed = true;
                    }
                    // WHY two beats: 150 ms were a hard upper limit
                    // of about 7 frames per second - no matter how fast the
                    // camera delivered, the window did not draw more often.
                    // Justin saw exactly that as "bad frame rate".
                    // So draw smoothly in a conversation; on the
                    // join screen little is enough, hardly anything moves there.
                    vctx.request_repaint_after(Duration::from_millis(if in_conversation {
                        16
                    } else {
                        60
                    }));
                },
            );
            if closed {
                self.close_meet_win();
            }
        }

        // --shot: let a few frames be drawn, then request an image
        self.shot_ui(ctx);
    }
}