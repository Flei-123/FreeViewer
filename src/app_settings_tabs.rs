//! Split out of main.rs (r35): mechanical move, behaviour unchanged.
use super::*;

impl App {
    pub(crate) fn set_audio(&mut self, ui: &mut egui::Ui) {
        let p = theme::palette();
        let mut look = self.look.clone();
        // Fetch the device list once - cpal asks Windows for this, which
        // costs a few milliseconds depending on the driver.
        if self.snd_in.is_empty() && self.snd_out.is_empty() {
            self.snd_in = audio::input_devices();
            self.snd_out = audio::output_devices();
            self.snd_default = audio::default_names();
        }
        let ins = self.snd_in.clone();
        let outs = self.snd_out.clone();
        let defs = self.snd_default.clone();
        card(ui, |ui| {
            label_small(ui, i18n::t("set.audio_default"));
            if check(ui, &mut look.mic_on, i18n::t("set.mic_default"))
                .on_hover_text(i18n::t("set.mic_default_tip"))
                .changed()
            {}
            if check(ui, &mut look.snd_on, i18n::t("set.snd_default"))
                .changed()
            {}
            ui.add_space(6.0);
            divider(ui);
            ui.add_space(5.0);
            label_small(ui, i18n::t("set.audio_now"));
            ui.horizontal(|ui| {
                self.voice_buttons(ui, 18.0);
                ui.add_space(8.0);
                let v = self.shared.voice.clone();
                level_bar(
                    ui,
                    v.level_out.load(Ordering::Relaxed),
                    v.level_in.load(Ordering::Relaxed),
                );
            });
            ui.add_space(6.0);
            ui.label(
                egui::RichText::new(i18n::t("set.audio_note"))
                    .size(11.5)
                    .color(p.muted),
            );
            ui.add_space(6.0);
            divider(ui);
            ui.add_space(5.0);
            label_small(ui, i18n::t("set.dev_pick"));
            if let Some(sel) = device_row(
                ui,
                "fv_mic_dev",
                i18n::t("set.mic_dev"),
                &look.mic_dev,
                &defs.0,
                &ins,
            ) {
                look.mic_dev = sel;
            }
            ui.add_space(4.0);
            if let Some(sel) = device_row(
                ui,
                "fv_spk_dev",
                i18n::t("set.spk_dev"),
                &look.spk_dev,
                &defs.1,
                &outs,
            ) {
                look.spk_dev = sel;
            }
            ui.add_space(6.0);
            if ghost_button(ui, i18n::t("set.dev_refresh")).clicked() {
                self.snd_in.clear();
                self.snd_out.clear();
            }
        });
        if look != self.look {
            self.apply_look(ui.ctx(), look);
        }
    }

    pub(crate) fn set_look(&mut self, ui: &mut egui::Ui) {
        let mut a = self.look.clone();

        label_small(ui, i18n::t("set.template"));
        card(ui, |ui| {
            ui.horizontal_wrapped(|ui| {
                for (key, name, pal) in theme::PRESETS.iter() {
                    let sel = a.preset == *key;
                    let (rect, resp) =
                        ui.allocate_exact_size(egui::vec2(140.0, 74.0), egui::Sense::click());
                    let pt = ui.painter();
                    pt.rect_filled(rect, 8.0, pal.bg);
                    pt.rect_filled(
                        egui::Rect::from_min_size(
                            rect.min + egui::vec2(9.0, 9.0),
                            egui::vec2(122.0, 18.0),
                        ),
                        5.0,
                        pal.card,
                    );
                    pt.rect_filled(
                        egui::Rect::from_min_size(
                            rect.min + egui::vec2(9.0, 33.0),
                            egui::vec2(54.0, 10.0),
                        ),
                        4.0,
                        pal.accent,
                    );
                    pt.rect_filled(
                        egui::Rect::from_min_size(
                            rect.min + egui::vec2(68.0, 33.0),
                            egui::vec2(38.0, 10.0),
                        ),
                        4.0,
                        pal.card_hi,
                    );
                    pt.text(
                        rect.min + egui::vec2(10.0, 50.0),
                        egui::Align2::LEFT_TOP,
                        name,
                        egui::FontId::proportional(12.0),
                        pal.text,
                    );
                    pt.rect_stroke(
                        rect,
                        8.0,
                        egui::Stroke::new(
                            if sel { 2.0 } else { 1.0 },
                            if sel { theme::accent() } else { theme::line() },
                        ),
                        egui::StrokeKind::Inside,
                    );
                    if resp.clicked() {
                        a.preset = key.to_string();
                    }
                    ui.add_space(6.0);
                }
            });
        });

        ui.add_space(8.0);
        label_small(ui, i18n::t("set.accent"));
        card(ui, |ui| {
            ui.horizontal(|ui| {
                if ui
                    .selectable_label(a.accent.is_none(), i18n::t("set.accent_preset"))
                    .clicked()
                {
                    a.accent = None;
                }
                ui.add_space(6.0);
                for (name, col) in theme::ACCENTS.iter() {
                    let want = [col.r(), col.g(), col.b()];
                    let sel = a.accent == Some(want);
                    let (rect, resp) =
                        ui.allocate_exact_size(egui::vec2(22.0, 22.0), egui::Sense::click());
                    ui.painter().rect_filled(rect, 11.0, *col);
                    if sel {
                        ui.painter().rect_stroke(
                            rect.expand(2.0),
                            13.0,
                            egui::Stroke::new(2.0, theme::text()),
                            egui::StrokeKind::Outside,
                        );
                    }
                    if resp.on_hover_text(*name).clicked() {
                        a.accent = Some(want);
                    }
                    ui.add_space(4.0);
                }
                ui.add_space(6.0);
                let cur = theme::accent();
                let mut rgb = a.accent.unwrap_or([cur.r(), cur.g(), cur.b()]);
                if ui.color_edit_button_srgb(&mut rgb).changed() {
                    a.accent = Some(rgb);
                }
            });
        });

        ui.add_space(8.0);
        card(ui, |ui| {
            let pct = format!("{:.0} %", a.scale * 100.0);
            slider_row(ui, i18n::t("set.size"), &mut a.scale, 0.85..=1.35, &pct);
            ui.add_space(6.0);
            let mut r = a.radius as f32;
            let rtxt = format!("{:.0} px", r);
            if slider_row(ui, i18n::t("set.radius"), &mut r, 0.0..=16.0, &rtxt) {
                a.radius = r.round() as u8;
            }
            ui.add_space(6.0);
            if ghost_button(ui, i18n::t("set.reset")).clicked() {
                let lang = a.lang.clone();
                a = theme::Appearance {
                    lang,
                    ..Default::default()
                };
            }
        });

        if a != self.look {
            self.apply_look(ui.ctx(), a);
        }
    }

    /// Update card: version, search, automatic.
    /// Feedback: bug or idea, straight from the program.
    pub(crate) fn set_feedback(&mut self, ui: &mut egui::Ui) {
        let p = theme::palette();
        card(ui, |ui| {
            ui.label(
                egui::RichText::new(i18n::t("fb.intro"))
                    .size(11.5)
                    .color(p.muted),
            );
            ui.add_space(6.0);
            ui.horizontal(|ui| {
                if icons::text_button(ui, "shield", i18n::t("fb.bug"), self.fb_bug).clicked() {
                    self.fb_bug = true;
                }
                if icons::text_button(ui, "star", i18n::t("fb.idea"), !self.fb_bug).clicked() {
                    self.fb_bug = false;
                }
            });
            ui.add_space(6.0);
            ui.add(
                egui::TextEdit::multiline(&mut self.fb_text)
                    .desired_rows(4)
                    .desired_width(ui.available_width().min(430.0))
                    .hint_text(i18n::t("fb.hint")),
            );
            ui.add_space(4.0);
            ui.add(
                egui::TextEdit::singleline(&mut self.fb_contact)
                    .desired_width(260.0)
                    .margin(egui::Margin::symmetric(8, 4))
                    .hint_text(i18n::t("fb.contact")),
            );
            ui.add_space(6.0);
            let busy = feedback::STATE.load(Ordering::Relaxed) == 1;
            ui.horizontal(|ui| {
                if accent_button(ui, i18n::t("fb.send"), !busy && !self.fb_text.trim().is_empty())
                    .clicked()
                {
                    let id = self.shared.my_id.lock().unwrap().clone();
                    let dev = self.shared.device_name.lock().unwrap().clone();
                    feedback::send(
                        if self.fb_bug { "fehler" } else { "idee" },
                        &self.fb_text,
                        &self.fb_contact,
                        &id,
                        &dev,
                    );
                }
                let st = feedback::STATE.load(Ordering::Relaxed);
                if st == 2 {
                    self.fb_text.clear();
                }
                let msg = feedback::MESSAGE.lock().unwrap().clone();
                if !msg.is_empty() {
                    ui.label(
                        egui::RichText::new(msg)
                            .size(11.5)
                            .color(if st == 3 { p.muted } else { p.green }),
                    );
                }
            });
        });
    }

    pub(crate) fn set_about(&mut self, ui: &mut egui::Ui) {
        card(ui, |ui| {
            ui.horizontal(|ui| {
                icons::show(ui, "shield", 16.0, theme::green());
                ui.label(
                    egui::RichText::new(i18n::t("set.e2e"))
                        .size(12.5)
                        .color(theme::text()),
                );
            });
            ui.add_space(6.0);
            divider(ui);
            ui.add_space(5.0);
            info_row(ui, i18n::t("set.version"), update::VERSION);
            info_row(ui, i18n::t("set.relay"), &self.shared.relay_url);
            ui.add_space(3.0);
            ui.label(
                egui::RichText::new(i18n::t("set.e2e_note"))
                    .size(11.0)
                    .color(theme::muted()),
            );
            info_row(
                ui,
                i18n::t("set.config"),
                &ident::config_dir().display().to_string(),
            );
            ui.add_space(6.0);
            self.update_ui(ui);
        });
        #[cfg(feature = "license")]
        self.license_card(ui);
    }

    /// License card in the settings: show status, enter or
    /// remove key (X-Remote only).
    #[cfg(feature = "license")]
    pub(crate) fn license_card(&mut self, ui: &mut egui::Ui) {
        ui.add_space(10.0);
        card(ui, |ui| {
            ui.label(egui::RichText::new(i18n::t("lic.title")).size(13.5).color(theme::text()));
            ui.add_space(4.0);
            match license::status() {
                license::Status::Active(lic) => {
                    info_row(ui, i18n::t("lic.holder"), &lic.name);
                    info_row(ui, i18n::t("lic.expires"), &lic.expiry_text());
                    ui.add_space(4.0);
                    if ghost_button(ui, i18n::t("lic.remove")).clicked() {
                        license::revoke();
                        self.lic_show = true;
                    }
                }
                license::Status::Missing => {
                    ui.label(
                        egui::RichText::new(i18n::t("lic.none"))
                            .size(11.5)
                            .color(theme::muted()),
                    );
                }
                license::Status::Invalid(e) => {
                    ui.label(
                        egui::RichText::new(i18n::tf("lic.invalid", &e))
                            .size(11.5)
                            .color(theme::muted()),
                    );
                }
            }
            if !matches!(license::status(), license::Status::Active(_)) {
                ui.add_space(6.0);
                if accent_button(ui, i18n::t("lic.activate"), false).clicked() {
                    self.lic_show = true;
                }
            }
        });
    }

    /// Apply look: save, rebuild style, adjust title bar.
    pub(crate) fn apply_look(&mut self, ctx: &egui::Context, a: theme::Appearance) {
        self.look = a.clone();
        theme::save(&a);
        i18n::set_lang(&a.lang);
        theme::apply(ctx, &a);
        chrome::paint_from_theme();
    }

    /// Confirmation before a permanent password is replaced.
    pub(crate) fn pw_overwrite_ui(&mut self, ctx: &egui::Context) {
        let Some(what) = self.pw_overwrite.clone() else {
            return;
        };
        let mut yes = false;
        let mut no = false;
        let mut old_input = std::mem::take(&mut self.pw_old_input);
        let wrong = self.pw_old_wrong;
        egui::Window::new(i18n::t("pw.overwrite_title"))
            .collapsible(false)
            .resizable(false)
            .anchor(egui::Align2::CENTER_CENTER, [0.0, 0.0])
            .show(ctx, |ui| {
                ui.set_min_width(340.0);
                ui.label(i18n::t("pw.overwrite_text"));
                ui.add_space(8.0);
                // the current password must be typed: a stray click (or
                // somebody walking past the screen) cannot replace it (r10)
                label_small(ui, i18n::t("pw.overwrite_old"));
                let r = ui.add(
                    egui::TextEdit::singleline(&mut old_input)
                        .password(true)
                        .desired_width(240.0)
                        .margin(egui::Margin::symmetric(7, 4)),
                );
                if r.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter)) {
                    yes = true;
                }
                if wrong {
                    ui.label(
                        egui::RichText::new(i18n::t("pw.overwrite_wrong"))
                            .size(12.0)
                            .color(egui::Color32::from_rgb(230, 110, 90)),
                    );
                }
                ui.add_space(8.0);
                ui.horizontal(|ui| {
                    if accent_button(ui, i18n::t("pw.overwrite_yes"), true).clicked() {
                        yes = true;
                    }
                    if ui.add(ghost(egui::vec2(110.0, 34.0), i18n::t("pw.overwrite_no"))).clicked() {
                        no = true;
                    }
                });
            });
        self.pw_old_input = old_input;
        if yes {
            let current = ident::fixed_password()
                .unwrap_or_else(|| self.shared.password.lock().unwrap().clone());
            if !secstore::same_secret(self.pw_old_input.trim(), &current) {
                self.pw_old_wrong = true;
                return;
            }
            let fresh = match what {
                PwOverwrite::Regenerate => ident::random_password(),
                PwOverwrite::Setup(pw) => pw,
            };
            *self.shared.password.lock().unwrap() = fresh.clone();
            let _ = ident::set_fixed_password(Some(fresh.as_str()));
            self.pw_fixed = true;
            self.pw_overwrite = None;
            self.pw_old_input.clear();
            self.pw_old_wrong = false;
        } else if no {
            self.pw_overwrite = None;
            self.pw_old_input.clear();
            self.pw_old_wrong = false;
        }
    }

}
