//! Transport am Desktop: HTTP über `ureq` (rustls, System-Zertifikate) und WebSocket über `tungstenite`, beides
//! in Hintergrund-Threads. Die Ergebnisse kommen per Kanal in den UI-Thread und werden dort in [`pump`] ausgeliefert
//! (die `done`-Callbacks sind nicht `Send` und bleiben deshalb im UI-Thread).
//!
//! Das Sitzungs-Token (Cookie `owndraw_session`) liegt in `konto.json` im Konfigurationsordner (Unix: Rechte 0600).
//! Es wird nie ausgegeben, geloggt oder in Fehlermeldungen übernommen.

use super::{repaint, Body, Resp, SockEv};
use serde_json::Value;
use std::cell::RefCell;
use std::collections::HashMap;
use std::io::{Read, Write};
use std::net::{TcpStream, ToSocketAddrs};
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{channel, Receiver, Sender, TryRecvError};
use std::sync::{Arc, Mutex, OnceLock};
use std::time::{Duration, Instant};

const DEFAULT_URL: &str = "https://owndraw.thepepelex.at";
const COOKIE: &str = "owndraw_session";
const FILE: &str = "konto.json";
const CONNECT_TIMEOUT: Duration = Duration::from_secs(15);

// ---------------------------------------------------------------- Zustand

/// Kontext für Hintergrund-Threads (`repaint()` aus `mod.rs` ist thread-lokal und nur im UI-Thread nutzbar).
static CTX: Mutex<Option<egui::Context>> = Mutex::new(None);

struct State {
    dir: Option<PathBuf>,
    server: String,
    /// Token samt dem Server, für den es ausgestellt wurde.
    token: Option<String>,
    token_server: String,
    loaded: bool,
}

static STATE: Mutex<State> = Mutex::new(State { dir: None, server: String::new(), token: None, token_server: String::new(), loaded: false });

fn state() -> std::sync::MutexGuard<'static, State> {
    STATE.lock().unwrap_or_else(|e| e.into_inner())
}

fn wake() {
    if let Some(c) = CTX.lock().unwrap_or_else(|e| e.into_inner()).as_ref() {
        c.request_repaint();
    }
}

fn normalize(url: &str) -> String {
    let mut u = url.trim().trim_end_matches('/').trim().to_string();
    if u.is_empty() {
        return DEFAULT_URL.to_string();
    }
    if !u.contains("://") {
        u = format!("https://{u}");
    }
    u
}

impl State {
    fn server(&self) -> &str {
        if self.server.is_empty() {
            DEFAULT_URL
        } else {
            &self.server
        }
    }
    fn path(&self) -> PathBuf {
        let dir = self.dir.clone().or_else(|| dirs::config_dir().map(|d| d.join("owndraw"))).unwrap_or_else(|| PathBuf::from("."));
        dir.join(FILE)
    }
    fn ensure_loaded(&mut self) {
        if self.loaded {
            return;
        }
        self.loaded = true;
        if let Ok(text) = std::fs::read_to_string(self.path()) {
            if let Ok(v) = serde_json::from_str::<Value>(&text) {
                let tok = v.get("token").and_then(Value::as_str).filter(|t| !t.is_empty());
                let srv = v.get("server").and_then(Value::as_str).unwrap_or("");
                if let Some(t) = tok {
                    self.token = Some(t.to_string());
                    self.token_server = normalize(srv);
                }
            }
        }
    }
    /// Token, falls es zum aktuellen Server gehört.
    fn active_token(&mut self) -> Option<String> {
        self.ensure_loaded();
        let t = self.token.clone()?;
        (self.token_server == self.server()).then_some(t)
    }
    fn set_token(&mut self, token: Option<String>) {
        self.ensure_loaded();
        self.token = token;
        self.token_server = self.server().to_string();
        self.persist();
    }
    fn persist(&self) {
        let path = self.path();
        match &self.token {
            Some(t) => {
                let body = serde_json::json!({ "server": self.token_server, "token": t }).to_string();
                if let Some(d) = path.parent() {
                    let _ = std::fs::create_dir_all(d);
                }
                if let Err(e) = write_private(&path, body.as_bytes()) {
                    eprintln!("OwnDraw: Konto-Datei konnte nicht gespeichert werden: {}", e.kind());
                }
            }
            None => {
                let _ = std::fs::remove_file(&path);
            }
        }
    }
}

/// Atomar schreiben (temp + rename), unter Unix mit Rechten 0600 schon beim Anlegen.
fn write_private(path: &std::path::Path, data: &[u8]) -> std::io::Result<()> {
    let tmp = path.with_extension("json.tmp");
    let _ = std::fs::remove_file(&tmp);
    let mut o = std::fs::OpenOptions::new();
    o.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        o.mode(0o600);
    }
    let r = (|| {
        let mut f = o.open(&tmp)?;
        f.write_all(data)?;
        f.sync_all()?;
        std::fs::rename(&tmp, path)
    })();
    if r.is_err() {
        let _ = std::fs::remove_file(&tmp);
    }
    r
}

// ---------------------------------------------------------------- Öffentliche Einstellungen

/// Ordner für `konto.json` (vor [`init`] aufrufen).
pub fn set_config_dir(dir: PathBuf) {
    let mut s = state();
    s.dir = Some(dir);
    s.loaded = false;
    s.token = None;
}

/// Kontext merken, Sitzungs-Token laden. Am Desktop gibt es keine Einladung aus der Adresse.
pub fn init(ctx: &egui::Context) -> Option<String> {
    *CTX.lock().unwrap_or_else(|e| e.into_inner()) = Some(ctx.clone());
    state().ensure_loaded();
    None
}

pub fn base_url() -> String {
    state().server().to_string()
}

/// Server-Adresse ändern. Gehört das gespeicherte Token zu einem anderen Server, wird es verworfen.
pub fn set_base_url(url: &str) {
    let url = normalize(url);
    let mut s = state();
    s.ensure_loaded();
    s.server = url;
    if s.token.is_some() && s.token_server != s.server {
        s.token = None;
        s.persist();
    }
}

/// Adresse des Servers für Einladungslinks.
pub fn origin() -> String {
    base_url()
}

/// Sitzung lokal vergessen (Abmelden, auch wenn der Server nicht antwortet).
pub fn forget_session() {
    state().set_token(None);
}

/// Wie JS `encodeURIComponent`.
pub fn encode(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for b in s.bytes() {
        if b.is_ascii_alphanumeric() || b"-_.!~*'()".contains(&b) {
            out.push(b as char);
        } else {
            out.push_str(&format!("%{b:02X}"));
        }
    }
    out
}

// ---------------------------------------------------------------- HTTP

type Done = Box<dyn FnOnce(Result<Resp, String>)>;
type Answer = (u64, Result<Resp, String>);

struct Pending {
    tx: Sender<Answer>,
    rx: Receiver<Answer>,
    next: u64,
    callbacks: HashMap<u64, Done>,
}

thread_local! {
    static PENDING: RefCell<Pending> = RefCell::new({
        let (tx, rx) = channel();
        Pending { tx, rx, next: 0, callbacks: HashMap::new() }
    });
}

fn agent() -> &'static ureq::Agent {
    static AGENT: OnceLock<ureq::Agent> = OnceLock::new();
    AGENT.get_or_init(|| {
        ureq::Agent::config_builder()
            .http_status_as_error(false)
            .https_only(false)
            // Keine Weiterleitungen: das Sitzungs-Cookie darf nie an einen anderen Host gehen.
            .max_redirects(0)
            .timeout_connect(Some(CONNECT_TIMEOUT))
            .timeout_recv_response(Some(Duration::from_secs(30)))
            .timeout_recv_body(Some(Duration::from_secs(30)))
            .user_agent(concat!("OwnDraw-Desktop/", env!("CARGO_PKG_VERSION")))
            .tls_config(ureq::tls::TlsConfig::builder().root_certs(ureq::tls::RootCerts::PlatformVerifier).build())
            .build()
            .into()
    })
}

fn net_error(e: ureq::Error) -> String {
    use ureq::Error as E;
    match e {
        E::Timeout(_) => "Zeitüberschreitung: Der Server antwortet nicht".into(),
        E::HostNotFound | E::ConnectionFailed => "Server nicht erreichbar".into(),
        E::BodyExceedsLimit(_) => "Die Antwort des Servers ist zu groß".into(),
        E::BadUri(_) => "Ungültige Server-Adresse".into(),
        e => {
            let m = e.to_string();
            let l = m.to_lowercase();
            if l.contains("certificate") || l.contains("tls") || l.contains("rustls") {
                "Sichere Verbindung zum Server fehlgeschlagen (Zertifikat nicht vertrauenswürdig?)".into()
            } else if l.contains("refused") || l.contains("unreachable") || l.contains("resolve") || l.contains("lookup") || l.contains("connect") {
                "Server nicht erreichbar".into()
            } else if l.contains("timed out") || l.contains("timeout") {
                "Zeitüberschreitung: Der Server antwortet nicht".into()
            } else {
                format!("Netzwerkfehler: {m}")
            }
        }
    }
}

/// Wertet `Set-Cookie` aus: leerer Wert oder `Max-Age=0` löscht das Token, sonst wird es gespeichert.
fn apply_set_cookie(value: &str) {
    let mut parts = value.split(';');
    let Some((name, val)) = parts.next().and_then(|p| p.split_once('=')) else { return };
    if name.trim() != COOKIE {
        return;
    }
    let val = val.trim();
    let expired = parts.any(|a| a.trim().split_once('=').is_some_and(|(k, v)| k.trim().eq_ignore_ascii_case("max-age") && v.trim().parse::<i64>().is_ok_and(|n| n <= 0)));
    let mut s = state();
    if val.is_empty() || expired {
        s.set_token(None);
    } else {
        s.set_token(Some(val.to_string()));
    }
}

fn perform(method: &'static str, path: &str, body: Option<Body>, headers: &[(&'static str, String)]) -> Result<Resp, String> {
    let (base, token) = {
        let mut s = state();
        (s.server().to_string(), s.active_token())
    };
    let mut rb = ureq::http::Request::builder().method(method).uri(format!("{base}{path}"));
    if method != "GET" {
        rb = rb.header("X-OwnDraw", "1");
    }
    if let Some(t) = &token {
        rb = rb.header("Cookie", format!("{COOKIE}={t}"));
    }
    match &body {
        Some(Body::Json(_)) => rb = rb.header("Content-Type", "application/json"),
        Some(Body::Bytes(_)) => rb = rb.header("Content-Type", "application/octet-stream"),
        None => {}
    }
    for (k, v) in headers {
        rb = rb.header(*k, v.as_str());
    }
    let bad = |_| "Ungültige Anfrage".to_string();
    let resp = match body {
        None => agent().run(rb.body(()).map_err(bad)?),
        Some(Body::Json(v)) => agent().run(rb.body(v.to_string().into_bytes()).map_err(bad)?),
        Some(Body::Bytes(b)) => agent().run(rb.body(b).map_err(bad)?),
    }
    .map_err(net_error)?;
    for v in resp.headers().get_all("set-cookie") {
        if let Ok(v) = v.to_str() {
            apply_set_cookie(v);
        }
    }
    let status = resp.status().as_u16();
    let version = resp.headers().get("x-board-version").and_then(|v| v.to_str().ok()).and_then(|v| v.trim().parse::<i64>().ok());
    let mut resp = resp;
    let body = resp.body_mut().with_config().limit(256 * 1024 * 1024).read_to_vec().map_err(net_error)?;
    Ok(Resp { status, version, body })
}

pub fn request(method: &'static str, path: String, body: Option<Body>, headers: Vec<(&'static str, String)>, done: Box<dyn FnOnce(Result<Resp, String>)>) {
    let (id, tx) = PENDING.with(|p| {
        let mut p = p.borrow_mut();
        let id = p.next;
        p.next += 1;
        p.callbacks.insert(id, done);
        (id, p.tx.clone())
    });
    let t2 = tx.clone();
    let spawned = std::thread::Builder::new().name("owndraw-http".into()).spawn(move || {
        let r = perform(method, &path, body, &headers);
        let _ = t2.send((id, r));
        wake();
    });
    if spawned.is_err() {
        let _ = tx.send((id, Err("Netzwerk-Thread konnte nicht gestartet werden".into())));
    }
}

/// Pro Frame im UI-Thread: fertige Antworten ausliefern.
pub fn pump() {
    let mut ran = false;
    loop {
        // Nicht während des Aufrufs geborgt halten: `done` darf selbst `request` aufrufen.
        let next = PENDING.with(|p| {
            let mut p = p.borrow_mut();
            match p.rx.try_recv() {
                Ok((id, r)) => Some((p.callbacks.remove(&id), r)),
                Err(_) => None,
            }
        });
        let Some((cb, r)) = next else { break };
        ran = true;
        if let Some(cb) = cb {
            cb(r);
        }
    }
    if ran {
        repaint();
    }
}

// ---------------------------------------------------------------- WebSocket

enum Stream {
    Plain(TcpStream),
    Tls(Box<rustls::StreamOwned<rustls::ClientConnection, TcpStream>>),
}

impl Stream {
    fn tcp(&self) -> &TcpStream {
        match self {
            Stream::Plain(s) => s,
            Stream::Tls(s) => &s.sock,
        }
    }
}

impl Read for Stream {
    fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
        match self {
            Stream::Plain(s) => s.read(buf),
            Stream::Tls(s) => s.read(buf),
        }
    }
}

impl Write for Stream {
    fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        match self {
            Stream::Plain(s) => s.write(buf),
            Stream::Tls(s) => s.write(buf),
        }
    }
    fn flush(&mut self) -> std::io::Result<()> {
        match self {
            Stream::Plain(s) => s.flush(),
            Stream::Tls(s) => s.flush(),
        }
    }
}

pub struct Socket {
    inbox: Arc<Mutex<Vec<SockEv>>>,
    tx: Sender<Value>,
    open: Arc<AtomicBool>,
    stop: Arc<AtomicBool>,
}

impl Socket {
    /// Öffnet die Verbindung im Hintergrund; Ergebnis kommt als `SockEv::Open` bzw. `SockEv::Closed`.
    pub fn connect(board: &str) -> Result<Socket, String> {
        let (base, token) = {
            let mut s = state();
            (s.server().to_string(), s.active_token())
        };
        let url = ws_url(&base, board)?;
        let inbox: Arc<Mutex<Vec<SockEv>>> = Arc::default();
        let (tx, rx) = channel();
        let open = Arc::new(AtomicBool::new(false));
        let stop = Arc::new(AtomicBool::new(false));
        let ctx = Ctx { inbox: inbox.clone(), open: open.clone(), stop: stop.clone() };
        std::thread::Builder::new().name("owndraw-ws".into()).spawn(move || ws_thread(url, token, rx, ctx)).map_err(|_| "Verbindungs-Thread konnte nicht gestartet werden".to_string())?;
        Ok(Socket { inbox, tx, open, stop })
    }

    pub fn send(&self, v: &Value) -> bool {
        self.open.load(Ordering::SeqCst) && self.tx.send(v.clone()).is_ok()
    }

    pub fn take(&self) -> Vec<SockEv> {
        std::mem::take(&mut *self.inbox.lock().unwrap_or_else(|e| e.into_inner()))
    }
}

impl Drop for Socket {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::SeqCst);
        self.open.store(false, Ordering::SeqCst);
    }
}

fn ws_url(base: &str, board: &str) -> Result<String, String> {
    let rest = if let Some(r) = base.strip_prefix("https://") {
        format!("wss://{r}")
    } else if let Some(r) = base.strip_prefix("http://") {
        format!("ws://{r}")
    } else {
        return Err("Ungültige Server-Adresse".into());
    };
    Ok(format!("{rest}/api/collab/{}", encode(board)))
}

struct Ctx {
    inbox: Arc<Mutex<Vec<SockEv>>>,
    open: Arc<AtomicBool>,
    stop: Arc<AtomicBool>,
}

impl Ctx {
    fn push(&self, ev: SockEv) {
        if self.stop.load(Ordering::SeqCst) {
            return;
        }
        self.inbox.lock().unwrap_or_else(|e| e.into_inner()).push(ev);
        wake();
    }
    fn closed(&self, code: u16, reason: String) {
        self.open.store(false, Ordering::SeqCst);
        self.push(SockEv::Closed { code, reason });
    }
}

fn tls_config() -> Result<Arc<rustls::ClientConfig>, String> {
    use rustls_platform_verifier::BuilderVerifierExt;
    static CFG: OnceLock<Result<Arc<rustls::ClientConfig>, String>> = OnceLock::new();
    CFG.get_or_init(|| {
        let provider = Arc::new(rustls::crypto::ring::default_provider());
        let b = rustls::ClientConfig::builder_with_provider(provider)
            .with_safe_default_protocol_versions()
            .map_err(|_| "TLS-Konfiguration fehlgeschlagen".to_string())?
            .with_platform_verifier()
            .map_err(|_| "Zertifikatsprüfung des Systems nicht verfügbar".to_string())?;
        Ok(Arc::new(b.with_no_client_auth()))
    })
    .clone()
}

fn connect_tcp(host: &str, port: u16) -> Result<TcpStream, String> {
    let addrs = (host, port).to_socket_addrs().map_err(|_| "Server nicht erreichbar".to_string())?;
    for a in addrs {
        if let Ok(s) = TcpStream::connect_timeout(&a, CONNECT_TIMEOUT) {
            return Ok(s);
        }
    }
    Err("Server nicht erreichbar".into())
}

fn open_ws(url: &str, token: Option<&str>) -> Result<tungstenite::WebSocket<Stream>, String> {
    use tungstenite::client::IntoClientRequest;
    let mut req = url.into_client_request().map_err(|_| "Ungültige Server-Adresse".to_string())?;
    if let Some(t) = token {
        let v = tungstenite::http::HeaderValue::from_str(&format!("{COOKIE}={t}")).map_err(|_| "Ungültige Sitzung".to_string())?;
        req.headers_mut().insert("Cookie", v);
    }
    // Bewusst kein Origin-Header: der Server erlaubt Clients ohne Origin.
    let uri = req.uri().clone();
    let tls = uri.scheme_str() == Some("wss");
    let host = uri.host().ok_or("Ungültige Server-Adresse")?.trim_start_matches('[').trim_end_matches(']').to_string();
    let port = uri.port_u16().unwrap_or(if tls { 443 } else { 80 });
    let tcp = connect_tcp(&host, port)?;
    let _ = tcp.set_nodelay(true);
    let _ = tcp.set_read_timeout(Some(CONNECT_TIMEOUT));
    let _ = tcp.set_write_timeout(Some(CONNECT_TIMEOUT));
    let stream = if tls {
        let name = rustls::pki_types::ServerName::try_from(host.clone()).map_err(|_| "Ungültiger Servername".to_string())?;
        let conn = rustls::ClientConnection::new(tls_config()?, name).map_err(|_| "Sichere Verbindung fehlgeschlagen".to_string())?;
        Stream::Tls(Box::new(rustls::StreamOwned::new(conn, tcp)))
    } else {
        Stream::Plain(tcp)
    };
    let deadline = Instant::now() + CONNECT_TIMEOUT;
    let mut hs = tungstenite::client::client(req, stream);
    let ws = loop {
        match hs {
            Ok((ws, _)) => break ws,
            Err(tungstenite::HandshakeError::Interrupted(mid)) if Instant::now() < deadline => hs = mid.handshake(),
            Err(tungstenite::HandshakeError::Interrupted(_)) => return Err("Zeitüberschreitung beim Verbindungsaufbau".into()),
            Err(tungstenite::HandshakeError::Failure(e)) => return Err(handshake_error(e)),
        }
    };
    let _ = ws.get_ref().tcp().set_read_timeout(Some(Duration::from_millis(40)));
    Ok(ws)
}

fn handshake_error(e: tungstenite::Error) -> String {
    use tungstenite::Error as E;
    match e {
        E::Http(r) => match r.status().as_u16() {
            401 => "Nicht angemeldet".into(),
            403 => "Kein Zugriff auf dieses Board".into(),
            404 => "Board nicht gefunden".into(),
            c => format!("Server hat die Verbindung abgelehnt (HTTP {c})"),
        },
        E::Io(_) | E::Tls(_) => "Verbindung zum Server fehlgeschlagen".into(),
        e => format!("Verbindung fehlgeschlagen: {e}"),
    }
}

fn is_timeout(e: &std::io::Error) -> bool {
    matches!(e.kind(), std::io::ErrorKind::WouldBlock | std::io::ErrorKind::TimedOut)
}

fn ws_thread(url: String, token: Option<String>, rx: Receiver<Value>, ctx: Ctx) {
    use tungstenite::{Error as E, Message};
    let mut ws = match open_ws(&url, token.as_deref()) {
        Ok(ws) => ws,
        Err(e) => return ctx.closed(1006, e),
    };
    if ctx.stop.load(Ordering::SeqCst) {
        let _ = ws.close(None);
        return;
    }
    ctx.open.store(true, Ordering::SeqCst);
    ctx.push(SockEv::Open);
    loop {
        let mut quit = ctx.stop.load(Ordering::SeqCst);
        loop {
            match rx.try_recv() {
                // Noch wartende Nachrichten gehen vor dem Schließen raus.
                Ok(v) => match ws.send(Message::Text(v.to_string().into())) {
                    Ok(()) => {}
                    Err(E::Io(e)) if is_timeout(&e) => {}
                    Err(_) => return ctx.closed(1006, String::new()),
                },
                Err(TryRecvError::Empty) => break,
                Err(TryRecvError::Disconnected) => {
                    quit = true;
                    break;
                }
            }
        }
        if quit {
            ctx.open.store(false, Ordering::SeqCst);
            let _ = ws.close(None);
            let _ = ws.flush();
            return;
        }
        match ws.read() {
            Ok(Message::Text(t)) => {
                if let Ok(v) = serde_json::from_str::<Value>(t.as_str()) {
                    ctx.push(SockEv::Msg(v));
                }
            }
            Ok(Message::Close(f)) => {
                let (code, reason) = f.map(|f| (u16::from(f.code), f.reason.to_string())).unwrap_or((1005, String::new()));
                let _ = ws.flush();
                return ctx.closed(code, reason);
            }
            Ok(_) => {}
            Err(E::Io(e)) if is_timeout(&e) => {}
            Err(_) => return ctx.closed(1006, String::new()),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::rc::Rc;

    /// Die Tests teilen sich den globalen Zustand (Server-Adresse, Token).
    static LOCK: Mutex<()> = Mutex::new(());

    type Slot = Rc<RefCell<Option<Result<Resp, String>>>>;

    fn wait(slot: &Slot) -> Result<Resp, String> {
        let t = Instant::now();
        while slot.borrow().is_none() {
            assert!(t.elapsed() < Duration::from_secs(10), "keine Antwort");
            pump();
            std::thread::sleep(Duration::from_millis(5));
        }
        slot.borrow_mut().take().unwrap()
    }

    fn call(method: &'static str, path: &str, body: Option<Body>) -> Result<Resp, String> {
        let slot: Slot = Rc::new(RefCell::new(None));
        let s2 = slot.clone();
        request(method, path.to_string(), body, vec![("X-Extra", "1".into())], Box::new(move |r| *s2.borrow_mut() = Some(r)));
        wait(&slot)
    }

    fn reset() {
        set_config_dir(std::env::temp_dir().join("owndraw-test-unused"));
        set_base_url("");
    }

    #[test]
    fn encode_wie_encode_uri_component() {
        assert_eq!(encode("AZaz09-_.!~*'()"), "AZaz09-_.!~*'()");
        assert_eq!(encode("a b/ä€?#&="), "a%20b%2F%C3%A4%E2%82%AC%3F%23%26%3D");
        assert_eq!(encode(""), "");
    }

    #[test]
    fn adresse_normalisieren() {
        assert_eq!(normalize("  owndraw.example.org/ "), "https://owndraw.example.org");
        assert_eq!(normalize("http://127.0.0.1:8080//"), "http://127.0.0.1:8080");
        assert_eq!(normalize(""), DEFAULT_URL);
        assert_eq!(ws_url("https://a.b", "x y").unwrap(), "wss://a.b/api/collab/x%20y");
        assert_eq!(ws_url("http://127.0.0.1:1", "x").unwrap(), "ws://127.0.0.1:1/api/collab/x");
    }

    #[test]
    fn request_cookie_und_csrf() {
        let _g = LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let dir = tempfile::tempdir().unwrap();
        set_config_dir(dir.path().to_path_buf());
        let srv = tiny_http::Server::http("127.0.0.1:0").unwrap();
        let url = format!("http://{}", srv.server_addr().to_ip().unwrap());
        set_base_url(&url);
        fn h(r: &tiny_http::Request, n: &'static str) -> Option<String> {
            r.headers().iter().find(|h| h.field.equiv(n)).map(|h| h.value.to_string())
        }
        let th = std::thread::spawn(move || {
            // 1. Login: CSRF-Header, noch kein Cookie, Set-Cookie wird übernommen
            let mut r = srv.recv().unwrap();
            assert_eq!(h(&r, "X-OwnDraw").as_deref(), Some("1"));
            assert_eq!(h(&r, "Cookie"), None);
            assert_eq!(h(&r, "Content-Type").as_deref(), Some("application/json"));
            assert_eq!(h(&r, "X-Extra").as_deref(), Some("1"));
            assert!(h(&r, "User-Agent").unwrap().starts_with("OwnDraw-Desktop/"));
            let mut b = String::new();
            std::io::Read::read_to_string(r.as_reader(), &mut b).unwrap();
            assert_eq!(b, r#"{"a":1}"#);
            let resp = tiny_http::Response::from_string("{}")
                .with_header(tiny_http::Header::from_bytes("Set-Cookie", "owndraw_session=tok123; HttpOnly; Path=/; Max-Age=100").unwrap())
                .with_header(tiny_http::Header::from_bytes("X-Board-Version", "7").unwrap());
            r.respond(resp).unwrap();
            // 2. GET: Cookie, kein CSRF-Header
            let r = srv.recv().unwrap();
            assert_eq!(h(&r, "Cookie").as_deref(), Some("owndraw_session=tok123"));
            assert_eq!(h(&r, "X-OwnDraw"), None);
            r.respond(tiny_http::Response::from_string("hallo").with_status_code(401)).unwrap();
            // 3. Bytes + Abmelden (Max-Age=0)
            let r = srv.recv().unwrap();
            assert_eq!(h(&r, "Content-Type").as_deref(), Some("application/octet-stream"));
            let resp = tiny_http::Response::from_string("")
                .with_header(tiny_http::Header::from_bytes("Set-Cookie", "owndraw_session=; HttpOnly; Path=/; Max-Age=0").unwrap());
            r.respond(resp).unwrap();
            // 4. danach kein Cookie mehr
            let r = srv.recv().unwrap();
            assert_eq!(h(&r, "Cookie"), None);
            r.respond(tiny_http::Response::from_string("")).unwrap();
        });
        let r = call("POST", "/api/auth/login", Some(Body::Json(serde_json::json!({"a": 1})))).unwrap();
        assert_eq!((r.status, r.version), (200, Some(7)));
        let file = dir.path().join(FILE);
        let saved: Value = serde_json::from_str(&std::fs::read_to_string(&file).unwrap()).unwrap();
        assert_eq!(saved["token"], "tok123");
        assert_eq!(saved["server"], url);
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            assert_eq!(std::fs::metadata(&file).unwrap().permissions().mode() & 0o777, 0o600);
        }
        let r = call("GET", "/api/me", None).unwrap();
        assert_eq!((r.status, r.body.as_slice(), r.version), (401, &b"hallo"[..], None));
        call("PUT", "/api/boards/x", Some(Body::Bytes(vec![1, 2, 3]))).unwrap();
        assert!(!file.exists());
        call("GET", "/api/me", None).unwrap();
        th.join().unwrap();
        reset();
    }

    #[test]
    fn token_laden_und_server_wechsel() {
        let _g = LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join(FILE), r#"{"server":"http://127.0.0.1:9","token":"abc"}"#).unwrap();
        set_config_dir(dir.path().to_path_buf());
        init(&egui::Context::default());
        set_base_url("http://127.0.0.1:9/");
        assert_eq!(state().active_token().as_deref(), Some("abc"));
        assert!(dir.path().join(FILE).exists());
        // anderer Server: Token verworfen, Datei weg
        set_base_url("127.0.0.1:9");
        assert_eq!(base_url(), "https://127.0.0.1:9");
        assert!(!dir.path().join(FILE).exists());
        assert!(state().active_token().is_none());
        reset();
    }

    #[test]
    fn netzfehler_ist_verstaendlich() {
        let _g = LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let l = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let port = l.local_addr().unwrap().port();
        drop(l);
        set_base_url(&format!("http://127.0.0.1:{port}"));
        let e = call("GET", "/api/me", None).err().unwrap();
        assert!(e.contains("nicht erreichbar"), "{e}");
        reset();
    }

    fn collect(sock: &Socket, n: usize) -> Vec<SockEv> {
        let mut evs = Vec::new();
        let t = Instant::now();
        while evs.len() < n && t.elapsed() < Duration::from_secs(10) {
            evs.extend(sock.take());
            std::thread::sleep(Duration::from_millis(10));
        }
        evs
    }

    #[test]
    fn websocket_cookie_nachrichten_und_schliessen() {
        let _g = LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let dir = tempfile::tempdir().unwrap();
        set_config_dir(dir.path().to_path_buf());
        let l = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        set_base_url(&format!("http://{}", l.local_addr().unwrap()));
        state().set_token(Some("wstok".into()));
        let th = std::thread::spawn(move || {
            let (s, _) = l.accept().unwrap();
            let cb = |req: &tungstenite::handshake::server::Request, resp: tungstenite::handshake::server::Response| {
                assert_eq!(req.uri().path(), "/api/collab/b1");
                assert_eq!(req.headers().get("cookie").unwrap(), "owndraw_session=wstok");
                assert!(req.headers().get("origin").is_none());
                Ok(resp)
            };
            let mut ws = tungstenite::accept_hdr(s, cb).unwrap();
            ws.send(tungstenite::Message::Text(r#"{"t":"hello"}"#.into())).unwrap();
            let m = ws.read().unwrap();
            assert_eq!(m.into_text().unwrap().as_str(), r#"{"t":"ping"}"#);
            // Der Client schließt, wenn das Socket fallen gelassen wird
            loop {
                match ws.read() {
                    Ok(tungstenite::Message::Close(_)) | Err(_) => break,
                    Ok(_) => {}
                }
            }
        });
        let sock = Socket::connect("b1").unwrap();
        assert!(!sock.send(&serde_json::json!({"t": "ping"})), "vor Open darf nichts gesendet werden");
        let evs = collect(&sock, 2);
        assert!(matches!(evs[0], SockEv::Open));
        assert!(matches!(&evs[1], SockEv::Msg(v) if v["t"] == "hello"));
        assert!(sock.send(&serde_json::json!({"t": "ping"})));
        drop(sock);
        th.join().unwrap();
        state().set_token(None);
        reset();
    }

    #[test]
    fn websocket_abgelehnt_gibt_closed() {
        let _g = LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let srv = tiny_http::Server::http("127.0.0.1:0").unwrap();
        set_base_url(&format!("http://{}", srv.server_addr().to_ip().unwrap()));
        let th = std::thread::spawn(move || srv.recv().unwrap().respond(tiny_http::Response::from_string("{}").with_status_code(404)).unwrap());
        let sock = Socket::connect("nix").unwrap();
        let evs = collect(&sock, 1);
        assert!(matches!(&evs[0], SockEv::Closed { code: 1006, reason } if reason.contains("nicht gefunden")));
        assert!(!sock.send(&serde_json::json!({})));
        th.join().unwrap();
        reset();
    }

    /// Manuell gegen einen echten, lokal laufenden Server:
    /// `OWNDRAW_TEST_SERVER=http://127.0.0.1:8080 cargo test -p owndraw --lib net::native -- --ignored --nocapture`
    /// Optional `OWNDRAW_TEST_BOARD=<id>` oder `new` (legt ein Board an, nur mit Lizenz) für den WebSocket; sonst wird ein nicht vorhandenes Board geprüft.
    #[test]
    #[ignore]
    fn echter_server() {
        let Ok(url) = std::env::var("OWNDRAW_TEST_SERVER") else { return };
        let _g = LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let dir = tempfile::tempdir().unwrap();
        set_config_dir(dir.path().to_path_buf());
        set_base_url(&url);
        // Mit OWNDRAW_TEST_EMAIL (Konto mit Lizenz und Board) anmelden, sonst ein neues Konto registrieren.
        let (email, route) = match std::env::var("OWNDRAW_TEST_EMAIL") {
            Ok(e) => (e, "login"),
            Err(_) => (format!("t{}@example.org", std::process::id()), "register"),
        };
        let r = call("GET", "/api/me", None).unwrap();
        assert_eq!(r.status, 401);
        let body = serde_json::json!({"email": email, "password": "ein-langes-passwort-123", "name": "Test"});
        let r = call("POST", &format!("/api/auth/{route}"), Some(Body::Json(body))).unwrap();
        assert!(r.ok(), "{} {}", r.status, String::from_utf8_lossy(&r.body));
        assert!(dir.path().join(FILE).exists(), "Token nicht gespeichert");
        let r = call("GET", "/api/me", None).unwrap();
        assert!(r.ok());
        assert_eq!(r.json()["email"], email.as_str());
        let mut board = std::env::var("OWNDRAW_TEST_BOARD").unwrap_or_else(|_| "gibt-es-nicht".into());
        if board == "new" {
            // Board anlegen (das Konto braucht dafür eine Lizenz)
            let doc = owndraw_core::doc::Document::new("Test", owndraw_core::pages::BoardMode::Infinite);
            let bytes = owndraw_core::doc::write_owndraw(&doc).unwrap();
            let r = call("POST", "/api/boards", Some(Body::Bytes(bytes))).unwrap();
            assert!(r.ok(), "{} {}", r.status, String::from_utf8_lossy(&r.body));
            board = r.json()["id"].as_str().unwrap().to_string();
        }
        let sock = Socket::connect(&board).unwrap();
        let evs = collect(&sock, 2);
        println!("Socket-Ereignisse: {}", evs.len());
        for e in &evs {
            match e {
                SockEv::Open => println!("  Open"),
                SockEv::Msg(v) => println!("  Msg t={}", v["t"]),
                SockEv::Closed { code, reason } => println!("  Closed {code} {reason}"),
            }
        }
        if board != "gibt-es-nicht" {
            assert!(matches!(evs[0], SockEv::Open));
            assert!(matches!(&evs[1], SockEv::Msg(v) if v["t"] == "hello"));
        }
        drop(sock);
        let r = call("POST", "/api/auth/logout", None).unwrap();
        assert!(r.ok());
        assert!(!dir.path().join(FILE).exists());
        reset();
    }
}
