//! Kommentar-Stecknadel: Tippen setzt eine Nadel und öffnet das Kommentarfenster (Text, Antworten, Löschen).
//! Antippen einer vorhandenen Nadel öffnet es mit jedem Werkzeug. Nadeln sind Objekte und laufen über Live mit;
//! mit dem Recht „Nur ansehen“ lässt sich lesen, aber nicht schreiben.

use super::{Editor, Lay, M};
use crate::app::Shared;
use crate::layout::min_touch;
use crate::style::{self, c};
use egui::{Align2, Key, Order, Pos2};
use owndraw_core::item::{Kind, Pin, PIN_R};

/// Offenes Kommentarfenster.
pub(super) struct PinWin {
    pub id: u64,
    text: String,
    /// Text beim Öffnen bzw. letzter Stand des Boards (fremde Änderungen übernehmen, solange nichts getippt ist).
    orig: String,
    reply: String,
    /// Eben gesetzt: leer geschlossen wird die Nadel wieder entfernt.
    fresh: bool,
    focus: bool,
}

/// TT.MM.JJJJ HH:MM (UTC).
fn when(ms: i64) -> String {
    if ms <= 0 {
        return String::new();
    }
    let s = owndraw_core::doc::rfc3339(ms);
    format!("{}.{}.{} {}", &s[8..10], &s[5..7], &s[0..4], &s[11..16])
}

impl Editor {
    fn pin_of(&self, id: u64) -> Option<Pin> {
        match &self.board.items.iter().find(|it| it.id == id)?.kind {
            Kind::Pin(p) => Some(p.clone()),
            _ => None,
        }
    }

    pub(super) fn open_pin(&mut self, id: u64) {
        if self.xs.pin.as_ref().is_some_and(|w| w.id == id) {
            return;
        }
        self.close_pin();
        let Some(p) = self.pin_of(id) else { return };
        self.xs.pin = Some(PinWin { id, text: p.text.clone(), orig: p.text, reply: String::new(), fresh: false, focus: false });
    }

    /// Neue Nadel am Weltpunkt (Werkzeug Kommentar).
    pub(super) fn new_pin(&mut self, wx: f32, wy: f32) {
        self.close_pin();
        if !self.x_can_write() {
            self.x_notice("Du kannst dieses Board nur ansehen");
            return;
        }
        let author = self.x_author();
        match self.board.add_pin(wx, wy, &author, owndraw_core::store::now_ms(), "") {
            Some(id) => self.xs.pin = Some(PinWin { id, text: String::new(), orig: String::new(), reply: String::new(), fresh: true, focus: true }),
            None => self.x_notice("Die Ebene ist gesperrt. Entsperre sie, um einen Kommentar zu setzen."),
        }
    }

    /// Schließt das Fenster und übernimmt den Text (eine leere, eben gesetzte Nadel wird entfernt).
    pub(super) fn close_pin(&mut self) {
        let Some(w) = self.xs.pin.take() else { return };
        let Some(p) = self.pin_of(w.id) else { return };
        let text = w.text.trim();
        if w.fresh && text.is_empty() && p.replies.is_empty() && p.text.is_empty() {
            self.board.delete_item(w.id);
        } else if self.x_can_write() && text != p.text {
            self.board.set_pin_text(w.id, text);
        }
    }

    pub(super) fn pin_ui(&mut self, ctx: &egui::Context, sh: &mut Shared, lay: &Lay) {
        let Some(id) = self.xs.pin.as_ref().map(|w| w.id) else { return };
        let Some(p) = self.pin_of(id) else {
            // gelöscht (auch von anderen)
            self.xs.pin = None;
            return;
        };
        let can = self.x_can_write();
        let me = self.x_author();
        let head = self.w2s(p.x, p.y - PIN_R * 2.2);
        let off = PIN_R * self.board.sc + 8.0;
        let canvas = lay.canvas;
        let h = min_touch(sh.env.touch).max(32.0);
        let dim = c(style::pal().dim);
        let w = self.xs.pin.as_mut().unwrap();
        if w.text == w.orig && p.text != w.orig {
            w.text = p.text.clone();
            w.orig = p.text.clone();
        }
        enum A {
            None,
            Close,
            Reply,
            Delete,
        }
        let mut act = A::None;
        let pos = Pos2::new(head.x + off, head.y - 12.0);
        Editor::float(ctx, "pin-win", pos, Align2::LEFT_TOP, canvas.shrink(M), Order::Foreground, |ui| {
            style::pill(14).show(ui, |ui| {
                egui::Frame::new().inner_margin(egui::Margin::same(10)).show(ui, |ui| {
                    ui.set_width(270.0);
                    ui.horizontal(|ui| {
                        ui.label(egui::RichText::new(if p.author.is_empty() { "Kommentar" } else { &p.author }).strong().size(14.0));
                        ui.label(egui::RichText::new(when(p.time)).size(11.5).color(dim));
                    });
                    if can {
                        let r = ui.add(egui::TextEdit::multiline(&mut w.text).hint_text("Kommentar schreiben …").desired_rows(2).desired_width(f32::INFINITY));
                        if w.focus {
                            r.request_focus();
                            w.focus = false;
                        }
                    } else if !p.text.is_empty() {
                        ui.label(&p.text);
                    }
                    if !p.replies.is_empty() {
                        ui.add_space(4.0);
                        egui::ScrollArea::vertical().max_height(220.0).auto_shrink([false, true]).show(ui, |ui| {
                            for r in &p.replies {
                                ui.separator();
                                ui.horizontal(|ui| {
                                    ui.label(egui::RichText::new(&r.author).strong().size(13.0));
                                    ui.label(egui::RichText::new(when(r.time)).size(11.0).color(dim));
                                });
                                ui.label(&r.text);
                            }
                        });
                    }
                    if can {
                        ui.add_space(6.0);
                        let r = ui.add(egui::TextEdit::singleline(&mut w.reply).hint_text("Antworten …").desired_width(f32::INFINITY));
                        if r.lost_focus() && ui.input(|i| i.key_pressed(Key::Enter)) && !w.reply.trim().is_empty() {
                            act = A::Reply;
                        }
                    } else {
                        ui.label(egui::RichText::new("Nur ansehen: Antworten ist nicht möglich.").size(12.0).color(dim));
                    }
                    ui.add_space(6.0);
                    ui.horizontal(|ui| {
                        if can && style::primary_btn(ui, "Antworten", h).clicked() && !w.reply.trim().is_empty() {
                            act = A::Reply;
                        }
                        if style::plain_btn(ui, "Schließen", h).clicked() {
                            act = A::Close;
                        }
                        if can {
                            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                                if style::danger_btn(ui, "Löschen", h).clicked() {
                                    act = A::Delete;
                                }
                            });
                        }
                    });
                });
            });
        });
        if matches!(act, A::None) && ctx.input(|i| i.key_pressed(Key::Escape)) {
            act = A::Close;
        }
        match act {
            A::None => {}
            A::Close => self.close_pin(),
            A::Reply => {
                let t = std::mem::take(&mut w.reply);
                w.fresh = false;
                // eigener Text zuerst, damit die Antwort nicht den Kommentar überschreibt
                let text = w.text.trim().to_string();
                if text != p.text {
                    self.board.set_pin_text(id, &text);
                    if let Some(w) = self.xs.pin.as_mut() {
                        w.orig = text;
                    }
                }
                self.board.reply_pin(id, &me, owndraw_core::store::now_ms(), t.trim());
            }
            A::Delete => {
                self.xs.pin = None;
                self.board.delete_item(id);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ui::editor::clip::tests::editor;

    #[test]
    fn kommentar_text_und_antwort() {
        let mut ed = editor(false);
        ed.new_pin(50.0, 50.0);
        let id = ed.xs.pin.as_ref().unwrap().id;
        ed.xs.pin.as_mut().unwrap().text = "Bitte prüfen".into();
        ed.close_pin();
        assert_eq!(ed.pin_of(id).unwrap().text, "Bitte prüfen");
        ed.board.reply_pin(id, "Ich", 1, "Ok");
        ed.open_pin(id);
        assert_eq!(ed.xs.pin.as_ref().unwrap().text, "Bitte prüfen");
        ed.close_pin();
        assert_eq!(ed.pin_of(id).unwrap().replies.len(), 1);
        assert_eq!(when(0), "");
        assert_eq!(when(86_400_000), "02.01.1970 00:00");
    }
}
