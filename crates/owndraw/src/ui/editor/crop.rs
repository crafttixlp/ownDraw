//! Bild zuschneiden: ganzes Bild mit abgedunkeltem Rest, vier Kanten- und vier Eckgriffe, Ausschnitt verschieben;
//! Fertig/Abbrechen/Zurücksetzen. Übernommen wird mit `Board::set_crop` (ein Rückgängig-Schritt).

use super::{Editor, Lay, M};
use crate::app::Shared;
use crate::layout::min_touch;
use crate::style::{self, c};
use egui::{Align2, Color32, Key, Order, Pos2, Sense, Shape, Stroke, Vec2};
use owndraw_core::board;
use owndraw_core::item::{self, Kind};

/// Kleinster Ausschnitt (Anteil des Bildes).
const MIN: f32 = 0.03;

/// Laufender Zuschnitt.
pub(super) struct Crop {
    pub id: u64,
    /// [links, oben, rechts, unten] relativ zum ganzen Bild.
    pub cr: [f32; 4],
    orig: Option<[f32; 4]>,
    /// Ganzes Bild (x, y, w, h) ungedreht, Drehpunkt und Drehung des Objekts.
    full: (f32, f32, f32, f32),
    center: (f32, f32),
    ang: f32,
    file: String,
    tex: Option<egui::TextureHandle>,
    tried: bool,
    /// Gezogener Griff (0 links, 1 oben, 2 rechts, 3 unten, 4–7 Ecken lo/ro/ru/lu, 8 verschieben), Ausschnitt und
    /// Bildpunkt beim Start.
    drag: Option<(u8, [f32; 4], (f32, f32))>,
}

/// Neuer Ausschnitt nach Ziehen von Griff `h` um (du, dv).
pub(super) fn drag_crop(h: u8, c0: [f32; 4], du: f32, dv: f32) -> [f32; 4] {
    let mut c = c0;
    if h == 8 {
        let du = du.clamp(-c0[0], 1.0 - c0[2]);
        let dv = dv.clamp(-c0[1], 1.0 - c0[3]);
        return [c0[0] + du, c0[1] + dv, c0[2] + du, c0[3] + dv];
    }
    let (l, t, r, b) = match h {
        0 => (true, false, false, false),
        1 => (false, true, false, false),
        2 => (false, false, true, false),
        3 => (false, false, false, true),
        4 => (true, true, false, false),
        5 => (false, true, true, false),
        6 => (false, false, true, true),
        _ => (true, false, false, true),
    };
    if l {
        c[0] = (c0[0] + du).clamp(0.0, c0[2] - MIN);
    }
    if r {
        c[2] = (c0[2] + du).clamp(c0[0] + MIN, 1.0);
    }
    if t {
        c[1] = (c0[1] + dv).clamp(0.0, c0[3] - MIN);
    }
    if b {
        c[3] = (c0[3] + dv).clamp(c0[1] + MIN, 1.0);
    }
    c
}

impl Editor {
    /// Startet den Zuschnitt des Bildes `id`.
    pub(super) fn start_crop(&mut self, id: u64) {
        let Some(it) = self.board.items.iter().find(|it| it.id == id) else { return };
        let Kind::Img(m) = &it.kind else { return };
        let full = m.full_rect().unwrap_or((m.x, m.y, m.w, m.h));
        let cr = m.cr.unwrap_or([0.0, 0.0, 1.0, 1.0]);
        self.xs.crop = Some(Crop { id, cr, orig: m.cr, full, center: (m.x + m.w / 2.0, m.y + m.h / 2.0), ang: m.ang, file: m.file.clone(), tex: None, tried: false, drag: None });
        self.board.set_hidden(Some(id));
        self.st.opts_open = false;
        self.st.menu_open = false;
    }

    /// Beendet den Zuschnitt (`apply` = übernehmen) und wählt das Bild wieder aus.
    pub(super) fn end_crop(&mut self, apply: bool) {
        let Some(cs) = self.xs.crop.take() else { return };
        self.board.set_hidden(None);
        if apply {
            let v = item::valid_crop(cs.cr);
            if v != cs.orig {
                self.board.set_crop(cs.id, v);
            }
        }
        if self.board.tool == board::SELECT && self.board.items.iter().any(|it| it.id == cs.id) {
            self.board.select_only(cs.id);
        }
        self.board.invalidate();
    }

    fn crop_tex(&mut self, ctx: &egui::Context) {
        let Some(cs) = self.xs.crop.as_mut() else { return };
        if cs.tried {
            return;
        }
        cs.tried = true;
        let bytes = self.doc.images.files.get(&cs.file).cloned().or_else(|| std::fs::read(self.img_dir.join(&cs.file)).ok());
        if let Some(img) = bytes.and_then(|b| image::load_from_memory(&b).ok()) {
            let rgba = img.to_rgba8();
            let ci = egui::ColorImage::from_rgba_unmultiplied([rgba.width() as usize, rgba.height() as usize], rgba.as_raw());
            cs.tex = Some(ctx.load_texture(format!("crop-{}", cs.file), ci, egui::TextureOptions::LINEAR));
        }
    }

    /// Zuschnitt-Modus: Überlagerung über der ganzen Zeichenfläche (nimmt alle Eingaben), Knöpfe unten.
    pub(super) fn crop_ui(&mut self, ctx: &egui::Context, sh: &mut Shared, lay: &Lay) {
        let Some(id) = self.xs.crop.as_ref().map(|c| c.id) else { return };
        if !self.board.items.iter().any(|it| it.id == id) {
            // anderswo gelöscht (Live)
            self.xs.crop = None;
            self.board.set_hidden(None);
            return;
        }
        self.crop_tex(ctx);
        let canvas = lay.canvas;
        let (sc, tx, ty, cmin) = (self.board.sc, self.board.tx, self.board.ty, canvas.min);
        let cs = self.xs.crop.as_mut().unwrap();
        let (fx, fy, fw, fh) = cs.full;
        let (cx, cy, ang) = (cs.center.0, cs.center.1, cs.ang);
        // Bildanteil (u, v) -> Bildschirm und zurück
        let to_s = move |u: f32, v: f32| -> Pos2 {
            let (wx, wy) = item::rot_pt(fx + u * fw, fy + v * fh, cx, cy, ang);
            cmin + Vec2::new(wx * sc + tx, wy * sc + ty)
        };
        let to_uv = move |p: Pos2| -> (f32, f32) {
            let (wx, wy) = ((p.x - cmin.x - tx) / sc, (p.y - cmin.y - ty) / sc);
            let (lx, ly) = item::rot_pt(wx, wy, cx, cy, -ang);
            ((lx - fx) / fw.max(1e-6), (ly - fy) / fh.max(1e-6))
        };
        let mut act = 0u8;
        egui::Area::new(egui::Id::unique("crop-overlay")).order(Order::Foreground).fixed_pos(canvas.min).show(ctx, |ui| {
            let (rect, resp) = ui.allocate_exact_size(canvas.size(), Sense::click_and_drag());
            let pt = ui.painter_at(rect);
            let cr = cs.cr;
            let quad = |l: f32, t: f32, r: f32, b: f32| vec![to_s(l, t), to_s(r, t), to_s(r, b), to_s(l, b)];
            // ganzes Bild
            match &cs.tex {
                Some(tex) => {
                    let mut mesh = egui::Mesh::with_texture(tex.id());
                    for (u, v) in [(0.0, 0.0), (1.0, 0.0), (1.0, 1.0), (0.0, 1.0)] {
                        mesh.vertices.push(egui::epaint::Vertex { pos: to_s(u, v), uv: Pos2::new(u, v), color: Color32::WHITE });
                    }
                    mesh.indices.extend_from_slice(&[0, 1, 2, 0, 2, 3]);
                    pt.add(Shape::mesh(mesh));
                }
                None => {
                    pt.add(Shape::convex_polygon(quad(0.0, 0.0, 1.0, 1.0), Color32::from_gray(128), Stroke::NONE));
                }
            }
            // Rest abdunkeln
            let dim = Color32::from_black_alpha(150);
            for q in [quad(0.0, 0.0, 1.0, cr[1]), quad(0.0, cr[3], 1.0, 1.0), quad(0.0, cr[1], cr[0], cr[3]), quad(cr[2], cr[1], 1.0, cr[3])] {
                pt.add(Shape::convex_polygon(q, dim, Stroke::NONE));
            }
            let acc = c(style::pal().accent);
            let mut border = quad(cr[0], cr[1], cr[2], cr[3]);
            border.push(border[0]);
            pt.add(Shape::line(border, Stroke::new(2.0, Color32::WHITE)));
            // Griffe
            let (mu, mv) = ((cr[0] + cr[2]) / 2.0, (cr[1] + cr[3]) / 2.0);
            let handles: [(u8, f32, f32); 8] = [(0, cr[0], mv), (1, mu, cr[1]), (2, cr[2], mv), (3, mu, cr[3]), (4, cr[0], cr[1]), (5, cr[2], cr[1]), (6, cr[2], cr[3]), (7, cr[0], cr[3])];
            for (h, u, v) in handles {
                let p = to_s(u, v);
                let r = if h >= 4 { 7.0 } else { 5.5 };
                pt.circle_filled(p, r, Color32::WHITE);
                pt.circle_stroke(p, r, Stroke::new(2.0, acc));
            }
            if resp.drag_started() {
                if let Some(p) = resp.interact_pointer_pos() {
                    let near = handles.iter().map(|(h, u, v)| (*h, to_s(*u, *v).distance(p))).filter(|(_, d)| *d < 24.0).min_by(|a, b| a.1.total_cmp(&b.1)).map(|x| x.0);
                    let uv = to_uv(p);
                    let inside = uv.0 >= cr[0] && uv.0 <= cr[2] && uv.1 >= cr[1] && uv.1 <= cr[3];
                    cs.drag = near.or(inside.then_some(8)).map(|h| (h, cr, uv));
                }
            }
            if resp.dragged() {
                if let (Some((h, c0, uv0)), Some(p)) = (cs.drag, resp.interact_pointer_pos()) {
                    let uv = to_uv(p);
                    cs.cr = drag_crop(h, c0, uv.0 - uv0.0, uv.1 - uv0.1);
                }
            }
            if resp.drag_stopped() {
                cs.drag = None;
            }
            if resp.double_clicked() {
                act = 1;
            }
        });
        // Knöpfe
        let h = min_touch(sh.env.touch).max(36.0);
        Editor::float(ctx, "crop-bar", Pos2::new(canvas.center().x, canvas.bottom() - M), Align2::CENTER_BOTTOM, canvas.shrink(M), Order::Tooltip, |ui| {
            style::pill(18).show(ui, |ui| {
                ui.horizontal(|ui| {
                    if style::plain_btn(ui, "Abbrechen", h).clicked() {
                        act = 2;
                    }
                    if style::plain_btn(ui, "Zurücksetzen", h).on_hover_text("Ganzes Bild").clicked() {
                        act = 3;
                    }
                    if style::primary_btn(ui, "Fertig", h).clicked() {
                        act = 1;
                    }
                });
            });
        });
        if !ctx.egui_wants_keyboard_input() {
            if ctx.input(|i| i.key_pressed(Key::Escape)) {
                act = 2;
            } else if ctx.input(|i| i.key_pressed(Key::Enter)) {
                act = 1;
            }
        }
        match act {
            1 => self.end_crop(true),
            2 => self.end_crop(false),
            3 => {
                if let Some(cs) = self.xs.crop.as_mut() {
                    cs.cr = [0.0, 0.0, 1.0, 1.0];
                }
            }
            _ => {}
        }
        if self.xs.crop.is_some() {
            ctx.request_repaint();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn griffe() {
        let c0 = [0.1, 0.1, 0.9, 0.9];
        assert_eq!(drag_crop(0, c0, -0.5, 0.0), [0.0, 0.1, 0.9, 0.9]);
        let c = drag_crop(6, c0, -2.0, -2.0);
        assert!((c[2] - (0.1 + MIN)).abs() < 1e-6 && (c[3] - (0.1 + MIN)).abs() < 1e-6);
        let m = drag_crop(8, c0, 0.5, -0.5);
        assert!((m[0] - 0.2).abs() < 1e-6 && m[1].abs() < 1e-6 && (m[2] - 1.0).abs() < 1e-6);
    }

    #[test]
    fn zuschneiden_uebernehmen() {
        let mut ed = crate::ui::editor::clip::tests::editor(false);
        let rgba: Vec<u8> = vec![200; 20 * 10 * 4];
        crate::ui::editor::clip::insert_clip(&mut ed, crate::platform::ClipData::Image { w: 20, h: 10, rgba }, Some((50.0, 50.0))).unwrap();
        let id = ed.board.items[0].id;
        ed.board.set_tool(board::SELECT);
        ed.start_crop(id);
        assert_eq!(ed.board.hidden, Some(id));
        ed.xs.crop.as_mut().unwrap().cr = [0.0, 0.0, 0.5, 1.0];
        ed.end_crop(true);
        assert_eq!(ed.board.hidden, None);
        assert!(matches!(&ed.board.items[0].kind, Kind::Img(m) if m.cr == Some([0.0, 0.0, 0.5, 1.0])));
        assert_eq!(ed.board.sel, vec![id]);
        // Abbrechen ändert nichts
        ed.start_crop(id);
        ed.xs.crop.as_mut().unwrap().cr = [0.0, 0.0, 1.0, 1.0];
        ed.end_crop(false);
        assert!(matches!(&ed.board.items[0].kind, Kind::Img(m) if m.cr.is_some()));
    }
}
