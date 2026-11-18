//! Boards: `.owndraw`-Dateien unter `data/boards/<id>.owndraw`, Metadaten in SQLite.

use crate::auth::{read_limited, Auth, J};
use crate::db::{self, now};
use crate::error::{ApiError, ApiResult};
use crate::state::AppState;
use axum::body::Body;
use axum::extract::{Path, State};
use axum::http::header::CONTENT_TYPE;
use axum::http::{HeaderMap, HeaderValue};
use axum::response::{IntoResponse, Response};
use axum::Json;
use rusqlite::{params, Connection, OptionalExtension};
use serde::Deserialize;
use serde_json::{json, Value};
use std::path::{Path as FsPath, PathBuf};

pub(crate) struct BoardRow {
    pub id: String,
    pub owner_id: String,
    pub owner_name: String,
    pub name: String,
    pub version: i64,
    pub size: i64,
    pub modified: i64,
}

const BOARD_SELECT: &str = "SELECT b.id,b.owner_id,o.name,b.name,b.version,b.size,b.modified FROM boards b JOIN users o ON o.id=b.owner_id";

fn board_from_row(r: &rusqlite::Row) -> rusqlite::Result<BoardRow> {
    Ok(BoardRow {
        id: r.get(0)?,
        owner_id: r.get(1)?,
        owner_name: r.get(2)?,
        name: r.get(3)?,
        version: r.get(4)?,
        size: r.get(5)?,
        modified: r.get(6)?,
    })
}

pub(crate) fn board_row(conn: &Connection, id: &str) -> rusqlite::Result<Option<BoardRow>> {
    conn.query_row(&format!("{BOARD_SELECT} WHERE b.id=?1"), [id], board_from_row).optional()
}

/// Rolle des Nutzers am Board: `owner`, `edit`, `view` oder `None` (kein Zugriff).
pub(crate) fn role_for(conn: &Connection, b: &BoardRow, uid: &str) -> rusqlite::Result<Option<String>> {
    if b.owner_id == uid {
        return Ok(Some("owner".into()));
    }
    conn.query_row("SELECT role FROM members WHERE board_id=?1 AND user_id=?2", params![b.id, uid], |r| r.get(0)).optional()
}

pub(crate) fn info(b: &BoardRow, role: &str) -> Value {
    json!({
        "id": b.id,
        "name": b.name,
        "owner": {"id": b.owner_id, "name": b.owner_name},
        "role": role,
        "version": b.version,
        "size": b.size,
        "modified": b.modified,
    })
}

/// Board sehen (Rolle) oder 404 (auch bei fehlender Berechtigung, damit IDs nichts verraten).
pub(crate) fn visible(conn: &Connection, id: &str, uid: &str) -> ApiResult<(BoardRow, String)> {
    let b = board_row(conn, id)?.ok_or_else(|| ApiError::not_found("Board nicht gefunden."))?;
    let role = role_for(conn, &b, uid)?.ok_or_else(|| ApiError::not_found("Board nicht gefunden."))?;
    Ok((b, role))
}

pub(crate) fn board_path(dir: &FsPath, id: &str) -> PathBuf {
    dir.join(format!("{id}.owndraw"))
}

pub(crate) fn write_atomic(path: &FsPath, bytes: &[u8]) -> std::io::Result<()> {
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir)?;
    }
    let tmp = path.with_extension(format!("tmp-{}", db::random_hex(4)));
    let res = std::fs::write(&tmp, bytes).and_then(|_| std::fs::rename(&tmp, path));
    if res.is_err() {
        let _ = std::fs::remove_file(&tmp);
    }
    res
}

fn validate_zip(bytes: &[u8]) -> ApiResult<()> {
    if bytes.starts_with(b"PK\x03\x04") {
        Ok(())
    } else {
        Err(ApiError::invalid("Das ist keine gültige .owndraw-Datei (ZIP erwartet)."))
    }
}

fn board_name(h: &HeaderMap) -> String {
    let raw = h.get("x-board-name").map(|v| v.as_bytes()).unwrap_or_default();
    let name = percent_encoding::percent_decode(raw).decode_utf8_lossy().trim().chars().take(200).collect::<String>();
    if name.is_empty() {
        "Unbenannt".into()
    } else {
        name
    }
}

pub(crate) fn license_msg_owner(own: bool) -> String {
    if own {
        "Dafür ist eine gültige Lizenz nötig. Die Boards bleiben les- und herunterladbar.".into()
    } else {
        "Die Lizenz der Besitzerin oder des Besitzers ist abgelaufen; das Board ist nur noch lesbar.".into()
    }
}

// ---------- Routen ----------

pub async fn list(State(s): State<AppState>, a: Auth) -> ApiResult<Json<Value>> {
    let uid = a.user.id;
    let v =
        s.db.call(move |c| {
            let mut st = c.prepare(&format!(
                "{BOARD_SELECT} LEFT JOIN members m ON m.board_id=b.id AND m.user_id=?1
                 WHERE b.owner_id=?1 OR m.user_id=?1 ORDER BY b.modified DESC, b.id"
            ))?;
            let rows = st.query_map([&uid], board_from_row)?.collect::<Result<Vec<_>, _>>()?;
            let mut out = Vec::new();
            for b in rows {
                let role = role_for(c, &b, &uid)?.unwrap_or_else(|| "view".into());
                out.push(info(&b, &role));
            }
            Ok(Value::Array(out))
        })
        .await?;
    Ok(Json(v))
}

pub async fn create(State(s): State<AppState>, a: Auth, headers: HeaderMap, body: Body) -> ApiResult<Json<Value>> {
    let uid = a.user.id;
    let u2 = uid.clone();
    s.db.call(move |c| {
        db::active_license_of(c, &u2, now())?.map(|_| ()).ok_or_else(|| ApiError::license_required(license_msg_owner(true)))
    })
    .await?;
    let bytes = read_limited(body, s.cfg.max_board_bytes).await?;
    validate_zip(&bytes)?;
    let name = board_name(&headers);
    let dir = s.cfg.boards_dir();
    let v =
        s.db.call(move |c| {
            let t = now();
            let lic = db::active_license_of(c, &uid, t)?.ok_or_else(|| ApiError::license_required(license_msg_owner(true)))?;
            if db::used_bytes(c, &uid)? + bytes.len() as i64 > lic.quota_mb * 1_048_576 {
                return Err(ApiError::quota(format!("Dein Speicherplatz ({} MB) ist voll.", lic.quota_mb)));
            }
            let id = uuid::Uuid::new_v4().to_string();
            write_atomic(&board_path(&dir, &id), &bytes)?;
            c.execute(
                "INSERT INTO boards(id,owner_id,name,version,size,modified) VALUES(?1,?2,?3,1,?4,?5)",
                params![id, uid, name, bytes.len() as i64, t],
            )?;
            crate::stats::count_board_created(c)?;
            let b = board_row(c, &id)?.ok_or_else(|| ApiError::internal("Board fehlt nach dem Anlegen"))?;
            Ok(info(&b, "owner"))
        })
        .await?;
    Ok(Json(v))
}

pub async fn get(State(s): State<AppState>, a: Auth, Path(id): Path<String>) -> ApiResult<Response> {
    let uid = a.user.id;
    let id2 = id.clone();
    let (b, _) = s.db.call(move |c| visible(c, &id2, &uid)).await?;
    let bytes = tokio::fs::read(board_path(&s.cfg.boards_dir(), &id))
        .await
        .map_err(|_| ApiError::not_found("Die Board-Datei fehlt auf dem Server."))?;
    let mut r = bytes.into_response();
    let h = r.headers_mut();
    h.insert(CONTENT_TYPE, HeaderValue::from_static("application/octet-stream"));
    h.insert("x-board-version", HeaderValue::from(b.version));
    h.insert("cache-control", HeaderValue::from_static("no-store"));
    Ok(r)
}

fn parse_if_match(h: &HeaderMap) -> ApiResult<i64> {
    let raw = h.get("if-match").and_then(|v| v.to_str().ok()).ok_or_else(|| ApiError::invalid("Der Header If-Match (Version) fehlt."))?;
    raw.trim()
        .trim_start_matches("W/")
        .trim_matches('"')
        .parse()
        .map_err(|_| ApiError::invalid("If-Match muss die Versionsnummer enthalten."))
}

pub async fn put(State(s): State<AppState>, a: Auth, Path(id): Path<String>, headers: HeaderMap, body: Body) -> ApiResult<Json<Value>> {
    let want = parse_if_match(&headers)?;
    if s.hub.is_active(&id) {
        return Err(ApiError::conflict("Das Board wird gerade live bearbeitet; Änderungen laufen über die Live-Verbindung.").with_extra(json!({"live": true})));
    }
    let uid = a.user.id;
    let bytes = read_limited(body, s.cfg.max_board_bytes).await?;
    validate_zip(&bytes)?;
    let dir = s.cfg.boards_dir();
    let v =
        s.db.call(move |c| {
            let t = now();
            let (b, role) = visible(c, &id, &uid)?;
            if role == "view" {
                return Err(ApiError::forbidden("Du darfst dieses Board nur ansehen."));
            }
            let lic =
                db::active_license_of(c, &b.owner_id, t)?.ok_or_else(|| ApiError::license_required(license_msg_owner(role == "owner")))?;
            if b.version != want {
                return Err(ApiError::conflict("Das Board wurde zwischenzeitlich geändert.").with_extra(json!({"version": b.version})));
            }
            let used = db::used_bytes(c, &b.owner_id)? - b.size + bytes.len() as i64;
            if used > lic.quota_mb * 1_048_576 {
                return Err(ApiError::quota(format!("Der Speicherplatz ({} MB) ist voll.", lic.quota_mb)));
            }
            write_atomic(&board_path(&dir, &id), &bytes)?;
            c.execute("UPDATE boards SET version=version+1, size=?2, modified=?3 WHERE id=?1", params![id, bytes.len() as i64, t])?;
            let b = board_row(c, &id)?.ok_or_else(|| ApiError::internal("Board fehlt nach dem Speichern"))?;
            Ok(info(&b, &role))
        })
        .await?;
    Ok(Json(v))
}

#[derive(Deserialize)]
pub struct RenameBody {
    name: String,
}

pub async fn rename(State(s): State<AppState>, a: Auth, Path(id): Path<String>, J(b): J<RenameBody>) -> ApiResult<Json<Value>> {
    let name = b.name.trim().chars().take(200).collect::<String>();
    if name.is_empty() {
        return Err(ApiError::invalid("Der Name darf nicht leer sein."));
    }
    let uid = a.user.id;
    let v =
        s.db.call(move |c| {
            let (row, role) = visible(c, &id, &uid)?;
            if role != "owner" {
                return Err(ApiError::forbidden("Nur die Besitzerin oder der Besitzer darf umbenennen."));
            }
            if db::active_license_of(c, &uid, now())?.is_none() {
                return Err(ApiError::license_required(license_msg_owner(true)));
            }
            c.execute("UPDATE boards SET name=?2 WHERE id=?1", params![id, name])?;
            let b = board_row(c, &row.id)?.ok_or_else(|| ApiError::internal("Board fehlt"))?;
            Ok(info(&b, "owner"))
        })
        .await?;
    Ok(Json(v))
}

pub async fn delete(State(s): State<AppState>, a: Auth, Path(id): Path<String>) -> ApiResult<Json<Value>> {
    let uid = a.user.id;
    let dir = s.cfg.boards_dir();
    s.db.call(move |c| {
        let (_, role) = visible(c, &id, &uid)?;
        if role != "owner" {
            return Err(ApiError::forbidden("Nur die Besitzerin oder der Besitzer darf das Board löschen."));
        }
        c.execute("DELETE FROM boards WHERE id=?1", [&id])?;
        let _ = std::fs::remove_file(board_path(&dir, &id));
        Ok(id)
    })
    .await
    .map(|id| s.hub.kick(&id, None, "not_found", "Das Board wurde gelöscht."))?;
    Ok(Json(json!({})))
}

// ---------- Mitglieder ----------

fn members_json(c: &Connection, b: &BoardRow) -> ApiResult<Value> {
    let owner = db::user_by_id(c, &b.owner_id)?.ok_or_else(|| ApiError::internal("Besitzer fehlt"))?;
    let mut out = vec![json!({"user_id": owner.id, "email": owner.email, "name": owner.name, "role": "owner", "guest": false})];
    // Gäste (per Einladungslink) ohne E-Mail, nach den Konten
    let mut st = c.prepare(
        "SELECT u.id,u.email,u.name,m.role,u.guest FROM members m JOIN users u ON u.id=m.user_id WHERE m.board_id=?1 ORDER BY u.guest, u.email, u.name",
    )?;
    let rows = st.query_map([&b.id], |r| {
        let guest = r.get::<_, i64>(4)? != 0;
        let email = if guest { String::new() } else { r.get::<_, String>(1)? };
        Ok(json!({"user_id": r.get::<_, String>(0)?, "email": email, "name": r.get::<_, String>(2)?, "role": r.get::<_, String>(3)?, "guest": guest}))
    })?;
    for r in rows {
        out.push(r?);
    }
    Ok(Value::Array(out))
}

pub async fn members(State(s): State<AppState>, a: Auth, Path(id): Path<String>) -> ApiResult<Json<Value>> {
    let uid = a.user.id;
    let v =
        s.db.call(move |c| {
            let (b, _) = visible(c, &id, &uid)?;
            members_json(c, &b)
        })
        .await?;
    Ok(Json(v))
}

#[derive(Deserialize)]
pub struct AddMemberBody {
    email: String,
    role: String,
}

pub async fn add_member(State(s): State<AppState>, a: Auth, Path(id): Path<String>, J(body): J<AddMemberBody>) -> ApiResult<Json<Value>> {
    crate::features::check(&s, Some(&a.user), owndraw_core::features::SHARE).await?;
    if body.role != "edit" && body.role != "view" {
        return Err(ApiError::invalid("Die Rolle muss \"edit\" oder \"view\" sein."));
    }
    let email = body.email.trim().to_lowercase();
    let uid = a.user.id;
    let v =
        s.db.call(move |c| {
            let (b, role) = visible(c, &id, &uid)?;
            if role != "owner" {
                return Err(ApiError::forbidden("Nur die Besitzerin oder der Besitzer darf teilen."));
            }
            if db::active_license_of(c, &uid, now())?.is_none() {
                return Err(ApiError::license_required(license_msg_owner(true)));
            }
            let target = db::user_by_email(c, &email)?
                .filter(|u| !u.disabled && !u.guest)
                .ok_or_else(|| ApiError::not_found("Zu dieser E-Mail-Adresse gibt es kein Konto."))?;
            if target.id == uid {
                return Err(ApiError::invalid("Du bist bereits Besitzer dieses Boards."));
            }
            c.execute(
                "INSERT INTO members(board_id,user_id,role) VALUES(?1,?2,?3)
                 ON CONFLICT(board_id,user_id) DO UPDATE SET role=excluded.role",
                params![b.id, target.id, body.role],
            )?;
            Ok((members_json(c, &b)?, b.id.clone(), target.id))
        })
        .await
        .map(|(v, bid, tid)| {
            // laufende Verbindung bekommt die neue Rolle beim Wiederverbinden
            s.hub.kick(&bid, Some(&tid), "role_changed", "Deine Rolle für dieses Board wurde geändert.");
            v
        })?;
    Ok(Json(v))
}

pub async fn remove_member(State(s): State<AppState>, a: Auth, Path((id, user_id)): Path<(String, String)>) -> ApiResult<Json<Value>> {
    let uid = a.user.id;
    let v =
        s.db.call(move |c| {
            let (b, role) = visible(c, &id, &uid)?;
            // Besitzer entfernt beliebige Mitglieder; Mitglieder dürfen sich selbst entfernen.
            if role != "owner" && user_id != uid {
                return Err(ApiError::forbidden("Nur die Besitzerin oder der Besitzer darf Mitglieder entfernen."));
            }
            let n = c.execute("DELETE FROM members WHERE board_id=?1 AND user_id=?2", params![b.id, user_id])?;
            if n == 0 {
                return Err(ApiError::not_found("Dieses Mitglied gibt es nicht."));
            }
            let v = if role == "owner" {
                members_json(c, &b)?
            } else {
                // Wer sich selbst entfernt hat, sieht das Board nicht mehr.
                json!([])
            };
            Ok((v, b.id.clone(), user_id))
        })
        .await
        .map(|(v, bid, gone)| {
            s.hub.kick(&bid, Some(&gone), "removed", "Du hast keinen Zugriff mehr auf dieses Board.");
            v
        })?;
    Ok(Json(v))
}
