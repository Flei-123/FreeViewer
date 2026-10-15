//! Split out of main.rs (r35): mechanical move, behaviour unchanged.
use crate::lockutil::LockExt;
use super::*;

impl App {
    /// The running meeting.
    pub(crate) fn meet_lauf_ui(&mut self, ctx: &egui::Context, m: &meet::Meeting) {
        let sicht: meetwindow::Sicht;
        let bilder: meetwindow::Bilder;
        // Whose screen is currently seen large - this determines
        // who we send our zoom request to.
        let mut main_screen_peer: Option<u64> = None;
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
            let z = n.state();
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
            match n.own_screen.as_ref() {
                Some((w, h, px)) => {
                    let frisch = self
                        .native_own_screen
                        .as_ref()
                        .map(|(alt, _)| *alt != n.own_screen_state)
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
                        self.native_own_screen = Some((n.own_screen_state, tex));
                    }
                }
                None => self.native_own_screen = None,
            }

            // ---- Sicht zusammenstellen
            let ich_name = if self.meet_win.name.trim().is_empty() {
                presence::device_name()
            } else {
                self.meet_win.name.trim().to_string()
            };
            let mut leute = vec![meetwindow::Person {
                id: z.ich,
                name: ich_name,
                is_muted: n.is_muted,
                kamera_aus: !n.kamera_an,
                hand: n.hand,
                gastgeber: z.gastgeber != 0 && z.gastgeber == z.ich,
                fvid: String::new(),
                ich: true,
                spricht: !n.is_muted && n.pegel > 0.06,
            }];
            for t in z.leute.iter() {
                leute.push(meetwindow::Person {
                    id: t.id,
                    name: t.name.clone(),
                    is_muted: t.ton_aus,
                    kamera_aus: t.bild_aus,
                    hand: t.hand,
                    gastgeber: z.gastgeber != 0 && z.gastgeber == t.id,
                    fvid: t.fvid.clone(),
                    ich: false,
                    spricht: n.spricht(t.id),
                });
            }
            let chat: Vec<meetwindow::Chatzeile> = n
                .chat
                .iter()
                .map(|(von, text)| meetwindow::Chatzeile {
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
            if self.meet_z.seite_offen && self.meet_z.reiter == meetwindow::Reiter::Chat {
                self.meet_gelesen = chat.len();
            }
            let ungelesen = chat.len().saturating_sub(self.meet_gelesen) as u32;

            let mut protokoll = n.protokoll.clone();
            for notice in [&n.notice, &n.camera_notice, &n.screen_notice] {
                if !notice.is_empty() {
                    protokoll.push(notice.clone());
                }
            }
            protokoll.push(format!(
                "Ton: {} raus / {} rein · Bild: {} raus / {} rein",
                zahlen.gesendet, zahlen.receive, zahlen.bild_gesendet, zahlen.frame_received
            ));
            if n.screen_on {
                protokoll.push(format!(
                    "Bildschirm geteilt: {} Bilder gesendet",
                    n.screen_sent
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
            let warte_text = if z.im_warteraum { n.notice.clone() } else { String::new() };
            let mut schirme: Vec<(u64, String)> = self
                .nativ_schirme
                .keys()
                .map(|id| (*id, n.name_von(*id)))
                .collect();
            // The OWN share belongs on the stage too - like in the
            // browser. Otherwise you click "Screen" and see nothing.
            if n.screen_on && self.native_own_screen.is_some() {
                schirme.push((z.ich, "Du".to_string()));
            }

            let ton_namen = n.ton_namen();
            // Whose screen is the main content? The first foreign one.
            let screen_main: Option<u64> = schirme
                .iter()
                .map(|(id, _)| *id)
                .find(|id| *id != z.ich)
                .or_else(|| schirme.first().map(|(id, _)| *id));
            main_screen_peer = screen_main;
            sicht = meetwindow::Sicht {
                raum: if z.raum.is_empty() { m.id.clone() } else { z.raum.clone() },
                titel: if z.titel.is_empty() { m.titel.clone() } else { z.titel.clone() },
                gastgeber: n.bin_gastgeber(),
                connection: if zahlen.verbunden {
                    "direkt verbunden".to_string()
                } else if z.verbunden {
                    "Medien verbinden …".to_string()
                } else {
                    "verbinde …".to_string()
                },
                connect_tone: if zahlen.verbunden {
                    meetwindow::Ton::Gut
                } else {
                    meetwindow::Ton::Warn
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
                is_muted: n.is_muted,
                kamera_an: n.kamera_an,
                screen_on: n.screen_on,
                hand: n.hand,
                steuer_frei: n.steuer_frei,
                steuer_anfragen: n.steuer_anfragen(),
                steuer_erlaubt: n.steuer_erlaubt(),
                steuer_gefragt: n.steuer_gefragt,
                steuer_zusage: n.steuer_zusage(),
                foreign_screen: n.fremder_teilt(),
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
                teiler_zeiger: match screen_main {
                    Some(p) if p == z.ich => n.eigener_zeiger(),
                    Some(p) => n.zeiger_des_teilers(p),
                    None => None,
                },
                // Same for the crop: the own announcement only goes
                // to the others. Whoever looks at the own share must still know the
                // crop - otherwise it is zoomed TWICE and
                // the image turns black from about 140 %.
                screen_region: match screen_main {
                    Some(p) if p == z.ich => Some(n.screen_region),
                    Some(p) => n.bereich_des_teilers(p),
                    None => None,
                },
                scharf_moeglich: match screen_main {
                    Some(p) if p == z.ich => true,
                    Some(p) => n.kann_scharf(p),
                    None => false,
                },
                monitore: self.meet_win.monitore.clone(),
                window: self.meet_win.window.clone(),
                rahmen_an: n.rahmen_an,
                avatar_an: n.avatar_an,
                avatar: n.avatar,
                sperre_an: n.sperre_an,
                sperre_staerke: n.sperre_staerke,
                sperre_offen: n.sperre_offen(),
                sperre_grund: n.sperre_grundrauschen(),
                eigener_pegel: n.pegel,
            };
            bilder = meetwindow::Bilder {
                eigen: self.nativ_eigen.as_ref().map(|(_, t)| t.clone()),
                kameras: self
                    .nativ_bilder
                    .iter()
                    .map(|(k, (_, t))| (*k, t.clone()))
                    .collect(),
                schirme: self
                    .native_own_screen
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
            if meetwindow::warteschirm_ui(ctx, &sicht.titel, &sicht.warte_text) {
                if let Some(n) = self.nativ_meet.take() {
                    n.verlassen();
                }
                self.meet_win.beigetreten = false;
                self.meet_kam_start = None;
            }
            return;
        }
        let mut aktionen = meetwindow::meeting_ui(ctx, &sicht, &mut self.meet_z, &bilder);
        // Picture-in-picture: own small window that stays on top - otherwise
        // you see nobody anymore when sharing your own screen.
        if self.meet_z.pip && !self.headless {
            let mut zu = false;
            // The switch lives in the window state, but the close is in the
            // viewport callback - hence a copy here that is written back
            // afterwards (otherwise `self` would be borrowed twice).
            let mut selbst = self.meet_z.pip_selbst;
            let mut pip_aktionen: Vec<meetwindow::Aktion> = Vec::new();
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
                                .extend(meetwindow::pip_inhalt(ui, &sicht, &bilder, &mut selbst));
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
        if let (Some(p), Some(n)) = (main_screen_peer, self.nativ_meet.as_mut()) {
            let f = self.meet_z.zoom_window;
            let zoomt = self.meet_z.zoom > 1.001;
            n.zoom_wunsch(p, f, zoomt && self.meet_z.scharf);
        }

        let mut steuern_zu: Option<(String, String)> = None;
        let mut verlassen = false;
        // If sharing fails, it MUST become visible. Previously it was
        // only in the info tab - for the user it looked as if nothing
        // happened at all.
        let mut screen_error: Option<String> = None;
        for a in aktionen {
            let n = match self.nativ_meet.as_mut() {
                Some(n) => n,
                None => break,
            };
            match a {
                meetwindow::Aktion::Muted(v) => n.toggle_muted(v),
                meetwindow::Aktion::Kamera(v) => n.kamera_schalten(v),
                meetwindow::Aktion::Screen(v) => {
                    if let Some(m) = n.report_toggle_screen(v, 0) {
                        screen_error = Some(m);
                    }
                }
                meetwindow::Aktion::PickScreen(i) => {
                    if let Some(m) = n.report_toggle_screen(true, i) {
                        screen_error = Some(m);
                    }
                }
                meetwindow::Aktion::PickWindow(k) => {
                    if let Some(m) = n.quelle_schalten_melden(
                        true,
                        meetschirm::Quelle::Window(k),
                    ) {
                        screen_error = Some(m);
                    }
                }
                meetwindow::Aktion::Rahmen(v) => n.rahmen_schalten(v),
                meetwindow::Aktion::Rauschsperre(an, st) => n.rauschsperre_setzen(an, st),
                meetwindow::Aktion::Avatar(v) => n.avatar_schalten(v),
                meetwindow::Aktion::AvatarAussehen(a) => n.avatar_setzen(a),
                meetwindow::Aktion::Hand(v) => n.hand_heben(v),
                meetwindow::Aktion::Control(v) => n.release_control(v),
                meetwindow::Aktion::RequestControl => n.request_control(),
                meetwindow::Aktion::ControlReply(id, ja) => n.reply_control(id, ja),
                meetwindow::Aktion::ControlGiveBack(p) => n.revoke_control(p),
                meetwindow::Aktion::Send(t) => {
                    n.input_text = t;
                    n.send_out();
                }
                meetwindow::Aktion::Tippt(v) => n.tippt(v),
                meetwindow::Aktion::Einlassen(id) => n.einlassen(id),
                meetwindow::Aktion::Abweisen(id) => n.abweisen(id),
                meetwindow::Aktion::AlleEinlassen => n.alle_einlassen(),
                meetwindow::Aktion::Warteraum(v) => n.warteraum(v),
                meetwindow::Aktion::MuteAll => n.gastgeber_aktion("mute-all", None),
                meetwindow::Aktion::Stummschalten(id) => n.gastgeber_aktion("mute", Some(id)),
                meetwindow::Aktion::Rauswerfen(id) => n.gastgeber_aktion("kick", Some(id)),
                meetwindow::Aktion::Leave => {
                    n.gastgeber_aktion("end", None);
                    verlassen = true;
                }
                meetwindow::Aktion::Verlassen => verlassen = true,
                meetwindow::Aktion::Steuern(fvid, wer) => steuern_zu = Some((fvid, wer)),
                meetwindow::Aktion::KameraGeraet(i) => {
                    self.meet_win.cam_sel = i;
                    let g = if i == 0 {
                        None
                    } else {
                        self.meet_win.cams.get(i - 1).cloned()
                    };
                    n.kamera_waehlen(g);
                }
                meetwindow::Aktion::MikroGeraet(i) => {
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
                meetwindow::Aktion::LautsprecherGeraet(i) => {
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
                meetwindow::Aktion::ReloadDevices => {
                    // Re-reading runs in the background again - the
                    // PnP query takes almost a second on Windows.
                    self.meet_win.geraete_da = false;
                    *self.meet_win.geraete.lock_ok() = None;
                    let slot = self.meet_win.geraete.clone();
                    std::thread::spawn(move || {
                        let (mics, mut cams) = meet::geraete();
                        let echte = crate::meetcam::liste();
                        if !echte.is_empty() {
                            cams = echte.into_iter().map(|g| g.name).collect();
                        }
                        let (_, spks) = crate::meetaudio::geraete_liste();
                        *slot.lock_ok() = Some((mics, cams, spks));
                    });
                }
                meetwindow::Aktion::EinladungKopieren => {
                    ctx.copy_text(meet::invite(m));
                    self.meet_win.toast = Some((
                        i18n::t("meet.toast_copied").to_string(),
                        std::time::Instant::now(),
                    ));
                }
            }
        }
        if let Some(m) = screen_error {
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
        let teilt_jetzt = self.nativ_meet.as_ref().map(|n| n.screen_on).unwrap_or(false);
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
