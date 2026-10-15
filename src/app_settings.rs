//! Split out of main.rs (r35): mechanical move, behaviour unchanged.
use super::*;

impl App {
    /// Settings: the areas on the left, the selected area on the right.
    pub(crate) fn settings_view(&mut self, ui: &mut egui::Ui) {
        let p = theme::palette();
        ui.horizontal_top(|ui| {
            // ---- Bereichsliste
            ui.vertical(|ui| {
                ui.set_width(178.0);
                for (tab, icon, label) in [
                    (SettingsTab::General, "settings", i18n::t("set.general")),
                    (SettingsTab::Access, "shield", i18n::t("set.access")),
                    (SettingsTab::Account, "user", i18n::t("set.account")),
                    (SettingsTab::Deploy, "laptop", i18n::t("set.deploy")),
                    (SettingsTab::Audio, "mic", i18n::t("set.audio")),
                    (SettingsTab::Look, "palette", i18n::t("set.look")),
                    (SettingsTab::Feedback, "chat", i18n::t("set.feedback")),
                    (SettingsTab::About, "eye", i18n::t("set.about")),
                ] {
                    let sel = self.stab == tab;
                    // draw it yourself, otherwise the fixed fill of the
                    // egui button swallows every reaction to the mouse
                    let (rect, r) =
                        ui.allocate_exact_size(egui::vec2(166.0, 32.0), egui::Sense::click());
                    let hov = r.hovered();
                    let fill = if sel {
                        p.accent.gamma_multiply(0.14)
                    } else if hov {
                        p.card_hi
                    } else {
                        egui::Color32::TRANSPARENT
                    };
                    if fill != egui::Color32::TRANSPARENT {
                        ui.painter().rect_filled(rect, 7.0, fill);
                    }
                    let fg = if sel {
                        p.accent
                    } else if hov {
                        p.text
                    } else {
                        p.muted
                    };
                    let icon_rect = egui::Rect::from_min_size(
                        egui::pos2(rect.min.x + 9.0, rect.center().y - 8.0),
                        egui::vec2(16.0, 16.0),
                    );
                    ui.put(icon_rect, icons::image(icon, 16.0, fg));
                    ui.painter().text(
                        egui::pos2(rect.min.x + 34.0, rect.center().y),
                        egui::Align2::LEFT_CENTER,
                        label,
                        egui::FontId::proportional(13.0),
                        if sel { p.text } else { fg },
                    );
                    let r = zeigefinger(r);
                    if r.clicked() {
                        self.stab = tab;
                    }
                    ui.add_space(2.0);
                }
            });
            ui.add_space(14.0);
            // ---- Inhalt
            ui.vertical(|ui| {
                ui.set_width(ui.available_width());
                match self.stab {
                    SettingsTab::General => self.set_general(ui),
                    SettingsTab::Access => self.set_access(ui),
                    SettingsTab::Account => self.set_account(ui),
                    SettingsTab::Deploy => self.set_deploy(ui),
                    SettingsTab::Audio => self.set_audio(ui),
                    SettingsTab::Look => self.set_look(ui),
                    SettingsTab::Feedback => self.set_feedback(ui),
                    SettingsTab::About => self.set_about(ui),
                }
            });
        });
        if !self.hint.is_empty() {
            ui.add_space(6.0);
            ui.label(egui::RichText::new(&self.hint).color(p.accent).size(12.0));
        }
    }

    pub(crate) fn set_general(&mut self, ui: &mut egui::Ui) {
        card(ui, |ui| {
            label_small(ui, i18n::t("set.name"));
            // Empty means: nothing of its own is entered yet - then we show
            // the machine name, just as the other side sees it too.
            let mut name = {
                let mut n = self.shared.device_name.lock().unwrap().clone();
                if n.trim().is_empty() {
                    n = presence::machine_name();
                    *self.shared.device_name.lock().unwrap() = n.clone();
                }
                n
            };
            if ui
                .add(
                    egui::TextEdit::singleline(&mut name)
                        .desired_width(240.0)
                        .margin(egui::Margin::symmetric(8, 4))
                        .hint_text(i18n::t("set.name_hint")),
                )
                .changed()
            {
                let clean = presence::clean(&name);
                *self.shared.device_name.lock().unwrap() = clean.clone();
                let _ = presence::save_device_name(&clean);
            }
            ui.add_space(8.0);
            label_small(ui, i18n::t("set.lang"));
            let mut a = self.look.clone();
            let cur = i18n::LANGS
                .iter()
                .find(|(c, _)| *c == a.lang)
                .map(|(_, n)| *n)
                .unwrap_or("Deutsch");
            egui::ComboBox::from_id_salt("lang_pick")
                .selected_text(cur)
                .width(160.0)
                .show_ui(ui, |ui| {
                    for (code, name) in i18n::LANGS.iter() {
                        if ui.selectable_label(a.lang == *code, *name).clicked() {
                            a.lang = code.to_string();
                        }
                    }
                });
            if a != self.look {
                self.apply_look(ui.ctx(), a);
            }
        });
        ui.add_space(10.0);
        self.install_card(ui);
    }

    /// Installation: portabel laufen lassen oder richtig einrichten.
    pub(crate) fn install_card(&mut self, ui: &mut egui::Ui) {
        let p = theme::palette();
        label_small(ui, i18n::t("set.install"));
        card(ui, |ui| {
            let installed = setup::is_installed();
            let here = setup::running_installed();
            let state = if here {
                i18n::tf("set.install_here", &setup::install_dir().display().to_string())
            } else if installed {
                i18n::tf("set.install_other", &setup::install_dir().display().to_string())
            } else {
                i18n::t("set.install_portable").to_string()
            };
            ui.horizontal(|ui| {
                dot(ui, here);
                ui.label(egui::RichText::new(state).size(12.5).color(p.text));
            });
            ui.add_space(6.0);
            ui.label(
                egui::RichText::new(i18n::t("set.install_note"))
                    .size(11.5)
                    .color(p.muted),
            );
            ui.add_space(8.0);
            ui.horizontal(|ui| {
                if !installed {
                    if ghost_button(ui, i18n::t("set.install_do")).clicked() {
                        let flags = if self.install_service {
                            "--install --with-service"
                        } else {
                            "--install"
                        };
                        match service::elevate(flags) {
                            Ok(()) => self.hint = i18n::t("set.install_running").to_string(),
                            Err(e) => self.hint = format!("{}", e),
                        }
                    }
                    ui.add_space(8.0);
                    check(
                        ui,
                        &mut self.install_service,
                        i18n::t("set.install_with_service"),
                    )
                    .on_hover_text(i18n::t("set.service_tip"));
                } else {
                    // Repair: files freshly on top (stays without an
                    // additional question, the rights have already been given)
                    if ghost_button(ui, i18n::t("set.repair")).clicked() {
                        let flags = if self.service_on {
                            "--install --with-service"
                        } else {
                            "--install"
                        };
                        match service::elevate(flags) {
                            Ok(()) => self.hint = i18n::t("set.repair_running").to_string(),
                            Err(e) => self.hint = format!("{}", e),
                        }
                    }
                    ui.add_space(8.0);
                    if ghost_button(ui, i18n::t("set.uninstall_do")).clicked() {
                        self.uninstall_ask = true;
                    }
                }
            });
        });
    }

    pub(crate) fn set_access(&mut self, ui: &mut egui::Ui) {
        card(ui, |ui| {
            let mut boot = self.autostart;
            if check(ui, &mut boot, i18n::t("set.autostart"))
                .on_hover_text(i18n::t("set.autostart_tip"))
                .changed()
            {
                match autostart::set(boot) {
                    Ok(()) => self.autostart = boot,
                    Err(e) => self.hint = format!("{}", e),
                }
            }
            ui.add_space(4.0);
            let mut svc = self.service_on;
            if check(ui, &mut svc, i18n::t("set.service"))
                .on_hover_text(i18n::t("set.service_tip"))
                .changed()
            {
                let flag = if svc {
                    "--install-service"
                } else {
                    "--uninstall-service"
                };
                match service::elevate(flag) {
                    Ok(()) => self.hint = i18n::t("set.service_tip").to_string(),
                    Err(e) => self.hint = format!("{}", e),
                }
            }
            ui.add_space(4.0);
            let mut clip = self.shared.clip_on.load(Ordering::Relaxed);
            if check(ui, &mut clip, i18n::t("set.clip"))
                .on_hover_text(i18n::t("set.clip_tip"))
                .changed()
            {
                self.shared.clip_on.store(clip, Ordering::Relaxed);
                ident::set_clipboard(clip);
            }
            ui.add_space(4.0);
            let mut keep = self.pw_fixed;
            if check(ui, &mut keep, i18n::t("start.keep_pw"))
                .on_hover_text(i18n::t("start.keep_pw_tip"))
                .changed()
            {
                self.pw_fixed = keep;
                let pw = self.shared.password.lock().unwrap().clone();
                let store = if keep { Some(pw.as_str()) } else { None };
                let _ = ident::set_fixed_password(store);
            }
        });
        ui.add_space(10.0);
        self.pw_card(ui);
    }

    /// Fixed passwords: any number, always as dots, with a label -
    /// like an array in the Unity inspector.
    pub(crate) fn pw_card(&mut self, ui: &mut egui::Ui) {
        let p = theme::palette();
        label_small(ui, i18n::t("set.pw_perm"));
        card(ui, |ui| {
            ui.label(
                egui::RichText::new(i18n::t("set.pw_perm_tip"))
                    .size(11.5)
                    .color(p.muted),
            );
            ui.add_space(8.0);
            let mut remove: Option<usize> = None;
            let arm_before = self.pw_del_arm;
            let mut dirty = false;
            for i in 0..self.pw_list.len() {
                let shown = self.pw_show == Some(i);
                ui.horizontal(|ui| {
                    let mut label = self.pw_list[i].label.clone();
                    if ui
                        .add(
                            egui::TextEdit::singleline(&mut label)
                                .desired_width(118.0)
                                .hint_text(i18n::t("set.pw_label_hint"))
                                .margin(egui::Margin::symmetric(7, 4)),
                        )
                        .changed()
                    {
                        self.pw_list[i].label = label;
                        dirty = true;
                    }
                    ui.add_space(8.0);
                    if shown {
                        // read-only: a stored password is never edited in
                        // place, a change = delete + add (r10)
                        ui.add(
                            egui::Label::new(
                                egui::RichText::new(self.pw_list[i].pw.clone())
                                    .size(14.0)
                                    .family(egui::FontFamily::Monospace)
                                    .color(p.text),
                            )
                            .selectable(true),
                        );
                    } else {
                        ui.add(egui::Label::new(
                            egui::RichText::new(pwlist::masked(&self.pw_list[i].pw))
                                .size(16.0)
                                .family(egui::FontFamily::Monospace)
                                .color(p.text),
                        ));
                    }
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        let armed = self.pw_del_arm == Some(i);
                        let tip = if armed {
                            i18n::t("set.pw_del_sure")
                        } else {
                            i18n::t("set.pw_del")
                        };
                        if icon_ghost(ui, if armed { "check" } else { "trash" }, tip).clicked() {
                            if armed {
                                remove = Some(i);
                            } else {
                                self.pw_del_arm = Some(i);
                            }
                        }
                        if icon_ghost(
                            ui,
                            "eye",
                            if shown {
                                i18n::t("set.pw_hide")
                            } else {
                                i18n::t("set.pw_show")
                            },
                        )
                        .clicked()
                        {
                            self.pw_show = if shown { None } else { Some(i) };
                        }
                        if icon_ghost(ui, "copy", i18n::t("start.copy")).clicked() {
                            let pw = self.pw_list[i].pw.clone();
                            ui.ctx().copy_text(pw);
                        }
                    });
                });
                ui.add_space(4.0);
            }
            if self.pw_list.is_empty() {
                ui.label(
                    egui::RichText::new(i18n::t("set.pw_empty"))
                        .size(12.0)
                        .color(p.muted),
                );
            }
            if remove.is_none() && arm_before.is_some() && self.pw_del_arm == arm_before
                && ui.input(|i| i.pointer.any_click())
            {
                // clicked somewhere else: disarm the delete button
                self.pw_del_arm = None;
            }
            if let Some(i) = remove {
                self.pw_list.remove(i);
                self.pw_show = None;
                self.pw_del_arm = None;
                dirty = true;
            }
            ui.add_space(8.0);
            divider(ui);
            ui.add_space(8.0);
            ui.horizontal(|ui| {
                ui.add(
                    egui::TextEdit::singleline(&mut self.pw_new_label)
                        .desired_width(118.0)
                        .hint_text(i18n::t("set.pw_label_hint"))
                        .margin(egui::Margin::symmetric(7, 4)),
                );
                ui.add_space(8.0);
                let enter = ui
                    .add(
                        egui::TextEdit::singleline(&mut self.pw_new)
                            .desired_width(160.0)
                            .password(true)
                            .hint_text(i18n::t("set.pw_value"))
                            .margin(egui::Margin::symmetric(7, 4)),
                    )
                    .lost_focus()
                    && ui.input(|i| i.key_pressed(egui::Key::Enter));
                ui.add_space(8.0);
                if icon_ghost(ui, "plus", i18n::t("set.pw_add")).clicked() || enter {
                    let pw = self.pw_new.trim().to_string();
                    match pwlist::why_not(&self.pw_list, &pw) {
                        Some(err) => self.hint = i18n::t(err).to_string(),
                        None => {
                            let label = if self.pw_new_label.trim().is_empty() {
                                format!("#{}", self.pw_list.len() + 1)
                            } else {
                                self.pw_new_label.trim().to_string()
                            };
                            self.pw_list.push(pwlist::Entry { label, pw });
                            self.pw_new.clear();
                            self.pw_new_label.clear();
                            dirty = true;
                        }
                    }
                }
            });
            if dirty {
                if let Err(e) = pwlist::save(&self.pw_list) {
                    self.hint = format!("{}", e);
                }
            }
        });
    }

    /// Settings -> Account. Without login everything works as before; with
    /// login the device list additionally lives with the FleiTec account.
    pub(crate) fn set_account(&mut self, ui: &mut egui::Ui) {
        let p = theme::palette();
        self.take_sync_result();
        label_small(ui, i18n::t("acc.title"));
        card(ui, |ui| {
            match self.acc.clone() {
                None => {
                    ui.label(
                        egui::RichText::new(i18n::t("acc.why"))
                            .size(12.0)
                            .color(p.muted),
                    );
                    ui.add_space(8.0);
                    label_small(ui, i18n::t("acc.user"));
                    ui.add(
                        egui::TextEdit::singleline(&mut self.acc_user)
                            .desired_width(240.0)
                            .margin(egui::Margin::symmetric(8, 4)),
                    );
                    ui.add_space(6.0);
                    label_small(ui, i18n::t("acc.pass"));
                    let enter = ui
                        .add(
                            egui::TextEdit::singleline(&mut self.acc_pass)
                                .desired_width(240.0)
                                .password(true)
                                .margin(egui::Margin::symmetric(8, 4)),
                        )
                        .lost_focus()
                        && ui.input(|i| i.key_pressed(egui::Key::Enter));
                    ui.add_space(8.0);
                    let go = accent_button(ui, i18n::t("acc.login"), true).clicked() || enter;
                    if go {
                        self.do_login();
                    }
                }
                Some(sess) => {
                    ui.horizontal(|ui| {
                        icons::show(ui, "user", 16.0, p.accent);
                        ui.add_space(4.0);
                        ui.label(egui::RichText::new(&sess.user).size(15.0).strong());
                    });
                    ui.add_space(2.0);
                    let when = if sess.synced == 0 {
                        i18n::t("acc.never").to_string()
                    } else {
                        presence::ago_ms(sess.synced * 1000)
                    };
                    ui.label(
                        egui::RichText::new(format!("{}: {}", i18n::t("acc.last_sync"), when))
                            .size(12.0)
                            .color(p.muted),
                    );
                    ui.add_space(10.0);
                    ui.horizontal(|ui| {
                        let busy = self.acc_busy.load(Ordering::Relaxed);
                        if accent_button(ui, i18n::t("acc.sync_now"), !busy).clicked() {
                            self.start_sync(true);
                        }
                        if ghost_button(ui, i18n::t("acc.logout")).clicked() {
                            account::forget();
                            self.acc = None;
                            self.acc_msg = i18n::t("acc.logged_out").to_string();
                        }
                    });
                    ui.add_space(6.0);
                    let n = self.book.sorted().len();
                    ui.label(
                        egui::RichText::new(i18n::tf("acc.count", &n.to_string()))
                            .size(12.0)
                            .color(p.muted),
                    );
                }
            }
        });
        if !self.acc_msg.is_empty() {
            ui.add_space(8.0);
            ui.label(egui::RichText::new(&self.acc_msg).size(12.0).color(p.accent));
        }
        ui.add_space(10.0);
        label_small(ui, i18n::t("acc.privacy_head"));
        card(ui, |ui| {
            ui.label(
                egui::RichText::new(i18n::t("acc.privacy"))
                    .size(12.0)
                    .color(p.muted),
            );
        });
    }

}
