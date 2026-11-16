//! Mitteilungen an alle Besucher (`/api/notices`, `/api/admin/notices`) und Wartungsmodus
//! (`/api/maintenance`, `/api/admin/maintenance`).

use crate::auth::{AdminAuth, Auth, J};
use crate::db::{self, now};
use crate::error::{ApiError, ApiResult};
use crate::state::AppState;
use axum::extract::{Path, Request, State};
use axum::http::header::{CACHE_CONTROL, RETRY_AFTER};
use axum::http::{HeaderValue, Method, StatusCode};
use axum::middleware::Next;
use axum::response::{Html, IntoResponse, Response};
use axum::Json;
use rusqlite::{params, Connection, OptionalExtension, Row};
use serde::{Deserialize, Deserializer, Serialize};
use serde_json::{json, Value};

const MAX_NOTICES: i64 = 100;
const MAX_TITLE: usize = 120;
const MAX_BODY: usize = 4000;
const MAX_MAINT_MSG: usize = 1000;

/// Wartungsseite (eingebettet; Platzhalter werden beim Ausliefern ersetzt).
const MAINTENANCE_HTML: &str = include_str!("maintenance.html");

/// Standardtext, wenn der Admin keine eigene Nachricht angibt.
pub const DEFAULT_MAINT_MSG: &str = "OwnDraw wird gerade gewartet. Bitte versuche es später noch einmal.";

// ---------- Mitteilungen ----------

struct Notice {
    id: String,
    title: String,
    body: String,
    kind: String,
    active: bool,
    starts_at: Option<i64>,
    ends_at: Option<i64>,
    created: i64,
    updated: i64,
    /// Angehängtes Board (Nur-Lesen-Bild in der Mitteilung); Name/Version `None`, wenn das Board gelöscht wurde.
    board_id: Option<String>,
    board_name: Option<String>,
    board_version: Option<i64>,
}

const COLS: &str = "n.id,n.title,n.body,n.kind,n.active,n.starts_at,n.ends_at,n.created,n.updated,n.board_id,b.name,b.version";
const FROM: &str = "FROM notices n LEFT JOIN boards b ON b.id=n.board_id";

impl Notice {
    fn from_row(r: &Row) -> rusqlite::Result<Self> {
        Ok(Self {
            id: r.get(0)?,
            title: r.get(1)?,
            body: r.get(2)?,
            kind: r.get(3)?,
            active: r.get::<_, i64>(4)? != 0,
            starts_at: r.get(5)?,
            ends_at: r.get(6)?,
            created: r.get(7)?,
            updated: r.get(8)?,
            board_id: r.get(9)?,
            board_name: r.get(10)?,
            board_version: r.get(11)?,
        })
    }

    /// Gerade sichtbar: aktiv und im Zeitraum.
    fn visible(&self, t: i64) -> bool {
        self.active && self.starts_at.is_none_or(|s| s <= t) && self.ends_at.is_none_or(|e| t < e)
    }

    fn admin_json(&self, t: i64) -> Value {
        json!({
            "id": self.id, "title": self.title, "body": self.body, "kind": self.kind, "active": self.active,
            "starts_at": self.starts_at, "ends_at": self.ends_at, "created": self.created, "updated": self.updated,
            "visible": self.visible(t), "board_id": self.board_id, "board_name": self.board_name,
            "board_missing": self.board_id.is_some() && self.board_name.is_none(),
        })
    }

    fn public_json(&self) -> Value {
        let board = match (&self.board_name, self.board_version) {
            (Some(name), Some(v)) => json!({"name": name, "url": format!("api/notices/{}/board.png?v={v}", self.id)}),
            _ => Value::Null,
        };
        json!({"id": self.id, "title": self.title, "body": self.body, "kind": self.kind, "updated": self.updated, "board": board})
    }
}

fn all_notices(c: &Connection) -> rusqlite::Result<Vec<Notice>> {
    let mut st = c.prepare(&format!("SELECT {COLS} {FROM} ORDER BY n.position, n.created DESC, n.id"))?;
    let v = st.query_map([], Notice::from_row)?.collect();
    v
}

fn notice_by_id(c: &Connection, id: &str) -> rusqlite::Result<Option<Notice>> {
    c.query_row(&format!("SELECT {COLS} {FROM} WHERE n.id=?1"), [id], Notice::from_row).optional()
}

fn clean_text(s: &str, max: usize) -> String {
    s.trim().chars().filter(|c| *c == '\n' || *c == '\t' || !c.is_control()).take(max).collect()
}

fn check_kind(k: &str) -> ApiResult<()> {
    if matches!(k, "info" | "warning") {
        Ok(())
    } else {
        Err(ApiError::invalid("Der Typ muss „info“ oder „warning“ sein."))
    }
}

/// Leere ID = kein Board; sonst muss das Board existieren.
fn check_board(c: &Connection, id: Option<String>) -> ApiResult<Option<String>> {
    let Some(id) = id.map(|s| s.trim().to_string()).filter(|s| !s.is_empty()) else { return Ok(None) };
    if crate::boards::board_row(c, &id)?.is_none() {
        return Err(ApiError::invalid("Das gewählte Board gibt es nicht (mehr)."));
    }
    Ok(Some(id))
}

fn check_range(s: Option<i64>, e: Option<i64>) -> ApiResult<()> {
    if let (Some(s), Some(e)) = (s, e) {
        if e <= s {
            return Err(ApiError::invalid("Das Ende muss nach dem Beginn liegen."));
        }
    }
    Ok(())
}

/// Öffentlich: alle gerade sichtbaren Mitteilungen (in der vom Admin festgelegten Reihenfolge). Auch im Wartungsmodus erreichbar.
pub async fn public_list(State(s): State<AppState>) -> ApiResult<Response> {
    let v = s.db.call(|c| {
        let t = now();
        Ok(all_notices(c)?.into_iter().filter(|n| n.visible(t)).map(|n| n.public_json()).collect::<Vec<_>>())
    })
    .await?;
    let mut r = Json(Value::Array(v)).into_response();
    r.headers_mut().insert(CACHE_CONTROL, HeaderValue::from_static("no-store"));
    Ok(r)
}

pub async fn admin_list(State(s): State<AppState>, _a: AdminAuth) -> ApiResult<Json<Value>> {
    let v = s.db.call(|c| {
        let t = now();
        Ok(Value::Array(all_notices(c)?.iter().map(|n| n.admin_json(t)).collect()))
    })
    .await?;
    Ok(Json(v))
}

fn default_kind() -> String {
    "info".into()
}
fn default_true() -> bool {
    true
}

#[derive(Deserialize)]
pub struct CreateNotice {
    #[serde(default)]
    title: String,
    #[serde(default)]
    body: String,
    #[serde(default = "default_kind")]
    kind: String,
    #[serde(default = "default_true")]
    active: bool,
    #[serde(default)]
    starts_at: Option<i64>,
    #[serde(default)]
    ends_at: Option<i64>,
    #[serde(default)]
    board_id: Option<String>,
}

pub async fn create(State(s): State<AppState>, _a: AdminAuth, J(b): J<CreateNotice>) -> ApiResult<Json<Value>> {
    let title = clean_text(&b.title, MAX_TITLE);
    let body = clean_text(&b.body, MAX_BODY);
    if title.is_empty() && body.is_empty() {
        return Err(ApiError::invalid("Bitte einen Titel oder Text eingeben."));
    }
    check_kind(&b.kind)?;
    check_range(b.starts_at, b.ends_at)?;
    let v = s.db.call(move |c| {
        let n: i64 = c.query_row("SELECT COUNT(*) FROM notices", [], |r| r.get(0))?;
        if n >= MAX_NOTICES {
            return Err(ApiError::invalid(format!("Es sind höchstens {MAX_NOTICES} Mitteilungen möglich. Bitte alte löschen.")));
        }
        let board_id = check_board(c, b.board_id)?;
        let t = now();
        let id = uuid::Uuid::new_v4().to_string();
        c.execute(
            "INSERT INTO notices(id,title,body,kind,active,starts_at,ends_at,created,updated,position,board_id)
             VALUES(?1,?2,?3,?4,?5,?6,?7,?8,?8,(SELECT COALESCE(MAX(position),-1)+1 FROM notices),?9)",
            params![id, title, body, b.kind, b.active as i64, b.starts_at, b.ends_at, t, board_id],
        )?;
        Ok(notice_by_id(c, &id)?.ok_or_else(|| ApiError::internal("Mitteilung fehlt"))?.admin_json(t))
    })
    .await?;
    Ok(Json(v))
}

#[derive(Deserialize)]
pub struct OrderBody {
    ids: Vec<String>,
}

/// `PUT /api/admin/notices/order`: neue Reihenfolge (alle IDs, erste = wird zuerst angezeigt).
pub async fn reorder(State(s): State<AppState>, _a: AdminAuth, J(b): J<OrderBody>) -> ApiResult<Json<Value>> {
    let v = s.db.call(move |c| {
        let tx = c.transaction()?;
        let mut have: Vec<String> = all_notices(&tx)?.into_iter().map(|n| n.id).collect();
        let mut want = b.ids.clone();
        have.sort();
        want.sort();
        if have != want {
            return Err(ApiError::conflict("Die Liste der Mitteilungen hat sich geändert. Bitte neu laden."));
        }
        for (i, id) in b.ids.iter().enumerate() {
            tx.execute("UPDATE notices SET position=?2 WHERE id=?1", params![id, i as i64])?;
        }
        let t = now();
        let v = Value::Array(all_notices(&tx)?.iter().map(|n| n.admin_json(t)).collect());
        tx.commit()?;
        Ok(v)
    })
    .await?;
    Ok(Json(v))
}

fn double_option<'de, D, T>(d: D) -> Result<Option<Option<T>>, D::Error>
where
    D: Deserializer<'de>,
    T: Deserialize<'de>,
{
    Ok(Some(Option::deserialize(d)?))
}

#[derive(Deserialize)]
pub struct PatchNotice {
    title: Option<String>,
    body: Option<String>,
    kind: Option<String>,
    active: Option<bool>,
    /// Fehlt = unverändert, `null` = ohne Grenze, Zahl = Unix-Sekunden.
    #[serde(default, deserialize_with = "double_option")]
    starts_at: Option<Option<i64>>,
    #[serde(default, deserialize_with = "double_option")]
    ends_at: Option<Option<i64>>,
    /// Fehlt = unverändert, `null`/leer = kein Board.
    #[serde(default, deserialize_with = "double_option")]
    board_id: Option<Option<String>>,
}

pub async fn patch(State(s): State<AppState>, _a: AdminAuth, Path(id): Path<String>, J(b): J<PatchNotice>) -> ApiResult<Json<Value>> {
    if let Some(k) = &b.kind {
        check_kind(k)?;
    }
    let v = s.db.call(move |c| {
        let n = notice_by_id(c, &id)?.ok_or_else(|| ApiError::not_found("Mitteilung nicht gefunden."))?;
        let title = b.title.as_deref().map(|t| clean_text(t, MAX_TITLE)).unwrap_or(n.title);
        let body = b.body.as_deref().map(|t| clean_text(t, MAX_BODY)).unwrap_or(n.body);
        if title.is_empty() && body.is_empty() {
            return Err(ApiError::invalid("Bitte einen Titel oder Text eingeben."));
        }
        let starts_at = b.starts_at.unwrap_or(n.starts_at);
        let ends_at = b.ends_at.unwrap_or(n.ends_at);
        check_range(starts_at, ends_at)?;
        let board_id = match b.board_id {
            Some(v) => check_board(c, v)?,
            None => n.board_id,
        };
        let t = now();
        c.execute(
            "UPDATE notices SET title=?2, body=?3, kind=?4, active=?5, starts_at=?6, ends_at=?7, updated=?8, board_id=?9 WHERE id=?1",
            params![id, title, body, b.kind.unwrap_or(n.kind), b.active.unwrap_or(n.active) as i64, starts_at, ends_at, t, board_id],
        )?;
        Ok(notice_by_id(c, &id)?.ok_or_else(|| ApiError::not_found("Mitteilung nicht gefunden."))?.admin_json(t))
    })
    .await?;
    Ok(Json(v))
}

pub async fn delete(State(s): State<AppState>, _a: AdminAuth, Path(id): Path<String>) -> ApiResult<Json<Value>> {
    s.db.call(move |c| {
        if c.execute("DELETE FROM notices WHERE id=?1", [&id])? == 0 {
            return Err(ApiError::not_found("Mitteilung nicht gefunden."));
        }
        Ok(())
    })
    .await?;
    Ok(Json(json!({})))
}

// ---------- Angehängte Boards ----------

/// Größte Bildbreite bzw. -höhe des Board-Bildes (Seiten-Boards werden untereinander gezeigt, daher höher erlaubt).
const BOARD_MAX_W: f32 = 2000.0;
const BOARD_MAX_H: f32 = 6000.0;

type PngCache = std::sync::Mutex<std::collections::HashMap<String, (i64, std::sync::Arc<Vec<u8>>)>>;

/// Zuletzt gerenderte Bilder je Board (Schlüssel: Board-ID, gültig für eine Board-Version).
fn png_cache() -> &'static PngCache {
    static C: std::sync::OnceLock<PngCache> = std::sync::OnceLock::new();
    C.get_or_init(Default::default)
}

/// Ganzes Board als PNG (mit Hintergrund und Bildern). `None` bei leerem oder unlesbarem Board.
fn render_board_png(bytes: &[u8]) -> Option<Vec<u8>> {
    let doc = owndraw_core::doc::read_owndraw(bytes).ok()?;
    let r = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        owndraw_core::item::clear_mem_images();
        for (n, b) in &doc.images.files {
            owndraw_core::item::put_mem_image(n, b.clone());
        }
        let mut b = owndraw_core::board::Board::new(1.0);
        b.from_json(&doc.board);
        b.board_mode = doc.mode.clone();
        let png = (|| {
            let e = b.export_bounds()?;
            let (w, h) = (e.width().max(1.0), e.height().max(1.0));
            let m = if matches!(doc.mode, owndraw_core::pages::BoardMode::Pages(_)) { 0.0 } else { w.max(h) * 0.03 + 8.0 };
            let r = owndraw_core::geom::RectF::new(e.left - m, e.top - m, e.left + w + m, e.top + h + m);
            let k = (BOARD_MAX_W / r.width()).min(BOARD_MAX_H / r.height()).min(3.0);
            b.render_composite(&r, k, true)?.encode_png().ok()
        })();
        owndraw_core::item::clear_mem_images();
        png
    }));
    r.ok().flatten()
}

/// Liefert das Bild eines Boards (aus dem Zwischenspeicher oder frisch gerendert).
async fn board_png(s: &AppState, board_id: String) -> ApiResult<Response> {
    let bid = board_id.clone();
    let version = s
        .db
        .call(move |c| Ok(crate::boards::board_row(c, &bid)?.map(|b| b.version)))
        .await?
        .ok_or_else(|| ApiError::not_found("Board nicht gefunden."))?;
    let hit = png_cache().lock().unwrap_or_else(|p| p.into_inner()).get(&board_id).filter(|(v, _)| *v == version).map(|(_, p)| p.clone());
    let png = match hit {
        Some(p) => p,
        None => {
            let path = crate::boards::board_path(&s.cfg.boards_dir(), &board_id);
            let png = tokio::task::spawn_blocking(move || render_board_png(&std::fs::read(path).ok()?))
                .await
                .map_err(ApiError::internal)?
                .ok_or_else(|| ApiError::not_found("Das Board ist leer oder konnte nicht gelesen werden."))?;
            let png = std::sync::Arc::new(png);
            let mut c = png_cache().lock().unwrap_or_else(|p| p.into_inner());
            if c.len() >= 32 {
                c.clear();
            }
            c.insert(board_id, (version, png.clone()));
            png
        }
    };
    let mut r = png.as_ref().clone().into_response();
    r.headers_mut().insert(axum::http::header::CONTENT_TYPE, HeaderValue::from_static("image/png"));
    r.headers_mut().insert(CACHE_CONTROL, HeaderValue::from_static("no-cache"));
    Ok(r)
}

/// Öffentlich: Bild des an eine gerade sichtbare Mitteilung angehängten Boards (Nur-Lesen, auch für Gäste
/// ohne Zugriff auf das Board selbst).
pub async fn public_board(State(s): State<AppState>, Path(id): Path<String>) -> ApiResult<Response> {
    let board_id = s
        .db
        .call(move |c| Ok(notice_by_id(c, &id)?.filter(|n| n.visible(now()) && n.board_name.is_some()).and_then(|n| n.board_id)))
        .await?
        .ok_or_else(|| ApiError::not_found("Kein Board an dieser Mitteilung."))?;
    board_png(&s, board_id).await
}

/// Admin: Vorschau eines beliebigen Boards (Auswahl im Tab Mitteilungen).
pub async fn admin_board_preview(State(s): State<AppState>, _a: AdminAuth, Path(id): Path<String>) -> ApiResult<Response> {
    board_png(&s, id).await
}

/// Admin: alle Boards zur Auswahl (neueste zuerst).
pub async fn admin_boards(State(s): State<AppState>, _a: AdminAuth) -> ApiResult<Json<Value>> {
    let v = s
        .db
        .call(|c| {
            let mut st = c.prepare(
                "SELECT b.id,b.name,o.name,o.email,b.modified FROM boards b JOIN users o ON o.id=b.owner_id ORDER BY b.modified DESC LIMIT 1000",
            )?;
            let v = st
                .query_map([], |r| {
                    Ok(json!({"id": r.get::<_, String>(0)?, "name": r.get::<_, String>(1)?, "owner": r.get::<_, String>(2)?,
                              "owner_email": r.get::<_, Option<String>>(3)?, "modified": r.get::<_, i64>(4)?}))
                })?
                .collect::<rusqlite::Result<Vec<_>>>()?;
            Ok(Value::Array(v))
        })
        .await?;
    Ok(Json(v))
}

// ---------- Wartungsmodus ----------

/// Gespeichert als JSON in `settings` unter `maintenance`; im Speicher zwischengespeichert (`AppState::maintenance`).
#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq)]
pub struct Maintenance {
    pub active: bool,
    #[serde(default)]
    pub message: String,
    /// Endet automatisch zu diesem Zeitpunkt (Unix-Sekunden), `None` = bis zum Ausschalten.
    #[serde(default)]
    pub until: Option<i64>,
    #[serde(default)]
    pub since: Option<i64>,
}

impl Maintenance {
    /// Gerade wirksam (eingeschaltet und Endzeit nicht erreicht).
    pub fn on(&self, t: i64) -> bool {
        self.active && self.until.is_none_or(|u| t < u)
    }

    pub fn text(&self) -> &str {
        if self.message.trim().is_empty() {
            DEFAULT_MAINT_MSG
        } else {
            &self.message
        }
    }

    fn json(&self, t: i64) -> Value {
        let on = self.on(t);
        json!({
            "active": on,
            "message": if on { self.text() } else { "" },
            "until": if on { self.until } else { None },
            "since": if on { self.since } else { None },
        })
    }

    fn admin_json(&self, t: i64) -> Value {
        json!({
            "active": self.on(t), "enabled": self.active, "message": self.message, "until": self.until,
            "since": self.since, "default_message": DEFAULT_MAINT_MSG,
        })
    }
}

fn load_maintenance(c: &Connection) -> rusqlite::Result<Maintenance> {
    Ok(db::get_setting(c, "maintenance")?.and_then(|s| serde_json::from_str(&s).ok()).unwrap_or_default())
}

/// Aktueller Wartungszustand (beim ersten Zugriff aus der DB geladen).
pub async fn current(s: &AppState) -> ApiResult<Maintenance> {
    if let Some(m) = s.maintenance.read().unwrap_or_else(|p| p.into_inner()).clone() {
        return Ok(m);
    }
    let m = s.db.call(|c| Ok(load_maintenance(c)?)).await?;
    *s.maintenance.write().unwrap_or_else(|p| p.into_inner()) = Some(m.clone());
    Ok(m)
}

/// Öffentlich: Wartungszustand (für die Web-App und die Wartungsseite).
pub async fn public_state(State(s): State<AppState>) -> ApiResult<Response> {
    let m = current(&s).await?;
    let mut r = Json(m.json(now())).into_response();
    r.headers_mut().insert(CACHE_CONTROL, HeaderValue::from_static("no-store"));
    Ok(r)
}

pub async fn admin_state(State(s): State<AppState>, _a: AdminAuth) -> ApiResult<Json<Value>> {
    Ok(Json(current(&s).await?.admin_json(now())))
}

#[derive(Deserialize)]
pub struct PutMaintenance {
    active: bool,
    #[serde(default)]
    message: String,
    #[serde(default)]
    until: Option<i64>,
}

pub async fn put_state(State(s): State<AppState>, _a: AdminAuth, J(b): J<PutMaintenance>) -> ApiResult<Json<Value>> {
    let t = now();
    if b.active && b.until.is_some_and(|u| u <= t) {
        return Err(ApiError::invalid("Die Endzeit liegt in der Vergangenheit."));
    }
    let old = current(&s).await?;
    let m = Maintenance {
        active: b.active,
        message: clean_text(&b.message, MAX_MAINT_MSG),
        until: if b.active { b.until } else { None },
        since: if !b.active {
            None
        } else if old.on(t) {
            old.since.or(Some(t))
        } else {
            Some(t)
        },
    };
    let raw = serde_json::to_string(&m).map_err(ApiError::internal)?;
    s.db.call(move |c| Ok(db::set_setting(c, "maintenance", &raw)?)).await?;
    *s.maintenance.write().unwrap_or_else(|p| p.into_inner()) = Some(m.clone());
    if m.on(t) {
        s.hub.kick_non_admins(m.text());
    }
    Ok(Json(m.admin_json(t)))
}

/// Anfragen, die im Wartungsmodus auch Nicht-Admins beantwortet werden (Pfad ohne `/api`).
fn always_allowed(method: &Method, path: &str) -> bool {
    matches!(path, "/health" | "/notices" | "/maintenance" | "/auth/login" | "/auth/logout")
        || path.starts_with("/auth/2fa/")
        || (method == Method::GET && path == "/me")
        || path.starts_with("/admin/")
}

/// Ist die Anfrage von einem angemeldeten Admin?
pub(crate) async fn is_admin(s: &AppState, parts: &mut axum::http::request::Parts) -> bool {
    use axum::extract::FromRequestParts;
    Auth::from_request_parts(parts, s).await.is_ok_and(|a| a.user.admin)
}

fn maintenance_error(m: &Maintenance) -> Response {
    let mut r = ApiError::maintenance(m.text())
        .with_extra(json!({"until": m.until}))
        .into_response();
    r.headers_mut().insert(RETRY_AFTER, HeaderValue::from_static("120"));
    r
}

/// API-Middleware: im Wartungsmodus bekommen Nicht-Admins 503 `maintenance`.
pub async fn api_guard(State(s): State<AppState>, req: Request, next: Next) -> Response {
    let m = match current(&s).await {
        Ok(m) => m,
        Err(e) => return e.into_response(),
    };
    if !m.on(now()) {
        return next.run(req).await;
    }
    let path = req.uri().path();
    let path = path.strip_prefix("/api").unwrap_or(path).to_string();
    if always_allowed(req.method(), &path) {
        return next.run(req).await;
    }
    let (mut parts, body) = req.into_parts();
    if is_admin(&s, &mut parts).await {
        return next.run(Request::from_parts(parts, body)).await;
    }
    maintenance_error(&m)
}

fn html_escape(s: &str) -> String {
    let mut o = String::with_capacity(s.len());
    for ch in s.chars() {
        match ch {
            '&' => o.push_str("&amp;"),
            '<' => o.push_str("&lt;"),
            '>' => o.push_str("&gt;"),
            '"' => o.push_str("&quot;"),
            '\'' => o.push_str("&#39;"),
            c => o.push(c),
        }
    }
    o
}

/// Wartungsseite mit Nachricht und (falls gesetzt) Endzeit.
pub fn maintenance_page(m: &Maintenance) -> Response {
    let html = MAINTENANCE_HTML
        .replace("__MESSAGE__", &html_escape(m.text()))
        .replace("__UNTIL__", &m.until.map(|u| u.to_string()).unwrap_or_default());
    let mut r = (StatusCode::SERVICE_UNAVAILABLE, Html(html)).into_response();
    let h = r.headers_mut();
    h.insert(CACHE_CONTROL, HeaderValue::from_static("no-store"));
    h.insert(RETRY_AFTER, HeaderValue::from_static("120"));
    r
}

/// Statische Auslieferung: im Wartungsmodus bekommen Nicht-Admins statt der Web-App die Wartungsseite.
/// Nur Seitenaufrufe (HTML) werden ersetzt; Bilder, Skripte usw. bleiben unverändert.
pub async fn static_guard(State(s): State<AppState>, req: Request, next: Next) -> Response {
    let is_page = matches!(*req.method(), Method::GET | Method::HEAD) && {
        let p = req.uri().path();
        p == "/" || p.ends_with("/index.html") || p.ends_with('/')
    };
    if !is_page {
        return next.run(req).await;
    }
    let Ok(m) = current(&s).await else { return next.run(req).await };
    if !m.on(now()) {
        return next.run(req).await;
    }
    let (mut parts, body) = req.into_parts();
    if is_admin(&s, &mut parts).await {
        return next.run(Request::from_parts(parts, body)).await;
    }
    maintenance_page(&m)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn wartung_endet_automatisch() {
        let m = Maintenance { active: true, message: String::new(), until: Some(100), since: Some(1) };
        assert!(m.on(99));
        assert!(!m.on(100));
        assert_eq!(m.text(), DEFAULT_MAINT_MSG);
        assert!(!Maintenance::default().on(0));
    }

    #[test]
    fn escape_und_text() {
        assert_eq!(html_escape("<a href=\"x\">&'"), "&lt;a href=&quot;x&quot;&gt;&amp;&#39;");
        assert_eq!(clean_text("  a\u{7}b\nc  ", 10), "ab\nc");
    }
}
