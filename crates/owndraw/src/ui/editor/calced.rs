//! Eingabefeld des Werkzeugs „Rechnen“: eine Rechenzeile in einfacher Schreibweise (`a: 1 N`, `M = a·b`, `M =`).
//!
//! Tippen auf freie Fläche öffnet ein einzeiliges Feld. Enter legt aus der Zeile ein Formel-Objekt an
//! (LaTeX-Quelltext aus `calc::to_latex`) und öffnet das Feld für die nächste Zeile darunter. Leere Zeile + Enter
//! oder Esc schließt. Tippen auf eine vorhandene Formel öffnet sie zum Bearbeiten (`calc::to_plain`).
//! Das Ausrechnen und Zeichnen der Ergebnisse übernimmt der Kern.

use super::mathed::insert_snippet;
use super::Editor;
use crate::app::Shared;
use crate::layout::min_touch;
use crate::style::{self, c};
use egui::text::{CCursor, CCursorRange};
use egui::{Align2, Order, Vec2};
use owndraw_core::calc;

pub struct CalcEd {
    /// Linke obere Ecke (Welt) der Zeile, die gerade getippt wird.
    pub at: (f32, f32),
    pub edit: Option<u64>,
    pub src: String,
    pub size: f32,
    pub color: u32,
    focus: bool,
}

/// Schnellknöpfe: Zeichen, die auf der Tastatur fehlen.
const QUICK: [(&str, &str); 8] = [
    ("·", "Malpunkt"),
    ("²", "Quadrat"),
    ("√", "Wurzel"),
    ("π", "Pi"),
    ("°", "Grad"),
    ("Ω", "Ohm"),
    ("µ", "Mikro"),
    ("Δ", "Delta"),
];

/// Zeilenhöhe auf dem Blatt (Vielfaches der Formelgröße).
const LINE: f32 = 1.6;

impl Editor {
    /// Neue Zeile am Weltpunkt bzw. vorhandene Formel bearbeiten (Tippen mit dem Werkzeug „Rechnen“).
    pub(super) fn start_calc(&mut self, sh: &mut Shared, wx: f32, wy: f32) {
        if !self.board.calc.on {
            sh.toast("Rechnen ist ausgeschaltet (Einstellungen → Werkzeugleisten). Zeilen eingeben geht trotzdem.");
        }
        let size = self.board.math_size();
        let ed = match self.board.math_at(wx, wy) {
            Some(id) => {
                let Some(it) = self.board.items.iter().find(|it| it.id == id) else { return };
                let Some(m) = it.as_math() else { return };
                CalcEd { at: (m.x, m.y), edit: Some(id), src: calc::to_plain(&m.src), size: m.size, color: it.color, focus: true }
            }
            None => CalcEd { at: (wx, wy), edit: None, src: String::new(), size, color: self.board.color, focus: true },
        };
        self.calc = Some(ed);
    }

    /// Schließt das Feld.
    pub(super) fn finish_calc(&mut self) {
        self.calc = None;
        self.board.invalidate();
    }

    /// Legt die Zeile als Formel an (bzw. speichert sie). Rückgabe: Feld bleibt offen (nächste Zeile).
    fn commit_calc(&mut self) -> bool {
        let Some(ce) = self.calc.as_mut() else { return false };
        let text = ce.src.trim().to_string();
        if text.is_empty() {
            return false;
        }
        let latex = calc::to_latex(&text);
        match ce.edit {
            Some(id) => {
                self.board.set_math_src(id, &latex);
                if self.board.tool == owndraw_core::board::SELECT {
                    self.board.select_only(id);
                }
                false
            }
            None => {
                let col = self.board.color;
                self.board.color = ce.color;
                self.board.add_math(ce.at.0, ce.at.1, ce.size, &latex);
                self.board.color = col;
                ce.at.1 += ce.size * LINE;
                ce.src.clear();
                ce.focus = true;
                true
            }
        }
    }

    pub(super) fn calc_editor(&mut self, ctx: &egui::Context, sh: &Shared) {
        let Some(ce) = self.calc.as_mut() else { return };
        let b = &self.board;
        let canvas = self.canvas;
        let pal = style::pal();
        let h = min_touch(sh.env.touch).max(30.0);
        let w = (canvas.width() - 24.0).clamp(240.0, 420.0);
        // unter (bzw. über) der Zeile, innerhalb der Zeichenfläche
        let sp = canvas.min + Vec2::new(ce.at.0 * b.sc + b.tx, ce.at.1 * b.sc + b.ty);
        let below = sp.y + ce.size * b.sc * LINE + 8.0;
        let (pos, pivot) = if below + 130.0 < canvas.bottom() { (egui::pos2(sp.x, below), Align2::LEFT_TOP) } else { (egui::pos2(sp.x, sp.y - 8.0), Align2::LEFT_BOTTOM) };
        let mut cancel = ctx.input_mut(|i| super::keys::consume(i, &sh.settings, "cancel"));
        let mut enter = false;
        let id = egui::Id::unique("calc-src");
        let r = Editor::float(ctx, "calc-ed", pos, pivot, canvas.shrink(8.0), Order::Foreground, |ui| {
            style::card().show(ui, |ui| {
                ui.set_width(w);
                ui.horizontal(|ui| {
                    ui.label(egui::RichText::new(if ce.edit.is_some() { "Zeile bearbeiten" } else { "Rechnen" }).size(15.0).strong());
                    ui.label(egui::RichText::new("Enter = nächste Zeile").size(12.0).color(c(pal.dim)));
                });
                ui.add_space(4.0);
                let mut snip: Option<&str> = None;
                ui.horizontal_wrapped(|ui| {
                    ui.spacing_mut().item_spacing = Vec2::new(4.0, 4.0);
                    for (ch, tip) in QUICK {
                        if style::chip(ui, ch, false, h * 0.9).on_hover_text(tip).clicked() {
                            snip = Some(ch);
                        }
                    }
                });
                ui.add_space(4.0);
                let resp = ui.add(
                    egui::TextEdit::singleline(&mut ce.src)
                        .id(id)
                        .font(egui::FontId::proportional(16.0))
                        .hint_text("z. B. a: 2 kN  ·  M = a·b  ·  M =")
                        .margin(Vec2::new(10.0, 8.0))
                        .desired_width(f32::INFINITY),
                );
                enter = resp.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter));
                if ce.focus || enter {
                    resp.request_focus();
                    ce.focus = false;
                }
                if let Some(s) = snip {
                    let mut st = egui::text_edit::TextEditState::load(ui.ctx(), id).unwrap_or_default();
                    let n = ce.src.chars().count();
                    let sel = st.cursor.char_range().map(|r| (r.primary.index.0, r.secondary.index.0)).unwrap_or((n, n));
                    let cur = insert_snippet(&mut ce.src, sel, s);
                    st.cursor.set_char_range(Some(CCursorRange::one(CCursor::new(cur))));
                    st.store(ui.ctx(), id);
                    resp.request_focus();
                }
                ui.add_space(4.0);
                ui.horizontal(|ui| {
                    if style::accent_btn(ui, "Zeile einfügen", h).on_hover_text("Enter").clicked() {
                        enter = true;
                    }
                    if style::plain_btn(ui, "Fertig", h).on_hover_text("Esc").clicked() {
                        cancel = true;
                    }
                });
            });
        });
        self.st.popup_rects.push(r);
        if enter {
            // leere Zeile + Enter schließt
            if !self.commit_calc() {
                self.finish_calc();
            }
        } else if cancel {
            self.finish_calc();
        }
    }
}

/// Optionen im Popover des Werkzeugs.
pub(super) fn options(ui: &mut egui::Ui, ed: &mut Editor, sh: &mut Shared) {
    let h = min_touch(sh.env.touch);
    let pal = style::pal();
    let cp = &mut sh.settings.calc;
    ui.label(egui::RichText::new("Aufs Blatt tippen und eine Rechenzeile schreiben, z. B. a: 2 kN, M = a·b, M =. Auf eine Zeile tippen: bearbeiten.").size(12.0).color(c(pal.dim)));
    if !cp.on {
        ui.label(egui::RichText::new("Rechnen ist ausgeschaltet (Einstellungen).").size(12.0).color(c(pal.danger_text)));
    }
    ui.add_space(4.0);
    ui.checkbox(&mut cp.steps, "Rechenweg anzeigen");
    style::caption(ui, "Winkel");
    ui.horizontal_wrapped(|ui| {
        for (deg, l) in [(true, "Grad"), (false, "Bogenmaß")] {
            if style::chip(ui, l, cp.deg == deg, h).clicked() {
                cp.deg = deg;
            }
        }
    });
    style::caption(ui, "Gültige Stellen");
    let mut d = cp.digits.clamp(2, 10) as u32;
    ui.spacing_mut().slider_width = 180.0;
    if ui.add(egui::Slider::new(&mut d, 2..=10)).changed() {
        cp.digits = d as u8;
    }
    style::caption(ui, "Dezimalzeichen");
    ui.horizontal_wrapped(|ui| {
        for (comma, l) in [(true, "Komma"), (false, "Punkt")] {
            if style::chip(ui, l, cp.comma == comma, h).clicked() {
                cp.comma = comma;
            }
        }
    });
    let _ = ed;
}
