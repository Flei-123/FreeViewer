//! Split out of main.rs (r35): mechanical move, behaviour unchanged.
use crate::lockutil::LockExt;
use super::*;

impl App {
    pub(crate) fn new(shared: Arc<Shared>, start_hidden: bool, auto_setup: Option<(String, String)>) -> Self {
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
            friends: None,
            friends_id: String::new(),
            friends_notice: String::new(),
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
            meet_shot: None,
            meet_shot_n: 0,
            meet_shot_arm: 0,
            meet_shot_join: false,
            shot_warte: 6,
            look: theme::load(),
            stab: SettingsTab::General,
            fb_bug: true,
            fb_text: String::new(),
            fb_contact: String::new(),
            folder: String::new(),
            add_dev: None,
            edit_dev: None,
            caption_tick: std::time::Instant::now() - Duration::from_secs(9),
            shot_n: 0,
            pw_list: pwlist::load(),
            pw_new: String::new(),
            pw_new_label: String::new(),
            pw_visible: false,
            pw_visible_at: None,
            pw_old_input: String::new(),
            pw_old_wrong: false,
            pw_del_arm: None,
            last_ask: false,
            pw_overwrite: None,
            pw_show: None,
            install_service: true,
            link_check: std::time::Instant::now() - Duration::from_secs(2),
            meet_title: String::new(),
            meet_e2e: false,
            meet_id: String::new(),
            meet_pw: String::new(),
            meet_last: None,
            meet_list: Arc::new(std::sync::Mutex::new(Vec::new())),
            meet_chrome: 99,
            meet_busy: Arc::new(std::sync::atomic::AtomicBool::new(false)),
            meet_err: Arc::new(std::sync::Mutex::new(String::new())),
            meet_loaded: false,
            meet_offer_control: true,
            meet_win: MeetWin::default(),
            nativ_meet: None,
            nativ_bilder: std::collections::HashMap::new(),
            nativ_schirme: std::collections::HashMap::new(),
            meet_teilt_war: false,
            nativ_eigen: None,
            native_own_screen: None,
            vorprobe: meetui::Vorprobe::default(),
            meet_z: meetfenster::Fensterzustand::default(),
            meet_gelesen: 0,
            meet_kam_start: None,
            headless: false,
            meet_demo: None,
            full: false,
            bar_x: 0.5,
            bar_w: 700.0,
            bar_pinned: true,
            bar_seen: std::time::Instant::now(),
            frame_at: std::time::Instant::now(),
            acc: account::load(),
            acc_user: String::new(),
            acc_pass: String::new(),
            acc_msg: String::new(),
            acc_busy: Arc::new(std::sync::atomic::AtomicBool::new(false)),
            acc_out: Arc::new(std::sync::Mutex::new(None)),
            acc_next: std::time::Instant::now(),
            dep_pw: ident::random_password(),
            dep_hint_name: String::new(),
            dep_count: 1,
            dep_link: String::new(),
            dep_msg: String::new(),
            dep_pending: Vec::new(),
            dep_claimed: Vec::new(),
            dep_seen: std::collections::HashSet::new(),
            dep_booted: false,
            dep_out: Arc::new(std::sync::Mutex::new(None)),
            dep_next: std::time::Instant::now(),
            setup_dlg: None,
            pw_ask: None,
            uninstall_ask: false,
            // Ask for a license once at start if there is no valid one.
            #[cfg(feature = "license")]
            lic_key: String::new(),
            #[cfg(feature = "license")]
            lic_msg: String::new(),
            #[cfg(feature = "license")]
            lic_show: !matches!(license::status(), license::Status::Active(_)),
            auto_setup,
            upd_flag_at: std::time::Instant::now() - Duration::from_secs(9),
            upd_running: false,
            snd_in: Vec::new(),
            snd_out: Vec::new(),
            snd_default: (String::new(), String::new()),
        }
    }

    /// Everything around the tray icon: hide instead of quit, pick up what
    /// the menu did, keep the checkbox in sync.
    pub(crate) fn tray_ui(&mut self, ctx: &egui::Context) {
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
                    &format!("{} laeuft weiter", crate::brand::NAME),
                    "Das Fenster ist nur zugeklappt. Ueber das Symbol im Infobereich kommst du zurueck - oder beendest FreeViewer ganz.",
                );
            }
        }
        if tray::is_hidden() {
            // no picture to draw, but the menu must stay responsive
            ctx.request_repaint_after(Duration::from_millis(400));
        }
    }

    pub(crate) fn start_session(&mut self) {
        if self.is_me(&self.partner_id.clone()) {
            self.hint = i18n::t("start.self").to_string();
            return;
        }
        let id: String = self
            .partner_id
            .chars()
            .filter(|c| c.is_ascii_digit())
            .collect();
        if id.len() < 9 {
            self.shared
                .set_viewer_status(i18n::t("start.bad_id"));
            return;
        }
        let sh = self.shared.clone();
        let pw = self.partner_pw.clone();
        self.last_ask = false;
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
    pub(crate) fn start_ask_session(&mut self) {
        if self.is_me(&self.partner_id.clone()) {
            self.hint = i18n::t("start.self").to_string();
            return;
        }
        let id: String = self
            .partner_id
            .chars()
            .filter(|c| c.is_ascii_digit())
            .collect();
        if id.len() < 9 {
            self.shared
                .set_viewer_status(i18n::t("start.bad_id"));
            return;
        }
        let sh = self.shared.clone();
        self.last_ask = true;
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
    pub(crate) fn close_session(&mut self) {
        self.full = false;
        if let Some((id, started)) = self.session.take() {
            self.book.ended(&id, started.elapsed().as_secs());
        }
        self.hint_until = None;
    }

    pub(crate) fn stop_session(&mut self) {
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
        *self.shared.input_tx.lock_ok() = None;
        *self.shared.frame.lock_ok() = None;
        self.shared.set_viewer_status("Getrennt");
        self.tex = None;
        self.last_seq = 0;
        self.last_mods = egui::Modifiers::default();
    }

    pub(crate) fn set_mode(&mut self, mode: u8) {
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

    /// Is a freeviewer:// address waiting? Check at most once per second
    /// - the UI draws much more often.
    pub(crate) fn handle_link(&mut self, ctx: &egui::Context) {
        if self.link_check.elapsed() < Duration::from_millis(900) {
            return;
        }
        self.link_check = std::time::Instant::now();
        let action = match link::take() {
            Some(a) => a,
            None => return,
        };
        ctx.send_viewport_cmd(egui::ViewportCommand::Visible(true));
        ctx.send_viewport_cmd(egui::ViewportCommand::Focus);
        match action {
            link::Action::Control(id) => {
                self.view = View::Start;
                self.hint = i18n::tf("link.control", &partners::pretty_id(&id));
                self.connect_to(&id);
            }
            link::Action::Setup(code) => {
                // One-time link from the owner: set this machine up and
                // then install it properly (with service).
                self.setup_dlg = Some(SetupDlg {
                    code,
                    name: presence::machine_name(),
                    ..Default::default()
                });
            }
            link::Action::Meet { room, pass } => {
                self.view = View::Meet;
                self.meet_id = room.clone();
                self.meet_pw = pass.clone();
                // Invitation links now land in the own meeting window
                // (choose devices, copy invitation), no longer immediately
                // in the browser.
                self.meet_win_open(meet::Meeting {
                    id: room.clone(),
                    titel: String::new(),
                    passphrase: pass.clone(),
                    termin_text: String::new(),
                    e2e: String::new(),
                });
            }
        }
    }

    /// Files dropped onto the window go to the other side of the session.
    pub(crate) fn handle_drops(&mut self, ctx: &egui::Context) {
        // As long as something hovers over the window: show that dropping
        // is allowed here. Without it you guess whether drag and drop works.
        if ctx.input(|i| !i.raw.hovered_files.is_empty()) {
            let p = theme::palette();
            let screen = ctx.content_rect();
            let pt = ctx.layer_painter(egui::LayerId::new(
                egui::Order::Foreground,
                egui::Id::new("fv_drop_hint"),
            ));
            pt.rect_filled(screen, 0.0, p.accent.gamma_multiply(0.16));
            pt.rect_stroke(
                screen.shrink(10.0),
                10.0,
                egui::Stroke::new(2.0, p.accent),
                egui::StrokeKind::Inside,
            );
            let live = self.shared.xfer.lock_ok().is_some();
            pt.text(
                screen.center(),
                egui::Align2::CENTER_CENTER,
                if live {
                    i18n::t("sess.drop")
                } else {
                    i18n::t("sess.drop_none")
                },
                egui::FontId::proportional(19.0),
                p.text,
            );
        }
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
        let mut guard = self.shared.xfer.lock_ok();
        match guard.as_mut() {
            Some(x) => {
                let n = files.len();
                for p in files {
                    x.send_path(p);
                }
                self.hint = i18n::tf("sess.drop_sent", &n.to_string());
            }
            None => {
                self.hint = i18n::t("sess.drop_none").to_string();
            }
        }
    }

    pub(crate) fn pick_and_send(&mut self) {
        let files = rfd::FileDialog::new()
            .set_title("Datei(en) an die Gegenstelle senden")
            .pick_files();
        if let Some(files) = files {
            let mut guard = self.shared.xfer.lock_ok();
            if let Some(x) = guard.as_mut() {
                for p in files {
                    x.send_path(p);
                }
            } else {
                self.hint = "Keine aktive Sitzung".to_string();
            }
        }
    }

    pub(crate) fn open_drop_dir(&self) {
        let dir = self.shared.drop_dir.lock_ok().clone();
        let _ = std::fs::create_dir_all(&dir);
        #[cfg(windows)]
        {
            let _ = std::process::Command::new("explorer").arg(&dir).spawn();
        }
    }

    /// Bottom bar with every running/finished transfer of this session.
    pub(crate) fn transfer_ui(&mut self, ctx: &egui::Context) {
        let list = self.shared.xfers.lock_ok().clone();
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

    /// Row for the self-update.
    pub(crate) fn update_ui(&mut self, ui: &mut egui::Ui) {
        let pending = self.shared.update.lock_ok().clone();
        let status = self.shared.update_status.lock_ok().clone();
        let dienst = service::installed();
        ui.horizontal(|ui| {
            if icons::text_button(ui, "refresh", i18n::t("upd.check"), false).clicked() {
                let sh = self.shared.clone();
                sh.set_update_status(i18n::t("upd.checking"));
                std::thread::spawn(move || match update::check() {
                    Ok(rel) => {
                        if update::newer(&rel.version, update::VERSION) {
                            sh.set_update_status(format!(
                                "{} {}",
                                i18n::t("upd.found"),
                                rel.version
                            ));
                            *sh.update.lock_ok() = Some(rel);
                        } else {
                            sh.set_update_status(format!(
                                "{} (v{})",
                                i18n::t("upd.current"),
                                update::VERSION
                            ));
                        }
                    }
                    Err(e) => sh.set_update_status(format!("{}: {}", i18n::t("upd.failed"), e)),
                });
            }
            let mut auto = self.shared.auto_update.load(Ordering::Relaxed);
            if check(ui, &mut auto, i18n::t("upd.auto")).changed() {
                self.shared.auto_update.store(auto, Ordering::Relaxed);
                ident::set_auto_update(auto);
            }
        });
        if let Some(rel) = pending {
            ui.add_space(4.0);
            if ghost_button(ui, &format!("Update {} installieren", rel.version)).clicked() {
                if dienst {
                    // the service installs it - as SYSTEM, without a prompt
                    update::bitte_dienst();
                    self.shared
                        .set_update_status(i18n::t("upd.by_service"));
                } else {
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
        }
        if !status.is_empty() {
            ui.add_space(2.0);
            ui.label(egui::RichText::new(status).color(theme::muted()).size(11.5));
        }
    }
    pub(crate) fn pull_frame(&mut self, ctx: &egui::Context) {
        let img = {
            let guard = self.shared.frame.lock_ok();
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
            self.frame_at = std::time::Instant::now();
            match self.tex.as_mut() {
                Some(t) => t.set(ci, egui::TextureOptions::LINEAR),
                None => {
                    self.tex = Some(ctx.load_texture("remote", ci, egui::TextureOptions::LINEAR))
                }
            }
        }
    }

    // ----------------------------------------------------------- Oberflaeche

    /// Header with the three areas.
    // ----------------------------------------------------------- Oberfläche

    /// Header: brand on the left, the three areas as pills, status on the right.
    pub(crate) fn top_bar(&mut self, ui: &mut egui::Ui) {
        let my_id = self.shared.my_id.lock_ok().clone();
        let online = !my_id.is_empty();
        ui.horizontal(|ui| {
            ui.add_space(2.0);
            logo(ui);
            ui.add_space(2.0);
            ui.vertical(|ui| {
                ui.add_space(2.0);
                ui.label(egui::RichText::new(crate::brand::NAME).size(17.0).strong());
                ui.label(
                    egui::RichText::new(format!("Version {}", update::VERSION))
                        .size(11.0)
                        .color(theme::muted()),
                );
            });
            ui.add_space(10.0);
            // The counter on "Friends" matters: a friend
            // request nobody sees is not a request.
            let offen = self
                .friends
                .as_ref()
                .map(|d| d.eingehende().len() + d.einladungen().len())
                .unwrap_or(0);
            let friends_text = if offen > 0 {
                format!("Freunde ({})", offen)
            } else {
                "Freunde".to_string()
            };
            for (v, name) in [
                (View::Start, i18n::t("nav.start").to_string()),
                (View::Devices, i18n::t("nav.devices").to_string()),
                (View::Friends, friends_text),
                (View::Settings, i18n::t("nav.settings").to_string()),
            ] {
                if tab(ui, &name, self.view == v).clicked() {
                    self.view = v;
                }
            }
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                let state = self.shared.net_state();
                let online = online && state == 1;
                status_pill(
                    ui,
                    online,
                    if online {
                        i18n::t("pill.ready")
                    } else if state == 2 {
                        i18n::t("st.no_net")
                    } else {
                        i18n::t("pill.connecting")
                    },
                );
            });
        });
        ui.add_space(6.0);
        let line = ui.available_rect_before_wrap();
        ui.painter().hline(
            line.left()..=line.right(),
            line.top(),
            egui::Stroke::new(1.0, theme::line()),
        );
        ui.add_space(8.0);
    }

    pub(crate) fn home_ui(&mut self, ui: &mut egui::Ui) {
        match self.view {
            View::Start => self.start_view(ui),
            View::Devices => self.devices_view(ui),
            View::Friends => self.friends_view(ui),
            View::Meet => self.meet_view(ui),
            View::Settings => self.settings_view(ui),

        }
    }
}
