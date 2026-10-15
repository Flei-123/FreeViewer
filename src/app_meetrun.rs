//! Split out of main.rs (r35): mechanical move, behaviour unchanged.
use super::*;

impl App {
    /// The running meeting.
    pub(crate) fn meet_lauf_ui(&mut self, ctx: &egui::Context, m: &meet::Meeting) {
        let sicht: meetfenster::Sicht;
        let bilder: meetfenster::Bilder;
        // Whose screen is currently seen large - this determines
        // who we send our zoom request to.
        let mut haupt_schirm_peer: Option<u64> = None;
        {
            let n = match self.nativ_meet.as_mut() {
                Some(n) => n,
                None => return,
            };
            n.pumpe();
            // Start the camera only now - the preview has just
            // released it and needs a moment for that.
            if let Some(wann) = self.meet_kam_start {
                if std::time::Instant::now() >= wann {
                    self.meet_kam_start = None;
                    n.kamera_schalten(true);
                }
            }
            let z = n.zustand();
            // Only when taking images (--meetshot): let waiting people in right away.
            // Otherwise you would never see a second participant in the proof image.
            if self.meet_shot_join && !z.wartende.is_empty() {
                n.alle_einlassen();
            }
            let zahlen = n.zahlen();

            // ---- Reload textures (only for a really new image)
            match n.eigen.as_ref() {
                Some((w, h, px)) => {
                    let frisch = self
                        .nativ_eigen
                        .as_ref()
                        .map(|(alt, _)| *alt != n.eigen_stand)
                        .unwrap_or(true);
                    if frisch {
                        let bild = egui::ColorImage::from_rgba_unmultiplied(
                            [*w as usize, *h as usize],
                            px,
                        );
                        let tex = ctx.load_texture("meeteigen", bild, egui::TextureOptions::LINEAR);
                        self.nativ_eigen = Some((n.eigen_stand, tex));
                    }
                }
                None => self.nativ_eigen = None,
            }
            for (peer, (w, h, px)) in n.bilder.bilder.iter() {
                let stand = *n.bilder.stand.get(peer).unwrap_or(&0);
                let frisch = self
                    .nativ_bilder
                    .get(peer)
                    .map(|(alt, _)| *alt != stand)
                    .unwrap_or(true);
                if frisch {
                    let bild =
                        egui::ColorImage::from_rgba_unmultiplied([*w as usize, *h as usize], px);
                    let tex =
                        ctx.load_texture(format!("meetcam{}", peer), bild, egui::TextureOptions::LINEAR);
                    self.nativ_bilder.insert(*peer, (stand, tex));
                }
            }
            self.nativ_bilder
                .retain(|k, _| n.bilder.bilder.contains_key(k));
            for (peer, (w, h, px)) in n.schirme.bilder.iter() {
                let stand = *n.schirme.stand.get(peer).unwrap_or(&0);
                let frisch = self
                    .nativ_schirme
                    .get(peer)
                    .map(|(alt, _)| *alt != stand)
                    .unwrap_or(true);
                if frisch {
                    let bild =
                        egui::ColorImage::from_rgba_unmultiplied([*w as usize, *h as usize], px);
                    let tex = ctx.load_texture(
                        format!("meetschirm{}", peer),
                        bild,
                        egui::TextureOptions::LINEAR,
                    );
                    self.nativ_schirme.insert(*peer, (stand, tex));
                }
            }
            self.nativ_schirme
                .retain(|k, _| n.schirme.bilder.contains_key(k));
            // The own shared image. Without this tile you see nothing
            // at all when sharing - the button then seems to have no effect.
            match n.eigen_schirm.as_ref() {
                Some((w, h, px)) => {
                    let frisch = self
                        .nativ_eigen_schirm
                        .as_ref()
                        .map(|(alt, _)| *alt != n.eigen_schirm_stand)
                        .unwrap_or(true);
                    if frisch {
                        let bild = egui::ColorImage::from_rgba_unmultiplied(
                            [*w as usize, *h as usize],
                            px,
                        );
                        let tex = ctx.load_texture(
                            "meeteigenschirm",
                            bild,
                            egui::TextureOptions::LINEAR,
                        );
                        self.nativ_eigen_schirm = Some((n.eigen_schirm_stand, tex));
                    }
                }
                None => self.nativ_eigen_schirm = None,
            }

            // ---- Sicht zusammenstellen
            let ich_name = if self.meet_win.name.trim().is_empty() {
                presence::device_name()
            } else {
                self.meet_win.name.trim().to_string()
            };
            let mut leute = vec![meetfenster::Person {
                id: z.ich,
                name: ich_name,
                stumm: n.stumm,
                kamera_aus: !n.kamera_an,
                hand: n.hand,
                gastgeber: z.gastgeber != 0 && z.gastgeber == z.ich,
                fvid: String::new(),
                ich: true,
                spricht: !n.stumm && n.pegel > 0.06,
            }];
            for t in z.leute.iter() {
                leute.push(meetfenster::Person {
                    id: t.id,
                    name: t.name.clone(),
                    stumm: t.ton_aus,
                    kamera_aus: t.bild_aus,
                    hand: t.hand,
                    gastgeber: z.gastgeber != 0 && z.gastgeber == t.id,
                    fvid: t.fvid.clone(),
                    ich: false,
                    spricht: n.spricht(t.id),
                });
            }
            let chat: Vec<meetfenster::Chatzeile> = n
                .chat
                .iter()
                .map(|(von, text)| meetfenster::Chatzeile {
                    von: *von,
                    name: if *von == 0 {
                        String::new()
                    } else {
                        n.name_von(*von)
                    },
                    text: text.clone(),
                    eigen: *von == z.ich,
                })
                .collect();
            // Chat counts as read as long as the sidebar shows it.
            if self.meet_z.seite_offen && self.meet_z.reiter == meetfenster::Reiter::Chat {
                self.meet_gelesen = chat.len();
            }
            let ungelesen = chat.len().saturating_sub(self.meet_gelesen) as u32;

            let mut protokoll = n.protokoll.clone();
            for meldung in [&n.meldung, &n.kamera_meldung, &n.schirm_meldung] {
                if !meldung.is_empty() {
                    protokoll.push(meldung.clone());
                }
            }
            protokoll.push(format!(
                "Ton: {} raus / {} rein · Bild: {} raus / {} rein",
                zahlen.gesendet, zahlen.empfangen, zahlen.bild_gesendet, zahlen.bild_empfangen
            ));
            if n.schirm_an {
                protokoll.push(format!(
                    "Bildschirm geteilt: {} Bilder gesendet",
                    n.schirm_gesendet
                ));
            }
            protokoll.push(format!("Meine Nummer: {}", n.meine_fvid()));

            let bandbreite = if n.kbit_rein + n.kbit_raus > 0 {
                format!("{} kbit/s", n.kbit_rein + n.kbit_raus)
            } else {
                String::new()
            };
            let tippen: Vec<String> = n.tippen.iter().map(|id| n.name_von(*id)).collect();
            // The server text about the waiting room is in the last message.
            let warte_text = if z.im_warteraum { n.meldung.clone() } else { String::new() };
            let mut schirme: Vec<(u64, String)> = self
                .nativ_schirme
                .keys()
                .map(|id| (*id, n.name_von(*id)))
                .collect();
            // The OWN share belongs on the stage too - like in the
            // browser. Otherwise you click "Screen" and see nothing.
            if n.schirm_an && self.nativ_eigen_schirm.is_some() {
                schirme.push((z.ich, "Du".to_string()));
            }

            let ton_namen = n.ton_namen();
            // Whose screen is the main content? The first foreign one.
            let schirm_haupt: Option<u64> = schirme
                .iter()
                .map(|(id, _)| *id)
                .find(|id| *id != z.ich)
                .or_else(|| schirme.first().map(|(id, _)| *id));
            haupt_schirm_peer = schirm_haupt;
            sicht = meetfenster::Sicht {
                raum: if z.raum.is_empty() { m.id.clone() } else { z.raum.clone() },
                titel: if z.titel.is_empty() { m.titel.clone() } else { z.titel.clone() },
                gastgeber: n.bin_gastgeber(),
                verbindung: if zahlen.verbunden {
                    "direkt verbunden".to_string()
                } else if z.verbunden {
                    "Medien verbinden …".to_string()
                } else {
                    "verbinde …".to_string()
                },
                verbindung_ton: if zahlen.verbunden {
                    meetfenster::Ton::Gut
                } else {
                    meetfenster::Ton::Warn
                },
                // Green only if encryption is REALLY happening - i.e. a
                // key exists. A green mark without encryption
                // would be the worst kind of lie.
                e2e: n.e2e_an,
                bandbreite,
                leute,
                wartende: z.wartende.clone(),
                warteraum_an: z.warteraum_an,
                chat,
                protokoll,
                stumm: n.stumm,
                kamera_an: n.kamera_an,
                schirm_an: n.schirm_an,
                hand: n.hand,
                steuer_frei: n.steuer_frei,
                steuer_anfragen: n.steuer_anfragen(),
                steuer_erlaubt: n.steuer_erlaubt(),
                steuer_gefragt: n.steuer_gefragt,
                steuer_zusage: n.steuer_zusage(),
                fremd_schirm: n.fremder_teilt(),
                schirme,
                ungelesen,
                tippen,
                im_warteraum: z.im_warteraum,
                warte_text: warte_text.clone(),
                cams: self.meet_win.cams.clone(),
                mics: self.meet_win.mics.clone(),
                spks: self.meet_win.spks.clone(),
                cam_sel: self.meet_win.cam_sel,
                mic_sel: self.meet_win.mic_sel,
                spk_sel: self.meet_win.spk_sel,
                kamera_name: n.kamera_name(),
                ton_ein: ton_namen.0,
                ton_aus: ton_namen.1,
                // Pointer OF THE ONE whose screen is currently seen large.
                // If it is the OWN one, nothing comes back over the network -
                // then look it up directly.
                teiler_zeiger: match schirm_haupt {
                    Some(p) if p == z.ich => n.eigener_zeiger(),
                    Some(p) => n.zeiger_des_teilers(p),
                    None => None,
                },
                // Same for the crop: the own announcement only goes
                // to the others. Whoever looks at the own share must still know the
                // crop - otherwise it is zoomed TWICE and
                // the image turns black from about 140 %.
                schirm_bereich: match schirm_haupt {
                    Some(p) if p == z.ich => Some(n.schirm_bereich),
                    Some(p) => n.bereich_des_teilers(p),
                    None => None,
                },
                scharf_moeglich: match schirm_haupt {
                    Some(p) if p == z.ich => true,
                    Some(p) => n.kann_scharf(p),
                    None => false,
                },
                monitore: self.meet_win.monitore.clone(),
                fenster: self.meet_win.fenster.clone(),
                rahmen_an: n.rahmen_an,
                avatar_an: n.avatar_an,
                avatar: n.avatar,
                sperre_an: n.sperre_an,
                sperre_staerke: n.sperre_staerke,
                sperre_offen: n.sperre_offen(),
                sperre_grund: n.sperre_grundrauschen(),
                eigener_pegel: n.pegel,
            };
            bilder = meetfenster::Bilder {
                eigen: self.nativ_eigen.as_ref().map(|(_, t)| t.clone()),
                kameras: self
                    .nativ_bilder
                    .iter()
                    .map(|(k, (_, t))| (*k, t.clone()))
                    .collect(),
                schirme: self
                    .nativ_eigen_schirm
                    .as_ref()
                    .map(|(_, t)| (z.ich, t.clone()))
                    .into_iter()
                    .chain(
                        self.nativ_schirme
                            .iter()
                            .map(|(k, (_, t))| (*k, t.clone())),
                    )
                    .collect(),
            };
        }

        // At the door: the waiting screen instead of an empty room. Without it
        // the guest would sit in front of a black area and think the program
        // is broken - exactly the case noticed during the rehearsal.
        if sicht.im_warteraum {
            if meetfenster::warteschirm_ui(ctx, &sicht.titel, &sicht.warte_text) {
                if let Some(n) = self.nativ_meet.take() {
                    n.verlassen();
                }
                self.meet_win.beigetreten = false;
                self.meet_kam_start = None;
            }
            return;
        }
        let mut aktionen = meetfenster::meeting_ui(ctx, &sicht, &mut self.meet_z, &bilder);
        // Picture-in-picture: own small window that stays on top - otherwise
        // you see nobody anymore when sharing your own screen.
        if self.meet_z.pip && !self.headless {
            let mut zu = false;
            // The switch lives in the window state, but the close is in the
            // viewport callback - hence a copy here that is written back
            // afterwards (otherwise `self` would be borrowed twice).
            let mut selbst = self.meet_z.pip_selbst;
            let mut pip_aktionen: Vec<meetfenster::Aktion> = Vec::new();
            ctx.show_viewport_immediate(
                egui::ViewportId::from_hash_of("fv_meet_pip"),
                egui::ViewportBuilder::default()
                    .with_title("Meeting")
                    .with_inner_size([260.0, 330.0])
                    .with_always_on_top(),
                |pctx, _class| {
                    egui::CentralPanel::default()
                        .frame(egui::Frame::NONE.fill(theme::bg()))
                        .show(pctx, |ui| {
                            pip_aktionen
                                .extend(meetfenster::pip_inhalt(ui, &sicht, &bilder, &mut selbst));
                        });
                    if pctx.input(|i| i.viewport().close_requested()) {
                        zu = true;
                    }
                    // The small window shows the same images - it must
                    // not be the slower of the two.
                    pctx.request_repaint_after(Duration::from_millis(33));
                },
            );
            self.meet_z.pip_selbst = selbst;
            aktionen.extend(pip_aktionen);
            if zu {
                self.meet_z.pip = false;
            }
        }

        // Tell the sender which crop we are looking at. It then cuts
        // exactly that from its native capture - same bandwidth, but
        // real pixels. Without this message it would stay with upscaling.
        if let (Some(p), Some(n)) = (haupt_schirm_peer, self.nativ_meet.as_mut()) {
            let f = self.meet_z.zoomfenster;
            let zoomt = self.meet_z.zoom > 1.001;
            n.zoom_wunsch(p, f, zoomt && self.meet_z.scharf);
        }

        let mut steuern_zu: Option<(String, String)> = None;
        let mut verlassen = false;
        // If sharing fails, it MUST become visible. Previously it was
        // only in the info tab - for the user it looked as if nothing
        // happened at all.
        let mut schirm_fehler: Option<String> = None;
        for a in aktionen {
            let n = match self.nativ_meet.as_mut() {
                Some(n) => n,
                None => break,
            };
            match a {
                meetfenster::Aktion::Stumm(v) => n.stumm_schalten(v),
                meetfenster::Aktion::Kamera(v) => n.kamera_schalten(v),
                meetfenster::Aktion::Schirm(v) => {
                    if let Some(m) = n.schirm_schalten_melden(v, 0) {
                        schirm_fehler = Some(m);
                    }
                }
                meetfenster::Aktion::SchirmWaehlen(i) => {
                    if let Some(m) = n.schirm_schalten_melden(true, i) {
                        schirm_fehler = Some(m);
                    }
                }
                meetfenster::Aktion::FensterWaehlen(k) => {
                    if let Some(m) = n.quelle_schalten_melden(
                        true,
                        meetschirm::Quelle::Fenster(k),
                    ) {
                        schirm_fehler = Some(m);
                    }
                }
                meetfenster::Aktion::Rahmen(v) => n.rahmen_schalten(v),
                meetfenster::Aktion::Rauschsperre(an, st) => n.rauschsperre_setzen(an, st),
                meetfenster::Aktion::Avatar(v) => n.avatar_schalten(v),
                meetfenster::Aktion::AvatarAussehen(a) => n.avatar_setzen(a),
                meetfenster::Aktion::Hand(v) => n.hand_heben(v),
                meetfenster::Aktion::Steuerung(v) => n.steuerung_freigeben(v),
                meetfenster::Aktion::SteuerungAnfragen => n.steuerung_anfragen(),
                meetfenster::Aktion::SteuerungAntwort(id, ja) => n.steuerung_antworten(id, ja),
                meetfenster::Aktion::SteuerungZurueck(p) => n.steuerung_zuruecknehmen(p),
                meetfenster::Aktion::Senden(t) => {
                    n.eingabe = t;
                    n.senden();
                }
                meetfenster::Aktion::Tippt(v) => n.tippt(v),
                meetfenster::Aktion::Einlassen(id) => n.einlassen(id),
                meetfenster::Aktion::Abweisen(id) => n.abweisen(id),
                meetfenster::Aktion::AlleEinlassen => n.alle_einlassen(),
                meetfenster::Aktion::Warteraum(v) => n.warteraum(v),
                meetfenster::Aktion::AlleStumm => n.gastgeber_aktion("mute-all", None),
                meetfenster::Aktion::Stummschalten(id) => n.gastgeber_aktion("mute", Some(id)),
                meetfenster::Aktion::Rauswerfen(id) => n.gastgeber_aktion("kick", Some(id)),
                meetfenster::Aktion::Beenden => {
                    n.gastgeber_aktion("end", None);
                    verlassen = true;
                }
                meetfenster::Aktion::Verlassen => verlassen = true,
                meetfenster::Aktion::Steuern(fvid, wer) => steuern_zu = Some((fvid, wer)),
                meetfenster::Aktion::KameraGeraet(i) => {
                    self.meet_win.cam_sel = i;
                    let g = if i == 0 {
                        None
                    } else {
                        self.meet_win.cams.get(i - 1).cloned()
                    };
                    n.kamera_waehlen(g);
                }
                meetfenster::Aktion::MikroGeraet(i) => {
                    self.meet_win.mic_sel = i;
                    let mik = if i == 0 {
                        None
                    } else {
                        self.meet_win.mics.get(i - 1).cloned()
                    };
                    let spk = if self.meet_win.spk_sel == 0 {
                        None
                    } else {
                        self.meet_win.spks.get(self.meet_win.spk_sel - 1).cloned()
                    };
                    n.ton_waehlen(mik, spk);
                }
                meetfenster::Aktion::LautsprecherGeraet(i) => {
                    self.meet_win.spk_sel = i;
                    let spk = if i == 0 {
                        None
                    } else {
                        self.meet_win.spks.get(i - 1).cloned()
                    };
                    let mik = if self.meet_win.mic_sel == 0 {
                        None
                    } else {
                        self.meet_win.mics.get(self.meet_win.mic_sel - 1).cloned()
                    };
                    n.ton_waehlen(mik, spk);
                }
                meetfenster::Aktion::GeraeteNeuLesen => {
                    // Re-reading runs in the background again - the
                    // PnP query takes almost a second on Windows.
                    self.meet_win.geraete_da = false;
                    *self.meet_win.geraete.lock().unwrap() = None;
                    let slot = self.meet_win.geraete.clone();
                    std::thread::spawn(move || {
                        let (mics, mut cams) = meet::geraete();
                        let echte = crate::meetcam::liste();
                        if !echte.is_empty() {
                            cams = echte.into_iter().map(|g| g.name).collect();
                        }
                        let (_, spks) = crate::meetaudio::geraete_liste();
                        *slot.lock().unwrap() = Some((mics, cams, spks));
                    });
                }
                meetfenster::Aktion::EinladungKopieren => {
                    ctx.copy_text(meet::invite(m));
                    self.meet_win.toast = Some((
                        i18n::t("meet.toast_copied").to_string(),
                        std::time::Instant::now(),
                    ));
                }
            }
        }
        if let Some(m) = schirm_fehler {
            self.meet_win.toast = Some((m, std::time::Instant::now()));
        }
        // When sharing starts: small window opens, large window
        // gets out of the way. Otherwise you share your own meeting window -
        // the famous hall-of-mirrors effect - and see nobody
        // yourself anymore. When it ends both are restored, but ONLY if we
        // caused it ourselves.
        //
        // IMPORTANT: the large window is only minimized, not hidden.
        // So that the small window (a child window of the same program)
        // keeps running, drawing explicitly continues during the share
        // - Windows otherwise sends no draw requests to a
        // minimized window.
        let teilt_jetzt = self.nativ_meet.as_ref().map(|n| n.schirm_an).unwrap_or(false);
        if teilt_jetzt != self.meet_teilt_war {
            self.meet_teilt_war = teilt_jetzt;
            if teilt_jetzt {
                if !self.meet_z.pip {
                    self.meet_z.pip = true;
                    self.meet_z.pip_auto = true;
                }
                if !self.headless {
                    ctx.send_viewport_cmd(egui::ViewportCommand::Minimized(true));
                    self.meet_z.minimiert_auto = true;
                }
            } else {
                if self.meet_z.pip_auto {
                    self.meet_z.pip = false;
                    self.meet_z.pip_auto = false;
                }
                if self.meet_z.minimiert_auto {
                    self.meet_z.minimiert_auto = false;
                    if !self.headless {
                        ctx.send_viewport_cmd(egui::ViewportCommand::Minimized(false));
                        ctx.send_viewport_cmd(egui::ViewportCommand::Focus);
                    }
                }
            }
        }
        if teilt_jetzt {
            ctx.request_repaint_after(Duration::from_millis(33));
        }
        if let Some((fvid, wer)) = steuern_zu {
            // From the meeting directly into the session: main window to the
            // front, start page, connect.
            self.view = View::Start;
            self.hint = i18n::tf("link.control", &partners::pretty_id(&fvid));
            self.meet_win.toast = Some((format!("Steuere {}", wer), std::time::Instant::now()));
            self.connect_to(&fvid);
        }
        if verlassen {
            if let Some(n) = self.nativ_meet.take() {
                n.verlassen();
            }
            self.nativ_bilder.clear();
            self.nativ_schirme.clear();
            self.nativ_eigen = None;
            self.meet_win.beigetreten = false;
            self.meet_z.pip = false;
            self.meet_z.pip_auto = false;
            // Whoever hangs up during the share would otherwise sit in front of a
            // minimized window and not know where the program went.
            if self.meet_z.minimiert_auto {
                self.meet_z.minimiert_auto = false;
                if !self.headless {
                    ctx.send_viewport_cmd(egui::ViewportCommand::Minimized(false));
                    ctx.send_viewport_cmd(egui::ViewportCommand::Focus);
                }
            }
            self.meet_teilt_war = false;
            self.meet_kam_start = None;
        }
    }

}
