//! Split out of main.rs (r35): mechanical move, behaviour unchanged.
use crate::lockutil::LockExt;
use super::*;

impl App {
    /// Log in in the background - the UI must not stall meanwhile.
    pub(crate) fn do_login(&mut self) {
        let relay = self.shared.relay_url.clone();
        let user = self.acc_user.trim().to_string();
        let pass = std::mem::take(&mut self.acc_pass);
        if user.is_empty() || pass.is_empty() {
            self.acc_msg = i18n::t("acc.need_both").to_string();
            return;
        }
        self.acc_msg = i18n::t("acc.logging_in").to_string();
        let busy = self.acc_busy.clone();
        let out = self.acc_out.clone();
        if busy.swap(true, Ordering::SeqCst) {
            return;
        }
        std::thread::spawn(move || {
            let res = match account::login(&relay, &user, &pass) {
                Ok(sess) => {
                    let _ = account::save(&sess);
                    account::SyncOut::LoggedIn(sess.user.clone())
                }
                Err(e) => account::SyncOut::Failed(e.to_string()),
            };
            *out.lock_ok() = Some(res);
            busy.store(false, Ordering::SeqCst);
        });
    }

    /// Adressbuch hoch, zusammengefuehrte Liste go_back.
    pub(crate) fn start_sync(&mut self, sichtbar: bool) {
        let Some(sess) = self.acc.clone() else { return };
        if sichtbar {
            self.acc_msg = i18n::t("acc.syncing").to_string();
        }
        self.acc_next = std::time::Instant::now() + Duration::from_secs(120);
        account::sync_async(
            self.shared.relay_url.clone(),
            sess.token.clone(),
            self.book.to_sync(),
            self.acc_out.clone(),
            self.acc_busy.clone(),
        );
    }

    /// Ergebnis eines Hintergrundlaufs abholen (Anmeldung oder Abgleich).
    pub(crate) fn take_sync_result(&mut self) {
        let res = self.acc_out.lock_ok().take();
        let Some(res) = res else { return };
        match res {
            account::SyncOut::Ok { devices, at } => {
                let changed = self.book.merge_remote(&devices);
                if let Some(sess) = self.acc.as_mut() {
                    sess.synced = at;
                    let _ = account::save(sess);
                }
                self.acc_msg = if changed {
                    i18n::t("acc.updated").to_string()
                } else {
                    i18n::t("acc.uptodate").to_string()
                };
            }
            account::SyncOut::LoggedOut => {
                account::forget();
                self.acc = None;
                self.acc_msg = i18n::t("acc.expired").to_string();
            }
            account::SyncOut::LoggedIn(user) => {
                // Login worked - the session is already on disk
                self.acc = account::load();
                self.acc_msg = i18n::tf("acc.hello", &user);
                self.acc_user.clear();
                self.start_sync(false);
            }
            account::SyncOut::Failed(msg) => {
                self.acc_msg = msg;
            }
        }
    }

    /// Einrichten-Tab: per Einmal-Link ein neues Geraet aufsetzen.
    pub(crate) fn set_deploy(&mut self, ui: &mut egui::Ui) {
        let p = theme::palette();
        self.take_setup_result();
        label_small(ui, i18n::t("dep.title"));
        let Some(sess) = self.acc.clone() else {
            card(ui, |ui| {
                ui.label(
                    egui::RichText::new(i18n::t("dep.need_login"))
                        .size(12.0)
                        .color(p.muted),
                );
                ui.add_space(8.0);
                if accent_button(ui, i18n::t("dep.to_account"), true).clicked() {
                    self.stab = SettingsTab::Account;
                }
            });
            return;
        };
        card(ui, |ui| {
            ui.label(
                egui::RichText::new(i18n::t("dep.lead"))
                    .size(12.0)
                    .color(p.muted),
            );
            ui.add_space(8.0);
            label_small(ui, i18n::t("dep.pw"));
            ui.horizontal(|ui| {
                ui.add(
                    egui::TextEdit::singleline(&mut self.dep_pw)
                        .desired_width(200.0)
                        .margin(egui::Margin::symmetric(8, 4)),
                );
                if ghost_button(ui, i18n::t("dep.pw_new")).clicked() {
                    self.dep_pw = ident::random_password();
                }
            });
            ui.add_space(6.0);
            label_small(ui, i18n::t("dep.name_hint"));
            ui.add(
                egui::TextEdit::singleline(&mut self.dep_hint_name)
                    .desired_width(200.0)
                    .margin(egui::Margin::symmetric(8, 4))
                    .hint_text(i18n::t("dep.name_hint_ph")),
            );
            ui.add_space(6.0);
            label_small(ui, i18n::t("dep.count"));
            ui.horizontal(|ui| {
                let mut unbegrenzt = self.dep_count == 0;
                if unbegrenzt {
                    // a gray "0" would be confusing - better the symbol
                    ui.label(
                        egui::RichText::new("∞")
                            .size(19.0)
                            .strong()
                            .color(p.accent),
                    );
                } else {
                    ui.add(
                        egui::DragValue::new(&mut self.dep_count)
                            .range(1..=25)
                            .speed(0.2),
                    );
                }
                ui.add_space(6.0);
                if check(ui, &mut unbegrenzt, i18n::t("dep.unlimited"))
                    .on_hover_text(i18n::t("dep.unlimited_tip"))
                    .changed()
                {
                    // 0 heisst am Relay: so oft wie man will
                    self.dep_count = if unbegrenzt { 0 } else { 1 };
                }
            });
            ui.add_space(8.0);
            let busy = self.acc_busy.load(Ordering::Relaxed);
            let label = if busy {
                i18n::t("dep.making")
            } else {
                i18n::t("dep.make")
            };
            if accent_button(ui, label, !busy && !self.dep_pw.trim().is_empty()).clicked() {
                account::setup_create_async(
                    self.shared.relay_url.clone(),
                    sess.token.clone(),
                    self.dep_pw.trim().to_string(),
                    self.dep_hint_name.trim().to_string(),
                    self.dep_count,
                    self.dep_out.clone(),
                    self.acc_busy.clone(),
                );
            }
            if !self.dep_msg.is_empty() {
                ui.add_space(6.0);
                ui.label(egui::RichText::new(&self.dep_msg).size(12.0).color(p.accent));
            }
        });
        ui.add_space(10.0);
        label_small(ui, i18n::t("dep.status"));
        card(ui, |ui| {
            // As long as the tab is open: check the inbox every 5 s.
            if std::time::Instant::now() >= self.dep_next
                && !self.acc_busy.load(Ordering::Relaxed)
            {
                self.dep_next = std::time::Instant::now() + Duration::from_secs(5);
                account::setup_inbox_async(
                    self.shared.relay_url.clone(),
                    sess.token.clone(),
                    self.dep_out.clone(),
                    self.acc_busy.clone(),
                );
                ui.ctx().request_repaint_after(Duration::from_secs(5));
            }
            if self.dep_pending.is_empty() && self.dep_claimed.is_empty() {
                ui.label(
                    egui::RichText::new(i18n::t("dep.none"))
                        .size(12.0)
                        .color(p.muted),
                );
            }
            let pending = self.dep_pending.clone();
            let mut revoke: Option<String> = None;
            let mut kopiert = false;
            for pend in &pending {
                ui.horizontal(|ui| {
                    icons::show(ui, "refresh", 14.0, p.muted);
                    let was = if pend.name_hint.is_empty() {
                        pend.code.clone()
                    } else {
                        format!("{} ({})", pend.name_hint, pend.code)
                    };
                    let zeile = if pend.max_uses == 0 {
                        format!(
                            "{} - {} ({})",
                            was,
                            i18n::t("dep.waiting"),
                            i18n::tf("dep.used_unlimited", &pend.uses.to_string())
                        )
                    } else if pend.max_uses > 1 {
                        format!(
                            "{} - {} ({})",
                            was,
                            i18n::t("dep.waiting"),
                            i18n::tf(
                                "dep.left",
                                &format!(
                                    "{} ({} von {})",
                                    pend.max_uses.saturating_sub(pend.uses),
                                    pend.uses,
                                    pend.max_uses
                                )
                            )
                        )
                    } else {
                        format!("{} - {}", was, i18n::t("dep.waiting"))
                    };
                    ui.label(
                        egui::RichText::new(zeile).size(12.0).color(p.muted),
                    );
                    // The link sits next to it (not below) - with copy
                    // and revoke right in the same row.
                    let web = account::setup_web_link(&pend.code);
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        if icon_ghost(ui, "trash", i18n::t("dep.revoke_tip")).clicked() {
                            revoke = Some(pend.code.clone());
                        }
                        if icon_ghost(ui, "copy", i18n::t("dep.copy")).clicked() {
                            ui.ctx().copy_text(web.clone());
                            kopiert = true;
                        }
                        ui.label(
                            egui::RichText::new(&web)
                                .size(11.5)
                                .color(p.accent),
                        );
                    });
                });
                ui.add_space(6.0);
            }
            if kopiert {
                self.dep_msg = i18n::t("dep.copied").to_string();
            }
            if let Some(code) = revoke {
                account::setup_revoke_async(
                    self.shared.relay_url.clone(),
                    sess.token.clone(),
                    code,
                );
                // re-read right away so that the row disappears
                self.dep_next = std::time::Instant::now();
            }
            let claimed = self.dep_claimed.clone();
            for cl in &claimed {
                ui.horizontal(|ui| {
                    dot(ui, true);
                    ui.label(
                        egui::RichText::new(format!(
                            "{}  {}  -  {}",
                            cl.name,
                            partners::pretty_id(&cl.id),
                            i18n::t("dep.done_row")
                        ))
                        .size(12.0),
                    );
                    if icon_ghost(ui, "copy", &cl.password).clicked() {
                        ui.ctx().copy_text(cl.password.clone());
                    }
                });
                ui.add_space(2.0);
            }
        });
    }

}
