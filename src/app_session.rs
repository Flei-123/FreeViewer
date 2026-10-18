//! Split out of main.rs (r35): mechanical move, behaviour unchanged.
use crate::lockutil::LockExt;
use super::*;

impl App {
    pub(crate) fn knock_ui(&mut self, ctx: &egui::Context) {
        let knock = self.shared.knock.lock_ok().clone();
        let Some(k) = knock else {
            return;
        };
        let left = 60_u64.saturating_sub(k.at.elapsed().as_secs());
        egui::Window::new("Verbindungsanfrage")
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
                ui.set_min_width(340.0);
                ui.label(
                    egui::RichText::new(format!("„{}“ möchte sich verbinden", k.from))
                        .size(16.0)
                        .strong(),
                );
                if k.unverified {
                    ui.label(
                        egui::RichText::new(i18n::t(if k.friend_like {
                            "knock.friend_like"
                        } else {
                            "knock.unverified"
                        }))
                        .color(egui::Color32::from_rgb(230, 160, 40))
                        .size(12.0),
                    );
                }
                ui.add_space(6.0);
                ui.horizontal(|ui| {
                    label_small(ui, "Sitzungscode");
                    ui.label(
                        egui::RichText::new(&k.code)
                            .monospace()
                            .color(theme::accent())
                            .size(22.0)
                            .strong(),
                    );
                });
                ui.label(
                    egui::RichText::new(
                        "Lass dir den Code nennen – stimmt er überein, redet ihr wirklich miteinander.",
                    )
                    .color(theme::muted())
                    .size(11.5),
                );
                ui.add_space(6.0);
                ui.horizontal(|ui| {
                    if accent_button(ui, "Zulassen", true).clicked() {
                        self.shared.knock_answer.store(1, Ordering::Relaxed);
                    }
                    if ui.add(ghost(egui::vec2(110.0, 34.0), "Ablehnen")).clicked() {
                        self.shared.knock_answer.store(2, Ordering::Relaxed);
                    }
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        ui.label(
                            egui::RichText::new(format!("noch {} s", left))
                                .color(theme::muted())
                                .size(11.5),
                        );
                    });
                });
            });
        ctx.request_repaint_after(Duration::from_millis(250));
    }

    /// What was clicked in the control bar. The bar exists twice
    /// (on top in the window and floating in fullscreen), but the content only
    /// once - so it collects its wishes here.
    pub(crate) fn session_bar(&mut self, ui: &mut egui::Ui, a: &mut BarActs, compact: bool) {
        let stats = *self.shared.stats.lock_ok();
        let game = self.shared.game_mode();
        let p = theme::palette();

        if compact {
            // Griff zum Verschieben
            let (rect, r) = ui.allocate_exact_size(egui::vec2(16.0, 22.0), egui::Sense::drag());
            ui.put(rect, icons::image("grip", 14.0, p.muted));
            if r.dragged() {
                a.drag += r.drag_delta().x;
            }
            let r = r.on_hover_text(i18n::t("sess.move_bar"));
            if r.hovered() || r.dragged() {
                ui.ctx().set_cursor_icon(egui::CursorIcon::Grab);
            }
            ui.add_space(2.0);
        }

        if index_finger(ui.button(i18n::t("sess.disconnect"))).clicked() {
            a.disconnect = true;
        }
        ui.separator();

        if !compact {
            ui.label(i18n::t("sess.mode"));
        }
        if index_finger(
            ui.selectable_label(!game, i18n::t("sess.mode_admin"))
                .on_hover_text(i18n::t("sess.mode_admin_tip")),
        )
        .clicked()
            && game
        {
            a.mode = Some(proto::MODE_ADMIN);
        }
        if index_finger(
            ui.selectable_label(game, i18n::t("sess.mode_game"))
                .on_hover_text(i18n::t("sess.mode_game_tip")),
        )
        .clicked()
            && !game
        {
            a.mode = Some(proto::MODE_GAME);
        }

        let mons = self.shared.monitors.lock_ok().clone();
        if mons.len() > 1 {
            ui.separator();
            let act = self.shared.active_monitor.load(Ordering::Relaxed) as usize;
            let label = |i: usize, m: &proto::MonitorInfo| {
                if compact {
                    format!("{}. {}", i + 1, m.name)
                } else {
                    format!("{}. {} ({}x{})", i + 1, m.name, m.w, m.h)
                }
            };
            let cur = mons
                .get(act)
                .map(|m| label(act, m))
                .unwrap_or_else(|| i18n::t("sess.screen").to_string());
            egui::ComboBox::from_id_salt("monitor_pick")
                .selected_text(cur)
                .show_ui(ui, |ui| {
                    for (i, m) in mons.iter().enumerate() {
                        if ui.selectable_label(i == act, label(i, m)).clicked() {
                            a.monitor = Some(i as u8);
                        }
                    }
                });
        }

        // Resolution of the remote screen - the host switches and restores
        // the previous one at the end of the session.
        ui.separator();
        {
            let (rw, rh) = *self.shared.remote_size.lock_ok();
            // What the remote screen can really do (the host says); as long as
            // there is nothing, the common steps.
            let modi = {
                let l = self.shared.remote_resolutions.lock_ok().clone();
                if l.is_empty() {
                    vec![
                        (1280u32, 720u32),
                        (1600, 900),
                        (1920, 1080),
                        (2560, 1440),
                        (3440, 1440),
                        (3840, 2160),
                    ]
                } else {
                    l
                }
            };
            egui::ComboBox::from_id_salt("res_pick")
                .selected_text(format!("{}x{}", rw, rh))
                .show_ui(ui, |ui| {
                    for (w, h) in modi {
                        if ui
                            .selectable_label(w == rw && h == rh, format!("{}x{}", w, h))
                            .clicked()
                        {
                            a.res = Some((w, h));
                        }
                    }
                });
        }

        ui.separator();
        self.voice_buttons(ui, 16.0);
        ui.separator();

        let mut want_pick = false;
        let mut want_open = false;
        ui.menu_button(i18n::t("sess.files"), |ui| {
            if ui.button(i18n::t("sess.send_file")).clicked() {
                want_pick = true;
                ui.close();
            }
            if ui.button(i18n::t("sess.open_dir")).clicked() {
                want_open = true;
                ui.close();
            }
            ui.label(
                egui::RichText::new(i18n::t("sess.drop_tip"))
                    .weak()
                    .size(11.0),
            );
        });
        if want_pick {
            a.pick = true;
        }
        if want_open {
            a.open_dir = true;
        }

        ui.separator();
        ui.menu_button(i18n::t("sess.keys"), |ui| {
            for (text, code) in [
                ("Strg+Alt+Entf", proto::SPECIAL_CAD),
                ("Task-Manager (Strg+Shift+Esc)", proto::SPECIAL_TASKMGR),
                ("Windows-Taste", proto::SPECIAL_WIN),
                ("Alt+Tab", proto::SPECIAL_ALTTAB),
                ("Sperren (Win+L)", proto::SPECIAL_LOCK),
            ] {
                if ui.button(text).clicked() {
                    a.special = Some(code);
                    ui.close();
                }
            }
        });

        ui.separator();
        // Fullscreen on/off - in fullscreen additionally the pinning
        let (icon, tip) = if self.full {
            ("shrink", i18n::t("sess.full_off"))
        } else {
            ("expand", i18n::t("sess.full_on"))
        };
        if icon_ghost(ui, icon, tip).clicked() {
            a.toggle_full = true;
        }
        if self.full {
            let pinned = self.bar_pinned;
            if icon_toggle(ui, "pin", pinned, 15.0, i18n::t("sess.pin")).clicked() {
                a.toggle_pin = true;
            }
        }

        ui.separator();
        let (rw, rh) = *self.shared.remote_size.lock_ok();
        let direct = self.shared.direct.load(Ordering::Relaxed);
        let text = if compact {
            format!("{:.0} fps  {:.0} ms", stats.fps, stats.latency_ms)
        } else {
            format!(
                "{}x{}   {:.0} fps   {:.0} kbit/s   {:.0} ms   {}",
                rw,
                rh,
                stats.fps,
                stats.kbps,
                stats.latency_ms,
                if direct {
                    i18n::t("sess.direct")
                } else {
                    i18n::t("sess.via_relay")
                }
            )
        };
        ui.label(text).on_hover_text(format!(
            "{}x{}, {:.0} kbit/s, {}",
            rw,
            rh,
            stats.kbps,
            if direct {
                i18n::t("sess.direct")
            } else {
                i18n::t("sess.via_relay")
            }
        ));

        if !compact {
            ui.separator();
            ui.label(egui::RichText::new(i18n::t("sess.escape")).weak().size(11.0))
                .on_hover_text(i18n::t("sess.escape_tip"));
        }
        if game {
            ui.separator();
            if vinput::is_active() {
                ui.colored_label(theme::green(), i18n::t("sess.grabbed"));
            } else {
                ui.colored_label(
                    egui::Color32::from_rgb(230, 190, 90),
                    i18n::t("sess.not_grabbed"),
                );
            }
        }
    }

    /// Switch fullscreen on and off. In fullscreen the Windows frame
    /// disappears completely - just like in the Windows remote connection.
    pub(crate) fn set_full(&mut self, ctx: &egui::Context, on: bool) {
        if self.full == on {
            return;
        }
        self.full = on;
        self.bar_seen = std::time::Instant::now();
        ctx.send_viewport_cmd(egui::ViewportCommand::Fullscreen(on));
    }

    pub(crate) fn session_ui(&mut self, ctx: &egui::Context) {
        let mut a = BarActs::default();

        // F11 toggles fullscreen - the key is consumed here so that
        // it does not also land on the other machine.
        if ctx.input_mut(|i| i.consume_key(egui::Modifiers::NONE, egui::Key::F11)) {
            a.toggle_full = true;
        }

        if self.full {
            // ---- schwebende Leiste (Windows-Fernverbindung laesst gruessen)
            let screen = ctx.screen_rect();
            // Like in the Windows remote connection: the bar comes back
            // as soon as the mouse touches the top edge of the screen.
            let pointer_top = ctx
                .input(|i| i.pointer.hover_pos())
                .map(|p| p.y <= screen.top() + 3.0)
                .unwrap_or(false);
            if pointer_top {
                self.bar_seen = std::time::Instant::now();
            }
            let offen = self.bar_pinned
                || self.bar_seen.elapsed() < Duration::from_millis(2500);
            let width_px = self.bar_w.max(120.0);
            let links = (screen.width() - width_px).max(0.0) * self.bar_x.clamp(0.0, 1.0);
            let pos = egui::pos2(screen.left() + links, screen.top() + 2.0);
            let p = theme::palette();

            // Hidden means hidden: no handle, no shadow,
            // nothing. The top edge of the screen is the switch - just like
            // in the Windows remote connection.
            if offen {
                let area = egui::Area::new(egui::Id::new("fv_fullbar"))
                    .order(egui::Order::Foreground)
                    .fade_in(false)
                    .fixed_pos(pos)
                    .show(ctx, |ui| {
                        egui::Frame::NONE
                            .fill(p.card.gamma_multiply(0.97))
                            .stroke(egui::Stroke::new(1.0, p.line))
                            .corner_radius(9.0)
                            .shadow(egui::epaint::Shadow {
                                offset: [0, 3],
                                blur: 12,
                                spread: 0,
                                color: egui::Color32::from_black_alpha(90),
                            })
                            .inner_margin(egui::Margin::symmetric(8, 4))
                            .show(ui, |ui| {
                                ui.horizontal(|ui| {
                                    self.session_bar(ui, &mut a, true);
                                });
                            });
                    });
                let bar_rect = area.response.rect;
                if bar_rect.width() > 20.0 {
                    self.bar_w = bar_rect.width();
                }
                if area.response.hovered() || area.response.contains_pointer() {
                    self.bar_seen = std::time::Instant::now();
                }
            }
            if a.drag != 0.0 {
                let span = (screen.width() - self.bar_w).max(1.0);
                self.bar_x = (self.bar_x + a.drag / span).clamp(0.0, 1.0);
                self.bar_seen = std::time::Instant::now();
            }
        } else {
            egui::TopBottomPanel::top("session_bar").show(ctx, |ui| {
                ui.horizontal(|ui| {
                    self.session_bar(ui, &mut a, false);
                });
            });
        }

        if let Some(until) = self.hint_until {
            if std::time::Instant::now() < until {
                let text = if self.hint.is_empty() {
                    i18n::t("sess.escape_long").to_string()
                } else {
                    self.hint.clone()
                };
                if !self.full {
                    egui::TopBottomPanel::top("escape_hint").show(ctx, |ui| {
                        ui.horizontal(|ui| {
                            ui.label(
                                egui::RichText::new(text)
                                    .color(egui::Color32::from_rgb(240, 220, 120)),
                            );
                        });
                    });
                }
            } else {
                self.hint_until = None;
                self.hint.clear();
            }
        }

        let mut image_rect: Option<egui::Rect> = None;
        let mut clicked_image = false;
        egui::CentralPanel::default()
            .frame(egui::Frame::NONE.fill(egui::Color32::from_rgb(18, 18, 18)))
            .show(ctx, |ui| {
                if let Some(tex) = &self.tex {
                    let size_px = tex.size();
                    let (iw, ih) = (size_px[0] as f32, size_px[1] as f32);
                    let avail = ui.available_size();
                    let scale = (avail.x / iw).min(avail.y / ih).max(0.05);
                    let size = egui::vec2(iw * scale, ih * scale);
                    ui.vertical_centered(|ui| {
                        let resp = ui.add(
                            egui::Image::new(egui::load::SizedTexture::new(tex.id(), size))
                                .sense(egui::Sense::click_and_drag()),
                        );
                        clicked_image = resp.clicked();
                        image_rect = Some(resp.rect);
                    });
                } else {
                    ui.centered_and_justified(|ui| {
                        ui.label(i18n::t("sess.waiting"));
                    });
                }
            });

        // No new image for a few seconds? Then we say what it
        // almost always comes down to, instead of just showing "0 fps".
        if self.frame_at.elapsed() > Duration::from_secs(4) {
            let p = theme::palette();
            egui::Area::new(egui::Id::new("fv_stall"))
                .order(egui::Order::Foreground)
                .fade_in(false)
                .anchor(egui::Align2::CENTER_CENTER, egui::vec2(0.0, 40.0))
                .show(ctx, |ui| {
                    egui::Frame::NONE
                        .fill(p.card.gamma_multiply(0.95))
                        .stroke(egui::Stroke::new(1.0, p.line))
                        .corner_radius(10.0)
                        .inner_margin(egui::Margin::symmetric(16, 12))
                        .show(ui, |ui| {
                            ui.set_max_width(460.0);
                            ui.label(
                                egui::RichText::new(i18n::t("sess.stall_head"))
                                    .size(14.0)
                                    .strong(),
                            );
                            ui.add_space(4.0);
                            ui.label(
                                egui::RichText::new(i18n::t("sess.stall_body"))
                                    .size(12.0)
                                    .color(p.muted),
                            );
                        });
                });
        }

        if let Some(rect) = image_rect {
            self.draw_remote_cursor(ctx, rect, game_mode(&self.shared));
            self.update_grab(ctx, rect, game_mode(&self.shared), clicked_image);
            self.forward_input(ctx, rect, game_mode(&self.shared));
        }

        if a.toggle_full {
            let on = !self.full;
            self.set_full(ctx, on);
        }
        if a.toggle_pin {
            self.bar_pinned = !self.bar_pinned;
            self.bar_seen = std::time::Instant::now();
        }
        if let Some(m) = a.mode {
            self.set_mode(m);
        }
        if let Some(i) = a.monitor {
            self.shared.active_monitor.store(i, Ordering::Relaxed);
            self.shared.send_input(Msg::SetMonitor { index: i });
            self.tex = None;
            self.last_seq = 0;
        }
        if let Some((w, h)) = a.res {
            self.shared.send_input(Msg::SetResolution { width: w, height: h });
        }
        if let Some(code) = a.special {
            self.shared.send_input(Msg::Special { code });
        }
        if a.pick {
            self.pick_and_send();
        }
        if a.open_dir {
            self.open_drop_dir();
        }
        if a.disconnect {
            self.set_full(ctx, false);
            self.stop_session();
        }
    }

    /// The duplication API does not paint the cursor into the frame. In game
    /// mode the local pointer is locked and invisible, so the remote one has
    /// to be drawn - in remote maintenance the local pointer sits exactly
    /// where the remote one is and a second arrow only confuses.
    pub(crate) fn draw_remote_cursor(&self, ctx: &egui::Context, rect: egui::Rect, game: bool) {
        if !game {
            return;
        }
        let (x, y, visible) = *self.shared.remote_cursor.lock_ok();
        if !visible {
            return;
        }
        let p = egui::pos2(
            rect.left() + rect.width() * (x as f32 / 10000.0).clamp(0.0, 1.0),
            rect.top() + rect.height() * (y as f32 / 10000.0).clamp(0.0, 1.0),
        );
        let painter = ctx.layer_painter(egui::LayerId::new(
            egui::Order::Foreground,
            egui::Id::new("remote_cursor"),
        ));
        let pts = vec![
            p,
            egui::pos2(p.x, p.y + 17.0),
            egui::pos2(p.x + 4.5, p.y + 12.5),
            egui::pos2(p.x + 10.5, p.y + 12.0),
        ];
        painter.add(egui::Shape::convex_polygon(
            pts,
            egui::Color32::WHITE,
            egui::Stroke::new(1.2, egui::Color32::BLACK),
        ));
    }

    /// Keeps the pointer lock in sync with focus and clicks.
    pub(crate) fn update_grab(&mut self, ctx: &egui::Context, rect: egui::Rect, game: bool, clicked: bool) {
        // The keyboard is fully taken over in BOTH modes -
        // Windows key, Alt+Tab, Alt+F4 then go to the other machine
        // and no longer to the own Windows. Right Ctrl brings it back.
        vinput::set_relative(game);
        let focused = ctx.input(|i| i.viewport().focused.unwrap_or(true));
        if !focused {
            vinput::set_active(false);
            return;
        }
        // center of the picture in physical screen pixels
        let ppp = ctx.pixels_per_point();
        let origin = ctx
            .input(|i| i.viewport().inner_rect)
            .map(|r| r.min)
            .unwrap_or(egui::Pos2::ZERO);
        let c = rect.center();
        vinput::set_center(
            ((origin.x + c.x) * ppp) as i32,
            ((origin.y + c.y) * ppp) as i32,
        );
        if clicked && !vinput::is_active() {
            vinput::set_active(true);
        }
        if vinput::is_active() && game {
            ctx.set_cursor_icon(egui::CursorIcon::None);
        }
    }

    pub(crate) fn forward_input(&mut self, ctx: &egui::Context, rect: egui::Rect, game: bool) {
        let grabbed = game && vinput::is_active();

        // modifier keys are not part of egui's Key enum, so we diff the state.
        // In game mode the low level hook already forwards everything.
        if !grabbed {
            let mods = ctx.input(|i| i.modifiers);
            if mods.shift != self.last_mods.shift {
                self.shared.send_input(Msg::Key {
                    code: proto::KEY_SHIFT,
                    named: true,
                    down: mods.shift,
                });
            }
            if mods.ctrl != self.last_mods.ctrl {
                self.shared.send_input(Msg::Key {
                    code: proto::KEY_CTRL,
                    named: true,
                    down: mods.ctrl,
                });
            }
            if mods.alt != self.last_mods.alt {
                self.shared.send_input(Msg::Key {
                    code: proto::KEY_ALT,
                    named: true,
                    down: mods.alt,
                });
            }
            self.last_mods = mods;
        }

        let norm = |pos: egui::Pos2| -> Option<(i32, i32)> {
            if rect.width() < 1.0 || rect.height() < 1.0 || !rect.contains(pos) {
                return None;
            }
            let x = ((pos.x - rect.left()) / rect.width() * 10000.0).clamp(0.0, 10000.0) as i32;
            let y = ((pos.y - rect.top()) / rect.height() * 10000.0).clamp(0.0, 10000.0) as i32;
            Some((x, y))
        };

        let events = ctx.input(|i| i.events.clone());
        for ev in events {
            match ev {
                egui::Event::PointerMoved(pos) => {
                    // in game mode the raw delta path owns the mouse
                    if !grabbed {
                        if let Some((x, y)) = norm(pos) {
                            self.shared.send_input(Msg::MouseMove { x, y });
                        }
                    }
                }
                egui::Event::PointerButton {
                    pos,
                    button,
                    pressed,
                    ..
                } => {
                    let b = match button {
                        egui::PointerButton::Secondary => 1u8,
                        egui::PointerButton::Middle => 2u8,
                        egui::PointerButton::Extra1 => 3u8,
                        egui::PointerButton::Extra2 => 4u8,
                        _ => 0u8,
                    };
                    if grabbed {
                        self.shared.send_input(Msg::MouseButton {
                            button: b,
                            down: pressed,
                        });
                    } else if let Some((x, y)) = norm(pos) {
                        self.shared.send_input(Msg::MouseMove { x, y });
                        self.shared.send_input(Msg::MouseButton {
                            button: b,
                            down: pressed,
                        });
                    }
                }
                egui::Event::MouseWheel { unit, delta, .. } => {
                    let lines = match unit {
                        egui::MouseWheelUnit::Line => delta.y,
                        egui::MouseWheelUnit::Point => delta.y / 50.0,
                        egui::MouseWheelUnit::Page => delta.y * 3.0,
                    };
                    let l = lines.round() as i32;
                    if l != 0 {
                        self.shared.send_input(Msg::Wheel { lines: l });
                    }
                }
                egui::Event::Key {
                    key,
                    pressed,
                    repeat,
                    ..
                } => {
                    if !repeat && !grabbed {
                        if let Some((code, named)) = map_key(key) {
                            self.shared.send_input(Msg::Key {
                                code,
                                named,
                                down: pressed,
                            });
                        }
                    }
                }
                _ => {}
            }
        }
    }
}
