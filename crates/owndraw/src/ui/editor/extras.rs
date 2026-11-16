//! Werkzeuge ohne eigenes Board-Werkzeug: Lineal/Geodreieck ([`super::ruler`]), Laserpointer und Platz schaffen.
//!
//! Die Zeiger-Eingabe läuft zuerst durch [`Editor::ext_hook`]: Was das Lineal, der Laser oder „Platz schaffen“
//! braucht, geht nicht ans Board (`Hook::Eat`); Striche an einer Linealkante gehen projiziert weiter (`Hook::Pass`).
//! - Laser: Leuchtspur ([`owndraw_core::laser::Laser`]), verblasst nach 1,5 s, nie gespeichert, kein Undo;
//!   live an die anderen Teilnehmer (`live.rs`, Nachricht `laser`).
//! - Platz schaffen: senkrecht ziehen; beim Loslassen `Board::make_space` (ein Rückgängig-Schritt).

use super::ruler::{self, Hit, Ruler};
use super::{Editor, Tool};
use crate::style::{self, c};
use egui::{Color32, CornerRadius, FontId, Pos2, Rect, Stroke, Ui, Vec2};
use owndraw_core::board::{self, Board};
use owndraw_core::laser::{Laser, FADE, LASER_COLOR};

/// Zeiger-Kennung der Maus (Touch-Kennungen sind kleiner).
pub const MOUSE: u64 = u64::MAX;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Ph {
    Down,
    Move,
    Up,
    Cancel,
}

pub(super) enum Hook {
    /// An das Board weitergeben (ggf. projizierter Punkt).
    Pass(Pos2),
    /// Verbraucht.
    Eat,
}

/// Laufendes „Platz schaffen“ (Weltkoordinaten).
#[derive(Clone, Copy, Debug)]
struct SpaceDrag {
    id: u64,
    page: Option<usize>,
    y0: f32,
    y1: f32,
}

pub struct Extras {
    /// Aktives Werkzeug ohne Board-Werkzeug (Laser, Platz schaffen).
    pub xtool: Option<Tool>,
    pub ruler: Ruler,
    /// Das Optionen-Popover gehört zum Lineal.
    pub ruler_pop: bool,
    /// Letztes Zeichenwerkzeug (Board), zu dem das Lineal zurückkehrt.
    last_draw: i32,
    /// Formerkennung während eines Strichs am Lineal (aus), danach zurück.
    pub saved_recog: Option<i32>,
    pub laser: Laser,
    /// Laser der anderen Teilnehmer.
    remote: Vec<(u64, Laser)>,
    laser_ptr: Option<u64>,
    /// Ausgang: fertige Abschnitte, laufender Abschnitt (erster Punkt ggf. schon gesendet).
    out: Vec<Vec<(f32, f32)>>,
    out_cur: Vec<(f32, f32)>,
    out_sent: usize,
    out_at: f64,
    space: Option<SpaceDrag>,
    /// Zeiger, die hier verbraucht werden (nicht ans Board).
    eaten: Vec<u64>,
    pub now: f64,
}

impl Default for Extras {
    fn default() -> Self {
        Extras {
            xtool: None,
            ruler: Ruler::default(),
            ruler_pop: false,
            last_draw: board::PEN,
            saved_recog: None,
            laser: Laser::new(LASER_COLOR),
            remote: Vec::new(),
            laser_ptr: None,
            out: Vec::new(),
            out_cur: Vec::new(),
            out_sent: 0,
            out_at: 0.0,
            space: None,
            eaten: Vec::new(),
            now: 0.0,
        }
    }
}

/// Werkzeuge, deren Striche am Lineal entlang laufen: Stift, Marker, Linien-Formen (Linie, Pfeile, Maßlinie).
fn guided(b: &Board) -> bool {
    match b.tool {
        board::PEN | board::MARKER => true,
        board::SHAPE => matches!(b.shape_kind, 2 | 3 | 4) || b.shape_kind == owndraw_core::item::DIM,
        _ => false,
    }
}

impl Extras {
    /// Laser-Abschnitte zum Senden (höchstens alle 40 ms; ein fortgesetzter Abschnitt beginnt mit dem letzten
    /// gesendeten Punkt, damit die Spur beim Empfänger zusammenhängt).
    #[cfg_attr(target_os = "android", allow(dead_code))]
    pub fn take_laser_out(&mut self, now: f64) -> Vec<Vec<(f32, f32)>> {
        if self.out_cur.len() > self.out_sent && now - self.out_at >= 0.04 {
            let last = *self.out_cur.last().unwrap_or(&(0.0, 0.0));
            self.out.push(std::mem::replace(&mut self.out_cur, vec![last]));
            self.out_sent = 1;
            self.out_at = now;
        }
        std::mem::take(&mut self.out)
    }

    /// Laser eines anderen Teilnehmers.
    #[cfg_attr(target_os = "android", allow(dead_code))]
    pub fn laser_in(&mut self, peer: u64, color: u32, pts: &[(f32, f32)], now: f64) {
        let i = match self.remote.iter().position(|r| r.0 == peer) {
            Some(i) => i,
            None => {
                self.remote.push((peer, Laser::new(color)));
                self.remote.len() - 1
            }
        };
        let l = &mut self.remote[i].1;
        l.color = color;
        l.add_remote(pts, now);
    }

    fn laser_pt(&mut self, x: f32, y: f32) {
        self.laser.add(x, y, self.now);
        self.out_cur.push((x, y));
    }

    fn laser_lift(&mut self) {
        self.laser.lift();
        if self.out_cur.len() > self.out_sent {
            self.out.push(std::mem::take(&mut self.out_cur));
        }
        self.out_cur.clear();
        self.out_sent = 0;
    }
}

impl Editor {
    /// Zu Beginn jedes Frames.
    pub(super) fn ext_frame(&mut self, now: f64) {
        self.ext.now = now;
        if matches!(self.board.tool, board::PEN | board::MARKER | board::SHAPE) {
            self.ext.last_draw = self.board.tool;
        }
        // Formerkennung nach einem Strich am Lineal wieder herstellen
        if self.ext.ruler.proj.is_none() {
            if let Some(r) = self.ext.saved_recog.take() {
                self.board.recog = r;
            }
        }
    }

    /// Werkzeug „Lineal“: Einblendung umschalten; beim Einblenden zurück zum vorigen Zeichenwerkzeug.
    pub(super) fn toggle_ruler(&mut self) {
        let r = &mut self.ext.ruler;
        r.visible = !r.visible;
        self.ext.ruler_pop = false;
        if r.visible {
            r.place(self.canvas.width().max(1.0), self.canvas.height().max(1.0));
            self.ext.xtool = None;
            if !matches!(self.board.tool, board::PEN | board::MARKER | board::SHAPE) {
                self.board.set_tool(self.ext.last_draw);
            }
        }
        self.board.invalidate();
    }

    /// Zeiger-Ereignis vor dem Board (`p` relativ zur Zeichenfläche, `id` = Finger bzw. [`MOUSE`]).
    pub(super) fn ext_hook(&mut self, id: u64, ph: Ph, p: Pos2, touch: bool) -> Hook {
        match ph {
            Ph::Down => {
                // weitere Finger, während hier schon ein Zeiger greift (zweiter Finger dreht das Lineal)
                if !self.ext.eaten.is_empty() {
                    if touch {
                        self.ext.ruler.add_finger(id, p);
                        self.ext.eaten.push(id);
                        return Hook::Eat;
                    }
                    return Hook::Pass(p);
                }
                let board_busy = touch && !self.touches.is_empty();
                if board_busy {
                    return Hook::Pass(p);
                }
                let draws_here = !touch || self.board.finger_draw;
                if self.ext.ruler.visible {
                    let hit = self.ext.ruler.hit(p);
                    let guide = self.ext.xtool.is_none() && guided(&self.board) && draws_here;
                    match hit {
                        Hit::Edge(..) if guide => {
                            if let Some(q) = self.ext.ruler.start_proj(id, p) {
                                // Formerkennung aus, damit die Linie nicht auf ein anderes Winkelraster springt
                                if self.ext.saved_recog.is_none() {
                                    self.ext.saved_recog = Some(self.board.recog);
                                }
                                self.board.recog = 2;
                                return Hook::Pass(q);
                            }
                        }
                        Hit::Handle | Hit::Body | Hit::Edge(_, true) => {
                            self.ext.ruler.grab(id, p, hit);
                            self.ext.eaten.push(id);
                            return Hook::Eat;
                        }
                        _ => {}
                    }
                }
                if !draws_here {
                    return Hook::Pass(p);
                }
                let (wx, wy) = self.board.screen_to_world(p.x, p.y);
                match self.ext.xtool {
                    Some(Tool::Laser) => {
                        self.ext.laser_lift();
                        self.ext.laser_ptr = Some(id);
                        self.ext.laser_pt(wx, wy);
                    }
                    Some(Tool::Space) => {
                        let page = self.board.page_at(wx, wy);
                        if self.board.pages().is_some() && page.is_none() {
                            self.board.events.push(board::Event::Notice("Zieh auf einer Seite, um Platz zu schaffen".into()));
                        } else {
                            self.ext.space = Some(SpaceDrag { id, page, y0: wy, y1: wy });
                        }
                    }
                    _ => return Hook::Pass(p),
                }
                self.ext.eaten.push(id);
                Hook::Eat
            }
            Ph::Move => {
                if self.ext.eaten.contains(&id) {
                    if self.ext.ruler.owns(id) {
                        let cfg = self.board.snap_cfg;
                        self.ext.ruler.moved(id, p, &cfg);
                    } else if self.ext.laser_ptr == Some(id) {
                        let (wx, wy) = self.board.screen_to_world(p.x, p.y);
                        self.ext.laser_pt(wx, wy);
                    } else if let Some(s) = self.ext.space.as_mut().filter(|s| s.id == id) {
                        s.y1 = (p.y - self.board.ty) / self.board.sc;
                    }
                    return Hook::Eat;
                }
                Hook::Pass(self.ext.ruler.proj_point(id, p).unwrap_or(p))
            }
            Ph::Up | Ph::Cancel => {
                if let Some(i) = self.ext.eaten.iter().position(|x| *x == id) {
                    self.ext.eaten.remove(i);
                    self.ext.ruler.released(id);
                    if self.ext.laser_ptr == Some(id) {
                        self.ext.laser_ptr = None;
                        self.ext.laser_lift();
                    }
                    if let Some(s) = self.ext.space.filter(|s| s.id == id) {
                        self.ext.space = None;
                        let dy = s.y1 - s.y0;
                        if ph == Ph::Up && (dy * self.board.sc).abs() >= 4.0 && self.board.make_space(s.page, s.y0, dy) == 0 {
                            self.board.events.push(board::Event::Notice("Darunter liegt nichts zum Verschieben".into()));
                        }
                    }
                    return Hook::Eat;
                }
                let q = self.ext.ruler.proj_point(id, p).unwrap_or(p);
                self.ext.ruler.end_proj(id);
                Hook::Pass(q)
            }
        }
    }

    /// Hält ein Werkzeug hier gerade die Maus?
    pub(super) fn ext_grabs(&self, id: u64) -> bool {
        self.ext.eaten.contains(&id)
    }

    /// Zeichnet Lineal, Laser und die Vorschau von „Platz schaffen“ über das Board.
    pub(super) fn ext_paint(&mut self, ui: &Ui, ctx: &egui::Context, rect: Rect) {
        let now = self.ext.now;
        let pt = ui.painter().with_clip_rect(rect);
        let b = &self.board;
        let w2s = |x: f32, y: f32| rect.min + Vec2::new(x * b.sc + b.tx, y * b.sc + b.ty);
        // Platz schaffen
        if let Some(s) = self.ext.space {
            let pal = style::pal();
            let (mut l, mut r, mut bottom) = (rect.left(), rect.right(), rect.bottom());
            if let Some(pr) = s.page.and_then(|i| b.page_rects().get(i).copied()) {
                l = w2s(pr.left, 0.0).x;
                r = w2s(pr.right, 0.0).x;
                bottom = w2s(0.0, pr.bottom).y;
            }
            let (y0, y1) = (w2s(0.0, s.y0).y, w2s(0.0, s.y1).y);
            let acc = c(pal.accent);
            if y1 >= y0 {
                pt.rect_filled(Rect::from_min_max(Pos2::new(l, y1), Pos2::new(r, bottom.max(y1) + (y1 - y0))), CornerRadius::ZERO, acc.gamma_multiply(0.12));
                pt.rect_filled(Rect::from_min_max(Pos2::new(l, y0), Pos2::new(r, y1)), CornerRadius::ZERO, acc.gamma_multiply(0.06));
            } else {
                pt.rect_filled(Rect::from_min_max(Pos2::new(l, y1), Pos2::new(r, y0)), CornerRadius::ZERO, c(pal.danger).gamma_multiply(0.18));
                pt.rect_filled(Rect::from_min_max(Pos2::new(l, y1), Pos2::new(r, bottom.max(y0))), CornerRadius::ZERO, acc.gamma_multiply(0.08));
            }
            pt.add(egui::Shape::dashed_line(&[Pos2::new(l, y0), Pos2::new(r, y0)], Stroke::new(1.5, c(pal.dim)), 6.0, 4.0));
            pt.line_segment([Pos2::new(l, y1), Pos2::new(r, y1)], Stroke::new(2.0, acc));
            let cm = ruler::screen_cm(y1 - y0, b.sc);
            let txt = format!("{}{}", if y1 >= y0 { "+" } else { "−" }, ruler::cm_label(cm.abs()));
            let g = pt.layout_no_wrap(txt, FontId::proportional(12.0), Color32::WHITE);
            let at = Pos2::new((l + r) / 2.0, y1 + if y1 >= y0 { 14.0 } else { -14.0 });
            let rr = Rect::from_center_size(at, g.size() + Vec2::new(12.0, 4.0));
            pt.rect_filled(rr, CornerRadius::same(8), acc);
            pt.galley(rr.min + Vec2::new(6.0, 2.0), g, Color32::WHITE);
        }
        // Laser (eigener und fremde)
        let mut visible = self.ext.laser.prune(now);
        self.ext.remote.retain_mut(|(_, l)| l.prune(now));
        visible |= !self.ext.remote.is_empty();
        let b = &self.board;
        for l in std::iter::once(&self.ext.laser).chain(self.ext.remote.iter().map(|r| &r.1)) {
            paint_laser(&pt, l, now, &|x, y| rect.min + Vec2::new(x * b.sc + b.tx, y * b.sc + b.ty));
        }
        if visible {
            ctx.request_repaint();
        }
        let _ = self.ext.laser.take_unsent();
        if !self.laser_live() {
            // ohne Live-Verbindung nichts aufheben (keine alten Spuren beim Verbinden)
            self.ext.out.clear();
            self.ext.out_sent = self.ext.out_cur.len();
        }
        // Lineal
        if self.ext.ruler.visible {
            self.ext.ruler.paint(&pt, rect.min, self.board.sc);
            if let Some(h) = self.hover {
                match self.ext.ruler.hit((h - rect.min).to_pos2()) {
                    Hit::Handle => ctx.set_cursor_icon(egui::CursorIcon::Alias),
                    Hit::Body | Hit::Edge(_, true) if !self.drawing => ctx.set_cursor_icon(egui::CursorIcon::Move),
                    _ => {}
                }
            }
        }
    }

    /// Laser-Punkte werden live gesendet.
    fn laser_live(&self) -> bool {
        #[cfg(not(target_os = "android"))]
        {
            self.live.as_ref().is_some_and(|l| l.is_live())
        }
        #[cfg(target_os = "android")]
        {
            false
        }
    }
}

/// Leuchtspur wie `Laser::draw`, mit egui (Breite in Bildschirmpunkten).
fn paint_laser(pt: &egui::Painter, l: &Laser, now: f64, to: &dyn Fn(f32, f32) -> Pos2) {
    let (r, g, b) = ((l.color >> 16) as u8, (l.color >> 8) as u8, l.color as u8);
    let mut prev: Option<Pos2> = None;
    for p in l.points() {
        let Some((x, y, t)) = *p else {
            prev = None;
            continue;
        };
        let k = (1.0 - (now - t) / FADE).clamp(0.0, 1.0) as f32;
        let q = to(x, y);
        if k > 0.0 {
            let a = prev.unwrap_or(q);
            let glow = Color32::from_rgba_unmultiplied(r, g, b, (k * 70.0) as u8);
            let core = Color32::from_rgba_unmultiplied(r, g, b, (k * 255.0) as u8);
            let (gw, cw) = (12.0 * (0.5 + 0.5 * k), 4.0 * (0.4 + 0.6 * k));
            pt.line_segment([a, q], Stroke::new(gw, glow));
            pt.line_segment([a, q], Stroke::new(cw, core));
            pt.circle_filled(q, cw / 2.0, core);
        }
        prev = Some(q);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn laser_ausgang_haengt_zusammen() {
        let mut e = Extras::default();
        e.now = 0.0;
        e.laser_pt(0.0, 0.0);
        e.laser_pt(1.0, 0.0);
        let a = e.take_laser_out(1.0);
        assert_eq!(a, vec![vec![(0.0, 0.0), (1.0, 0.0)]]);
        assert!(e.take_laser_out(2.0).is_empty(), "nichts Neues");
        e.laser_pt(2.0, 0.0);
        e.laser_lift();
        // Fortsetzung beginnt mit dem zuletzt gesendeten Punkt
        assert_eq!(e.take_laser_out(3.0), vec![vec![(1.0, 0.0), (2.0, 0.0)]]);
        e.laser_lift();
        assert!(e.take_laser_out(4.0).is_empty());
        e.laser_in(9, 0xFF00FF00, &[(0.0, 0.0), (5.0, 5.0)], 4.0);
        assert_eq!(e.remote.len(), 1);
        assert!(e.remote[0].1.prune(4.5) && !e.remote[0].1.prune(6.0));
    }
}
