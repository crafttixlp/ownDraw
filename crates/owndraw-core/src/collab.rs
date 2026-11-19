//! Zusammenarbeit in Echtzeit: Operationen auf Board-Objekten (nach dauerhafter Kennung `uid`), Diff,
//! Anwenden und der Abgleich eines Clients mit dem Server ([`Sync`]). Protokoll: `docs/web-api.md`.
//!
//! - Objekte: [`Change::Upsert`] (ganzes Item, danach hinter `after` einsortieren; `None` = ganz unten),
//!   [`Change::Move`] (nur Reihenfolge), [`Change::Delete`].
//! - Ebenen, Seiten, Hintergrund: als ganzer Zustand ([`BoardState`], Op `board`).
//!
//! Der Server legt die Reihenfolge fest (Sequenznummer) und wendet alles in dieser Reihenfolge an. Ein Client hält
//! den bestätigten Stand (`shadow`) und seine noch unbestätigten Operationen (`pending`); sein sichtbarer Stand ist
//! immer `shadow + pending`. Sobald alle Operationen bestätigt sind, haben alle denselben Stand.

use crate::board::Board;
use crate::item::{self, Item};
use crate::layer::Layer;
use serde_json::{json, Map, Value};
use std::collections::{HashMap, HashSet, VecDeque};
use std::rc::Rc;

/// Änderung an der Objektliste (`T` = Item-JSON beim Server, `Rc<Item>` im Client).
#[derive(Clone, Debug, PartialEq)]
pub enum Change<T> {
    Upsert(T, Option<u64>),
    Move(u64, Option<u64>),
    Delete(u64),
}

/// Ebenen, Seiten-Einstellungen (Manifest-Ausschnitt wie [`crate::doc::mode_to_json`]) und Hintergrund.
#[derive(Clone, Debug, PartialEq)]
pub struct BoardState {
    pub layers: Vec<Value>,
    pub mode: Value,
    pub bg: i32,
    pub dark: bool,
}

impl BoardState {
    pub fn of(b: &Board) -> BoardState {
        BoardState { layers: b.layers.iter().map(Layer::json).collect(), mode: crate::doc::mode_to_json(&b.board_mode), bg: b.bg, dark: b.dark }
    }
    /// Aus einem Board-JSON (`board.json`) und dem Modus.
    pub fn from_board_json(board: &Value, mode: &Value) -> BoardState {
        BoardState {
            layers: board.get("layers").and_then(Value::as_array).cloned().unwrap_or_default(),
            mode: mode.clone(),
            bg: board.get("bg").and_then(item::jint).unwrap_or(0),
            dark: item::jbool(board, "dark"),
        }
    }
    fn json(&self) -> Value {
        json!({"o": "board", "layers": self.layers, "mode": self.mode, "bg": self.bg, "dark": self.dark})
    }
    fn parse(o: &Value) -> Option<BoardState> {
        let layers = o.get("layers")?.as_array()?.iter().filter(|l| l.is_object()).cloned().collect::<Vec<_>>();
        if layers.is_empty() {
            return None;
        }
        let mode = o.get("mode").filter(|m| m.is_object()).cloned().unwrap_or_else(|| json!({"mode": "infinite"}));
        Some(BoardState { layers, mode, bg: o.get("bg").and_then(item::jint).unwrap_or(0).clamp(0, 3), dark: item::jbool(o, "dark") })
    }
}

/// Operation im Protokoll.
#[derive(Clone, Debug, PartialEq)]
pub enum Op {
    Item(Change<Value>),
    Board(BoardState),
}

fn uid_json(u: Option<u64>) -> Value {
    u.map(|u| Value::String(item::uid_str(u))).unwrap_or(Value::Null)
}

fn after_of(o: &Value) -> Option<Option<u64>> {
    match o.get("after") {
        None | Some(Value::Null) => Some(None),
        Some(v) => item::parse_uid(v).map(Some),
    }
}

/// uid eines Item-JSON.
pub fn uid_of_json(v: &Value) -> Option<u64> {
    v.get("uid").and_then(item::parse_uid)
}

impl Op {
    pub fn to_json(&self) -> Value {
        match self {
            Op::Item(Change::Upsert(it, after)) => json!({"o": "up", "item": it, "after": uid_json(*after)}),
            Op::Item(Change::Move(u, after)) => json!({"o": "mv", "uid": item::uid_str(*u), "after": uid_json(*after)}),
            Op::Item(Change::Delete(u)) => json!({"o": "del", "uid": item::uid_str(*u)}),
            Op::Board(b) => b.json(),
        }
    }
    /// Liest eine Operation; None bei unbekannter Art oder ungültigen Feldern (Item ohne `uid`, kaputtes Item).
    pub fn from_json(o: &Value) -> Option<Op> {
        Some(match o.get("o")?.as_str()? {
            "up" => {
                let it = o.get("item")?;
                uid_of_json(it)?;
                Item::from_json(it)?;
                Op::Item(Change::Upsert(it.clone(), after_of(o)?))
            }
            "mv" => Op::Item(Change::Move(item::parse_uid(o.get("uid")?)?, after_of(o)?)),
            "del" => Op::Item(Change::Delete(item::parse_uid(o.get("uid")?)?)),
            "board" => Op::Board(BoardState::parse(o)?),
            _ => return None,
        })
    }
    /// Bilddatei, auf die ein Upsert verweist (Bild-Item, Feld `f`).
    pub fn image(&self) -> Option<&str> {
        match self {
            Op::Item(Change::Upsert(it, _)) if it.get("t").and_then(Value::as_str) == Some("i") => it.get("f").and_then(Value::as_str),
            _ => None,
        }
    }
}

/// Laserpointer-Nachricht (`{"t":"laser","c":Farbe,"p":[x,y,…]}`, vom Server zusätzlich mit `peer`): neue Punkte
/// einer Spur in Board-Koordinaten. Wird nur weitergereicht – nicht gespeichert, keine Operation, kein Rückgängig.
#[derive(Clone, Debug, PartialEq)]
pub struct LaserMsg {
    pub peer: Option<u64>,
    pub color: u32,
    /// Spur-Punkte (Weltkoordinaten), darf leer sein.
    pub pts: Vec<(f32, f32)>,
    /// Durchmesser des Punkts in Bildschirmpunkten.
    pub size: f32,
    /// Aktuelle Position des Laserpunkts (Weltkoordinaten).
    pub dot: Option<(f32, f32)>,
    /// Laserpunkt ausblenden.
    pub off: bool,
}

impl LaserMsg {
    /// Höchstzahl Punkte je Nachricht.
    pub const MAX_PTS: usize = 256;

    pub fn to_json(&self) -> Value {
        let p: Vec<Value> = self.pts.iter().flat_map(|&(x, y)| [item::num((x * 10.0).round() / 10.0), item::num((y * 10.0).round() / 10.0)]).collect();
        let mut o = json!({"t": "laser", "c": self.color as i32, "p": p, "s": item::num(self.size.round())});
        if let Some((x, y)) = self.dot {
            o["d"] = json!([item::num((x * 10.0).round() / 10.0), item::num((y * 10.0).round() / 10.0)]);
        }
        if self.off {
            o["off"] = json!(true);
        }
        if let Some(peer) = self.peer {
            o["peer"] = json!(peer);
        }
        o
    }

    /// None bei anderer Nachricht oder ungültigen Feldern; zu viele Punkte werden abgeschnitten.
    pub fn from_json(o: &Value) -> Option<LaserMsg> {
        if o.get("t")?.as_str()? != "laser" {
            return None;
        }
        let p = o.get("p")?.as_array()?;
        let mut pts = Vec::with_capacity(p.len() / 2);
        for xy in p.chunks_exact(2).take(Self::MAX_PTS) {
            let (x, y) = (xy[0].as_f64()? as f32, xy[1].as_f64()? as f32);
            if !x.is_finite() || !y.is_finite() {
                return None;
            }
            pts.push((x, y));
        }
        let size = o.get("s").and_then(Value::as_f64).map(|s| s as f32).filter(|s| s.is_finite()).unwrap_or(crate::laser::DOT_SIZE).clamp(crate::laser::DOT_MIN, crate::laser::DOT_MAX);
        let dot = o.get("d").and_then(Value::as_array).and_then(|d| Some((d.first()?.as_f64()? as f32, d.get(1)?.as_f64()? as f32))).filter(|d| d.0.is_finite() && d.1.is_finite());
        let off = o.get("off").and_then(Value::as_bool).unwrap_or(false);
        Some(LaserMsg { peer: o.get("peer").and_then(Value::as_u64), color: o.get("c").and_then(item::jint).map(|c| c as u32).unwrap_or(crate::laser::LASER_COLOR), pts, size, dot, off })
    }

    /// Enthält etwas zum Anzeigen (Spur oder Punkt)?
    pub fn is_empty(&self) -> bool {
        self.pts.is_empty() && self.dot.is_none() && !self.off
    }
}

pub fn ops_to_json(ops: &[Op]) -> Value {
    Value::Array(ops.iter().map(Op::to_json).collect())
}

/// Liest eine Liste von Operationen; None, wenn eine davon ungültig ist.
pub fn ops_from_json(v: &Value) -> Option<Vec<Op>> {
    v.as_array()?.iter().map(Op::from_json).collect()
}

/// Wendet eine Änderung auf eine Liste an. Fehlt der Vorgänger `after` (gleichzeitig gelöscht), bleibt ein
/// vorhandenes Objekt an seiner Stelle; ein neues kommt nach oben.
pub fn apply<T: Clone>(list: &mut Vec<T>, ch: &Change<T>, uid: impl Fn(&T) -> u64) {
    let pos = |list: &Vec<T>, u: u64| list.iter().position(|x| uid(x) == u);
    let insert_at = |list: &Vec<T>, after: Option<u64>| -> Option<usize> {
        match after {
            None => Some(0),
            Some(a) => list.iter().position(|x| uid(x) == a).map(|i| i + 1),
        }
    };
    match ch {
        Change::Upsert(v, after) => {
            let u = uid(v);
            let old = pos(list, u);
            if let Some(i) = old {
                list.remove(i);
            }
            let at = insert_at(list, *after).or(old).unwrap_or(list.len()).min(list.len());
            list.insert(at, v.clone());
        }
        Change::Move(u, after) => {
            let Some(i) = pos(list, *u) else { return };
            let v = list.remove(i);
            let at = insert_at(list, *after).unwrap_or(i).min(list.len());
            list.insert(at, v);
        }
        Change::Delete(u) => list.retain(|x| uid(x) != *u),
    }
}

/// Indizes (in `seq`) einer längsten streng steigenden Teilfolge.
fn lis(seq: &[usize]) -> HashSet<usize> {
    let mut tails: Vec<usize> = Vec::new(); // Index in seq des kleinsten Endwerts je Länge
    let mut prev = vec![usize::MAX; seq.len()];
    for i in 0..seq.len() {
        let k = tails.partition_point(|&t| seq[t] < seq[i]);
        if k > 0 {
            prev[i] = tails[k - 1];
        }
        if k == tails.len() {
            tails.push(i);
        } else {
            tails[k] = i;
        }
    }
    let mut out = HashSet::new();
    let mut cur = tails.last().copied().unwrap_or(usize::MAX);
    while cur != usize::MAX {
        out.insert(cur);
        cur = prev[cur];
    }
    out
}

/// Änderungen von `old` nach `new` (nach uid; unveränderte Objekte werden über `Rc::ptr_eq` bzw. JSON erkannt).
/// Angewendet in der gelieferten Reihenfolge ergibt sich aus `old` genau `new`.
pub fn diff_items(old: &[Rc<Item>], new: &[Rc<Item>]) -> Vec<Change<Rc<Item>>> {
    if old.len() == new.len() && old.iter().zip(new).all(|(a, b)| Rc::ptr_eq(a, b)) {
        return Vec::new();
    }
    let opos: HashMap<u64, usize> = old.iter().enumerate().map(|(i, it)| (it.uid, i)).collect();
    let nset: HashSet<u64> = new.iter().map(|it| it.uid).collect();
    let mut out: Vec<Change<Rc<Item>>> = old.iter().filter(|it| !nset.contains(&it.uid)).map(|it| Change::Delete(it.uid)).collect();
    // gemeinsame Objekte: in alter Reihenfolge bleibende (längste steigende Teilfolge) brauchen kein Move
    let common: Vec<(usize, usize)> = new.iter().enumerate().filter_map(|(k, it)| opos.get(&it.uid).map(|&i| (k, i))).collect();
    let keep_idx = lis(&common.iter().map(|c| c.1).collect::<Vec<_>>());
    let keep: HashSet<usize> = keep_idx.iter().map(|&j| common[j].0).collect();
    let mut prev: Option<u64> = None;
    for (k, it) in new.iter().enumerate() {
        match opos.get(&it.uid) {
            None => out.push(Change::Upsert(it.clone(), prev)),
            Some(&i) => {
                let o = &old[i];
                let changed = !Rc::ptr_eq(o, it) && o.json() != it.json();
                if changed {
                    out.push(Change::Upsert(it.clone(), prev));
                } else if !keep.contains(&k) {
                    out.push(Change::Move(it.uid, prev));
                }
            }
        }
        prev = Some(it.uid);
    }
    out
}

/// Client-Änderung → Protokoll.
pub fn change_to_op(c: &Change<Rc<Item>>) -> Op {
    Op::Item(match c {
        Change::Upsert(it, a) => Change::Upsert(it.json(), *a),
        Change::Move(u, a) => Change::Move(*u, *a),
        Change::Delete(u) => Change::Delete(*u),
    })
}

/// Protokoll → Client-Änderung (Item-JSON wird gelesen).
fn op_to_change(c: &Change<Value>) -> Option<Change<Rc<Item>>> {
    Some(match c {
        Change::Upsert(v, a) => {
            let mut it = Item::from_json(v)?;
            it.uid = uid_of_json(v)?;
            Change::Upsert(Rc::new(it), *a)
        }
        Change::Move(u, a) => Change::Move(*u, *a),
        Change::Delete(u) => Change::Delete(*u),
    })
}

/// Sichtbarer Stand eines Boards für den Abgleich.
#[derive(Clone, Debug)]
pub struct State {
    pub items: Vec<Rc<Item>>,
    pub board: BoardState,
}

impl State {
    pub fn of(b: &Board) -> State {
        State { items: b.items.clone(), board: BoardState::of(b) }
    }
    /// Aus Board-JSON (Items mit oder ohne uid) und Modus-JSON.
    pub fn from_json(board: &Value, mode: &Value) -> State {
        let items = board
            .get("items")
            .and_then(Value::as_array)
            .map(|a| a.iter().filter_map(Item::from_json).map(Rc::new).collect())
            .unwrap_or_default();
        State { items, board: BoardState::from_board_json(board, mode) }
    }
    fn apply_op(&mut self, op: &ClientOp) {
        match op {
            ClientOp::Item(c) => apply(&mut self.items, c, |it| it.uid),
            ClientOp::Board(b) => self.board = b.clone(),
        }
    }
}

/// Operation im Client (Items als `Rc<Item>`, teilen sich den Speicher mit Board und Undo-Schnappschüssen).
#[derive(Clone, Debug)]
pub enum ClientOp {
    Item(Change<Rc<Item>>),
    Board(BoardState),
}

impl ClientOp {
    pub fn to_op(&self) -> Op {
        match self {
            ClientOp::Item(c) => change_to_op(c),
            ClientOp::Board(b) => Op::Board(b.clone()),
        }
    }
    pub fn from_op(op: &Op) -> Option<ClientOp> {
        Some(match op {
            Op::Item(c) => ClientOp::Item(op_to_change(c)?),
            Op::Board(b) => ClientOp::Board(b.clone()),
        })
    }
}

/// Änderungen von `old` nach `new` (Objekte und Board-Zustand).
pub fn diff(old: &State, new: &State) -> Vec<ClientOp> {
    let mut out: Vec<ClientOp> = diff_items(&old.items, &new.items).into_iter().map(ClientOp::Item).collect();
    if old.board != new.board {
        out.push(ClientOp::Board(new.board.clone()));
    }
    out
}

/// Abgleich eines Clients mit dem Server.
pub struct Sync {
    /// Vom Server bestätigter Stand (alle empfangenen Sequenznummern angewendet).
    shadow: State,
    /// Gesendete, noch nicht bestätigte Operationen (Client-Kennung `cid`).
    pending: VecDeque<(u64, Vec<ClientOp>)>,
    /// Sichtbarer Stand beim letzten Abgleich (`shadow + pending`).
    base: State,
    next_cid: u64,
}

impl Sync {
    pub fn new(state: State) -> Sync {
        Sync { shadow: state.clone(), base: state, pending: VecDeque::new(), next_cid: 1 }
    }

    /// Unbestätigte Operationen?
    pub fn has_pending(&self) -> bool {
        !self.pending.is_empty()
    }

    /// Lokale Änderungen seit dem letzten Abgleich → neue Operationen (mit `cid`) zum Senden.
    pub fn local(&mut self, now: &State) -> Option<(u64, Vec<Op>)> {
        let ops = diff(&self.base, now);
        self.base = now.clone();
        if ops.is_empty() {
            return None;
        }
        let cid = self.next_cid;
        self.next_cid += 1;
        let wire = ops.iter().map(ClientOp::to_op).collect();
        self.pending.push_back((cid, ops));
        Some((cid, wire))
    }

    fn rebuild(&mut self) -> State {
        let mut s = self.shadow.clone();
        for (_, ops) in &self.pending {
            for o in ops {
                s.apply_op(o);
            }
        }
        self.base = s.clone();
        s
    }

    /// Operationen vom Server (in Sequenzreihenfolge). `own` = eigene `cid`, wenn es die Bestätigung eigener
    /// Operationen ist. Liefert den neuen sichtbaren Stand, wenn er sich durch fremde Operationen ändert.
    /// Vorher [`Sync::local`] aufrufen, damit keine lokalen Änderungen verloren gehen.
    pub fn remote(&mut self, ops: &[Op], own: Option<u64>) -> Option<State> {
        let cops: Vec<ClientOp> = ops.iter().filter_map(ClientOp::from_op).collect();
        for o in &cops {
            self.shadow.apply_op(o);
        }
        if let Some(cid) = own {
            if let Some(i) = self.pending.iter().position(|(c, _)| *c == cid) {
                // ältere unbestätigte gingen verloren (z. B. abgelehnt): ab hier neu aufbauen
                let older = i > 0;
                self.pending.drain(..=i);
                if !older {
                    return None;
                }
                return Some(self.rebuild());
            }
        }
        if cops.is_empty() {
            return None;
        }
        Some(self.rebuild())
    }

    /// Server hat Operationen abgelehnt (z. B. nur Lesen): verwerfen und Stand zurücksetzen.
    pub fn reject(&mut self, cid: u64) -> State {
        self.pending.retain(|(c, _)| *c != cid);
        self.rebuild()
    }

    /// Neu verbunden: `server` ist der aktuelle Stand des Servers. Alle unbestätigten Operationen werden darauf
    /// angewendet und neu gesendet (gleiche Operationen doppelt anzuwenden schadet nicht).
    /// Liefert den neuen sichtbaren Stand und die zu sendenden Operationen. `keep_local = false` (nur Lesen):
    /// lokale Änderungen verwerfen.
    pub fn reset(&mut self, server: State, keep_local: bool) -> (State, Vec<(u64, Vec<Op>)>) {
        self.shadow = server;
        if !keep_local {
            self.pending.clear();
        }
        let s = self.rebuild();
        let resend = self.pending.iter().map(|(c, ops)| (*c, ops.iter().map(ClientOp::to_op).collect())).collect();
        (s, resend)
    }
}

/// Übernimmt einen Stand von außen ins Board, ohne Undo-Schritt. Die Undo-/Redo-Schnappschüsse bekommen dieselben
/// Änderungen (Diff alter → neuer Stand), damit Rückgängig nur eigene Schritte zurücknimmt. Auswahl bleibt über
/// die uid erhalten.
pub fn apply_to_board(b: &mut Board, s: &State) {
    let old = State::of(b);
    let delta = diff(&old, s);
    if delta.is_empty() {
        return;
    }
    let item_delta: Vec<Change<Rc<Item>>> = delta.iter().filter_map(|o| if let ClientOp::Item(c) = o { Some(c.clone()) } else { None }).collect();
    let board = delta.iter().find_map(|o| if let ClientOp::Board(bs) = o { Some(bs.clone()) } else { None });
    b.patch_history(|items| {
        for c in &item_delta {
            apply(items, c, |it| it.uid);
        }
    });
    let layers = board.as_ref().map(|bs| bs.layers.iter().filter(|l| l.is_object()).map(Layer::from_json).collect::<Vec<_>>());
    let mode = board.as_ref().map(|bs| crate::doc::mode_from_manifest(&bs.mode));
    b.set_remote(s.items.clone(), layers, mode, board.as_ref().map(|bs| (bs.bg, bs.dark)));
}

/// Board-JSON (`board.json`) aus Objekten (als JSON) und Board-Zustand; Ansicht (`s`, `tx`, `ty`, `active`) aus
/// `base`.
pub fn board_json(items: &[Value], st: &BoardState, base: &Value) -> Value {
    let mut o: Map<String, Value> = base.as_object().cloned().unwrap_or_default();
    o.insert("items".into(), Value::Array(items.to_vec()));
    o.insert("layers".into(), Value::Array(st.layers.clone()));
    o.insert("bg".into(), json!(st.bg));
    o.insert("dark".into(), json!(st.dark));
    Value::Object(o)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::item::{INK, POLY};

    fn shape(x: f32) -> Rc<Item> {
        Rc::new(Item::shape(POLY, vec![x, 0.0, x + 10.0, 0.0, x + 10.0, 10.0], INK, 2.0))
    }

    fn uids(v: &[Rc<Item>]) -> Vec<u64> {
        v.iter().map(|i| i.uid).collect()
    }

    fn json_eq(a: &[Rc<Item>], b: &[Rc<Item>]) -> bool {
        a.len() == b.len() && a.iter().zip(b).all(|(x, y)| x.json() == y.json())
    }

    /// Kleiner Zufallsgenerator (deterministisch).
    struct Rng(u64);
    impl Rng {
        fn next(&mut self, n: usize) -> usize {
            self.0 ^= self.0 << 13;
            self.0 ^= self.0 >> 7;
            self.0 ^= self.0 << 17;
            (self.0 % n as u64) as usize
        }
    }

    #[test]
    fn uid_kompatibel_mit_altem_format() {
        // altes Item ohne uid (Java 2.x / Rust < 4.6): bekommt eine, die beim Speichern erhalten bleibt
        let old = json!({"t":"g","c":-16777216,"w":2.0,"k":5,"h":false,"q":[0.0,0.0,10.0,0.0,10.0,10.0],"ly":1});
        let a = Item::from_json(&old).unwrap();
        let b = Item::from_json(&old).unwrap();
        assert_ne!(a.uid, 0);
        assert_ne!(a.uid, b.uid);
        let j = a.json();
        assert_eq!(j["uid"].as_str().unwrap().len(), 16);
        let back = Item::from_json(&j).unwrap();
        assert_eq!(back.uid, a.uid);
        assert!(back.extra.is_none(), "uid ist kein unbekanntes Feld");
        // alle bisherigen Felder unverändert, nur uid dazu
        let mut without = j.as_object().unwrap().clone();
        without.remove("uid");
        assert_eq!(Value::Object(without), old);
        // Kopie bekommt eine neue uid, Rc::make_mut behält sie
        assert_ne!(a.dup().uid, a.uid);
        let mut rc = Rc::new(a.clone());
        let _keep = rc.clone();
        Rc::make_mut(&mut rc).width = 5.0;
        assert_eq!(rc.uid, a.uid);
        // ungültige uid → neue
        let mut bad = old.clone();
        bad["uid"] = json!("xyz");
        assert_ne!(Item::from_json(&bad).unwrap().uid, 0);
        // doppelte uids beim Laden eines Boards werden eindeutig
        let mut bd = Board::new(1.0);
        bd.from_json(&json!({"items": [j.clone(), j.clone()], "layers": []}));
        assert_eq!(bd.items.len(), 2);
        assert_ne!(bd.items[0].uid, bd.items[1].uid);
        assert!(bd.items[0].uid == a.uid || bd.items[1].uid == a.uid);
    }

    #[test]
    fn laser_nachricht() {
        let m = LaserMsg { peer: None, color: 0xFFFF0000, pts: vec![(1.04, 2.0), (3.0, -4.5)], size: 40.0, dot: Some((5.0, 6.0)), off: false };
        let j = m.to_json();
        assert_eq!(j["t"], "laser");
        let back = LaserMsg::from_json(&j).unwrap();
        assert_eq!(back.pts, vec![(1.0, 2.0), (3.0, -4.5)]);
        assert_eq!(back.color, 0xFFFF0000);
        assert_eq!((back.size, back.dot, back.off), (40.0, Some((5.0, 6.0)), false));
        // alte Nachricht ohne Punkt: Standardgröße, nur Spur
        let old = LaserMsg::from_json(&json!({"t":"laser","c":-65536,"p":[1,2]})).unwrap();
        assert_eq!((old.size, old.dot, old.off), (crate::laser::DOT_SIZE, None, false));
        let off = LaserMsg { peer: None, color: 0, pts: vec![], size: 20.0, dot: None, off: true };
        assert!(!LaserMsg::from_json(&off.to_json()).unwrap().is_empty());
        let mut j2 = j.clone();
        j2["peer"] = json!(7);
        assert_eq!(LaserMsg::from_json(&j2).unwrap().peer, Some(7));
        assert!(LaserMsg::from_json(&json!({"t":"cursor","x":1,"y":2})).is_none());
        assert!(LaserMsg::from_json(&json!({"t":"laser","p":[1,"x"]})).is_none());
        // keine Operation: wird nie gespeichert
        assert!(Op::from_json(&j).is_none());
    }

    #[test]
    fn op_json_roundtrip_und_pruefung() {
        let it = shape(1.0);
        let ops = vec![
            change_to_op(&Change::Upsert(it.clone(), None)),
            change_to_op(&Change::Upsert(it.clone(), Some(42))),
            Op::Item(Change::Move(it.uid, Some(7))),
            Op::Item(Change::Delete(it.uid)),
            Op::Board(BoardState { layers: vec![Layer::new(1, "a").json()], mode: json!({"mode": "infinite"}), bg: 2, dark: true }),
        ];
        let j = ops_to_json(&ops);
        assert_eq!(ops_from_json(&j).unwrap(), ops);
        // Item ohne uid, unbekannte Art, kaputtes Item → abgelehnt
        let mut no_uid = it.json();
        no_uid.as_object_mut().unwrap().remove("uid");
        assert!(Op::from_json(&json!({"o":"up","item": no_uid})).is_none());
        assert!(Op::from_json(&json!({"o":"zap"})).is_none());
        assert!(Op::from_json(&json!({"o":"up","item":{"t":"s","uid":"00000000000000ab"}})).is_none());
        assert!(Op::from_json(&json!({"o":"board","layers":[]})).is_none());
        assert!(ops_from_json(&json!([{"o":"del","uid":"01"}, {"o":"x"}])).is_none());
    }

    #[test]
    fn diff_und_apply_ergibt_neuen_stand() {
        let mut rng = Rng(0x9E37_79B9_7F4A_7C15);
        for round in 0..300 {
            let n = rng.next(12);
            let old: Vec<Rc<Item>> = (0..n).map(|i| shape(i as f32)).collect();
            let mut new = old.clone();
            // zufällige Änderungen: löschen, einfügen, ändern, umsortieren
            for _ in 0..rng.next(6) {
                match rng.next(4) {
                    0 if !new.is_empty() => {
                        let i = rng.next(new.len());
                        new.remove(i);
                    }
                    1 => {
                        let i = rng.next(new.len() + 1);
                        new.insert(i, shape(100.0 + round as f32));
                    }
                    2 if !new.is_empty() => {
                        let i = rng.next(new.len());
                        Rc::make_mut(&mut new[i]).width = 9.0;
                    }
                    _ if new.len() > 1 => {
                        let (i, j) = (rng.next(new.len()), rng.next(new.len()));
                        let x = new.remove(i);
                        new.insert(j, x);
                    }
                    _ => {}
                }
            }
            let d = diff_items(&old, &new);
            let mut got = old.clone();
            for c in &d {
                apply(&mut got, c, |it| it.uid);
            }
            assert_eq!(uids(&got), uids(&new), "Runde {round}");
            assert!(json_eq(&got, &new));
            // über das Protokoll (JSON) genauso
            let mut srv: Vec<Value> = old.iter().map(|i| i.json()).collect();
            for c in &d {
                let Op::Item(ch) = Op::from_json(&change_to_op(c).to_json()).unwrap() else { panic!() };
                apply(&mut srv, &ch, |v| uid_of_json(v).unwrap());
            }
            assert_eq!(srv, new.iter().map(|i| i.json()).collect::<Vec<_>>());
        }
        // nur Reihenfolge geändert: Move ohne Item-Daten; unverändert: nichts
        let a: Vec<Rc<Item>> = (0..5).map(|i| shape(i as f32)).collect();
        assert!(diff_items(&a, &a.clone()).is_empty());
        let mut b = a.clone();
        let x = b.remove(0);
        b.push(x);
        let d = diff_items(&a, &b);
        assert_eq!(d.len(), 1);
        assert!(matches!(d[0], Change::Move(_, Some(_))));
    }

    #[test]
    fn apply_mit_fehlendem_vorgaenger() {
        let a: Vec<Rc<Item>> = (0..3).map(|i| shape(i as f32)).collect();
        let mut l = a.clone();
        let n = shape(50.0);
        apply(&mut l, &Change::Upsert(n.clone(), Some(12345)), |it| it.uid);
        assert_eq!(l.last().unwrap().uid, n.uid, "neues Objekt ohne Vorgänger: nach oben");
        let mut c1 = (*a[1]).clone();
        c1.width = 7.0;
        let changed = Rc::new(c1);
        apply(&mut l, &Change::Upsert(changed, Some(999)), |it| it.uid);
        assert_eq!(l[1].width, 7.0, "vorhandenes bleibt an seiner Stelle");
        apply(&mut l, &Change::Move(a[0].uid, Some(999)), |it| it.uid);
        assert_eq!(l[0].uid, a[0].uid);
        apply(&mut l, &Change::Delete(777), |it| it.uid);
        assert_eq!(l.len(), 4);
    }

    fn state(items: &[Rc<Item>]) -> State {
        let b = Board::new(1.0);
        State { items: items.to_vec(), board: BoardState::of(&b) }
    }

    /// Server-Nachbildung: wendet Operationen in Reihenfolge an.
    fn srv_apply(srv: &mut Vec<Value>, ops: &[Op]) {
        for o in ops {
            if let Op::Item(c) = o {
                apply(srv, c, |v| uid_of_json(v).unwrap());
            }
        }
    }

    #[test]
    fn zwei_clients_laufen_zusammen() {
        let start: Vec<Rc<Item>> = (0..3).map(|i| shape(i as f32)).collect();
        let mut srv: Vec<Value> = start.iter().map(|i| i.json()).collect();
        let (mut a, mut b) = (Sync::new(state(&start)), Sync::new(state(&start)));
        let (mut la, mut lb) = (start.clone(), start.clone());
        // beide ändern gleichzeitig dasselbe Objekt und fügen etwas ein
        Rc::make_mut(&mut la[1]).width = 5.0;
        la.push(shape(10.0));
        Rc::make_mut(&mut lb[1]).width = 8.0;
        lb.remove(0);
        lb.push(shape(20.0));
        let (ca, oa) = a.local(&state(&la)).unwrap();
        let (cb, ob) = b.local(&state(&lb)).unwrap();
        // Server: erst b, dann a
        srv_apply(&mut srv, &ob);
        srv_apply(&mut srv, &oa);
        // a bekommt b (fremd), dann die eigene Bestätigung
        if let Some(s) = a.remote(&ob, None) {
            la = s.items;
        }
        assert!(a.remote(&oa, Some(ca)).is_none());
        // b bekommt die eigene Bestätigung, dann a
        assert!(b.remote(&ob, Some(cb)).is_none());
        if let Some(s) = b.remote(&oa, None) {
            lb = s.items;
        }
        let jv = |v: &[Rc<Item>]| v.iter().map(|i| i.json()).collect::<Vec<_>>();
        assert_eq!(jv(&la), srv);
        assert_eq!(jv(&lb), srv);
        assert_eq!(la[0].width, 5.0, "zuletzt angewendete Änderung (a) gewinnt");
        assert!(!a.has_pending() && !b.has_pending());
        assert!(a.local(&state(&la)).is_none(), "keine Echo-Schleife");
    }

    #[test]
    fn neu_verbinden_sendet_unbestaetigtes_und_ablehnen() {
        let start: Vec<Rc<Item>> = (0..2).map(|i| shape(i as f32)).collect();
        let mut s = Sync::new(state(&start));
        let mut l = start.clone();
        l.push(shape(5.0));
        let (cid, _) = s.local(&state(&l)).unwrap();
        // Server hat inzwischen etwas anderes (fremdes Objekt dazu)
        let mut server = start.clone();
        server.push(shape(9.0));
        let (vis, resend) = s.reset(state(&server), true);
        assert_eq!(resend.len(), 1);
        assert_eq!(resend[0].0, cid);
        assert_eq!(vis.items.len(), 4, "fremdes und eigenes Objekt");
        // abgelehnt: eigenes verschwindet wieder
        let back = s.reject(cid);
        assert_eq!(uids(&back.items), uids(&server));
        // nur lesen: lokale Änderungen verworfen
        let mut l2 = server.clone();
        l2.clear();
        s.local(&state(&l2)).unwrap();
        let (vis, resend) = s.reset(state(&server), false);
        assert!(resend.is_empty());
        assert_eq!(uids(&vis.items), uids(&server));
    }

    #[test]
    fn fremde_aenderung_laesst_undo_heil() {
        let mut b = Board::new(1.0);
        b.set_size(800.0, 600.0, 1.0);
        b.add_note(10.0, 10.0, "eigene", false);
        b.modified = false;
        let own = b.items[0].uid;
        let base = State::of(&b);
        let mut sync = Sync::new(base.clone());
        // fremdes Objekt kommt an
        let other = shape(300.0);
        let ops = vec![change_to_op(&Change::Upsert(other.clone(), Some(own)))];
        let s = sync.remote(&ops, None).unwrap();
        apply_to_board(&mut b, &s);
        assert_eq!(b.items.len(), 2);
        assert!(!b.modified, "fremde Änderung ist keine eigene");
        // Rückgängig nimmt nur die eigene Notiz zurück, das fremde Objekt bleibt
        b.undo();
        assert_eq!(uids(&b.items), vec![other.uid]);
        // Diff danach: nur das Löschen der eigenen Notiz
        let d = sync.local(&State::of(&b)).unwrap().1;
        assert_eq!(d, vec![Op::Item(Change::Delete(own))]);
        b.redo();
        assert_eq!(b.items.len(), 2);
    }

    #[test]
    fn board_zustand_und_auswahl() {
        let mut b = Board::new(1.0);
        b.set_size(800.0, 600.0, 1.0);
        b.add_note(10.0, 10.0, "x", false);
        b.select_only(b.items[0].id);
        let mut s = State::of(&b);
        s.board.dark = true;
        s.board.layers.push(Layer::new(2, "neu").json());
        // gleiches Objekt mit neuer Breite (wie von einem anderen Client)
        let mut it = (*s.items[0]).clone();
        it.width = 6.0;
        let uid = it.uid;
        s.items[0] = Rc::new(Item::from_json(&it.json()).unwrap());
        apply_to_board(&mut b, &s);
        assert!(b.dark);
        assert_eq!(b.layers.len(), 2);
        assert_eq!(b.items[0].width, 6.0);
        assert_eq!(b.items[0].uid, uid);
        assert_eq!(b.sel, vec![b.items[0].id], "Auswahl bleibt über die uid erhalten");
    }
}
