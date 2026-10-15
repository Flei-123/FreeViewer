//! Split out of main.rs (r35): mechanical move, behaviour unchanged.
use super::*;

/// Fonts. On Windows the bundled Segoe UI looks right, on
/// the Mac the system font, on Linux DejaVu/Noto. If nothing is found,
/// the font built into egui remains.
///
/// IMPORTANT: The family "head" MUST always be assigned at least one
/// font at the end. Up to v0.26.1 it was only created if a
/// Windows file was found - on Mac and Linux it was empty and egui
/// aborted on the first drawing of a heading
/// ("FontFamily::Name(\"head\") is not bound to any fonts").
/// That was exactly the crash "X-Remote quit unexpectedly" on the Mac.
pub(crate) fn install_fonts(ctx: &egui::Context) {
    let mut fonts = egui::FontDefinitions::default();

    // Candidates per operating system, in order of preference.
    #[cfg(windows)]
    let (regular, bold): (&[&str], &[&str]) = (
        &["C:/Windows/Fonts/segoeui.ttf", "C:/Windows/Fonts/SegUIVar.ttf"],
        &["C:/Windows/Fonts/seguisb.ttf", "C:/Windows/Fonts/segoeuib.ttf"],
    );
    #[cfg(target_os = "macos")]
    let (regular, bold): (&[&str], &[&str]) = (
        &[
            "/System/Library/Fonts/SFNS.ttf",
            "/System/Library/Fonts/SFNSText.ttf",
            "/System/Library/Fonts/Helvetica.ttc",
            "/Library/Fonts/Arial.ttf",
        ],
        &[
            "/System/Library/Fonts/SFNSRounded.ttf",
            "/System/Library/Fonts/SFNSDisplay.ttf",
            "/System/Library/Fonts/SFNS.ttf",
            "/System/Library/Fonts/Helvetica.ttc",
        ],
    );
    #[cfg(all(unix, not(target_os = "macos")))]
    let (regular, bold): (&[&str], &[&str]) = (
        &[
            "/usr/share/fonts/truetype/dejavu/DejaVuSans.ttf",
            "/usr/share/fonts/truetype/noto/NotoSans-Regular.ttf",
            "/usr/share/fonts/truetype/liberation/LiberationSans-Regular.ttf",
        ],
        &[
            "/usr/share/fonts/truetype/dejavu/DejaVuSans-Bold.ttf",
            "/usr/share/fonts/truetype/noto/NotoSans-Bold.ttf",
            "/usr/share/fonts/truetype/liberation/LiberationSans-Bold.ttf",
        ],
    );

    let mut body: Option<&str> = None;
    let mut head: Option<&str> = None;
    for (key, files) in [("ui", regular), ("ui_bold", bold)] {
        for f in files {
            if let Ok(bytes) = std::fs::read(f) {
                fonts.font_data.insert(
                    key.to_owned(),
                    std::sync::Arc::new(egui::FontData::from_owned(bytes)),
                );
                if key == "ui" {
                    body = Some(key);
                } else {
                    head = Some(key);
                }
                break;
            }
        }
    }
    if let Some(b) = body {
        fonts
            .families
            .entry(egui::FontFamily::Proportional)
            .or_default()
            .insert(0, b.to_owned());
    }

    // The heading family ALWAYS gets content: first the bold
    // font (if found), then the complete list of normal
    // fonts as a fallback. This way it can never be empty.
    let fallback = fonts
        .families
        .get(&egui::FontFamily::Proportional)
        .cloned()
        .unwrap_or_default();
    let liste = fonts
        .families
        .entry(egui::FontFamily::Name("head".into()))
        .or_insert_with(|| fallback.clone());
    if let Some(h) = head {
        liste.insert(0, h.to_owned());
    }
    if liste.is_empty() {
        // Letzte Rettung - darf eigentlich nie passieren.
        liste.extend(fallback.iter().cloned());
        if liste.is_empty() {
            liste.push("Ubuntu-Light".to_owned());
        }
    }
    ctx.set_fonts(fonts);
}

/// Bold font for headings, otherwise the normal one.
pub(crate) fn head_font(size: f32) -> egui::FontId {
    egui::FontId::new(size, egui::FontFamily::Name("head".into()))
}

pub(crate) fn install_theme(ctx: &egui::Context) {
    install_fonts(ctx);
    // SVG icons: the image loader of egui_extras rasterizes them at the size
    // in which they are needed.
    egui_extras::install_image_loaders(ctx);
    let look = theme::load();
    i18n::set_lang(&look.lang);
    theme::apply(ctx, &look);
}
/// Color haze and fine grid behind everything - the same trick as on
/// fleitec pages, just painted instead of via CSS.
pub(crate) fn paint_background(ctx: &egui::Context) {
    let painter = ctx.layer_painter(egui::LayerId::background());
    let r = ctx.screen_rect();
    painter.rect_filled(r, 0.0, theme::bg());

    if !theme::palette().orbs {
        return;
    }
    // zwei weiche Farborbs (viele Kreise mit wenig Deckkraft = Verlauf)
    for (rel_x, rel_y, radius, color) in [
        (0.18, 0.05, 0.55, theme::accent()),
        (0.92, 0.75, 0.50, theme::violet()),
    ] {
        let center = egui::pos2(r.left() + r.width() * rel_x, r.top() + r.height() * rel_y);
        let max = r.width().max(r.height()) * radius;
        let steps = 14;
        for i in 0..steps {
            let f = 1.0 - i as f32 / steps as f32;
            painter.circle_filled(center, max * f, color.gamma_multiply(0.012));
        }
    }

    // 64px grid that fades out downwards
    let step = 64.0;
    let mut y = r.top();
    while y < r.bottom() {
        let fade = (1.0 - (y - r.top()) / r.height()).clamp(0.0, 1.0);
        painter.hline(
            r.left()..=r.right(),
            y,
            egui::Stroke::new(1.0, egui::Color32::from_white_alpha((10.0 * fade) as u8)),
        );
        y += step;
    }
    let mut x = r.left();
    while x < r.right() {
        painter.vline(
            x,
            r.top()..=r.bottom(),
            egui::Stroke::new(1.0, egui::Color32::from_white_alpha(5)),
        );
        x += step;
    }
}

/// Marke oben links: abgerundetes Quadrat mit Auge.
pub(crate) fn logo(ui: &mut egui::Ui) {
    // The real FreeViewer logo - the same image as on the website and as the
    // window icon. Load once as a texture, afterwards just paint.
    let id = egui::Id::new("fv_logo_tex");
    let tex = ui
        .ctx()
        .data_mut(|d| d.get_temp::<egui::TextureHandle>(id))
        .unwrap_or_else(|| {
            let bild = image::load_from_memory(include_bytes!("../assets/icon.png"))
                .map(|i| i.to_rgba8())
                .unwrap_or_else(|_| {
                    image::RgbaImage::from_pixel(1, 1, image::Rgba([0x38, 0xbd, 0xf8, 0xff]))
                });
            let (w, h) = bild.dimensions();
            let ci =
                egui::ColorImage::from_rgba_unmultiplied([w as usize, h as usize], &bild.into_raw());
            let t = ui
                .ctx()
                .load_texture("fv_logo", ci, egui::TextureOptions::LINEAR);
            ui.ctx().data_mut(|d| d.insert_temp(id, t.clone()));
            t
        });
    ui.image((tex.id(), egui::vec2(34.0, 34.0)));
}

/// Tab in the header.
pub(crate) fn tab(ui: &mut egui::Ui, text: &str, selected: bool) -> egui::Response {
    let label = egui::RichText::new(text)
        .size(13.5)
        .color(if selected { theme::bg() } else { theme::muted() });
    let btn = egui::Button::new(label)
        .fill(if selected {
            theme::accent()
        } else {
            egui::Color32::TRANSPARENT
        })
        .stroke(egui::Stroke::NONE)
        .corner_radius(9)
        .min_size(egui::vec2(0.0, 24.0));
    ui.add(btn)
}

/// Heading above a card.
pub(crate) fn section(ui: &mut egui::Ui, title: &str) {
    ui.label(
        egui::RichText::new(title)
            .font(head_font(15.5))
            .color(theme::text()),
    );
    ui.add_space(3.0);
}

/// Small gray label above a field.
pub(crate) fn label_small(ui: &mut egui::Ui, text: &str) {
    ui.label(egui::RichText::new(text).size(11.5).color(theme::muted()));
    ui.add_space(2.0);
}

/// Thin divider line inside a card.
pub(crate) fn divider(ui: &mut egui::Ui) {
    let r = ui.available_rect_before_wrap();
    ui.painter().hline(
        r.left()..=r.right(),
        r.top(),
        egui::Stroke::new(1.0, theme::line()),
    );
    ui.add_space(1.0);
}

/// A card in the house style.
pub(crate) fn card<R>(ui: &mut egui::Ui, add: impl FnOnce(&mut egui::Ui) -> R) -> R {
    egui::Frame::group(ui.style())
        .fill(theme::card())
        .stroke(egui::Stroke::new(1.0, theme::line()))
        .corner_radius(13)
        .inner_margin(egui::Margin::same(10))
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            add(ui)
        })
        .inner
}

/// The one blue button.
pub(crate) fn accent_button(ui: &mut egui::Ui, text: &str, enabled: bool) -> egui::Response {
    let btn = egui::Button::new(
        egui::RichText::new(text)
            .size(14.5)
            .color(if enabled { theme::bg() } else { theme::muted() }),
    )
    .fill(if enabled { theme::accent() } else { theme::card_hi() })
    .stroke(egui::Stroke::NONE)
    .corner_radius(10)
    .min_size(egui::vec2(200.0, 31.0));
    ui.add_enabled(enabled, btn)
}

/// Zweitrangiger Knopf mit Rand.
pub(crate) fn ghost(size: egui::Vec2, text: &str) -> egui::Button<'static> {
    egui::Button::new(egui::RichText::new(text).size(13.5).color(theme::text()))
        .fill(theme::card_hi())
        .stroke(egui::Stroke::new(1.0, theme::line()))
        .corner_radius(10)
        .min_size(size)
}

/// Kleiner zweitrangiger Knopf.
pub(crate) fn ghost_button(ui: &mut egui::Ui, text: &str) -> egui::Response {
    zeigefinger(ui.add(
        egui::Button::new(egui::RichText::new(text).size(12.5).color(theme::muted()))
            .fill(theme::card_hi())
            .stroke(egui::Stroke::new(1.0, theme::line()))
            .corner_radius(8),
    ))
}

/// Green or gray dot in front of a device.
pub(crate) fn dot(ui: &mut egui::Ui, online: bool) {
    let (rect, _) = ui.allocate_exact_size(egui::vec2(12.0, 12.0), egui::Sense::hover());
    let c = if online { theme::green() } else { theme::muted().gamma_multiply(0.55) };
    if online {
        ui.painter().circle_filled(rect.center(), 6.5, c.gamma_multiply(0.25));
    }
    ui.painter().circle_filled(rect.center(), 4.0, c);
}

/// State des eigenen Rechners, oben rechts.
pub(crate) fn status_pill(ui: &mut egui::Ui, ok: bool, text: &str) {
    egui::Frame::group(ui.style())
        .fill(if ok {
            theme::green().gamma_multiply(0.14)
        } else {
            theme::card_hi()
        })
        .stroke(egui::Stroke::new(
            1.0,
            if ok { theme::green().gamma_multiply(0.6) } else { theme::line() },
        ))
        .corner_radius(12)
        .inner_margin(egui::Margin::symmetric(10, 3))
        .show(ui, |ui| {
            ui.label(
                egui::RichText::new(text)
                    .size(11.5)
                    .color(if ok { theme::green() } else { theme::muted() }),
            );
        });
}

/// "Key   Value" row in the info card.
pub(crate) fn info_row(ui: &mut egui::Ui, key: &str, value: &str) {
    ui.horizontal(|ui| {
        ui.label(egui::RichText::new(key).color(theme::muted()).size(11.5));
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            ui.add(
                egui::Label::new(egui::RichText::new(value).size(12.5))
                    .selectable(true)
                    .truncate(),
            );
        });
    });
    ui.add_space(2.0);
}

/// Window icon: the same eye as in the tray, 32x32 RGBA.
pub(crate) fn app_icon() -> egui::IconData {
    // The same drawing that is also in the EXE (see build.rs), so that
    // window, taskbar, start menu and Explorer show the same thing.
    const RAW: &[u8] = include_bytes!("../assets/icon.png");
    match image::load_from_memory(RAW) {
        Ok(img) => {
            let rgba = img.to_rgba8();
            let (w, h) = rgba.dimensions();
            egui::IconData {
                rgba: rgba.into_raw(),
                width: w,
                height: h,
            }
        }
        Err(_) => egui::IconData {
            rgba: vec![0x38, 0xbd, 0xf8, 0xff],
            width: 1,
            height: 1,
        },
    }
}

/// Saves an image of the UI as JPEG (for --shot).
pub(crate) fn save_shot(img: &egui::ColorImage, path: &std::path::Path) {
    use image::ImageEncoder;
    let [w, h] = img.size;
    let mut rgb = Vec::with_capacity(w * h * 3);
    for p in &img.pixels {
        rgb.extend_from_slice(&[p.r(), p.g(), p.b()]);
    }
    match std::fs::File::create(path) {
        Ok(mut f) => {
            let enc = image::codecs::jpeg::JpegEncoder::new_with_quality(&mut f, 92);
            match enc.write_image(&rgb, w as u32, h as u32, image::ExtendedColorType::Rgb8) {
                Ok(()) => println!("SHOT {}x{} -> {}", w, h, path.display()),
                Err(e) => eprintln!("SHOT FAIL: {}", e),
            }
        }
        Err(e) => eprintln!("SHOT FAIL: {}", e),
    }
}

// ------------------------------------------------------------ kleine Helfer

/// Icon button without a frame - hover and press come from the palette.
pub(crate) fn zeigefinger(r: egui::Response) -> egui::Response {
    r.on_hover_cursor(egui::CursorIcon::PointingHand)
}

pub(crate) fn icon_ghost(ui: &mut egui::Ui, icon: &str, tip: &str) -> egui::Response {
    let r = ui
        .scope(|ui| {
            let v = ui.visuals_mut();
            v.widgets.inactive.weak_bg_fill = egui::Color32::TRANSPARENT;
            v.widgets.inactive.bg_stroke = egui::Stroke::NONE;
            v.widgets.hovered.bg_stroke = egui::Stroke::NONE;
            v.widgets.active.bg_stroke = egui::Stroke::NONE;
            ui.add(
                egui::Button::image(icons::image(icon, 15.0, theme::muted()))
                    .corner_radius(7)
                    .min_size(egui::vec2(28.0, 24.0)),
            )
        })
        .inner;
    zeigefinger(r).on_hover_text(tip)
}

/// Icon button that can be toggled (microphone, audio).
pub(crate) fn icon_toggle(ui: &mut egui::Ui, icon: &str, on: bool, size: f32, tip: &str) -> egui::Response {
    let col = if on { theme::accent() } else { theme::muted() };
    let r = ui
        .scope(|ui| {
            let v = ui.visuals_mut();
            v.widgets.inactive.weak_bg_fill = if on {
                theme::accent().gamma_multiply(0.16)
            } else {
                egui::Color32::TRANSPARENT
            };
            v.widgets.inactive.bg_stroke = egui::Stroke::NONE;
            v.widgets.hovered.bg_stroke = egui::Stroke::NONE;
            v.widgets.active.bg_stroke = egui::Stroke::NONE;
            ui.add(
                egui::Button::image(icons::image(icon, size, col))
                    .corner_radius(8)
                    .min_size(egui::vec2(size + 14.0, size + 11.0)),
            )
        })
        .inner;
    zeigefinger(r).on_hover_text(tip)
}

/// Two small level bars: out and in.
pub(crate) fn level_bar(ui: &mut egui::Ui, out: u32, inn: u32) {
    let p = theme::palette();
    for (val, col) in [(out, p.accent), (inn, p.green)] {
        let (rect, _) = ui.allocate_exact_size(egui::vec2(44.0, 6.0), egui::Sense::hover());
        ui.painter().rect_filled(rect, 3.0, p.card_hi);
        let w = rect.width() * (val.min(100) as f32 / 100.0);
        if w > 0.5 {
            ui.painter().rect_filled(
                egui::Rect::from_min_size(rect.min, egui::vec2(w, rect.height())),
                3.0,
                col,
            );
        }
        ui.add_space(4.0);
    }
}

/// Monitor or laptop - depending on what the device is called.
pub(crate) fn device_symbol(label: &str) -> &'static str {
    let l = label.to_lowercase();
    if l.contains("laptop") || l.contains("book") || l.contains("note") {
        "laptop"
    } else {
        "monitor"
    }
}

/// Offline steht grau da, online gruen bzw. in normaler Schrift.
pub(crate) fn row_color(online: bool, is_text: bool) -> egui::Color32 {
    let p = theme::palette();
    if online {
        if is_text {
            p.text
        } else {
            p.green
        }
    } else if is_text {
        p.muted.gamma_multiply(0.75)
    } else {
        p.muted.gamma_multiply(0.55)
    }
}


// ============================================================ Controls
//
// egui brings its own checkboxes and sliders, but they deform into dots
// with the configured rounding and are barely visible in light mode.
// So we draw both ourselves - angular, with a visible border
// and a reaction to the mouse.

/// Eckiges Kaestchen mit Haken.
pub(crate) fn check(ui: &mut egui::Ui, on: &mut bool, text: &str) -> egui::Response {
    let p = theme::palette();
    let side = 17.0;
    let gap = 9.0;
    let font = egui::TextStyle::Body.resolve(ui.style());
    let galley = ui.painter().layout_no_wrap(text.to_string(), font, p.text);
    let size = egui::vec2(side + gap + galley.size().x, side.max(galley.size().y));
    let (rect, mut resp) = ui.allocate_exact_size(size, egui::Sense::click());
    if resp.clicked() {
        *on = !*on;
        resp.mark_changed();
    }
    let hov = resp.hovered();
    let br = egui::Rect::from_min_size(
        egui::pos2(rect.min.x, rect.center().y - side / 2.0),
        egui::vec2(side, side),
    );
    let pt = ui.painter();
    let idle_fill = if p.dark { p.field } else { p.card_hi };
    let idle_edge = if p.dark {
        p.line
    } else {
        // in light mode the hairline is not enough - the border has to carry
        p.muted.gamma_multiply(0.85)
    };
    let fill = if *on {
        p.accent
    } else if hov {
        p.accent.gamma_multiply(0.12)
    } else {
        idle_fill
    };
    let edge = if *on || hov { p.accent } else { idle_edge };
    pt.rect_filled(br, 3.0, fill);
    pt.rect_stroke(
        br,
        3.0,
        egui::Stroke::new(if hov { 2.0 } else { 1.4 }, edge),
        egui::StrokeKind::Inside,
    );
    if *on {
        let c = br.center();
        let s = side * 0.30;
        pt.add(egui::Shape::line(
            vec![
                egui::pos2(c.x - s, c.y),
                egui::pos2(c.x - s * 0.15, c.y + s * 0.8),
                egui::pos2(c.x + s, c.y - s * 0.75),
            ],
            egui::Stroke::new(2.2, p.on_accent),
        ));
    }
    let ty = rect.center().y - galley.size().y / 2.0;
    pt.galley(egui::pos2(br.max.x + gap, ty), galley, p.text);
    zeigefinger(resp)
}

/// Slider with label on the left and value on the right - nothing slips
/// into each other anymore like with "Schriftgroesse 100 %" and "Rundung".
pub(crate) fn slider_row(
    ui: &mut egui::Ui,
    label: &str,
    value: &mut f32,
    range: std::ops::RangeInclusive<f32>,
    shown: &str,
) -> bool {
    let p = theme::palette();
    let mut changed = false;
    ui.horizontal(|ui| {
        ui.allocate_ui_with_layout(
            egui::vec2(108.0, 22.0),
            egui::Layout::left_to_right(egui::Align::Center),
            |ui| {
                ui.label(egui::RichText::new(label).size(12.5).color(p.muted));
            },
        );
        let w = (ui.available_width() - 66.0).clamp(90.0, 460.0);
        changed = ui
            .add_sized(
                egui::vec2(w, 22.0),
                egui::Slider::new(value, range).show_value(false),
            )
            .changed();
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            ui.label(egui::RichText::new(shown).size(12.5).color(p.text));
        });
    });
    changed
}

/// Selection list with label on the left. Returns the new value if
/// something changed. `None` in the list = system default.
pub(crate) fn device_row(
    ui: &mut egui::Ui,
    id: &str,
    label: &str,
    current: &Option<String>,
    default_name: &str,
    list: &[String],
) -> Option<Option<String>> {
    let p = theme::palette();
    let mut picked: Option<Option<String>> = None;
    let shown = match current {
        Some(n) => n.clone(),
        None => format!("{} ({})", i18n::t("set.dev_default"), default_name),
    };
    ui.horizontal(|ui| {
        ui.allocate_ui_with_layout(
            egui::vec2(108.0, 22.0),
            egui::Layout::left_to_right(egui::Align::Center),
            |ui| {
                ui.label(egui::RichText::new(label).size(12.5).color(p.muted));
            },
        );
        egui::ComboBox::from_id_salt(id)
            .selected_text(egui::RichText::new(shown).size(12.5))
            .width((ui.available_width() - 8.0).clamp(160.0, 420.0))
            .show_ui(ui, |ui| {
                if ui
                    .selectable_label(
                        current.is_none(),
                        format!("{} ({})", i18n::t("set.dev_default"), default_name),
                    )
                    .on_hover_text(i18n::t("set.dev_default_tip"))
                    .clicked()
                {
                    picked = Some(None);
                }
                for name in list.iter() {
                    if ui
                        .selectable_label(current.as_deref() == Some(name.as_str()), name)
                        .clicked()
                    {
                        picked = Some(Some(name.clone()));
                    }
                }
            });
    });
    // A selected device that is no longer there must be visible.
    if let Some(n) = current {
        if !list.iter().any(|x| x == n) && !list.is_empty() {
            ui.label(
                egui::RichText::new(format!("{} - {}", n, i18n::t("set.dev_gone")))
                    .size(11.0)
                    .color(theme::palette().violet),
            );
        }
    }
    picked
}