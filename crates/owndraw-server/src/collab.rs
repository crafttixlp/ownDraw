//! Live-Zusammenarbeit: WebSocket `GET /api/collab/{board_id}` und Bilder (`/api/boards/{id}/images/{name}`).
//! Protokoll: `docs/web-api.md`, Abschnitt „Kollaboration“.
//!
//! Pro geöffnetem Board gibt es einen Raum im Speicher mit dem aktuellen Stand (Objekte als JSON, Board-Zustand,
//! restliches Dokument). Der Server vergibt Sequenznummern, wendet Operationen in dieser Reihenfolge an (Funktionen
//! aus `owndraw_core::collab`), verteilt sie an alle Teilnehmer (auch an den Absender, als Bestätigung) und
//! speichert gedrosselt ([`SAVE_EVERY`]) sowie beim Verlassen des letzten Teilnehmers. Schreiben dürfen
//! `owner`/`edit`, solange die Lizenz der Besitzerin/des Besitzers gültig ist; `view` empfängt nur.

use crate::auth::{read_limited, Auth};
use crate::boards::{board_path, board_row, license_msg_owner, visible, write_atomic};
use crate::db::{self, now};
use crate::error::{ApiError, ApiResult};
use crate::state::AppState;
use axum::body::Body;
use axum::extract::ws::{CloseFrame, Message, WebSocket, WebSocketUpgrade};
use axum::extract::{Path, State};
use axum::http::header::{CACHE_CONTROL, CONTENT_TYPE};
use axum::http::{HeaderMap, HeaderValue};
use axum::response::{IntoResponse, Response};
use axum::Json;
use futures_util::{SinkExt, StreamExt};
use owndraw_core::collab::{self as oc, BoardState, Op};
use owndraw_core::doc::Document;
use rusqlite::params;
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::collections::{HashMap, HashSet};
use std::sync::{Arc, Mutex, MutexGuard};
use std::time::{Duration, Instant};
use tokio::sync::mpsc;

/// Speichern spätestens so oft, solange es Änderungen gibt.
pub const SAVE_EVERY: Duration = Duration::from_secs(5);
/// So lange gilt eine Lizenzprüfung, bevor sie bei der nächsten Operation wiederholt wird.
const LICENSE_RECHECK: Duration = Duration::from_secs(30);
/// Protokollversion (Feld `proto` im Hello).
pub const PROTO: u32 = 1;
const COLORS: [&str; 8] = ["#e5484d", "#3e63dd", "#30a46c", "#f76b15", "#8e4ec6", "#12a594", "#d6409f", "#c2a000"];

fn lock<T>(m: &Mutex<T>) -> MutexGuard<'_, T> {
    m.lock().unwrap_or_else(|p| p.into_inner())
}

enum Out {
    Text(String),
    Close(u16, String),
}

struct Peer {
    id: u64,
    user_id: String,
    name: String,
    color: &'static str,
    role: String,
    /// Admins bleiben im Wartungsmodus verbunden.
    admin: bool,
    guest: bool,
    tx: mpsc::UnboundedSender<Out>,
}

impl Peer {
    fn json(&self) -> Value {
        json!({"peer": self.id, "user_id": self.user_id, "name": self.name, "color": self.color, "role": self.role})
    }
    fn send(&self, v: &Value) {
        let _ = self.tx.send(Out::Text(v.to_string()));
    }
}

struct RoomState {
    /// Übriges Dokument (Meta, Bilder, Vorschau, Ansicht in `board`); `items`/Ebenen kommen aus den Feldern unten.
    doc: Document,
    items: Vec<Value>,
    board: BoardState,
    seq: u64,
    version: i64,
    owner_id: String,
    dirty: bool,
    write_ok: bool,
    license_at: Instant,
    peers: Vec<Peer>,
    next_peer: u64,
    /// Raum aufgelöst (letzter Teilnehmer weg oder Board gelöscht).
    closed: bool,
}

impl RoomState {
    fn broadcast(&self, v: &Value, except: Option<u64>) {
        let s = v.to_string();
        for p in self.peers.iter().filter(|p| Some(p.id) != except) {
            let _ = p.tx.send(Out::Text(s.clone()));
        }
    }
    fn can_write(&self, role: &str) -> bool {
        self.write_ok && role != "view"
    }
    fn board_json(&self) -> Value {
        oc::board_json(&self.items, &self.board, &self.doc.board)
    }
    fn set_write_ok(&mut self, ok: bool) {
        if ok == self.write_ok {
            return;
        }
        self.write_ok = ok;
        for p in &self.peers {
            p.send(&json!({"t": "access", "can_write": self.can_write(&p.role)}));
        }
    }
}

pub struct Room {
    id: String,
    st: Mutex<RoomState>,
}

/// Kurzinfo zu einem offenen Raum (`Hub::snapshot`).
pub struct RoomInfo {
    pub id: String,
    pub peers: Vec<(String, bool)>,
}

/// Alle offenen Räume.
#[derive(Default)]
pub struct Hub {
    rooms: Mutex<HashMap<String, Arc<Room>>>,
}

/// Vergibt fehlende/doppelte uids (ältere Dateien) und liefert die Objekte.
fn items_with_uids(board: &Value) -> Vec<Value> {
    let mut seen = HashSet::new();
    let mut out = Vec::new();
    for it in board.get("items").and_then(Value::as_array).into_iter().flatten() {
        if owndraw_core::item::Item::from_json(it).is_none() {
            continue;
        }
        let mut it = it.clone();
        let u = match oc::uid_of_json(&it) {
            Some(u) if !seen.contains(&u) => u,
            _ => {
                let u = owndraw_core::item::new_uid();
                it["uid"] = json!(owndraw_core::item::uid_str(u));
                u
            }
        };
        seen.insert(u);
        out.push(it);
    }
    out
}

impl Hub {
    /// Wartungsmodus: trennt alle Live-Verbindungen außer denen von Admins (Fehler `maintenance`, Close 4503).
    pub fn kick_non_admins(&self, message: &str) {
        let rooms: Vec<Arc<Room>> = lock(&self.rooms).values().cloned().collect();
        let msg = json!({"t": "error", "code": "maintenance", "message": message});
        for r in rooms {
            let st = lock(&r.st);
            for p in st.peers.iter().filter(|p| !p.admin) {
                p.send(&msg);
                let _ = p.tx.send(Out::Close(4503, "maintenance".into()));
            }
        }
    }

    /// Offene Räume mit ihren Teilnehmern (Nutzer-ID, Gast) für die Admin-Statistik.
    pub fn snapshot(&self) -> Vec<RoomInfo> {
        let rooms: Vec<Arc<Room>> = lock(&self.rooms).values().cloned().collect();
        rooms
            .iter()
            .filter_map(|r| {
                let st = lock(&r.st);
                (!st.closed && !st.peers.is_empty())
                    .then(|| RoomInfo { id: r.id.clone(), peers: st.peers.iter().map(|p| (p.user_id.clone(), p.guest)).collect() })
            })
            .collect()
    }

    /// Offener Raum zu einem Board (für PUT/Bilder).
    pub fn get(&self, id: &str) -> Option<Arc<Room>> {
        lock(&self.rooms).get(id).cloned()
    }

    pub fn is_active(&self, id: &str) -> bool {
        lock(&self.rooms).contains_key(id)
    }

    /// Verbindungen beenden: alle eines Boards (`user = None`, z. B. gelöscht) oder die eines Nutzers (Rolle
    /// geändert/entfernt; der Client verbindet sich neu und bekommt die neue Rolle).
    pub fn kick(&self, board_id: &str, user: Option<&str>, code: &str, msg: &str) {
        let Some(room) = self.get(board_id) else { return };
        let mut st = lock(&room.st);
        let err = json!({"t": "error", "code": code, "message": msg});
        st.peers.retain(|p| {
            if user.is_none_or(|u| u == p.user_id) {
                p.send(&err);
                let _ = p.tx.send(Out::Close(4000, code.to_string()));
                false
            } else {
                true
            }
        });
        if user.is_none() {
            st.closed = true;
            st.dirty = false;
            drop(st);
            lock(&self.rooms).remove(board_id);
        }
    }

    /// Alle Live-Verbindungen eines Nutzers in allen Räumen beenden (gesperrt oder gelöscht).
    pub fn kick_user(&self, user_id: &str, code: &str, msg: &str) {
        let ids: Vec<String> = lock(&self.rooms).keys().cloned().collect();
        for id in ids {
            self.kick(&id, Some(user_id), code, msg);
        }
    }

    /// Raum holen oder das Board laden und einen neuen Raum anlegen.
    async fn open(&self, s: &AppState, board_id: &str) -> ApiResult<Arc<Room>> {
        if let Some(r) = self.get(board_id) {
            return Ok(r);
        }
        let id = board_id.to_string();
        let row = s.db.call(move |c| Ok(board_row(c, &id)?)).await?.ok_or_else(|| ApiError::not_found("Board nicht gefunden."))?;
        let path = board_path(&s.cfg.boards_dir(), board_id);
        let doc = tokio::task::spawn_blocking(move || -> Result<Document, String> {
            let bytes = std::fs::read(&path).map_err(|e| e.to_string())?;
            owndraw_core::doc::read_owndraw(&bytes).map_err(|e| e.to_string())
        })
        .await
        .map_err(ApiError::internal)?
        .map_err(|e| ApiError::internal(format!("Board {board_id} nicht lesbar: {e}")))?;
        let items = items_with_uids(&doc.board);
        let board = BoardState::from_board_json(&doc.board, &owndraw_core::doc::mode_to_json(&doc.mode));
        let room = Arc::new(Room {
            id: board_id.to_string(),
            st: Mutex::new(RoomState {
                doc,
                items,
                board,
                seq: 0,
                version: row.version,
                owner_id: row.owner_id,
                dirty: false,
                write_ok: false,
                license_at: Instant::now() - LICENSE_RECHECK * 2,
                peers: Vec::new(),
                next_peer: 1,
                closed: false,
            }),
        });
        let mut rooms = lock(&self.rooms);
        if let Some(r) = rooms.get(board_id) {
            return Ok(r.clone()); // gleichzeitig von jemand anderem geöffnet
        }
        rooms.insert(board_id.to_string(), room.clone());
        drop(rooms);
        let (s2, r2) = (s.clone(), room.clone());
        tokio::spawn(async move {
            loop {
                tokio::time::sleep(SAVE_EVERY).await;
                if lock(&r2.st).closed {
                    break;
                }
                save(&s2, &r2).await;
                try_close(&s2, &r2);
            }
        });
        Ok(room)
    }

    /// Alle offenen Räume speichern (beim Beenden des Servers).
    pub async fn flush_all(&self, s: &AppState) {
        let rooms: Vec<Arc<Room>> = lock(&self.rooms).values().cloned().collect();
        for r in rooms {
            save(s, &r).await;
        }
    }
}

/// Serialisiert den Stand (mit neuer Vorschau) und schreibt ihn, falls es Änderungen gibt.
async fn save(s: &AppState, room: &Arc<Room>) {
    let (mut doc, owner) = {
        let mut st = lock(&room.st);
        if !st.dirty || st.closed {
            return;
        }
        st.dirty = false;
        let mut d = st.doc.clone();
        d.board = st.board_json();
        d.mode = owndraw_core::doc::mode_from_manifest(&st.board.mode);
        (d, st.owner_id.clone())
    };
    let res = tokio::task::spawn_blocking(move || -> Result<Vec<u8>, String> {
        doc.meta.modified = owndraw_core::doc::rfc3339(owndraw_core::store::now_ms());
        doc.thumb = render_thumb(&doc).or(doc.thumb.take());
        owndraw_core::doc::write_owndraw(&doc).map_err(|e| e.to_string())
    })
    .await;
    let bytes = match res {
        Ok(Ok(b)) => b,
        other => {
            eprintln!("Board {} nicht serialisierbar: {other:?}", room.id);
            lock(&room.st).dirty = true;
            return;
        }
    };
    let (id, dir) = (room.id.clone(), s.cfg.boards_dir());
    let r = s
        .db
        .call(move |c| {
            let t = now();
            let b = board_row(c, &id)?.ok_or_else(|| ApiError::not_found("Board nicht gefunden."))?;
            let lic = db::active_license_of(c, &owner, t)?.ok_or_else(|| ApiError::license_required(license_msg_owner(false)))?;
            if db::used_bytes(c, &owner)? - b.size + bytes.len() as i64 > lic.quota_mb * 1_048_576 {
                return Err(ApiError::quota(format!("Der Speicherplatz ({} MB) ist voll.", lic.quota_mb)));
            }
            write_atomic(&board_path(&dir, &id), &bytes)?;
            c.execute("UPDATE boards SET version=version+1, size=?2, modified=?3 WHERE id=?1", params![id, bytes.len() as i64, t])?;
            Ok(b.version + 1)
        })
        .await;
    let mut st = lock(&room.st);
    match r {
        Ok(v) => {
            st.version = v;
            st.broadcast(&json!({"t": "saved", "version": v}), None);
        }
        Err(e) => {
            eprintln!("Board {} nicht gespeichert: {}", room.id, e.message);
            if e.code != "not_found" {
                st.dirty = true;
            }
            if e.code == "license_required" || e.code == "quota" {
                st.set_write_ok(false);
            }
            st.broadcast(&json!({"t": "error", "code": e.code, "message": e.message}), None);
        }
    }
}

/// Vorschaubild wie in der App (480 × 300), mit den Bildern des Dokuments.
fn render_thumb(doc: &Document) -> Option<Vec<u8>> {
    let r = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        owndraw_core::item::clear_mem_images();
        for (n, b) in &doc.images.files {
            owndraw_core::item::put_mem_image(n, b.clone());
        }
        let mut b = owndraw_core::board::Board::new(1.0);
        b.from_json(&doc.board);
        b.board_mode = doc.mode.clone();
        let png = b.thumb().encode_png().ok();
        owndraw_core::item::clear_mem_images();
        png
    }));
    r.ok().flatten()
}

// ---------- WebSocket ----------

/// Browser schicken bei WebSockets immer `Origin`; er muss zur eigenen Adresse passen (Schutz vor fremden Seiten,
/// die mit dem Cookie der Nutzerin eine Verbindung öffnen). Ohne `Origin` (andere Clients) erlaubt.
fn check_origin(h: &HeaderMap) -> ApiResult<()> {
    let Some(origin) = h.get("origin").and_then(|v| v.to_str().ok()) else { return Ok(()) };
    let host = h.get("x-forwarded-host").or_else(|| h.get("host")).and_then(|v| v.to_str().ok()).unwrap_or("");
    let o_host = origin.split_once("://").map(|(_, r)| r).unwrap_or(origin).trim_end_matches('/');
    if !host.is_empty() && o_host.eq_ignore_ascii_case(host) {
        Ok(())
    } else {
        Err(ApiError::forbidden("Verbindung von einer fremden Seite abgelehnt."))
    }
}

pub async fn ws(
    State(s): State<AppState>,
    a: Auth,
    Path(board_id): Path<String>,
    headers: HeaderMap,
    up: Result<WebSocketUpgrade, axum::extract::ws::rejection::WebSocketUpgradeRejection>,
) -> ApiResult<Response> {
    check_origin(&headers)?;
    let up = up.map_err(|_| ApiError::invalid("WebSocket-Verbindung erwartet."))?;
    let (uid, bid) = (a.user.id.clone(), board_id.clone());
    let (role, write_ok) =
        s.db.call(move |c| {
            let (b, role) = visible(c, &bid, &uid)?;
            Ok((role, db::active_license_of(c, &b.owner_id, now())?.is_some()))
        })
        .await?;
    let limit = s.cfg.max_board_bytes as usize + 1024 * 1024;
    Ok(up.max_message_size(limit).on_upgrade(move |sock| session(s, sock, board_id, a.user, role, write_ok)))
}

async fn session(s: AppState, sock: WebSocket, board_id: String, user: db::UserRow, role: String, write_ok: bool) {
    let (mut sink, mut stream) = sock.split();
    let room = match s.hub.open(&s, &board_id).await {
        Ok(r) => r,
        Err(e) => {
            let _ = sink.send(Message::Text(json!({"t": "error", "code": e.code, "message": e.message}).to_string().into())).await;
            let _ = sink.send(Message::Close(Some(CloseFrame { code: 4000, reason: e.code.into() }))).await;
            return;
        }
    };
    let (tx, mut rx) = mpsc::unbounded_channel::<Out>();
    let peer_id = {
        let mut st = lock(&room.st);
        st.license_at = Instant::now();
        st.set_write_ok(write_ok);
        let id = st.next_peer;
        st.next_peer += 1;
        let peer = Peer { id, user_id: user.id.clone(), name: user.name.clone(), color: COLORS[(id as usize - 1) % COLORS.len()], role: role.clone(), admin: user.admin, guest: user.guest, tx };
        let hello = json!({
            "t": "hello",
            "proto": PROTO,
            "you": peer.json(),
            "can_write": st.can_write(&role),
            "seq": st.seq,
            "version": st.version,
            "peers": st.peers.iter().map(Peer::json).collect::<Vec<_>>(),
            "board": st.board_json(),
            "mode": st.board.mode,
        });
        peer.send(&hello);
        st.broadcast(&json!({"t": "join", "peer": peer.json()}), None);
        st.peers.push(peer);
        id
    };
    let writer = tokio::spawn(async move {
        while let Some(m) = rx.recv().await {
            let r = match m {
                Out::Text(t) => sink.send(Message::Text(t.into())).await,
                Out::Close(code, reason) => {
                    let _ = sink.send(Message::Close(Some(CloseFrame { code, reason: reason.into() }))).await;
                    break;
                }
            };
            if r.is_err() {
                break;
            }
        }
    });
    while let Some(Ok(msg)) = stream.next().await {
        match msg {
            Message::Text(t) => handle(&s, &room, peer_id, &t).await,
            Message::Close(_) => break,
            _ => {}
        }
        if writer.is_finished() {
            break;
        }
    }
    writer.abort();
    leave(&s, &room, peer_id).await;
}

async fn leave(s: &AppState, room: &Arc<Room>, peer_id: u64) {
    let empty = {
        let mut st = lock(&room.st);
        let before = st.peers.len();
        st.peers.retain(|p| p.id != peer_id);
        if st.peers.len() != before {
            st.broadcast(&json!({"t": "leave", "peer": peer_id}), None);
        }
        st.peers.is_empty() && !st.closed
    };
    if !empty {
        return;
    }
    save(s, room).await;
    try_close(s, room);
}

/// Raum auflösen, wenn niemand mehr verbunden und alles gespeichert ist.
fn try_close(s: &AppState, room: &Arc<Room>) {
    let mut rooms = lock(&s.hub.rooms);
    let mut st = lock(&room.st);
    if st.peers.is_empty() && !st.dirty && !st.closed {
        st.closed = true;
        if rooms.get(&room.id).is_some_and(|r| Arc::ptr_eq(r, room)) {
            rooms.remove(&room.id);
        }
    }
}

fn nack(room: &Room, peer_id: u64, cid: &Value, code: &str, msg: &str) {
    let st = lock(&room.st);
    if let Some(p) = st.peers.iter().find(|p| p.id == peer_id) {
        p.send(&json!({"t": "nack", "cid": cid, "code": code, "message": msg}));
    }
}

async fn handle(s: &AppState, room: &Arc<Room>, peer_id: u64, text: &str) {
    let Ok(m) = serde_json::from_str::<Value>(text) else { return };
    match m.get("t").and_then(Value::as_str) {
        Some("ops") => {
            let cid = m.get("cid").cloned().unwrap_or(Value::Null);
            // Rolle und Lizenz
            let (role, recheck, owner) = {
                let st = lock(&room.st);
                let Some(p) = st.peers.iter().find(|p| p.id == peer_id) else { return };
                (p.role.clone(), st.license_at.elapsed() >= LICENSE_RECHECK, st.owner_id.clone())
            };
            if role == "view" {
                return nack(room, peer_id, &cid, "forbidden", "Du darfst dieses Board nur ansehen.");
            }
            if recheck {
                let ok = s.db.call(move |c| Ok(db::active_license_of(c, &owner, now())?.is_some())).await.unwrap_or(false);
                let mut st = lock(&room.st);
                st.license_at = Instant::now();
                st.set_write_ok(ok);
            }
            let Some(ops) = m.get("ops").and_then(oc::ops_from_json) else {
                return nack(room, peer_id, &cid, "invalid", "Ungültige Operation.");
            };
            let mut st = lock(&room.st);
            if !st.write_ok {
                drop(st);
                return nack(room, peer_id, &cid, "license_required", &license_msg_owner(false));
            }
            for op in &ops {
                match op {
                    Op::Item(c) => oc::apply(&mut st.items, c, |v| oc::uid_of_json(v).unwrap_or(0)),
                    Op::Board(b) => st.board = b.clone(),
                }
            }
            if ops.is_empty() {
                return;
            }
            st.seq += 1;
            st.dirty = true;
            let seq = st.seq;
            st.broadcast(&json!({"t": "ops", "seq": seq, "peer": peer_id, "cid": cid, "ops": oc::ops_to_json(&ops)}), None);
        }
        Some("cursor") => {
            let f = |k: &str| m.get(k).and_then(Value::as_f64).filter(|v| v.is_finite());
            let pos = f("x").zip(f("y"));
            let st = lock(&room.st);
            let (x, y) = pos.map(|(x, y)| (json!(x), json!(y))).unwrap_or((Value::Null, Value::Null));
            st.broadcast(&json!({"t": "cursor", "peer": peer_id, "x": x, "y": y}), Some(peer_id));
        }
        Some("laser") => {
            // Laserpointer: nur weiterreichen (nie gespeichert); auch Gäste mit Leserecht dürfen zeigen
            let Some(mut l) = oc::LaserMsg::from_json(&m) else { return };
            if l.pts.is_empty() {
                return;
            }
            l.peer = Some(peer_id);
            let st = lock(&room.st);
            st.broadcast(&l.to_json(), Some(peer_id));
        }
        Some("ping") => {
            let st = lock(&room.st);
            if let Some(p) = st.peers.iter().find(|p| p.id == peer_id) {
                p.send(&json!({"t": "pong"}));
            }
        }
        _ => {}
    }
}

// ---------- Bilder ----------

/// Bildnamen sind inhaltsadressiert: `<sha256>.<endung>`.
fn check_image_name(name: &str) -> ApiResult<(&str, &str)> {
    let (hash, ext) = name.split_once('.').ok_or_else(|| ApiError::invalid("Ungültiger Bildname."))?;
    let ok = hash.len() == 64
        && hash.bytes().all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
        && (1..=5).contains(&ext.len())
        && ext.bytes().all(|b| b.is_ascii_lowercase() || b.is_ascii_digit());
    if ok {
        Ok((hash, ext))
    } else {
        Err(ApiError::invalid("Ungültiger Bildname."))
    }
}

fn mime_of(ext: &str) -> &'static str {
    match ext {
        "jpg" | "jpeg" => "image/jpeg",
        "gif" => "image/gif",
        "webp" => "image/webp",
        "bmp" => "image/bmp",
        _ => "image/png",
    }
}

/// `PUT /api/boards/{id}/images/{name}`: Bild für eine laufende Live-Sitzung hochladen (vor der Operation, die es
/// verwendet). Nur `owner`/`edit`, Lizenz der Besitzerin/des Besitzers nötig.
pub async fn put_image(State(s): State<AppState>, a: Auth, Path((id, name)): Path<(String, String)>, body: Body) -> ApiResult<Json<Value>> {
    let (hash, _) = check_image_name(&name)?;
    let bytes = read_limited(body, s.cfg.max_board_bytes).await?;
    if hex::encode(Sha256::digest(&bytes)) != hash {
        return Err(ApiError::invalid("Der Bildname passt nicht zum Inhalt (SHA-256)."));
    }
    let (uid, bid) = (a.user.id, id.clone());
    s.db.call(move |c| {
        let (b, role) = visible(c, &bid, &uid)?;
        if role == "view" {
            return Err(ApiError::forbidden("Du darfst dieses Board nur ansehen."));
        }
        db::active_license_of(c, &b.owner_id, now())?.ok_or_else(|| ApiError::license_required(license_msg_owner(role == "owner")))?;
        Ok(())
    })
    .await?;
    let room = s.hub.get(&id).ok_or_else(|| ApiError::conflict("Für dieses Board läuft keine Live-Sitzung."))?;
    let mut st = lock(&room.st);
    st.doc.images.files.entry(name).or_insert_with(|| bytes.to_vec());
    st.dirty = true;
    Ok(Json(json!({})))
}

/// `GET /api/boards/{id}/images/{name}`: Bild eines Boards (aus der Live-Sitzung oder der gespeicherten Datei).
pub async fn get_image(State(s): State<AppState>, a: Auth, Path((id, name)): Path<(String, String)>) -> ApiResult<Response> {
    let (_, ext) = check_image_name(&name)?;
    let mime = mime_of(ext);
    let (uid, bid) = (a.user.id, id.clone());
    s.db.call(move |c| visible(c, &bid, &uid).map(|_| ())).await?;
    let from_room = s.hub.get(&id).and_then(|r| lock(&r.st).doc.images.files.get(&name).cloned());
    let bytes = match from_room {
        Some(b) => b,
        None => {
            let path = board_path(&s.cfg.boards_dir(), &id);
            let n = name.clone();
            tokio::task::spawn_blocking(move || {
                let bytes = std::fs::read(path).ok()?;
                owndraw_core::doc::read_owndraw(&bytes).ok()?.images.files.remove(&n)
            })
            .await
            .map_err(ApiError::internal)?
            .ok_or_else(|| ApiError::not_found("Bild nicht gefunden."))?
        }
    };
    let mut r = bytes.into_response();
    r.headers_mut().insert(CONTENT_TYPE, HeaderValue::from_static(mime));
    r.headers_mut().insert(CACHE_CONTROL, HeaderValue::from_static("private, max-age=31536000, immutable"));
    Ok(r)
}
