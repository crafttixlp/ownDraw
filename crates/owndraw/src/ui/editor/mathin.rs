//! Formeln direkt auf dem Blatt bearbeiten (ohne Formel-Fenster).
//!
//! Die Formel wird an ihrer Stelle zu einem Eingabefeld in einfacher Schreibweise (`M = a·b`, `sqrt(2)/3`,
//! `x^2`); darunter stehen Vorschau und Schnellknöpfe. Enter, Esc oder Tippen daneben übernimmt. Formeln, die
//! sich nicht einfach schreiben lassen (Integral, Summe …), zeigen ihren LaTeX-Quelltext im selben Feld.
//! Baut auf der Texteingabe am Ort auf (`text.rs`, `TextEdState::math`).

use super::mathed::insert_snippet;
use super::text::TextEdState;
use super::Editor;
use crate::app::Shared;
use crate::layout::min_touch;
use crate::style::{self, c};
use egui::text::{CCursor, CCursorRange};
use egui::{Align2, Order, Vec2};
use owndraw_core::canvas::Canvas;
use owndraw_core::tiny_skia::Pixmap;
use owndraw_core::{board, calc, item, math};

/// Zustand der Formel-Eingabe am Ort.
pub struct MathIn {
    /// Text beim Öffnen (unverändert = nichts speichern).
    orig: String,
    /// Vorschau: Textur und der Stand, für den sie gilt (Quelltext, Farbe, Pixel je Punkt, Größe).
    tex: Option<(egui::TextureHandle, String, u32, f32, Vec2)>,
}

/// Kennung des Eingabefelds (für die Schnellknöpfe).
pub(super) fn field_id() -> egui::Id {
    egui::Id::unique("math-inline-edit")
}

/// Getippter Text → LaTeX-Quelltext der Formel. Text mit Backslash gilt schon als LaTeX.
pub fn to_src(text: &str) -> String {
    let t = text.trim();
    if t.contains('\\') {
        t.to_string()
    } else {
        calc::to_latex(t)
    }
}

/// LaTeX-Quelltext → Text im Feld: einfache Schreibweise, wenn möglich, sonst der Quelltext selbst.
pub fn to_text(src: &str) -> String {
    let p = calc::to_plain(src);
    if p.contains('\\') {
        src.to_string()
    } else {
        p
    }
}

impl Editor {
    /// Neue Formel am Weltpunkt direkt auf dem Blatt schreiben.
    pub(super) fn start_math_inline(&mut self, wx: f32, wy: f32) {
        let size = self.board.math_size();
        let mi = MathIn { orig: String::new(), tex: None };
        self.text = Some(TextEdState { r: (wx, wy - size * 0.7, size, 0.0, 0.0), note: false, edit: None, text: String::new(), focus: true, color: self.board.color, bg: 0, commit: false, cell: None, math: Some(mi) });
    }

    /// Vorhandene Formel direkt auf dem Blatt bearbeiten.
    pub fn edit_math_inline(&mut self, id: u64) {
        let Some(it) = self.board.items.iter().find(|it| it.id == id) else { return };
        let Some(m) = it.as_math() else { return };
        let text = to_text(&m.src);
        let mi = MathIn { orig: text.clone(), tex: None };
        self.text = Some(TextEdState { r: (m.x, m.y, m.size, 0.0, 0.0), note: false, edit: Some(id), text, focus: true, color: it.color, bg: 0, commit: false, cell: None, math: Some(mi) });
        self.board.set_hidden(Some(id));
    }

    /// Übernimmt die Formel-Eingabe (Aufruf aus `finish_text`). Leerer Text löscht eine vorhandene Formel.
    pub(super) fn finish_math_inline(&mut self, te: TextEdState, mi: MathIn) {
        let t = te.text.trim();
        match te.edit {
            Some(id) if t.is_empty() => self.board.delete_item(id),
            Some(id) => {
                if t != mi.orig.trim() {
                    self.board.set_math_src(id, &to_src(t));
                }
                if self.board.tool == board::SELECT {
                    self.board.select_only(id);
                }
            }
            None if t.is_empty() => {}
            None => {
                let col = self.board.color;
                self.board.color = te.color;
                self.board.add_math(te.r.0, te.r.1, te.r.2, &to_src(t));
                self.board.color = col;
            }
        }
        self.board.invalidate();
    }

    /// Wechselt von der Eingabe am Ort ins Formel-Fenster (LaTeX), mit dem bisher Getippten.
    fn inline_to_latex(&mut self) {
        let Some(te) = self.text.take() else { return };
        self.board.set_hidden(None);
        self.open_latex(te.edit, (te.r.0, te.r.1), te.r.2, te.color, &to_src(&te.text));
    }

    /// Leiste unter dem Feld: Vorschau, Schnellknöpfe, „Fertig“, „LaTeX …“. Vor `text_editor` aufrufen.
    pub(super) fn math_inline_bar(&mut self, ctx: &egui::Context, sh: &Shared) {
        let Some(te) = self.text.as_mut() else { return };
        let (x, y, size, ..) = te.r;
        let color = te.color;
        let Some(mi) = te.math.as_mut() else { return };
        let b = &self.board;
        let canvas = self.canvas;
        let pal = style::pal();
        let ppp = ctx.pixels_per_point();
        let h = min_touch(sh.env.touch).max(30.0);
        let w = (canvas.width() - 24.0).clamp(220.0, 380.0);
        let fpx = (size * b.sc).clamp(4.0, 400.0);
        let sp = canvas.min + Vec2::new(x * b.sc + b.tx, y * b.sc + b.ty);
        let below = sp.y + fpx * 1.15 + 10.0;
        let (pos, pivot) = if below + 150.0 < canvas.bottom() { (egui::pos2(sp.x, below), Align2::LEFT_TOP) } else { (egui::pos2(sp.x, sp.y - 10.0), Align2::LEFT_BOTTOM) };
        let fg = item::ink(color, b.dark);
        let src = to_src(&te.text);
        let latex_mode = te.text.contains('\\');
        let f = math::layout(&src, 24.0);
        let stale = mi.tex.as_ref().map(|(_, s, col, p, _)| (s.as_str(), *col, *p) != (src.as_str(), fg, ppp)).unwrap_or(true);
        if stale {
            let scale = (ppp * (w - 16.0) / f.w.max(1.0)).min(ppp).max(0.05);
            let (pw, ph) = ((f.w * scale).ceil().max(1.0) as u32, (f.h * scale).ceil().max(1.0) as u32);
            if let Some(mut pm) = Pixmap::new(pw.min(4096), ph.min(4096)) {
                {
                    let mut cv = Canvas::new(&mut pm);
                    cv.scale(scale, scale);
                    f.draw(&mut cv, fg);
                }
                let img = egui::ColorImage::from_rgba_premultiplied([pm.width() as usize, pm.height() as usize], pm.data());
                let tex = ctx.load_texture("math-inline-preview", img, egui::TextureOptions::LINEAR);
                mi.tex = Some((tex, src.clone(), fg, ppp, Vec2::new(pm.width() as f32 / ppp, pm.height() as f32 / ppp)));
            }
        }
        let empty = te.text.trim().is_empty();
        let mut snip: Option<&str> = None;
        let (mut done, mut latex) = (false, false);
        let r = Editor::float(ctx, "math-inline-bar", pos, pivot, canvas.shrink(8.0), Order::Foreground, |ui| {
            style::card().show(ui, |ui| {
                ui.set_width(w);
                if !empty {
                    // Vorschau auf Blattfarbe, damit die Schriftfarbe stimmt
                    egui::Frame::new().fill(c(b.bg_color())).corner_radius(8).inner_margin(4).show(ui, |ui| {
                        ui.set_min_width(ui.available_width());
                        if let Some((t, .., sz)) = &mi.tex {
                            ui.add(egui::Image::new((t.id(), *sz)));
                        }
                    });
                    for e in f.errors.iter().take(2) {
                        ui.label(egui::RichText::new(e).size(12.0).color(c(math::ERR_COLOR)));
                    }
                    ui.add_space(4.0);
                } else {
                    ui.label(egui::RichText::new("Formel einfach tippen, z. B. M = a·b, x^2, sqrt(2)/3").size(12.0).color(c(pal.dim)));
                    ui.add_space(4.0);
                }
                if !latex_mode {
                    ui.horizontal_wrapped(|ui| {
                        ui.spacing_mut().item_spacing = Vec2::new(4.0, 4.0);
                        for (ch, tip) in super::calced::QUICK {
                            if style::chip(ui, ch, false, h * 0.9).on_hover_text(tip).clicked() {
                                snip = Some(ch);
                            }
                        }
                    });
                    ui.add_space(4.0);
                }
                ui.horizontal(|ui| {
                    if style::accent_btn(ui, "Fertig", h).on_hover_text("Enter").clicked() {
                        done = true;
                    }
                    if style::plain_btn(ui, "LaTeX …", h).on_hover_text("Im Formel-Fenster mit LaTeX-Schreibweise weiterbearbeiten").clicked() {
                        latex = true;
                    }
                });
            });
        });
        self.st.popup_rects.push(r);
        if let Some(s) = snip {
            let id = field_id();
            let mut st = egui::text_edit::TextEditState::load(ctx, id).unwrap_or_default();
            let n = te.text.chars().count();
            let sel = st.cursor.char_range().map(|r| (r.primary.index.0, r.secondary.index.0)).unwrap_or((n, n));
            let cur = insert_snippet(&mut te.text, sel, s);
            st.cursor.set_char_range(Some(CCursorRange::one(CCursor::new(cur))));
            st.store(ctx, id);
            te.focus = true;
        }
        if done {
            te.commit = true;
        }
        if latex {
            self.inline_to_latex();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{to_src, to_text};
    use crate::ui::editor::clip::tests::editor;
    use owndraw_core::board;

    fn src(ed: &crate::ui::editor::Editor, id: u64) -> String {
        ed.board.items.iter().find(|i| i.id == id).unwrap().as_math().unwrap().src.clone()
    }

    #[test]
    fn schreibweise() {
        assert_eq!(to_text("M = a \\cdot b"), "M = a·b");
        assert_eq!(to_text("x = \\frac{\\sqrt{2}}{3}"), "x = sqrt(2)/3");
        // nicht einfach schreibbar: LaTeX bleibt
        assert_eq!(to_text("\\int_a^b f(x)\\,dx"), "\\int_a^b f(x)\\,dx");
        assert_eq!(to_src("M = a·b"), "M = a \\cdot b");
        assert_eq!(to_src("\\sum_{k=1}^n k"), "\\sum_{k=1}^n k");
    }

    #[test]
    fn formel_am_ort_bearbeiten() {
        let mut ed = editor(false);
        let id = ed.board.add_math(50.0, 60.0, 30.0, "M = a \\cdot b").unwrap();
        ed.board.set_tool(board::SELECT);
        // öffnen: Formel verschwindet, Feld zeigt einfache Schreibweise, kein Formel-Fenster
        ed.edit_math_inline(id);
        assert!(ed.math.is_none());
        assert_eq!(ed.board.hidden, Some(id));
        assert_eq!(ed.text_input().as_deref(), Some("M = a·b"));
        // unverändert schließen: Quelltext bleibt
        ed.finish_text();
        assert_eq!(ed.board.hidden, None);
        assert_eq!(src(&ed, id), "M = a \\cdot b");
        // ändern = ein Schritt
        ed.edit_math_inline(id);
        ed.text.as_mut().unwrap().text = "M = a·b/2".into();
        ed.finish_text();
        assert_eq!(src(&ed, id), "M = \\frac{a \\cdot b}{2}");
        assert_eq!(ed.board.sel, vec![id]);
        // ändern = ein Rückgängig-Schritt
        ed.board.undo();
        assert_eq!(src(&ed, id), "M = a \\cdot b");
        ed.board.redo();
        // LaTeX-Formel: Quelltext im Feld, bleibt LaTeX
        let id2 = ed.board.add_math(50.0, 200.0, 30.0, "\\int_0^1 x\\,dx").unwrap();
        ed.edit_math_inline(id2);
        assert_eq!(ed.text_input().as_deref(), Some("\\int_0^1 x\\,dx"));
        ed.text.as_mut().unwrap().text = "\\int_0^2 x\\,dx".into();
        ed.finish_text();
        assert_eq!(src(&ed, id2), "\\int_0^2 x\\,dx");
        // leeren löscht
        ed.edit_math_inline(id2);
        ed.text.as_mut().unwrap().text.clear();
        ed.finish_text();
        assert!(ed.board.items.iter().all(|i| i.id != id2));
        // neue Formel
        let k = ed.board.items.len();
        ed.start_math_inline(300.0, 300.0);
        ed.text.as_mut().unwrap().text = "x^2 + 1".into();
        ed.finish_text();
        assert_eq!(ed.board.items.len(), k + 1);
        assert_eq!(ed.board.items.last().unwrap().as_math().unwrap().src, "x^{2} + 1");
    }

    #[test]
    fn wechsel_ins_latex_fenster() {
        let mut ed = editor(false);
        let id = ed.board.add_math(50.0, 60.0, 30.0, "M = a \\cdot b").unwrap();
        ed.edit_math_inline(id);
        ed.text.as_mut().unwrap().text = "M = a·c".into();
        ed.inline_to_latex();
        assert!(ed.text.is_none());
        assert_eq!(ed.board.hidden, None);
        assert_eq!(ed.math.as_ref().map(|m| (m.edit, m.src.as_str())), Some((Some(id), "M = a \\cdot c")));
    }
}
