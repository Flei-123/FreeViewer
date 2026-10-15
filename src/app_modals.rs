//! Split out of main.rs (r35): mechanical move, behaviour unchanged.
use crate::lockutil::LockExt;
use super::*;

impl App {
    /// Collect the results of the setup background runs.
    pub(crate) fn take_setup_result(&mut self) {
        let res = self.dep_out.lock_ok().take();
        let Some(res) = res else { return };
        match res {
            account::SetupOut::Created { link, web, .. } => {
                self.dep_link = if web.is_empty() { link } else { web };
                // freshly created = immediately in the clipboard, without
                // anyone having to look for a button afterwards
                if let Ok(mut cb) = arboard::Clipboard::new() {
                    let _ = cb.set_text(self.dep_link.clone());
                }
                self.dep_msg = i18n::t("dep.copied").to_string();
                self.dep_next = std::time::Instant::now();
            }
            account::SetupOut::Inbox { pending, claimed } => {
                if self.dep_booted {
                    let mut neue = Vec::new();
                    for cl in &claimed {
                        if !self.dep_seen.contains(&cl.id) {
                            self.dep_seen.insert(cl.id.clone());
                            neue.push(cl.name.clone());
                        }
                    }
                    if !neue.is_empty() {
                        // liegt jetzt im Konto-Adressbuch - einmal abgleichen
                        self.hint = i18n::tf("dep.arrived", &neue.join(", "));
                        self.start_sync(false);
                    }
                } else {
                    // erste Fuellung: nur merken, nichts melden
                    for cl in &claimed {
                        self.dep_seen.insert(cl.id.clone());
                    }
                    self.dep_booted = true;
                }
                self.dep_pending = pending;
                self.dep_claimed = claimed;
            }
            account::SetupOut::Failed(msg) => {
                self.dep_msg = msg;
            }
        }
    }

    /// Safety question before uninstalling - right in the app, without
    /// the detour via "Apps & Features".
    pub(crate) fn uninstall_modal(&mut self, ctx: &egui::Context) {
        if !self.uninstall_ask {
            return;
        }
        let mut offen = true;
        let mut sicher = false;
        let mut weg = false;
        egui::Window::new(i18n::t("set.uninstall_title"))
            .collapsible(false)
            .resizable(false)
            .anchor(egui::Align2::CENTER_CENTER, [0.0, 0.0])
            .open(&mut offen)
            .frame(
                egui::Frame::group(&ctx.style())
                    .fill(theme::card())
                    .stroke(egui::Stroke::new(1.0, theme::accent()))
                    .corner_radius(14)
                    .inner_margin(egui::Margin::same(10)),
            )
            .show(ctx, |ui| {
                ui.set_min_width(340.0);
                ui.label(
                    egui::RichText::new(i18n::t("set.uninstall_sure"))
                        .size(13.0),
                );
                ui.add_space(4.0);
                ui.label(
                    egui::RichText::new(i18n::t("set.uninstall_keep"))
                        .size(11.5)
                        .color(theme::muted()),
                );
                ui.add_space(10.0);
                ui.horizontal(|ui| {
                    if accent_button(ui, i18n::t("set.uninstall_yes"), true).clicked() {
                        sicher = true;
                    }
                    if ghost_button(ui, i18n::t("setup.cancel")).clicked() {
                        weg = true;
                    }
                });
            });
        if weg {
            offen = false;
        }
        if sicher {
            self.uninstall_ask = false;
            match service::elevate("--uninstall") {
                Ok(()) => self.hint = i18n::t("set.uninstall_running").to_string(),
                Err(e) => self.hint = format!("{}", e),
            }
        } else if !offen {
            self.uninstall_ask = false;
        }
    }

    /// License window (X-Remote only): enter key, verify, store.
    /// Without a valid license it appears once at every start - dismissing it
    /// works, the program keeps running for now (soft introduction).
    #[cfg(feature = "license")]
    pub(crate) fn license_modal(&mut self, ctx: &egui::Context) {
        if !self.lic_show {
            return;
        }
        let mut offen = true;
        let mut spaeter = false;
        egui::Window::new(i18n::t("lic.title"))
            .collapsible(false)
            .resizable(false)
            .anchor(egui::Align2::CENTER_CENTER, [0.0, 0.0])
            .open(&mut offen)
            .frame(
                egui::Frame::group(&ctx.style())
                    .fill(theme::card())
                    .stroke(egui::Stroke::new(1.0, theme::accent()))
                    .corner_radius(14)
                    .inner_margin(egui::Margin::same(10)),
            )
            .show(ctx, |ui| {
                ui.set_min_width(380.0);
                ui.label(egui::RichText::new(i18n::t("lic.intro")).size(12.5));
                ui.add_space(8.0);
                ui.add(
                    egui::TextEdit::singleline(&mut self.lic_key)
                        .hint_text(i18n::t("lic.key_ph"))
                        .desired_width(360.0)
                        .font(egui::TextStyle::Monospace),
                );
                if !self.lic_msg.is_empty() {
                    ui.add_space(6.0);
                    ui.label(
                        egui::RichText::new(&self.lic_msg)
                            .size(11.5)
                            .color(theme::accent()),
                    );
                }
                ui.add_space(10.0);
                ui.horizontal(|ui| {
                    if accent_button(ui, i18n::t("lic.activate"), true).clicked() {
                        match license::activate(&self.lic_key) {
                            Ok(lic) => {
                                self.lic_msg = String::new();
                                self.lic_show = false;
                                self.hint =
                                    i18n::tf("lic.ok_msg", &lic.name);
                            }
                            Err(e) => {
                                self.lic_msg = i18n::tf("lic.invalid", &e);
                            }
                        }
                    }
                    if ghost_button(ui, i18n::t("lic.later")).clicked() {
                        spaeter = true;
                    }
                });
            });
        if spaeter || !offen {
            self.lic_show = false;
        }
    }

    /// Loading window while the service installs an update. Afterwards
    /// all parts are terminated and the service restarts with the fresh state.
    pub(crate) fn update_modal(&mut self, ctx: &egui::Context) {
        if self.upd_flag_at.elapsed() > Duration::from_millis(500) {
            self.upd_flag_at = std::time::Instant::now();
            self.upd_running = update::updating_flag()
                .map(|p| p.exists())
                .unwrap_or(false);
        }
        if !self.upd_running {
            return;
        }
        egui::Window::new(i18n::t("upd.working"))
            .collapsible(false)
            .resizable(false)
            .anchor(egui::Align2::CENTER_CENTER, [0.0, 0.0])
            .frame(
                egui::Frame::group(&ctx.style())
                    .fill(theme::card())
                    .stroke(egui::Stroke::new(1.0, theme::accent()))
                    .corner_radius(14)
                    .inner_margin(egui::Margin::same(10)),
            )
            .show(ctx, |ui| {
                ui.set_min_width(300.0);
                ui.horizontal(|ui| {
                    ui.spinner();
                    ui.add_space(6.0);
                    ui.label(
                        egui::RichText::new(i18n::t("upd.restart_soon")).size(13.0),
                    );
                });
            });
    }

    /// Password prompt: "Connect" means connect directly. Without a
    /// saved password this dialog asks for it - the request to the
    /// other side (knocking) is a deliberate second option here.
    pub(crate) fn pw_ask_ui(&mut self, ctx: &egui::Context) {
        let Some(id) = self.pw_ask.clone() else {
            return;
        };
        let mut offen = true;
        let mut verbinden = false;
        let mut anfragen = false;
        let mut weg = false;
        let name = self
            .book
            .get(&id)
            .map(|x| x.label())
            .filter(|l| !l.is_empty())
            .unwrap_or_else(|| partners::pretty_id(&id));
        egui::Window::new(i18n::tf("pwask.title", &name))
            .collapsible(false)
            .resizable(false)
            .anchor(egui::Align2::CENTER_CENTER, [0.0, 0.0])
            .open(&mut offen)
            .frame(
                egui::Frame::group(&ctx.style())
                    .fill(theme::card())
                    .stroke(egui::Stroke::new(1.0, theme::accent()))
                    .corner_radius(14)
                    .inner_margin(egui::Margin::same(10)),
            )
            .show(ctx, |ui| {
                ui.set_min_width(320.0);
                ui.label(
                    egui::RichText::new(i18n::t("pwask.note"))
                        .size(12.0)
                        .color(theme::muted()),
                );
                ui.add_space(8.0);
                label_small(ui, i18n::t("pwask.pw"));
                let enter = ui
                    .add(
                        egui::TextEdit::singleline(&mut self.partner_pw)
                            .desired_width(220.0)
                            .margin(egui::Margin::symmetric(8, 4)),
                    )
                    .lost_focus()
                    && ui.input(|i| i.key_pressed(egui::Key::Enter));
                ui.add_space(10.0);
                ui.horizontal(|ui| {
                    let hat_pw = !self.partner_pw.trim().is_empty();
                    if accent_button(ui, i18n::t("start.connect"), hat_pw).clicked()
                        || (enter && hat_pw)
                    {
                        verbinden = true;
                    }
                    if ghost_button(ui, i18n::t("pwask.ask")).clicked() {
                        anfragen = true;
                    }
                    if ghost_button(ui, i18n::t("setup.cancel")).clicked() {
                        weg = true;
                    }
                });
            });
        if verbinden {
            self.pw_ask = None;
            self.start_session();
        } else if anfragen {
            self.pw_ask = None;
            self.start_ask_session();
        } else if weg || !offen {
            self.pw_ask = None;
        }
    }

    /// Setup dialog: set this device up with a one-time code and
    /// then install (with service).
    pub(crate) fn setup_dialog_ui(&mut self, ctx: &egui::Context) {
        let Some(dlg) = self.setup_dlg.as_mut() else {
            return;
        };
        // Ergebnis des Einloesens abholen
        let got = dlg.out.lock_ok().take();
        if let Some(res) = got {
            match res {
                Ok(r) => {
                    let name = presence::clean(&dlg.name);
                    *self.shared.device_name.lock_ok() = name.clone();
                    let _ = presence::save_device_name(&name);
                    if !r.password.is_empty() {
                        let current = self.shared.password.lock_ok().clone();
                        if self.pw_fixed && current != r.password {
                            // never replace an existing permanent password silently
                            self.pw_overwrite = Some(PwOverwrite::Setup(r.password.clone()));
                        } else {
                            *self.shared.password.lock_ok() = r.password.clone();
                            let _ = ident::set_fixed_password(Some(&r.password));
                            self.pw_fixed = true;
                        }
                    }
                    dlg.owner = if r.owner.is_empty() { r.name_hint } else { r.owner };
                    dlg.done = true;
                    // Installation with service - asks once for admin rights
                    if !setup::running_installed() {
                        match service::elevate("--install --with-service") {
                            Ok(()) if dlg.auto => {
                                // The downloaded file has served its purpose:
                                // the installed version takes over - this one
                                // may exit (and is deleted).
                                std::thread::spawn(|| {
                                    std::thread::sleep(std::time::Duration::from_millis(1500));
                                    std::process::exit(0);
                                });
                            }
                            Ok(()) => {}
                            Err(e) => dlg.err = format!("{}", e),
                        }
                    }
                }
                Err(e) => dlg.err = e,
            }
        }
        let mut offen = true;
        let mut los = false;
        let mut weg = false;
        // From the browser download: no further click needed.
        if dlg.auto && !dlg.done && dlg.err.is_empty() && !dlg.busy.load(Ordering::Relaxed) {
            los = true;
        }
        egui::Window::new(i18n::t("setup.title"))
            .collapsible(false)
            .resizable(false)
            .anchor(egui::Align2::CENTER_CENTER, [0.0, 0.0])
            .open(&mut offen)
            .frame(
                egui::Frame::group(&ctx.style())
                    .fill(theme::card())
                    .stroke(egui::Stroke::new(1.0, theme::accent()))
                    .corner_radius(14)
                    .inner_margin(egui::Margin::same(10)),
            )
            .show(ctx, |ui| {
                ui.set_min_width(360.0);
                if dlg.auto && !dlg.done {
                    ui.horizontal(|ui| {
                        ui.spinner();
                        ui.add_space(6.0);
                        ui.label(
                            egui::RichText::new(i18n::t("setup.auto")).size(13.0),
                        );
                    });
                    if !dlg.err.is_empty() {
                        ui.add_space(6.0);
                        ui.label(
                            egui::RichText::new(&dlg.err)
                                .size(12.0)
                                .color(theme::muted()),
                        );
                    }
                    return;
                }
                if dlg.done {
                    ui.label(
                        egui::RichText::new(i18n::tf("setup.done", &dlg.owner))
                            .size(13.5),
                    );
                    if !dlg.err.is_empty() {
                        ui.add_space(4.0);
                        ui.label(
                            egui::RichText::new(&dlg.err)
                                .size(12.0)
                                .color(theme::muted()),
                        );
                    }
                    ui.add_space(10.0);
                    if accent_button(ui, "OK", true).clicked() {
                        weg = true;
                    }
                    return;
                }
                ui.label(
                    egui::RichText::new(i18n::t("setup.lead"))
                        .size(12.0)
                        .color(theme::muted()),
                );
                ui.add_space(8.0);
                label_small(ui, i18n::t("setup.name"));
                let enter = ui
                    .add(
                        egui::TextEdit::singleline(&mut dlg.name)
                            .desired_width(240.0)
                            .margin(egui::Margin::symmetric(8, 4)),
                    )
                    .lost_focus()
                    && ui.input(|i| i.key_pressed(egui::Key::Enter));
                if !dlg.err.is_empty() {
                    ui.add_space(4.0);
                    ui.label(
                        egui::RichText::new(&dlg.err)
                            .size(12.0)
                            .color(theme::muted()),
                    );
                }
                ui.add_space(10.0);
                ui.horizontal(|ui| {
                    let busy = dlg.busy.load(Ordering::Relaxed);
                    let label = if busy {
                        i18n::t("setup.working")
                    } else {
                        i18n::t("setup.ok")
                    };
                    if accent_button(ui, label, !busy).clicked() || (enter && !busy) {
                        if dlg.name.trim().is_empty() {
                            dlg.err = i18n::t("setup.noname").to_string();
                        } else {
                            dlg.err.clear();
                            los = true;
                        }
                    }
                    if ghost_button(ui, i18n::t("setup.cancel")).clicked() {
                        weg = true;
                    }
                });
            });
        if los {
            let dlg = self.setup_dlg.as_ref().unwrap();
            let relay = self.shared.relay_url.clone();
            let code = dlg.code.clone();
            let name = presence::clean(&dlg.name);
            let id = self.shared.my_id.lock_ok().clone();
            let busy = dlg.busy.clone();
            let out = dlg.out.clone();
            if !busy.swap(true, Ordering::SeqCst) {
                std::thread::spawn(move || {
                    let res = account::setup_claim(&relay, &code, &id, &name)
                        .map_err(|e| e.to_string());
                    *out.lock_ok() = Some(res);
                    busy.store(false, Ordering::SeqCst);
                });
            }
        }
        if weg || !offen {
            self.setup_dlg = None;
        }
    }

}
