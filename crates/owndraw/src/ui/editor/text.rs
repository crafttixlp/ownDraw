//! Texteingabe für Text und Notizen (schwebendes Eingabefeld).

use super::{Editor, Lay};
use crate::layout::FormFactor;
use crate::style::{self, c};
use egui::{CornerRadius, Key, Stroke, Vec2};
use owndraw_core::board;

pub struct TextEdState {
    pub wx: f32,
    pub wy: f32,
    pub note: bool,
    pub edit: Option<u64>,
    pub text: String,
    pub focus: bool,
}

impl Editor {
    pub(super) fn text_editor(&mut self, ctx: &egui::Context, lay: &Lay) {
        let Some(te) = self.text.as_mut() else { return };
        let rect = lay.canvas;
        let pal = style::pal();
        let b = &self.board;
        let w = (300.0f32).min(rect.width() - 20.0);
        let sx = te.wx * b.sc + b.tx;
        let sy = te.wy * b.sc + b.ty;
        let x = sx.clamp(12.0, (rect.width() - w - 12.0).max(12.0));
        let top = 70.0;
        let y = if lay.env.ff == FormFactor::Compact {
            // Handy: oben, damit die Bildschirmtastatur nichts verdeckt
            top + if lay.row2 { 60.0 } else { 0.0 }
        } else {
            sy.clamp(top, (rect.height() / 2.0 - 200.0).max(top))
        };
        let note_col = board::note_bg(b.color);
        let (bg, fg, hint) = if te.note { (note_col, 0xFF1F2025u32, "Notiz schreiben …") } else { (pal.card, pal.text, "Text schreiben …") };
        let mut done = ctx.input_mut(|i| i.consume_key(egui::Modifiers::COMMAND, Key::Enter) || i.consume_key(egui::Modifiers::CTRL, Key::Enter));
        let mut cancel = ctx.input_mut(|i| i.consume_key(egui::Modifiers::NONE, Key::Escape));
        let mh = lay.env.btn;
        let r = egui::Area::new(egui::Id::unique("text-ed")).order(egui::Order::Foreground).fixed_pos(rect.min + Vec2::new(x, y)).show(ctx, |ui| {
            egui::Frame::new()
                .fill(c(bg))
                .stroke(Stroke::new(1.0, c(if te.note { 0x22000000 } else { pal.line })))
                .corner_radius(CornerRadius::same(18))
                .inner_margin(egui::Margin::same(12))
                .shadow(egui::Shadow { offset: [0, 4], blur: 18, spread: 0, color: egui::Color32::from_black_alpha(110) })
                .show(ui, |ui| {
                    ui.set_width(w - 24.0);
                    let hint_col = if te.note { 0x801F2025 } else { 0x808B8D98 };
                    let ed = egui::TextEdit::multiline(&mut te.text)
                        .hint_text(egui::RichText::new(hint).color(c(hint_col)))
                        .font(egui::FontId::proportional(17.0))
                        .text_color(c(fg))
                        .desired_rows(4)
                        .desired_width(w - 24.0)
                        .frame(egui::Frame::NONE)
                        .background_color(egui::Color32::TRANSPARENT);
                    let r = egui::ScrollArea::vertical().max_height(17.0 * 1.35 * 4.0 + 8.0).show(ui, |ui| ui.add(ed)).inner;
                    if te.focus {
                        r.request_focus();
                        te.focus = false;
                    }
                    ui.add_space(8.0);
                    ui.horizontal(|ui| {
                        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                            let (okc, cc) = if te.note { (0xFF3B4BD6u32, 0xFF4A4D5Cu32) } else { (pal.accent, pal.dim) };
                            let hint = if lay.env.hints() { "Fertig (Strg+↵)" } else { "Fertig" };
                            if ui.add(egui::Button::new(egui::RichText::new(hint).strong().color(c(okc))).fill(egui::Color32::TRANSPARENT).min_size(Vec2::new(0.0, mh))).clicked() {
                                done = true;
                            }
                            if ui.add(egui::Button::new(egui::RichText::new("Abbrechen").color(c(cc))).fill(egui::Color32::TRANSPARENT).min_size(Vec2::new(0.0, mh))).clicked() {
                                cancel = true;
                            }
                        });
                    });
                });
        });
        let _ = r;
        if done {
            let te = self.text.take().unwrap();
            let t = te.text.trim();
            if !t.is_empty() {
                match te.edit {
                    Some(id) => self.board.edit_note(id, t),
                    None => self.board.add_note(te.wx, te.wy, t, te.note),
                }
            }
        } else if cancel {
            self.text = None;
        }
    }
}
