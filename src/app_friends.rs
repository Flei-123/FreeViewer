//! Split out of main.rs (r35): mechanical move, behaviour unchanged.
use crate::lockutil::LockExt;
use super::*;

impl App {
    /// Take an image of the own UI and exit (--shot).
    pub(crate) fn shot_ui(&mut self, ctx: &egui::Context) {
if let Some(path) = self.shot.clone() {
            self.shot_n += 1;
            // As soon as the device list is there, trigger the join.
            if self.meet_shot_join && self.meet_shot_arm == 0 && self.meet_win.geraete_da {
                self.meet_shot_arm = self.shot_n;
                self.meet_win.nativ_start = true;
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
    /// (`--meetdemo N --shot datei.jpg`). Without this path there would never be a
    /// real image of the stage - the build server has neither a camera nor a second
    /// participant.
    pub(crate) fn meet_demo_ui(&mut self, ctx: &egui::Context, nr: usize) {
        // EXAMPLES+1: the waiting screen.
        if nr == meetfenster::BEISPIELE + 1 {
            meetfenster::warteschirm_ui(
                ctx,
                "Wochenbesprechung",
                "Der Gastgeber wurde benachrichtigt.",
            );
            return;
        }
        // From EXAMPLES: the join screen.
        if nr >= meetfenster::BEISPIELE {
            let bilder = meetfenster::pruefbilder(ctx);
            let mut b = meetfenster::beispiel_beitritt();
            meetfenster::beitritt_ui(ctx, &mut b, bilder.eigen.as_ref());
            return;
        }
        let bilder = meetfenster::pruefbilder(ctx);
        let (_name, sicht, mut z) = meetfenster::beispiel(nr);
        meetfenster::meeting_ui(ctx, &sicht, &mut z, &bilder);
    }

    /// Kurzer, sichtbarer Hinweis ("Link kopiert").
    pub(crate) fn meet_toast(&mut self, ctx: &egui::Context) {
        let p = theme::palette();
        if let Some((text, seit)) = self.meet_win.toast.clone() {
            let alt = seit.elapsed().as_secs_f32();
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

    /// Kopfzeile: Seitenname, Suche, eigener Zustand.
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
                        View::Freunde => i18n::t("nav.friends"),
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
                        let zustand = self.shared.net_state();
                        let online = online && zustand == 1;
                        status_pill(
                            ui,
                            online,
                            if online {
                                i18n::t("st.ready")
                            } else if zustand == 2 {
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
    pub(crate) fn freunde_dienst(&mut self) -> std::sync::Arc<freunde::Dienst> {
        if let Some(d) = &self.freunde {
            return d.clone();
        }
        let d = freunde::Dienst::vom_geraet(self.shared.relay_url.clone());
        // With an account the list works across devices, without an account it is local -
        // both must work, so the symbol is only an extra.
        if let Some(sess) = &self.acc {
            d.konto_setzen(Some(&sess.token));
        }
        d.starten();
        self.freunde = Some(d.clone());
        d
    }

    /// Friends: request, confirm, and then with ONE click into the
    /// meeting or into remote maintenance.
    ///
    /// WHY mutual: a friendship that one side can establish alone
    /// is none - it would just be a list everybody enters themselves into.
    /// The relay enforces this, not this window.
    pub(crate) fn freunde_view(&mut self, ui: &mut egui::Ui) {
        let p = theme::palette();
        let d = self.freunde_dienst();
        while let Some(m) = d.meldung() {
            // Show only the messages that really concern the user.
            self.freunde_meldung = match m {
                // The sync runs every 20 seconds - turning each one into
                // a message would be pure noise.
                freunde::Meldung::Abgeglichen { .. } => continue,
                freunde::Meldung::Angefragt(w) => {
                    format!("Anfrage an {} ist raus.", w)
                }
                freunde::Meldung::SofortBefreundet(w) => {
                    format!("{} hatte dich auch gefragt - ihr seid jetzt Freunde.", w)
                }
                freunde::Meldung::Angenommen(w) => format!("{} ist jetzt dein Freund.", w),
                freunde::Meldung::Abgelehnt(w) => format!("Anfrage von {} abgelehnt.", w),
                freunde::Meldung::Entfernt(w) => format!("{} entfernt.", w),
                freunde::Meldung::EinladungGesendet(w) => {
                    format!("Einladung ist bei {} angekommen (Link liegt zusätzlich in der Zwischenablage).", w)
                }
                freunde::Meldung::EinladungNurLink(w) => format!(
                    "Der Relay kann noch keine Einladungen zustellen – der Link liegt in der Zwischenablage, schick ihn {} (z. B. per Chat).",
                    w
                ),
                freunde::Meldung::EinladungErhalten(w, t) => {
                    if t.trim().is_empty() {
                        format!("{} lädt dich in ein Meeting ein.", w)
                    } else {
                        format!("{} lädt dich ein: {}", w, t)
                    }
                }
                freunde::Meldung::Fehlgeschlagen(t) => t,
            };
        }

        let mut verbinden: Option<String> = None;
        let mut meeting_mit: Option<freunde::Freund> = None;
        // Invitations a friend has sent: (number, address, true = join)
        let mut einladung_aktion: Option<(String, String, bool)> = None;

        ui.horizontal(|ui| {
            ui.label(
                egui::RichText::new("Freunde")
                    .size(19.0)
                    .strong()
                    .color(p.text),
            );
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                let (text, farbe) = if !d.verbunden() {
                    ("Relay nicht erreicht", p.muted)
                } else if d.angemeldet() {
                    ("mit Konto – auf allen Geräten", p.accent)
                } else {
                    ("ohne Konto – nur dieses Gerät", p.muted)
                };
                ui.label(egui::RichText::new(text).size(11.0).color(farbe));
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
        for e in d.einladungen() {
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
                            if !e.titel.trim().is_empty() {
                                ui.label(egui::RichText::new(&e.titel).size(11.0).color(p.muted));
                            }
                        });
                        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                            if ui.button("Verwerfen").clicked() {
                                einladung_aktion = Some((e.id.clone(), e.link.clone(), false));
                            }
                            if ui.button("Beitreten").clicked() {
                                einladung_aktion = Some((e.id.clone(), e.link.clone(), true));
                            }
                        });
                    });
                });
            ui.add_space(5.0);
        }

        // ------------------------------------------------ neue Anfrage
        egui::Frame::NONE
            .fill(p.card)
            .stroke(egui::Stroke::new(1.0, p.line))
            .corner_radius(10)
            .inner_margin(egui::Margin::same(12))
            .show(ui, |ui| {
                ui.set_width(ui.available_width());
                label_small(
                    ui,
                    if d.angemeldet() {
                        "FreeViewer-Nummer oder @Name des Freundes"
                    } else {
                        "FreeViewer-Nummer des Freundes"
                    },
                );
                ui.horizontal(|ui| {
                    let feld = ui.add(
                        egui::TextEdit::singleline(&mut self.freunde_id)
                            .desired_width(190.0)
                            .hint_text(if d.angemeldet() { "497 628 420 oder @anna" } else { "497 628 420" }),
                    );
                    let senden = ui.button("Anfrage senden").clicked()
                        || (feld.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter)));
                    if senden {
                        match d.anfragen(&self.freunde_id) {
                            Ok(()) => {
                                self.freunde_meldung = format!(
                                    "Anfrage an {} unterwegs - sie gilt erst, wenn er zustimmt.",
                                    freunde::ziel_anzeige(&self.freunde_id)
                                );
                                self.freunde_id.clear();
                            }
                            Err(e) => self.freunde_meldung = format!("{}", e),
                        }
                    }
                });
                if !self.freunde_meldung.is_empty() {
                    ui.add_space(4.0);
                    ui.label(
                        egui::RichText::new(&self.freunde_meldung)
                            .size(11.0)
                            .color(p.muted),
                    );
                }
            });
        ui.add_space(10.0);

        // ------------------------------------------------ offene Anfragen
        let eingehend = d.eingehende();
        let ausgehend = d.ausgehende();
        if !eingehend.is_empty() || !ausgehend.is_empty() {
            label_small(ui, "Anfragen");
            for a in eingehend.iter() {
                egui::Frame::NONE
                    .fill(p.card_hi)
                    .stroke(egui::Stroke::new(1.0, p.accent))
                    .corner_radius(10)
                    .inner_margin(egui::Margin::symmetric(12, 8))
                    .show(ui, |ui| {
                        ui.set_width(ui.available_width());
                        ui.horizontal(|ui| {
                            ui.label(
                                egui::RichText::new(format!("{} möchte dein Freund sein", a.anzeige()))
                                    .size(12.5)
                                    .color(p.text),
                            );
                            ui.with_layout(
                                egui::Layout::right_to_left(egui::Align::Center),
                                |ui| {
                                    if ui.button("Ablehnen").clicked() {
                                        if let Err(e) = d.ablehnen(&a.gegenueber()) {
                                            self.freunde_meldung = format!("{}", e);
                                        }
                                    }
                                    if ui.button("Annehmen").clicked() {
                                        match d.annehmen(&a.gegenueber()) {
                                            Ok(()) => {
                                                self.freunde_meldung =
                                                    format!("{} ist jetzt dein Freund.", a.anzeige())
                                            }
                                            Err(e) => self.freunde_meldung = format!("{}", e),
                                        }
                                    }
                                },
                            );
                        });
                        if !a.nachricht.is_empty() {
                            ui.label(
                                egui::RichText::new(if a.nachricht.chars().count() > 90 {
                                    format!(
                                        "{}…",
                                        a.nachricht.chars().take(89).collect::<String>()
                                    )
                                } else {
                                    a.nachricht.clone()
                                })
                                    .size(11.0)
                                    .color(p.muted),
                            );
                        }
                    });
                ui.add_space(4.0);
            }
            for a in ausgehend.iter() {
                ui.horizontal(|ui| {
                    ui.label(
                        egui::RichText::new(format!("an {} – wartet auf Zustimmung", a.anzeige()))
                            .size(11.5)
                            .color(p.muted),
                    );
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        if ui.button("Zurücknehmen").clicked() {
                            if let Err(e) = d.ablehnen(&a.gegenueber()) {
                                self.freunde_meldung = format!("{}", e);
                            }
                        }
                    });
                });
            }
            ui.add_space(10.0);
        }

        // ------------------------------------------------ the list
        let liste = d.liste();
        label_small(ui, "Deine Freunde");
        if liste.is_empty() {
            ui.add_space(6.0);
            ui.label(
                egui::RichText::new("Noch niemand. Schick oben eine Anfrage.")
                    .size(12.0)
                    .color(p.muted),
            );
        }
        egui::ScrollArea::vertical().show(ui, |ui| {
            for fr in liste.iter() {
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
                                    egui::RichText::new(fr.anzeige()).size(13.0).color(p.text),
                                );
                                // With an account: @name, then the PC (with several
                                // "+N"), otherwise "no FreeViewer PC".
                                let mut unter: Vec<String> = Vec::new();
                                if !fr.handle.is_empty() && fr.name.trim() != format!("@{}", fr.handle) {
                                    unter.push(format!("@{}", fr.handle));
                                }
                                if fr.hat_pc() {
                                    let mehr = fr.geraete.len().saturating_sub(1);
                                    unter.push(if mehr > 0 {
                                        format!("{} +{} PC", partners::pretty_id(&fr.fvid), mehr)
                                    } else {
                                        partners::pretty_id(&fr.fvid)
                                    });
                                    unter.push(fr.zuletzt());
                                } else {
                                    unter.push("kein FreeViewer-PC".to_string());
                                }
                                let r = ui.label(
                                    egui::RichText::new(unter.join(" · "))
                                        .size(10.5)
                                        .color(p.muted),
                                );
                                if fr.geraete.len() > 1 {
                                    r.on_hover_text(
                                        fr.geraete
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
                                        if let Err(e) = d.entfernen(&fr.schluessel()) {
                                            self.freunde_meldung = format!("{}", e);
                                        }
                                    }
                                    // Only offer what actually works:
                                    // remote maintenance to an offline
                                    // machine would be a button into nothing.
                                    ui.add_enabled_ui(fr.erreichbar(), |ui| {
                                        if ui
                                            .button("Fernwartung")
                                            .on_hover_text("Sofort verbinden – zulassen muss er es trotzdem")
                                            .clicked()
                                        {
                                            verbinden = Some(fr.fvid.clone());
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

        if let Some(id) = verbinden {
            self.view = View::Start;
            self.hint = i18n::tf("link.control", &partners::pretty_id(&id));
            self.connect_to(&id);
        }
        if let Some((id, link, beitreten)) = einladung_aktion {
            if beitreten {
                // Only open the meeting page of this brand - never an arbitrary
                // address a friend (or a stranger in their name) sends.
                if freunde::einladung_link_ok(&link, &meet::base()) {
                    match meet::open_window(&link) {
                        Ok(()) => {
                            self.freunde_meldung = "Meeting wird geöffnet.".to_string();
                            d.einladung_erledigt(&id);
                        }
                        Err(e) => self.freunde_meldung = format!("Meeting geht nicht: {}", e),
                    }
                } else {
                    self.freunde_meldung =
                        "Dieser Einladungslink zeigt nicht auf die Meeting-Seite - nicht geöffnet.".to_string();
                    d.einladung_erledigt(&id);
                }
            } else {
                d.einladung_erledigt(&id);
            }
        }
        if let Some(fr) = meeting_mit {
            let wer = fr.anzeige();
            match meet::create(&format!("Mit {}", wer), false) {
                Ok(m) => {
                    // The link is ALWAYS in the clipboard: if delivery
                    // fails (old relay, friend without a PC), this way remains.
                    let link = meet::invite(&m);
                    ui.ctx().copy_text(link);
                    meet::merken(&m);
                    // Only the bare address goes to the relay - and only without an
                    // end-to-end key (the relay must never see that).
                    let relay_link = if m.e2e.is_empty() {
                        Some(meet::join_url(&m.id, &m.passwort))
                    } else {
                        None
                    };
                    self.freunde_meldung = match relay_link
                        .as_deref()
                        .map(|l| d.meeting_einladen(&fr, &format!("Mit {}", wer), l))
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
                Err(e) => self.freunde_meldung = format!("Meeting geht nicht: {}", e),
            }
        }
    }

}
