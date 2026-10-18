//! Split out of main.rs (r35): mechanical move, behaviour unchanged.
use crate::lockutil::LockExt;
use super::*;

impl App {
    /// Take an image of the own UI and exit (--shot).
    pub(crate) fn shot_ui(&mut self, ctx: &egui::Context) {
if let Some(path) = self.shot.clone() {
            self.shot_n += 1;
            // As soon as the device list is there, trigger the join.
            if self.meet_shot_join && self.meet_shot_arm == 0 && self.meet_win.devices_there {
                self.meet_shot_arm = self.shot_n;
                self.meet_win.native_start = true;
            }
            if self.shot_n == self.shot_warte {
                ctx.send_viewport_cmd(egui::ViewportCommand::Screenshot(
                    egui::UserData::default(),
                ));
            }
            if self.shot_n > self.shot_warte {
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
                if self.shot_n > self.shot_warte + 150 {
                    eprintln!("SHOT: kein Bild bekommen");
                    std::process::exit(2);
                }
            }
            ctx.request_repaint();
        }
    }

    /// Only for images: draw an example state of the meeting
    /// (`--meetdemo N --shot file.jpg`). Without this path there would never be a
    /// real image of the stage - the build server has neither a camera nor a second
    /// participant.
    pub(crate) fn meet_demo_ui(&mut self, ctx: &egui::Context, nr: usize) {
        // EXAMPLES+1: the waiting screen.
        if nr == meetwindow::EXAMPLES + 1 {
            meetwindow::waiting_screen_ui(
                ctx,
                "Wochenbesprechung",
                "Der Gastgeber wurde benachrichtigt.",
            );
            return;
        }
        // From EXAMPLES: the join screen.
        if nr >= meetwindow::EXAMPLES {
            let images = meetwindow::test_images(ctx);
            let mut b = meetwindow::example_join();
            meetwindow::join_ui(ctx, &mut b, images.own.as_ref());
            return;
        }
        let images = meetwindow::test_images(ctx);
        let (_name, view_state, mut z) = meetwindow::example(nr);
        meetwindow::meeting_ui(ctx, &view_state, &mut z, &images);
    }

    /// Kurzer, sichtbarer Hinweis ("Link kopiert").
    pub(crate) fn meet_toast(&mut self, ctx: &egui::Context) {
        let p = theme::palette();
        if let Some((text, since)) = self.meet_win.toast.clone() {
            let alt = since.elapsed().as_secs_f32();
            if alt > 2.4 {
                self.meet_win.toast = None;
                return;
            }
            let deck = if alt > 1.8 {
                ((2.4 - alt) / 0.6).clamp(0.0, 1.0)
            } else {
                1.0
            };
            egui::Area::new(egui::Id::new("meet_toast"))
                .anchor(egui::Align2::CENTER_BOTTOM, egui::vec2(0.0, -90.0))
                .order(egui::Order::Foreground)
                .show(ctx, |ui| {
                    egui::Frame::NONE
                        .fill(p.accent.gamma_multiply(deck))
                        .corner_radius(egui::CornerRadius::same(14))
                        .inner_margin(egui::Margin::symmetric(16, 8))
                        .show(ui, |ui| {
                            ui.label(
                                egui::RichText::new(&text)
                                    .size(13.0)
                                    .color(p.on_accent.gamma_multiply(deck)),
                            );
                        });
                });
            ctx.request_repaint_after(Duration::from_millis(50));
        }
    }

    /// Kopfzeile: Seitenname, Suche, eigener State.
    pub(crate) fn header(&mut self, ctx: &egui::Context) {
        let p = theme::palette();
        let my_id = self.shared.my_id.lock_ok().clone();
        let online = !my_id.is_empty();
        egui::TopBottomPanel::top("fv_header")
            .exact_height(48.0)
            .frame(
                egui::Frame::NONE
                    .fill(p.card)
                    .inner_margin(egui::Margin::symmetric(14, 6)),
            )
            .show(ctx, |ui| {
                ui.horizontal_centered(|ui| {
                    let title = match self.view {
                        View::Start => i18n::t("nav.start"),
                        View::Devices => i18n::t("nav.devices"),
                        View::Meet => i18n::t("nav.meet"),
                        View::Friends => i18n::t("nav.friends"),
                        View::Settings => i18n::t("nav.settings"),
                    };
                    ui.label(egui::RichText::new(title).size(16.0).strong().color(p.text));
                    ui.add_space(12.0);
                    icons::show(ui, "search", 14.0, p.muted);
                    let mut q = self.search.clone();
                    if ui
                        .add(
                            egui::TextEdit::singleline(&mut q)
                                .desired_width(230.0)
                                .hint_text(i18n::t("hdr.search"))
                                .margin(egui::Margin::symmetric(8, 4)),
                        )
                        .changed()
                    {
                        self.search = q;
                        if !self.search.is_empty() {
                            self.view = View::Devices;
                        }
                    }
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        // The state comes from the line itself, not from
                        // "we once had a number": 0 connecting, 1 ready,
                        // 2 offline.
                        let state = self.shared.net_state();
                        let online = online && state == 1;
                        status_pill(
                            ui,
                            online,
                            if online {
                                i18n::t("st.ready")
                            } else if state == 2 {
                                i18n::t("st.no_net")
                            } else {
                                i18n::t("st.connecting")
                            },
                        );
                        if !my_id.is_empty() {
                            ui.add_space(8.0);
                            ui.label(
                                egui::RichText::new(partners::pretty_id(&my_id))
                                    .size(12.0)
                                    .color(p.muted),
                            );
                        }
                    });
                });
            });
    }

    /// Devices: folders on the left, the list in the middle, on the right the selected
    /// device with everything you can set on it.
    /// Fetch the friends service (and start it the first time).
    ///
    /// Why not already at program start: that costs a request to the relay
    /// every 20 seconds. Whoever never opens the page should not
    /// be burdened with it either.
    pub(crate) fn friends_service(&mut self) -> std::sync::Arc<friends::Service> {
        if let Some(d) = &self.friends {
            return d.clone();
        }
        let d = friends::Service::from_device(self.shared.relay_url.clone());
        // With an account the list works across devices, without an account it is local -
        // both must work, so the symbol is only an extra.
        if let Some(sess) = &self.acc {
            d.set_account(Some(&sess.token));
        }
        d.launch();
        self.friends = Some(d.clone());
        d
    }

    /// Friends: request, confirm, and then with ONE click into the
    /// meeting or into remote maintenance.
    ///
    /// WHY mutual: a friendship that one side can establish alone
    /// is none - it would just be a list everybody enters themselves into.
    /// The relay enforces this, not this window.
    pub(crate) fn friends_view(&mut self, ui: &mut egui::Ui) {
        let p = theme::palette();
        let d = self.friends_service();
        while let Some(m) = d.notice() {
            // Show only the messages that really concern the user.
            self.friends_notice = match m {
                // The sync runs every 20 seconds - turning each one into
                // a message would be pure noise.
                friends::Notice::Synced { .. } => continue,
                friends::Notice::Requested(w) => {
                    format!("Anfrage an {} ist raus.", w)
                }
                friends::Notice::ImmediatelyBefriended(w) => {
                    format!("{} hatte dich auch gefragt - ihr seid jetzt Freunde.", w)
                }
                friends::Notice::Accepted(w) => format!("{} ist jetzt dein Freund.", w),
                friends::Notice::Rejected(w) => format!("Anfrage von {} abgelehnt.", w),
                friends::Notice::Removed(w) => format!("{} entfernt.", w),
                friends::Notice::InviteSent(w) => {
                    format!("Einladung ist bei {} angekommen (Link liegt zusätzlich in der Zwischenablage).", w)
                }
                friends::Notice::InviteOnlyLink(w) => format!(
                    "Der Relay kann noch keine Einladungen zustellen – der Link liegt in der Zwischenablage, schick ihn {} (z. B. per Chat).",
                    w
                ),
                friends::Notice::InviteReceived(w, t) => {
                    if t.trim().is_empty() {
                        format!("{} lädt dich in ein Meeting ein.", w)
                    } else {
                        format!("{} lädt dich ein: {}", w, t)
                    }
                }
                friends::Notice::Failed(t) => t,
            };
        }

        let mut connect_now: Option<String> = None;
        let mut meeting_mit: Option<friends::Friend> = None;
        // Invitations a friend has sent: (number, address, true = join)
        let mut invite_action: Option<(String, String, bool)> = None;

        ui.horizontal(|ui| {
            ui.label(
                egui::RichText::new("Freunde")
                    .size(19.0)
                    .strong()
                    .color(p.text),
            );
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                let (text, tint) = if !d.connected() {
                    ("Relay nicht erreicht", p.muted)
                } else if d.signed_in() {
                    ("mit Konto – auf allen Geräten", p.accent)
                } else {
                    ("ohne Konto – nur dieses Gerät", p.muted)
                };
                ui.label(egui::RichText::new(text).size(11.0).color(tint));
            });
        });
        ui.add_space(2.0);
        ui.label(
            egui::RichText::new(
                "Eine Freundschaft gilt erst, wenn BEIDE zugestimmt haben. Danach genügt ein Klick.",
            )
            .size(11.0)
            .color(p.muted),
        );
        ui.add_space(10.0);

        // ------------------------------------------------ Meeting invitations
        // A friend created a meeting by button and invited us.
        for e in d.invites() {
            egui::Frame::NONE
                .fill(p.card)
                .stroke(egui::Stroke::new(1.0, p.accent))
                .corner_radius(10)
                .inner_margin(egui::Margin::symmetric(12, 8))
                .show(ui, |ui| {
                    ui.set_width(ui.available_width());
                    ui.horizontal(|ui| {
                        ui.vertical(|ui| {
                            ui.label(
                                egui::RichText::new(format!("{} lädt dich in ein Meeting ein", e.von()))
                                    .size(13.0)
                                    .strong()
                                    .color(p.text),
                            );
                            if !e.heading.trim().is_empty() {
                                ui.label(egui::RichText::new(&e.heading).size(11.0).color(p.muted));
                            }
                        });
                        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                            if ui.button("Verwerfen").clicked() {
                                invite_action = Some((e.id.clone(), e.link.clone(), false));
                            }
                            if ui.button("Beitreten").clicked() {
                                invite_action = Some((e.id.clone(), e.link.clone(), true));
                            }
                        });
                    });
                });
            ui.add_space(5.0);
        }

        // ------------------------------------------------ neue Request
        egui::Frame::NONE
            .fill(p.card)
            .stroke(egui::Stroke::new(1.0, p.line))
            .corner_radius(10)
            .inner_margin(egui::Margin::same(12))
            .show(ui, |ui| {
                ui.set_width(ui.available_width());
                label_small(
                    ui,
                    if d.signed_in() {
                        "FreeViewer-Nummer oder @Name des Freundes"
                    } else {
                        "FreeViewer-Nummer des Freundes"
                    },
                );
                ui.horizontal(|ui| {
                    let field = ui.add(
                        egui::TextEdit::singleline(&mut self.friends_id)
                            .desired_width(190.0)
                            .hint_text(if d.signed_in() { "497 628 420 oder @anna" } else { "497 628 420" }),
                    );
                    let send_out = ui.button("Anfrage senden").clicked()
                        || (field.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter)));
                    if send_out {
                        match d.request(&self.friends_id) {
                            Ok(()) => {
                                self.friends_notice = format!(
                                    "Anfrage an {} unterwegs - sie gilt erst, wenn er zustimmt.",
                                    friends::target_label(&self.friends_id)
                                );
                                self.friends_id.clear();
                            }
                            Err(e) => self.friends_notice = format!("{}", e),
                        }
                    }
                });
                if !self.friends_notice.is_empty() {
                    ui.add_space(4.0);
                    ui.label(
                        egui::RichText::new(&self.friends_notice)
                            .size(11.0)
                            .color(p.muted),
                    );
                }
            });
        ui.add_space(10.0);

        // ------------------------------------------------ offene Anfragen
        let incoming = d.incoming_requests();
        let outgoing = d.outgoing();
        if !incoming.is_empty() || !outgoing.is_empty() {
            label_small(ui, "Anfragen");
            for a in incoming.iter() {
                egui::Frame::NONE
                    .fill(p.card_hi)
                    .stroke(egui::Stroke::new(1.0, p.accent))
                    .corner_radius(10)
                    .inner_margin(egui::Margin::symmetric(12, 8))
                    .show(ui, |ui| {
                        ui.set_width(ui.available_width());
                        ui.horizontal(|ui| {
                            ui.label(
                                egui::RichText::new(format!("{} möchte dein Freund sein", a.display_name()))
                                    .size(12.5)
                                    .color(p.text),
                            );
                            ui.with_layout(
                                egui::Layout::right_to_left(egui::Align::Center),
                                |ui| {
                                    if ui.button("Ablehnen").clicked() {
                                        if let Err(e) = d.reject(&a.opposite()) {
                                            self.friends_notice = format!("{}", e);
                                        }
                                    }
                                    if ui.button("Annehmen").clicked() {
                                        match d.accept_request(&a.opposite()) {
                                            Ok(()) => {
                                                self.friends_notice =
                                                    format!("{} ist jetzt dein Freund.", a.display_name())
                                            }
                                            Err(e) => self.friends_notice = format!("{}", e),
                                        }
                                    }
                                },
                            );
                        });
                        if !a.message.is_empty() {
                            ui.label(
                                egui::RichText::new(if a.message.chars().count() > 90 {
                                    format!(
                                        "{}…",
                                        a.message.chars().take(89).collect::<String>()
                                    )
                                } else {
                                    a.message.clone()
                                })
                                    .size(11.0)
                                    .color(p.muted),
                            );
                        }
                    });
                ui.add_space(4.0);
            }
            for a in outgoing.iter() {
                ui.horizontal(|ui| {
                    ui.label(
                        egui::RichText::new(format!("an {} – wartet auf Zustimmung", a.display_name()))
                            .size(11.5)
                            .color(p.muted),
                    );
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        if ui.button("Zurücknehmen").clicked() {
                            if let Err(e) = d.reject(&a.opposite()) {
                                self.friends_notice = format!("{}", e);
                            }
                        }
                    });
                });
            }
            ui.add_space(10.0);
        }

        // ------------------------------------------------ the list
        let list_items = d.list_items();
        label_small(ui, "Deine Freunde");
        if list_items.is_empty() {
            ui.add_space(6.0);
            ui.label(
                egui::RichText::new("Noch niemand. Schick oben eine Anfrage.")
                    .size(12.0)
                    .color(p.muted),
            );
        }
        egui::ScrollArea::vertical().show(ui, |ui| {
            for fr in list_items.iter() {
                egui::Frame::NONE
                    .fill(p.card)
                    .stroke(egui::Stroke::new(1.0, p.line))
                    .corner_radius(10)
                    .inner_margin(egui::Margin::symmetric(12, 8))
                    .show(ui, |ui| {
                        ui.set_width(ui.available_width());
                        ui.horizontal(|ui| {
                            ui.painter().circle_filled(
                                ui.cursor().min + egui::vec2(4.0, 9.0),
                                4.0,
                                if fr.online { p.accent } else { p.muted },
                            );
                            ui.add_space(14.0);
                            ui.vertical(|ui| {
                                ui.label(
                                    egui::RichText::new(fr.display_name()).size(13.0).color(p.text),
                                );
                                // With an account: @name, then the PC (with several
                                // "+N"), otherwise "no FreeViewer PC".
                                let mut under: Vec<String> = Vec::new();
                                if !fr.handle.is_empty() && fr.name.trim() != format!("@{}", fr.handle) {
                                    under.push(format!("@{}", fr.handle));
                                }
                                if fr.hat_pc() {
                                    let more = fr.devices.len().saturating_sub(1);
                                    under.push(if more > 0 {
                                        format!("{} +{} PC", partners::pretty_id(&fr.fvid), more)
                                    } else {
                                        partners::pretty_id(&fr.fvid)
                                    });
                                    under.push(fr.lately());
                                } else {
                                    under.push("kein FreeViewer-PC".to_string());
                                }
                                let r = ui.label(
                                    egui::RichText::new(under.join(" · "))
                                        .size(10.5)
                                        .color(p.muted),
                                );
                                if fr.devices.len() > 1 {
                                    r.on_hover_text(
                                        fr.devices
                                            .iter()
                                            .map(|g| {
                                                format!(
                                                    "{}  {}{}",
                                                    partners::pretty_id(&g.fvid),
                                                    g.label,
                                                    if g.online { "  (online)" } else { "" }
                                                )
                                            })
                                            .collect::<Vec<_>>()
                                            .join("\n"),
                                    );
                                }
                            });
                            ui.with_layout(
                                egui::Layout::right_to_left(egui::Align::Center),
                                |ui| {
                                    if ui.button("Entfernen").clicked() {
                                        if let Err(e) = d.remove_entry(&fr.entry_key()) {
                                            self.friends_notice = format!("{}", e);
                                        }
                                    }
                                    // Only offer what actually works:
                                    // remote maintenance to an offline
                                    // machine would be a button into nothing.
                                    ui.add_enabled_ui(fr.reachable(), |ui| {
                                        if ui
                                            .button("Fernwartung")
                                            .on_hover_text("Sofort verbinden – zulassen muss er es trotzdem")
                                            .clicked()
                                        {
                                            connect_now = Some(fr.fvid.clone());
                                        }
                                    });
                                    if ui
                                        .button("Meeting")
                                        .on_hover_text("Meeting anlegen, Einladung an den Freund schicken und den Link in die Zwischenablage legen")
                                        .clicked()
                                    {
                                        meeting_mit = Some(fr.clone());
                                    }
                                },
                            );
                        });
                    });
                ui.add_space(5.0);
            }
        });

        if let Some(id) = connect_now {
            self.view = View::Start;
            self.hint = i18n::tf("link.control", &partners::pretty_id(&id));
            self.connect_to(&id);
        }
        if let Some((id, link, join_meeting)) = invite_action {
            if join_meeting {
                // Only open the meeting page of this brand - never an arbitrary
                // address a friend (or a stranger in their name) sends.
                if friends::invite_link_ok(&link, &meet::base()) {
                    match meet::open_window(&link) {
                        Ok(()) => {
                            self.friends_notice = "Meeting wird geöffnet.".to_string();
                            d.invite_done(&id);
                        }
                        Err(e) => self.friends_notice = format!("Meeting geht nicht: {}", e),
                    }
                } else {
                    self.friends_notice =
                        "Dieser Einladungslink zeigt nicht auf die Meeting-Seite - nicht geöffnet.".to_string();
                    d.invite_done(&id);
                }
            } else {
                d.invite_done(&id);
            }
        }
        if let Some(fr) = meeting_mit {
            let wer = fr.display_name();
            match meet::create(&format!("Mit {}", wer), false) {
                Ok(m) => {
                    // The link is ALWAYS in the clipboard: if delivery
                    // fails (old relay, friend without a PC), this way remains.
                    let link = meet::invite(&m);
                    ui.ctx().copy_text(link);
                    meet::remember(&m);
                    // Only the bare address goes to the relay - and only without an
                    // end-to-end key (the relay must never see that).
                    let relay_link = if m.e2e.is_empty() {
                        Some(meet::join_url(&m.id, &m.passphrase))
                    } else {
                        None
                    };
                    self.friends_notice = match relay_link
                        .as_deref()
                        .map(|l| d.meeting_invite(&fr, &format!("Mit {}", wer), l))
                    {
                        Some(Ok(())) => format!(
                            "Meeting angelegt - Einladung geht an {} (Link zusätzlich kopiert).",
                            wer
                        ),
                        Some(Err(e)) => format!(
                            "Meeting angelegt, Einladung kopiert - schick sie {} (z. B. per Chat). ({})",
                            wer, e
                        ),
                        None => format!(
                            "Meeting angelegt, Einladung kopiert - schick sie {} (z. B. per Chat).",
                            wer
                        ),
                    };
                    self.meet_win.meeting = Some(m);
                    self.view = View::Meet;
                }
                Err(e) => self.friends_notice = format!("Meeting geht nicht: {}", e),
            }
        }
    }

}
