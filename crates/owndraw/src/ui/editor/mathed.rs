//! Formel-Editor (Werkzeug „Formel“): Quelltext im LaTeX-Stil, Live-Vorschau, Schnellknöpfe.
//!
//! Neues Fenster: Tippen mit dem Werkzeug auf freie Fläche. Bearbeiten: Tippen auf eine vorhandene Formel
//! (Werkzeug „Formel“) oder erneutes Tippen auf eine ausgewählte Formel. Strg+Enter übernimmt, Esc bricht ab.

use super::Editor;
use crate::layout::min_touch;
use crate::style::{self, c};
use egui::text::{CCursor, CCursorRange};
use egui::{Align2, Order, Vec2};
use owndraw_core::canvas::Canvas;
use owndraw_core::tiny_skia::Pixmap;
use owndraw_core::{board, item, math};

pub struct MathEd {
    /// Linke obere Ecke (Welt) einer neuen Formel.
    pub at: (f32, f32),
    pub edit: Option<u64>,
    pub src: String,
    pub size: f32,
    pub color: u32,
    focus: bool,
    /// Vorschau: Textur und der Stand, für den sie gilt (Quelltext, Farbe, Pixel je Punkt).
    tex: Option<(egui::TextureHandle, String, u32, f32, Vec2)>,
}

/// Schnellknöpfe: Beschriftung, Einfügetext (▮ = Auswahl bzw. Cursor), Tooltip.
const QUICK: [(&str, &str, &str); 10] = [
    ("a/b", "\\frac{▮}{}", "Bruch \\frac{a}{b}"),
    ("√", "\\sqrt{▮}", "Wurzel \\sqrt{x}"),
    ("³√", "\\sqrt[]{▮}", "n-te Wurzel \\sqrt[n]{x}"),
    ("x²", "^{▮}", "Hochstellen ^{…}"),
    ("x₂", "_{▮}", "Tiefstellen _{…}"),
    ("∑", "\\sum_{▮}^{}", "Summe \\sum_{i=1}^{n}"),
    ("∫", "\\int_{▮}^{}", "Integral \\int_a^b"),
    ("( )", "\\left( ▮ \\right)", "Mitwachsende Klammern \\left( … \\right)"),
    ("lim", "\\lim_{▮}", "Grenzwert \\lim_{x \\to 0}"),
    ("Text", "\\text{▮}", "Normaler Text \\text{…}"),
];

const GREEK: [(&str, &str); 24] = [
    ("α", "alpha"),
    ("β", "beta"),
    ("γ", "gamma"),
    ("δ", "delta"),
    ("ε", "epsilon"),
    ("ζ", "zeta"),
    ("η", "eta"),
    ("θ", "theta"),
    ("λ", "lambda"),
    ("μ", "mu"),
    ("ν", "nu"),
    ("ξ", "xi"),
    ("π", "pi"),
    ("ρ", "rho"),
    ("σ", "sigma"),
    ("τ", "tau"),
    ("φ", "phi"),
    ("ω", "omega"),
    ("Γ", "Gamma"),
    ("Δ", "Delta"),
    ("Θ", "Theta"),
    ("Σ", "Sigma"),
    ("Φ", "Phi"),
    ("Ω", "Omega"),
];

const SYMBOLS: [(&str, &str); 12] = [
    ("·", "cdot"),
    ("×", "times"),
    ("±", "pm"),
    ("≤", "le"),
    ("≥", "ge"),
    ("≠", "ne"),
    ("≈", "approx"),
    ("∞", "infty"),
    ("→", "rightarrow"),
    ("⇒", "Rightarrow"),
    ("∈", "in"),
    ("∂", "partial"),
];

/// Fügt `snip` an der Cursorposition ein (▮ = markierter Text); liefert den neuen Cursor (Zeichenindex).
pub fn insert_snippet(src: &mut String, sel: (usize, usize), snip: &str) -> usize {
    let n = src.chars().count();
    let (a, b) = (sel.0.min(sel.1).min(n), sel.0.max(sel.1).min(n));
    let ba = src.char_indices().nth(a).map_or(src.len(), |(i, _)| i);
    let bb = src.char_indices().nth(b).map_or(src.len(), |(i, _)| i);
    let picked: String = src[ba..bb].to_string();
    let (pre, post) = snip.split_once('▮').unwrap_or((snip, ""));
    let ins = format!("{pre}{picked}{post}");
    src.replace_range(ba..bb, &ins);
    let pre_n = pre.chars().count();
    if picked.is_empty() {
        a + pre_n
    } else {
        // Auswahl steht schon drin: Cursor in das nächste leere {} bzw. ans Ende
        let after = a + pre_n + picked.chars().count();
        match post.find("{}") {
            Some(i) => after + post[..i].chars().count() + 1,
            None => after + post.chars().count(),
        }
    }
}

impl Editor {
    /// Neue Formel am Weltpunkt (Tippen mit dem Werkzeug „Formel“).
    pub(super) fn start_math(&mut self, wx: f32, wy: f32) {
        let size = self.board.math_size();
        self.math = Some(MathEd { at: (wx, wy - size * 0.7), edit: None, src: String::new(), size, color: self.board.color, focus: true, tex: None });
    }

    /// Vorhandene Formel bearbeiten.
    pub fn edit_math(&mut self, id: u64) {
        let Some(it) = self.board.items.iter().find(|it| it.id == id) else { return };
        let Some(m) = it.as_math() else { return };
        self.math = Some(MathEd { at: (m.x, m.y), edit: Some(id), src: m.src.clone(), size: m.size, color: it.color, focus: true, tex: None });
    }

    /// Formel-Fenster mit vorgegebenem Quelltext (Wechsel aus der Eingabe am Ort, mathin.rs).
    pub(super) fn open_latex(&mut self, edit: Option<u64>, at: (f32, f32), size: f32, color: u32, src: &str) {
        self.math = Some(MathEd { at, edit, src: src.to_string(), size, color, focus: true, tex: None });
    }

    /// Abnahme-Hilfe (demo.rs): Formel-Fenster mit Inhalt.
    pub fn demo_math(&mut self, wx: f32, wy: f32, src: &str) {
        self.start_math(wx, wy);
        if let Some(me) = self.math.as_mut() {
            me.src = src.to_string();
        }
    }

    /// Übernimmt (`ok`) oder verwirft die Eingabe. Leerer Quelltext löscht eine vorhandene Formel.
    pub(super) fn finish_math(&mut self, ok: bool) {
        let Some(me) = self.math.take() else { return };
        if !ok {
            self.board.invalidate();
            return;
        }
        let src = me.src.trim();
        match me.edit {
            Some(id) if src.is_empty() => self.board.delete_item(id),
            Some(id) => {
                self.board.set_math_src(id, src);
                if self.board.tool == board::SELECT {
                    self.board.select_only(id);
                }
            }
            None if src.is_empty() => {}
            None => {
                let col = self.board.color;
                self.board.color = me.color;
                self.board.add_math(me.at.0, me.at.1, me.size, src);
                self.board.color = col;
            }
        }
        self.board.invalidate();
    }

    pub(super) fn math_editor(&mut self, ctx: &egui::Context, sh: &crate::app::Shared) {
        let Some(me) = self.math.as_mut() else { return };
        let b = &self.board;
        let canvas = self.canvas;
        let ppp = ctx.pixels_per_point();
        let pal = style::pal();
        let h = min_touch(sh.env.touch).max(30.0);
        let w = (canvas.width() - 24.0).clamp(240.0, 460.0);
        // unter (bzw. über) der Stelle auf dem Blatt, innerhalb der Zeichenfläche
        let sp = canvas.min + Vec2::new(me.at.0 * b.sc + b.tx, me.at.1 * b.sc + b.ty);
        let below = sp.y + me.size * b.sc * 1.6 + 8.0;
        let (pos, pivot) = if below + 260.0 < canvas.bottom() { (egui::pos2(sp.x, below), Align2::LEFT_TOP) } else { (egui::pos2(sp.x, sp.y - 8.0), Align2::LEFT_BOTTOM) };
        let mut done: Option<bool> = None;
        if ctx.input_mut(|i| super::keys::consume(i, &sh.settings, "text_done")) {
            done = Some(true);
        } else if ctx.input_mut(|i| super::keys::consume(i, &sh.settings, "cancel")) {
            done = Some(false);
        }
        let id = egui::Id::unique("math-src");
        let fg = item::ink(me.color, pal.dark);
        let r = Editor::float(ctx, "math-ed", pos, pivot, canvas.shrink(8.0), Order::Foreground, |ui| {
            style::card().show(ui, |ui| {
                ui.set_width(w);
                ui.horizontal(|ui| {
                    ui.label(egui::RichText::new(if me.edit.is_some() { "Formel bearbeiten" } else { "Formel" }).size(15.0).strong());
                    ui.label(egui::RichText::new("LaTeX-Schreibweise").size(12.0).color(c(pal.dim)));
                });
                ui.add_space(4.0);
                // Schnellknöpfe
                let mut snip: Option<String> = None;
                ui.horizontal_wrapped(|ui| {
                    ui.spacing_mut().item_spacing = Vec2::new(4.0, 4.0);
                    for (l, s, tip) in QUICK {
                        if style::chip(ui, l, false, h * 0.9).on_hover_text(tip).clicked() {
                            snip = Some(s.to_string());
                        }
                    }
                    ui.menu_button(egui::RichText::new("α β γ"), |ui| {
                        ui.set_max_width(260.0);
                        ui.horizontal_wrapped(|ui| {
                            for (g, n) in GREEK {
                                if ui.add(egui::Button::new(egui::RichText::new(g).size(17.0)).min_size(Vec2::splat(32.0))).on_hover_text(format!("\\{n}")).clicked() {
                                    snip = Some(format!("\\{n} ▮"));
                                    ui.close();
                                }
                            }
                        });
                    });
                    ui.menu_button(egui::RichText::new("≤ ≠ ∞"), |ui| {
                        ui.set_max_width(260.0);
                        ui.horizontal_wrapped(|ui| {
                            for (g, n) in SYMBOLS {
                                if ui.add(egui::Button::new(egui::RichText::new(g).size(17.0)).min_size(Vec2::splat(32.0))).on_hover_text(format!("\\{n}")).clicked() {
                                    snip = Some(format!("\\{n} ▮"));
                                    ui.close();
                                }
                            }
                        });
                    });
                });
                ui.add_space(4.0);
                let resp = ui.add(
                    egui::TextEdit::multiline(&mut me.src)
                        .id(id)
                        .font(egui::TextStyle::Monospace)
                        .hint_text("z. B. \\frac{a}{b} + \\sqrt{x^2+1}")
                        .desired_rows(2)
                        .desired_width(f32::INFINITY),
                );
                if me.focus {
                    resp.request_focus();
                    me.focus = false;
                }
                if let Some(s) = snip {
                    let mut st = egui::text_edit::TextEditState::load(ui.ctx(), id).unwrap_or_default();
                    let n = me.src.chars().count();
                    let sel = st.cursor.char_range().map(|r| (r.primary.index.0, r.secondary.index.0)).unwrap_or((n, n));
                    let cur = insert_snippet(&mut me.src, sel, &s);
                    st.cursor.set_char_range(Some(CCursorRange::one(CCursor::new(cur))));
                    st.store(ui.ctx(), id);
                    resp.request_focus();
                }
                // Vorschau
                ui.add_space(6.0);
                let key = (me.src.clone(), fg, ppp);
                let stale = me.tex.as_ref().map(|(_, s, col, p, _)| (s, *col, *p) != (&key.0, key.1, key.2)).unwrap_or(true);
                let f = math::layout(&me.src, 26.0);
                if stale {
                    let scale = (ppp * (w - 8.0) / f.w).min(ppp).max(0.05);
                    let (pw, ph) = ((f.w * scale).ceil().max(1.0) as u32, (f.h * scale).ceil().max(1.0) as u32);
                    if let Some(mut pm) = Pixmap::new(pw.min(4096), ph.min(4096)) {
                        {
                            let mut cv = Canvas::new(&mut pm);
                            cv.scale(scale, scale);
                            f.draw(&mut cv, fg);
                        }
                        let img = egui::ColorImage::from_rgba_premultiplied([pm.width() as usize, pm.height() as usize], pm.data());
                        let tex = ui.ctx().load_texture("math-preview", img, egui::TextureOptions::LINEAR);
                        me.tex = Some((tex, key.0, key.1, key.2, Vec2::new(pm.width() as f32 / ppp, pm.height() as f32 / ppp)));
                    }
                }
                egui::Frame::new().fill(c(pal.sunken)).corner_radius(8).inner_margin(4).show(ui, |ui| {
                    ui.set_min_size(Vec2::new(ui.available_width(), 40.0));
                    if me.src.trim().is_empty() {
                        ui.label(egui::RichText::new("Vorschau").color(c(pal.dim)));
                    } else if let Some((t, .., size)) = &me.tex {
                        ui.add(egui::Image::new((t.id(), *size)));
                    }
                });
                for e in f.errors.iter().take(3) {
                    ui.label(egui::RichText::new(e).size(12.0).color(c(math::ERR_COLOR)));
                }
                ui.add_space(6.0);
                ui.horizontal(|ui| {
                    if style::accent_btn(ui, if me.edit.is_some() { "Übernehmen" } else { "Einfügen" }, h).on_hover_text("Strg+Enter").clicked() {
                        done = Some(true);
                    }
                    if style::plain_btn(ui, "Abbrechen", h).on_hover_text("Esc").clicked() {
                        done = Some(false);
                    }
                });
            });
        });
        self.st.popup_rects.push(r);
        if let Some(ok) = done {
            self.finish_math(ok);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::insert_snippet;

    #[test]
    fn schnellknoepfe_einfuegen() {
        let mut s = String::from("x+");
        let c = insert_snippet(&mut s, (2, 2), "\\frac{▮}{}");
        assert_eq!(s, "x+\\frac{}{}");
        assert_eq!(c, 8);
        // Auswahl wird eingepackt, Cursor ins nächste {}
        let mut s = String::from("ab");
        let c = insert_snippet(&mut s, (0, 1), "\\frac{▮}{}");
        assert_eq!(s, "\\frac{a}{}b");
        assert_eq!(c, 9);
        let mut s = String::from("α");
        let c = insert_snippet(&mut s, (5, 9), "^{▮}");
        assert_eq!((s.as_str(), c), ("α^{}", 3));
    }
}
