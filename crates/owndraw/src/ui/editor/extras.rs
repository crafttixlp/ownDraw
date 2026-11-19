//! Werkzeuge ohne eigenes Board-Werkzeug: Lineal/Geodreieck ([`super::ruler`]), Laserpointer und Platz schaffen.
//!
//! Die Zeiger-Eingabe läuft zuerst durch [`Editor::ext_hook`]: Was das Lineal, der Laser oder „Platz schaffen“
//! braucht, geht nicht ans Board (`Hook::Eat`); Striche an einer Linealkante gehen projiziert weiter (`Hook::Pass`).
//! - Laser: Laserpunkt am Mauszeiger bzw. am berührten Punkt (dunkler Kern, durchsichtiger Hof; Farbe und Größe
//!   einstellbar), wahlweise mit Leuchtspur ([`owndraw_core::laser::Laser`], verblasst nach 1,5 s); nie gespeichert,
//!   kein Undo; live an die anderen Teilnehmer (`live.rs`, Nachricht `laser` mit Punkt `d`/`off` und Spur `p`).
//! - Platz schaffen: senkrecht ziehen; beim Loslassen `Board::make_space` (ein Rückgängig-Schritt).
//! - Umwandeln: Lasso-Weg bzw. Tipp sammeln; beim Loslassen die Striche wählen ([`super::convert`]).

use super::ruler::{self, Hit, Ruler};
use super::{Editor, Tool};
use crate::style::{self, c};
use egui::{Color32, CornerRadius, FontId, Pos2, Rect, Stroke, Ui, Vec2};
use owndraw_core::board::{self, Board};
use owndraw_core::collab::LaserMsg;
use owndraw_core::laser::{self as lz, Laser, DOT_FADE, FADE, LASER_COLOR};

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

/// Laserpunkt (Weltkoordinaten): sichtbar an `pos`, nach dem Ausblenden kurz verblassend an `last`.
#[derive(Clone, Copy, Debug, Default)]
struct Dot {
    pos: Option<(f32, f32)>,
    last: (f32, f32),
    off_at: f64,
}

impl Dot {
    fn set(&mut self, p: Option<(f32, f32)>, now: f64) {
        match p {
            Some(q) => self.last = q,
            None if self.pos.is_some() => self.off_at = now,
            None => {}
        }
        self.pos = p;
    }

    /// Position und Deckkraft (1 = sichtbar, sinkt beim Ausblenden auf 0).
    fn shown(&self, now: f64) -> Option<((f32, f32), f32)> {
        match self.pos {
            Some(p) => Some((p, 1.0)),
            None => {
                let k = 1.0 - (now - self.off_at) / DOT_FADE;
                (k > 0.0).then_some((self.last, k as f32))
            }
        }
    }
}

/// Laserpointer eines anderen Teilnehmers.
struct Remote {
    peer: u64,
    trail: Laser,
    dot: Dot,
    size: f32,
    seen: f64,
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
    /// Durchmesser des Laserpunkts (Bildschirmpunkte).
    pub laser_size: f32,
    /// Leuchtspur nachziehen (sonst nur der Punkt).
    pub laser_trail: bool,
    /// Eigener Laserpunkt.
    dot: Dot,
    /// Zuletzt gesendeter Punkt und Zeit.
    dot_sent: Option<(f32, f32)>,
    dot_sent_at: f64,
    /// Letzte Eingabe kam vom Touchscreen (dann Punkt nur beim Berühren, nicht beim Schweben).
    pub last_touch: bool,
    /// Weltposition des drückenden Laser-Zeigers.
    laser_at: Option<(f32, f32)>,
    /// Laser der anderen Teilnehmer.
    remote: Vec<Remote>,
    laser_ptr: Option<u64>,
    /// Ausgang: fertige Abschnitte, laufender Abschnitt (erster Punkt ggf. schon gesendet).
    out: Vec<Vec<(f32, f32)>>,
    out_cur: Vec<(f32, f32)>,
    out_sent: usize,
    out_at: f64,
    space: Option<SpaceDrag>,
    /// Werkzeug „Umwandeln“: laufendes Lasso (Zeiger, Weltpunkte).
    pub lasso: Option<(u64, Vec<(f32, f32)>)>,
    /// Werkzeug „Umwandeln“: fertige Auswahl (Striche), wird im Frame übernommen.
    pub convert_pick: Option<Vec<u64>>,
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
            laser_size: lz::DOT_SIZE,
            laser_trail: false,
            dot: Dot::default(),
            dot_sent: None,
            dot_sent_at: 0.0,
            last_touch: false,
            laser_at: None,
            remote: Vec::new(),
            laser_ptr: None,
            out: Vec::new(),
            out_cur: Vec::new(),
            out_sent: 0,
            out_at: 0.0,
            space: None,
            lasso: None,
            convert_pick: None,
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

    /// Laserpunkt zum Senden, wenn er sich bewegt hat (höchstens alle 40 ms), verschwunden ist oder zuletzt vor
    /// 1 s gesendet wurde (damit er bei den anderen nicht ausläuft). `Some(None)` = ausblenden.
    #[cfg_attr(target_os = "android", allow(dead_code))]
    pub fn take_dot_out(&mut self, now: f64) -> Option<Option<(f32, f32)>> {
        let cur = self.dot.pos;
        let due = now - self.dot_sent_at;
        if (cur != self.dot_sent && due >= 0.04) || (cur.is_some() && due >= 1.0) {
            self.dot_sent = cur;
            self.dot_sent_at = now;
            return Some(cur);
        }
        None
    }

    /// Laser eines anderen Teilnehmers.
    #[cfg_attr(target_os = "android", allow(dead_code))]
    pub fn laser_in(&mut self, peer: u64, m: &LaserMsg, now: f64) {
        let i = match self.remote.iter().position(|r| r.peer == peer) {
            Some(i) => i,
            None => {
                self.remote.push(Remote { peer, trail: Laser::new(m.color), dot: Dot::default(), size: m.size, seen: now });
                self.remote.len() - 1
            }
        };
        let r = &mut self.remote[i];
        r.trail.color = m.color;
        r.size = m.size;
        r.seen = now;
        if !m.pts.is_empty() {
            r.trail.add_remote(&m.pts, now);
        }
        if m.dot.is_some() || m.off {
            r.dot.set(m.dot, now);
        }
    }

    fn laser_pt(&mut self, x: f32, y: f32) {
        self.laser_at = Some((x, y));
        if self.laser_trail {
            self.laser.add(x, y, self.now);
            self.out_cur.push((x, y));
        }
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

    /// Demo `laser`: eigener Punkt (gedrückt) und zwei Punkte anderer Teilnehmer, einer mit Spur.
    pub fn demo_laser(&mut self) {
        self.ext.xtool = Some(Tool::Laser);
        self.ext.laser_ptr = Some(MOUSE);
        self.ext.laser_at = Some((520.0, 300.0));
        let m = |c: u32, s: f32, pts: Vec<(f32, f32)>, d: (f32, f32)| LaserMsg { peer: None, color: c, pts, size: s, dot: Some(d), off: false };
        self.ext.laser_in(1, &m(0xFF007AFF, 56.0, vec![], (760.0, 420.0)), 0.0);
        let arc: Vec<(f32, f32)> = (0..=20).map(|i| { let a = i as f32 * 0.25; (380.0 + 70.0 * a.cos(), 520.0 + 40.0 * a.sin()) }).collect();
        let end = *arc.last().unwrap();
        self.ext.laser_in(2, &m(0xFF34C759, 24.0, arc, end), 0.0);
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
                        self.ext.last_touch = touch;
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
                    Some(Tool::Convert) => self.ext.lasso = Some((id, vec![(wx, wy)])),
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
                    } else if let Some((_, pts)) = self.ext.lasso.as_mut().filter(|l| l.0 == id) {
                        let (wx, wy) = self.board.screen_to_world(p.x, p.y);
                        pts.push((wx, wy));
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
                        self.ext.laser_at = None;
                        self.ext.laser_lift();
                    }
                    if let Some((_, pts)) = self.ext.lasso.take_if(|l| l.0 == id) {
                        if ph == Ph::Up {
                            self.ext.convert_pick = Some(self.convert_ids(&pts));
                        }
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

    /// Striche zum Lasso-Weg (Weltpunkte): kurzer Weg = Antippen (Strich + Nachbarn der Zeile), sonst Lasso.
    fn convert_ids(&self, pts: &[(f32, f32)]) -> Vec<u64> {
        let (mut x0, mut y0, mut x1, mut y1) = (f32::MAX, f32::MAX, f32::MIN, f32::MIN);
        for &(x, y) in pts {
            (x0, y0, x1, y1) = (x0.min(x), y0.min(y), x1.max(x), y1.max(y));
        }
        let extent = (x1 - x0).max(y1 - y0) * self.board.sc;
        if extent < 12.0 {
            let (x, y) = pts[0];
            return self.board.ink_at(x, y);
        }
        let ids = self.board.ink_in_lasso(pts);
        if ids.is_empty() && extent < 40.0 {
            // kurzer Wisch über einen Strich: wie Antippen
            return self.board.ink_at(pts[0].0, pts[0].1);
        }
        ids
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
        self.convert_paint(&pt, rect);
        // Laser (eigener und fremde): Spuren, darüber die Punkte
        let own = if self.ext.xtool != Some(Tool::Laser) {
            None
        } else if self.ext.laser_ptr.is_some() {
            self.ext.laser_at
        } else if !self.ext.last_touch && !self.drawing {
            self.hover.map(|h| self.board.screen_to_world(h.x - rect.min.x, h.y - rect.min.y))
        } else {
            None
        };
        self.ext.dot.set(own, now);
        if own.is_some() && !self.ext.last_touch {
            ctx.set_cursor_icon(egui::CursorIcon::None);
        }
        let mut anim = self.ext.laser.prune(now);
        for r in &mut self.ext.remote {
            // Punkt ohne Lebenszeichen (Verbindung weg) ausblenden
            if r.dot.pos.is_some() && now - r.seen > 3.0 {
                r.dot.set(None, now);
            }
        }
        self.ext.remote.retain_mut(|r| r.trail.prune(now) | r.dot.shown(now).is_some() || now - r.seen < 5.0);
        let b = &self.board;
        let to = |(x, y): (f32, f32)| rect.min + Vec2::new(x * b.sc + b.tx, y * b.sc + b.ty);
        for l in std::iter::once(&self.ext.laser).chain(self.ext.remote.iter().map(|r| &r.trail)) {
            paint_laser(&pt, l, now, &|x, y| to((x, y)));
        }
        let mut remote_dot = false;
        for r in &self.ext.remote {
            if let Some((p, a)) = r.dot.shown(now) {
                paint_dot(&pt, to(p), r.size, r.trail.color, a);
                anim |= a < 1.0;
                remote_dot = true;
            }
        }
        if let Some((p, a)) = self.ext.dot.shown(now) {
            paint_dot(&pt, to(p), self.ext.laser_size, self.ext.laser.color, a);
            anim |= a < 1.0;
        }
        anim |= self.ext.remote.iter().any(|r| !r.trail.is_empty());
        if anim {
            ctx.request_repaint();
        } else if remote_dot || own.is_some() {
            ctx.request_repaint_after(std::time::Duration::from_millis(250));
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

/// Laserpunkt: durchsichtiger Hof, dunklerer kräftiger Kern (`size` = Durchmesser, `a` = Deckkraft 0..1).
fn paint_dot(pt: &egui::Painter, at: Pos2, size: f32, color: u32, a: f32) {
    let (r, g, b) = ((color >> 16) as u8, (color >> 8) as u8, color as u8);
    let col = |al: f32| Color32::from_rgba_unmultiplied(r, g, b, (al * a * 255.0).round() as u8);
    let rad = size / 2.0;
    pt.circle_filled(at, rad, col(0.16));
    pt.circle_filled(at, rad * 0.7, col(0.22));
    pt.circle_filled(at, rad * 0.45, col(0.35));
    let dk = |v: u8| (v as f32 * 0.78) as u8;
    let core = (rad * 0.28).max(3.0);
    pt.circle_filled(at, core, Color32::from_rgba_unmultiplied(dk(r), dk(g), dk(b), (a * 255.0).round() as u8));
}

/// Optionen des Laserpointers (Farbe, Größe, Spur); in den Einstellungen gemerkt.
pub(super) fn options(ui: &mut Ui, ed: &mut Editor, sh: &mut crate::app::Shared) {
    let h = crate::layout::min_touch(sh.env.touch).max(32.0);
    style::caption(ui, "Farbe");
    let size = sh.env.btn.min(ui.available_width() / lz::COLORS.len() as f32).max(28.0);
    let p = style::pal();
    ui.horizontal_wrapped(|ui| {
        ui.spacing_mut().item_spacing = Vec2::ZERO;
        for (col, name) in lz::COLORS {
            let (r, resp) = ui.allocate_exact_size(Vec2::splat(size), egui::Sense::click());
            paint_dot(ui.painter(), r.center(), 22.0, col, 1.0);
            if ed.ext.laser.color == col {
                ui.painter().circle_stroke(r.center(), 13.0, Stroke::new(2.0, c(p.text)));
            } else if resp.hovered() {
                ui.painter().circle_stroke(r.center(), 13.0, Stroke::new(1.5, c(p.line)));
            }
            if resp.on_hover_text(name).clicked() {
                ed.ext.laser.color = col;
                sh.settings.extra.insert("laser_color".into(), serde_json::json!(col));
            }
        }
    });
    style::caption(ui, "Größe");
    ui.horizontal(|ui| {
        let mut s = ed.ext.laser_size;
        ui.spacing_mut().slider_width = 150.0;
        if ui.add(egui::Slider::new(&mut s, lz::DOT_MIN..=lz::DOT_MAX).max_decimals(0).suffix(" pt")).changed() {
            ed.ext.laser_size = s.clamp(lz::DOT_MIN, lz::DOT_MAX);
            sh.settings.extra.insert("laser_size".into(), serde_json::json!(ed.ext.laser_size));
        }
        // Vorschau in echter Größe (gedeckelt)
        let d = ed.ext.laser_size.min(48.0);
        let (r, _) = ui.allocate_exact_size(Vec2::splat(d.max(24.0)), egui::Sense::hover());
        paint_dot(ui.painter(), r.center(), d, ed.ext.laser.color, 1.0);
    });
    if style::chip(ui, "Leuchtspur nachziehen", ed.ext.laser_trail, h).clicked() {
        ed.ext.laser_trail = !ed.ext.laser_trail;
        sh.settings.extra.insert("laser_trail".into(), serde_json::json!(ed.ext.laser_trail));
    }
    ui.label(egui::RichText::new("Der Punkt folgt der Maus bzw. dem Finger. In der Live-Zusammenarbeit sehen ihn alle.").size(12.0).color(c(p.dim)));
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
        e.laser_trail = true;
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
        let m = |pts: Vec<(f32, f32)>, dot, off| LaserMsg { peer: Some(9), color: 0xFF00FF00, pts, size: 40.0, dot, off };
        e.laser_in(9, &m(vec![(0.0, 0.0), (5.0, 5.0)], None, false), 4.0);
        assert_eq!(e.remote.len(), 1);
        assert!(e.remote[0].trail.prune(4.5) && !e.remote[0].trail.prune(6.0));
    }

    #[test]
    fn laserpunkt_senden_und_empfangen() {
        let mut e = Extras::default();
        assert_eq!(e.take_dot_out(0.0), None, "nichts zu senden");
        e.dot.set(Some((1.0, 2.0)), 0.0);
        assert_eq!(e.take_dot_out(0.1), Some(Some((1.0, 2.0))));
        assert_eq!(e.take_dot_out(0.5), None, "unverändert");
        assert_eq!(e.take_dot_out(1.2), Some(Some((1.0, 2.0))), "Lebenszeichen");
        e.dot.set(None, 1.3);
        assert_eq!(e.take_dot_out(1.3), Some(None), "ausblenden");
        assert_eq!(e.take_dot_out(3.0), None);
        // ohne Spur: Punkt ja, Spur nein
        e.laser_pt(3.0, 4.0);
        assert_eq!(e.laser_at, Some((3.0, 4.0)));
        assert!(e.laser.is_empty() && e.take_laser_out(5.0).is_empty());
        // fremder Punkt: sichtbar, nach Ausblenden verblassend
        let m = |dot, off| LaserMsg { peer: Some(2), color: 0xFFFF0000, pts: vec![], size: 50.0, dot, off };
        e.laser_in(2, &m(Some((7.0, 8.0)), false), 10.0);
        assert_eq!(e.remote[0].dot.shown(10.0), Some(((7.0, 8.0), 1.0)));
        assert_eq!(e.remote[0].size, 50.0);
        e.laser_in(2, &m(None, true), 11.0);
        assert!(e.remote[0].dot.shown(11.1).is_some_and(|d| d.1 < 1.0));
        assert!(e.remote[0].dot.shown(12.0).is_none());
    }
}
