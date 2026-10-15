//! Split out of main.rs (r35): mechanical move, behaviour unchanged.
use super::*;

impl App {
    /// Conferences. The directory lives on the Meet server, so all
    /// queries run in their own thread - a hanging line must not freeze the
    /// UI.
    pub(crate) fn meet_refresh(&mut self) {
        if self.meet_busy.load(Ordering::Relaxed) {
            return;
        }
        self.meet_busy.store(true, Ordering::Relaxed);
        let list = self.meet_list.clone();
        let err = self.meet_err.clone();
        let busy = self.meet_busy.clone();
        std::thread::spawn(move || {
            match meet::list() {
                Ok(v) => {
                    *list.lock().unwrap() = v;
                    err.lock().unwrap().clear();
                }
                Err(e) => *err.lock().unwrap() = format!("{}", e),
            }
            busy.store(false, Ordering::Relaxed);
        });
    }

    pub(crate) fn meet_view(&mut self, ui: &mut egui::Ui) {
        let p = theme::palette();
        if !self.meet_loaded {
            self.meet_loaded = true;
            self.meet_refresh();
        }

        ui.columns(2, |cols| {
            // ---------------------------------------------- neues Meeting
            let ui = &mut cols[0];
            section(ui, i18n::t("meet.new"));
            card(ui, |ui| {
                ui.add(
                    egui::TextEdit::singleline(&mut self.meet_title)
                        .desired_width(240.0)
                        .hint_text(i18n::t("meet.title_hint"))
                        .margin(egui::Margin::symmetric(8, 5)),
                );
                ui.add_space(8.0);
                check(ui, &mut self.meet_e2e, i18n::t("meet.e2e"));
                ui.add_space(4.0);
                label_small(
                    ui,
                    if self.meet_e2e {
                        i18n::t("meet.e2e_on")
                    } else {
                        i18n::t("meet.e2e_off")
                    },
                );
                ui.add_space(8.0);
                if ghost_button(ui, i18n::t("meet.start")).clicked() {
                    match meet::create(&self.meet_title, self.meet_e2e) {
                        Ok(m) => {
                            self.meet_id = m.id.clone();
                            self.meet_pw = m.passwort.clone();
                            self.meet_last = Some(m.clone());
                            // Sofort ins eigene Meeting-Fenster wechseln.
                            self.meet_win_open(m);
                            self.hint = i18n::t("meet.created").to_string();
                            self.meet_loaded = false;
                        }
                        Err(e) => self.hint = format!("{}: {}", i18n::t("meet.err"), e),
                    }
                }
                if let Some(m) = self.meet_last.clone() {
                    ui.add_space(10.0);
                    divider(ui);
                    ui.add_space(8.0);
                    label_small(ui, i18n::t("meet.id"));
                    ui.add(
                        egui::Label::new(
                            egui::RichText::new(&m.id)
                                .size(24.0)
                                .family(egui::FontFamily::Monospace)
                                .color(p.text),
                        )
                        .selectable(true),
                    );
                    ui.add_space(4.0);
                    label_small(ui, i18n::t("meet.pw"));
                    ui.add(
                        egui::Label::new(
                            egui::RichText::new(&m.passwort)
                                .size(18.0)
                                .family(egui::FontFamily::Monospace)
                                .color(p.text),
                        )
                        .selectable(true),
                    );
                    ui.add_space(10.0);
                    ui.horizontal(|ui| {
                        if ghost_button(ui, i18n::t("meet.join")).clicked() {
                            self.meet_win_open(m.clone());
                        }
                        ui.add_space(6.0);
                        if ghost_button(ui, i18n::t("meet.copy_invite")).clicked() {
                            ui.ctx().copy_text(meet::invite(&m));
                            self.hint = i18n::t("meet.copied").to_string();
                        }
                    });
                }
            });

            // ---------------------------------------------- beitreten
            let ui = &mut cols[1];
            section(ui, i18n::t("meet.join_head"));
            card(ui, |ui| {
                label_small(ui, i18n::t("meet.id"));
                ui.add(
                    egui::TextEdit::singleline(&mut self.meet_id)
                        .desired_width(180.0)
                        .hint_text("123-456-789")
                        .font(egui::FontId::new(16.0, egui::FontFamily::Monospace))
                        .margin(egui::Margin::symmetric(8, 5)),
                );
                ui.add_space(6.0);
                label_small(ui, i18n::t("meet.pw"));
                ui.add(
                    egui::TextEdit::singleline(&mut self.meet_pw)
                        .desired_width(180.0)
                        .password(true)
                        .margin(egui::Margin::symmetric(8, 5)),
                );
                ui.add_space(10.0);
                check(ui, &mut self.meet_offer_control, i18n::t("meet.offer"))
                    .on_hover_text(i18n::t("meet.offer_tip"));
                ui.add_space(8.0);
                if ghost_button(ui, i18n::t("meet.join")).clicked() {
                    let id = self.meet_id.trim().to_string();
                    if id.is_empty() {
                        self.hint = i18n::t("meet.need_id").to_string();
                    } else {
                        self.meet_win_open(meet::Meeting {
                            id,
                            titel: String::new(),
                            passwort: self.meet_pw.clone(),
                            termin_text: String::new(),
                            e2e: String::new(),
                        });
                    }
                }
                ui.add_space(8.0);
                ui.label(
                    egui::RichText::new(i18n::t("meet.note"))
                        .size(11.5)
                        .color(p.muted),
                );
            });

            ui.add_space(12.0);
            section(ui, i18n::t("meet.running"));
            card(ui, |ui| {
                let err = self.meet_err.lock().unwrap().clone();
                let list = self.meet_list.lock().unwrap().clone();
                if !err.is_empty() {
                    ui.label(
                        egui::RichText::new(format!("{}: {}", i18n::t("meet.err"), err))
                            .size(12.0)
                            .color(p.violet),
                    );
                } else if list.is_empty() {
                    ui.label(
                        egui::RichText::new(i18n::t("meet.none"))
                            .size(12.0)
                            .color(p.muted),
                    );
                }
                for m in list.iter() {
                    // The WHOLE row is clickable, not just the small
                    // arrow on the right - and it shows that with the pointing hand.
                    let zeile = ui.horizontal(|ui| {
                        dot(ui, true);
                        ui.label(
                            egui::RichText::new(&m.id)
                                .size(13.0)
                                .family(egui::FontFamily::Monospace)
                                .color(p.text),
                        );
                        if !m.titel.is_empty() {
                            ui.label(egui::RichText::new(&m.titel).size(12.0).color(p.muted));
                        }
                        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                            if icon_ghost(ui, "connect", i18n::t("meet.join")).clicked() {
                                self.meet_id = m.id.clone();
                            }
                            ui.add_space(ui.available_width());
                        });
                    })
                    .response;
                    let zeile = zeile.interact(egui::Sense::click());
                    if zeile.hovered() {
                        ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
                    }
                    if zeile.clicked() {
                        self.meet_id = m.id.clone();
                    }
                    ui.add_space(3.0);
                }
                ui.add_space(6.0);
                if ghost_button(ui, i18n::t("meet.refresh")).clicked() {
                    self.meet_loaded = false;
                }
            });
        });
    }

    /// Opens the own meeting window for this meeting (Zoom flow:
    /// first check devices and copy invitation, then join).
    pub(crate) fn meet_win_open(&mut self, m: meet::Meeting) {
        self.meet_chrome = 0;
        // Add to the OWN list. The overview no longer comes from the
        // server (it showed it to every stranger) - each machine keeps its
        // own.
        meet::merken(&m);
        self.meet_win.offen = true;
        if self.meet_win.name.trim().is_empty() {
            // Suggestion: the machine name - that is how you are called in the browser too.
            self.meet_win.name = presence::device_name();
        }
        self.meet_win.beigetreten = false;
        self.meet_win.toast = None;
        self.meet_win.meeting = Some(m);
        self.meet_win.tn.lock().unwrap().clear();
        // Check right away who is already there.
        self.meet_win.tn_next = std::time::Instant::now();
        // Recolor the title bar of the meeting window. It only appears
        // a few frames after opening - so once is not enough.
        if self.meet_chrome < 30 {
            self.meet_chrome += 1;
            chrome::paint_from_theme();
        }
        // Screens are available immediately (pure system query) - we need them
        // to ASK which one should be shared when there are several.
        self.meet_win.monitore = meetschirm::liste()
            .into_iter()
            .map(|m| (m.name, m.breite, m.hoehe))
            .collect();
        if std::time::Instant::now() >= self.meet_win.fenster_naechste {
            self.meet_win.fenster_naechste =
                std::time::Instant::now() + std::time::Duration::from_millis(1500);
            self.meet_win.fenster = fenster::liste()
                .into_iter()
                .map(|f| (f.kennung, f.titel, f.programm))
                .collect();
        }
        if !self.meet_win.geraete_geladen {
            self.meet_win.geraete_geladen = true;
            let slot = self.meet_win.geraete.clone();
            std::thread::spawn(move || {
                let (mics, mut cams) = meet::geraete();
                // For the selection IN the meeting, the names the camera
                // opener itself knows count (Media Foundation). The
                // PnP list is only a fallback if that stays empty.
                let echte = crate::meetcam::liste();
                if !echte.is_empty() {
                    cams = echte.into_iter().map(|g| g.name).collect();
                }
                let (_, spks) = crate::meetaudio::geraete_liste();
                *slot.lock().unwrap() = Some((mics, cams, spks));
            });
        }
    }

    /// Reload the participant list - every 5 s, in a background thread,
    /// so that a hanging line does not freeze the window.
    pub(crate) fn meet_win_poll(&mut self) {
        if !self.meet_win.offen
            || self.meet_win.tn_busy.load(Ordering::Relaxed)
            || std::time::Instant::now() < self.meet_win.tn_next
        {
            return;
        }
        let (id, pass) = match self.meet_win.meeting.as_ref() {
            Some(m) => (m.id.clone(), m.passwort.clone()),
            None => return,
        };
        if pass.is_empty() {
            // Without a password the server does not reveal the list.
            return;
        }
        self.meet_win.tn_busy.store(true, Ordering::Relaxed);
        self.meet_win.tn_next = std::time::Instant::now() + Duration::from_secs(5);
        let tn = self.meet_win.tn.clone();
        let busy = self.meet_win.tn_busy.clone();
        std::thread::spawn(move || {
            if let Ok(v) = meet::teilnehmer(&id, &pass) {
                *tn.lock().unwrap() = v;
            }
            busy.store(false, Ordering::Relaxed);
        });
    }

    /// The own meeting window - just like the FreeMeet browser client.
    ///
    /// Before joining there is the centered card with live self preview,
    /// afterwards the meeting: marks on top, stage in the middle, sidebar
    /// on the right, control bar at the bottom. DRAWING happens in `meetfenster` - here
    /// we only gather what it needs and execute what the
    /// user clicked. This separation is the reason why every
    /// view can be rendered through in `--uitest` without server,
    /// camera and sound card.
    /// Meeting window closed = leave the meeting.
    ///
    /// WHY not just `offen = false`: the window holds real devices.
    /// Previously the camera kept running after closing (light on) and
    /// the server kept considering us present - exactly what Justin
    /// reported. The cross must do the same as "Leave".
    pub(crate) fn meet_win_schliessen(&mut self) {
        // The preview before joining holds camera AND microphone.
        self.vorprobe.aus();
        if let Some(n) = self.nativ_meet.take() {
            n.verlassen();
        }
        self.nativ_bilder.clear();
        self.nativ_schirme.clear();
        self.nativ_eigen = None;
        self.meet_win.beigetreten = false;
        self.meet_win.nativ_start = false;
        self.meet_kam_start = None;
        // The small window must close too, otherwise it is left standing alone.
        self.meet_z = meetfenster::Fensterzustand::default();
        self.meet_win.offen = false;
    }

    pub(crate) fn meet_win_ui(&mut self, ctx: &egui::Context) {
        // Take over the device list as soon as the background thread is done.
        if !self.meet_win.geraete_da {
            if let Some((mics, cams, spks)) = self.meet_win.geraete.lock().unwrap().clone() {
                self.meet_win.mics = mics;
                self.meet_win.cams = cams;
                self.meet_win.spks = spks;
                self.meet_win.geraete_da = true;
            }
        }
        self.meet_win_poll();
        let m = match self.meet_win.meeting.clone() {
            Some(m) => m,
            None => return,
        };
        if self.nativ_meet.is_none() {
            self.meet_beitritt_ui(ctx, &m);
        } else {
            self.meet_lauf_ui(ctx, &m);
        }
        self.meet_toast(ctx);
    }

    /// The screen before joining (`#vorbereiten` in the browser).
    pub(crate) fn meet_beitritt_ui(&mut self, ctx: &egui::Context, m: &meet::Meeting) {
        // Bring camera and microphone of the preview in line with the wish. Only touch on
        // a REAL change - otherwise a device error would trigger a new
        // open attempt every frame.
        let will_kamera = !self.meet_win.ohne_video;
        if will_kamera != self.vorprobe.wunsch_kamera {
            self.vorprobe.kamera(will_kamera);
        }
        let mic = if self.meet_win.mic_sel == 0 {
            None
        } else {
            self.meet_win.mics.get(self.meet_win.mic_sel - 1).cloned()
        };
        let will_mikro = !self.meet_win.stumm;
        if will_mikro != self.vorprobe.wunsch_mikro || mic != self.vorprobe.mikro_geraet {
            self.vorprobe.mikro(will_mikro, mic.clone());
        }
        self.vorprobe.pumpe();

        // Pour the preview image into a texture (only if there is a new one).
        match self.vorprobe.eigen.as_ref() {
            Some((w, h, px)) => {
                let stand = self.vorprobe.stand;
                let frisch = self
                    .nativ_eigen
                    .as_ref()
                    .map(|(alt, _)| *alt != stand)
                    .unwrap_or(true);
                if frisch {
                    let bild =
                        egui::ColorImage::from_rgba_unmultiplied([*w as usize, *h as usize], px);
                    let tex = ctx.load_texture("meeteigen", bild, egui::TextureOptions::LINEAR);
                    self.nativ_eigen = Some((stand, tex));
                }
            }
            None => self.nativ_eigen = None,
        }

        let mut b = meetfenster::Beitritt {
            raum: m.id.clone(),
            titel: m.titel.clone(),
            name: self.meet_win.name.clone(),
            mics: self.meet_win.mics.clone(),
            cams: self.meet_win.cams.clone(),
            mic_sel: self.meet_win.mic_sel,
            cam_sel: self.meet_win.cam_sel,
            mikro_an: will_mikro,
            kamera_an: will_kamera,
            geraete_da: self.meet_win.geraete_da,
            pegel: self.vorprobe.pegel,
            hinweis: self.vorprobe.meldung.clone(),
            laeuft: self.meet_win.nativ_start,
        };
        let tex = self.nativ_eigen.as_ref().map(|(_, t)| t.clone());
        let aktion = meetfenster::beitritt_ui(ctx, &mut b, tex.as_ref());
        self.meet_win.name = b.name;
        self.meet_win.mic_sel = b.mic_sel;
        self.meet_win.cam_sel = b.cam_sel;

        match aktion {
            Some(meetfenster::Beitrittsaktion::MikroUm) => {
                self.meet_win.stumm = !self.meet_win.stumm;
            }
            Some(meetfenster::Beitrittsaktion::KameraUm) => {
                self.meet_win.ohne_video = !self.meet_win.ohne_video;
            }
            Some(meetfenster::Beitrittsaktion::Zurueck) => {
                self.meet_win_schliessen();
            }
            Some(meetfenster::Beitrittsaktion::Beitreten) => {
                self.meet_win.nativ_start = true;
            }
            None => {}
        }

        // Joining happens only AFTER the frame is drawn: the
        // preview has to release the camera first, otherwise the
        // meeting does not get it.
        if std::mem::take(&mut self.meet_win.nativ_start) {
            self.vorprobe.aus();
            self.nativ_eigen = None;
            let me = self.shared.my_id.lock().unwrap().clone();
            let name = if self.meet_win.name.trim().is_empty() {
                presence::device_name()
            } else {
                self.meet_win.name.trim().to_string()
            };
            let mic = if self.meet_win.mic_sel == 0 {
                None
            } else {
                self.meet_win.mics.get(self.meet_win.mic_sel - 1).cloned()
            };
            match meetui::NativMeet::beitreten(
                &meet::base(),
                &m.id,
                &m.passwort,
                &name,
                &me,
                self.meet_offer_control,
                mic,
                None,
                // End-to-end: the key is set in the meeting (from the
                // link or rolled when creating). Empty = as before.
                if m.e2e.trim().is_empty() { None } else { Some(m.e2e.clone()) },
            ) {
                Ok(mut n) => {
                    if self.meet_win.stumm {
                        n.stumm_schalten(true);
                    }
                    self.meet_kam_start = if self.meet_win.ohne_video {
                        None
                    } else {
                        Some(std::time::Instant::now() + std::time::Duration::from_millis(600))
                    };
                    self.nativ_meet = Some(n);
                    self.meet_win.beigetreten = true;
                    self.meet_gelesen = 0;
                    self.meet_z = meetfenster::Fensterzustand::default();
                    self.meet_win.tn_next = std::time::Instant::now();
                }
                Err(e) => {
                    self.meet_win.toast = Some((format!("{}", e), std::time::Instant::now()));
                }
            }
        }
    }

}
