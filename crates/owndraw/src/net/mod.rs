//! Server-Anbindung (Vertrag: `docs/web-api.md`) für Web-App, Desktop und Android. Transport je Plattform: `web.rs`
//! (fetch/WebSocket des Browsers, gleiche Origin, Cookie vom Browser) bzw. `native.rs` (Desktop und Android: ureq/tungstenite
//! in Hintergrund-Threads, Server-Adresse einstellbar, Sitzungs-Cookie in einer Datei mit Rechten 0600).
//! Schreibende Anfragen mit `X-OwnDraw: 1`.
//!
//! - Beim Start `GET /api/me`: 200 → angemeldet, 401 → Server da, aber Gast; alles andere (statisches Hosting
//!   ohne `/api`, Netzfehler) → [`Server::Offline`], die App bleibt im Gastmodus.
//! - Boards am Server sind `FileRef::Uri("srv:<id>")`: Öffnen lädt die Bytes vorab (`open_board`) in
//!   [`Net::files`], danach öffnet die App sie wie jede Datei. `write_atomic` darauf speichert per PUT mit
//!   `If-Match` (Autosave), außer das Board ist gerade live verbunden (dann speichert der Server).
//! - Antworten kommen asynchron; sie landen in [`Net`] (thread-lokal) und lösen ein Neuzeichnen aus.
//! - [`Socket`]: WebSocket für die Live-Zusammenarbeit (`/api/collab/{id}`), siehe `ui/editor/live.rs`.

// Die Antwort-Typen bilden das Server-API vollständig ab; nicht jede Plattform nutzt jedes Feld.
#![allow(dead_code)]

use crate::platform::FileRef;
use serde_json::{json, Value};
use std::cell::RefCell;
use std::collections::{HashMap, HashSet};

#[cfg(target_arch = "wasm32")]
#[path = "web.rs"]
mod tr;
#[cfg(not(target_arch = "wasm32"))]
#[path = "native.rs"]
mod tr;
#[cfg(not(target_arch = "wasm32"))]
pub use tr::{base_url, set_base_url, set_config_dir};
pub use tr::pump;

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
    /// Gast per Einladungslink (Name ist die Gast-ID, keine E-Mail, keine Lizenz).
    pub guest: bool,
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
        Some(Session { id: v.get("id")?.as_str()?.to_string(), email: s("email"), name: s("name"), admin: v.get("admin").and_then(Value::as_bool).unwrap_or(false), guest: v.get("guest").and_then(Value::as_bool).unwrap_or(false), license })
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
    pub guest: bool,
}

/// Vorschau einer Einladung (`GET /api/invites/{token}`).
#[derive(Clone, Debug)]
pub struct InviteInfo {
    pub board_id: String,
    pub board_name: String,
    pub owner_name: String,
    /// `edit` oder `view`.
    pub role: String,
}

/// Vorgemerkte Einladung aus dem Link (`#invite=…`).
#[derive(Clone, Debug)]
pub struct PendingInvite {
    pub token: String,
    pub info: Option<InviteInfo>,
    pub error: Option<String>,
}

/// Einladungslink eines Boards (ohne Token; der steht nur direkt nach dem Anlegen in [`Net::new_link`]).
#[derive(Clone, Debug)]
pub struct Invite {
    pub id: String,
    pub role: String,
    pub created: i64,
    pub expires_at: Option<i64>,
    pub uses: i64,
}

impl Invite {
    fn parse(v: &Value) -> Option<Invite> {
        Some(Invite {
            id: v.get("id")?.as_str()?.to_string(),
            role: v.get("role").and_then(Value::as_str).unwrap_or("view").to_string(),
            created: v.get("created").and_then(Value::as_i64).unwrap_or(0),
            expires_at: v.get("expires_at").and_then(Value::as_i64),
            uses: v.get("uses").and_then(Value::as_i64).unwrap_or(0),
        })
    }
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
    /// Einladung aus dem Link, wartet auf Beitritt.
    pub invite: Option<PendingInvite>,
    /// Einladungslinks des Teilen-Dialogs (Board-ID, Liste).
    pub invites: Option<(String, Vec<Invite>)>,
    /// Frisch erstellter Link (Board-ID, vollständige URL).
    pub new_link: Option<(String, String)>,
    /// Ein Board wird gerade zwischen Browser und Server verschoben.
    pub moving: bool,
    /// Erfolgreich auf den Server verschobene Browser-Boards (die App entfernt sie aus „Zuletzt“).
    pub moved: Vec<FileRef>,
    /// Vom Server geladene Boards, die die App im Browser speichern soll.
    pub to_local: Vec<ToLocal>,
    /// Konto wurde gesperrt: Nachricht des Admins (leer = keine). Die App zeigt die Meldung, die Sitzung ist beendet.
    pub blocked: Option<String>,
    puts: HashMap<String, Put>,
}

/// Vom Server geladenes Board für den Browser-Speicher („In diesem Browser speichern“).
pub struct ToLocal {
    pub name: String,
    pub bytes: Vec<u8>,
    /// Board-ID am Server, die nach dem Speichern im Browser gelöscht wird (Verschieben); None = nur Kopie.
    pub delete_id: Option<String>,
}

thread_local! {
    static STARTED: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
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
        invite: None,
        invites: None,
        new_link: None,
        moving: false,
        moved: Vec::new(),
        to_local: Vec::new(),
        blocked: None,
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

/// Beim Start: Kontext merken, Anmeldung prüfen. Desktop: vorher [`set_config_dir`] und [`set_base_url`].
pub fn init(ctx: &egui::Context) {
    CTX.with(|c| *c.borrow_mut() = Some(ctx.clone()));
    STARTED.with(|s| s.set(true));
    if let Some(token) = tr::init(ctx) {
        invite_info(&token);
    }
    refresh_me();
}

/// Server-Anbindung gestartet? (Am Desktop nicht bei Screenshot-Demos ohne `OWNDRAW_SERVER`.)
pub fn started() -> bool {
    STARTED.with(|s| s.get())
}

/// Desktop: anderen Server verwenden (meldet ab, lädt den Status neu).
#[cfg(not(target_arch = "wasm32"))]
pub fn change_server(url: &str) {
    set_base_url(url);
    with(|n| {
        n.server = Server::Unknown;
        n.me = None;
        n.boards.clear();
        n.boards_loaded = false;
        n.invite = None;
        n.form_error = None;
    });
    if started() {
        refresh_me();
    }
}

/// Server erneut versuchen (nach „nicht erreichbar“).
pub fn retry() {
    with(|n| n.server = Server::Unknown);
    refresh_me();
}

pub fn online() -> bool {
    with(|n| n.server == Server::Ready)
}

pub fn licensed() -> bool {
    with(|n| n.me.as_ref().is_some_and(Session::licensed))
}

/// Angemeldetes Konto ist Admin (`admin` aus `GET /api/me`, kommt vom Server; Gäste nie).
pub fn is_admin() -> bool {
    with(|n| n.me.as_ref().is_some_and(|m| m.admin && !m.guest))
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

/// Anfrage im Hintergrund; `done` läuft im UI-Thread, danach wird neu gezeichnet.
pub fn request(method: &'static str, path: String, body: Option<Body>, headers: Vec<(&'static str, String)>, done: impl FnOnce(Result<Resp, String>) + 'static) {
    tr::request(
        method,
        path,
        body,
        headers,
        Box::new(move |r: Result<Resp, String>| {
            if let Ok(x) = &r {
                if x.code() == "account_disabled" {
                    account_blocked(x.json().get("admin_message").and_then(Value::as_str).unwrap_or(""));
                }
            }
            done(r)
        }),
    );
}

/// Server meldet „Konto gesperrt“: Meldung vormerken und lokal abmelden (der Server hat die Sitzung schon beendet).
fn account_blocked(admin_message: &str) {
    #[cfg(not(target_arch = "wasm32"))]
    tr::forget_session();
    with(|n| {
        n.blocked = Some(admin_message.trim().to_string());
        n.server = Server::Ready;
        n.me = None;
        n.boards.clear();
        n.boards_loaded = false;
        n.live.clear();
    });
}

/// Live-Verbindung wegen Sperre getrennt: Sperrmeldung samt Nachricht beim Server abholen.
pub fn check_blocked() {
    refresh_me();
}

fn notice(s: impl Into<String>) {
    with(|n| n.notices.push(s.into()));
}

/// Antwort einer Formular-Anfrage auswerten (Fehlertext oder Erfolg).
fn form_result(r: &Result<Resp, String>) -> bool {
    let ok = matches!(r, Ok(x) if x.ok());
    let err = match r {
        Ok(x) if x.ok() => None,
        // eigene Meldung (siehe `account_blocked`), nicht zusätzlich im Formular
        Ok(x) if x.code() == "account_disabled" => None,
        Ok(x) => Some(x.message()),
        Err(e) => Some(format!("Server nicht erreichbar. Prüfe deine Verbindung: {e}")),
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
    refresh_features();
}

/// Vom Admin gesperrte Werkzeuge/Funktionen laden (`GET /api/features`, für Admins leer) → `crate::locks`.
/// Ältere Server ohne diese Anfrage: nichts gesperrt.
pub fn refresh_features() {
    request("GET", "/api/features".into(), None, vec![], |r| {
        let off = match r {
            Ok(r) if r.ok() => r.json()["off"].as_array().map(|a| a.iter().filter_map(|x| x.as_str().map(String::from)).collect()).unwrap_or_default(),
            Ok(r) if r.status == 404 => Vec::new(),
            _ => return,
        };
        crate::locks::set(off);
    });
}

// ---------------------------------------------------------------- Konto

pub fn refresh_me() {
    request("GET", "/api/me".into(), None, vec![], |r| match r {
        Ok(r) if r.ok() && Session::parse(&r.json()).is_some() => set_me(&r.json()),
        Ok(r) if r.status == 401 && r.code() == "unauthorized" => {
            with(|n| {
                n.server = Server::Ready;
                n.me = None;
                n.boards.clear();
                n.boards_loaded = false;
            });
            refresh_features();
        }
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
                notice("Konto erstellt. Willkommen!");
            }
        }
    });
}

pub fn logout() {
    request("POST", "/api/auth/logout".into(), Some(Body::Json(json!({}))), vec![], |_| {
        #[cfg(not(target_arch = "wasm32"))]
        tr::forget_session();
        with(|n| {
            n.me = None;
            n.boards.clear();
            n.boards_loaded = false;
        });
        refresh_features();
        notice("Abgemeldet");
    });
}

pub fn redeem(code: &str) {
    start_form();
    request("POST", "/api/license/redeem".into(), Some(Body::Json(json!({"code": code.trim()}))), vec![], |r| {
        if form_result(&r) {
            if let Ok(r) = r {
                set_me(&r.json());
                notice("Lizenz eingelöst. Neue Boards werden jetzt auf dem Server gespeichert.");
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
                Err(e) => format!("Server nicht erreichbar. Prüfe deine Verbindung: {e}"),
            });
        }
    });
}

/// Legt ein Board am Server an (rohe `.owndraw`-Bytes) und öffnet es.
pub fn create_board(name: &str, bytes: Vec<u8>) {
    with(|n| n.opening = true);
    upload(name, bytes, None);
}

/// Freier Speicherplatz der Lizenz in Bytes (None ohne gültige Lizenz).
pub fn free_bytes() -> Option<i64> {
    with(|n| n.me.as_ref().filter(|m| m.licensed()).and_then(|m| m.license.as_ref()).map(|l| ((l.quota_mb - l.used_mb) * 1_048_576.0).max(0.0) as i64))
}

/// Verschiebt ein Browser-Board auf den Server: hochladen; erst wenn der Server es angenommen hat, kommt
/// `local` in [`Net::moved`] (die App nimmt es dann aus „Zuletzt“). Bei Fehlern bleibt das Board im Browser.
pub fn move_to_server(name: &str, bytes: Vec<u8>, local: FileRef) {
    with(|n| n.moving = true);
    upload(name, bytes, Some(local));
}

/// `POST /api/boards`; `local` = None → danach öffnen, sonst Verschieben aus dem Browser.
fn upload(name: &str, bytes: Vec<u8>, local: Option<FileRef>) {
    let enc = tr::encode(name);
    let copy = bytes.clone();
    let moving = local.is_some();
    request("POST", "/api/boards".into(), Some(Body::Bytes(bytes)), vec![("X-Board-Name", enc)], move |r| match r {
        Ok(r) if r.ok() => {
            let Some(b) = BoardInfo::parse(&r.json()) else {
                with(|n| {
                    n.opening = false;
                    n.moving = false;
                });
                return;
            };
            with(|n| {
                n.versions.insert(b.id.clone(), b.version);
                match local {
                    Some(f) => {
                        n.moving = false;
                        n.moved.push(f);
                        n.notices.push(format!("„{}“ liegt jetzt auf dem Server.", b.name));
                    }
                    None => {
                        n.opening = false;
                        let key = format!("{SRV}{}", b.id);
                        n.files.insert(key.clone(), copy);
                        n.open.push(FileRef::Uri(key));
                    }
                }
                n.boards.insert(0, b);
            });
            refresh_me();
        }
        other => {
            with(|n| {
                n.opening = false;
                n.moving = false;
            });
            let what = if moving { if cfg!(target_arch = "wasm32") { "Board konnte nicht auf den Server verschoben werden. Es bleibt in diesem Browser" } else { "Board konnte nicht auf den Server verschoben werden. Es bleibt auf diesem Gerät" } } else { "Board konnte nicht auf dem Server erstellt werden" };
            notice(match other {
                Ok(r) if r.status == 507 => format!("{what}: {} Lösche Boards auf dem Server oder speichere dieses Board {}.", r.message(), crate::ui::local_in()),
                Ok(r) => format!("{what}: {}", r.message()),
                Err(e) => format!("Server nicht erreichbar. Prüfe deine Verbindung: {e}"),
            });
        }
    });
}

/// Lädt ein Server-Board und übergibt es der App zum Speichern im Browser ([`Net::to_local`]).
/// `delete` = danach am Server löschen (nur Besitzer; die App löscht erst, wenn das Speichern geklappt hat).
pub fn fetch_to_local(id: &str, name: &str, delete: bool) {
    with(|n| n.moving = true);
    let (id, name) = (id.to_string(), name.to_string());
    request("GET", format!("/api/boards/{id}"), None, vec![], move |r| match r {
        Ok(r) if r.ok() => with(|n| {
            n.moving = false;
            n.to_local.push(ToLocal { name, bytes: r.body, delete_id: delete.then_some(id) });
        }),
        other => {
            with(|n| n.moving = false);
            notice(match other {
                Ok(r) => format!("Board konnte nicht geladen werden: {}", r.message()),
                Err(e) => format!("Server nicht erreichbar. Prüfe deine Verbindung: {e}"),
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
            n.save_errors.insert(id.clone(), "Die Version des Boards ist unbekannt. Öffne das Board neu.".into());
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
                    n.save_errors.insert(id.clone(), format!("Server nicht erreichbar. Prüfe deine Verbindung: {e}"));
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
        Err(e) => notice(format!("Server nicht erreichbar. Prüfe deine Verbindung: {e}")),
    });
}

pub fn delete_board(id: &str) {
    delete_with(id, None);
}

/// Löscht ein Board, das gerade in den Browser verschoben wurde (`name` für die Meldungen).
pub fn delete_moved(id: &str, name: &str) {
    delete_with(id, Some(name.to_string()));
}

fn delete_with(id: &str, moved: Option<String>) {
    let id2 = id.to_string();
    request("DELETE", format!("/api/boards/{id}"), None, vec![], move |r| {
        let fail = |m: String| match &moved {
            Some(name) => notice(format!("„{name}“ ist jetzt {} gespeichert, konnte am Server aber nicht gelöscht werden: {m}", crate::ui::local_in())),
            None => notice(m),
        };
        match r {
            Ok(r) if r.ok() => {
                with(|n| {
                    n.boards.retain(|b| b.id != id2);
                    n.files.remove(&format!("{SRV}{id2}"));
                });
                notice(match &moved {
                    Some(name) => format!("„{name}“ liegt jetzt {}.", crate::ui::local_in()),
                    None => "Board gelöscht".into(),
                });
                refresh_me();
            }
            Ok(r) => fail(r.message()),
            Err(e) => fail(format!("Server nicht erreichbar. Prüfe deine Verbindung: {e}")),
        }
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
                    Member { user_id: s("user_id"), email: s("email"), name: s("name"), role: s("role"), guest: m.get("guest").and_then(Value::as_bool).unwrap_or(false) }
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
        Err(e) => notice(format!("Server nicht erreichbar. Prüfe deine Verbindung: {e}")),
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
        Err(e) => notice(format!("Server nicht erreichbar. Prüfe deine Verbindung: {e}")),
    });
}

// ---------------------------------------------------------------- Einladungslinks

/// Lädt die Vorschau einer Einladung (ohne Anmeldung) und merkt sie vor.
pub fn invite_info(token: &str) {
    with(|n| n.invite = Some(PendingInvite { token: token.to_string(), info: None, error: None }));
    let tok = token.to_string();
    let enc = tr::encode(token);
    request("GET", format!("/api/invites/{enc}"), None, vec![], move |r| {
        with(|n| {
            let Some(p) = n.invite.as_mut().filter(|p| p.token == tok) else { return };
            match r {
                Ok(r) if r.ok() => {
                    let v = r.json();
                    let s = |k: &str| v.pointer(k).and_then(Value::as_str).unwrap_or("").to_string();
                    p.info = Some(InviteInfo { board_id: s("/board/id"), board_name: s("/board/name"), owner_name: s("/owner/name"), role: s("/role") });
                }
                Ok(r) if r.status == 404 => p.error = Some("Diese Einladung ist ungültig, abgelaufen oder wurde widerrufen.".into()),
                Ok(r) => p.error = Some(r.message()),
                Err(e) => p.error = Some(format!("Server nicht erreichbar. Prüfe deine Verbindung: {e}")),
            }
        });
    });
}

/// Tritt der Einladung bei. Ohne Anmeldung legt der Server einen Gast mit `name` an.
pub fn join_invite(token: &str, name: &str) {
    start_form();
    let enc = tr::encode(token);
    let body = if logged_in() { json!({}) } else { json!({"name": name.trim()}) };
    request("POST", format!("/api/invites/{enc}/join"), Some(Body::Json(body)), vec![], |r| {
        if form_result(&r) {
            if let Ok(r) = r {
                let v = r.json();
                let Some(b) = v.get("board").and_then(BoardInfo::parse) else { return };
                if let Some(me) = v.get("me") {
                    set_me(me);
                }
                let id = b.id.clone();
                with(|n| {
                    n.invite = None;
                    n.form_ok = false;
                    n.versions.insert(id.clone(), b.version);
                    match n.boards.iter_mut().find(|e| e.id == id) {
                        Some(e) => *e = b,
                        None => n.boards.insert(0, b),
                    }
                });
                open_board(&id);
                notice("Du bist dem Board beigetreten");
            }
        }
    });
}

fn set_invites(id: &str, v: &Value) {
    let list = v.as_array().map(|a| a.iter().filter_map(Invite::parse).collect()).unwrap_or_default();
    with(|n| n.invites = Some((id.to_string(), list)));
}

pub fn load_invites(board: &str) {
    with(|n| {
        n.invites = None;
        n.new_link = None;
    });
    let id = board.to_string();
    request("GET", format!("/api/boards/{board}/invites"), None, vec![], move |r| match r {
        Ok(r) if r.ok() => set_invites(&id, &r.json()),
        Ok(r) => notice(r.message()),
        Err(e) => notice(format!("Server nicht erreichbar. Prüfe deine Verbindung: {e}")),
    });
}

/// Neuer Einladungslink; `days` `None` = unbefristet. Das Ergebnis steht in [`Net::new_link`].
/// Laufzeit eines neuen Einladungslinks.
#[derive(Clone, Copy)]
pub enum LinkTime {
    Hours(u32),
    Days(u32),
    Forever,
}

pub fn create_invite(board: &str, role: &str, time: LinkTime) {
    let id = board.to_string();
    let body = match time {
        LinkTime::Hours(h) => json!({"role": role, "hours": h}),
        LinkTime::Days(d) => json!({"role": role, "days": d}),
        LinkTime::Forever => json!({"role": role, "days": null}),
    };
    request("POST", format!("/api/boards/{board}/invites"), Some(Body::Json(body)), vec![], move |r| match r {
        Ok(r) if r.ok() => {
            let v = r.json();
            let Some(token) = v.get("token").and_then(Value::as_str) else { return };
            let origin = tr::origin();
            let url = format!("{origin}/#invite={token}");
            let inv = Invite::parse(&v);
            with(|n| {
                n.new_link = Some((id.clone(), url));
                if let (Some(inv), Some((b, l))) = (inv, n.invites.as_mut()) {
                    if *b == id {
                        l.push(inv);
                    }
                }
            });
        }
        Ok(r) => notice(r.message()),
        Err(e) => notice(format!("Server nicht erreichbar. Prüfe deine Verbindung: {e}")),
    });
}

pub fn delete_invite(board: &str, invite_id: &str) {
    let id = board.to_string();
    request("DELETE", format!("/api/boards/{board}/invites/{invite_id}"), None, vec![], move |r| match r {
        Ok(r) if r.ok() => {
            set_invites(&id, &r.json());
            with(|n| n.new_link = None);
        }
        Ok(r) => notice(r.message()),
        Err(e) => notice(format!("Server nicht erreichbar. Prüfe deine Verbindung: {e}")),
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

pub use tr::Socket;
