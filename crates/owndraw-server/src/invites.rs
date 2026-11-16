//! Einladungslinks: Die Besitzerin/der Besitzer eines Boards (mit Lizenz) erzeugt Links mit Rolle und Laufzeit.
//! Wer den Link hat, tritt dem Board bei – mit Konto oder ohne als Gast. Ein Gast bekommt eine eigene ID
//! (`<Name>#<4 Zeichen>`) und eine Sitzung (Cookie, bleibt im Browser), aber kein Passwort, keine Lizenz und keine
//! eigenen Boards am Server. Im Link steht ein Zufalls-Token (192 Bit), in der DB nur dessen SHA-256.

use crate::auth::{self, hash_token, rate_check, rate_check_n, with_cookie, Auth, ClientIp, MaybeAuth, J, SESSION_SECS};
use crate::boards::{info, license_msg_owner, role_for, visible, BoardRow};
use crate::db::{self, now};
use crate::error::{ApiError, ApiResult};
use crate::state::AppState;
use axum::extract::{Path, State};
use axum::response::Response;
use axum::Json;
use rusqlite::{params, Connection, OptionalExtension};
use serde::Deserialize;
use serde_json::{json, Value};

/// Längste Laufzeit eines Links in Tagen.
pub const MAX_DAYS: i64 = 365;
/// Höchstens so viele gültige Links je Board.
pub const MAX_PER_BOARD: i64 = 20;

fn invite_json(r: &rusqlite::Row) -> rusqlite::Result<Value> {
    Ok(json!({
        "id": r.get::<_, String>(0)?,
        "role": r.get::<_, String>(1)?,
        "created": r.get::<_, i64>(2)?,
        "expires_at": r.get::<_, Option<i64>>(3)?,
        "uses": r.get::<_, i64>(4)?,
    }))
}

/// Gültige Links eines Boards (abgelaufene werden dabei gelöscht).
fn list_json(c: &Connection, board: &str) -> ApiResult<Value> {
    c.execute("DELETE FROM invites WHERE board_id=?1 AND expires_at IS NOT NULL AND expires_at<=?2", params![board, now()])?;
    let mut st = c.prepare("SELECT id,role,created,expires_at,uses FROM invites WHERE board_id=?1 ORDER BY created DESC, id")?;
    let rows = st.query_map([board], invite_json)?.collect::<Result<Vec<_>, _>>()?;
    Ok(Value::Array(rows))
}

/// Board, das der Nutzer besitzt; Links verwalten geht nur mit gültiger Lizenz (wie Teilen per E-Mail).
fn owned(c: &Connection, id: &str, uid: &str) -> ApiResult<BoardRow> {
    let (b, role) = visible(c, id, uid)?;
    if role != "owner" {
        return Err(ApiError::forbidden("Nur die Besitzerin oder der Besitzer darf Einladungslinks verwalten."));
    }
    if db::active_license_of(c, uid, now())?.is_none() {
        return Err(ApiError::license_required(license_msg_owner(true)));
    }
    Ok(b)
}

/// Gültiger Link → (Link-ID, Rolle, Board). Unbekannt, abgelaufen oder Board weg → 404.
fn resolve(c: &Connection, token: &str) -> ApiResult<(String, String, BoardRow)> {
    let gone = || ApiError::not_found("Dieser Einladungslink ist ungültig oder abgelaufen.");
    if token.len() != 48 || !token.bytes().all(|b| b.is_ascii_hexdigit()) {
        return Err(gone());
    }
    let row: Option<(String, String, String, Option<i64>)> = c
        .query_row("SELECT id,board_id,role,expires_at FROM invites WHERE token_hash=?1", [hash_token(&token.to_ascii_lowercase())], |r| {
            Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?))
        })
        .optional()?;
    let (id, board, role, expires) = row.ok_or_else(gone)?;
    if expires.is_some_and(|e| e <= now()) {
        return Err(gone());
    }
    let b = crate::boards::board_row(c, &board)?.ok_or_else(gone)?;
    Ok((id, role, b))
}

// ---------- Besitzer ----------

pub async fn list(State(s): State<AppState>, a: Auth, Path(id): Path<String>) -> ApiResult<Json<Value>> {
    let uid = a.user.id;
    let v = s.db.call(move |c| list_json(c, &owned(c, &id, &uid)?.id)).await?;
    Ok(Json(v))
}

fn seven() -> Option<i64> {
    Some(7)
}

#[derive(Deserialize)]
pub struct CreateBody {
    role: String,
    /// Laufzeit in Tagen; fehlt → 7, `null` → unbefristet.
    #[serde(default = "seven")]
    days: Option<i64>,
    /// Alternativ Laufzeit in Stunden (hat Vorrang vor `days`), 1 bis 365 × 24.
    #[serde(default)]
    hours: Option<i64>,
}

pub async fn create(State(s): State<AppState>, a: Auth, Path(id): Path<String>, J(b): J<CreateBody>) -> ApiResult<Json<Value>> {
    if b.role != "edit" && b.role != "view" {
        return Err(ApiError::invalid("Die Rolle muss \"edit\" oder \"view\" sein."));
    }
    if b.days.is_some_and(|d| !(1..=MAX_DAYS).contains(&d)) {
        return Err(ApiError::invalid(format!("Die Laufzeit muss zwischen 1 und {MAX_DAYS} Tagen liegen (oder leer für unbefristet).")));
    }
    if b.hours.is_some_and(|h| !(1..=MAX_DAYS * 24).contains(&h)) {
        return Err(ApiError::invalid(format!("Die Laufzeit muss zwischen 1 und {} Stunden liegen.", MAX_DAYS * 24)));
    }
    let secs = match b.hours {
        Some(h) => Some(h * 3600),
        None => b.days.map(|d| d * 86400),
    };
    let uid = a.user.id;
    let v =
        s.db.call(move |c| {
            let board = owned(c, &id, &uid)?;
            list_json(c, &board.id)?; // abgelaufene aufräumen
            let n: i64 = c.query_row("SELECT COUNT(*) FROM invites WHERE board_id=?1", [&board.id], |r| r.get(0))?;
            if n >= MAX_PER_BOARD {
                return Err(ApiError::conflict(format!("Höchstens {MAX_PER_BOARD} Einladungslinks je Board – lösche zuerst einen alten.")));
            }
            let token = db::random_hex(24);
            let iid = db::random_hex(8);
            let t = now();
            let expires = secs.map(|s| t + s);
            c.execute(
                "INSERT INTO invites(id,token_hash,board_id,role,created,expires_at,uses) VALUES(?1,?2,?3,?4,?5,?6,0)",
                params![iid, hash_token(&token), board.id, b.role, t, expires],
            )?;
            Ok(json!({"id": iid, "token": token, "role": b.role, "created": t, "expires_at": expires, "uses": 0}))
        })
        .await?;
    Ok(Json(v))
}

pub async fn delete(State(s): State<AppState>, a: Auth, Path((id, iid)): Path<(String, String)>) -> ApiResult<Json<Value>> {
    let uid = a.user.id;
    let v =
        s.db.call(move |c| {
            let board = owned(c, &id, &uid)?;
            if c.execute("DELETE FROM invites WHERE id=?1 AND board_id=?2", params![iid, board.id])? == 0 {
                return Err(ApiError::not_found("Diesen Einladungslink gibt es nicht."));
            }
            list_json(c, &board.id)
        })
        .await?;
    Ok(Json(v))
}

// ---------- Eingeladene ----------

/// `GET /api/invites/{token}`: Was hinter dem Link steckt (ohne Anmeldung, für den Beitreten-Dialog).
pub async fn show(State(s): State<AppState>, ClientIp(ip): ClientIp, Path(token): Path<String>) -> ApiResult<Json<Value>> {
    rate_check_n(&s, "invite", &ip, s.cfg.rate_limit.saturating_mul(5))?;
    let v =
        s.db.call(move |c| {
            let (_, role, b) = resolve(c, &token)?;
            Ok(json!({"board": {"id": b.id, "name": b.name}, "owner": {"name": b.owner_name}, "role": role}))
        })
        .await?;
    Ok(Json(v))
}

#[derive(Deserialize)]
pub struct JoinBody {
    /// Nur ohne Anmeldung: gewünschter Name der Gast-ID.
    #[serde(default)]
    name: String,
}

/// `POST /api/invites/{token}/join`: Board beitreten. Angemeldet → Mitglied mit der Rolle des Links (eine
/// bestehende `edit`-Rolle bleibt). Nicht angemeldet → neuer Gast mit eigener ID und Sitzungs-Cookie.
pub async fn join(State(s): State<AppState>, ClientIp(ip): ClientIp, cur: MaybeAuth, Path(token): Path<String>, J(b): J<JoinBody>) -> ApiResult<Response> {
    rate_check_n(&s, "join", &ip, s.cfg.rate_limit.saturating_mul(5))?;
    let user = cur.0.map(|a| a.user);
    let guest_name = match &user {
        Some(_) => None,
        None => {
            // neue Gäste zählen wie Registrierungen
            rate_check(&s, "register", &ip)?;
            Some(db::clean_guest_name(&b.name)?)
        }
    };
    let (body, cookie, kick) =
        s.db.call(move |c| {
            let tx = c.transaction()?;
            let t = now();
            let (iid, role, board) = resolve(&tx, &token)?;
            if db::active_license_of(&tx, &board.owner_id, t)?.is_none() {
                return Err(ApiError::license_required("Die Lizenz der Besitzerin oder des Besitzers ist abgelaufen; Beitreten ist gerade nicht möglich."));
            }
            let (user, cookie) = match user {
                Some(u) => (u, None),
                None => {
                    db::purge_guests(&tx, t)?;
                    let g = db::insert_guest(&tx, guest_name.as_deref().unwrap_or("Gast"))?;
                    let token = auth::insert_session(&tx, &g.id)?;
                    (g, Some(token))
                }
            };
            let before = role_for(&tx, &board, &user.id)?;
            let mut changed = false;
            if before.as_deref() != Some("owner") {
                // eine bestehende edit-Rolle wird durch einen view-Link nicht herabgestuft
                let new = if before.as_deref() == Some("edit") { "edit" } else { role.as_str() };
                if before.as_deref() != Some(new) {
                    tx.execute(
                        "INSERT INTO members(board_id,user_id,role) VALUES(?1,?2,?3)
                         ON CONFLICT(board_id,user_id) DO UPDATE SET role=excluded.role",
                        params![board.id, user.id, new],
                    )?;
                    changed = before.is_some();
                }
            }
            tx.execute("UPDATE invites SET uses=uses+1 WHERE id=?1", [&iid])?;
            let role = role_for(&tx, &board, &user.id)?.unwrap_or_else(|| "view".into());
            let body = json!({"me": db::me_json(&tx, &user)?, "board": info(&board, &role)});
            tx.commit()?;
            Ok((body, cookie, changed.then(|| (board.id.clone(), user.id.clone()))))
        })
        .await?;
    if let Some((bid, uid)) = kick {
        // laufende Verbindung bekommt die neue Rolle beim Wiederverbinden
        s.hub.kick(&bid, Some(&uid), "role_changed", "Deine Rolle für dieses Board wurde geändert.");
    }
    Ok(match cookie {
        Some(t) => with_cookie(&t, SESSION_SECS, body),
        None => axum::response::IntoResponse::into_response(Json(body)),
    })
}
