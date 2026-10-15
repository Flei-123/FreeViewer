//! Split out of main.rs (r35): mechanical move, behaviour unchanged.
use crate::lockutil::LockExt;
use super::*;

impl App {
    /// Start page: this PC on the left, the way out on the right.
    /// Start page: this PC on the left, the way out on the right. Deliberately
    /// little text - what needs explaining is in the tooltip.
    pub(crate) fn start_view(&mut self, ui: &mut egui::Ui) {
        let my_id = self.shared.my_id.lock_ok().clone();
        let host_status = self.shared.host_status.lock_ok().clone();
        let host_peer = self.shared.host_peer.lock_ok().clone();
        let connecting = self.shared.connecting.load(Ordering::Relaxed);
        let p = theme::palette();

        ui.columns(2, |cols| {
            // ------------------------------------------- dieser Rechner
            let ui = &mut cols[0];
            section(ui, i18n::t("start.share"));
            card(ui, |ui| {
                label_small(ui, i18n::t("start.your_id"));
                ui.horizontal(|ui| {
                    let id_text = if my_id.len() >= 9 {
                        partners::pretty_id(&my_id)
                    } else {
                        "– – –".to_string()
                    };
                    ui.add(
                        egui::Label::new(
                            egui::RichText::new(id_text).size(26.0).strong().color(p.text),
                        )
                        .selectable(true),
                    );
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        if !my_id.is_empty()
                            && icon_ghost(ui, "copy", i18n::t("start.copy")).clicked()
                        {
                            ui.ctx().copy_text(partners::pretty_id(&my_id));
                            self.hint = i18n::t("start.copy").to_string();
                        }
                    });
                });

                ui.add_space(8.0);
                label_small(ui, i18n::t("start.password"));
                // The session password is rolled by dice, not typed. Fixed
                // passwords are in the settings under "Zugriff".
                let pw_now = self.shared.password.lock_ok().clone();
                if self.pw_visible {
                    match self.pw_visible_at {
                        Some(t) if t.elapsed() < PW_REVEAL_SECS => {
                            ui.ctx().request_repaint_after(Duration::from_millis(500));
                        }
                        _ => {
                            self.pw_visible = false;
                            self.pw_visible_at = None;
                        }
                    }
                }
                ui.horizontal(|ui| {
                    // hidden by default: a screen share or screenshot must not
                    // leak the password
                    let shown_pw = if self.pw_visible {
                        pw_now.clone()
                    } else {
                        pwlist::masked(&pw_now)
                    };
                    ui.add(
                        egui::Label::new(
                            egui::RichText::new(shown_pw)
                                .size(20.0)
                                .family(egui::FontFamily::Monospace)
                                .color(p.text),
                        )
                        .selectable(true),
                    );
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        if icon_ghost(ui, "refresh", i18n::t("start.new_pw")).clicked() {
                            if self.pw_fixed {
                                // replaces the stored permanent password: ask first
                                self.pw_overwrite = Some(PwOverwrite::Regenerate);
                            } else {
                                *self.shared.password.lock_ok() = ident::random_password();
                            }
                        }
                        if icon_ghost(
                            ui,
                            "eye",
                            if self.pw_visible {
                                i18n::t("set.pw_hide")
                            } else {
                                i18n::t("set.pw_show")
                            },
                        )
                        .clicked()
                        {
                            self.pw_visible = !self.pw_visible;
                            self.pw_visible_at = self.pw_visible.then(std::time::Instant::now);
                        }
                        if icon_ghost(ui, "copy", i18n::t("start.copy")).clicked() {
                            ui.ctx().copy_text(pw_now.clone());
                        }
                    });
                });
                ui.label(
                    egui::RichText::new(i18n::t("start.pw_random"))
                        .size(11.0)
                        .color(p.muted),
                );

                ui.add_space(8.0);
                divider(ui);
                ui.add_space(6.0);
                ui.horizontal(|ui| {
                    dot(ui, !my_id.is_empty() && self.shared.net_state() == 1);
                    ui.label(egui::RichText::new(host_status).size(12.0).color(p.muted));
                });
                if host_peer != i18n::t("start.nosession") && !host_peer.is_empty() {
                    ui.label(egui::RichText::new(host_peer).size(12.0).color(p.muted));
                }
            });


            // ------------------------------------------------ verbinden
            let ui = &mut cols[1];
            section(ui, i18n::t("start.control"));
            card(ui, |ui| {
                label_small(ui, i18n::t("start.partner"));
                ui.horizontal(|ui| {
                    ui.add(
                        egui::TextEdit::singleline(&mut self.partner_id)
                            .font(egui::FontId::new(17.0, egui::FontFamily::Monospace))
                            .desired_width(190.0)
                            .margin(egui::Margin::symmetric(8, 4)),
                    );
                    let recent = self.book.sorted();
                    if !recent.is_empty() {
                        egui::ComboBox::from_id_salt("letzte_ids")
                            .selected_text(i18n::t("start.recent_short"))
                            .width(86.0)
                            .show_ui(ui, |ui| {
                                for e in recent.iter().take(8) {
                                    if ui.selectable_label(false, e.label()).clicked() {
                                        self.partner_id = e.id.clone();
                                        self.partner_pw =
                                            self.book.password(&e.id).unwrap_or_default();
                                    }
                                }
                            });
                    }
                });
                ui.add_space(6.0);
                label_small(ui, i18n::t("start.password"));
                ui.add(
                    egui::TextEdit::singleline(&mut self.partner_pw)
                        .password(true)
                        .desired_width(190.0)
                        .margin(egui::Margin::symmetric(8, 4))
                        .hint_text(i18n::t("start.pw_hint")),
                );
                ui.add_space(6.0);
                ui.horizontal(|ui| {
                    if accent_button(ui, i18n::t("start.connect"), !connecting).clicked() {
                        self.start_session();
                    }
                    if ui
                        .add_enabled(
                            !connecting,
                            ghost(egui::vec2(120.0, 30.0), i18n::t("start.ask")),
                        )
                        .on_hover_text(i18n::t("start.ask_tip"))
                        .clicked()
                    {
                        self.start_ask_session();
                    }
                });
                // what happened to the last try, in plain words
                let failed = self.shared.viewer_failed.load(Ordering::Relaxed);
                if connecting || failed {
                    let vs = self
                        .shared
                        .viewer_status
                        .lock()
                        .unwrap_or_else(|p| p.into_inner())
                        .clone();
                    if !vs.is_empty() {
                        ui.add_space(6.0);
                        ui.label(egui::RichText::new(vs).size(12.0).color(if failed {
                            egui::Color32::from_rgb(230, 110, 90)
                        } else {
                            p.muted
                        }));
                    }
                    if failed && !connecting && ghost_button(ui, i18n::t("viewer.retry")).clicked() {
                        if self.last_ask {
                            self.start_ask_session();
                        } else {
                            self.start_session();
                        }
                    }
                }
                ui.add_space(6.0);
                ui.horizontal(|ui| {
                    let mut remember = self.remember_pw;
                    if check(ui, &mut remember, i18n::t("start.remember")).changed() {
                        self.remember_pw = remember;
                    }
                    let mut game = self.shared.game_mode();
                    if check(ui, &mut game, i18n::t("start.game"))
                        .on_hover_text(i18n::t("start.game_tip"))
                        .changed()
                    {
                        self.set_mode(if game {
                            proto::MODE_GAME
                        } else {
                            proto::MODE_ADMIN
                        });
                    }
                });
            });

            // letzte Verbindungen, kurz
            let recent = self.book.sorted();
            if !recent.is_empty() {
                ui.add_space(8.0);
                section(ui, i18n::t("start.recent"));
                let mut go: Option<String> = None;
                card(ui, |ui| {
                    for e in recent.iter().take(4) {
                        let on = self.presence.online(&e.id);
                        ui.horizontal(|ui| {
                            icons::show(ui, device_symbol(&e.label()), 15.0, row_color(on, false));
                            ui.label(
                                egui::RichText::new(e.label())
                                    .size(12.5)
                                    .color(row_color(on, true)),
                            );
                            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                                if icon_ghost(ui, "connect", i18n::t("dev.connect")).clicked() {
                                    go = Some(e.id.clone());
                                }
                                status_pill(
                                    ui,
                                    on,
                                    if on {
                                        i18n::t("st.online")
                                    } else {
                                        i18n::t("st.offline")
                                    },
                                );
                            });
                        });
                    }
                });
                if let Some(id) = go {
                    self.connect_to(&id);
                }
            }

            // ------------------------------------------------- Friends
            // Short version of the friends list: whoever is online is one
            // click away. The whole page is in the bar on the left -
            // here is only what you really need on the start page.
            {
                let dienst = self.freunde.clone();
                let liste = dienst.as_ref().map(|d| d.liste()).unwrap_or_default();
                let offen = dienst.as_ref().map(|d| d.eingehende().len()).unwrap_or(0);
                ui.add_space(8.0);
                section(ui, i18n::t("nav.friends"));
                let mut go_f: Option<String> = None;
                let mut oeffnen = false;
                card(ui, |ui| {
                    if offen > 0 {
                        ui.horizontal(|ui| {
                            ui.painter().circle_filled(
                                ui.cursor().min + egui::vec2(4.0, 9.0),
                                4.0,
                                egui::Color32::from_rgb(0xef, 0x44, 0x44),
                            );
                            ui.add_space(12.0);
                            if ui
                                .link(
                                    egui::RichText::new(i18n::tf(
                                        "friends.pending",
                                        &offen.to_string(),
                                    ))
                                    .size(12.5)
                                    .strong(),
                                )
                                .clicked()
                            {
                                oeffnen = true;
                            }
                        });
                    }
                    if liste.is_empty() {
                        ui.label(
                            egui::RichText::new(i18n::t("friends.none"))
                                .size(12.0)
                                .color(p.muted),
                        );
                    }
                    for f in liste.iter().take(4) {
                        let on = f.online;
                        ui.horizontal(|ui| {
                            icons::show(ui, "friends", 15.0, row_color(on, false));
                            ui.label(
                                egui::RichText::new(f.anzeige())
                                    .size(12.5)
                                    .color(row_color(on, true)),
                            );
                            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                                if icon_ghost(ui, "connect", i18n::t("dev.connect")).clicked() {
                                    go_f = Some(f.fvid.clone());
                                }
                                status_pill(
                                    ui,
                                    on,
                                    if on {
                                        i18n::t("st.online")
                                    } else {
                                        i18n::t("st.offline")
                                    },
                                );
                            });
                        });
                    }
                    ui.add_space(4.0);
                    if ui
                        .link(
                            egui::RichText::new(i18n::t("friends.open"))
                                .size(12.0)
                                .color(p.accent),
                        )
                        .clicked()
                    {
                        oeffnen = true;
                    }
                });
                if let Some(id) = go_f {
                    self.connect_to(&id);
                } else if oeffnen {
                    self.view = View::Freunde;
                }
            }
        });

        if !self.hint.is_empty() {
            ui.add_space(6.0);
            ui.label(egui::RichText::new(&self.hint).color(p.accent).size(12.0));
        }
    }

    /// Microphone and audio as two icon switches - the same controls in the dashboard and in the
    /// session bar.
    pub(crate) fn voice_buttons(&mut self, ui: &mut egui::Ui, size: f32) {
        let v = self.shared.voice.clone();
        let mic = v.mic.load(Ordering::Relaxed);
        let snd = v.speaker.load(Ordering::Relaxed);
        if icon_toggle(
            ui,
            if mic { "mic" } else { "mic-off" },
            mic,
            size,
            if mic {
                i18n::t("sess.mic_on")
            } else {
                i18n::t("sess.mic_off")
            },
        )
        .clicked()
        {
            v.mic.store(!mic, Ordering::Relaxed);
        }
        if icon_toggle(
            ui,
            if snd { "sound" } else { "sound-off" },
            snd,
            size,
            if snd {
                i18n::t("sess.snd_on")
            } else {
                i18n::t("sess.snd_off")
            },
        )
        .clicked()
        {
            v.speaker.store(!snd, Ordering::Relaxed);
        }
    }

    /// Connects to an entry from the list.
    /// Is this the own ID? Then a connection is pointless - the
    /// screen would show itself until nothing works anymore.
    /// What a device is called in the list. Own name beats everything;
    /// otherwise the list takes the name the machine itself registered
    /// at the relay ("Flei-ONE") - a bare number tells nobody anything.
    pub(crate) fn dev_label(&self, e: &partners::Partner) -> String {
        if !e.name.trim().is_empty() {
            return e.name.clone();
        }
        if let Some(p) = self.presence.get(&e.id) {
            let n = p.name.trim();
            if !n.is_empty() {
                return n.to_string();
            }
        }
        partners::pretty_id(&e.id)
    }

    pub(crate) fn is_me(&self, id: &str) -> bool {
        let ziffern: String = id.chars().filter(|c| c.is_ascii_digit()).collect();
        let me = self.shared.my_id.lock_ok().clone();
        !ziffern.is_empty() && ziffern == me
    }

    pub(crate) fn connect_to(&mut self, id: &str) {
        if self.is_me(id) {
            self.hint = i18n::t("start.self").to_string();
            return;
        }
        self.partner_id = id.to_string();
        self.partner_pw = self.book.password(id).unwrap_or_default();
        self.selected = Some(id.to_string());
        if self.partner_pw.is_empty() {
            // No saved password: do not silently switch to "request"
            // - ask for the password first. Whoever does not know one
            // can still send a request in the dialog.
            self.pw_ask = Some(id.to_string());
        } else {
            self.start_session();
        }
    }

    /// Narrow icon column on the left - the three areas of the program.
    pub(crate) fn rail(&mut self, ctx: &egui::Context) {
        let p = theme::palette();
        // Previously the bar was completely tinted in the accent color in light mode -
        // the white icons disappeared in it. Now it is always
        // a calm surface, only what is currently selected gets tinted.
        let rail_bg = p.card;
        egui::SidePanel::left("fv_rail")
            .exact_width(56.0)
            .resizable(false)
            .frame(
                egui::Frame::NONE
                    .fill(rail_bg)
                    .inner_margin(egui::Margin::symmetric(0, 10)),
            )
            .show(ctx, |ui| {
                ui.vertical_centered(|ui| {
                    logo(ui);
                    ui.add_space(10.0);
                    // The friends service now runs from the first frame on.
                    // Previously it only started when the page was opened - a
                    // friend request nobody sees because the
                    // program never asks for it is not a request.
                    let fd = self.freunde_dienst();
                    let offen = fd.eingehende().len() + fd.einladungen().len();
                    for (v, name, tip) in [
                        (View::Start, "home", i18n::t("nav.start")),
                        (View::Devices, "devices", i18n::t("nav.devices")),
                        (View::Meet, "meet", i18n::t("nav.meet")),
                        (View::Freunde, "friends", i18n::t("nav.friends")),
                        (View::Settings, "settings", i18n::t("nav.settings")),
                    ] {
                        let sel = self.view == v;
                        // First claim the area, then draw yourself: only this way
                        // does EVERY button react to the mouse and not just the one
                        // you are standing on.
                        let (rect, r) =
                            ui.allocate_exact_size(egui::vec2(38.0, 34.0), egui::Sense::click());
                        let hov = r.hovered();
                        let fill = if sel {
                            p.accent.gamma_multiply(if p.dark { 0.20 } else { 0.14 })
                        } else if hov {
                            p.card_hi
                        } else {
                            egui::Color32::TRANSPARENT
                        };
                        if fill != egui::Color32::TRANSPARENT {
                            ui.painter().rect_filled(rect, 9.0, fill);
                        }
                        if sel {
                            // narrow bar on the left: shows the page even without color
                            ui.painter().rect_filled(
                                egui::Rect::from_min_size(
                                    egui::pos2(rect.min.x - 7.0, rect.center().y - 9.0),
                                    egui::vec2(3.0, 18.0),
                                ),
                                2.0,
                                p.accent,
                            );
                        }
                        let fg = if sel {
                            p.accent
                        } else if hov {
                            p.text
                        } else {
                            p.muted
                        };
                        ui.put(rect, icons::image(name, 19.0, fg));
                        // Open requests as a dot on the icon - visible
                        // without having to open the page.
                        let mut tip = tip.to_string();
                        if v == View::Freunde && offen > 0 {
                            ui.painter().circle_filled(
                                egui::pos2(rect.right() - 7.0, rect.top() + 7.0),
                                4.0,
                                egui::Color32::from_rgb(0xef, 0x44, 0x44),
                            );
                            tip = format!("{} ({} offen)", tip, offen);
                        }
                        let r = zeigefinger(r).on_hover_text(tip);
                        if r.clicked() {
                            self.view = v;
                        }
                        ui.add_space(4.0);
                    }
                });
            });
    }

}
