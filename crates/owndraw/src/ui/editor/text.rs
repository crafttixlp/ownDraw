//! Texteingabe direkt auf dem Blatt (Text und Haftnotizen) mit Cursor am Ort.
//!
//! Bestätigen: Tippen/Klicken daneben, Esc oder Strg+Enter. Enter = neue Zeile.

use super::Editor;
use crate::style::{self, c};
use egui::{CornerRadius, Stroke, Vec2};
use owndraw_core::{board, item};
use std::sync::Arc;

pub struct TextEdState {
    /// Rechteck in Weltkoordinaten: x, y, Schriftgröße, Breite (Notiz), Mindesthöhe (Notiz).
    pub r: (f32, f32, f32, f32, f32),
    pub note: bool,
    pub edit: Option<u64>,
    pub text: String,
    pub focus: bool,
    /// Textfarbe (Weltfarbe des Objekts).
    pub color: u32,
    pub bg: u32,
    /// Danebengetippt: im nächsten Frame übernehmen.
    pub commit: bool,
    /// Tabellenzelle (id, Zeile, Spalte) statt Text/Notiz (table.rs).
    pub cell: Option<(u64, usize, usize)>,
}

/// Zeilenhöhe wie beim Zeichnen auf dem Blatt (TextLayout, Faktor 1,15).
const LINE: f32 = 1.15;

impl Editor {
    /// Neuer Text am Weltpunkt.
    pub(super) fn start_text(&mut self, wx: f32, wy: f32, note: bool) {
        let b = &self.board;
        let r = b.new_note_rect(wx, wy, note);
        let bg = if note { board::note_bg(b.color) } else { 0 };
        self.text = Some(TextEdState { r, note, edit: None, text: String::new(), focus: true, color: b.color, bg, commit: false, cell: None });
    }

    /// Abnahme-Hilfe (demo.rs): neuer Text mit Inhalt.
    pub fn demo_text(&mut self, wx: f32, wy: f32, t: &str) {
        self.start_text(wx, wy, false);
        if let Some(te) = self.text.as_mut() {
            te.text = t.trim_end_matches('|').to_string();
        }
    }

    /// Abnahme-Hilfe (demo.rs): Text an laufende Eingabe anhängen.
    pub fn demo_append(&mut self, t: &str) {
        if let Some(te) = self.text.as_mut() {
            te.text.push_str(t);
        }
    }

    pub fn edit_note(&mut self, id: u64) {
        let Some(it) = self.board.items.iter().find(|it| it.id == id) else { return };
        let Some(n) = it.as_note() else { return };
        let st = TextEdState { r: (n.x, n.y, n.size, n.w, n.mh), note: n.bg != 0, edit: Some(id), text: n.text.clone(), focus: true, color: it.color, bg: n.bg, commit: false, cell: None };
        self.text = Some(st);
        self.board.set_hidden(Some(id));
    }

    pub(super) fn text_editor(&mut self, ctx: &egui::Context, keymap: &crate::settings::Settings) {
        let Some(te) = self.text.as_mut() else { return };
        let b = &self.board;
        let canvas = self.canvas;
        let (x, y, size, w, mh) = te.r;
        let sc = b.sc;
        let fpx = (size * sc).clamp(4.0, 400.0);
        let pad = if te.note { size * 0.7 * sc } else { 0.0 };
        let pos = canvas.min + Vec2::new(x * sc + b.tx, y * sc + b.ty);
        let fg = if te.note { 0xFF1B1B1Fu32 } else { item::ink(te.color, b.dark) };
        let cell = te.cell.is_some();
        let paper = b.bg_color();
        let inner_w = if cell {
            (w * sc).max(fpx)
        } else if te.note {
            ((w * sc) - 2.0 * pad).max(fpx)
        } else {
            // freier Text wächst mit der längsten Zeile (wie auf dem Blatt)
            let mw = te.text.split('\n').map(|l| owndraw_core::text::measure(l, fpx)).fold(0.0f32, f32::max);
            mw.max(fpx * 4.0) + fpx
        };
        let min_h = if te.note || cell { mh * sc } else { 0.0 };
        let mut done = te.commit || ctx.input_mut(|i| super::keys::consume(i, keymap, "text_done"));
        let pal = style::pal();
        let hint = if cell { "Zelle …" } else if te.note { "Notiz schreiben …" } else { "Text schreiben …" };
        let hint_col = if te.note { 0x801F2025u32 } else { (pal.dim & 0x00FF_FFFF) | 0x9900_0000 };
        let accent = pal.accent;
        let (note, bg) = (te.note, te.bg);
        let mut layouter = move |ui: &egui::Ui, buf: &dyn egui::TextBuffer, wrap: f32| -> Arc<egui::Galley> {
            let mut job = egui::text::LayoutJob::simple(buf.as_str().to_owned(), egui::FontId::proportional(fpx), c(fg), wrap);
            for s in job.sections.iter_mut() {
                s.format.line_height = Some(fpx * LINE);
            }
            ui.fonts_mut(|f| f.layout_job(job))
        };
        let area = egui::Area::new(egui::Id::unique("text-inline")).order(egui::Order::Foreground).fixed_pos(pos).show(ctx, |ui| {
            let frame = if note {
                egui::Frame::new()
                    .fill(c(bg))
                    .corner_radius(CornerRadius::same((size * 0.4 * sc).clamp(0.0, 255.0) as u8))
                    .inner_margin(egui::Margin::same(pad.round().clamp(0.0, 127.0) as i8))
                    .shadow(egui::Shadow { offset: [0, 3], blur: 10, spread: 0, color: egui::Color32::from_black_alpha(60) })
            } else if cell {
                // Zelle: Blatt-Hintergrund deckt den gezeichneten Zelltext ab
                egui::Frame::new().fill(c(paper)).stroke(Stroke::new(1.0, c((accent & 0x00FF_FFFF) | 0x8800_0000)))
            } else {
                // dezenter Rahmen, damit man sieht, wo geschrieben wird
                egui::Frame::new().stroke(Stroke::new(1.0, c((accent & 0x00FF_FFFF) | 0x8800_0000))).outer_margin(egui::Margin::same(0)).inner_margin(egui::Margin::ZERO)
            };
            frame.show(ui, |ui| {
                ui.set_min_size(Vec2::new(inner_w, (min_h - 2.0 * pad).max(fpx * LINE)));
                let ed = egui::TextEdit::multiline(&mut te.text)
                    .hint_text(egui::RichText::new(hint).size(fpx).color(c(hint_col)))
                    .font(egui::FontId::proportional(fpx))
                    .text_color(c(fg))
                    .desired_rows(1)
                    .desired_width(inner_w)
                    .margin(egui::Margin::ZERO)
                    .frame(egui::Frame::NONE)
                    .background_color(egui::Color32::TRANSPARENT)
                    .layouter(&mut layouter);
                let r = ui.add(ed);
                if te.focus {
                    r.request_focus();
                    te.focus = false;
                }
            });
        });
        // Klick ins Leere außerhalb (z. B. Leisten) beendet ebenfalls
        if !done && ctx.input(|i| i.pointer.any_pressed()) {
            if let Some(p) = ctx.input(|i| i.pointer.interact_pos()) {
                if !area.response.rect.contains(p) && canvas.contains(p) && ctx.layer_id_at(p).map(|l| l.order == egui::Order::Background).unwrap_or(true) {
                    done = true;
                }
            }
        }
        if done {
            self.finish_text();
        }
    }

    /// Übernimmt die laufende Eingabe (leerer Text löscht ein vorhandenes Objekt).
    pub(super) fn finish_text(&mut self) {
        let Some(te) = self.text.take() else { return };
        self.board.set_hidden(None);
        let t = te.text.trim_end();
        let t = t.trim_start_matches('\n');
        if let Some((id, r, cc)) = te.cell {
            self.board.set_cell_text(id, r, cc, t);
            self.board.invalidate();
            return;
        }
        match te.edit {
            Some(id) if t.trim().is_empty() => self.board.delete_item(id),
            Some(id) => {
                let orig = self.board.items.iter().find(|it| it.id == id).and_then(|it| it.as_note().map(|n| n.text.clone()));
                if orig.as_deref() != Some(t) {
                    self.board.edit_note(id, t);
                } else if self.board.tool == board::SELECT {
                    // unverändert: wieder auswählen, damit das Menü bleibt
                    self.board.select_only(id);
                }
                if self.board.tool != board::SELECT {
                    self.board.deselect();
                }
            }
            None if t.trim().is_empty() => {}
            None => {
                let col = self.board.color;
                self.board.color = te.color;
                self.board.add_note_rect(t, te.r, te.note);
                self.board.color = col;
            }
        }
        self.board.invalidate();
    }
}
