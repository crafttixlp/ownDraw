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

pub fn show(ui: &mut egui::Ui, sh: &mut Shared, st: &mut StartState, recent: &Recent) {
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
                });
            });
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
            ui.add_space(22.0);
            ui.label(egui::RichText::new("Zuletzt geöffnet").size(18.0).strong());
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
                    pt.text(Pos2::new(r.left() + 14.0, r.top() + ph + 39.0), Align2::LEFT_CENTER, fmt_time(e.modified_ms), egui::FontId::proportional(12.0), c(p.dim));
                    if resp.hovered() {
                        pt.rect_stroke(r, CornerRadius::same(16), Stroke::new(1.5, c(p.accent)), egui::StrokeKind::Inside);
                    }
                    if resp.clicked() {
                        sh.reqs.push(Req::OpenFile(e.file.clone()));
                    }
                    let remove = |ui: &mut egui::Ui, sh: &mut Shared| {
                        if style::menu_row(ui, "Öffnen", false, 34.0) {
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
