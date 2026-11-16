//! Live-Zusammenarbeit im Editor (nur Browser, Server-Boards `srv:<id>`). Protokoll: `docs/web-api.md`.
//!
//! - Verbindung `GET /api/collab/{id}` (WebSocket), automatische Wiederverbindung mit wachsender Wartezeit.
//!   Solange keine Verbindung besteht, speichert der Autosave wie gewohnt per PUT.
//! - Eigene Änderungen: nach jeder Änderung des Boards ([`Editor::live_dirty`]) Diff gegen den letzten Stand
//!   (`owndraw_core::collab::Sync`) → Operationen an den Server. Fremde Operationen werden in das Board übernommen,
//!   ohne Undo-Schritt (die Undo-Schnappschüsse bekommen sie nachgetragen), nie während einer laufenden Geste.
//!   Eigene Operationen kommen als Bestätigung zurück (`cid`) und werden nicht erneut angewendet (kein Echo).
//! - Bilder: vor dem Senden per HTTP hochladen; fremde Bilder werden nachgeladen (bei 404 mit Wiederholung).
//! - Präsenz: Teilnehmerliste mit Farben, fremde Mauszeiger mit Namen (eigener Zeiger höchstens alle 60 ms).

use super::{toolbar, Editor, Lay, M};
use crate::app::Shared;
use crate::platform::web::api::{self, SockEv, Socket};
use crate::session;
use crate::style::{self, c};
use egui::{Align2, Color32, CornerRadius, Pos2, Rect, Sense, Stroke, Ui, Vec2};
use owndraw_core::collab::{self as oc, Op, State, Sync};
use serde_json::{json, Value};
use std::cell::RefCell;
use std::collections::{HashMap, HashSet};
use std::rc::Rc;

/// Eigener Zeiger höchstens so oft (Sekunden).
const CURSOR_EVERY: f64 = 0.06;
/// Fremde Zeiger ohne Bewegung so lange zeigen (Sekunden).
const CURSOR_FADE: f64 = 8.0;

/// Geladenes Bild (Name, Bytes oder None bei Fehler).
type ImageResult = (String, Option<Vec<u8>>);

#[derive(Clone, Copy, PartialEq, Debug)]
enum Conn {
    /// Verbindung wird aufgebaut (bis zum Hello).
    Connecting,
    Live,
    /// Getrennt, neuer Versuch ab dem Zeitpunkt.
    Retry(f64),
    /// Endgültig (kein Zugriff mehr, Board gelöscht).
    Gone,
}

pub struct Peer {
    pub id: u64,
    pub name: String,
    pub color: Color32,
    pub role: String,
    cursor: Option<(f32, f32)>,
    seen: f64,
}

struct ShareDlg {
    email: String,
    edit: bool,
    /// Einladungslink: Rolle und Laufzeit (Zahl + Einheit oder unbefristet).
    link_edit: bool,
    link_forever: bool,
    link_amount: String,
    link_hours: bool,
}

impl Default for ShareDlg {
    fn default() -> Self {
        ShareDlg { email: String::new(), edit: true, link_edit: true, link_forever: false, link_amount: "7".into(), link_hours: false }
    }
}

/// Höchste Laufzeit eines Links (wie am Server): 365 Tage.
const LINK_MAX_DAYS: u32 = 365;

/// Restlaufzeit eines Links, z. B. „noch 5 Std.“ oder „noch 3 Tage (bis 20.11.2026)“.
fn remaining(expires: i64) -> String {
    let left = expires - owndraw_core::store::now_ms() / 1000;
    if left <= 0 {
        "abgelaufen".into()
    } else if left < 3600 {
        format!("noch {} Min.", (left + 59) / 60)
    } else if left < 48 * 3600 {
        format!("noch {} Std.", (left + 1799) / 3600)
    } else {
        format!("noch {} Tage (bis {})", left / 86400, date(expires))
    }
}

pub struct Live {
    pub board_id: String,
    sock: Option<Socket>,
    conn: Conn,
    /// Fehlversuche in Folge (Wartezeit 1, 2, 4 … 30 s).
    fails: u32,
    sync: Sync,
    me: Option<u64>,
    pub peers: Vec<Peer>,
    can_write: bool,
    role: String,
    /// Fremde Operationen, die auf das Ende einer Geste warten.
    queued: Vec<Value>,
    cursor_at: f64,
    cursor: Option<(f32, f32)>,
    /// Bilder, die der Server schon hat.
    uploaded: HashSet<String>,
    /// Bilder, die gerade geladen werden bzw. (Versuche, nächster Versuch).
    fetching: HashSet<String>,
    retry: HashMap<String, (u32, f64)>,
    images_in: Rc<RefCell<Vec<ImageResult>>>,
    share: Option<ShareDlg>,
    conflict_open: bool,
    view_hint_at: f64,
}

/// TT.MM.JJJJ aus Unix-Sekunden.
fn date(secs: i64) -> String {
    let s = owndraw_core::doc::rfc3339(secs * 1000);
    format!("{}.{}.{}", &s[8..10], &s[5..7], &s[0..4])
}

fn color_of(s: &str) -> Color32 {
    let h = s.trim_start_matches('#');
    let v = u32::from_str_radix(h, 16).unwrap_or(0x3E63DD);
    Color32::from_rgb((v >> 16) as u8, (v >> 8) as u8, v as u8)
}

impl Live {
    pub fn new(board_id: &str, ed: &Editor) -> Live {
        Live {
            board_id: board_id.to_string(),
            sock: None,
            conn: Conn::Retry(0.0),
            fails: 0,
            sync: Sync::new(State::of(&ed.board)),
            me: None,
            peers: Vec::new(),
            can_write: true,
            role: api::board(board_id).map(|b| b.role).unwrap_or_default(),
            queued: Vec::new(),
            cursor_at: 0.0,
            cursor: None,
            uploaded: ed.doc.images.files.keys().cloned().collect(),
            fetching: HashSet::new(),
            retry: HashMap::new(),
            images_in: Rc::default(),
            share: None,
            conflict_open: false,
            view_hint_at: -100.0,
        }
    }

    pub fn is_live(&self) -> bool {
        self.conn == Conn::Live
    }

    /// Text für die Speicheranzeige.
    pub fn status(&self) -> (&'static str, bool) {
        match self.conn {
            Conn::Live if !self.can_write => ("Live – nur ansehen", false),
            Conn::Live => ("Live verbunden – wird am Server gespeichert", false),
            Conn::Connecting => ("Live-Verbindung wird aufgebaut …", false),
            Conn::Retry(_) => ("Live-Verbindung getrennt – neuer Versuch …", false),
            Conn::Gone => ("Kein Zugriff mehr auf dieses Board", true),
        }
    }

    fn send(&self, v: Value) -> bool {
        self.sock.as_ref().is_some_and(|s| s.send(&v))
    }

    fn set_live(&self, on: bool) {
        let id = self.board_id.clone();
        api::with(|n| {
            if on {
                n.live.insert(id);
            } else {
                n.live.remove(&id);
            }
        });
    }

    fn disconnected(&mut self, now: f64, code: u16, reason: &str, sh: &mut Shared) {
        self.sock = None;
        self.set_live(false);
        self.peers.clear();
        self.me = None;
        if self.conn == Conn::Gone {
            return;
        }
        match (code, reason) {
            (4000, "not_found" | "removed") => {
                self.conn = Conn::Gone;
                return;
            }
            (4000, "role_changed") => {
                self.conn = Conn::Retry(now);
                return;
            }
            _ => {}
        }
        if self.conn == Conn::Live {
            sh.toast("Live-Verbindung getrennt – Änderungen werden weiter gespeichert");
        }
        self.fails = (self.fails + 1).min(6);
        let wait = (1u32 << (self.fails - 1)).min(30) as f64;
        self.conn = Conn::Retry(now + wait);
    }

    /// Pro Frame (nach dem Zeichnen der Leisten): Verbindung, Ein- und Ausgang.
    pub fn frame(&mut self, ed: &mut Editor, ctx: &egui::Context, sh: &mut Shared) {
        let now = sh.now;
        if let Conn::Retry(at) = self.conn {
            if now >= at && api::online() {
                match Socket::connect(&self.board_id) {
                    Ok(s) => {
                        self.sock = Some(s);
                        self.conn = Conn::Connecting;
                    }
                    Err(_) => self.disconnected(now, 1006, "", sh),
                }
            } else {
                ctx.request_repaint_after(std::time::Duration::from_secs_f64((at - now).clamp(0.05, 30.0)));
            }
        }
        let evs = self.sock.as_ref().map(Socket::take).unwrap_or_default();
        for ev in evs {
            match ev {
                SockEv::Open => {}
                SockEv::Msg(v) => self.message(ed, v, sh),
                SockEv::Closed { code, reason } => self.disconnected(now, code, &reason, sh),
            }
        }
        self.receive_images(ed);
        self.retry_images(now);
        if self.conn != Conn::Live {
            return;
        }
        // Warteschlange fremder Operationen nach dem Ende einer Geste
        if !ed.board.busy() && !self.queued.is_empty() {
            for v in std::mem::take(&mut self.queued) {
                self.apply_ops(ed, &v, sh);
            }
        }
        if ed.live_dirty && !ed.board.busy() {
            ed.live_dirty = false;
            self.push_local(ed, sh);
        }
        self.send_cursor(ed, now);
        // verblassende fremde Zeiger
        if self.peers.iter().any(|p| p.cursor.is_some()) {
            ctx.request_repaint_after(std::time::Duration::from_secs(1));
        }
    }

    /// Eigene Änderungen senden (ohne Schreibrecht: zurücknehmen).
    fn push_local(&mut self, ed: &mut Editor, sh: &mut Shared) {
        let Some((cid, ops)) = self.sync.local(&State::of(&ed.board)) else { return };
        if !self.can_write {
            let s = self.sync.reject(cid);
            oc::apply_to_board(&mut ed.board, &s);
            if sh.now - self.view_hint_at > 4.0 {
                self.view_hint_at = sh.now;
                sh.toast("Du kannst dieses Board nur ansehen");
            }
            return;
        }
        for name in ops.iter().filter_map(Op::image) {
            if !self.uploaded.contains(name) {
                if let Some(b) = ed.doc.images.files.get(name) {
                    api::upload_image(&self.board_id, name, b.clone());
                    self.uploaded.insert(name.to_string());
                }
            }
        }
        self.send(json!({"t": "ops", "cid": cid, "ops": oc::ops_to_json(&ops)}));
    }

    fn send_cursor(&mut self, ed: &Editor, now: f64) {
        let pos = ed.hover.map(|p| {
            let r = p - ed.canvas.min;
            ed.board.screen_to_world(r.x, r.y)
        });
        if pos == self.cursor || now - self.cursor_at < CURSOR_EVERY {
            return;
        }
        self.cursor = pos;
        self.cursor_at = now;
        let (x, y) = pos.map(|(x, y)| (json!(x), json!(y))).unwrap_or((Value::Null, Value::Null));
        self.send(json!({"t": "cursor", "x": x, "y": y}));
    }

    fn message(&mut self, ed: &mut Editor, v: Value, sh: &mut Shared) {
        let now = sh.now;
        match v.get("t").and_then(Value::as_str).unwrap_or("") {
            "hello" => {
                self.me = v.pointer("/you/peer").and_then(Value::as_u64);
                self.can_write = v.get("can_write").and_then(Value::as_bool).unwrap_or(false);
                self.role = v.pointer("/you/role").and_then(Value::as_str).unwrap_or("view").to_string();
                self.peers = v.get("peers").and_then(Value::as_array).map(|a| a.iter().filter_map(|p| peer_of(p, now)).collect()).unwrap_or_default();
                if let Some(ver) = v.get("version").and_then(Value::as_i64) {
                    let id = self.board_id.clone();
                    api::with(|n| n.versions.insert(id, ver));
                }
                // eigene, noch nicht gesendete Änderungen gehören dazu
                self.sync.local(&State::of(&ed.board));
                let server = State::from_json(v.get("board").unwrap_or(&Value::Null), v.get("mode").unwrap_or(&Value::Null));
                let (vis, resend) = self.sync.reset(server, self.can_write);
                oc::apply_to_board(&mut ed.board, &vis);
                ed.page_thumbs.dirty = true;
                self.conn = Conn::Live;
                self.fails = 0;
                self.set_live(true);
                ed.autosave.set_read_only(!self.can_write || ed.doc.meta.read_only);
                for (cid, ops) in resend {
                    self.send(json!({"t": "ops", "cid": cid, "ops": oc::ops_to_json(&ops)}));
                }
                self.want_images(ed, vis.items.iter().filter_map(|it| if let owndraw_core::item::Kind::Img(m) = &it.kind { Some(m.file.clone()) } else { None }).collect());
            }
            "ops" => {
                if ed.board.busy() || !self.queued.is_empty() {
                    self.queued.push(v);
                } else {
                    self.apply_ops(ed, &v, sh);
                }
            }
            "nack" => {
                if let Some(cid) = v.get("cid").and_then(Value::as_u64) {
                    let s = self.sync.reject(cid);
                    oc::apply_to_board(&mut ed.board, &s);
                }
                if let Some(m) = v.get("message").and_then(Value::as_str) {
                    sh.toast_for(m, 4.0);
                }
            }
            "join" => {
                if let Some(p) = v.get("peer").and_then(|p| peer_of(p, now)) {
                    if Some(p.id) != self.me {
                        sh.toast(&format!("{} ist dazugekommen", p.name));
                        self.peers.retain(|q| q.id != p.id);
                        self.peers.push(p);
                    }
                }
            }
            "leave" => {
                let id = v.get("peer").and_then(Value::as_u64);
                self.peers.retain(|p| Some(p.id) != id);
            }
            "cursor" => {
                let id = v.get("peer").and_then(Value::as_u64);
                if let Some(p) = self.peers.iter_mut().find(|p| Some(p.id) == id) {
                    let f = |k: &str| v.get(k).and_then(Value::as_f64).map(|x| x as f32);
                    p.cursor = f("x").zip(f("y"));
                    p.seen = now;
                }
            }
            "access" => {
                self.can_write = v.get("can_write").and_then(Value::as_bool).unwrap_or(false);
                ed.autosave.set_read_only(!self.can_write || ed.doc.meta.read_only);
                if !self.can_write {
                    sh.toast_for("Die Lizenz der Besitzerin/des Besitzers ist nicht mehr gültig – das Board ist nur noch lesbar.", 5.0);
                }
            }
            "saved" => {
                if let Some(ver) = v.get("version").and_then(Value::as_i64) {
                    let id = self.board_id.clone();
                    api::with(|n| n.versions.insert(id, ver));
                }
            }
            "error" => {
                if let Some(m) = v.get("message").and_then(Value::as_str) {
                    sh.toast_for(m, 5.0);
                }
                if matches!(v.get("code").and_then(Value::as_str), Some("not_found" | "removed")) {
                    self.conn = Conn::Gone;
                }
            }
            _ => {}
        }
    }

    fn apply_ops(&mut self, ed: &mut Editor, v: &Value, sh: &mut Shared) {
        let Some(ops) = v.get("ops").and_then(oc::ops_from_json) else { return };
        // erst eigene, noch nicht gesendete Änderungen abgleichen
        if ed.live_dirty {
            ed.live_dirty = false;
            self.push_local(ed, sh);
        }
        let own = (v.get("peer").and_then(Value::as_u64) == self.me && self.me.is_some()).then(|| v.get("cid").and_then(Value::as_u64)).flatten();
        let imgs: Vec<String> = ops.iter().filter_map(Op::image).map(String::from).collect();
        if let Some(s) = self.sync.remote(&ops, own) {
            oc::apply_to_board(&mut ed.board, &s);
            ed.page_thumbs.dirty = true;
        }
        if own.is_none() {
            self.want_images(ed, imgs);
        }
    }

    /// Fehlende Bilder vom Server laden.
    fn want_images(&mut self, ed: &Editor, names: Vec<String>) {
        for n in names {
            if ed.doc.images.files.contains_key(&n) || self.fetching.contains(&n) || n.is_empty() {
                continue;
            }
            self.fetch_image(n);
        }
    }

    fn fetch_image(&mut self, name: String) {
        self.fetching.insert(name.clone());
        let inbox = self.images_in.clone();
        let n2 = name.clone();
        api::fetch_image(&self.board_id, &name, move |b| inbox.borrow_mut().push((n2, b)));
    }

    fn receive_images(&mut self, ed: &mut Editor) {
        let got = std::mem::take(&mut *self.images_in.borrow_mut());
        for (name, bytes) in got {
            self.fetching.remove(&name);
            match bytes {
                Some(b) => {
                    let _ = session::put_image(&ed.img_dir, &name, &b);
                    ed.doc.images.files.insert(name.clone(), b);
                    self.uploaded.insert(name.clone());
                    self.retry.remove(&name);
                    ed.board.refresh();
                }
                None => {
                    // vermutlich noch nicht hochgeladen: später erneut
                    let e = self.retry.entry(name).or_insert((0, 0.0));
                    e.0 += 1;
                    e.1 = -1.0;
                }
            }
        }
    }

    fn retry_images(&mut self, now: f64) {
        let mut due = Vec::new();
        for (n, (tries, at)) in self.retry.iter_mut() {
            if *at < 0.0 {
                *at = now + (*tries as f64).min(8.0);
            } else if now >= *at && *tries <= 6 && !self.fetching.contains(n) {
                *at = f64::INFINITY;
                due.push(n.clone());
            }
        }
        for n in due {
            self.fetch_image(n);
        }
    }

    // ---------------------------------------------------------------- Anzeige

    /// Fremde Mauszeiger mit Namen über der Zeichenfläche.
    pub fn paint_cursors(&self, ui: &egui::Ui, ed: &Editor, now: f64) {
        let p = ui.painter().with_clip_rect(ed.canvas);
        for peer in &self.peers {
            let Some((x, y)) = peer.cursor else { continue };
            if now - peer.seen > CURSOR_FADE {
                continue;
            }
            let b = &ed.board;
            let s = ed.canvas.min + Vec2::new(x * b.sc + b.tx, y * b.sc + b.ty);
            if !ed.canvas.expand(20.0).contains(s) {
                continue;
            }
            let pts = vec![s, s + Vec2::new(0.0, 17.0), s + Vec2::new(4.5, 13.0), s + Vec2::new(11.5, 13.0)];
            p.add(egui::Shape::convex_polygon(pts, peer.color, Stroke::new(1.2, Color32::WHITE)));
            let g = p.layout_no_wrap(peer.name.clone(), egui::FontId::proportional(12.0), Color32::WHITE);
            let r = Rect::from_min_size(s + Vec2::new(12.0, 15.0), g.size() + Vec2::new(10.0, 4.0));
            p.rect_filled(r, CornerRadius::same(6), peer.color);
            p.galley(r.min + Vec2::new(5.0, 2.0), g, Color32::WHITE);
        }
    }

    /// Kleine Leiste rechts oben unter den Knöpfen: Status, Teilnehmer, „Teilen“.
    /// Verschiebbar und andockbar wie die anderen Leisten (Griff ⠿; seitlich angedockt senkrecht).
    pub(super) fn overlay(&mut self, ed: &mut Editor, ctx: &egui::Context, sh: &mut Shared, lay: &Lay) {
        let pal = style::pal();
        let btn = sh.env.btn;
        let h = btn.min(36.0);
        let owner = self.role == "owner" || api::board(&self.board_id).is_some_and(|b| b.role == "owner");
        let can_share = owner && api::licensed();
        let pos = Pos2::new(lay.canvas.right() - M, lay.y_below);
        let (col, tip) = match self.conn {
            Conn::Live => (0xFF30A46C, "Live verbunden"),
            Conn::Connecting => (0xFFFFC53D, "Verbinde …"),
            Conn::Retry(_) => (0xFFFFC53D, "Getrennt – neuer Versuch läuft"),
            Conn::Gone => (pal.danger, "Kein Zugriff"),
        };
        let n = self.peers.len();
        let label = match self.conn {
            Conn::Live if n == 0 => "Live".to_string(),
            Conn::Live => format!("Live · {}", n + 1),
            Conn::Connecting => "Verbinde …".into(),
            Conn::Retry(_) => "Offline".into(),
            Conn::Gone => "Kein Zugriff".into(),
        };
        let peers = &self.peers;
        let mut share = false;
        let r = toolbar::tile(ed, ctx, sh, lay, "live-bar", pos, Align2::RIGHT_TOP, None, false, |_ed, sh, ui, tl| {
            let vert = tl.vert;
            tl.frame
                .show(ui, |ui| {
                    let body = |ui: &mut Ui| {
                        ui.spacing_mut().item_spacing = if vert { Vec2::new(0.0, 4.0) } else { Vec2::new(4.0, 0.0) };
                        let g = toolbar::grip(ui, if vert { btn } else { h }, !vert);
                        let dot = if vert { Vec2::new(btn, 14.0) } else { Vec2::new(14.0, h) };
                        let (rect, resp) = ui.allocate_exact_size(dot, Sense::hover());
                        ui.painter().circle_filled(rect.center(), 4.5, c(col));
                        resp.on_hover_text(tip);
                        if vert {
                            // senkrecht schmal: nur Anzahl statt „Live · n“
                            let short = match self.conn {
                                Conn::Live => format!("{}", n + 1),
                                _ => "–".into(),
                            };
                            ui.label(egui::RichText::new(short).size(12.0).color(c(pal.text))).on_hover_text(&label);
                        } else {
                            ui.label(egui::RichText::new(&label).size(13.0).color(c(pal.text)));
                        }
                        // Teilnehmer als farbige Kreise mit Anfangsbuchstaben
                        let av = if vert { Vec2::new(btn, 26.0) } else { Vec2::new(26.0, h) };
                        for peer in peers.iter().take(6) {
                            let (rect, resp) = ui.allocate_exact_size(av, Sense::hover());
                            ui.painter().circle_filled(rect.center(), 11.0, peer.color);
                            let ch: String = peer.name.chars().next().map(|c| c.to_uppercase().collect()).unwrap_or_default();
                            ui.painter().text(rect.center(), Align2::CENTER_CENTER, ch, egui::FontId::proportional(12.0), Color32::WHITE);
                            let role = match peer.role.as_str() {
                                "owner" => "Besitzer",
                                "edit" => "bearbeiten",
                                _ => "ansehen",
                            };
                            resp.on_hover_text(format!("{} ({role})", peer.name));
                        }
                        if n > 6 {
                            ui.label(egui::RichText::new(format!("+{}", n - 6)).size(12.0).color(c(pal.dim)));
                        }
                        if can_share {
                            let clicked = if vert {
                                let (rect, resp) = ui.allocate_exact_size(Vec2::splat(btn - 6.0), Sense::click());
                                style::btn_bg(ui, rect, &resp, true);
                                sh.icons.paint(ui, rect, owndraw_core::icons::EXPORT, pal.accent, false, (btn * 0.5).round());
                                resp.on_hover_text("Teilen").clicked()
                            } else {
                                style::accent_btn(ui, "Teilen", h - 4.0).clicked()
                            };
                            share |= clicked;
                        }
                        g
                    };
                    if vert {
                        toolbar::orient(ui, true, btn, body)
                    } else {
                        ui.horizontal(body).inner
                    }
                })
                .inner
        });
        if share {
            api::load_members(&self.board_id);
            api::load_invites(&self.board_id);
            self.share = Some(ShareDlg::default());
        }
        ed.st.popup_rects.push(r);
        self.share_dialog(ctx, sh);
        self.conflict_dialog(ed, ctx, sh);
    }

    /// Öffnet den Teilen-Dialog (aus dem Menü ⋯).
    pub fn open_share(&mut self) {
        api::load_members(&self.board_id);
        api::load_invites(&self.board_id);
        self.share = Some(ShareDlg::default());
    }

    fn share_dialog(&mut self, ctx: &egui::Context, sh: &mut Shared) {
        let Some(d) = self.share.as_mut() else { return };
        let h = sh.env.btn;
        let mut close = false;
        let width = (ctx.content_rect().width() - 24.0).clamp(260.0, 460.0);
        let (members, busy, err, invites, new_link) = api::with(|n| {
            let m = n.members.as_ref().filter(|(id, _)| *id == self.board_id).map(|(_, l)| l.clone());
            let i = n.invites.as_ref().filter(|(id, _)| *id == self.board_id).map(|(_, l)| l.clone());
            let nl = n.new_link.as_ref().filter(|(id, _)| *id == self.board_id).map(|(_, u)| u.clone());
            (m, n.busy, n.form_error.clone(), i, nl)
        });
        let pal = style::pal();
        let id = self.board_id.clone();
        egui::Modal::new(egui::Id::unique("share-dlg")).frame(style::card()).show(ctx, |ui| {
            ui.set_width(width - 40.0);
            ui.label(egui::RichText::new("Board teilen").heading().strong());
            ui.label(egui::RichText::new("Per E-Mail Eingeladene brauchen ein Konto (ohne eigene Lizenz); per Link geht es auch als Gast. Änderungen erscheinen bei allen sofort.").size(12.5).color(c(pal.dim)));
            ui.add_space(8.0);
            style::caption(ui, "Mitglieder");
            match &members {
                None => {
                    ui.spinner();
                }
                Some(list) => {
                    for m in list {
                        ui.horizontal(|ui| {
                            ui.vertical(|ui| {
                                let n = if m.name.is_empty() { m.email.clone() } else { m.name.clone() };
                                ui.label(egui::RichText::new(n).size(14.0));
                                let sub = if m.guest { "Gast" } else { m.email.as_str() };
                                ui.label(egui::RichText::new(sub).size(11.5).color(c(pal.dim)));
                            });
                            ui.allocate_ui_with_layout(egui::vec2(ui.available_width(), 34.0), egui::Layout::right_to_left(egui::Align::Center), |ui| {
                                if m.role == "owner" {
                                    ui.label(egui::RichText::new("Besitzer").size(12.5).color(c(pal.dim)));
                                    return;
                                }
                                if style::plain_btn(ui, "Entfernen", 30.0).clicked() {
                                    api::remove_member(&id, &m.user_id);
                                }
                                let r = if m.role == "edit" { "bearbeiten" } else { "ansehen" };
                                ui.label(egui::RichText::new(r).size(12.5).color(c(pal.dim)));
                            });
                        });
                    }
                }
            }
            ui.add_space(8.0);
            style::caption(ui, "Einladen (E-Mail des Kontos)");
            let r = ui.add(egui::TextEdit::singleline(&mut d.email).hint_text("name@beispiel.de").desired_width(f32::INFINITY).font(egui::FontId::proportional(15.0)).margin(Vec2::new(10.0, 8.0)));
            let enter = r.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter));
            ui.horizontal(|ui| {
                if style::chip(ui, "Darf bearbeiten", d.edit, h).clicked() {
                    d.edit = true;
                }
                if style::chip(ui, "Nur ansehen", !d.edit, h).clicked() {
                    d.edit = false;
                }
            });
            if let Some(e) = &err {
                ui.label(egui::RichText::new(e).size(12.5).color(c(pal.danger_text)));
            }
            ui.add_space(10.0);
            ui.allocate_ui_with_layout(egui::vec2(ui.available_width(), h), egui::Layout::right_to_left(egui::Align::Center), |ui| {
                let valid = d.email.contains('@') && !busy;
                ui.add_enabled_ui(valid, |ui| {
                    if (style::primary_btn(ui, "Einladen", h).clicked() || enter) && valid {
                        api::add_member(&id, &d.email, if d.edit { "edit" } else { "view" });
                        d.email.clear();
                    }
                });
            });
            ui.add_space(12.0);
            style::caption(ui, "Einladungslink (auch für Gäste ohne Konto)");
            ui.horizontal_wrapped(|ui| {
                if style::chip(ui, "Darf bearbeiten", d.link_edit, h).clicked() {
                    d.link_edit = true;
                }
                if style::chip(ui, "Nur ansehen", !d.link_edit, h).clicked() {
                    d.link_edit = false;
                }
            });
            ui.horizontal_wrapped(|ui| {
                ui.label(egui::RichText::new("Gültig").size(12.5).color(c(pal.dim)));
                ui.add_enabled_ui(!d.link_forever, |ui| {
                    ui.add(egui::TextEdit::singleline(&mut d.link_amount).desired_width(48.0).font(egui::FontId::proportional(14.0)).margin(Vec2::new(8.0, 6.0)));
                });
                d.link_amount.retain(|ch| ch.is_ascii_digit());
                d.link_amount.truncate(4);
                if style::chip(ui, "Stunden", !d.link_forever && d.link_hours, h).clicked() {
                    d.link_hours = true;
                    d.link_forever = false;
                }
                if style::chip(ui, "Tage", !d.link_forever && !d.link_hours, h).clicked() {
                    d.link_hours = false;
                    d.link_forever = false;
                }
                if style::chip(ui, "Unbefristet", d.link_forever, h).clicked() {
                    d.link_forever = true;
                }
            });
            let n: u32 = d.link_amount.parse().unwrap_or(0);
            let max = if d.link_hours { LINK_MAX_DAYS * 24 } else { LINK_MAX_DAYS };
            let time_ok = d.link_forever || (1..=max).contains(&n);
            if !time_ok {
                let unit = if d.link_hours { "Stunden" } else { "Tage" };
                ui.label(egui::RichText::new(format!("Bitte 1 bis {max} {unit} angeben (höchstens 365 Tage).")).size(12.0).color(c(pal.danger_text)));
            }
            ui.add_space(4.0);
            ui.add_enabled_ui(time_ok, |ui| {
                if style::accent_btn(ui, "Link erstellen", h).clicked() {
                    let time = if d.link_forever {
                        api::LinkTime::Forever
                    } else if d.link_hours {
                        api::LinkTime::Hours(n)
                    } else {
                        api::LinkTime::Days(n)
                    };
                    api::create_invite(&id, if d.link_edit { "edit" } else { "view" }, time);
                }
            });
            if let Some(url) = &new_link {
                ui.add_space(4.0);
                let mut shown = url.clone();
                ui.horizontal(|ui| {
                    let w = (ui.available_width() - 90.0).max(80.0);
                    ui.add(egui::TextEdit::singleline(&mut shown).desired_width(w).font(egui::FontId::proportional(13.0)).margin(Vec2::new(8.0, 6.0)));
                    if style::plain_btn(ui, "Kopieren", h - 6.0).clicked() {
                        ui.ctx().copy_text(url.clone());
                    }
                });
                ui.label(egui::RichText::new("Der Link wird nur jetzt angezeigt – bitte kopieren.").size(11.5).color(c(pal.dim)));
            }
            match &invites {
                None => {
                    ui.spinner();
                }
                Some(list) => {
                    for i in list {
                        ui.horizontal(|ui| {
                            let role = if i.role == "edit" { "bearbeiten" } else { "ansehen" };
                            let until = i.expires_at.map(remaining).unwrap_or_else(|| "unbefristet".into());
                            ui.label(egui::RichText::new(format!("{role} · {until} · {}× benutzt", i.uses)).size(12.5));
                            ui.allocate_ui_with_layout(egui::vec2(ui.available_width(), 30.0), egui::Layout::right_to_left(egui::Align::Center), |ui| {
                                if style::plain_btn(ui, "Löschen", 30.0).clicked() {
                                    api::delete_invite(&id, &i.id);
                                }
                            });
                        });
                    }
                }
            }
            ui.add_space(10.0);
            ui.allocate_ui_with_layout(egui::vec2(ui.available_width(), h), egui::Layout::right_to_left(egui::Align::Center), |ui| {
                close |= style::plain_btn(ui, "Schließen", h).clicked() || ui.input(|i| i.key_pressed(egui::Key::Escape));
            });
        });
        if close {
            self.share = None;
            api::with(|n| n.form_error = None);
        }
    }

    /// Versionskonflikt beim Speichern per PUT (ohne Live-Verbindung): neu laden anbieten.
    fn conflict_dialog(&mut self, ed: &mut Editor, ctx: &egui::Context, sh: &mut Shared) {
        let id = self.board_id.clone();
        if !api::with(|n| n.conflicts.contains(&id)) {
            self.conflict_open = false;
            return;
        }
        self.conflict_open = true;
        let h = sh.env.btn;
        let (mut reload, mut later) = (false, false);
        egui::Modal::new(egui::Id::unique("conflict-dlg")).frame(style::card()).show(ctx, |ui| {
            ui.set_width(360.0);
            ui.label(egui::RichText::new("Board wurde anderswo geändert").heading().strong());
            ui.label("Deine letzten Änderungen konnten nicht gespeichert werden, weil das Board inzwischen auf einem anderen Gerät oder von jemand anderem gespeichert wurde. Neu laden verwirft deine ungespeicherten Änderungen.");
            ui.add_space(10.0);
            ui.allocate_ui_with_layout(egui::vec2(ui.available_width(), h), egui::Layout::right_to_left(egui::Align::Center), |ui| {
                reload = style::danger_btn(ui, "Neu laden", h).clicked();
                later = style::plain_btn(ui, "Später", h).clicked();
            });
        });
        if reload {
            // eigene Änderungen verwerfen, Editor schließen und den Stand des Servers öffnen
            ed.autosave.set_read_only(true);
            api::with(|n| {
                n.conflicts.remove(&id);
                n.save_errors.remove(&id);
            });
            sh.reqs.push(crate::app::Req::Back);
            api::open_board(&id);
        } else if later {
            api::with(|n| n.conflicts.remove(&id));
        }
    }
}

impl Drop for Live {
    fn drop(&mut self) {
        self.set_live(false);
    }
}

fn peer_of(p: &Value, now: f64) -> Option<Peer> {
    Some(Peer {
        id: p.get("peer")?.as_u64()?,
        name: p.get("name").and_then(Value::as_str).filter(|s| !s.is_empty()).unwrap_or("Gast").to_string(),
        color: color_of(p.get("color").and_then(Value::as_str).unwrap_or("")),
        role: p.get("role").and_then(Value::as_str).unwrap_or("view").to_string(),
        cursor: None,
        seen: now,
    })
}
