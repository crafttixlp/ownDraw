//! Server-Anbindung der Web-App (Vertrag: `docs/web-api.md`). Alles läuft unter derselben Origin: Cookie
//! `owndraw_session` (vom Browser verwaltet, `credentials: same-origin`), schreibende Anfragen mit `X-OwnDraw: 1`.
//!
//! - Beim Start `GET /api/me`: 200 → angemeldet, 401 → Server da, aber Gast; alles andere (statisches Hosting
//!   ohne `/api`, Netzfehler) → [`Server::Offline`], die App bleibt im Gastmodus.
//! - Boards am Server sind `FileRef::Uri("srv:<id>")`: Öffnen lädt die Bytes vorab (`open_board`) in
//!   [`Net::files`], danach öffnet die App sie wie jede Datei. `write_atomic` darauf speichert per PUT mit
//!   `If-Match` (Autosave), außer das Board ist gerade live verbunden (dann speichert der Server).
//! - Antworten kommen asynchron; sie landen in [`Net`] (thread-lokal) und lösen ein Neuzeichnen aus.
//! - [`Socket`]: WebSocket für die Live-Zusammenarbeit (`/api/collab/{id}`), siehe `ui/editor/live.rs`.

use crate::platform::FileRef;
use serde_json::{json, Value};
use std::cell::RefCell;
use std::collections::{HashMap, HashSet};
use std::rc::Rc;
use wasm_bindgen::prelude::*;
use wasm_bindgen::JsCast;
use wasm_bindgen_futures::{spawn_local, JsFuture};

/// Präfix der Verweise auf Server-Boards.
pub const SRV: &str = "srv:";

/// Board-ID eines Server-Verweises.
pub fn srv_id(f: &FileRef) -> Option<&str> {
    match f {
        FileRef::Uri(u) => u.strip_prefix(SRV),
        _ => None,
    }
}

pub fn srv_ref(id: &str) -> FileRef {
    FileRef::Uri(format!("{SRV}{id}"))
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Server {
    /// Noch keine Antwort auf `GET /api/me`.
    Unknown,
    /// Kein Server (statisches Hosting, offline): reiner Gastmodus.
    Offline,
    Ready,
}

#[derive(Clone, Debug, Default)]
pub struct License {
    pub active: bool,
    pub expires_at: Option<i64>,
    pub quota_mb: f64,
    pub used_mb: f64,
}

/// Angemeldeter Nutzer (Antwort `Me`).
#[derive(Clone, Debug, Default)]
pub struct Session {
    pub id: String,
    pub email: String,
    pub name: String,
    pub admin: bool,
    pub license: Option<License>,
}

impl Session {
    /// Gültige Lizenz: Boards am Server speichern und teilen.
    pub fn licensed(&self) -> bool {
        self.license.as_ref().is_some_and(|l| l.active)
    }
    fn parse(v: &Value) -> Option<Session> {
        let s = |k: &str| v.get(k).and_then(Value::as_str).unwrap_or("").to_string();
        let license = v.get("license").filter(|l| l.is_object()).map(|l| License {
            active: l.get("active").and_then(Value::as_bool).unwrap_or(false),
            expires_at: l.get("expires_at").and_then(Value::as_i64),
            quota_mb: l.get("quota_mb").and_then(Value::as_f64).unwrap_or(0.0),
            used_mb: l.get("used_mb").and_then(Value::as_f64).unwrap_or(0.0),
        });
        Some(Session { id: v.get("id")?.as_str()?.to_string(), email: s("email"), name: s("name"), admin: v.get("admin").and_then(Value::as_bool).unwrap_or(false), license })
    }
}

#[derive(Clone, Debug)]
pub struct BoardInfo {
    pub id: String,
    pub name: String,
    pub owner_name: String,
    /// `owner`, `edit` oder `view`.
    pub role: String,
    pub version: i64,
    pub size: i64,
    pub modified: i64,
}

impl BoardInfo {
    fn parse(v: &Value) -> Option<BoardInfo> {
        Some(BoardInfo {
            id: v.get("id")?.as_str()?.to_string(),
            name: v.get("name").and_then(Value::as_str).unwrap_or("Board").to_string(),
            owner_name: v.pointer("/owner/name").and_then(Value::as_str).unwrap_or("").to_string(),
            role: v.get("role").and_then(Value::as_str).unwrap_or("view").to_string(),
            version: v.get("version").and_then(Value::as_i64).unwrap_or(0),
            size: v.get("size").and_then(Value::as_i64).unwrap_or(0),
            modified: v.get("modified").and_then(Value::as_i64).unwrap_or(0),
        })
    }
}

#[derive(Clone, Debug)]
pub struct Member {
    pub user_id: String,
    pub email: String,
    pub name: String,
    pub role: String,
}

/// Laufendes/vorgemerktes Speichern eines Server-Boards.
#[derive(Default)]
struct Put {
    inflight: bool,
    queued: Option<Vec<u8>>,
}

/// Zustand der Server-Anbindung (thread-lokal, siehe [`with`]).
pub struct Net {
    pub server: Server,
    pub me: Option<Session>,
    pub boards: Vec<BoardInfo>,
    pub boards_loaded: bool,
    /// Anfrage eines Formulars (Anmelden, Einlösen …) läuft.
    pub busy: bool,
    /// Fehlertext der letzten Formular-Anfrage.
    pub form_error: Option<String>,
    /// Formular erfolgreich abgeschickt (Dialog schließen).
    pub form_ok: bool,
    /// Hinweise für die Anzeige (Toast).
    pub notices: Vec<String>,
    /// Fertig geladene Boards, die geöffnet werden sollen.
    pub open: Vec<FileRef>,
    /// Inhalte der Server-Boards (`srv:<id>` → Bytes), Quelle für `Platform::read`.
    pub files: HashMap<String, Vec<u8>>,
    /// Bekannte Version je Board (für `If-Match`).
    pub versions: HashMap<String, i64>,
    /// Boards mit Versionskonflikt beim Speichern (409).
    pub conflicts: HashSet<String>,
    /// Letzter Speicherfehler je Board.
    pub save_errors: HashMap<String, String>,
    /// Boards, die gerade live verbunden sind (der Server speichert, kein PUT).
    pub live: HashSet<String>,
    /// Mitgliederliste des Teilen-Dialogs (Board-ID, Liste).
    pub members: Option<(String, Vec<Member>)>,
    /// Ein Board wird gerade geöffnet/angelegt.
    pub opening: bool,
    puts: HashMap<String, Put>,
}

thread_local! {
    static CTX: RefCell<Option<egui::Context>> = const { RefCell::new(None) };
    static NET: RefCell<Net> = RefCell::new(Net {
        server: Server::Unknown,
        me: None,
        boards: Vec::new(),
        boards_loaded: false,
        busy: false,
        form_error: None,
        form_ok: false,
        notices: Vec::new(),
        open: Vec::new(),
        files: HashMap::new(),
        versions: HashMap::new(),
        conflicts: HashSet::new(),
        save_errors: HashMap::new(),
        live: HashSet::new(),
        members: None,
        opening: false,
        puts: HashMap::new(),
    });
}

/// Zugriff auf den Zustand.
pub fn with<R>(f: impl FnOnce(&mut Net) -> R) -> R {
    NET.with(|n| f(&mut n.borrow_mut()))
}

fn repaint() {
    CTX.with(|c| {
        if let Some(c) = c.borrow().as_ref() {
            c.request_repaint();
        }
    });
}

/// Beim Start: Kontext merken, Anmeldung prüfen.
pub fn init(ctx: &egui::Context) {
    CTX.with(|c| *c.borrow_mut() = Some(ctx.clone()));
    refresh_me();
}

pub fn online() -> bool {
    with(|n| n.server == Server::Ready)
}

pub fn licensed() -> bool {
    with(|n| n.me.as_ref().is_some_and(Session::licensed))
}

pub fn logged_in() -> bool {
    with(|n| n.me.is_some())
}

// ---------------------------------------------------------------- HTTP

pub enum Body {
    Json(Value),
    Bytes(Vec<u8>),
}

pub struct Resp {
    pub status: u16,
    pub version: Option<i64>,
    pub body: Vec<u8>,
}

impl Resp {
    pub fn ok(&self) -> bool {
        (200..300).contains(&self.status)
    }
    pub fn json(&self) -> Value {
        serde_json::from_slice(&self.body).unwrap_or(Value::Null)
    }
    /// Fehlertext des Servers (`message`), sonst ein allgemeiner.
    pub fn message(&self) -> String {
        self.json().get("message").and_then(Value::as_str).map(String::from).unwrap_or_else(|| format!("Serverfehler ({})", self.status))
    }
    pub fn code(&self) -> String {
        self.json().get("error").and_then(Value::as_str).unwrap_or("").to_string()
    }
}

fn js_text(e: JsValue) -> String {
    e.as_string().or_else(|| js_sys::Reflect::get(&e, &"message".into()).ok().and_then(|m| m.as_string())).unwrap_or_else(|| "Netzwerkfehler".into())
}

async fn fetch(method: &str, path: &str, body: Option<Body>, headers: &[(&str, String)]) -> Result<Resp, String> {
    let w = web_sys::window().ok_or("kein window")?;
    let init = web_sys::RequestInit::new();
    init.set_method(method);
    init.set_credentials(web_sys::RequestCredentials::SameOrigin);
    let h = web_sys::Headers::new().map_err(js_text)?;
    if method != "GET" {
        h.set("X-OwnDraw", "1").map_err(js_text)?;
    }
    match body {
        Some(Body::Json(v)) => {
            h.set("Content-Type", "application/json").map_err(js_text)?;
            init.set_body(&JsValue::from_str(&v.to_string()));
        }
        Some(Body::Bytes(b)) => {
            h.set("Content-Type", "application/octet-stream").map_err(js_text)?;
            init.set_body(&js_sys::Uint8Array::from(b.as_slice()));
        }
        None => {}
    }
    for (k, v) in headers {
        h.set(k, v).map_err(js_text)?;
    }
    init.set_headers(&h);
    let req = web_sys::Request::new_with_str_and_init(path, &init).map_err(js_text)?;
    let resp: web_sys::Response = JsFuture::from(w.fetch_with_request(&req)).await.map_err(js_text)?.dyn_into().map_err(js_text)?;
    let version = resp.headers().get("x-board-version").ok().flatten().and_then(|v| v.parse().ok());
    let buf = JsFuture::from(resp.array_buffer().map_err(js_text)?).await.map_err(js_text)?;
    Ok(Resp { status: resp.status(), version, body: js_sys::Uint8Array::new(&buf).to_vec() })
}

/// Anfrage im Hintergrund; `done` läuft im UI-Thread, danach wird neu gezeichnet.
pub fn request(method: &'static str, path: String, body: Option<Body>, headers: Vec<(&'static str, String)>, done: impl FnOnce(Result<Resp, String>) + 'static) {
    spawn_local(async move {
        let r = fetch(method, &path, body, &headers).await;
        done(r);
        repaint();
    });
}

fn notice(s: impl Into<String>) {
    with(|n| n.notices.push(s.into()));
}

/// Antwort einer Formular-Anfrage auswerten (Fehlertext oder Erfolg).
fn form_result(r: &Result<Resp, String>) -> bool {
    let ok = matches!(r, Ok(x) if x.ok());
    let err = match r {
        Ok(x) if x.ok() => None,
        Ok(x) => Some(x.message()),
        Err(e) => Some(format!("Server nicht erreichbar: {e}")),
    };
    with(|n| {
        n.busy = false;
        n.form_ok = ok;
        n.form_error = err;
    });
    ok
}

fn set_me(v: &Value) {
    let me = Session::parse(v);
    with(|n| {
        n.server = Server::Ready;
        n.me = me;
    });
    refresh_boards();
}

// ---------------------------------------------------------------- Konto

pub fn refresh_me() {
    request("GET", "/api/me".into(), None, vec![], |r| match r {
        Ok(r) if r.ok() && Session::parse(&r.json()).is_some() => set_me(&r.json()),
        Ok(r) if r.status == 401 && r.code() == "unauthorized" => with(|n| {
            n.server = Server::Ready;
            n.me = None;
            n.boards.clear();
            n.boards_loaded = false;
        }),
        // statisches Hosting (404 oder index.html) oder kein Netz: Gastmodus
        _ => with(|n| {
            if n.server == Server::Unknown {
                n.server = Server::Offline;
            }
        }),
    });
}

fn start_form() {
    with(|n| {
        n.busy = true;
        n.form_error = None;
        n.form_ok = false;
    });
}

pub fn login(email: &str, password: &str) {
    start_form();
    request("POST", "/api/auth/login".into(), Some(Body::Json(json!({"email": email.trim(), "password": password}))), vec![], |r| {
        if form_result(&r) {
            if let Ok(r) = r {
                set_me(&r.json());
            }
        }
    });
}

pub fn register(email: &str, password: &str, name: &str) {
    start_form();
    let body = json!({"email": email.trim(), "password": password, "name": name.trim()});
    request("POST", "/api/auth/register".into(), Some(Body::Json(body)), vec![], |r| {
        if form_result(&r) {
            if let Ok(r) = r {
                set_me(&r.json());
                notice("Konto angelegt – willkommen!");
            }
        }
    });
}

pub fn logout() {
    request("POST", "/api/auth/logout".into(), Some(Body::Json(json!({}))), vec![], |_| {
        with(|n| {
            n.me = None;
            n.boards.clear();
            n.boards_loaded = false;
        });
        notice("Abgemeldet");
    });
}

pub fn redeem(code: &str) {
    start_form();
    request("POST", "/api/license/redeem".into(), Some(Body::Json(json!({"code": code.trim()}))), vec![], |r| {
        if form_result(&r) {
            if let Ok(r) = r {
                set_me(&r.json());
                notice("Lizenz eingelöst – Boards werden jetzt am Server gespeichert");
            }
        }
    });
}

// ---------------------------------------------------------------- Boards

pub fn refresh_boards() {
    if !logged_in() {
        return;
    }
    request("GET", "/api/boards".into(), None, vec![], |r| {
        if let Some(r) = r.as_ref().ok().filter(|r| r.ok()) {
            let list: Vec<BoardInfo> = r.json().as_array().map(|a| a.iter().filter_map(BoardInfo::parse).collect()).unwrap_or_default();
            with(|n| {
                for b in &list {
                    // Version der Liste nur übernehmen, wenn das Board nicht gerade gespeichert wird
                    if !n.puts.get(&b.id).is_some_and(|p| p.inflight) {
                        n.versions.entry(b.id.clone()).or_insert(b.version);
                    }
                }
                n.boards = list;
                n.boards_loaded = true;
            });
        }
    });
}

pub fn board(id: &str) -> Option<BoardInfo> {
    with(|n| n.boards.iter().find(|b| b.id == id).cloned())
}

/// Lädt ein Board und öffnet es anschließend (über [`Net::open`]).
pub fn open_board(id: &str) {
    with(|n| n.opening = true);
    let id = id.to_string();
    request("GET", format!("/api/boards/{id}"), None, vec![], move |r| match r {
        Ok(r) if r.ok() => with(|n| {
            n.opening = false;
            let key = format!("{SRV}{id}");
            if let Some(v) = r.version {
                n.versions.insert(id.clone(), v);
            }
            n.conflicts.remove(&id);
            n.save_errors.remove(&id);
            n.files.insert(key.clone(), r.body);
            n.open.push(FileRef::Uri(key));
        }),
        other => {
            with(|n| n.opening = false);
            notice(match other {
                Ok(r) => r.message(),
                Err(e) => format!("Server nicht erreichbar: {e}"),
            });
        }
    });
}

/// Legt ein Board am Server an (rohe `.owndraw`-Bytes) und öffnet es.
pub fn create_board(name: &str, bytes: Vec<u8>) {
    with(|n| n.opening = true);
    let enc = String::from(js_sys::encode_uri_component(name));
    let copy = bytes.clone();
    request("POST", "/api/boards".into(), Some(Body::Bytes(bytes)), vec![("X-Board-Name", enc)], move |r| match r {
        Ok(r) if r.ok() => {
            let Some(b) = BoardInfo::parse(&r.json()) else { return };
            with(|n| {
                n.opening = false;
                let key = format!("{SRV}{}", b.id);
                n.versions.insert(b.id.clone(), b.version);
                n.files.insert(key.clone(), copy);
                n.open.push(FileRef::Uri(key));
                n.boards.insert(0, b);
            });
            refresh_me();
        }
        other => {
            with(|n| n.opening = false);
            notice(match other {
                Ok(r) => format!("Board konnte nicht am Server angelegt werden: {}", r.message()),
                Err(e) => format!("Server nicht erreichbar: {e}"),
            });
        }
    });
}

/// Autosave: `PUT` mit `If-Match`. Läuft schon ein Speichern, wird das neueste vorgemerkt.
pub fn put_board(id: &str, bytes: Vec<u8>) {
    let start = with(|n| {
        n.files.insert(format!("{SRV}{id}"), bytes.clone());
        let p = n.puts.entry(id.to_string()).or_default();
        if p.inflight {
            p.queued = Some(bytes.clone());
            false
        } else {
            p.inflight = true;
            true
        }
    });
    if start {
        send_put(id.to_string(), bytes);
    }
}

fn send_put(id: String, bytes: Vec<u8>) {
    let Some(v) = with(|n| n.versions.get(&id).copied()) else {
        with(|n| {
            n.puts.remove(&id);
            n.save_errors.insert(id.clone(), "Version des Boards unbekannt – bitte neu öffnen.".into());
        });
        return;
    };
    request("PUT", format!("/api/boards/{id}"), Some(Body::Bytes(bytes)), vec![("If-Match", v.to_string())], move |r| {
        let next = with(|n| {
            match &r {
                Ok(r) if r.ok() => {
                    if let Some(b) = BoardInfo::parse(&r.json()) {
                        n.versions.insert(id.clone(), b.version);
                        if let Some(e) = n.boards.iter_mut().find(|e| e.id == b.id) {
                            *e = b;
                        }
                    }
                    n.save_errors.remove(&id);
                }
                Ok(r) if r.status == 409 => {
                    if r.json().get("live").and_then(Value::as_bool) != Some(true) {
                        n.conflicts.insert(id.clone());
                    }
                    n.save_errors.insert(id.clone(), r.message());
                }
                Ok(r) => {
                    n.save_errors.insert(id.clone(), r.message());
                }
                Err(e) => {
                    n.save_errors.insert(id.clone(), format!("Server nicht erreichbar: {e}"));
                }
            }
            let p = n.puts.entry(id.clone()).or_default();
            let next = if n.conflicts.contains(&id) { None } else { p.queued.take() };
            p.inflight = next.is_some();
            if p.queued.is_none() && !p.inflight {
                n.puts.remove(&id);
            }
            next
        });
        if let Some(b) = next {
            send_put(id, b);
        }
    });
}

/// Speichern läuft oder steht an?
pub fn saving(id: &str) -> bool {
    with(|n| n.puts.contains_key(id))
}

pub fn rename_board(id: &str, name: &str) {
    let id2 = id.to_string();
    request("PATCH", format!("/api/boards/{id}"), Some(Body::Json(json!({"name": name}))), vec![], move |r| match r {
        Ok(r) if r.ok() => {
            if let Some(b) = BoardInfo::parse(&r.json()) {
                with(|n| {
                    if let Some(e) = n.boards.iter_mut().find(|e| e.id == id2) {
                        e.name = b.name;
                    }
                });
            }
        }
        Ok(r) => notice(r.message()),
        Err(e) => notice(format!("Server nicht erreichbar: {e}")),
    });
}

pub fn delete_board(id: &str) {
    let id2 = id.to_string();
    request("DELETE", format!("/api/boards/{id}"), None, vec![], move |r| match r {
        Ok(r) if r.ok() => {
            with(|n| {
                n.boards.retain(|b| b.id != id2);
                n.files.remove(&format!("{SRV}{id2}"));
            });
            notice("Board gelöscht");
            refresh_me();
        }
        Ok(r) => notice(r.message()),
        Err(e) => notice(format!("Server nicht erreichbar: {e}")),
    });
}

// ---------------------------------------------------------------- Mitglieder

fn set_members(id: &str, v: &Value) {
    let list = v
        .as_array()
        .map(|a| {
            a.iter()
                .map(|m| {
                    let s = |k: &str| m.get(k).and_then(Value::as_str).unwrap_or("").to_string();
                    Member { user_id: s("user_id"), email: s("email"), name: s("name"), role: s("role") }
                })
                .collect()
        })
        .unwrap_or_default();
    with(|n| n.members = Some((id.to_string(), list)));
}

pub fn load_members(id: &str) {
    with(|n| n.members = None);
    let id2 = id.to_string();
    request("GET", format!("/api/boards/{id}/members"), None, vec![], move |r| match r {
        Ok(r) if r.ok() => set_members(&id2, &r.json()),
        Ok(r) => notice(r.message()),
        Err(e) => notice(format!("Server nicht erreichbar: {e}")),
    });
}

pub fn add_member(id: &str, email: &str, role: &str) {
    start_form();
    let id2 = id.to_string();
    request("POST", format!("/api/boards/{id}/members"), Some(Body::Json(json!({"email": email.trim(), "role": role}))), vec![], move |r| {
        if form_result(&r) {
            if let Ok(r) = r {
                set_members(&id2, &r.json());
            }
        }
    });
}

pub fn remove_member(id: &str, user_id: &str) {
    let id2 = id.to_string();
    request("DELETE", format!("/api/boards/{id}/members/{user_id}"), None, vec![], move |r| match r {
        Ok(r) if r.ok() => set_members(&id2, &r.json()),
        Ok(r) => notice(r.message()),
        Err(e) => notice(format!("Server nicht erreichbar: {e}")),
    });
}

// ---------------------------------------------------------------- Bilder

pub fn upload_image(board: &str, name: &str, bytes: Vec<u8>) {
    request("PUT", format!("/api/boards/{board}/images/{name}"), Some(Body::Bytes(bytes)), vec![], |r| {
        if let Some(r) = r.as_ref().ok().filter(|r| !r.ok()) {
            notice(format!("Bild konnte nicht hochgeladen werden: {}", r.message()));
        }
    });
}

/// Lädt ein Bild (`done(None)` bei Fehler, z. B. noch nicht hochgeladen).
pub fn fetch_image(board: &str, name: &str, done: impl FnOnce(Option<Vec<u8>>) + 'static) {
    request("GET", format!("/api/boards/{board}/images/{name}"), None, vec![], move |r| done(r.ok().filter(|r| r.ok()).map(|r| r.body)));
}

// ---------------------------------------------------------------- WebSocket

pub enum SockEv {
    Open,
    Msg(Value),
    Closed { code: u16, reason: String },
}

/// WebSocket zu `/api/collab/{id}` (Cookie geht automatisch mit).
pub struct Socket {
    ws: web_sys::WebSocket,
    inbox: Rc<RefCell<Vec<SockEv>>>,
    _cbs: Vec<Closure<dyn FnMut(JsValue)>>,
}

impl Socket {
    pub fn connect(board: &str) -> Result<Socket, String> {
        let loc = web_sys::window().ok_or("kein window")?.location();
        let proto = if loc.protocol().map_err(js_text)? == "https:" { "wss" } else { "ws" };
        let url = format!("{proto}://{}/api/collab/{board}", loc.host().map_err(js_text)?);
        let ws = web_sys::WebSocket::new(&url).map_err(js_text)?;
        let inbox: Rc<RefCell<Vec<SockEv>>> = Rc::default();
        let mut cbs: Vec<Closure<dyn FnMut(JsValue)>> = Vec::new();
        let i = inbox.clone();
        cbs.push(Closure::new(move |_e: JsValue| {
            i.borrow_mut().push(SockEv::Open);
            repaint();
        }));
        ws.set_onopen(Some(cbs[0].as_ref().unchecked_ref()));
        let i = inbox.clone();
        cbs.push(Closure::new(move |e: JsValue| {
            let Ok(e) = e.dyn_into::<web_sys::MessageEvent>() else { return };
            if let Some(t) = e.data().as_string() {
                if let Ok(v) = serde_json::from_str(&t) {
                    i.borrow_mut().push(SockEv::Msg(v));
                    repaint();
                }
            }
        }));
        ws.set_onmessage(Some(cbs[1].as_ref().unchecked_ref()));
        let i = inbox.clone();
        cbs.push(Closure::new(move |e: JsValue| {
            let (code, reason) = e.dyn_into::<web_sys::CloseEvent>().map(|c| (c.code(), c.reason())).unwrap_or((1006, String::new()));
            i.borrow_mut().push(SockEv::Closed { code, reason });
            repaint();
        }));
        ws.set_onclose(Some(cbs[2].as_ref().unchecked_ref()));
        Ok(Socket { ws, inbox, _cbs: cbs })
    }

    pub fn send(&self, v: &Value) -> bool {
        self.ws.ready_state() == web_sys::WebSocket::OPEN && self.ws.send_with_str(&v.to_string()).is_ok()
    }

    pub fn take(&self) -> Vec<SockEv> {
        std::mem::take(&mut self.inbox.borrow_mut())
    }
}

impl Drop for Socket {
    fn drop(&mut self) {
        self.ws.set_onopen(None);
        self.ws.set_onmessage(None);
        self.ws.set_onclose(None);
        let _ = self.ws.close();
    }
}
