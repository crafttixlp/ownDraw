//! Farben, Leisten-Stil und Icon-Texturen der Desktop-Oberflaeche.

use egui::{Color32, CornerRadius, Frame, Margin, Response, Sense, Shadow, Stroke, TextureHandle, Ui, Vec2};
use owndraw_core::icons;
use std::collections::HashMap;

pub const BG: u32 = 0xFF101114;
pub const CARD: u32 = 0xFF1C1D22;
pub const PILL: u32 = 0xF51E1F25;
pub const LINE: u32 = 0x24FFFFFF;
pub const TEXT: u32 = 0xFFE9EAEE;
pub const DIM: u32 = 0xFF8B8D98;
pub const ACCENT: u32 = 0xFF7C8CFF;
pub const DANGER: u32 = 0xFFFF6B6B;
pub const DANGER_TEXT: u32 = 0xFFFF8A8A;
pub const ACTIVE: u32 = 0x3D7C8CFF;
pub const DISABLED: u32 = 0xFF55575F;

/// 0xAARRGGBB nach egui-Farbe.
pub fn c(argb: u32) -> Color32 {
    Color32::from_rgba_unmultiplied((argb >> 16) as u8, (argb >> 8) as u8, argb as u8, (argb >> 24) as u8)
}

pub fn pill(radius: u8) -> Frame {
    Frame::new()
        .fill(c(PILL))
        .stroke(Stroke::new(1.0, c(LINE)))
        .corner_radius(CornerRadius::same(radius))
        .inner_margin(Margin::same(4))
        .shadow(Shadow { offset: [0, 3], blur: 14, spread: 0, color: Color32::from_black_alpha(90) })
}

pub fn card() -> Frame {
    Frame::new()
        .fill(c(CARD))
        .stroke(Stroke::new(1.0, c(0x40FFFFFF)))
        .corner_radius(CornerRadius::same(14))
        .inner_margin(Margin { left: 24, right: 24, top: 20, bottom: 16 })
        .shadow(Shadow { offset: [0, 4], blur: 20, spread: 0, color: Color32::from_black_alpha(120) })
}

pub fn setup_visuals(ctx: &egui::Context) {
    let mut v = egui::Visuals::dark();
    v.panel_fill = c(BG);
    v.window_fill = c(CARD);
    v.window_stroke = Stroke::new(1.0, c(0x40FFFFFF));
    v.window_corner_radius = CornerRadius::same(14);
    v.menu_corner_radius = CornerRadius::same(10);
    v.extreme_bg_color = c(BG);
    v.selection.bg_fill = c(0x667C8CFF);
    v.selection.stroke = Stroke::new(1.0, c(ACCENT));
    v.hyperlink_color = c(ACCENT);
    v.override_text_color = Some(c(TEXT));
    v.widgets.hovered.weak_bg_fill = c(0x337C8CFF);
    v.widgets.active.weak_bg_fill = c(0x4D7C8CFF);
    ctx.set_visuals(v);
    ctx.all_styles_mut(|s| {
        s.spacing.item_spacing = Vec2::new(6.0, 6.0);
        s.spacing.button_padding = Vec2::new(10.0, 6.0);
        s.spacing.interact_size.y = 30.0;
        s.text_styles.insert(egui::TextStyle::Body, egui::FontId::proportional(14.5));
        s.text_styles.insert(egui::TextStyle::Button, egui::FontId::proportional(14.0));
        s.text_styles.insert(egui::TextStyle::Heading, egui::FontId::proportional(17.0));
    });
    ctx.options_mut(|o| o.zoom_with_keyboard = false);
}

/// Zwischenspeicher fuer gerenderte Icons (id, Farbe, gefuellt, Pixelgroesse).
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
}

/// Runder Icon-Knopf 40 × 40 im Stil der Leisten.
pub fn icon_btn(ui: &mut Ui, ic: &mut Icons, id: i32, tint: u32, active: bool, filled: bool, tip: &str) -> Response {
    let (rect, resp) = ui.allocate_exact_size(Vec2::splat(40.0), Sense::click());
    let bg = if active {
        Some(ACTIVE)
    } else if resp.is_pointer_button_down_on() {
        Some(0x26FFFFFF)
    } else if resp.hovered() {
        Some(0x17FFFFFF)
    } else {
        None
    };
    if let Some(b) = bg {
        ui.painter().rect_filled(rect.shrink(2.0), CornerRadius::same(24), c(b));
    }
    if let Some(t) = ic.get(ui.ctx(), id, tint, filled, 22.0) {
        let r = egui::Rect::from_center_size(rect.center(), Vec2::splat(22.0));
        ui.painter().image(t.id(), r, egui::Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(1.0, 1.0)), Color32::WHITE);
    }
    if !tip.is_empty() {
        resp.on_hover_text(tip)
    } else {
        resp
    }
}

/// Senkrechter Trenner zwischen Knopfgruppen.
pub fn sep(ui: &mut Ui) {
    let (rect, _) = ui.allocate_exact_size(Vec2::new(9.0, 40.0), Sense::hover());
    ui.painter().line_segment([rect.center_top() + Vec2::new(0.0, 10.0), rect.center_bottom() - Vec2::new(0.0, 10.0)], Stroke::new(1.0, c(LINE)));
}

/// Textknopf mit Akzentfarbe (Stil 1 der Java-Version).
pub fn accent_btn(ui: &mut Ui, label: &str) -> Response {
    let b = egui::Button::new(egui::RichText::new(label).color(c(ACCENT)).strong())
        .fill(c(0x1F7C8CFF))
        .stroke(Stroke::NONE)
        .corner_radius(CornerRadius::same(14));
    ui.add(b)
}

/// Gefuellter Hauptknopf (Stil 2).
pub fn primary_btn(ui: &mut Ui, label: &str) -> Response {
    let b = egui::Button::new(egui::RichText::new(label).color(c(BG)).strong()).fill(c(ACCENT)).corner_radius(CornerRadius::same(14));
    ui.add(b)
}

pub fn plain_btn(ui: &mut Ui, label: &str) -> Response {
    ui.add(egui::Button::new(label).fill(Color32::TRANSPARENT).corner_radius(CornerRadius::same(14)))
}

pub fn danger_btn(ui: &mut Ui, label: &str) -> Response {
    let b = egui::Button::new(egui::RichText::new(label).color(c(DANGER_TEXT)).strong()).fill(c(0x33FF6B6B)).corner_radius(CornerRadius::same(14));
    ui.add(b)
}

/// Menueeintrag (volle Breite).
pub fn menu_item(ui: &mut Ui, label: &str, danger: bool) -> bool {
    let txt = if danger { egui::RichText::new(label).color(c(DANGER_TEXT)) } else { egui::RichText::new(label) };
    let r = ui.add(egui::Button::new(txt).fill(Color32::TRANSPARENT).min_size(Vec2::new(170.0, 30.0)));
    if r.clicked() {
        ui.close();
        true
    } else {
        false
    }
}
