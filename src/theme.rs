//! Colors, fonts and the small custom widgets that give Gaza Man its look.

use std::sync::Arc;

use eframe::egui::{
    self, Color32, CornerRadius, FontId, Margin, RichText, Sense, Stroke, Theme, Vec2,
};

pub const FLAG_BLACK: Color32 = Color32::from_rgb(0x14, 0x14, 0x14);
pub const FLAG_WHITE: Color32 = Color32::from_rgb(0xF5, 0xF5, 0xF5);
pub const FLAG_GREEN: Color32 = Color32::from_rgb(0x14, 0x99, 0x54);
pub const FLAG_RED: Color32 = Color32::from_rgb(0xE4, 0x31, 0x2B);

#[derive(Clone, Copy)]
pub struct Palette {
    pub dark: bool,
    /// Window background behind everything.
    pub bg: Color32,
    pub sidebar: Color32,
    /// Cards and the editor surface.
    pub surface: Color32,
    /// Inputs and code areas.
    pub input: Color32,
    pub hover: Color32,
    pub border: Color32,
    pub text: Color32,
    pub muted: Color32,
    pub faint: Color32,
    pub accent: Color32,
    pub accent_soft: Color32,
    pub red: Color32,
    pub amber: Color32,
    pub blue: Color32,
    pub purple: Color32,
    pub teal: Color32,
    pub pink: Color32,
}

pub fn palette(dark: bool) -> Palette {
    if dark {
        Palette {
            dark,
            bg: Color32::from_rgb(0x0C, 0x0E, 0x12),
            sidebar: Color32::from_rgb(0x11, 0x14, 0x19),
            surface: Color32::from_rgb(0x15, 0x18, 0x1F),
            input: Color32::from_rgb(0x1B, 0x1F, 0x27),
            hover: Color32::from_rgb(0x21, 0x26, 0x30),
            border: Color32::from_rgb(0x27, 0x2D, 0x38),
            text: Color32::from_rgb(0xE8, 0xEB, 0xF1),
            muted: Color32::from_rgb(0x8C, 0x95, 0xA6),
            faint: Color32::from_rgb(0x5A, 0x62, 0x70),
            accent: Color32::from_rgb(0x1E, 0xB8, 0x6E),
            accent_soft: Color32::from_rgba_unmultiplied(0x1E, 0xB8, 0x6E, 34),
            red: Color32::from_rgb(0xF2, 0x55, 0x5A),
            amber: Color32::from_rgb(0xF5, 0xA5, 0x24),
            blue: Color32::from_rgb(0x52, 0x94, 0xFF),
            purple: Color32::from_rgb(0xA9, 0x82, 0xF6),
            teal: Color32::from_rgb(0x2D, 0xD4, 0xBF),
            pink: Color32::from_rgb(0xEC, 0x5F, 0xA8),
        }
    } else {
        Palette {
            dark,
            bg: Color32::from_rgb(0xF3, 0xF4, 0xF7),
            sidebar: Color32::from_rgb(0xFA, 0xFA, 0xFC),
            surface: Color32::from_rgb(0xFF, 0xFF, 0xFF),
            input: Color32::from_rgb(0xF4, 0xF5, 0xF8),
            hover: Color32::from_rgb(0xEC, 0xEE, 0xF2),
            border: Color32::from_rgb(0xE1, 0xE4, 0xEA),
            text: Color32::from_rgb(0x17, 0x1A, 0x21),
            muted: Color32::from_rgb(0x64, 0x6B, 0x78),
            faint: Color32::from_rgb(0x9A, 0xA1, 0xAD),
            accent: Color32::from_rgb(0x12, 0x8F, 0x4E),
            accent_soft: Color32::from_rgba_unmultiplied(0x12, 0x8F, 0x4E, 26),
            red: Color32::from_rgb(0xD9, 0x32, 0x3A),
            amber: Color32::from_rgb(0xC9, 0x7A, 0x00),
            blue: Color32::from_rgb(0x2F, 0x6F, 0xE0),
            purple: Color32::from_rgb(0x7C, 0x4D, 0xDB),
            teal: Color32::from_rgb(0x0E, 0x9F, 0x8E),
            pink: Color32::from_rgb(0xC0, 0x37, 0x80),
        }
    }
}

pub fn pal(ui: &egui::Ui) -> Palette {
    palette(ui.visuals().dark_mode)
}

pub fn method_color(p: &Palette, method: &str) -> Color32 {
    match method {
        "GET" => p.accent,
        "POST" => p.amber,
        "PUT" => p.blue,
        "PATCH" => p.purple,
        "DELETE" => p.red,
        "HEAD" => p.teal,
        "OPTIONS" => p.pink,
        "WS" => p.teal,
        "SIO" => p.pink,
        _ => p.muted,
    }
}

/// What to show in place of the method: `WS` / `SIO` for socket requests.
pub fn request_label(r: &crate::collection::Request) -> &str {
    match r.kind {
        crate::collection::RequestKind::Http => &r.method,
        crate::collection::RequestKind::WebSocket => "WS",
        crate::collection::RequestKind::SocketIo => "SIO",
    }
}

/// Short method names for the narrow sidebar column, like Postman's `DEL`.
pub fn method_short(method: &str) -> &str {
    match method {
        "DELETE" => "DEL",
        "OPTIONS" => "OPT",
        "PATCH" => "PATCH",
        m => m,
    }
}

pub fn status_color(p: &Palette, status: Option<u16>) -> Color32 {
    match status {
        Some(100..=199) => p.blue,
        Some(200..=299) => p.accent,
        Some(300..=399) => p.blue,
        Some(400..=499) => p.amber,
        _ => p.red,
    }
}

fn visuals(dark: bool) -> egui::Visuals {
    let p = palette(dark);
    let mut v = if dark {
        egui::Visuals::dark()
    } else {
        egui::Visuals::light()
    };
    let radius = CornerRadius::same(6);
    v.panel_fill = p.bg;
    v.window_fill = p.surface;
    v.window_stroke = Stroke::new(1.0, p.border);
    v.window_corner_radius = CornerRadius::same(14);
    v.menu_corner_radius = CornerRadius::same(10);
    v.window_shadow = egui::epaint::Shadow {
        offset: [0, 14],
        blur: 40,
        spread: 0,
        color: Color32::from_black_alpha(if dark { 150 } else { 45 }),
    };
    v.popup_shadow = egui::epaint::Shadow {
        offset: [0, 8],
        blur: 24,
        spread: 0,
        color: Color32::from_black_alpha(if dark { 130 } else { 35 }),
    };
    v.extreme_bg_color = p.input;
    v.text_edit_bg_color = Some(p.input);
    v.code_bg_color = p.input;
    v.faint_bg_color = if dark {
        Color32::from_rgb(0x18, 0x1C, 0x23)
    } else {
        Color32::from_rgb(0xF7, 0xF8, 0xFA)
    };
    v.hyperlink_color = p.accent;
    v.warn_fg_color = p.amber;
    v.error_fg_color = p.red;
    v.selection.bg_fill = if dark {
        Color32::from_rgba_unmultiplied(0x1E, 0xB8, 0x6E, 70)
    } else {
        Color32::from_rgba_unmultiplied(0x12, 0x8F, 0x4E, 50)
    };
    v.selection.stroke = Stroke::new(1.0, p.accent);
    v.override_text_color = None;
    v.collapsing_header_frame = false;
    v.indent_has_left_vline = false;
    v.striped = true;

    let w = &mut v.widgets;
    w.noninteractive.bg_fill = p.surface;
    w.noninteractive.weak_bg_fill = p.surface;
    w.noninteractive.bg_stroke = Stroke::new(1.0, p.border);
    w.noninteractive.fg_stroke = Stroke::new(1.0, p.text);
    w.noninteractive.corner_radius = radius;

    w.inactive.bg_fill = p.input;
    w.inactive.weak_bg_fill = p.input;
    w.inactive.bg_stroke = Stroke::new(1.0, p.border);
    w.inactive.fg_stroke = Stroke::new(1.0, p.text);
    w.inactive.corner_radius = radius;

    w.hovered.bg_fill = p.hover;
    w.hovered.weak_bg_fill = p.hover;
    w.hovered.bg_stroke = Stroke::new(
        1.0,
        if dark {
            Color32::from_rgb(0x3A, 0x42, 0x50)
        } else {
            Color32::from_rgb(0xC9, 0xCE, 0xD6)
        },
    );
    w.hovered.fg_stroke = Stroke::new(1.5, p.text);
    w.hovered.corner_radius = radius;
    w.hovered.expansion = 0.0;

    w.active.bg_fill = p.hover;
    w.active.weak_bg_fill = p.hover;
    w.active.bg_stroke = Stroke::new(1.0, p.accent);
    w.active.fg_stroke = Stroke::new(1.5, p.text);
    w.active.corner_radius = radius;
    w.active.expansion = 0.0;

    w.open.bg_fill = p.hover;
    w.open.weak_bg_fill = p.hover;
    w.open.bg_stroke = Stroke::new(1.0, p.border);
    w.open.fg_stroke = Stroke::new(1.0, p.text);
    w.open.corner_radius = radius;
    v
}

pub fn install(ctx: &egui::Context, dark: bool) {
    install_fonts(ctx);
    ctx.set_visuals_of(Theme::Dark, visuals(true));
    ctx.set_visuals_of(Theme::Light, visuals(false));
    ctx.set_theme(if dark { Theme::Dark } else { Theme::Light });
    ctx.all_styles_mut(|style| {
        use egui::TextStyle;
        style.text_styles = [
            (TextStyle::Heading, FontId::proportional(19.0)),
            (TextStyle::Body, FontId::proportional(13.5)),
            (TextStyle::Button, FontId::proportional(13.5)),
            (TextStyle::Small, FontId::proportional(11.0)),
            (TextStyle::Monospace, FontId::monospace(12.5)),
        ]
        .into();
        style.spacing.item_spacing = egui::vec2(8.0, 6.0);
        style.spacing.button_padding = egui::vec2(10.0, 5.0);
        style.spacing.interact_size.y = 26.0;
        style.spacing.menu_margin = Margin::same(6);
        style.spacing.window_margin = Margin::same(18);
        style.spacing.combo_width = 120.0;
        style.spacing.scroll.bar_width = 6.0;
        style.spacing.scroll.floating = true;
        style.interaction.selectable_labels = false;
    });
}

pub fn set_dark(ctx: &egui::Context, dark: bool) {
    ctx.set_theme(if dark { Theme::Dark } else { Theme::Light });
}

fn install_fonts(ctx: &egui::Context) {
    let mut fonts = egui::FontDefinitions::default();
    let mut add = |name: &str, paths: &[&str], family: egui::FontFamily, first: bool| {
        let Some(bytes) = paths.iter().find_map(|p| std::fs::read(p).ok()) else {
            return;
        };
        fonts
            .font_data
            .insert(name.into(), Arc::new(egui::FontData::from_owned(bytes)));
        let list = fonts.families.entry(family).or_default();
        if first {
            list.insert(0, name.into())
        } else {
            list.push(name.into())
        }
    };
    // The system UI fonts look much closer to a native app than egui's defaults.
    add(
        "ui",
        &[
            "/System/Library/Fonts/SFNS.ttf",
            "C:\\Windows\\Fonts\\segoeui.ttf",
        ],
        egui::FontFamily::Proportional,
        true,
    );
    add(
        "mono",
        &[
            "/System/Library/Fonts/SFNSMono.ttf",
            "C:\\Windows\\Fonts\\consola.ttf",
        ],
        egui::FontFamily::Monospace,
        true,
    );
    // egui's built-in fonts have no Arabic glyphs; borrow a system font as a fallback.
    const ARABIC: [&str; 6] = [
        "/System/Library/Fonts/SFArabic.ttf",
        "/System/Library/Fonts/Supplemental/Arial Unicode.ttf",
        "C:\\Windows\\Fonts\\arial.ttf",
        "/usr/share/fonts/truetype/noto/NotoSansArabic-Regular.ttf",
        "/usr/share/fonts/noto/NotoSansArabic-Regular.ttf",
        "/usr/share/fonts/truetype/dejavu/DejaVuSans.ttf",
    ];
    add("arabic", &ARABIC, egui::FontFamily::Proportional, false);
    add("arabic", &ARABIC, egui::FontFamily::Monospace, false);
    // Phosphor icons (MIT), shipped inside the app. Its glyphs live in the Private Use Area,
    // so they never replace real letters; see `icons.rs` for the names.
    fonts.font_data.insert(
        "icons".into(),
        Arc::new(egui::FontData::from_static(include_bytes!("../assets/fonts/Phosphor.ttf")).tweak(egui::FontTweak {
            y_offset_factor: 0.08,
            ..Default::default()
        })),
    );
    for family in [egui::FontFamily::Proportional, egui::FontFamily::Monospace] {
        fonts.families.entry(family).or_default().insert(1, "icons".into());
    }
    ctx.set_fonts(fonts);
}

// ---------------------------------------------------------------------------------------------
// Widgets
// ---------------------------------------------------------------------------------------------

pub fn draw_flag(ui: &mut egui::Ui, size: Vec2) {
    let (rect, _) = ui.allocate_exact_size(size, Sense::hover());
    paint_flag(ui.painter(), rect);
}

pub fn paint_flag(p: &egui::Painter, rect: egui::Rect) {
    let h = rect.height() / 3.0;
    let r = (rect.height() * 0.12).round() as u8;
    let painter = p.with_clip_rect(rect);
    painter.rect_filled(rect, r, FLAG_GREEN);
    painter.rect_filled(
        egui::Rect::from_min_max(rect.min, egui::pos2(rect.right(), rect.top() + 2.0 * h)),
        CornerRadius {
            nw: r,
            ne: r,
            sw: 0,
            se: 0,
        },
        FLAG_WHITE,
    );
    painter.rect_filled(
        egui::Rect::from_min_max(rect.min, egui::pos2(rect.right(), rect.top() + h)),
        CornerRadius {
            nw: r,
            ne: r,
            sw: 0,
            se: 0,
        },
        FLAG_BLACK,
    );
    let tri = vec![
        rect.left_top(),
        egui::pos2(rect.left() + rect.width() * 0.42, rect.center().y),
        rect.left_bottom(),
    ];
    painter.add(egui::Shape::convex_polygon(tri, FLAG_RED, Stroke::NONE));
}

/// A rounded, filled badge such as "201 Created".
pub fn badge(
    ui: &mut egui::Ui,
    text: impl Into<String>,
    fg: Color32,
    bg: Color32,
    size: f32,
) -> egui::Response {
    egui::Frame::new()
        .fill(bg)
        .corner_radius(5)
        .inner_margin(Margin::symmetric(7, 2))
        .show(ui, |ui| {
            ui.label(RichText::new(text.into()).size(size).strong().color(fg));
        })
        .response
}

/// A soft badge: tinted background in the same hue as the text.
pub fn soft_badge(
    ui: &mut egui::Ui,
    text: impl Into<String>,
    color: Color32,
    size: f32,
) -> egui::Response {
    let bg = Color32::from_rgba_unmultiplied(color.r(), color.g(), color.b(), 34);
    badge(ui, text, color, bg, size)
}

pub fn method_text(p: &Palette, method: &str, size: f32) -> RichText {
    RichText::new(method)
        .monospace()
        .size(size)
        .strong()
        .color(method_color(p, method))
}

/// A filled call-to-action button in the accent color.
pub fn primary(text: &str) -> egui::Button<'static> {
    egui::Button::new(
        RichText::new(text.to_string())
            .strong()
            .color(Color32::WHITE),
    )
    .fill(FLAG_GREEN)
    .stroke(Stroke::NONE)
    .corner_radius(7)
}

/// A frameless button that only shows a background on hover.
pub fn ghost(ui: &mut egui::Ui, text: impl Into<egui::WidgetText>) -> egui::Response {
    ui.add(
        egui::Button::new(text)
            .frame_when_inactive(false)
            .corner_radius(6),
    )
}

pub fn icon_button(ui: &mut egui::Ui, icon: &str, tooltip: &str) -> egui::Response {
    let p = pal(ui);
    ui.add(
        egui::Button::new(RichText::new(icon).size(14.0).color(p.muted))
            .frame_when_inactive(false)
            .corner_radius(6)
            .min_size(egui::vec2(26.0, 26.0)),
    )
    .on_hover_text(tooltip)
}

/// A row of underlined tabs, Postman style. `items` are `(value, label, count)`.
pub fn tab_strip<T: PartialEq + Copy>(
    ui: &mut egui::Ui,
    current: &mut T,
    items: &[(T, &str, Option<String>)],
) -> bool {
    let p = pal(ui);
    let mut changed = false;
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = 18.0;
        for (value, label, count) in items {
            let selected = *current == *value;
            let color = if selected { p.text } else { p.muted };
            let mut job = egui::text::LayoutJob::default();
            job.append(
                label,
                0.0,
                egui::TextFormat::simple(FontId::proportional(13.0), color),
            );
            if let Some(c) = count {
                job.append(
                    &format!(" {c}"),
                    0.0,
                    egui::TextFormat::simple(
                        FontId::proportional(11.5),
                        if selected { p.accent } else { p.faint },
                    ),
                );
            }
            let galley = ui.fonts_mut(|f| f.layout_job(job));
            let (rect, resp) =
                ui.allocate_exact_size(egui::vec2(galley.size().x, 28.0), Sense::click());
            let resp = resp.on_hover_cursor(egui::CursorIcon::PointingHand);
            ui.painter().galley(
                egui::pos2(rect.left(), rect.center().y - galley.size().y / 2.0 - 2.0),
                galley,
                color,
            );
            if selected {
                let line = egui::Rect::from_min_max(
                    egui::pos2(rect.left(), rect.bottom() - 2.0),
                    rect.right_bottom(),
                );
                ui.painter().rect_filled(line, 1, p.accent);
            } else if resp.hovered() {
                let line = egui::Rect::from_min_max(
                    egui::pos2(rect.left(), rect.bottom() - 2.0),
                    rect.right_bottom(),
                );
                ui.painter().rect_filled(line, 1, p.border);
            }
            if resp.clicked() && !selected {
                *current = *value;
                changed = true;
            }
        }
    });
    changed
}

/// A small segmented control, e.g. Pretty | Raw | Tree.
pub fn segmented<T: PartialEq + Copy>(ui: &mut egui::Ui, current: &mut T, items: &[(T, &str)]) {
    let p = pal(ui);
    egui::Frame::new()
        .fill(p.input)
        .corner_radius(7)
        .inner_margin(Margin::same(2))
        .show(ui, |ui| {
            ui.spacing_mut().item_spacing.x = 2.0;
            for (value, label) in items {
                let selected = *current == *value;
                let text =
                    RichText::new(*label)
                        .size(12.0)
                        .color(if selected { p.text } else { p.muted });
                let b = egui::Button::new(text)
                    .fill(if selected {
                        p.hover
                    } else {
                        Color32::TRANSPARENT
                    })
                    .stroke(Stroke::NONE)
                    .corner_radius(5)
                    .min_size(egui::vec2(0.0, 22.0));
                if ui.add(b).clicked() {
                    *current = *value;
                }
            }
        });
}

pub fn section_title(ui: &mut egui::Ui, text: &str) {
    let p = pal(ui);
    ui.label(
        RichText::new(text.to_uppercase())
            .size(10.5)
            .strong()
            .color(p.faint),
    );
}

pub fn card<R>(ui: &mut egui::Ui, add: impl FnOnce(&mut egui::Ui) -> R) -> R {
    let p = pal(ui);
    egui::Frame::new()
        .fill(p.surface)
        .stroke(Stroke::new(1.0, p.border))
        .corner_radius(10)
        .inner_margin(Margin::same(14))
        .show(ui, add)
        .inner
}

/// A centered, friendly placeholder for empty areas.
pub fn empty_state(ui: &mut egui::Ui, icon: &str, title: &str, subtitle: &str) {
    let p = pal(ui);
    ui.vertical_centered(|ui| {
        ui.add_space((ui.available_height() * 0.28).max(12.0));
        ui.label(RichText::new(icon).size(34.0).color(p.faint));
        ui.add_space(6.0);
        ui.label(RichText::new(title).size(15.0).strong().color(p.text));
        ui.add(egui::Label::new(RichText::new(subtitle).color(p.muted)).wrap());
    });
}

/// A keyboard shortcut written for this platform.
pub fn shortcut(keys: &str) -> String {
    if cfg!(target_os = "macos") {
        keys.replace("Ctrl", "⌘")
            .replace("Shift", "⇧")
            .replace("Alt", "⌥")
            .replace('+', "")
    } else {
        keys.to_string()
    }
}

pub fn format_size(n: usize) -> String {
    match n {
        0..1024 => format!("{n} B"),
        1024..1_048_576 => format!("{:.1} KB", n as f32 / 1024.0),
        _ => format!("{:.2} MB", n as f32 / 1_048_576.0),
    }
}

/// An on/off switch with a label, clearer than a checkbox on dark backgrounds.
pub fn toggle(ui: &mut egui::Ui, on: &mut bool, label: &str, help: &str) -> egui::Response {
    let p = pal(ui);
    let resp = ui
        .horizontal(|ui| {
            let size = egui::vec2(30.0, 17.0);
            let (rect, mut resp) = ui.allocate_exact_size(size, egui::Sense::click());
            if resp.clicked() {
                *on = !*on;
                resp.mark_changed();
            }
            let t = ui.ctx().animate_bool_responsive(resp.id, *on);
            let track = if *on { p.accent } else { p.border };
            ui.painter().rect_filled(rect, 8.5, track);
            let x = egui::lerp(rect.left() + 8.5..=rect.right() - 8.5, t);
            ui.painter().circle_filled(egui::pos2(x, rect.center().y), 6.5, Color32::WHITE);
            let text = ui.add(egui::Label::new(RichText::new(label).color(p.text)).sense(egui::Sense::click()));
            if text.clicked() {
                *on = !*on;
                resp.mark_changed();
            }
            resp
        })
        .inner;
    resp.on_hover_text(help)
}
