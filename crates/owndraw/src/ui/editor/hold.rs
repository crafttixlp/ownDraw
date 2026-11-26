//! Langes Drücken auf ein Objekt (Finger, Stift oder Maus, mit jedem Werkzeug): Objekt auswählen
//! und beim Loslassen das erweiterte Kontextmenü wie beim Auswahlwerkzeug öffnen.

use super::{extras, Editor};
use egui::{Pos2, TouchPhase};
use owndraw_core::board::{self, Action, Motion, ToolType};

/// Haltezeit in Sekunden (länger als das Halten zum Einrasten von Formen, 0,45 s).
pub(super) const HOLD: f64 = 0.6;
/// Erlaubte Bewegung während des Haltens (Punkte).
const SLOP: f32 = 10.0;

#[derive(Clone, Copy)]
pub(super) struct Hold {
    /// Finger-ID bzw. [`extras::MOUSE`].
    id: u64,
    /// Startpunkt (Bildschirm) und relativ zur Zeichenfläche.
    screen: Pos2,
    rel: Pos2,
    t0: f64,
    /// Ausgelöst: weitere Ereignisse dieses Zeigers gehen nicht mehr ans Board.
    fired: bool,
}

pub(super) enum Ph {
    Down,
    Move,
    Up,
}

impl Editor {
    /// Zeigerereignis mitverfolgen. true = gehört zu einem schon ausgelösten Halten und wird verschluckt.
    pub(super) fn hold_track(&mut self, id: u64, ph: Ph, screen: Pos2, rel: Pos2, now: f64) -> bool {
        if let Some(h) = self.hold.filter(|h| h.fired) {
            if h.id != id {
                return false;
            }
            if matches!(ph, Ph::Up) {
                self.hold = None;
                // Menü erst im nächsten Bild öffnen, damit das Loslassen es nicht gleich wieder schließt
                self.hold_menu = Some(h.screen);
            }
            return true;
        }
        match ph {
            Ph::Down => {
                // nur ein einzelner Zeiger; ein zweiter Finger (Zoom) bricht ab
                let alone = id == extras::MOUSE || self.touches.is_empty();
                self.hold = alone.then_some(Hold { id, screen, rel, t0: now, fired: false });
            }
            Ph::Move => {
                if self.hold.is_some_and(|h| h.id == id && h.screen.distance(screen) > SLOP) {
                    self.hold = None;
                }
            }
            Ph::Up => {
                if self.hold.is_some_and(|h| h.id == id) {
                    self.hold = None;
                }
            }
        }
        false
    }

    /// Wann das laufende Halten fällig wird.
    pub(super) fn hold_deadline(&self) -> Option<f64> {
        self.hold.filter(|h| !h.fired).map(|h| h.t0 + HOLD)
    }

    /// Einmal pro Bild: Halten lange genug und ein Objekt darunter → laufende Aktion abbrechen, Objekt auswählen.
    pub(super) fn hold_check(&mut self, now: f64) {
        let Some(h) = self.hold.filter(|h| !h.fired && now - h.t0 >= HOLD) else { return };
        let tool = self.board.tool;
        let busy = self.text.is_some() || self.xs.crop.is_some() || self.ext_grabs(h.id);
        if tool == board::SELECT || tool == board::ERASER || busy || !self.board.hit_at_screen(h.rel.x, h.rel.y) {
            self.hold = None;
            return;
        }
        // Begonnenen Strich/Form/Tipp verwerfen
        let touch = h.id != extras::MOUSE;
        self.ext_hook(h.id, extras::Ph::Cancel, h.rel, touch);
        if touch {
            if let Some(p) = self.touches.iter().find(|t| t.0 == h.id).map(|t| t.1) {
                self.touch(h.id, TouchPhase::Cancel, p, 0.5);
            }
        } else if self.drawing {
            self.drawing = false;
            self.touch_in(&Motion::single(Action::Cancel, h.rel.x, h.rel.y, 0.5, ToolType::Stylus));
        }
        self.panning = None;
        self.pan_start = None;
        self.xs.press = None;
        self.st.tap_down = None;
        if self.board.select_at_screen(h.rel.x, h.rel.y, false) {
            self.board.events.push(board::Event::Haptic);
            self.hold = Some(Hold { fired: true, ..h });
        } else {
            self.hold = None;
        }
    }

    /// Nach dem Loslassen: Ort für das Kontextmenü (einmalig).
    pub(crate) fn take_hold_menu(&mut self) -> Option<Pos2> {
        self.hold_menu.take()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ui::editor::clip::tests::editor;
    use owndraw_core::item::{self, Item};
    use std::rc::Rc;

    fn ed_mit_rechteck() -> (Editor, Pos2) {
        let mut ed = editor(false);
        let (x, y) = (100.0, 100.0);
        ed.board.items.push(Rc::new(Item::shape(item::POLY, vec![x, y, x + 80.0, y, x + 80.0, y + 60.0, x, y + 60.0], item::INK, 2.0)));
        ed.board.invalidate();
        let (sx, sy) = (x * ed.board.sc + ed.board.tx, y * ed.board.sc + ed.board.ty);
        (ed, Pos2::new(sx, sy))
    }

    fn druecken(ed: &mut Editor, p: Pos2) {
        assert!(!ed.hold_track(extras::MOUSE, Ph::Down, p, p, 0.0));
        ed.drawing = true;
        ed.touch_in(&Motion::single(Action::Down, p.x, p.y, 0.5, ToolType::Stylus));
    }

    #[test]
    fn halten_waehlt_objekt_und_oeffnet_menue() {
        let (mut ed, p) = ed_mit_rechteck();
        ed.board.set_tool(board::PEN);
        let n = ed.board.items.len();
        druecken(&mut ed, p);
        ed.hold_check(HOLD * 0.5);
        assert!(ed.board.sel.is_empty(), "zu früh");
        ed.hold_check(HOLD + 0.01);
        assert_eq!(ed.board.tool, board::SELECT);
        assert_eq!(ed.board.sel.len(), 1);
        assert!(!ed.drawing);
        assert_eq!(ed.board.items.len(), n, "angefangener Strich verworfen");
        // Bewegen und Loslassen gehen nicht mehr ans Board, danach öffnet das Menü
        assert!(ed.hold_track(extras::MOUSE, Ph::Move, p + egui::vec2(30.0, 0.0), p, 1.0));
        assert!(ed.hold_track(extras::MOUSE, Ph::Up, p, p, 1.0));
        assert_eq!(ed.take_hold_menu(), Some(p));
        // Abwählen: zurück zum Stift
        ed.board.deselect();
        assert_eq!(ed.board.tool, board::PEN);
    }

    #[test]
    fn bewegen_oder_leere_flaeche_zeichnet_normal() {
        let (mut ed, p) = ed_mit_rechteck();
        ed.board.set_tool(board::PEN);
        druecken(&mut ed, p);
        assert!(!ed.hold_track(extras::MOUSE, Ph::Move, p + egui::vec2(20.0, 0.0), p, 0.1));
        ed.hold_check(HOLD + 0.1);
        assert!(ed.drawing && ed.board.sel.is_empty() && ed.board.tool == board::PEN);
        ed.drawing = false;
        ed.touch_in(&Motion::single(Action::Up, p.x + 20.0, p.y, 0.5, ToolType::Stylus));
        // leere Fläche: nichts passiert
        let leer = Pos2::new(500.0, 500.0);
        druecken(&mut ed, leer);
        ed.hold_check(HOLD + 1.0);
        assert!(ed.drawing && ed.board.sel.is_empty() && ed.take_hold_menu().is_none());
    }
}
