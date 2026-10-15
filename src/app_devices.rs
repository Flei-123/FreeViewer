//! Split out of main.rs (r35): mechanical move, behaviour unchanged.
use super::*;

impl App {
    pub(crate) fn devices_view(&mut self, ui: &mut egui::Ui) {
        let p = theme::palette();
        let all = self.book.sorted();
        self.presence
            .watch(all.iter().map(|x| x.id.clone()).collect());
        // Fuzzy: case and separators do not matter -
        // "flei one" finds "FLEI-ONE", "497 628" finds the ID.
        let q = partners::search_norm(&self.search);
        let folder = self.folder.clone();
        let hit = |x: &partners::Partner| {
            let text = q.is_empty()
                || partners::search_norm(&x.label()).contains(&q)
                || partners::search_norm(&x.id).contains(&q);
            let fold = folder.is_empty() || x.group == folder;
            text && fold
        };

        let mut connect: Option<String> = None;
        let mut ask: Option<String> = None;
        let mut fav: Option<String> = None;
        let mut remove: Option<String> = None;
        let mut edit: Option<String> = None;

        let detail_w = if self.selected.is_some() { 288.0 } else { 0.0 };

        ui.horizontal_top(|ui| {
            // -------------------------------------------------- Ordner
            ui.vertical(|ui| {
                ui.set_width(148.0);
                label_small(ui, i18n::t("dev.folder"));
                let groups = self.book.groups();
                let mut rows: Vec<(String, String, usize)> = vec![(
                    String::new(),
                    i18n::t("dev.folder_all").to_string(),
                    all.len(),
                )];
                for g in groups.iter() {
                    let n = all.iter().filter(|x| &x.group == g).count();
                    rows.push((g.clone(), g.clone(), n));
                }
                for (key, label, n) in rows {
                    let sel = self.folder == key;
                    let r = ui.add(
                        egui::Button::image_and_text(
                            icons::image(
                                if key.is_empty() { "devices" } else { "files" },
                                14.0,
                                if sel { p.accent } else { p.muted },
                            ),
                            egui::RichText::new(format!("{}  {}", label, n))
                                .size(12.0)
                                .color(if sel { p.text } else { p.muted }),
                        )
                        .fill(if sel {
                            p.accent.gamma_multiply(0.14)
                        } else {
                            egui::Color32::TRANSPARENT
                        })
                        .stroke(egui::Stroke::NONE)
                        .min_size(egui::vec2(140.0, 28.0)),
                    );
                    if r.clicked() {
                        self.folder = key.clone();
                    }
                    ui.add_space(2.0);
                }
            });

            ui.add_space(10.0);

            // -------------------------------------------------- Liste
            ui.vertical(|ui| {
                let w = (ui.available_width() - detail_w - 12.0).max(320.0);
                ui.set_width(w);
                ui.horizontal(|ui| {
                    ui.label(
                        egui::RichText::new(i18n::tf("dev.count", &all.len().to_string()))
                            .size(12.0)
                            .color(p.muted),
                    );
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        if icon_ghost(ui, "plus", i18n::t("dev.add_tip")).clicked() {
                            // What is in the search field is usually exactly the ID
                            // that should be added - so take it over
                            // and let the rest be filled in in the window.
                            let id: String =
                                self.search.chars().filter(|c| c.is_ascii_digit()).collect();
                            self.add_dev = Some(AddDev {
                                id,
                                ..Default::default()
                            });
                        }
                        if icon_ghost(ui, "refresh", i18n::t("dev.refresh_tip")).clicked() {
                            self.presence
                                .watch(all.iter().map(|x| x.id.clone()).collect());
                        }
                    });
                });
                ui.add_space(4.0);

                egui::Frame::NONE
                    .fill(p.card)
                    .stroke(egui::Stroke::new(1.0, p.line))
                    .corner_radius(10)
                    .inner_margin(egui::Margin::symmetric(0, 5))
                    .show(ui, |ui| {
                        ui.set_width(ui.available_width());
                        ui.horizontal(|ui| {
                            ui.add_space(12.0);
                            ui.label(
                                egui::RichText::new(i18n::t("dev.name")).size(10.0).color(p.muted),
                            );
                            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                                ui.add_space(28.0);
                                ui.label(
                                    egui::RichText::new(i18n::t("dev.status"))
                                        .size(10.0)
                                        .color(p.muted),
                                );
                                ui.add_space(28.0);
                                ui.label(
                                    egui::RichText::new(i18n::t("dev.id")).size(10.0).color(p.muted),
                                );
                            });
                        });
                        ui.add_space(3.0);
                        divider(ui);
                        ui.add_space(3.0);

                        let mut groups: Vec<(String, Vec<partners::Partner>)> = Vec::new();
                        if all.len() > 3 && folder.is_empty() && q.is_empty() {
                            groups.push((
                                i18n::t("dev.group_recent").to_string(),
                                all.iter().filter(|x| hit(x)).take(3).cloned().collect(),
                            ));
                        }
                        let favs: Vec<partners::Partner> =
                            all.iter().filter(|x| hit(x) && x.favorite).cloned().collect();
                        if !favs.is_empty() {
                            groups.push((i18n::t("dev.group_fav").to_string(), favs));
                        }
                        groups.push((
                            i18n::t("dev.group_online").to_string(),
                            all.iter()
                                .filter(|x| hit(x) && self.presence.online(&x.id))
                                .cloned()
                                .collect(),
                        ));
                        groups.push((
                            i18n::t("dev.group_offline").to_string(),
                            all.iter()
                                .filter(|x| hit(x) && !self.presence.online(&x.id))
                                .cloned()
                                .collect(),
                        ));

                        let mut shown = 0usize;
                        for (title, list) in groups {
                            if list.is_empty() {
                                continue;
                            }
                            shown += list.len();
                            let gid = ui.make_persistent_id(("fv_group", title.clone()));
                            let open = ui.data_mut(|d| *d.get_temp_mut_or(gid, true));
                            let head = ui.add(
                                egui::Button::image_and_text(
                                    icons::image(
                                        if open { "chevron-down" } else { "chevron-right" },
                                        12.0,
                                        p.muted,
                                    ),
                                    egui::RichText::new(format!("{}  {}", title, list.len()))
                                        .size(12.0)
                                        .strong()
                                        .color(p.text),
                                )
                                .fill(p.card_hi)
                                .stroke(egui::Stroke::NONE)
                                .corner_radius(0)
                                .min_size(egui::vec2(ui.available_width(), 24.0)),
                            );
                            if head.clicked() {
                                ui.data_mut(|d| d.insert_temp(gid, !open));
                            }
                            if !open {
                                continue;
                            }
                            for x in list.iter() {
                                let on = self.presence.online(&x.id);
                                let sel = self.selected.as_deref() == Some(x.id.as_str());
                                egui::Frame::NONE
                                    .fill(if sel {
                                        p.row_sel
                                    } else {
                                        egui::Color32::TRANSPARENT
                                    })
                                    .inner_margin(egui::Margin::symmetric(10, 3))
                                    .show(ui, |ui| {
                                        ui.set_width(ui.available_width());
                                        ui.horizontal(|ui| {
                                            let shown = self.dev_label(x);
                                            icons::show(
                                                ui,
                                                device_symbol(&shown),
                                                15.0,
                                                row_color(on, false),
                                            );
                                            let name = ui.add(
                                                egui::Label::new(
                                                    egui::RichText::new(&shown)
                                                        .size(12.5)
                                                        .color(row_color(on, true)),
                                                )
                                                .sense(egui::Sense::click()),
                                            );
                                            if name.clicked() {
                                                self.selected = Some(x.id.clone());
                                            }
                                            if name.double_clicked() {
                                                connect = Some(x.id.clone());
                                            }
                                            if x.favorite {
                                                icons::show(ui, "star", 11.0, p.accent);
                                            }
                                            if self.is_me(&x.id) {
                                                ui.label(
                                                    egui::RichText::new(i18n::t("dev.me"))
                                                        .size(10.5)
                                                        .color(p.accent),
                                                );
                                            }
                                            ui.with_layout(
                                                egui::Layout::right_to_left(egui::Align::Center),
                                                |ui| {
                                                    ui.menu_image_button(
                                                        icons::image("dots", 13.0, p.muted),
                                                        |ui| {
                                                            if ui
                                                                .button(i18n::t("dev.connect"))
                                                                .clicked()
                                                            {
                                                                connect = Some(x.id.clone());
                                                                ui.close();
                                                            }
                                                            if ui
                                                                .button(i18n::t("dev.ask_connect"))
                                                                .clicked()
                                                            {
                                                                ask = Some(x.id.clone());
                                                                ui.close();
                                                            }
                                                            if ui.button(i18n::t("dev.edit")).clicked()
                                                            {
                                                                edit = Some(x.id.clone());
                                                                ui.close();
                                                            }
                                                            if ui
                                                                .button(if x.favorite {
                                                                    i18n::t("dev.fav_del")
                                                                } else {
                                                                    i18n::t("dev.fav_add")
                                                                })
                                                                .clicked()
                                                            {
                                                                fav = Some(x.id.clone());
                                                                ui.close();
                                                            }
                                                            if ui
                                                                .button(i18n::t("dev.remove"))
                                                                .clicked()
                                                            {
                                                                remove = Some(x.id.clone());
                                                                ui.close();
                                                            }
                                                        },
                                                    );
                                                    ui.add_space(4.0);
                                                    status_pill(
                                                        ui,
                                                        on,
                                                        if on {
                                                            i18n::t("st.online")
                                                        } else {
                                                            i18n::t("st.offline")
                                                        },
                                                    );
                                                    ui.add_space(10.0);
                                                    ui.label(
                                                        egui::RichText::new(partners::pretty_id(
                                                            &x.id,
                                                        ))
                                                        .size(12.0)
                                                        .color(if on {
                                                            p.muted
                                                        } else {
                                                            p.muted.gamma_multiply(0.7)
                                                        }),
                                                    );
                                                },
                                            );
                                        });
                                    });
                            }
                        }

                        if shown == 0 {
                            ui.add_space(8.0);
                            ui.vertical_centered(|ui| {
                                icons::show(ui, "devices", 24.0, p.muted);
                                ui.label(
                                    egui::RichText::new(if all.is_empty() {
                                        i18n::t("dev.empty")
                                    } else {
                                        i18n::t("dev.nohit")
                                    })
                                    .size(12.0)
                                    .color(p.muted),
                                );
                            });
                            ui.add_space(8.0);
                        }
                    });

                ui.add_space(6.0);
                ui.horizontal(|ui| {
                    dot(ui, true);
                    ui.label(
                        egui::RichText::new(i18n::t("dev.secure"))
                            .size(11.0)
                            .color(p.muted),
                    );
                });
            });

            // -------------------------------------------------- Detail
            if let Some(id) = self.selected.clone() {
                ui.add_space(12.0);
                ui.vertical(|ui| {
                    ui.set_width(276.0);
                    let entry = self.book.get(&id).cloned().unwrap_or_default();
                    let on = self.presence.online(&id);
                    let shown = self.dev_label(&entry);
                    card(ui, |ui| {
                        ui.horizontal(|ui| {
                            icons::show(ui, device_symbol(&shown), 18.0, row_color(on, false));
                            ui.label(
                                egui::RichText::new(&shown)
                                    .size(15.0)
                                    .strong()
                                    .color(p.text),
                            );
                            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                                if icon_ghost(ui, "dots", i18n::t("dev.edit")).clicked() {
                                    edit = Some(id.clone());
                                }
                            });
                        });
                        ui.add_space(2.0);
                        ui.horizontal(|ui| {
                            status_pill(
                                ui,
                                on,
                                if on {
                                    i18n::t("st.online")
                                } else {
                                    i18n::t("st.offline")
                                },
                            );
                            ui.label(
                                egui::RichText::new(partners::pretty_id(&entry.id))
                                    .size(12.0)
                                    .color(p.muted),
                            );
                        });
                        ui.add_space(8.0);
                        if accent_button(ui, i18n::t("dev.connect"), true).clicked() {
                            connect = Some(id.clone());
                        }
                        ui.add_space(4.0);
                        if ui
                            .add(ghost(egui::vec2(220.0, 28.0), i18n::t("dev.ask_connect")))
                            .on_hover_text(i18n::t("start.ask_tip"))
                            .clicked()
                        {
                            ask = Some(id.clone());
                        }
                        ui.add_space(8.0);
                        divider(ui);
                        ui.add_space(5.0);
                        info_row(
                            ui,
                            i18n::t("start.password"),
                            if entry.secret.is_some() {
                                i18n::t("dev.pw_stored")
                            } else {
                                i18n::t("dev.pw_none")
                            },
                        );
                        info_row(ui, i18n::t("dev.folder"), if entry.group.is_empty() { "-" } else { &entry.group });
                        info_row(ui, i18n::t("dev.stats"), &entry.count.to_string());
                        info_row(ui, i18n::t("dev.last"), &entry.ago());
                        if !entry.note.is_empty() {
                            ui.add_space(3.0);
                            ui.label(egui::RichText::new(&entry.note).size(11.5).color(p.muted));
                        }
                    });

                    // ---- Bearbeiten
                    if self.edit_dev.as_ref().map(|e| e.id.clone()) == Some(id.clone()) {
                        let mut e = self.edit_dev.clone().unwrap_or_default();
                        let mut save = false;
                        let mut cancel = false;
                        ui.add_space(8.0);
                        label_small(ui, i18n::t("dev.edit"));
                        card(ui, |ui| {
                            label_small(ui, i18n::t("dev.newname"));
                            ui.add(
                                egui::TextEdit::singleline(&mut e.name)
                                    .desired_width(230.0)
                                    .margin(egui::Margin::symmetric(8, 4)),
                            );
                            ui.add_space(5.0);
                            label_small(ui, i18n::t("dev.pw_set"));
                            ui.add(
                                egui::TextEdit::singleline(&mut e.password)
                                    .password(true)
                                    .desired_width(230.0)
                                    .margin(egui::Margin::symmetric(8, 4))
                                    .hint_text(i18n::t("start.pw_hint")),
                            );
                            ui.add_space(5.0);
                            label_small(ui, i18n::t("dev.folder"));
                            ui.horizontal(|ui| {
                                ui.add(
                                    egui::TextEdit::singleline(&mut e.folder)
                                        .desired_width(140.0)
                                        .margin(egui::Margin::symmetric(8, 4))
                                        .hint_text(i18n::t("dev.folder_new")),
                                );
                                let known = self.book.groups();
                                if !known.is_empty() {
                                    egui::ComboBox::from_id_salt("fold_pick")
                                        .selected_text("…")
                                        .width(60.0)
                                        .show_ui(ui, |ui| {
                                            for g in known.iter() {
                                                if ui.selectable_label(false, g).clicked() {
                                                    e.folder = g.clone();
                                                }
                                            }
                                        });
                                }
                            });
                            ui.add_space(5.0);
                            label_small(ui, i18n::t("dev.note"));
                            ui.add(
                                egui::TextEdit::multiline(&mut e.note)
                                    .desired_rows(2)
                                    .desired_width(230.0),
                            );
                            ui.add_space(6.0);
                            ui.horizontal(|ui| {
                                if accent_button(ui, i18n::t("dev.save"), true).clicked() {
                                    save = true;
                                }
                                if ghost_button(ui, i18n::t("dev.cancel")).clicked() {
                                    cancel = true;
                                }
                                ui.with_layout(
                                    egui::Layout::right_to_left(egui::Align::Center),
                                    |ui| {
                                        if icon_ghost(ui, "mic-off", i18n::t("dev.pw_clear"))
                                            .clicked()
                                        {
                                            self.book.set_password(&id, None);
                                        }
                                    },
                                );
                            });
                        });
                        if save {
                            self.book.rename(&id, &e.name);
                            self.book.set_group(&id, &e.folder);
                            self.book.set_note(&id, &e.note);
                            if !e.password.is_empty() {
                                self.book.set_password(&id, Some(&e.password));
                            }
                            self.edit_dev = None;
                        } else if cancel {
                            self.edit_dev = None;
                        } else {
                            self.edit_dev = Some(e);
                        }
                    }
                });
            }
        });

        if let Some(id) = edit {
            let e = self.book.get(&id).cloned().unwrap_or_default();
            self.selected = Some(id.clone());
            self.edit_dev = Some(DevEdit {
                id,
                name: e.name.clone(),
                password: String::new(),
                note: e.note.clone(),
                folder: e.group.clone(),
            });
        }
        if let Some(id) = connect {
            self.connect_to(&id);
        }
        if let Some(id) = ask {
            self.partner_id = id.clone();
            self.partner_pw.clear();
            self.selected = Some(id);
            self.start_ask_session();
        }
        if let Some(id) = fav {
            self.book.toggle_favorite(&id);
            self.book.save();
        }
        if let Some(id) = remove {
            self.book.remove(&id);
            self.book.save();
            if self.selected.as_deref() == Some(id.as_str()) {
                self.selected = None;
                self.edit_dev = None;
            }
        }
    }

    /// "Add device": a small window with everything an entry
    /// needs. Previously the plus button only did something if a complete
    /// ID happened to be in the search field - for everyone else
    /// it was broken.
    pub(crate) fn add_dev_ui(&mut self, ctx: &egui::Context) {
        let Some(mut e) = self.add_dev.clone() else {
            return;
        };
        let p = theme::palette();
        let mut open = true;
        let mut save = false;
        egui::Window::new(i18n::t("dev.add_title"))
            .collapsible(false)
            .resizable(false)
            .open(&mut open)
            .anchor(egui::Align2::CENTER_CENTER, egui::vec2(0.0, -40.0))
            .show(ctx, |ui| {
                ui.set_width(320.0);
                ui.add_space(2.0);
                label_small(ui, i18n::t("dev.add_id"));
                let id_field = ui.add(
                    egui::TextEdit::singleline(&mut e.id)
                        .desired_width(f32::INFINITY)
                        .hint_text(i18n::t("dev.add_id_hint")),
                );
                if self.add_dev.is_some() && !id_field.has_focus() && e.id.is_empty() {
                    id_field.request_focus();
                }
                ui.add_space(8.0);
                label_small(ui, i18n::t("dev.add_name"));
                ui.add(
                    egui::TextEdit::singleline(&mut e.name)
                        .desired_width(f32::INFINITY)
                        .hint_text(i18n::t("dev.add_name_hint")),
                );
                ui.add_space(8.0);
                label_small(ui, i18n::t("dev.folder"));
                ui.add(
                    egui::TextEdit::singleline(&mut e.folder)
                        .desired_width(f32::INFINITY)
                        .hint_text(i18n::t("dev.folder_new")),
                );
                ui.add_space(8.0);
                label_small(ui, i18n::t("dev.add_pw"));
                ui.add(
                    egui::TextEdit::singleline(&mut e.password)
                        .password(true)
                        .desired_width(f32::INFINITY)
                        .hint_text(i18n::t("dev.add_pw_hint")),
                );
                if !e.err.is_empty() {
                    ui.add_space(6.0);
                    ui.label(
                        egui::RichText::new(&e.err).size(11.5).color(p.violet),
                    );
                }
                ui.add_space(12.0);
                ui.horizontal(|ui| {
                    if accent_button(ui, i18n::t("dev.add_ok"), true).clicked() {
                        save = true;
                    }
                    if ghost_button(ui, i18n::t("dev.cancel")).clicked() {
                        self.add_dev = None;
                    }
                });
            });
        if ctx.input(|i| i.key_pressed(egui::Key::Enter)) {
            save = true;
        }
        if ctx.input(|i| i.key_pressed(egui::Key::Escape)) {
            self.add_dev = None;
            return;
        }
        if !open {
            self.add_dev = None;
            return;
        }
        if !save {
            self.add_dev = Some(e);
            return;
        }
        let id: String = e.id.chars().filter(|c| c.is_ascii_digit()).collect();
        if !(9..=10).contains(&id.len()) {
            e.err = i18n::t("dev.add_bad_id").to_string();
        } else if self.is_me(&id) {
            e.err = i18n::t("dev.add_self").to_string();
        } else if self.book.get(&id).is_some() {
            e.err = i18n::t("dev.add_known").to_string();
        } else {
            self.book.started(&id, "", false);
            if !e.name.trim().is_empty() {
                self.book.rename(&id, &e.name);
            }
            if !e.folder.trim().is_empty() {
                self.book.set_group(&id, &e.folder);
            }
            if !e.password.is_empty() {
                self.book.set_password(&id, Some(&e.password));
            }
            self.book.save();
            self.presence.watch(vec![id.clone()]);
            self.search.clear();
            self.selected = Some(id);
            self.add_dev = None;
            return;
        }
        self.add_dev = Some(e);
    }

}
