//! Seiten zeichnen (mit denselben Schriften wie PDF und Seitenumbruch) und Eingabe: Maus, Touch, Tastatur, IME.

use super::{Cmd, Writer};
use crate::app::Shared;
use crate::style::{self, c};
use egui::{Color32, CornerRadius, Event, FontFamily, FontId, Key, Modifiers, Pos2, Rect, Sense, Stroke, Vec2};
use owndraw_core::office::fonts::{Family, FontKey};
use owndraw_core::office::layout::Seg;
use owndraw_core::office::{tools, Align, Kind, Pos, TextDoc};

/// Abstand zwischen Seiten (Bildschirmpunkte).
const GAP: f32 = 24.0;

/// Meldet die acht Dokumentschriften bei egui an (einmal; wirksam ab dem nächsten Bild).
pub fn ensure_fonts(ctx: &egui::Context) {
    let want = FontFamily::Name(FontKey::default().id().into());
    if ctx.fonts(|f| f.definitions().families.contains_key(&want)) {
        return;
    }
    for k in FontKey::all() {
        ctx.add_font(egui::epaint::text::FontInsert::new(
            k.id(),
            egui::FontData::from_static(k.data()),
            vec![egui::epaint::text::InsertFontFamily { family: FontFamily::Name(k.id().into()), priority: egui::epaint::text::FontPriority::Highest }],
        ));
    }
}

fn fonts_ready(ctx: &egui::Context) -> bool {
    let want = FontFamily::Name(FontKey { fam: Family::Sans, bold: true, italic: true }.id().into());
    ctx.fonts(|f| f.definitions().families.contains_key(&want))
}

pub fn font_id(k: FontKey, size: f32) -> FontId {
    FontId::new(size.max(1.0), FontFamily::Name(k.id().into()))
}

/// Text mit Grundlinie bei `base` zeichnen.
fn word(p: &egui::Painter, x: f32, base: f32, s: &str, font: FontId, col: Color32) {
    let g = p.layout_no_wrap(s.to_string(), font, col);
    let off = g.rows.first().map(|r| r.pos.y + r.row.glyphs.first().map(|gl| gl.pos.y).unwrap_or(0.0)).unwrap_or(0.0);
    p.galley(Pos2::new(x, base - off), g, col);
}

struct Geo {
    k: f32,
    /// Linke obere Ecke je Seite (Bildschirm).
    tops: Vec<Pos2>,
    pw: f32,
    ph: f32,
}

impl Geo {
    fn to_screen(&self, page: usize, x: f32, y: f32) -> Pos2 {
        let t = self.tops.get(page).copied().unwrap_or(Pos2::ZERO);
        Pos2::new(t.x + x * self.k, t.y + y * self.k)
    }
    /// Bildschirm → (Seite, x, y) in pt.
    fn to_page(&self, p: Pos2) -> (usize, f32, f32) {
        let mut best = 0;
        for (i, t) in self.tops.iter().enumerate() {
            if p.y >= t.y - GAP * 0.5 {
                best = i;
            }
        }
        let t = self.tops.get(best).copied().unwrap_or(Pos2::ZERO);
        (best, (p.x - t.x) / self.k, ((p.y - t.y) / self.k).clamp(-5.0, self.ph + 5.0))
    }
}

impl Writer {
    /// Seiten mit Inhalt, Auswahl, Cursor; Eingabe.
    pub(super) fn canvas(&mut self, ui: &mut egui::Ui, sh: &mut Shared) {
        let ctx = ui.ctx().clone();
        let id = ui.make_persistent_id("writer-canvas");
        let avail = ui.available_size();
        let narrow = avail.x < 700.0;
        let lay = self.layout().clone();
        let pg = lay.page;
        // Maßstab: 100 % = 96 dpi; schmal: an die Breite anpassen
        let base_k = if narrow { ((avail.x - 16.0) / pg.w).min(1.333) } else { 1.333 };
        let k = base_k * self.zoom;
        self.k = k;
        let (pw, ph) = (pg.w * k, pg.h * k);
        let gap = if narrow { 12.0 } else { GAP };
        let content = Vec2::new((pw + 2.0 * gap).max(avail.x), lay.pages as f32 * (ph + gap) + gap);
        let touch = sh.platform.is_touch();
        let mut caret_screen = None;
        let scroll_caret = std::mem::take(&mut self.scroll_caret);
        egui::ScrollArea::both().id_salt("writer-scroll").auto_shrink([false, false]).show_viewport(ui, |ui, _vp| {
            let sense = if touch { Sense::click() } else { Sense::click_and_drag() };
            let (rect, _) = ui.allocate_exact_size(content, sense);
            let resp = ui.interact(rect, id, sense);
            let left = rect.center().x - pw / 2.0;
            let geo = Geo { k, tops: (0..lay.pages).map(|i| Pos2::new(left, rect.top() + gap + i as f32 * (ph + gap))).collect(), pw, ph };
            let painter = ui.painter_at(ui.clip_rect());
            let clip = ui.clip_rect();
            let pal = style::pal();
            // Seiten
            for (i, t) in geo.tops.iter().enumerate() {
                let r = Rect::from_min_size(*t, Vec2::new(pw, ph));
                if !r.intersects(clip) {
                    continue;
                }
                painter.rect_filled(r.translate(Vec2::new(0.0, 2.0)).expand(1.0), CornerRadius::same(3), Color32::from_black_alpha(if pal.dark { 90 } else { 28 }));
                painter.rect_filled(r, CornerRadius::same(2), Color32::WHITE);
                let _ = i;
            }
            let ready = fonts_ready(&ctx);
            if !ready {
                ctx.request_repaint();
            }
            if !self.td.ex.watermark.trim().is_empty() {
                let wm = self.td.ex.watermark.trim().to_string();
                for (i, t) in geo.tops.iter().enumerate() {
                    let r = Rect::from_min_size(*t, Vec2::new(pw, ph));
                    if !r.intersects(clip) {
                        continue;
                    }
                    let n = wm.chars().count().max(1) as f32;
                    let size = ((pw.hypot(ph) * 0.7) / (n * 0.62)).min(140.0 * k);
                    let g = painter.layout_no_wrap(wm.clone(), font_id(FontKey { fam: Family::Sans, bold: true, italic: false }, size), Color32::from_rgba_unmultiplied(112, 112, 128, 40));
                    let ang = -(ph.atan2(pw));
                    let (sz, cen) = (g.size(), r.center());
                    let (cs, sn) = (ang.cos(), ang.sin());
                    let off = Vec2::new(-sz.x / 2.0 * cs + sz.y / 2.0 * sn, -sz.x / 2.0 * sn - sz.y / 2.0 * cs);
                    painter.add(egui::epaint::TextShape::new(cen + off, g, Color32::GRAY).with_angle(ang));
                    let _ = i;
                }
            }
            // Suchtreffer und Auswahl
            if let Some(f) = &self.find {
                for (n, (a, b)) in f.hits.iter().enumerate() {
                    let col = if n == f.idx { c(0xFFFFB648) } else { c(0x88FFE27A) };
                    for (p, x0, y, x1, h) in lay.sel_rects(*a, *b) {
                        let r = Rect::from_two_pos(geo.to_screen(p, x0, y), geo.to_screen(p, x1, y + h));
                        if r.intersects(clip) {
                            painter.rect_filled(r, CornerRadius::ZERO, col);
                        }
                    }
                }
            }
            let (sa, sb) = self.sel();
            let focused = ui.memory(|m| m.has_focus(id));
            let sel_col = if focused { c((pal.accent & 0x00FF_FFFF) | 0x44 << 24) } else { c(0x33808080) };
            for (p, x0, y, x1, h) in lay.sel_rects(sa, sb) {
                let r = Rect::from_two_pos(geo.to_screen(p, x0, y), geo.to_screen(p, x1.max(x0 + 3.0), y + h));
                if r.intersects(clip) {
                    painter.rect_filled(r, CornerRadius::ZERO, sel_col);
                }
            }
            // Bilder
            for l in &lay.lines {
                let Some((f, x, w, h)) = &l.img else { continue };
                let r = Rect::from_two_pos(geo.to_screen(l.page, *x, l.y), geo.to_screen(l.page, x + w, l.y + h));
                if !r.intersects(clip) {
                    continue;
                }
                match self.texture(&ctx, f) {
                    Some(t) => {
                        painter.image(t.id(), r, Rect::from_min_max(Pos2::ZERO, Pos2::new(1.0, 1.0)), Color32::WHITE);
                    }
                    None => {
                        painter.rect_filled(r, CornerRadius::ZERO, c(0xFFE5E7EB));
                    }
                }
            }
            // Objekte
            for l in &lay.lines {
                let Some((x, w, h)) = l.obj else { continue };
                let r = Rect::from_two_pos(geo.to_screen(l.page, x, l.y), geo.to_screen(l.page, x + w, l.y + h));
                if !r.intersects(clip) {
                    continue;
                }
                if let Some(t) = self.obj_texture(&ctx, l.para, w * k) {
                    painter.image(t.id(), r, Rect::from_min_max(Pos2::ZERO, Pos2::new(1.0, 1.0)), Color32::WHITE);
                }
            }
            // Tabellen
            if ready {
                for l in &lay.lines {
                    let Some(rb) = &l.row else { continue };
                    let top = geo.to_screen(l.page, 0.0, l.y);
                    if top.y > clip.bottom() + 4.0 || top.y + l.h * k < clip.top() - 4.0 {
                        continue;
                    }
                    let border = self.td.paras.get(l.para).and_then(|p| p.table.as_ref()).map(|t| t.border).unwrap_or(0);
                    for cb in &rb.cells {
                        let r = Rect::from_two_pos(geo.to_screen(l.page, cb.x, l.y), geo.to_screen(l.page, cb.x + cb.w, l.y + l.h));
                        if cb.bg != 0 {
                            painter.rect_filled(r, CornerRadius::ZERO, c(cb.bg));
                        }
                        let open = self.x.cell.as_ref().is_some_and(|e| e.p == l.para && e.r == rb.r && !rb.repeat && rb.cells.iter().position(|q| std::ptr::eq(q, cb)) == Some(e.c));
                        if !open {
                            for (x, by, t) in &cb.lines {
                                let at = geo.to_screen(l.page, *x, l.y + by);
                                word(&painter, at.x, at.y, t, font_id(cb.font, cb.size * k), c(cb.color));
                            }
                        }
                        let bc = if border != 0 { c(border) } else { Color32::from_rgba_unmultiplied(150, 150, 160, 50) };
                        painter.rect_stroke(r, CornerRadius::ZERO, Stroke::new(if border != 0 { 1.0 } else { 0.5 }, bc), egui::StrokeKind::Middle);
                    }
                }
                // Fußnoten
                let nf = FontKey { fam: self.td.family, bold: false, italic: false };
                let mut last = usize::MAX;
                for nb in &lay.notes {
                    if nb.page != last {
                        last = nb.page;
                        let a = geo.to_screen(nb.page, pg.ml, nb.y - 5.7);
                        painter.line_segment([a, a + Vec2::new(120.0 * k, 0.0)], Stroke::new(0.8, c(0xFF9CA3AF)));
                    }
                    for (by, t) in &nb.lines {
                        let at = geo.to_screen(nb.page, nb.x, *by);
                        if clip.expand(30.0).contains(at) {
                            word(&painter, at.x, at.y, t, font_id(nf, owndraw_core::office::layout::NOTE_SIZE * k), c(0xFF33333D));
                        }
                    }
                }
            }
            // Text
            if ready {
                for l in &lay.lines {
                    let top = geo.to_screen(l.page, 0.0, l.y);
                    if l.img.is_some() || l.obj.is_some() || l.row.is_some() || top.y > clip.bottom() + 4.0 || top.y + l.h * k < clip.top() - 4.0 {
                        continue;
                    }
                    if l.pagebreak {
                        let y = top.y + l.h * k * 0.5;
                        let (x0, x1) = (geo.to_screen(l.page, pg.ml, 0.0).x, geo.to_screen(l.page, pg.w - pg.mr, 0.0).x);
                        let mut x = x0;
                        while x < x1 {
                            painter.line_segment([Pos2::new(x, y), Pos2::new((x + 6.0).min(x1), y)], Stroke::new(1.0, c(0xFF9AA0AE)));
                            x += 10.0;
                        }
                        let g = painter.layout_no_wrap("Seitenumbruch".into(), FontId::proportional(11.0), c(0xFF7A8090));
                        let r = Rect::from_center_size(Pos2::new((x0 + x1) / 2.0, y), g.size() + Vec2::new(12.0, 4.0));
                        painter.rect_filled(r, CornerRadius::same(4), Color32::WHITE);
                        painter.galley(r.min + Vec2::new(6.0, 2.0), g, c(0xFF7A8090));
                        continue;
                    }
                    if let Some(m) = &l.marker {
                        paint_seg(&painter, &geo, l.page, m, l.y, l.h, l.base);
                    }
                    for s in &l.segs {
                        paint_seg(&painter, &geo, l.page, s, l.y, l.h, l.base);
                    }
                }
                // Rechtschreibfehler: rote Wellenlinie
                for (a, b) in &self.x.miss {
                    for (p, x0, y, x1, h) in lay.sel_rects(*a, *b) {
                        let y = y + h * 0.92;
                        let (s0, s1) = (geo.to_screen(p, x0, y), geo.to_screen(p, x1, y));
                        if !clip.contains(s0) {
                            continue;
                        }
                        let mut pts = Vec::new();
                        let mut x = s0.x;
                        let mut up = false;
                        while x <= s1.x {
                            pts.push(Pos2::new(x, s0.y + if up { -1.5 } else { 1.0 }));
                            x += 2.5;
                            up = !up;
                        }
                        if pts.len() > 1 {
                            painter.add(egui::Shape::line(pts, Stroke::new(1.0, c(0xFFE5484D))));
                        }
                    }
                }
                // Kopf-/Fußzeile
                let hf = FontKey { fam: Family::Sans, bold: false, italic: false };
                for i in 0..lay.pages {
                    for (src, y, center) in [(&self.td.header, pg.mt * 0.5, false), (&self.td.footer, pg.h - pg.mb * 0.5 + 9.0 * 0.35, true)] {
                        if src.trim().is_empty() {
                            continue;
                        }
                        let t = TextDoc::hf_text(src, i, lay.pages);
                        let w: f32 = t.chars().map(|ch| owndraw_core::office::fonts::advance(hf, ch, 9.0)).sum();
                        let x = if center { pg.ml + (pg.content_w() - w) / 2.0 } else { pg.ml };
                        let p = geo.to_screen(i, x, y);
                        if clip.expand(40.0).contains(p) {
                            word(&painter, p.x, p.y, &t, font_id(hf, 9.0 * k), c(0xFF6B6B78));
                        }
                    }
                }
            }
            // Cursor
            let (cp, cx, cy, ch) = lay.caret(self.cur);
            let ctop = geo.to_screen(cp, cx, cy);
            let crect = Rect::from_min_size(ctop, Vec2::new(1.6, ch * k));
            caret_screen = Some(crect);
            let blink_on = ((sh.now - self.blink) % 1.0) < 0.6 || sh.now - self.blink < 0.6;
            // Ausgewähltes Bild: Rahmen und Griff zum Ziehen der Größe
            let sel_img = self.sel_block();
            if let Some(p) = sel_img.filter(|p| self.td.paras[*p].kind == Kind::Table && self.x.cell.is_none()) {
                // Ausgewählte Tabelle: Rahmen
                let (f, t) = lay.para_lines.get(p).copied().unwrap_or((0, 0));
                for l in lay.lines.get(f..=t).unwrap_or(&[]) {
                    if let Some((x0, x1)) = l.block_x() {
                        let r = Rect::from_two_pos(geo.to_screen(l.page, x0, l.y), geo.to_screen(l.page, x1, l.y + l.h));
                        painter.rect_stroke(r, CornerRadius::ZERO, Stroke::new(2.0, c(pal.accent)), egui::StrokeKind::Outside);
                    }
                }
            }
            if let Some(p) = sel_img {
                let line = lay.para_lines.get(p).and_then(|(a, _)| lay.lines.get(*a));
                let blk = line.and_then(|l| l.img.clone().map(|i| (l, (i.1, i.2, i.3))).or(l.obj.map(|o| (l, o))));
                if let Some((l, (x, w, h))) = blk {
                    let r = Rect::from_two_pos(geo.to_screen(l.page, x, l.y), geo.to_screen(l.page, x + w, l.y + h));
                    let acc = c(pal.accent);
                    painter.rect_stroke(r, CornerRadius::ZERO, Stroke::new(2.0, acc), egui::StrokeKind::Outside);
                    let hs = if touch { 22.0 } else { 12.0 };
                    let hr = Rect::from_center_size(r.right_bottom(), Vec2::splat(hs));
                    painter.rect_filled(hr, CornerRadius::same(2), acc);
                    painter.rect_stroke(hr, CornerRadius::same(2), Stroke::new(1.5, Color32::WHITE), egui::StrokeKind::Inside);
                    let hresp = ui.interact(hr.expand(4.0), id.with("img-size"), Sense::drag()).on_hover_cursor(egui::CursorIcon::ResizeNwSe).on_hover_text("Ziehen ändert die Größe");
                    let align = self.td.paras[p].align;
                    if hresp.drag_started() {
                        let (iw, ih) = match (&self.td.paras[p].img, &self.td.paras[p].obj) {
                            (Some(im), _) => (im.w, im.h),
                            (_, Some(o)) => (o.w, o.h),
                            _ => (w, h),
                        };
                        self.begin(super::Edit::Other, sh.now);
                        self.resize = Some((p, iw.min(pg.content_w()), ih));
                    }
                    if let (true, Some((rp, w0, _)), Some(o), Some(cur)) = (hresp.dragged(), self.resize, ui.input(|i| i.pointer.press_origin()), hresp.interact_pointer_pos()) {
                        let f = if align == owndraw_core::office::Align::Center { 2.0 } else { 1.0 };
                        self.resize_image(rp, w0 + (cur.x - o.x) / k * f, sh.now);
                    }
                    if hresp.drag_stopped() {
                        self.resize = None;
                        self.changed(sh.now);
                    }
                }
            }
            if focused && blink_on && !self.has_sel() && sel_img.is_none() {
                painter.rect_filled(crect, CornerRadius::ZERO, c(0xFF1D1D24));
            }
            if focused {
                let t = 0.6 - ((sh.now - self.blink) % 1.0);
                ctx.request_repaint_after(std::time::Duration::from_secs_f64(if t > 0.0 { t } else { 1.0 + t }.max(0.05)));
            }
            if scroll_caret {
                ui.scroll_to_rect(crect.expand2(Vec2::new(24.0, 36.0)), None);
            }
            self.cell_overlay(ui, &geo, &lay, sh);
            self.pointer(ui, &resp, &geo, sh, touch);
            if self.focus_req {
                self.focus_req = false;
                ui.memory_mut(|m| m.request_focus(id));
            }
            if ui.memory(|m| m.has_focus(id)) {
                ui.memory_mut(|m| m.set_focus_lock_filter(id, egui::EventFilter { tab: true, horizontal_arrows: true, vertical_arrows: true, escape: false }));
                self.keys(ui, id, sh, crect);
            }
            // Rechtsklick-Menü
            resp.context_menu(|ui| super::bars::context_menu(self, ui, sh));
        });
        let _ = caret_screen;
    }

    fn pointer(&mut self, ui: &egui::Ui, resp: &egui::Response, geo: &Geo, sh: &mut Shared, touch: bool) {
        let now = sh.now;
        let mods = ui.input(|i| i.modifiers);
        let hit = |w: &mut Writer, p: Pos2| {
            let (pg, x, y) = geo.to_page(p);
            w.layout().hit(pg, x, y)
        };
        if let Some(p) = resp.hover_pos() {
            let pos = hit(self, p);
            let link = self.td.paras.get(pos.p).and_then(|q| q.style_at(pos.o + 1).link.or(q.style_at(pos.o).link));
            if link.is_some() && mods.command {
                ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
            } else {
                ui.ctx().set_cursor_icon(egui::CursorIcon::Text);
            }
        }
        let pressed = ui.input(|i| i.pointer.primary_pressed());
        let press_pos = if touch { resp.clicked().then(|| resp.interact_pointer_pos()).flatten() } else { pressed.then(|| ui.input(|i| i.pointer.interact_pos())).flatten() }.filter(|p| resp.rect.contains(*p) && ui.clip_rect().contains(*p));
        if let Some(p) = press_pos {
            let (pg, x, y) = geo.to_page(p);
            if self.x.cell.is_some() {
                self.commit_cell(now);
                self.x.cell = None;
            }
            let lay = self.layout().clone();
            // Tabellenzelle
            let cell = lay.lines.iter().filter(|l| l.page == pg && y >= l.y && y < l.y + l.h).find_map(|l| l.row.as_ref().and_then(|rb| rb.cells.iter().position(|c| x >= c.x && x < c.x + c.w).map(|ci| (l.para, rb.r, ci))));
            if let Some((tp, r, ci)) = cell {
                if !self.protected() {
                    self.open_cell(tp, r, ci);
                    return;
                }
            }
            // Fußnote unten auf der Seite
            if let Some(nb) = lay.notes.iter().find(|n| n.page == pg && y >= n.y && y < n.y + n.h) {
                if let Some(owndraw_core::office::objects::Field::Note(t)) = tools::field_at(&self.td, nb.pos) {
                    self.x.dlg = Some(super::extra::XDlg::Note { at: nb.pos, text: t, new: false });
                    return;
                }
            }
        }
        if pressed && !touch {
            if let Some(p) = ui.input(|i| i.pointer.interact_pos()).filter(|p| resp.rect.contains(*p) && ui.clip_rect().contains(*p)) {
                self.focus_req = true;
                let pos = hit(self, p);
                self.drag_words = false;
                self.set_cur(pos, mods.shift, now);
                self.goal_x = None;
            }
        }
        if touch && resp.clicked() {
            if let Some(p) = resp.interact_pointer_pos() {
                let pos = hit(self, p);
                self.set_cur(pos, false, now);
                self.focus_req = true;
            }
        }
        if resp.clicked() && !mods.command && !mods.shift && !self.has_sel() {
            self.click_field(now);
        }
        if resp.double_clicked() && self.sel_block().is_some_and(|p| self.td.paras[p].kind == Kind::Object) {
            self.run(Cmd::EditObject, sh, &ui.ctx().clone());
            return;
        }
        if resp.double_clicked() {
            let (a, b) = self.td.word_at(self.cur);
            self.anchor = a;
            self.cur = b;
            self.drag_words = true;
        }
        if resp.triple_clicked() {
            let p = self.cur.p;
            self.anchor = Pos::new(p, 0);
            self.cur = Pos::new(p, self.td.paras[p].len());
        }
        if resp.dragged() && !touch {
            if let Some(p) = resp.interact_pointer_pos() {
                let pos = hit(self, p);
                if self.drag_words {
                    let (a, b) = self.td.word_at(pos);
                    self.cur = if pos < self.anchor { a } else { b };
                } else {
                    self.cur = pos;
                }
                self.blink = now;
                // am Rand weiter rollen
                let r = ui.clip_rect();
                if p.y > r.bottom() - 20.0 || p.y < r.top() + 20.0 {
                    ui.scroll_with_delta(Vec2::new(0.0, if p.y < r.top() + 20.0 { 12.0 } else { -12.0 }));
                    ui.ctx().request_repaint();
                }
            }
        }
        // Strg+Klick auf Link öffnet ihn
        if resp.clicked() && mods.command {
            if let Some(l) = self.td.paras.get(self.cur.p).and_then(|q| q.style_at(self.cur.o + 1).link.or(q.style_at(self.cur.o).link)) {
                sh.platform.open_url(&l);
            }
        }
    }

    fn keys(&mut self, ui: &mut egui::Ui, id: egui::Id, sh: &mut Shared, crect: Rect) {
        let now = sh.now;
        let ctx = ui.ctx().clone();
        // IME/Bildschirmtastatur
        ctx.output_mut(|o| {
            o.ime = Some(egui::output::IMEOutput { rect: ui.clip_rect(), cursor_rect: crect, purpose: Default::default(), should_interrupt_composition: false });
        });
        let _ = ui.memory(|m| m.owns_ime_events(id));
        let events = ui.input(|i| i.events.clone());
        for ev in events {
            match ev {
                Event::Text(t) => {
                    if t != "\n" && t != "\r" && !t.chars().all(|c| c.is_control()) {
                        self.clear_preedit(now);
                        self.insert(&t, now);
                    }
                }
                Event::Copy => self.run(Cmd::Copy, sh, &ctx),
                Event::Cut => self.run(Cmd::Cut, sh, &ctx),
                Event::Paste(s) => {
                    self.clear_preedit(now);
                    self.paste(&s, now)
                }
                Event::Ime(ime) => self.ime(ime, now),
                Event::Key { key, pressed: true, modifiers, .. } => self.key(key, modifiers, sh, &ctx),
                _ => {}
            }
        }
    }

    fn clear_preedit(&mut self, now: f64) {
        if let Some((a, b)) = self.preedit.take() {
            self.td.delete(a, b);
            self.cur = a;
            self.anchor = a;
            self.changed(now);
        }
    }

    fn ime(&mut self, ime: egui::ImeEvent, now: f64) {
        use egui::ImeEvent::*;
        match ime {
            Preedit { text, .. } => {
                if text.is_empty() && self.preedit.is_none() {
                    return;
                }
                if self.preedit.is_none() {
                    // Auswahl wird beim Vorschlag ersetzt
                    if self.has_sel() {
                        self.begin(super::Edit::Type, now);
                        self.delete_sel();
                    }
                }
                self.clear_preedit(now);
                if !text.is_empty() {
                    let st = self.pending.clone().unwrap_or_else(|| self.td.style_at(self.cur));
                    let a = self.cur;
                    let b = self.td.insert_text(a, &text, &st);
                    self.preedit = Some((a, b));
                    self.cur = b;
                    self.anchor = b;
                    self.rev += 1;
                }
            }
            Commit(t) => {
                if t.is_empty() && self.preedit.is_none() {
                    return;
                }
                self.clear_preedit(now);
                if t != "\n" && !t.is_empty() {
                    self.insert(&t, now);
                }
            }
            DeleteSurrounding { before_chars, after_chars } => {
                self.clear_preedit(now);
                let mut a = self.cur;
                for _ in 0..before_chars {
                    a = self.td.prev(a);
                }
                let mut b = self.cur;
                for _ in 0..after_chars {
                    b = self.td.next(b);
                }
                if a != b {
                    self.begin(super::Edit::Delete, now);
                    self.cur = self.td.delete(a, b);
                    self.anchor = self.cur;
                    self.changed(now);
                }
            }
            _ => {}
        }
    }

    fn key(&mut self, key: Key, m: Modifiers, sh: &mut Shared, ctx: &egui::Context) {
        let now = sh.now;
        let cmd = m.command;
        let lay_page_lines = (self.lay.page.h / 16.0) as i32;
        let shortcut = match (cmd, m.shift, m.alt, key) {
            (true, false, false, Key::B) => Some(Cmd::Bold),
            (true, false, false, Key::I) => Some(Cmd::Italic),
            (true, false, false, Key::U) => Some(Cmd::Underline),
            (true, false, false, Key::Z) => Some(Cmd::Undo),
            (true, true, false, Key::Z) | (true, false, false, Key::Y) => Some(Cmd::Redo),
            (true, false, false, Key::A) => Some(Cmd::SelectAll),
            (true, false, false, Key::K) => Some(Cmd::Link),
            (true, false, false, Key::F) => Some(Cmd::Find),
            (true, false, false, Key::H) => Some(Cmd::Replace),
            (true, false, false, Key::P) => Some(Cmd::Print),
            (true, false, false, Key::Enter) => Some(Cmd::PageBreak),
            (true, false, false, Key::Period) => Some(Cmd::Sup),
            (true, false, false, Key::Comma) => Some(Cmd::Sub),
            (true, true, false, Key::Period) => Some(Cmd::FontUp),
            (true, true, false, Key::Comma) => Some(Cmd::FontDown),
            (true, false, false, Key::Space) => Some(Cmd::ClearFmt),
            (true, true, false, Key::Num7) => Some(Cmd::Numbers),
            (true, true, false, Key::Num8) => Some(Cmd::Bullets),
            (true, true, false, Key::L) => Some(Cmd::Align(Align::Left)),
            (true, true, false, Key::E) => Some(Cmd::Align(Align::Center)),
            (true, true, false, Key::R) => Some(Cmd::Align(Align::Right)),
            (true, true, false, Key::J) => Some(Cmd::Align(Align::Justify)),
            (true, true, false, Key::C) => Some(Cmd::WordCount),
            (false, true, true, Key::Num5) => Some(Cmd::Strike),
            (true, _, true, Key::Num0) => Some(Cmd::Kind(Kind::Body)),
            (true, _, true, Key::Num1) => Some(Cmd::Kind(Kind::H1)),
            (true, _, true, Key::Num2) => Some(Cmd::Kind(Kind::H2)),
            (true, _, true, Key::Num3) => Some(Cmd::Kind(Kind::H3)),
            (true, false, false, Key::Plus | Key::Equals) => Some(Cmd::ZoomIn),
            (true, false, false, Key::Minus) => Some(Cmd::ZoomOut),
            (true, false, false, Key::Num0) => Some(Cmd::ZoomReset),
            (false, false, false, Key::F1) => Some(Cmd::Help),
            (false, false, false, Key::F7) => Some(Cmd::Spell),
            (false, false, false, Key::F9) => Some(Cmd::UpdateFields),
            (true, false, true, Key::M) => Some(Cmd::Comment),
            (true, false, true, Key::F) => Some(Cmd::Footnote),
            (false, true, true, Key::X) => Some(Cmd::IndexMark),
            (false, false, true, Key::Equals) => Some(Cmd::Formula),
            // Bild aus der Zwischenablage (Text kommt als Paste-Ereignis)
            (true, false, false, Key::V) if matches!(sh.platform.clipboard_get(), Some(crate::platform::ClipData::Image { .. })) => Some(Cmd::Paste),
            _ => None,
        };
        if let Some(c) = shortcut {
            self.run(c, sh, ctx);
            return;
        }
        if cmd && m.shift && key == Key::P {
            self.palette = Some((String::new(), 0));
            return;
        }
        let ext = m.shift;
        let goal = |w: &Writer| w.goal_x.unwrap_or_else(|| w.lay.caret(w.cur).1);
        match key {
            Key::ArrowLeft | Key::ArrowRight => {
                let right = key == Key::ArrowRight;
                let p = if self.has_sel() && !ext && !cmd {
                    let (a, b) = self.sel();
                    if right { b } else { a }
                } else if cmd || m.alt {
                    if right { self.td.word_right(self.cur) } else { self.td.word_left(self.cur) }
                } else if right {
                    self.td.next(self.cur)
                } else {
                    self.td.prev(self.cur)
                };
                self.set_cur(p, ext, now);
                self.goal_x = None;
            }
            Key::ArrowUp | Key::ArrowDown | Key::PageUp | Key::PageDown => {
                let dir = if matches!(key, Key::ArrowUp | Key::PageUp) { -1 } else { 1 };
                let n = if matches!(key, Key::PageUp | Key::PageDown) { lay_page_lines.max(1) } else { 1 };
                let gx = goal(self);
                self.layout();
                let mut p = self.cur;
                for _ in 0..n {
                    match self.lay.vertical(p, gx, dir) {
                        Some(q) => p = q,
                        None => {
                            p = if dir < 0 { Pos::default() } else { self.td.end() };
                            break;
                        }
                    }
                }
                self.set_cur(p, ext, now);
                self.goal_x = Some(gx);
            }
            Key::Home | Key::End => {
                let p = if cmd {
                    if key == Key::Home { Pos::default() } else { self.td.end() }
                } else {
                    self.layout();
                    let (a, b) = self.lay.line_bounds(self.cur);
                    if key == Key::Home { a } else { b }
                };
                self.set_cur(p, ext, now);
                self.goal_x = None;
            }
            Key::Backspace => {
                self.clear_preedit(now);
                self.backspace(cmd || m.alt, now)
            }
            Key::Delete => self.delete_fwd(cmd || m.alt, now),
            Key::Enter => {
                self.clear_preedit(now);
                if ext {
                    // Umschalt+Enter: Zeile im selben Absatz gibt es nicht – neuer Absatz gleicher Art
                    self.enter(now);
                } else {
                    self.enter(now);
                }
            }
            Key::Tab => {
                let list = self.cur_para().kind.is_list();
                if list || self.has_sel() {
                    self.run(if ext { Cmd::Outdent } else { Cmd::Indent }, sh, ctx);
                } else if !ext {
                    self.insert("\t", now);
                }
            }
            Key::Escape => {
                if self.find.is_some() {
                    self.find = None;
                } else {
                    self.anchor = self.cur;
                }
            }
            _ => {}
        }
    }
}

fn paint_seg(p: &egui::Painter, geo: &Geo, page: usize, s: &Seg, y: f32, h: f32, base: f32) {
    let st = &s.st;
    if st.hl != 0 {
        p.rect_filled(Rect::from_two_pos(geo.to_screen(page, s.x0, y), geo.to_screen(page, s.x1, y + h)), CornerRadius::ZERO, c(st.hl));
    }
    let bl = y + base + st.dy;
    let col = c(st.color);
    let font = font_id(st.font, st.size * geo.k);
    for (x, w) in &s.words {
        let at = geo.to_screen(page, *x, bl);
        word(p, at.x, at.y, w, font.clone(), col);
    }
    let th = ((st.size * 0.06).max(0.5) * geo.k).max(1.0);
    for (on, dy) in [(st.u, st.size * 0.12), (st.s, -st.size * 0.28)] {
        if on {
            let a = geo.to_screen(page, s.x0, bl + dy);
            let b = geo.to_screen(page, s.x1, bl + dy);
            p.rect_filled(Rect::from_min_max(a, Pos2::new(b.x, a.y + th)), CornerRadius::ZERO, col);
        }
    }
    let _ = geo.pw;
}

impl Writer {
    /// Textur eines Bildes (einmal dekodiert, danach aus dem Zwischenspeicher).
    fn texture(&mut self, ctx: &egui::Context, f: &str) -> Option<egui::TextureHandle> {
        if let Some(t) = self.tex.get(f) {
            return Some(t.clone());
        }
        let pm = owndraw_core::office::render::decode(self.doc.images.files.get(f)?)?;
        let img = egui::ColorImage::from_rgba_premultiplied([pm.width() as usize, pm.height() as usize], pm.data());
        let t = ctx.load_texture(format!("wimg-{f}"), img, egui::TextureOptions::LINEAR);
        self.tex.insert(f.to_string(), t.clone());
        Some(t)
    }

    /// Textur eines Objekts in Bildschirmbreite `px` (zwischengespeichert, bis sich das Objekt ändert).
    fn obj_texture(&mut self, ctx: &egui::Context, p: usize, px: f32) -> Option<egui::TextureHandle> {
        use std::hash::{Hash, Hasher};
        let o = self.td.paras.get(p)?.obj.as_ref()?;
        let mut hs = std::collections::hash_map::DefaultHasher::new();
        o.to_json().to_string().hash(&mut hs);
        ((px / 32.0).round() as i64).hash(&mut hs);
        let key = hs.finish();
        if let Some((k, t)) = self.x.tex.get(&p) {
            if *k == key {
                return Some(t.clone());
            }
        }
        let scale = (px / o.w.max(1.0)).clamp(0.2, 6.0) * ctx.pixels_per_point().max(1.0);
        let pm = owndraw_core::office::render::object_pixmap(o, scale)?;
        let img = egui::ColorImage::from_rgba_premultiplied([pm.width() as usize, pm.height() as usize], pm.data());
        let t = ctx.load_texture(format!("wobj-{p}"), img, egui::TextureOptions::LINEAR);
        self.x.tex.insert(p, (key, t.clone()));
        Some(t)
    }

    /// Kontrollkästchen und Auswahlfelder reagieren auf Klick.
    fn click_field(&mut self, now: f64) {
        use owndraw_core::office::objects::Field;
        for at in [self.cur, self.td.prev(self.cur)] {
            match tools::field_at(&self.td, at) {
                Some(Field::Check(b)) => {
                    self.begin(super::Edit::Other, now);
                    tools::set_field_at(&mut self.td, at, Field::Check(!b));
                    self.changed(now);
                    return;
                }
                Some(Field::Choice(o, i)) => {
                    self.begin(super::Edit::Other, now);
                    let n = o.len().max(1);
                    tools::set_field_at(&mut self.td, at, Field::Choice(o, (i + 1) % n));
                    self.changed(now);
                    return;
                }
                Some(Field::Note(t)) => {
                    self.x.dlg = Some(super::extra::XDlg::Note { at, text: t, new: false });
                    return;
                }
                _ => {}
            }
        }
    }

    /// Eingabefeld über der offenen Tabellenzelle.
    fn cell_overlay(&mut self, ui: &mut egui::Ui, geo: &Geo, lay: &owndraw_core::office::Layout, sh: &mut Shared) {
        let Some(mut e) = self.x.cell.clone() else { return };
        let Some(l) = lay.lines.iter().find(|l| l.para == e.p && l.row.as_ref().is_some_and(|r| r.r == e.r && !r.repeat)) else {
            self.x.cell = None;
            return;
        };
        let Some(cb) = l.row.as_ref().and_then(|r| r.cells.get(e.c)) else { return };
        let k = geo.k;
        let r = Rect::from_two_pos(geo.to_screen(l.page, cb.x, l.y), geo.to_screen(l.page, cb.x + cb.w, l.y + l.h));
        let id = egui::Id::new(("wcell", e.p, e.r, e.c));
        let (tab, back, esc) = ui.input(|i| (i.key_pressed(Key::Tab) && !i.modifiers.shift, i.key_pressed(Key::Tab) && i.modifiers.shift, i.key_pressed(Key::Escape)));
        let font = font_id(cb.font, cb.size * k);
        let te = egui::TextEdit::multiline(&mut e.text).id(id).font(font).frame(egui::Frame::NONE).margin(egui::Margin::same((4.0 * k) as i8)).desired_width(r.width()).text_color(c(cb.color)).lock_focus(true);
        ui.painter().rect_stroke(r, CornerRadius::ZERO, Stroke::new(2.0, c(style::pal().accent)), egui::StrokeKind::Inside);
        let resp = ui.put(r, te);
        if e.focus {
            resp.request_focus();
            e.focus = false;
        }
        let has = resp.has_focus();
        if self.x.cell.as_ref().is_some_and(|x| x.p == e.p && x.r == e.r && x.c == e.c) {
            if resp.changed() {
                // Tab-Zeichen nicht in die Zelle
                e.text = e.text.replace('\t', "");
            }
            self.x.cell = Some(e.clone());
            if resp.changed() {
                self.commit_cell(sh.now);
            }
        }
        if has && (tab || back) {
            if let Some(x) = self.x.cell.as_mut() {
                x.text = x.text.replace('\t', "");
            }
            self.cell_step(back, sh.now);
            ui.memory_mut(|m| m.request_focus(egui::Id::new(("wcell", e.p, self.x.cell.as_ref().map(|x| x.r).unwrap_or(0), self.x.cell.as_ref().map(|x| x.c).unwrap_or(0)))));
        } else if esc {
            self.commit_cell(sh.now);
            self.x.cell = None;
            self.focus_req = true;
        }
    }
}
