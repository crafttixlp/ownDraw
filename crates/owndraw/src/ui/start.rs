//! Startbildschirm: „Zuletzt geöffnet“ als Kacheln mit Vorschau, „Neu“, „Öffnen …“, Einstellungen.

use super::{elide, fmt_time};
use crate::app::{PickPurpose, Req, Shared};
use crate::layout::FormFactor;
use crate::platform::{FileKind, FileRef};
use crate::recent::Recent;
use crate::session;
use crate::style::{self, c};
use egui::{Align2, CornerRadius, Pos2, Rect, Sense, Stroke, TextureHandle, Vec2};
use std::collections::HashMap;

#[derive(Default)]
pub struct StartState {
    thumbs: HashMap<String, Option<TextureHandle>>,
    /// Konto und Boards am Server (nur Browser).
    #[cfg(not(target_os = "android"))]
    pub account: super::account::AccountUi,
    /// Suchtext (modernes Design).
    query: String,
    /// Filter: 0 = alle, 1 = Seiten, 2 = Unendlich.
    filter: u8,
    /// Demo `gast`: Gast-Hinweis wie im Browser auch am Desktop zeigen (nur Screenshots).
    #[cfg(not(target_arch = "wasm32"))]
    pub demo_banner: bool,
}

impl StartState {
    /// Vorschau einer Datei neu laden lassen (z. B. nach dem Bearbeiten).
    pub fn forget(&mut self, f: &FileRef) {
        self.thumbs.remove(&f.to_key());
    }

    /// Vorschau direkt setzen (Demo/Tests).
    pub fn set_thumb(&mut self, f: &FileRef, t: TextureHandle) {
        self.thumbs.insert(f.to_key(), Some(t));
    }

    fn load(&mut self, ctx: &egui::Context, sh: &Shared, f: &FileRef) {
        let key = f.to_key();
        if self.thumbs.contains_key(&key) {
            return;
        }
        let tex = session::read_thumb(&*sh.platform, f)
            .and_then(|b| owndraw_core::tiny_skia::Pixmap::decode_png(&b).ok())
            .map(|pm| {
                let img = egui::ColorImage::from_rgba_premultiplied([pm.width() as usize, pm.height() as usize], pm.data());
                ctx.load_texture(format!("thumb-{key}"), img, egui::TextureOptions::LINEAR)
            });
        self.thumbs.insert(key, tex);
    }
}

/// Halbdurchsichtige Fläche über ausgegrauten Kacheln (Farbe des Hintergrunds).
fn p_overlay(_ui: &egui::Ui) -> egui::Color32 {
    let bg = c(style::pal().bg);
    egui::Color32::from_rgba_unmultiplied(bg.r(), bg.g(), bg.b(), 170)
}

pub fn show(ui: &mut egui::Ui, sh: &mut Shared, st: &mut StartState, recent: &Recent) {
    if style::modern() {
        show_modern(ui, sh, st, recent);
        return;
    }
    let ctx = ui.ctx().clone();
    let p = style::pal();
    let env = sh.env;
    let compact = env.ff == FormFactor::Compact;
    let side: i8 = if compact { 16 } else if env.ff == FormFactor::Medium { 28 } else { 48 };
    let h = env.btn;
    egui::ScrollArea::vertical().auto_shrink([false, false]).show(ui, |ui| {
        egui::Frame::new().inner_margin(egui::Margin { left: side, right: side, top: 22, bottom: 24 }).show(ui, |ui| {
            // Kopf
            ui.horizontal(|ui| {
                ui.vertical(|ui| {
                    ui.label(egui::RichText::new("OwnDraw").size(if compact { 28.0 } else { 36.0 }).strong());
                    if !compact {
                        ui.label(egui::RichText::new("Zeichnen in Ebenen und auf Seiten").size(14.0).color(c(p.dim)));
                    }
                });
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if style::plain_btn(ui, "Einstellungen", h).clicked() {
                        sh.reqs.push(Req::OpenSettings);
                    }
                    if style::plain_btn(ui, "Anleitung", h).on_hover_text("Alle Funktionen, Tastenkürzel und Tipps").clicked() {
                        sh.reqs.push(Req::OpenTutorial);
                    }
                    if super::bug_report_available() && style::plain_btn(ui, "Fehler melden", h).clicked() {
                        super::open_bug_report(sh);
                    }
                    if super::bug_report_available() && style::plain_btn(ui, "Apps", h).on_hover_text("Android- und Linux-App herunterladen").clicked() {
                        super::open_downloads(sh);
                    }
                    if super::admin_available() && style::plain_btn(ui, "Admin-Panel", h).on_hover_text("Verwaltung öffnen (nur für Admins)").clicked() {
                        super::open_admin(sh);
                    }
                });
            });
            // Browser: Konto bzw. Gastmodus-Hinweis (dezent)
            #[cfg(not(target_os = "android"))]
            {
                ui.add_space(10.0);
                super::account::header(ui, sh, &mut st.account);
            }
            ui.add_space(14.0);
            // Aktionen
            let btn_w = if compact { (ui.available_width() - 8.0) / 2.0 } else { 140.0 };
            ui.horizontal(|ui| {
                if ui.add_sized([btn_w, h], egui::Button::new(egui::RichText::new("+ Neu").color(c(p.on_accent)).strong()).fill(c(p.accent)).corner_radius(CornerRadius::same(14))).clicked() {
                    sh.reqs.push(Req::NewDialog);
                }
                if ui.add_sized([btn_w, h], egui::Button::new(egui::RichText::new("Öffnen …").color(c(p.accent)).strong()).fill(c(p.accent_soft)).corner_radius(CornerRadius::same(14))).clicked() {
                    sh.reqs.push(Req::PickOpen { purpose: PickPurpose::OpenDoc, kinds: vec![FileKind::Owndraw, FileKind::Xopp] });
                }
            });
            #[cfg(not(target_os = "android"))]
            {
                super::account::boards(ui, sh, &mut st.account);
                ui.spacing_mut().item_spacing = egui::style::Spacing::default().item_spacing;
            }
            ui.add_space(22.0);
            ui.label(egui::RichText::new(if crate::platform::WEB { "Zuletzt geöffnet (in diesem Browser)" } else { "Zuletzt geöffnet" }).size(18.0).strong());
            ui.add_space(8.0);

            // Kachelraster
            let gap = if compact { 12.0 } else { 18.0 };
            let avail = ui.available_width();
            let min_w = if compact { 140.0 } else { 200.0 };
            let cols = (((avail + gap) / (min_w + gap)).floor() as usize).max(2);
            let tw = ((avail - gap * (cols as f32 - 1.0)) / cols as f32).floor();
            let ph = (tw * 300.0 / 480.0).round();
            let th = ph + 54.0;
            ui.spacing_mut().item_spacing = Vec2::splat(gap);
            let mut loaded = 0;
            ui.horizontal_wrapped(|ui| {
                // „Neu“-Kachel
                let (r, resp) = ui.allocate_exact_size(Vec2::new(tw, th), Sense::click());
                let pt = ui.painter();
                pt.rect_filled(r, CornerRadius::same(16), c(if resp.hovered() { p.active } else { p.accent_soft }));
                dashed_round_rect(pt, r.shrink(1.0), 16.0, Stroke::new(1.5, c(p.accent)));
                pt.text(r.center() - Vec2::new(0.0, 14.0), Align2::CENTER_CENTER, "+", egui::FontId::proportional(44.0), c(p.accent));
                pt.text(r.center() + Vec2::new(0.0, 28.0), Align2::CENTER_CENTER, "Neu", egui::FontId::proportional(16.0), c(p.text));
                if resp.clicked() {
                    sh.reqs.push(Req::NewDialog);
                }
                for e in recent.list() {
                    if loaded < 2 && !st.thumbs.contains_key(&e.file.to_key()) {
                        st.load(&ctx, sh, &e.file);
                        loaded += 1;
                    }
                    let tex = st.thumbs.get(&e.file.to_key()).cloned().flatten();
                    let missing = recent.is_missing(&e.file);
                    let (r, resp) = ui.allocate_exact_size(Vec2::new(tw, th), Sense::click());
                    let pt = ui.painter();
                    pt.rect_filled(r, CornerRadius::same(16), c(p.card));
                    let img_r = Rect::from_min_size(r.min, Vec2::new(tw, ph));
                    let top = CornerRadius { nw: 16, ne: 16, sw: 0, se: 0 };
                    pt.rect_filled(img_r, top, c(0xFFFBFBF8));
                    match tex {
                        Some(t) => {
                            // auf die Kachel zuschneiden (Mitte)
                            let ts = t.size_vec2();
                            let k = (img_r.width() / ts.x).max(img_r.height() / ts.y);
                            let (uw, vh) = (img_r.width() / (ts.x * k), img_r.height() / (ts.y * k));
                            let uv = Rect::from_min_max(Pos2::new((1.0 - uw) / 2.0, (1.0 - vh) / 2.0), Pos2::new((1.0 + uw) / 2.0, (1.0 + vh) / 2.0));
                            egui::Image::new(&t).uv(uv).corner_radius(top).paint_at(ui, img_r);
                        }
                        None => {
                            let g = if e.pages { "▤" } else { "∞" };
                            ui.painter().text(img_r.center(), Align2::CENTER_CENTER, g, egui::FontId::proportional(34.0), c(0xFFB8B9C0));
                        }
                    }
                    let pt = ui.painter();
                    pt.line_segment([img_r.left_bottom(), img_r.right_bottom()], Stroke::new(1.0, c(0x14000000)));
                    // Art-Marke
                    let badge = if e.pages { "Seiten" } else { "Unendlich" };
                    let bg = pt.layout_no_wrap(badge.to_string(), egui::FontId::proportional(11.0), c(0xFFFFFFFF));
                    let br = Rect::from_min_size(img_r.left_top() + Vec2::new(8.0, 8.0), bg.size() + Vec2::new(12.0, 6.0));
                    pt.rect_filled(br, CornerRadius::same(255), egui::Color32::from_black_alpha(120));
                    pt.galley(br.min + Vec2::new(6.0, 3.0), bg, c(0xFFFFFFFF));
                    let name = elide(&e.name, 15.0, tw - 32.0, ui);
                    let pt = ui.painter();
                    pt.text(Pos2::new(r.left() + 14.0, r.top() + ph + 18.0), Align2::LEFT_CENTER, name, egui::FontId::proportional(15.0), c(p.text));
                    let sub = if missing { "Datei nicht gefunden".to_string() } else { fmt_time(e.modified_ms) };
                    pt.text(Pos2::new(r.left() + 14.0, r.top() + ph + 39.0), Align2::LEFT_CENTER, sub, egui::FontId::proportional(12.0), c(if missing { p.danger } else { p.dim }));
                    if missing {
                        // ausgegraut
                        pt.rect_filled(r, CornerRadius::same(16), p_overlay(ui));
                    }
                    if resp.hovered() {
                        pt.rect_stroke(r, CornerRadius::same(16), Stroke::new(1.5, c(p.accent)), egui::StrokeKind::Inside);
                    }
                    if resp.clicked() {
                        sh.reqs.push(Req::OpenFile(e.file.clone()));
                    }
                    let remove = |ui: &mut egui::Ui, sh: &mut Shared| {
                        if !missing && style::menu_row(ui, "Öffnen", false, 34.0) {
                            sh.reqs.push(Req::OpenFile(e.file.clone()));
                            ui.close();
                        }
                        if style::menu_row(ui, "Aus der Liste entfernen", true, 34.0) {
                            sh.reqs.push(Req::RemoveRecent(e.file.clone()));
                            ui.close();
                        }
                    };
                    resp.context_menu(|ui| remove(ui, sh));
                    // sichtbarer Menüknopf (für Touch ohne Lang-Drücken)
                    let mr = Rect::from_center_size(Pos2::new(r.right() - 22.0, r.top() + ph + 24.0), Vec2::splat(36.0));
                    let mresp = ui.interact(mr, resp.id.with("more"), Sense::click());
                    if mresp.hovered() {
                        ui.painter().circle_filled(mr.center(), 16.0, c(p.hover));
                    }
                    sh.icons.paint(ui, mr, owndraw_core::icons::MORE, p.dim, false, 18.0);
                    egui::Popup::menu(&mresp).align(egui::RectAlign::BOTTOM_END).show(|ui| remove(ui, sh));
                }
            });
            if recent.list().is_empty() {
                ui.add_space(14.0);
                ui.label(egui::RichText::new("Noch nichts geöffnet. Lege mit „Neu“ ein Board an oder öffne eine Datei.").color(c(p.dim)));
            }
            if loaded > 0 {
                ctx.request_repaint();
            }
        });
    });
    #[cfg(not(target_os = "android"))]
    super::account::dialogs(&ctx, sh, &mut st.account);
}

/// Gestrichelte abgerundete Umrandung (für die „Neu“-Kachel).
fn dashed_round_rect(p: &egui::Painter, r: Rect, rad: f32, st: Stroke) {
    let mut pts = Vec::new();
    let n = 10;
    let corners = [(r.right() - rad, r.top() + rad, -90.0f32), (r.right() - rad, r.bottom() - rad, 0.0), (r.left() + rad, r.bottom() - rad, 90.0), (r.left() + rad, r.top() + rad, 180.0)];
    for (cx, cy, a0) in corners {
        for i in 0..=n {
            let a = (a0 + 90.0 * i as f32 / n as f32).to_radians();
            pts.push(Pos2::new(cx + rad * a.cos(), cy + rad * a.sin()));
        }
    }
    pts.push(pts[0]);
    p.extend(egui::Shape::dashed_line(&pts, st, 6.0, 6.0));
}

/// Zeit relativ zu jetzt: „gerade eben“, „vor 12 Min.“, „heute, 16:31“, „gestern, 09:10“, sonst Datum.
pub fn fmt_rel(ms: i64) -> String {
    let now = owndraw_core::store::now_ms();
    let diff = (now - ms) / 1000;
    if (0..60).contains(&diff) {
        return "gerade eben".into();
    }
    if (60..3600).contains(&diff) {
        return format!("vor {} Min.", diff / 60);
    }
    let full = fmt_time(ms);
    let (date, time) = full.split_once(" · ").unwrap_or((full.as_str(), ""));
    if fmt_time(now).starts_with(date) {
        return format!("heute, {time}");
    }
    if fmt_time(now - 86_400_000).starts_with(date) {
        return format!("gestern, {time}");
    }
    format!("{date}, {time}")
}

/// Hinweis-Banner (Konto, Gastmodus): Symbol, Titel und Erklärung links, Knöpfe rechts und senkrecht mittig.
/// Schmal (Handy) steht alles untereinander.
pub fn banner(ui: &mut egui::Ui, compact: bool, title: &str, body: &str, buttons: impl FnOnce(&mut egui::Ui)) {
    let p = style::pal();
    egui::Frame::new()
        .fill(c(p.card))
        .stroke(Stroke::new(1.0, c(p.line)))
        .corner_radius(CornerRadius::same(if style::modern() { 12 } else { 14 }))
        .inner_margin(egui::Margin::symmetric(if compact { 14 } else { 18 }, 12))
        .show(ui, |ui| {
            let w = ui.available_width();
            ui.set_width(w);
            let icon = |ui: &mut egui::Ui| {
                let (r, _) = ui.allocate_exact_size(Vec2::splat(34.0), Sense::hover());
                ui.painter().circle_filled(r.center(), 17.0, c(p.accent_soft));
                ui.painter().text(r.center(), Align2::CENTER_CENTER, "i", egui::FontId::proportional(17.0), c(p.accent));
            };
            let texts = |ui: &mut egui::Ui, tw: f32| {
                ui.set_width(tw);
                ui.spacing_mut().item_spacing.y = 4.0;
                ui.label(egui::RichText::new(title).size(14.0).strong().color(c(p.text)));
                ui.label(egui::RichText::new(body).size(12.5).color(c(p.dim)));
            };
            if compact {
                ui.horizontal(|ui| {
                    icon(ui);
                    let tw = ui.available_width();
                    ui.vertical(|ui| texts(ui, tw));
                });
                ui.add_space(style::sp::S);
                ui.horizontal_wrapped(|ui| {
                    ui.spacing_mut().item_spacing.y = 0.0;
                    buttons(ui)
                });
                return;
            }
            // Breite der Knöpfe aus dem letzten Frame (beim ersten Mal geschätzt)
            let id = ui.scope_id().with("banner-buttons");
            let bw: f32 = ui.data(|d| d.get_temp(id)).unwrap_or(240.0);
            let gap = 16.0;
            let tw = (w - 34.0 - 14.0 - gap - bw).max(180.0);
            let th = ui.painter().layout(title.to_string(), egui::FontId::proportional(14.0), c(p.text), tw).size().y
                + 4.0
                + ui.painter().layout(body.to_string(), egui::FontId::proportional(12.5), c(p.dim), tw).size().y;
            let row_h = th.max(36.0);
            ui.horizontal(|ui| {
                ui.spacing_mut().item_spacing.x = 14.0;
                ui.allocate_ui_with_layout(Vec2::new(34.0, row_h), egui::Layout::top_down(egui::Align::Center).with_main_align(egui::Align::Center), icon);
                ui.allocate_ui_with_layout(Vec2::new(tw, row_h), egui::Layout::top_down(egui::Align::Min), |ui| texts(ui, tw));
                let rest = ui.available_width();
                let used = ui
                    .allocate_ui_with_layout(Vec2::new(rest, row_h), egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        ui.spacing_mut().item_spacing.x = 8.0;
                        let x0 = ui.cursor().max.x;
                        buttons(ui);
                        x0 - ui.cursor().max.x
                    })
                    .inner;
                if (used - bw).abs() > 0.5 {
                    ui.data_mut(|d| d.insert_temp(id, used));
                    ui.ctx().request_repaint();
                }
            });
        });
}

/// OwnDraw-Logo (App-Icon) in der Größe `s`.
fn brand_mark(ui: &mut egui::Ui, s: f32) {
    let (r, _) = ui.allocate_exact_size(Vec2::splat(s), Sense::hover());
    if let Some(t) = style::logo(ui.ctx()) {
        egui::Image::new(&t).paint_at(ui, r);
    }
}

/// Stil eines Knopfes mit Icon.
#[derive(Clone, Copy, PartialEq)]
pub enum BtnKind {
    /// Gefüllt in Akzentfarbe (Hauptaktion).
    Primary,
    /// Mit Rahmen (Nebenaktion).
    Outline,
    /// Ohne Rahmen, nur beim Überfahren sichtbar (Kopfzeile).
    Ghost,
}

/// Breite eines Knopfes mit Icon und Text (für mittiges Setzen).
fn icon_label_width(ui: &egui::Ui, label: &str) -> f32 {
    let g = ui.painter().layout_no_wrap(label.to_string(), egui::FontId::proportional(style::fs::BODY), egui::Color32::WHITE);
    icon_label_w(g.size().x, label.is_empty())
}

fn icon_label_w(text_w: f32, icon_only: bool) -> f32 {
    (if icon_only { 0.0 } else { text_w + style::sp::S }) + 18.0 + 2.0 * style::sp::M
}

/// Knopf mit Icon links und Text (ohne Text: quadratischer Icon-Knopf der Höhe `h`, Beschriftung als Tooltip).
pub fn icon_label_btn(ui: &mut egui::Ui, icons: &mut style::Icons, icon: i32, label: &str, tip: &str, kind: BtnKind, h: f32) -> egui::Response {
    let p = style::pal();
    let g = ui.painter().layout_no_wrap(label.to_string(), egui::FontId::proportional(style::fs::BODY), egui::Color32::WHITE);
    let w = if label.is_empty() { h } else { icon_label_w(g.size().x, false) };
    let (r, resp) = ui.allocate_exact_size(Vec2::new(w, h), Sense::click());
    let down = resp.is_pointer_button_down_on();
    let rad = CornerRadius::same(style::btn_r());
    let pt = ui.painter();
    let (fg, icon_col) = match kind {
        BtnKind::Primary => (p.on_accent, p.on_accent),
        BtnKind::Outline => (p.text, p.dim),
        BtnKind::Ghost => (p.text, p.dim),
    };
    match kind {
        BtnKind::Primary => {
            pt.rect_filled(r, rad, c(p.accent));
            if down {
                pt.rect_filled(r, rad, egui::Color32::from_black_alpha(40));
            } else if resp.hovered() {
                pt.rect_filled(r, rad, egui::Color32::from_white_alpha(36));
            }
        }
        BtnKind::Outline => {
            pt.rect_filled(r, rad, c(p.card));
            if down {
                pt.rect_filled(r, rad, c(p.press));
            } else if resp.hovered() {
                pt.rect_filled(r, rad, c(p.hover));
            }
            pt.rect_stroke(r, rad, Stroke::new(1.0, c(p.line)), egui::StrokeKind::Inside);
        }
        BtnKind::Ghost => {
            if down {
                pt.rect_filled(r, rad, c(p.press));
            } else if resp.hovered() {
                pt.rect_filled(r, rad, c(p.hover));
            }
        }
    }
    if resp.has_focus() {
        pt.rect_stroke(r, rad, Stroke::new(2.0, c(p.accent)), egui::StrokeKind::Outside);
    }
    let ic_c = if label.is_empty() { r.center() } else { Pos2::new(r.left() + style::sp::M + 9.0, r.center().y) };
    icons.paint(ui, Rect::from_center_size(ic_c, Vec2::splat(24.0)), icon, if kind == BtnKind::Primary { icon_col } else if resp.hovered() { p.text } else { icon_col }, false, 18.0);
    if !label.is_empty() {
        let g = ui.painter().layout_no_wrap(label.to_string(), egui::FontId::proportional(style::fs::BODY), c(fg));
        let pos = Pos2::new(r.left() + style::sp::M + 18.0 + style::sp::S, r.center().y - g.size().y / 2.0);
        ui.painter().galley(pos, g, c(fg));
    }
    resp.widget_info(|| egui::WidgetInfo::labeled(egui::Role::Button, true, if label.is_empty() { tip } else { label }));
    let resp = resp.on_hover_cursor(egui::CursorIcon::PointingHand);
    if !tip.is_empty() {
        resp.on_hover_text(tip)
    } else {
        resp
    }
}

/// Suchfeld mit Lupe links und „Leeren“-Knopf rechts.
fn search_field(ui: &mut egui::Ui, icons: &mut style::Icons, text: &mut String, hint: &str, width: f32, h: f32) -> egui::Response {
    let p = style::pal();
    let te = egui::TextEdit::singleline(text)
        .hint_text(hint)
        .desired_width(width)
        .min_size(Vec2::new(width, h))
        .align(egui::Align2::LEFT_CENTER)
        .margin(egui::Margin { left: 36, right: 36, top: 4, bottom: 4 });
    let resp = ui.add(te);
    let r = resp.rect;
    let rad = CornerRadius::same(style::btn_r());
    ui.painter().rect_stroke(r, rad, Stroke::new(if resp.has_focus() { 1.5 } else { 1.0 }, c(if resp.has_focus() { p.accent } else { p.line })), egui::StrokeKind::Inside);
    icons.paint(ui, Rect::from_center_size(Pos2::new(r.left() + 18.0, r.center().y), Vec2::splat(24.0)), owndraw_core::icons::SEARCH, p.dim, false, 18.0);
    if !text.is_empty() {
        let xr = Rect::from_center_size(Pos2::new(r.right() - 18.0, r.center().y), Vec2::splat(28.0));
        let xresp = ui.interact(xr, resp.id.with("clear"), Sense::click()).on_hover_text("Suche leeren");
        if xresp.hovered() {
            ui.painter().rect_filled(xr, CornerRadius::same(6), c(p.hover));
        }
        icons.paint(ui, xr, owndraw_core::icons::CLOSE, p.dim, false, 14.0);
        if xresp.clicked() {
            text.clear();
            resp.request_focus();
        }
    }
    resp
}

/// Leerzustand ohne Boards: Symbol, kurzer freundlicher Text, beide Aktionen mittig.
fn empty_state(ui: &mut egui::Ui, sh: &mut Shared, h: f32) {
    use style::{fs, sp};
    let p = style::pal();
    egui::Frame::new()
        .fill(c(p.card))
        .stroke(Stroke::new(1.0, c(p.line)))
        .corner_radius(CornerRadius::same(12))
        .inner_margin(egui::Margin::symmetric(sp::XL as i8, sp::XXL as i8))
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.vertical_centered(|ui| {
                let (r, _) = ui.allocate_exact_size(Vec2::splat(72.0), Sense::hover());
                ui.painter().circle_filled(r.center(), 36.0, c(p.accent_soft));
                sh.icons.paint(ui, r, owndraw_core::icons::PEN, p.accent, false, 32.0);
                ui.add_space(sp::L);
                ui.label(egui::RichText::new("Hier ist noch Platz").size(20.0).strong().color(c(p.text)));
                ui.add_space(sp::XS);
                ui.add(egui::Label::new(egui::RichText::new("Lege dein erstes Board an – oder öffne eine .owndraw- oder Xournal++-Datei.").size(fs::BODY).color(c(p.dim))).wrap());
                ui.add_space(sp::XL);
                let (a, b) = ("Neues Board", "Öffnen …");
                let bw = icon_label_width(ui, a) + sp::S + icon_label_width(ui, b);
                let one_row = bw <= ui.available_width();
                ui.horizontal(|ui| {
                    ui.spacing_mut().item_spacing.x = sp::S;
                    if one_row {
                        ui.add_space(((ui.available_width() - bw) / 2.0).max(0.0));
                    }
                    if icon_label_btn(ui, &mut sh.icons, owndraw_core::icons::PLUS, a, "", BtnKind::Primary, h).clicked() {
                        sh.reqs.push(Req::NewDialog);
                    }
                    if icon_label_btn(ui, &mut sh.icons, owndraw_core::icons::FOLDER, b, "", BtnKind::Outline, h).clicked() {
                        sh.reqs.push(Req::PickOpen { purpose: PickPurpose::OpenDoc, kinds: vec![FileKind::Owndraw, FileKind::Xopp] });
                    }
                });
            });
        });
}

/// Vorschaufläche einer Karte: vertiefter Hintergrund, Blatt mit Schatten, Bild oder Platzhalter.
fn paint_thumb(ui: &mut egui::Ui, area: Rect, tex: Option<&TextureHandle>, pages: bool, corners: CornerRadius, inset: f32) {
    let p = style::pal();
    ui.painter().rect_filled(area, corners, c(p.sunken));
    let paper = area.shrink(inset);
    let rr = CornerRadius::same(6);
    ui.painter().add(egui::Shadow { offset: [0, 2], blur: 8, spread: 0, color: egui::Color32::from_black_alpha(if p.dark { 120 } else { 30 }) }.as_shape(paper, rr));
    match tex {
        Some(t) => {
            let ts = t.size_vec2();
            let k = (paper.width() / ts.x).max(paper.height() / ts.y);
            let (uw, vh) = (paper.width() / (ts.x * k), paper.height() / (ts.y * k));
            let uv = Rect::from_min_max(Pos2::new((1.0 - uw) / 2.0, (1.0 - vh) / 2.0), Pos2::new((1.0 + uw) / 2.0, (1.0 + vh) / 2.0));
            egui::Image::new(t).uv(uv).corner_radius(rr).paint_at(ui, paper);
        }
        None => {
            let pt = ui.painter();
            pt.rect_filled(paper, rr, c(0xFFFBFBF8));
            if pages {
                // liniertes Blatt andeuten
                let mut y = paper.top() + 18.0;
                while y < paper.bottom() - 8.0 {
                    pt.line_segment([Pos2::new(paper.left() + 10.0, y), Pos2::new(paper.right() - 10.0, y)], Stroke::new(1.0, c(0xFFE3E4EA)));
                    y += 12.0;
                }
            } else {
                pt.text(paper.center(), Align2::CENTER_CENTER, "∞", egui::FontId::proportional(30.0), c(0xFFB8B9C0));
            }
        }
    }
}

/// Eine Board-Karte. `row`: schmale Listenzeile (Handy) statt Kachel mit großer Vorschau.
fn board_card(ui: &mut egui::Ui, sh: &mut Shared, st: &mut StartState, e: &crate::recent::RecentEntry, missing: bool, size: Vec2, row: bool, ph: f32) {
    use style::{fs, sp};
    let p = style::pal();
    let tex = st.thumbs.get(&e.file.to_key()).cloned().flatten();
    let (r, resp) = ui.allocate_exact_size(size, Sense::click());
    let hov = resp.hovered() && !missing;
    let rad = CornerRadius::same(12);
    let touch = sh.env.touch;
    let pt = ui.painter();
    // weicher Schatten, beim Überfahren etwas stärker
    let sh_a = if p.dark { if hov { 110 } else { 70 } } else if hov { 28 } else { 14 };
    pt.add(egui::Shadow { offset: [0, if hov { 6 } else { 3 }], blur: if hov { 18 } else { 10 }, spread: 0, color: egui::Color32::from_black_alpha(sh_a) }.as_shape(r, rad));
    pt.rect_filled(r, rad, c(p.card));
    let (name_x, name_y, sub_y, max_w, mr) = if row {
        let area = Rect::from_min_size(r.min + Vec2::splat(sp::S), Vec2::new(104.0, r.height() - 2.0 * sp::S));
        paint_thumb(ui, area, tex.as_ref(), e.pages, CornerRadius::same(8), 8.0);
        let mr = Rect::from_center_size(Pos2::new(r.right() - 26.0, r.center().y), Vec2::splat(44.0));
        let x = area.right() + sp::M;
        (x, r.center().y - 10.0, r.center().y + 11.0, mr.left() - x, mr)
    } else {
        let img_r = Rect::from_min_size(r.min, Vec2::new(size.x, ph));
        let inset = (size.x * 0.07).clamp(10.0, 18.0);
        paint_thumb(ui, img_r, tex.as_ref(), e.pages, CornerRadius { nw: 12, ne: 12, sw: 0, se: 0 }, inset);
        ui.painter().line_segment([img_r.left_bottom(), img_r.right_bottom()], Stroke::new(1.0, c(p.line)));
        let fy = r.top() + ph + (size.y - ph) / 2.0;
        let mr = Rect::from_center_size(Pos2::new(r.right() - 26.0, fy), Vec2::splat(if touch { 44.0 } else { 36.0 }));
        (r.left() + sp::L, fy - 10.0, fy + 11.0, mr.left() - r.left() - sp::L, mr)
    };
    let name = elide(&e.name, fs::BODY, max_w, ui);
    let kind = if e.pages { "Seiten" } else { "Unendlich" };
    let sub = if missing { "Datei nicht gefunden".to_string() } else { format!("{kind} · {}", fmt_rel(e.modified_ms)) };
    let sub = elide(&sub, fs::SMALL, max_w + 20.0, ui);
    let pt = ui.painter();
    pt.text(Pos2::new(name_x, name_y), Align2::LEFT_CENTER, name, egui::FontId::proportional(fs::BODY), c(p.text));
    pt.text(Pos2::new(name_x, sub_y), Align2::LEFT_CENTER, sub, egui::FontId::proportional(fs::SMALL), c(if missing { p.danger } else { p.dim }));
    if missing {
        pt.rect_filled(r, rad, p_overlay(ui));
    }
    let stroke = if hov { Stroke::new(1.5, c(p.accent)) } else { Stroke::new(1.0, c(p.line)) };
    ui.painter().rect_stroke(r, rad, stroke, egui::StrokeKind::Inside);
    if resp.clicked() {
        sh.reqs.push(Req::OpenFile(e.file.clone()));
    }
    // Mit Lizenz: lokales Board (Browser-Speicher bzw. Datei am Desktop) auf den Server verschieben
    let to_server = !missing && crate::ui::server_store_available() && (matches!(&e.file, crate::platform::FileRef::Uri(u) if u.starts_with("mem:")) || (!crate::platform::WEB && matches!(&e.file, crate::platform::FileRef::Path(_))));
    let act = std::cell::Cell::new(None::<u8>);
    let menu = |ui: &mut egui::Ui, sh: &mut Shared| {
        use owndraw_core::icons;
        let mut items = Vec::new();
        let mut ids = Vec::new();
        if !missing {
            items.push(style::MenuItem { icon: icons::EXPORT, label: "Öffnen", danger: false });
            ids.push(0u8);
        }
        if to_server {
            items.push(style::MenuItem { icon: icons::COPY, label: "Auf Server verschieben", danger: false });
            ids.push(2);
        }
        items.push(style::MenuItem { icon: icons::TRASH, label: "Aus der Liste entfernen", danger: true });
        ids.push(1);
        if let Some(i) = style::card_menu(ui, &mut sh.icons, &items) {
            act.set(Some(ids[i]));
        }
    };
    resp.context_menu(|ui| menu(ui, sh));
    let mresp = ui.interact(mr, resp.id.with("more"), Sense::click()).on_hover_text("Weitere Aktionen");
    if mresp.hovered() {
        ui.painter().rect_filled(mr.shrink(if touch { 6.0 } else { 0.0 }), CornerRadius::same(8), c(p.hover));
    }
    sh.icons.paint(ui, mr, owndraw_core::icons::MORE, p.dim, false, 18.0);
    egui::Popup::menu(&mresp).align(egui::RectAlign::BOTTOM_END).show(|ui| menu(ui, sh));
    match act.get() {
        Some(0) => sh.reqs.push(Req::OpenFile(e.file.clone())),
        Some(1) => sh.reqs.push(Req::RemoveRecent(e.file.clone())),
        #[cfg(not(target_os = "android"))]
        Some(2) => super::account::ask_to_server(sh, &mut st.account, &e.file, &e.name),
        _ => {}
    }
}

/// Startseite im modernen Design: Kopfzeile mit Marke und Aktionen, Titel „Boards“ mit Hauptaktion und Suche, Filter, Kartenraster
/// (schmal: einspaltige Liste). Der Inhalt ist auf `MAX_W` begrenzt und mittig gesetzt.
fn show_modern(ui: &mut egui::Ui, sh: &mut Shared, st: &mut StartState, recent: &Recent) {
    use owndraw_core::icons;
    use style::{fs, sp};
    const MAX_W: f32 = 1120.0;
    let ctx = ui.ctx().clone();
    let p = style::pal();
    let env = sh.env;
    let compact = env.ff == FormFactor::Compact;
    let side = if compact { sp::L } else if env.ff == FormFactor::Medium { 28.0 } else { 40.0 };
    let h = if compact || env.touch { env.btn.max(crate::layout::min_touch(env.touch)) } else { 36.0 };
    egui::ScrollArea::vertical().auto_shrink([false, false]).show(ui, |ui| {
        // Spalte mittig setzen (egui-Ränder sind i8 und reichen dafür nicht)
        let full = ui.available_rect_before_wrap();
        let w = (full.width() - 2.0 * side).clamp(100.0, MAX_W);
        let col = Rect::from_min_size(Pos2::new(full.center().x - w / 2.0, full.top() + if compact { sp::M } else { sp::XL }), Vec2::new(w, f32::INFINITY));
        ui.scope_builder(egui::UiBuilder::new().max_rect(col).layout(egui::Layout::top_down(egui::Align::Min)), |ui| {
            ui.set_width(w);
            let stack = w < 660.0;
            // Kopfzeile: Marke links, Anleitung/Einstellungen (und Web-Extras) rechts; schmal nur Icons
            let labels = w >= 760.0;
            ui.horizontal(|ui| {
                ui.set_min_height(h);
                ui.spacing_mut().item_spacing.x = sp::S;
                brand_mark(ui, 32.0);
                ui.label(egui::RichText::new("OwnDraw").size(fs::SECTION + 4.0).strong().color(c(p.text)));
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    ui.spacing_mut().item_spacing.x = if labels { sp::XS } else { 0.0 };
                    let lab = |s: &'static str| if labels { s } else { "" };
                    if icon_label_btn(ui, &mut sh.icons, icons::GEAR, lab("Einstellungen"), "Einstellungen", BtnKind::Ghost, h).clicked() {
                        sh.reqs.push(Req::OpenSettings);
                    }
                    if icon_label_btn(ui, &mut sh.icons, icons::HELP, lab("Anleitung"), "Anleitung: Funktionen, Tastenkürzel und Tipps", BtnKind::Ghost, h).clicked() {
                        sh.reqs.push(Req::OpenTutorial);
                    }
                    if super::bug_report_available() && icon_label_btn(ui, &mut sh.icons, icons::BUG, lab("Fehler melden"), "Fehler melden", BtnKind::Ghost, h).clicked() {
                        super::open_bug_report(sh);
                    }
                    if super::bug_report_available() && icon_label_btn(ui, &mut sh.icons, icons::DOWNLOAD, lab("Apps"), "Android- und Linux-App herunterladen", BtnKind::Ghost, h).clicked() {
                        super::open_downloads(sh);
                    }
                    if super::admin_available() && icon_label_btn(ui, &mut sh.icons, icons::SLIDERS, lab("Admin"), "Verwaltung öffnen (nur für Admins)", BtnKind::Ghost, h).clicked() {
                        super::open_admin(sh);
                    }
                });
            });
            ui.add_space(if compact { sp::L } else { sp::XL });
            #[cfg(not(target_os = "android"))]
            {
                super::account::header(ui, sh, &mut st.account);
                ui.add_space(if compact { sp::L } else { sp::XL });
            }
            #[cfg(target_os = "android")]
            if st.demo_banner {
                banner(
                    ui,
                    compact,
                    "Du bist nicht angemeldet. Boards werden nur in diesem Browser gespeichert.",
                    "Mit einem Konto wirst du zu Boards eingeladen und arbeitest live mit. Mit einer Lizenz speicherst und teilst du Boards auf dem Server.",
                    |ui| {
                        let _ = style::primary_btn(ui, "Anmelden", h);
                        let _ = style::outline_btn(ui, "Konto erstellen", h);
                    },
                );
                ui.add_space(if compact { sp::L } else { sp::XL });
            }

            // Titel mit Aktionen
            let n = recent.list().len();
            let sub = match n {
                0 => "Noch keine Boards".to_string(),
                1 => "1 Board".to_string(),
                n => format!("{n} Boards"),
            };
            let sub = if crate::platform::WEB && n > 0 { format!("{sub} in diesem Browser") } else { sub };
            // Ohne Boards trägt der Leerzustand die Aktionen (am PC), damit sie nicht doppelt erscheinen
            let show_actions = n > 0;
            let main_actions = |ui: &mut egui::Ui, sh: &mut Shared, rtl: bool| {
                let new = |ui: &mut egui::Ui, sh: &mut Shared| {
                    if icon_label_btn(ui, &mut sh.icons, icons::PLUS, "Neues Board", "", BtnKind::Primary, h).clicked() {
                        sh.reqs.push(Req::NewDialog);
                    }
                };
                let open = |ui: &mut egui::Ui, sh: &mut Shared| {
                    if icon_label_btn(ui, &mut sh.icons, icons::FOLDER, "Öffnen …", "Board aus einer Datei öffnen", BtnKind::Outline, h).clicked() {
                        sh.reqs.push(Req::PickOpen { purpose: PickPurpose::OpenDoc, kinds: vec![FileKind::Owndraw, FileKind::Xopp] });
                    }
                };
                // rechtsbündig (rtl): erster Knopf steht ganz rechts, daher in beiden Fällen dieselbe Reihenfolge
                let _ = rtl;
                new(ui, sh);
                open(ui, sh);
            };
            if stack {
                ui.label(style::title("Boards"));
                ui.label(style::hint(&sub));
                if show_actions {
                    ui.add_space(sp::M);
                    ui.horizontal_wrapped(|ui| {
                        ui.spacing_mut().item_spacing = Vec2::splat(sp::S);
                        main_actions(ui, sh, false);
                    });
                }
                if n > 0 {
                    ui.add_space(sp::M);
                    let sw = ui.available_width();
                    search_field(ui, &mut sh.icons, &mut st.query, "Boards durchsuchen …", sw, h);
                }
            } else {
                ui.horizontal(|ui| {
                    ui.set_min_height(h.max(40.0));
                    ui.vertical(|ui| {
                        ui.spacing_mut().item_spacing.y = 2.0;
                        ui.label(style::title("Boards"));
                        ui.label(style::hint(&sub));
                    });
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        ui.spacing_mut().item_spacing.x = sp::S;
                        if show_actions {
                            main_actions(ui, sh, true);
                        }
                        if n > 0 {
                            search_field(ui, &mut sh.icons, &mut st.query, "Boards durchsuchen …", 240.0, h);
                        }
                    });
                });
            }
            #[cfg(not(target_os = "android"))]
            {
                let server_section = super::account::boards(ui, sh, &mut st.account);
                ui.spacing_mut().item_spacing = Vec2::new(8.0, 6.0);
                // Neben „Auf dem Server“ die Browser-Boards klar benennen
                if server_section && n > 0 {
                    ui.add_space(sp::XL);
                    ui.label(egui::RichText::new(super::wd("In diesem Browser", "Auf diesem Computer")).size(fs::SECTION + 3.0).strong());
                    ui.label(style::hint("Nur auf diesem Gerät gespeichert. Teilen geht erst nach „Auf Server verschieben“ im Kartenmenü."));
                }
            }

            if n == 0 {
                ui.add_space(sp::L);
                empty_state(ui, sh, h);
                return;
            }

            // Filter
            ui.add_space(sp::L);
            ui.horizontal_wrapped(|ui| {
                ui.spacing_mut().item_spacing = Vec2::splat(sp::S);
                for (i, l) in [(0u8, "Alle"), (1, "Seiten"), (2, "Unendlich")] {
                    if style::chip(ui, l, st.filter == i, if compact || env.touch { env.btn } else { 32.0 }).clicked() {
                        st.filter = i;
                    }
                }
            });
            ui.add_space(sp::L);

            // Kartenraster; schmal: eine Spalte als Liste
            let row = w < 560.0;
            let gap = if row { sp::M } else { sp::XL };
            let avail = ui.available_width();
            let min_w = 240.0;
            let cols = if row { 1 } else { (((avail + gap) / (min_w + gap)).floor() as usize).max(2) };
            let tw = ((avail - gap * (cols as f32 - 1.0)) / cols as f32).floor();
            let ph = (tw * 0.7).round();
            let th = if row { 88.0 } else { ph + 62.0 };
            let q = st.query.trim().to_lowercase();
            let list: Vec<_> = recent
                .list()
                .iter()
                .filter(|e| match st.filter {
                    1 => e.pages,
                    2 => !e.pages,
                    _ => true,
                })
                .filter(|e| q.is_empty() || e.name.to_lowercase().contains(&q))
                .cloned()
                .collect();
            ui.spacing_mut().item_spacing = Vec2::splat(gap);
            let mut loaded = 0;
            let mut cards = |ui: &mut egui::Ui| {
                for e in &list {
                    if loaded < 2 && !st.thumbs.contains_key(&e.file.to_key()) {
                        st.load(&ctx, sh, &e.file);
                        loaded += 1;
                    }
                    let missing = recent.is_missing(&e.file);
                    board_card(ui, sh, st, e, missing, Vec2::new(tw, th), row, ph);
                }
                // „Neues Board“-Karte am Ende (schmal reicht der Knopf oben)
                if st.query.trim().is_empty() && !row {
                    let (r, resp) = ui.allocate_exact_size(Vec2::new(tw, th), Sense::click());
                    let rad = CornerRadius::same(12);
                    let pt = ui.painter();
                    let hov = resp.hovered();
                    pt.rect_filled(r, rad, c(if hov { p.accent_soft } else { p.card }));
                    dashed_round_rect(pt, r.shrink(0.75), 12.0, Stroke::new(1.5, c(if hov { p.accent } else { p.line })));
                    let ic = r.center() - Vec2::new(0.0, 16.0);
                    pt.circle_filled(ic, 24.0, c(if hov { p.accent } else { p.accent_soft }));
                    sh.icons.paint(ui, Rect::from_center_size(ic, Vec2::splat(48.0)), icons::PLUS, if hov { p.on_accent } else { p.accent }, false, 22.0);
                    let pt = ui.painter();
                    pt.text(r.center() + Vec2::new(0.0, 26.0), Align2::CENTER_CENTER, "Neues Board", egui::FontId::proportional(fs::BODY), c(p.text));
                    pt.text(r.center() + Vec2::new(0.0, 46.0), Align2::CENTER_CENTER, "Unendlich oder Seiten", egui::FontId::proportional(fs::SMALL), c(p.dim));
                    if resp.on_hover_cursor(egui::CursorIcon::PointingHand).clicked() {
                        sh.reqs.push(Req::NewDialog);
                    }
                }
            };
            if row {
                ui.vertical(|ui| cards(ui));
            } else {
                ui.horizontal_wrapped(|ui| cards(ui));
            }
            if list.is_empty() {
                ui.add_space(sp::S);
                ui.label(egui::RichText::new("Nichts gefunden. Versuche ein anderes Stichwort oder wähle „Alle“.").size(fs::BODY).color(c(p.dim)));
            }
            ui.add_space(sp::XXL);
            if loaded > 0 {
                ctx.request_repaint();
            }
        });
    });
    #[cfg(not(target_os = "android"))]
    super::account::dialogs(&ctx, sh, &mut st.account);
}
