//! Farben (hell/dunkel, Akzent aus den Einstellungen), Leisten-Stil und Icon-Texturen.

use crate::settings::{Settings, Theme};
use egui::{Color32, CornerRadius, Frame, Margin, Response, Sense, Shadow, Stroke, TextureHandle, Ui, Vec2};
use owndraw_core::icons;
use std::cell::Cell;
use std::collections::HashMap;

/// Farbpalette der Oberfläche (alle Werte 0xAARRGGBB).
#[derive(Clone, Copy, Debug)]
pub struct Pal {
    pub dark: bool,
    pub bg: u32,
    pub card: u32,
    pub pill: u32,
    pub line: u32,
    pub text: u32,
    pub dim: u32,
    pub accent: u32,
    pub active: u32,
    pub accent_soft: u32,
    pub hover: u32,
    pub press: u32,
    pub danger: u32,
    pub danger_text: u32,
    pub disabled: u32,
    /// Textfarbe auf Akzentflächen.
    pub on_accent: u32,
    /// Vertiefte Fläche (Vorschaubilder, Eingabefelder) im modernen Design.
    pub sunken: u32,
}

fn with_alpha(argb: u32, a: u8) -> u32 {
    (argb & 0x00FF_FFFF) | ((a as u32) << 24)
}

fn luminance(argb: u32) -> f32 {
    let (r, g, b) = (((argb >> 16) & 255) as f32, ((argb >> 8) & 255) as f32, (argb & 255) as f32);
    (0.299 * r + 0.587 * g + 0.114 * b) / 255.0
}

pub fn palette(dark: bool, accent: u32) -> Pal {
    if modern() {
        palette_modern(dark, accent)
    } else {
        palette_classic(dark, accent)
    }
}

/// Palette des modernen Designs: ruhigere Neutraltöne mit leichtem Blaustich, deckende Linien.
pub fn palette_modern(dark: bool, accent: u32) -> Pal {
    let accent = with_alpha(accent, 0xFF);
    let on_accent = if luminance(accent) > 0.55 { 0xFF111116 } else { 0xFFFFFFFF };
    if dark {
        Pal {
            dark,
            bg: 0xFF111114,
            card: 0xFF18181C,
            pill: 0xFF1F1F24,
            line: 0xFF2A2A31,
            text: 0xFFECECF1,
            dim: 0xFF9A9AA8,
            accent,
            active: with_alpha(accent, 0x33),
            accent_soft: with_alpha(accent, 0x1C),
            hover: 0x14FFFFFF,
            press: 0x22FFFFFF,
            danger: 0xFFF2555A,
            danger_text: 0xFFFF8484,
            disabled: 0xFF55555F,
            on_accent,
            sunken: 0xFF0C0C0F,
        }
    } else {
        Pal {
            dark,
            bg: 0xFFF6F6F8,
            card: 0xFFFFFFFF,
            pill: 0xFFFFFFFF,
            line: 0xFFE2E2E8,
            text: 0xFF17171C,
            dim: 0xFF6B6B78,
            accent,
            active: with_alpha(accent, 0x2E),
            accent_soft: with_alpha(accent, 0x1A),
            hover: 0x0F000000,
            press: 0x1C000000,
            danger: 0xFFD93F3F,
            danger_text: 0xFFC22F2F,
            disabled: 0xFFB0B0BA,
            on_accent,
            sunken: 0xFFEEEEF2,
        }
    }
}

/// Palette des klassischen Designs (bis 4.8).
pub fn palette_classic(dark: bool, accent: u32) -> Pal {
    let accent = with_alpha(accent, 0xFF);
    let on_accent = if luminance(accent) > 0.55 { 0xFF16171B } else { 0xFFFFFFFF };
    if dark {
        Pal {
            dark,
            bg: 0xFF101114,
            card: 0xFF1C1D22,
            pill: 0xF51E1F25,
            line: 0x24FFFFFF,
            text: 0xFFE9EAEE,
            dim: 0xFF8B8D98,
            accent,
            active: with_alpha(accent, 0x3D),
            accent_soft: with_alpha(accent, 0x1F),
            hover: 0x17FFFFFF,
            press: 0x26FFFFFF,
            danger: 0xFFFF6B6B,
            danger_text: 0xFFFF8A8A,
            disabled: 0xFF55575F,
            on_accent,
            sunken: 0xFF101114,
        }
    } else {
        Pal {
            dark,
            bg: 0xFFEDEDF0,
            card: 0xFFFFFFFF,
            pill: 0xF8FFFFFF,
            line: 0x26000000,
            text: 0xFF1F2025,
            dim: 0xFF666875,
            accent,
            active: with_alpha(accent, 0x38),
            accent_soft: with_alpha(accent, 0x22),
            hover: 0x12000000,
            press: 0x20000000,
            danger: 0xFFD93F3F,
            danger_text: 0xFFC22F2F,
            disabled: 0xFFB0B2BB,
            on_accent,
            sunken: 0xFFEDEDF0,
        }
    }
}

thread_local! {
    /// Modernes Design aktiv (Einstellung „design“, Vorgabe modern).
    static MODERN: Cell<bool> = const { Cell::new(true) };
    static PAL: Cell<Pal> = Cell::new(palette(true, 0xFF7C8CFF));
    /// Abrundung der Kacheln/Leisten in px (Einstellungen, Standard 26).
    static TILE_R: Cell<f32> = const { Cell::new(TILE_RADIUS_DEFAULT) };
}

/// Entwurfswert der Leisten-Abrundung (klassisches Design, entspricht 4.1); `pill(r)` skaliert relativ dazu.
pub const TILE_RADIUS_DEFAULT: f32 = 26.0;
/// Standard-Abrundung der Leisten im modernen Design.
pub const TILE_RADIUS_MODERN: f32 = 12.0;

/// Ist im Einstellungen-Objekt das moderne Design gewählt? (Vorgabe: ja)
pub fn is_modern(s: &Settings) -> bool {
    s.extra.get("design").and_then(|v| v.as_str()) != Some("classic")
}

/// Design umschalten (schreibt `extra["design"]`).
pub fn set_modern(s: &mut Settings, on: bool) {
    s.extra.insert("design".into(), serde_json::json!(if on { "modern" } else { "classic" }));
}

/// Aktuell aktives Design (gesetzt von `apply`).
pub fn modern() -> bool {
    MODERN.with(|m| m.get())
}

/// Schlüssel der Abrundung je Design (jedes Design merkt sich seinen eigenen Wert).
fn tile_radius_key(s: &Settings) -> &'static str {
    if is_modern(s) { "tile_radius_modern" } else { "tile_radius" }
}

/// Standard-Abrundung der Kacheln im gewählten Design.
pub fn tile_radius_default(s: &Settings) -> f32 {
    if is_modern(s) { TILE_RADIUS_MODERN } else { TILE_RADIUS_DEFAULT }
}

/// Eingestellte Abrundung der Kacheln aus den Einstellungen (0–40 px).
pub fn tile_radius_setting(s: &Settings) -> f32 {
    s.extra.get(tile_radius_key(s)).and_then(|v| v.as_f64()).map(|v| v as f32).unwrap_or(tile_radius_default(s)).clamp(0.0, 40.0)
}

/// Abrundung der Kacheln setzen (für das gewählte Design).
pub fn set_tile_radius(s: &mut Settings, v: f32) {
    let k = tile_radius_key(s);
    s.extra.insert(k.into(), serde_json::json!(v));
}

/// Abrundung von Knöpfen und Eingabefeldern.
pub fn btn_r() -> u8 {
    if modern() { 8 } else { 14 }
}

/// Schatten schwebender Leisten.
fn float_shadow(dark: bool) -> Shadow {
    if modern() {
        Shadow { offset: [0, 4], blur: 18, spread: 0, color: Color32::from_black_alpha(if dark { 110 } else { 22 }) }
    } else {
        Shadow { offset: [0, 3], blur: 14, spread: 0, color: Color32::from_black_alpha(if dark { 90 } else { 45 }) }
    }
}

/// Aktuelle Palette (gesetzt von `apply`).
pub fn pal() -> Pal {
    PAL.with(|p| p.get())
}

/// 0xAARRGGBB nach egui-Farbe.
pub fn c(argb: u32) -> Color32 {
    Color32::from_rgba_unmultiplied((argb >> 16) as u8, (argb >> 8) as u8, argb as u8, (argb >> 24) as u8)
}

/// Kachel/Leiste; `radius` ist der Entwurfswert bei Standard-Abrundung, er wird nach der Einstellung skaliert.
pub fn pill(radius: u8) -> Frame {
    let p = pal();
    let radius = (radius as f32 * TILE_R.with(|r| r.get()) / TILE_RADIUS_DEFAULT).round().clamp(0.0, 255.0) as u8;
    Frame::new()
        .fill(c(p.pill))
        .stroke(Stroke::new(1.0, c(p.line)))
        .corner_radius(CornerRadius::same(radius))
        .inner_margin(Margin::same(4))
        .shadow(float_shadow(p.dark))
}

pub fn card() -> Frame {
    let p = pal();
    if modern() {
        return Frame::new()
            .fill(c(p.card))
            .stroke(Stroke::new(1.0, c(p.line)))
            .corner_radius(CornerRadius::same(12))
            .inner_margin(Margin { left: 18, right: 18, top: 14, bottom: 12 });
    }
    Frame::new()
        .fill(c(p.card))
        .stroke(Stroke::new(1.0, c(p.line)))
        .corner_radius(CornerRadius::same(14))
        .inner_margin(Margin { left: 20, right: 20, top: 18, bottom: 14 })
        .shadow(Shadow { offset: [0, 4], blur: 20, spread: 0, color: Color32::from_black_alpha(if p.dark { 120 } else { 60 }) })
}

/// Fläche für Bottom-Sheets (oben abgerundet) und angedockte Panels.
pub fn sheet(top_radius: u8) -> Frame {
    let p = pal();
    Frame::new()
        .fill(c(p.card))
        .stroke(Stroke::new(1.0, c(p.line)))
        .corner_radius(CornerRadius { nw: top_radius, ne: top_radius, sw: 0, se: 0 })
        .inner_margin(Margin { left: 16, right: 16, top: 8, bottom: 12 })
        .shadow(Shadow { offset: [0, -2], blur: 18, spread: 0, color: Color32::from_black_alpha(if p.dark { 110 } else { 50 }) })
}

/// Wendet Theme, UI-Größe und Akzentfarbe aus den Einstellungen an.
pub fn apply(ctx: &egui::Context, s: &Settings) {
    ctx.set_theme(match s.theme {
        Theme::System => egui::ThemePreference::System,
        Theme::Light => egui::ThemePreference::Light,
        Theme::Dark => egui::ThemePreference::Dark,
    });
    MODERN.with(|m| m.set(is_modern(s)));
    let dark = ctx.theme() == egui::Theme::Dark;
    let p = palette(dark, s.accent);
    PAL.with(|x| x.set(p));
    TILE_R.with(|r| r.set(tile_radius_setting(s)));
    ctx.set_zoom_factor((s.ui_scale.clamp(75, 200) as f32) / 100.0);
    for theme in [egui::Theme::Dark, egui::Theme::Light] {
        let d = theme == egui::Theme::Dark;
        let q = palette(d, s.accent);
        let mut v = if d { egui::Visuals::dark() } else { egui::Visuals::light() };
        v.panel_fill = c(q.bg);
        v.window_fill = c(q.card);
        v.window_stroke = Stroke::new(1.0, c(q.line));
        let m = is_modern(s);
        v.window_corner_radius = CornerRadius::same(if m { 12 } else { 14 });
        v.menu_corner_radius = CornerRadius::same(if m { 8 } else { 10 });
        if m {
            v.window_shadow = Shadow { offset: [0, 8], blur: 28, spread: 0, color: Color32::from_black_alpha(if d { 140 } else { 40 }) };
            v.popup_shadow = Shadow { offset: [0, 4], blur: 16, spread: 0, color: Color32::from_black_alpha(if d { 120 } else { 30 }) };
            for w in [&mut v.widgets.inactive, &mut v.widgets.hovered, &mut v.widgets.active, &mut v.widgets.open, &mut v.widgets.noninteractive] {
                w.corner_radius = CornerRadius::same(8);
            }
            v.widgets.inactive.weak_bg_fill = c(q.hover);
            v.widgets.inactive.bg_fill = c(q.line);
            v.widgets.noninteractive.bg_stroke = Stroke::new(1.0, c(q.line));
            // Eingabefelder heben sich vom Karten-/Dialoghintergrund ab (vorher gleiche Farbe: Feld unsichtbar)
            v.text_edit_bg_color = Some(c(q.bg));
        }
        v.extreme_bg_color = c(q.bg);
        v.selection.bg_fill = c(with_alpha(q.accent, 0x66));
        v.selection.stroke = Stroke::new(1.0, c(q.accent));
        v.hyperlink_color = c(q.accent);
        v.override_text_color = Some(c(q.text));
        v.widgets.hovered.weak_bg_fill = c(with_alpha(q.accent, 0x33));
        v.widgets.active.weak_bg_fill = c(with_alpha(q.accent, 0x4D));
        ctx.set_visuals_of(theme, v);
    }
    let m = is_modern(s);
    ctx.all_styles_mut(|st| {
        st.spacing.item_spacing = if m { Vec2::new(8.0, 6.0) } else { Vec2::new(6.0, 6.0) };
        st.spacing.button_padding = Vec2::new(10.0, 6.0);
        st.spacing.interact_size.y = 30.0;
        st.text_styles.insert(egui::TextStyle::Body, egui::FontId::proportional(14.5));
        st.text_styles.insert(egui::TextStyle::Button, egui::FontId::proportional(14.0));
        st.text_styles.insert(egui::TextStyle::Heading, egui::FontId::proportional(if m { 18.0 } else { 17.0 }));
    });
    ctx.options_mut(|o| o.zoom_with_keyboard = false);
}

/// Zwischenspeicher für gerenderte Icons (id, Farbe, gefüllt, Pixelgröße).
#[derive(Default)]
pub struct Icons {
    map: HashMap<(i32, u32, bool, u32), TextureHandle>,
}

impl Icons {
    pub fn get(&mut self, ctx: &egui::Context, id: i32, tint: u32, filled: bool, size_pt: f32) -> Option<TextureHandle> {
        let px = (size_pt * ctx.pixels_per_point()).round().max(8.0) as u32;
        let key = (id, tint, filled, px);
        if let Some(t) = self.map.get(&key) {
            return Some(t.clone());
        }
        let pm = icons::render(id, px, tint, filled)?;
        let img = egui::ColorImage::from_rgba_premultiplied([px as usize, px as usize], pm.data());
        let t = ctx.load_texture(format!("ic{id}-{tint:x}-{filled}-{px}"), img, egui::TextureOptions::LINEAR);
        self.map.insert(key, t.clone());
        Some(t)
    }

    /// Zeichnet ein Icon mittig in `rect`.
    pub fn paint(&mut self, ui: &Ui, rect: egui::Rect, id: i32, tint: u32, filled: bool, size: f32) {
        if let Some(t) = self.get(ui.ctx(), id, tint, filled, size) {
            let r = egui::Rect::from_center_size(rect.center(), Vec2::splat(size));
            ui.painter().image(t.id(), r, egui::Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(1.0, 1.0)), Color32::WHITE);
        }
    }
}

/// Hintergrund eines runden Knopfes je nach Zustand.
pub fn btn_bg(ui: &Ui, rect: egui::Rect, resp: &Response, active: bool) {
    let p = pal();
    let bg = if active {
        Some(p.active)
    } else if resp.is_pointer_button_down_on() {
        Some(p.press)
    } else if resp.hovered() {
        Some(p.hover)
    } else {
        None
    };
    if let Some(b) = bg {
        let r = if modern() { CornerRadius::same(8) } else { CornerRadius::same(255) };
        ui.painter().rect_filled(rect.shrink(2.0), r, c(b));
    }
}

/// Runder Icon-Knopf mit Kantenlänge `size`.
#[allow(clippy::too_many_arguments)]
pub fn icon_btn(ui: &mut Ui, ic: &mut Icons, id: i32, tint: u32, active: bool, filled: bool, size: f32, tip: &str) -> Response {
    let (rect, resp) = ui.allocate_exact_size(Vec2::splat(size), Sense::click());
    btn_bg(ui, rect, &resp, active);
    ic.paint(ui, rect, id, tint, filled, (size * 0.55).round());
    if !tip.is_empty() {
        resp.on_hover_text(tip)
    } else {
        resp
    }
}

/// Textknopf mit Akzentfarbe.
pub fn accent_btn(ui: &mut Ui, label: &str, min_h: f32) -> Response {
    let p = pal();
    let b = egui::Button::new(egui::RichText::new(label).color(c(p.accent)).strong())
        .fill(c(p.accent_soft))
        .stroke(Stroke::NONE)
        .min_size(Vec2::new(0.0, min_h))
        .corner_radius(CornerRadius::same(btn_r()));
    ui.add(b)
}

/// Gefüllter Hauptknopf.
pub fn primary_btn(ui: &mut Ui, label: &str, min_h: f32) -> Response {
    let p = pal();
    let b = egui::Button::new(egui::RichText::new(label).color(c(p.on_accent)).strong())
        .fill(c(p.accent))
        .min_size(Vec2::new(0.0, min_h))
        .corner_radius(CornerRadius::same(btn_r()));
    ui.add(b)
}

pub fn plain_btn(ui: &mut Ui, label: &str, min_h: f32) -> Response {
    ui.add(egui::Button::new(label).fill(Color32::TRANSPARENT).min_size(Vec2::new(0.0, min_h)).corner_radius(CornerRadius::same(btn_r())))
}

pub fn danger_btn(ui: &mut Ui, label: &str, min_h: f32) -> Response {
    let p = pal();
    let b = egui::Button::new(egui::RichText::new(label).color(c(p.danger_text)).strong())
        .fill(c(with_alpha(p.danger, 0x33)))
        .min_size(Vec2::new(0.0, min_h))
        .corner_radius(CornerRadius::same(btn_r()));
    ui.add(b)
}

/// Auswahl-Chip (Segment) mit Text; `selected` hebt ihn hervor.
pub fn chip(ui: &mut Ui, label: &str, selected: bool, min_h: f32) -> Response {
    let p = pal();
    let t = egui::RichText::new(label).color(c(if selected { p.accent } else { p.text }));
    let t = if selected { t.strong() } else { t };
    let b = egui::Button::new(t)
        .fill(c(if selected { p.active } else { p.hover }))
        .stroke(if selected { Stroke::new(1.0, c(p.accent)) } else { Stroke::NONE })
        .min_size(Vec2::new(0.0, min_h))
        .corner_radius(CornerRadius::same(if modern() { 8 } else { 255 }));
    ui.add(b)
}

/// Knopf mit Rahmen (Nebenaktion im modernen Design).
pub fn outline_btn(ui: &mut Ui, label: &str, min_h: f32) -> Response {
    let p = pal();
    let b = egui::Button::new(egui::RichText::new(label).color(c(p.text)))
        .fill(c(p.card))
        .stroke(Stroke::new(1.0, c(p.line)))
        .min_size(Vec2::new(0.0, min_h))
        .corner_radius(CornerRadius::same(btn_r()));
    ui.add(b)
}

/// Menüzeile über die volle Breite; schließt ein umgebendes Menü nicht selbst.
pub fn menu_row(ui: &mut Ui, label: &str, danger: bool, min_h: f32) -> bool {
    let p = pal();
    let txt = if danger { egui::RichText::new(label).color(c(p.danger_text)) } else { egui::RichText::new(label) };
    let w = ui.available_width();
    let r = ui.add(egui::Button::new(txt).fill(Color32::TRANSPARENT).min_size(Vec2::new(w, min_h)).corner_radius(CornerRadius::same(10)));
    r.clicked()
}

/// Eintrag eines kompakten Kontextmenüs (Board-Karten).
pub struct MenuItem<'a> {
    pub icon: i32,
    pub label: &'a str,
    /// Rot und durch eine Linie von den übrigen Einträgen getrennt.
    pub danger: bool,
}

/// Kompaktes Kontextmenü mit fester Breite: Icon + Text je Zeile, Trennlinie vor dem ersten roten Eintrag.
/// Liefert den Index des angeklickten Eintrags (das umgebende Popup schließt sich dann).
pub fn card_menu(ui: &mut Ui, ic: &mut Icons, items: &[MenuItem]) -> Option<usize> {
    let p = pal();
    const W: f32 = 196.0;
    const H: f32 = 34.0;
    ui.set_width(W);
    ui.spacing_mut().item_spacing = Vec2::new(0.0, 2.0);
    let mut hit = None;
    let mut sep_done = false;
    for (i, it) in items.iter().enumerate() {
        if it.danger && !sep_done && i > 0 {
            let (r, _) = ui.allocate_exact_size(Vec2::new(W, 9.0), Sense::hover());
            ui.painter().line_segment([egui::pos2(r.left() + 6.0, r.center().y), egui::pos2(r.right() - 6.0, r.center().y)], Stroke::new(1.0, c(p.line)));
            sep_done = true;
        }
        let (r, resp) = ui.allocate_exact_size(Vec2::new(W, H), Sense::click());
        let col = if it.danger { p.danger_text } else { p.text };
        if resp.hovered() || resp.has_focus() {
            let bg = if it.danger { with_alpha(p.danger, 0x22) } else { p.hover };
            ui.painter().rect_filled(r, CornerRadius::same(6), c(bg));
        }
        let ir = egui::Rect::from_center_size(egui::pos2(r.left() + 18.0, r.center().y), Vec2::splat(20.0));
        ic.paint(ui, ir, it.icon, if it.danger { p.danger_text } else { p.dim }, false, 16.0);
        ui.painter().text(egui::pos2(r.left() + 36.0, r.center().y), egui::Align2::LEFT_CENTER, it.label, egui::FontId::proportional(14.0), c(col));
        if resp.clicked() {
            hit = Some(i);
        }
    }
    if hit.is_some() {
        ui.close();
    }
    hit
}

/// Kleine Abschnittsüberschrift in Menüs und Optionen.
pub fn caption(ui: &mut Ui, text: &str) {
    ui.add_space(4.0);
    ui.label(egui::RichText::new(text).size(12.0).strong().color(c(pal().dim)));
}

/// Gezeichnetes Icon für den Maschinenbau-Pfeil (gefülltes, schmales Dreieck am Linienende).
/// TODO(WP3): durch `icons::ARROW_ENG` ersetzen, sobald der Kern das Icon liefert.
pub fn paint_arrow_eng(painter: &egui::Painter, rect: egui::Rect, color: Color32) {
    let r = rect.shrink(rect.width() * 0.22);
    let (a, b) = (r.left_bottom(), r.right_top());
    let d = (b - a).normalized();
    let n = Vec2::new(-d.y, d.x);
    let len = (b - a).length() * 0.45;
    let base = b - d * len;
    let half = len * (7.5f32.to_radians().tan());
    let half = half.max(2.2);
    painter.line_segment([a, base], Stroke::new(1.8, color));
    painter.add(egui::Shape::convex_polygon(vec![b, base + n * half, base - n * half], color, Stroke::NONE));
}

/// OwnDraw-Logo (assets/owndraw.png) als Textur, einmal pro Kontext geladen.
pub fn logo(ctx: &egui::Context) -> Option<TextureHandle> {
    let id = egui::Id::unique("owndraw-logo");
    if let Some(t) = ctx.data(|d| d.get_temp::<TextureHandle>(id)) {
        return Some(t);
    }
    let pm = owndraw_core::tiny_skia::Pixmap::decode_png(include_bytes!("../../../assets/owndraw.png")).ok()?;
    let img = egui::ColorImage::from_rgba_premultiplied([pm.width() as usize, pm.height() as usize], pm.data());
    let t = ctx.load_texture("owndraw-logo", img, egui::TextureOptions::LINEAR);
    ctx.data_mut(|d| d.insert_temp(id, t.clone()));
    Some(t)
}

// ---------------------------------------------------------------- Design-Tokens (UI-Update 4.27)

/// Abstände (Punkte). Überall diese Stufen statt freier Zahlen verwenden.
pub mod sp {
    pub const XS: f32 = 4.0;
    pub const S: f32 = 8.0;
    pub const M: f32 = 12.0;
    pub const L: f32 = 16.0;
    pub const XL: f32 = 24.0;
    pub const XXL: f32 = 32.0;
}

/// Schriftgrößen (Punkte): Seitentitel, Abschnitt, Fließtext, Hinweis.
pub mod fs {
    pub const TITLE: f32 = 22.0;
    pub const SECTION: f32 = 15.0;
    pub const BODY: f32 = 14.5;
    pub const SMALL: f32 = 12.5;
    pub const CAPTION: f32 = 11.5;
}

/// Seitentitel (groß, kräftig).
pub fn title(text: &str) -> egui::RichText {
    egui::RichText::new(text).size(fs::TITLE).strong().color(c(pal().text))
}

/// Gedämpfter Hinweistext.
pub fn hint(text: &str) -> egui::RichText {
    egui::RichText::new(text).size(fs::SMALL).color(c(pal().dim))
}

/// Menüzeile mit Icon links und optionalem Kürzel/Hinweis rechts (Editor-Menü „⋯“).
pub fn menu_row_ic(ui: &mut Ui, ic: &mut Icons, icon: i32, label: &str, right: &str, danger: bool, min_h: f32) -> bool {
    let p = pal();
    let w = ui.available_width();
    let (r, resp) = ui.allocate_exact_size(Vec2::new(w, min_h), Sense::click());
    let col = if danger { p.danger_text } else { p.text };
    if resp.hovered() || resp.has_focus() || resp.is_pointer_button_down_on() {
        let bg = if danger { with_alpha(p.danger, 0x22) } else if resp.is_pointer_button_down_on() { p.press } else { p.hover };
        ui.painter().rect_filled(r, CornerRadius::same(if modern() { 6 } else { 10 }), c(bg));
    }
    let ir = egui::Rect::from_center_size(egui::pos2(r.left() + 18.0, r.center().y), Vec2::splat(20.0));
    ic.paint(ui, ir, icon, if danger { p.danger_text } else { p.dim }, false, 17.0);
    ui.painter().text(egui::pos2(r.left() + 38.0, r.center().y), egui::Align2::LEFT_CENTER, label, egui::FontId::proportional(14.0), c(col));
    if !right.is_empty() {
        ui.painter().text(egui::pos2(r.right() - 10.0, r.center().y), egui::Align2::RIGHT_CENTER, right, egui::FontId::proportional(fs::CAPTION), c(p.dim));
    }
    resp.clicked()
}

/// Dünne Trennlinie über die volle Breite (Menüs, Karten).
pub fn separator(ui: &mut Ui) {
    let (r, _) = ui.allocate_exact_size(Vec2::new(ui.available_width(), 9.0), Sense::hover());
    ui.painter().line_segment([egui::pos2(r.left() + 6.0, r.center().y), egui::pos2(r.right() - 6.0, r.center().y)], Stroke::new(1.0, c(pal().line)));
}

/// Kleines Abzeichen („Neu“, Kürzel) in Akzentfarbe.
pub fn badge(ui: &mut Ui, text: &str) {
    let p = pal();
    let g = ui.painter().layout_no_wrap(text.to_string(), egui::FontId::proportional(fs::CAPTION), c(p.accent));
    let (r, _) = ui.allocate_exact_size(g.size() + Vec2::new(12.0, 4.0), Sense::hover());
    ui.painter().rect_filled(r, CornerRadius::same(255), c(p.accent_soft));
    ui.painter().galley(r.center() - g.size() / 2.0, g, c(p.accent));
}

/// Hinweiskarte mit Akzentstreifen links (Erklärungen, z. B. „neue Werkzeuge sind aus“).
pub fn info_card<R>(ui: &mut Ui, body: impl FnOnce(&mut Ui) -> R) -> R {
    let p = pal();
    let resp = Frame::new()
        .fill(c(p.accent_soft))
        .corner_radius(CornerRadius::same(if modern() { 10 } else { 14 }))
        .inner_margin(Margin { left: 16, right: 14, top: 12, bottom: 12 })
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            body(ui)
        });
    let r = resp.response.rect;
    ui.painter().rect_filled(egui::Rect::from_min_size(r.min + Vec2::new(0.0, 8.0), Vec2::new(3.0, (r.height() - 16.0).max(0.0))), CornerRadius::same(2), c(p.accent));
    resp.inner
}
