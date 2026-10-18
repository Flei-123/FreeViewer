//! Split out of main.rs (r35): mechanical move, behaviour unchanged.
use crate::lockutil::LockExt;
use super::*;

impl App {
    /// The running meeting.
    pub(crate) fn meet_run_ui(&mut self, ctx: &egui::Context, m: &meet::Meeting) {
        let view_state: meetwindow::MeetView;
        let images: meetwindow::Images;
        // Whose screen is currently seen large - this determines
        // who we send our zoom request to.
        let mut main_screen_peer: Option<u64> = None;
        {
            let n = match self.native_meet.as_mut() {
                Some(n) => n,
                None => return,
            };
            n.pump();
            // Start the camera only now - the preview has just
            // released it and needs a moment for that.
            if let Some(wann) = self.meet_kam_start {
                if std::time::Instant::now() >= wann {
                    self.meet_kam_start = None;
                    n.camera_toggle(true);
                }
            }
            let z = n.state();
            // Only when taking images (--meetshot): let waiting people in right away.
            // Otherwise you would never see a second participant in the proof image.
            if self.meet_shot_join && !z.waiting.is_empty() {
                n.all_admit();
            }
            let numbers = n.numbers();

            // ---- Reload textures (only for a really new image)
            match n.own.as_ref() {
                Some((w, h, px)) => {
                    let fresh = self
                        .native_own
                        .as_ref()
                        .map(|(alt, _)| *alt != n.own_state)
                        .unwrap_or(true);
                    if fresh {
                        let picture = egui::ColorImage::from_rgba_unmultiplied(
                            [*w as usize, *h as usize],
                            px,
                        );
                        let tex = ctx.load_texture("meeteigen", picture, egui::TextureOptions::LINEAR);
                        self.native_own = Some((n.own_state, tex));
                    }
                }
                None => self.native_own = None,
            }
            for (peer, (w, h, px)) in n.images.images.iter() {
                let stand = *n.images.stand.get(peer).unwrap_or(&0);
                let fresh = self
                    .native_images
                    .get(peer)
                    .map(|(alt, _)| *alt != stand)
                    .unwrap_or(true);
                if fresh {
                    let picture =
                        egui::ColorImage::from_rgba_unmultiplied([*w as usize, *h as usize], px);
                    let tex =
                        ctx.load_texture(format!("meetcam{}", peer), picture, egui::TextureOptions::LINEAR);
                    self.native_images.insert(*peer, (stand, tex));
                }
            }
            self.native_images
                .retain(|k, _| n.images.images.contains_key(k));
            for (peer, (w, h, px)) in n.screens.images.iter() {
                let stand = *n.screens.stand.get(peer).unwrap_or(&0);
                let fresh = self
                    .native_screens
                    .get(peer)
                    .map(|(alt, _)| *alt != stand)
                    .unwrap_or(true);
                if fresh {
                    let picture =
                        egui::ColorImage::from_rgba_unmultiplied([*w as usize, *h as usize], px);
                    let tex = ctx.load_texture(
                        format!("meetschirm{}", peer),
                        picture,
                        egui::TextureOptions::LINEAR,
                    );
                    self.native_screens.insert(*peer, (stand, tex));
                }
            }
            self.native_screens
                .retain(|k, _| n.screens.images.contains_key(k));
            // The own shared image. Without this tile you see nothing
            // at all when sharing - the button then seems to have no effect.
            match n.own_screen.as_ref() {
                Some((w, h, px)) => {
                    let fresh = self
                        .native_own_screen
                        .as_ref()
                        .map(|(alt, _)| *alt != n.own_screen_state)
                        .unwrap_or(true);
                    if fresh {
                        let picture = egui::ColorImage::from_rgba_unmultiplied(
                            [*w as usize, *h as usize],
                            px,
                        );
                        let tex = ctx.load_texture(
                            "meeteigenschirm",
                            picture,
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
            let mut people = vec![meetwindow::Person {
                id: z.ich,
                name: ich_name,
                is_muted: n.is_muted,
                camera_from: !n.camera_on,
                hand: n.hand,
                is_host: z.is_host != 0 && z.is_host == z.ich,
                fvid: String::new(),
                ich: true,
                speaking: !n.is_muted && n.level > 0.06,
            }];
            for t in z.people.iter() {
                people.push(meetwindow::Person {
                    id: t.id,
                    name: t.name.clone(),
                    is_muted: t.ton_aus,
                    camera_from: t.image_from,
                    hand: t.hand,
                    is_host: z.is_host != 0 && z.is_host == t.id,
                    fvid: t.fvid.clone(),
                    ich: false,
                    speaking: n.speaking(t.id),
                });
            }
            let chat: Vec<meetwindow::ChatLine> = n
                .chat
                .iter()
                .map(|(von, text)| meetwindow::ChatLine {
                    von: *von,
                    name: if *von == 0 {
                        String::new()
                    } else {
                        n.name_von(*von)
                    },
                    text: text.clone(),
                    own: *von == z.ich,
                })
                .collect();
            // Chat counts as read as long as the sidebar shows it.
            if self.meet_z.page_open && self.meet_z.tab == meetwindow::Tab::Chat {
                self.meet_read = chat.len();
            }
            let unread = chat.len().saturating_sub(self.meet_read) as u32;

            let mut protocol = n.protocol.clone();
            for notice in [&n.notice, &n.camera_notice, &n.screen_notice] {
                if !notice.is_empty() {
                    protocol.push(notice.clone());
                }
            }
            protocol.push(format!(
                "Ton: {} raus / {} rein · Bild: {} raus / {} rein",
                numbers.sent, numbers.receive, numbers.image_sent, numbers.frame_received
            ));
            if n.screen_on {
                protocol.push(format!(
                    "Bildschirm geteilt: {} Bilder gesendet",
                    n.screen_sent
                ));
            }
            protocol.push(format!("Meine Nummer: {}", n.my_fvid()));

            let bandwidth = if n.kbit_rein + n.kbit_out > 0 {
                format!("{} kbit/s", n.kbit_rein + n.kbit_out)
            } else {
                String::new()
            };
            let type_text: Vec<String> = n.type_text.iter().map(|id| n.name_von(*id)).collect();
            // The server text about the waiting room is in the last message.
            let warte_text = if z.in_waiting_room { n.notice.clone() } else { String::new() };
            let mut screens: Vec<(u64, String)> = self
                .native_screens
                .keys()
                .map(|id| (*id, n.name_von(*id)))
                .collect();
            // The OWN share belongs on the stage too - like in the
            // browser. Otherwise you click "Screen" and see nothing.
            if n.screen_on && self.native_own_screen.is_some() {
                screens.push((z.ich, "Du".to_string()));
            }

            let ton_namen = n.ton_namen();
            // Whose screen is the main content? The first foreign one.
            let screen_main: Option<u64> = screens
                .iter()
                .map(|(id, _)| *id)
                .find(|id| *id != z.ich)
                .or_else(|| screens.first().map(|(id, _)| *id));
            main_screen_peer = screen_main;
            view_state = meetwindow::MeetView {
                room: if z.room.is_empty() { m.id.clone() } else { z.room.clone() },
                heading: if z.heading.is_empty() { m.heading.clone() } else { z.heading.clone() },
                is_host: n.am_host(),
                connection: if numbers.connected {
                    "direkt verbunden".to_string()
                } else if z.connected {
                    "Medien verbinden …".to_string()
                } else {
                    "verbinde …".to_string()
                },
                connect_tone: if numbers.connected {
                    meetwindow::Ton::Gut
                } else {
                    meetwindow::Ton::Warn
                },
                // Green only if encryption is REALLY happening - i.e. a
                // key exists. A green mark without encryption
                // would be the worst kind of lie.
                e2e: n.e2e_an,
                bandwidth,
                people,
                waiting: z.waiting.clone(),
                waiting_room_on: z.waiting_room_on,
                chat,
                protocol,
                is_muted: n.is_muted,
                camera_on: n.camera_on,
                screen_on: n.screen_on,
                hand: n.hand,
                control_free: n.control_free,
                control_request: n.control_request(),
                control_allowed: n.control_allowed(),
                control_asked: n.control_asked,
                control_grant: n.control_grant(),
                foreign_screen: n.foreign_shares(),
                screens,
                unread,
                type_text,
                in_waiting_room: z.in_waiting_room,
                warte_text: warte_text.clone(),
                cams: self.meet_win.cams.clone(),
                mics: self.meet_win.mics.clone(),
                spks: self.meet_win.spks.clone(),
                cam_sel: self.meet_win.cam_sel,
                mic_sel: self.meet_win.mic_sel,
                spk_sel: self.meet_win.spk_sel,
                camera_name: n.camera_name(),
                ton_ein: ton_namen.0,
                ton_aus: ton_namen.1,
                // Pointer OF THE ONE whose screen is currently seen large.
                // If it is the OWN one, nothing comes back over the network -
                // then look it up directly.
                divider_pointer: match screen_main {
                    Some(p) if p == z.ich => n.own_pointer(),
                    Some(p) => n.pointer_the_divider(p),
                    None => None,
                },
                // Same for the crop: the own announcement only goes
                // to the others. Whoever looks at the own share must still know the
                // crop - otherwise it is zoomed TWICE and
                // the image turns black from about 140 %.
                screen_region: match screen_main {
                    Some(p) if p == z.ich => Some(n.screen_region),
                    Some(p) => n.region_the_divider(p),
                    None => None,
                },
                sharp_possible: match screen_main {
                    Some(p) if p == z.ich => true,
                    Some(p) => n.can_sharp(p),
                    None => false,
                },
                monitore: self.meet_win.monitore.clone(),
                window: self.meet_win.window.clone(),
                frame_on: n.frame_on,
                avatar_an: n.avatar_an,
                avatar: n.avatar,
                lock_on: n.lock_on,
                lock_strength: n.lock_strength,
                lock_open: n.lock_open(),
                lock_reason: n.lock_noise_floor(),
                own_level: n.level,
            };
            images = meetwindow::Images {
                own: self.native_own.as_ref().map(|(_, t)| t.clone()),
                cameras: self
                    .native_images
                    .iter()
                    .map(|(k, (_, t))| (*k, t.clone()))
                    .collect(),
                screens: self
                    .native_own_screen
                    .as_ref()
                    .map(|(_, t)| (z.ich, t.clone()))
                    .into_iter()
                    .chain(
                        self.native_screens
                            .iter()
                            .map(|(k, (_, t))| (*k, t.clone())),
                    )
                    .collect(),
            };
        }

        // At the door: the waiting screen instead of an empty room. Without it
        // the guest would sit in front of a black area and think the program
        // is broken - exactly the case noticed during the rehearsal.
        if view_state.in_waiting_room {
            if meetwindow::waiting_screen_ui(ctx, &view_state.heading, &view_state.warte_text) {
                if let Some(n) = self.native_meet.take() {
                    n.leave();
                }
                self.meet_win.joined = false;
                self.meet_kam_start = None;
            }
            return;
        }
        let mut actions = meetwindow::meeting_ui(ctx, &view_state, &mut self.meet_z, &images);
        // Picture-in-picture: own small window that stays on top - otherwise
        // you see nobody anymore when sharing your own screen.
        if self.meet_z.pip && !self.headless {
            let mut zu = false;
            // The switch lives in the window state, but the close is in the
            // viewport callback - hence a copy here that is written back
            // afterwards (otherwise `self` would be borrowed twice).
            let mut myself = self.meet_z.pip_self;
            let mut pip_actions: Vec<meetwindow::Action> = Vec::new();
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
                            pip_actions
                                .extend(meetwindow::pip_content(ui, &view_state, &images, &mut myself));
                        });
                    if pctx.input(|i| i.viewport().close_requested()) {
                        zu = true;
                    }
                    // The small window shows the same images - it must
                    // not be the slower of the two.
                    pctx.request_repaint_after(Duration::from_millis(33));
                },
            );
            self.meet_z.pip_self = myself;
            actions.extend(pip_actions);
            if zu {
                self.meet_z.pip = false;
            }
        }

        // Tell the sender which crop we are looking at. It then cuts
        // exactly that from its native capture - same bandwidth, but
        // real pixels. Without this message it would stay with upscaling.
        if let (Some(p), Some(n)) = (main_screen_peer, self.native_meet.as_mut()) {
            let f = self.meet_z.zoom_window;
            let zooms = self.meet_z.zoom > 1.001;
            n.zoom_wish(p, f, zooms && self.meet_z.sharp);
        }

        let mut control_to: Option<(String, String)> = None;
        let mut leave = false;
        // If sharing fails, it MUST become visible. Previously it was
        // only in the info tab - for the user it looked as if nothing
        // happened at all.
        let mut screen_error: Option<String> = None;
        for a in actions {
            let n = match self.native_meet.as_mut() {
                Some(n) => n,
                None => break,
            };
            match a {
                meetwindow::Action::Muted(v) => n.toggle_muted(v),
                meetwindow::Action::WebCam(v) => n.camera_toggle(v),
                meetwindow::Action::Screen(v) => {
                    if let Some(m) = n.report_toggle_screen(v, 0) {
                        screen_error = Some(m);
                    }
                }
                meetwindow::Action::PickScreen(i) => {
                    if let Some(m) = n.report_toggle_screen(true, i) {
                        screen_error = Some(m);
                    }
                }
                meetwindow::Action::PickWindow(k) => {
                    if let Some(m) = n.source_toggle_report(
                        true,
                        meetschirm::Source::Window(k),
                    ) {
                        screen_error = Some(m);
                    }
                }
                meetwindow::Action::FrameBox(v) => n.frame_toggle(v),
                meetwindow::Action::NoiseGate(an, st) => n.noise_gate_set(an, st),
                meetwindow::Action::Avatar(v) => n.avatar_toggle(v),
                meetwindow::Action::AvatarLook(a) => n.avatar_set(a),
                meetwindow::Action::Hand(v) => n.hand_heben(v),
                meetwindow::Action::Control(v) => n.release_control(v),
                meetwindow::Action::RequestControl => n.request_control(),
                meetwindow::Action::ControlReply(id, ja) => n.reply_control(id, ja),
                meetwindow::Action::ControlGiveBack(p) => n.revoke_control(p),
                meetwindow::Action::Send(t) => {
                    n.input_text = t;
                    n.send_out();
                }
                meetwindow::Action::Types(v) => n.types(v),
                meetwindow::Action::Admit(id) => n.admit(id),
                meetwindow::Action::Refuse(id) => n.refuse(id),
                meetwindow::Action::AllAdmit => n.all_admit(),
                meetwindow::Action::WaitingRoom(v) => n.waiting_room(v),
                meetwindow::Action::MuteAll => n.host_action("mute-all", None),
                meetwindow::Action::Mute(id) => n.host_action("mute", Some(id)),
                meetwindow::Action::Kick(id) => n.host_action("kick", Some(id)),
                meetwindow::Action::Leave => {
                    n.host_action("end", None);
                    leave = true;
                }
                meetwindow::Action::LeaveMeet => leave = true,
                meetwindow::Action::AskControl(fvid, wer) => control_to = Some((fvid, wer)),
                meetwindow::Action::CameraDevice(i) => {
                    self.meet_win.cam_sel = i;
                    let g = if i == 0 {
                        None
                    } else {
                        self.meet_win.cams.get(i - 1).cloned()
                    };
                    n.camera_pick(g);
                }
                meetwindow::Action::MicDevice(i) => {
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
                    n.tone_pick(mik, spk);
                }
                meetwindow::Action::SpeakerDevice(i) => {
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
                    n.tone_pick(mik, spk);
                }
                meetwindow::Action::ReloadDevices => {
                    // Re-reading runs in the background again - the
                    // PnP query takes almost a second on Windows.
                    self.meet_win.devices_there = false;
                    *self.meet_win.devices.lock_ok() = None;
                    let slot = self.meet_win.devices.clone();
                    std::thread::spawn(move || {
                        let (mics, mut cams) = meet::devices();
                        let real = crate::meetcam::list_items();
                        if !real.is_empty() {
                            cams = real.into_iter().map(|g| g.name).collect();
                        }
                        let (_, spks) = crate::meetaudio::devices_list();
                        *slot.lock_ok() = Some((mics, cams, spks));
                    });
                }
                meetwindow::Action::InviteCopy => {
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
        let shares_now = self.native_meet.as_ref().map(|n| n.screen_on).unwrap_or(false);
        if shares_now != self.meet_shares_was {
            self.meet_shares_was = shares_now;
            if shares_now {
                if !self.meet_z.pip {
                    self.meet_z.pip = true;
                    self.meet_z.pip_auto = true;
                }
                if !self.headless {
                    ctx.send_viewport_cmd(egui::ViewportCommand::Minimized(true));
                    self.meet_z.minimized_auto = true;
                }
            } else {
                if self.meet_z.pip_auto {
                    self.meet_z.pip = false;
                    self.meet_z.pip_auto = false;
                }
                if self.meet_z.minimized_auto {
                    self.meet_z.minimized_auto = false;
                    if !self.headless {
                        ctx.send_viewport_cmd(egui::ViewportCommand::Minimized(false));
                        ctx.send_viewport_cmd(egui::ViewportCommand::Focus);
                    }
                }
            }
        }
        if shares_now {
            ctx.request_repaint_after(Duration::from_millis(33));
        }
        if let Some((fvid, wer)) = control_to {
            // From the meeting directly into the session: main window to the
            // front, start page, connect.
            self.view = View::Start;
            self.hint = i18n::tf("link.control", &partners::pretty_id(&fvid));
            self.meet_win.toast = Some((format!("Steuere {}", wer), std::time::Instant::now()));
            self.connect_to(&fvid);
        }
        if leave {
            if let Some(n) = self.native_meet.take() {
                n.leave();
            }
            self.native_images.clear();
            self.native_screens.clear();
            self.native_own = None;
            self.meet_win.joined = false;
            self.meet_z.pip = false;
            self.meet_z.pip_auto = false;
            // Whoever hangs up during the share would otherwise sit in front of a
            // minimized window and not know where the program went.
            if self.meet_z.minimized_auto {
                self.meet_z.minimized_auto = false;
                if !self.headless {
                    ctx.send_viewport_cmd(egui::ViewportCommand::Minimized(false));
                    ctx.send_viewport_cmd(egui::ViewportCommand::Focus);
                }
            }
            self.meet_shares_was = false;
            self.meet_kam_start = None;
        }
    }

}
