//! Admin-API: Nutzer und Lizenzen (`/api/admin/users`, `/api/admin/licenses`).

use crate::auth::{AdminAuth, J};
use crate::boards::board_path;
use crate::db::{self, now, UserRow};
use crate::error::{ApiError, ApiResult};
use crate::state::AppState;
use axum::extract::{Path, State};
use axum::Json;
use rusqlite::{params, Connection};
use serde::{Deserialize, Deserializer};
use serde_json::{json, Value};

fn user_json(c: &Connection, u: &UserRow) -> ApiResult<Value> {
    let t = now();
    let boards: i64 = c.query_row("SELECT COUNT(*) FROM boards WHERE owner_id=?1", [&u.id], |r| r.get(0))?;
    Ok(json!({
        "id": u.id,
        "email": if u.guest { "" } else { u.email.as_str() },
        "name": u.name,
        "admin": u.admin,
        "guest": u.guest,
        "disabled": u.disabled,
        "disabled_msg": if u.disabled { db::disabled_msg(c, &u.id)? } else { String::new() },
        "created": u.created,
        "license": db::license_of(c, &u.id)?.map(|l| l.to_json(t)),
        "used_mb": db::mb(db::used_bytes(c, &u.id)?),
        "boards": boards,
    }))
}

pub async fn users(State(s): State<AppState>, _a: AdminAuth) -> ApiResult<Json<Value>> {
    let v =
        s.db.call(|c| {
            let mut st = c.prepare("SELECT id FROM users ORDER BY created, email")?;
            let ids = st.query_map([], |r| r.get::<_, String>(0))?.collect::<Result<Vec<_>, _>>()?;
            let mut out = Vec::new();
            for id in ids {
                if let Some(u) = db::user_by_id(c, &id)? {
                    out.push(user_json(c, &u)?);
                }
            }
            Ok(Value::Array(out))
        })
        .await?;
    Ok(Json(v))
}

#[derive(Deserialize)]
pub struct PatchUser {
    disabled: Option<bool>,
    /// Nachricht an den Nutzer beim Sperren (optional).
    disabled_msg: Option<String>,
    admin: Option<bool>,
}

pub async fn patch_user(State(s): State<AppState>, a: AdminAuth, Path(id): Path<String>, J(b): J<PatchUser>) -> ApiResult<Json<Value>> {
    let me = a.0.user.id;
    let (v, kicked) =
        s.db.call(move |c| {
            let u = db::user_by_id(c, &id)?.ok_or_else(|| ApiError::not_found("Nutzer nicht gefunden."))?;
            if u.id == me && (b.disabled == Some(true) || b.admin == Some(false)) {
                return Err(ApiError::invalid("Du kannst dich nicht selbst sperren oder deine Admin-Rechte entziehen."));
            }
            if u.guest && b.admin == Some(true) {
                return Err(ApiError::invalid("Gäste können keine Admins werden."));
            }
            let tx = c.transaction()?;
            let mut kicked = false;
            if let Some(d) = b.disabled {
                // Sperren löscht nichts: Boards und Lizenz bleiben, Entsperren stellt alles wieder her.
                // Sitzungen bleiben bis zum nächsten Aufruf (dann Meldung + Abmeldung, siehe `Auth`).
                let msg: Option<String> = if d { b.disabled_msg.as_deref().map(str::trim).filter(|m| !m.is_empty()).map(|m| m.chars().take(1000).collect()) } else { None };
                tx.execute("UPDATE users SET disabled=?2, disabled_msg=?3 WHERE id=?1", params![id, d as i64, msg])?;
                kicked = d;
            }
            if let Some(ad) = b.admin {
                tx.execute("UPDATE users SET admin=?2 WHERE id=?1", params![id, ad as i64])?;
            }
            let u = db::user_by_id(&tx, &id)?.ok_or_else(|| ApiError::not_found("Nutzer nicht gefunden."))?;
            let v = user_json(&tx, &u)?;
            tx.commit()?;
            Ok((v, kicked.then_some(u.id)))
        })
        .await?;
    if let Some(uid) = kicked {
        // Offene Live-Verbindungen enden; der Client holt sich dann die Sperrmeldung ab.
        s.hub.kick_user(&uid, "account_disabled", "Dein Konto ist gesperrt.");
    }
    Ok(Json(v))
}

/// Löscht ein Konto samt eigenen Boards (Dateien) und Mitgliedschaften; Lizenzen werden widerrufen (Code bleibt
/// zur Nachvollziehbarkeit erhalten). Liefert die gelöschten Boards (für `hub.kick`). Admin und Selbstlöschung.
pub(crate) fn remove_user(c: &Connection, dir: &std::path::Path, id: &str) -> rusqlite::Result<Vec<String>> {
    let boards = owned_boards(c, id)?;
    c.execute("UPDATE licenses SET revoked=1, note=note || ' (Nutzer gelöscht)' WHERE redeemed_by=?1", [id])?;
    c.execute("DELETE FROM users WHERE id=?1", [id])?;
    for b in &boards {
        let _ = std::fs::remove_file(board_path(dir, b));
    }
    Ok(boards)
}

pub(crate) fn owned_boards(c: &Connection, user_id: &str) -> rusqlite::Result<Vec<String>> {
    let mut st = c.prepare("SELECT id FROM boards WHERE owner_id=?1")?;
    let ids = st.query_map([user_id], |r| r.get::<_, String>(0))?.collect::<Result<Vec<_>, _>>()?;
    Ok(ids)
}

pub async fn delete_user(State(s): State<AppState>, a: AdminAuth, Path(id): Path<String>) -> ApiResult<Json<Value>> {
    let me = a.0.user.id;
    let dir = s.cfg.boards_dir();
    let uid = id.clone();
    let boards = s
        .db
        .call(move |c| {
            if id == me {
                return Err(ApiError::invalid("Du kannst dein eigenes Konto nicht löschen."));
            }
            db::user_by_id(c, &id)?.ok_or_else(|| ApiError::not_found("Nutzer nicht gefunden."))?;
            Ok(remove_user(c, &dir, &id)?)
        })
        .await?;
    for b in &boards {
        s.hub.kick(b, None, "not_found", "Das Board wurde gelöscht.");
    }
    s.hub.kick_user(&uid, "unauthorized", "Dein Konto wurde gelöscht.");
    Ok(Json(json!({})))
}

pub async fn licenses(State(s): State<AppState>, _a: AdminAuth) -> ApiResult<Json<Value>> {
    let v =
        s.db.call(|c| {
            let t = now();
            Ok(Value::Array(db::all_licenses(c)?.iter().map(|l| l.to_json(t)).collect()))
        })
        .await?;
    Ok(Json(v))
}

fn default_count() -> i64 {
    1
}
fn default_quota() -> i64 {
    500
}

#[derive(Deserialize)]
pub struct CreateLicenses {
    #[serde(default = "default_count")]
    count: i64,
    #[serde(default)]
    days: Option<i64>,
    #[serde(default = "default_quota")]
    quota_mb: i64,
    #[serde(default)]
    note: String,
}

pub async fn create_licenses(State(s): State<AppState>, _a: AdminAuth, J(b): J<CreateLicenses>) -> ApiResult<Json<Value>> {
    if !(1..=100).contains(&b.count) {
        return Err(ApiError::invalid("Die Anzahl muss zwischen 1 und 100 liegen."));
    }
    if b.days.is_some_and(|d| !(1..=36_500).contains(&d)) {
        return Err(ApiError::invalid("Die Laufzeit muss zwischen 1 und 36500 Tagen liegen (oder leer für unbegrenzt)."));
    }
    if !(1..=10_000_000).contains(&b.quota_mb) {
        return Err(ApiError::invalid("Der Speicherplatz muss mindestens 1 MB betragen."));
    }
    let note: String = b.note.trim().chars().take(500).collect();
    let v =
        s.db.call(move |c| {
            let t = now();
            let tx = c.transaction()?;
            let mut out = Vec::new();
            for _ in 0..b.count {
                let mut inserted = false;
                for _ in 0..5 {
                    let code = db::new_license_code();
                    let n = tx.execute(
                        "INSERT OR IGNORE INTO licenses(code,created,days,expires_at,quota_mb,note,revoked) VALUES(?1,?2,?3,NULL,?4,?5,0)",
                        params![code, t, b.days, b.quota_mb, note],
                    )?;
                    if n == 1 {
                        out.push(db::license_by_code(&tx, &code)?.ok_or_else(|| ApiError::internal("Lizenz fehlt"))?.to_json(t));
                        inserted = true;
                        break;
                    }
                }
                if !inserted {
                    return Err(ApiError::internal("Kein freier Lizenzcode gefunden"));
                }
            }
            tx.commit()?;
            Ok(Value::Array(out))
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
pub struct PatchLicense {
    revoked: Option<bool>,
    /// Fehlt = unverändert, `null` = unbegrenzt, Zahl = Unix-Sekunden.
    #[serde(default, deserialize_with = "double_option")]
    expires_at: Option<Option<i64>>,
    quota_mb: Option<i64>,
    note: Option<String>,
}

pub async fn patch_license(
    State(s): State<AppState>,
    _a: AdminAuth,
    Path(code): Path<String>,
    J(b): J<PatchLicense>,
) -> ApiResult<Json<Value>> {
    if b.quota_mb.is_some_and(|q| !(1..=10_000_000).contains(&q)) {
        return Err(ApiError::invalid("Der Speicherplatz muss mindestens 1 MB betragen."));
    }
    let code = code.trim().to_uppercase();
    let v =
        s.db.call(move |c| {
            db::license_by_code(c, &code)?.ok_or_else(|| ApiError::not_found("Lizenz nicht gefunden."))?;
            if let Some(r) = b.revoked {
                c.execute("UPDATE licenses SET revoked=?2 WHERE code=?1", params![code, r as i64])?;
            }
            if let Some(e) = b.expires_at {
                c.execute("UPDATE licenses SET expires_at=?2 WHERE code=?1", params![code, e])?;
            }
            if let Some(q) = b.quota_mb {
                c.execute("UPDATE licenses SET quota_mb=?2 WHERE code=?1", params![code, q])?;
            }
            if let Some(n) = b.note {
                let n: String = n.trim().chars().take(500).collect();
                c.execute("UPDATE licenses SET note=?2 WHERE code=?1", params![code, n])?;
            }
            let l = db::license_by_code(c, &code)?.ok_or_else(|| ApiError::not_found("Lizenz nicht gefunden."))?;
            Ok(l.to_json(now()))
        })
        .await?;
    Ok(Json(v))
}

#[derive(Deserialize)]
pub struct UserLicenseBody {
    /// `true` = Lizenz aktivieren (vorhandene reaktivieren, freien Code zuweisen oder neuen erzeugen), `false` = deaktivieren.
    active: bool,
    /// Freier Code, der dem Nutzer zugewiesen wird (nur beim Aktivieren).
    #[serde(default)]
    code: Option<String>,
    /// Laufzeit eines neu erzeugten Codes; fehlt/`null` = unbegrenzt.
    #[serde(default)]
    days: Option<i64>,
    #[serde(default = "default_quota")]
    quota_mb: i64,
}

/// Weist eine freie Lizenz einem Nutzer zu (wie Einlösen, nur durch den Admin).
fn assign_license(c: &Connection, l: &db::LicenseRow, user_id: &str, t: i64) -> ApiResult<()> {
    let expires = match l.days {
        Some(d) => Some(t + d * 86400),
        None => l.expires_at,
    };
    c.execute(
        "UPDATE licenses SET redeemed_by=?2, redeemed_at=?3, expires_at=?4, revoked=0 WHERE code=?1",
        params![l.code, user_id, t, expires],
    )?;
    Ok(())
}

/// `POST /api/admin/users/{id}/license`: Lizenz eines Nutzers aktivieren oder deaktivieren. Deaktivieren nimmt
/// die Lizenzrechte (Speichern am Server, Teilen, Live), Boards bleiben erhalten; Aktivieren stellt sie wieder her.
pub async fn user_license(
    State(s): State<AppState>,
    _a: AdminAuth,
    Path(id): Path<String>,
    J(b): J<UserLicenseBody>,
) -> ApiResult<Json<Value>> {
    if b.days.is_some_and(|d| !(1..=36_500).contains(&d)) {
        return Err(ApiError::invalid("Die Laufzeit muss zwischen 1 und 36500 Tagen liegen (oder leer für unbegrenzt)."));
    }
    if !(1..=10_000_000).contains(&b.quota_mb) {
        return Err(ApiError::invalid("Der Speicherplatz muss mindestens 1 MB betragen."));
    }
    let v =
        s.db.call(move |c| {
            let t = now();
            let tx = c.transaction()?;
            let u = db::user_by_id(&tx, &id)?.ok_or_else(|| ApiError::not_found("Nutzer nicht gefunden."))?;
            if u.guest {
                return Err(ApiError::invalid("Gäste können keine Lizenz haben."));
            }
            let current = db::license_of(&tx, &id)?;
            if !b.active {
                let l = current.filter(|l| !l.revoked).ok_or_else(|| ApiError::invalid("Dieser Nutzer hat keine aktive Lizenz."))?;
                tx.execute("UPDATE licenses SET revoked=1 WHERE code=?1", [&l.code])?;
            } else if let Some(code) = b.code.as_deref().map(|c| c.trim().to_uppercase()).filter(|c| !c.is_empty()) {
                let l = db::license_by_code(&tx, &code)?.ok_or_else(|| ApiError::not_found("Diesen Lizenzcode gibt es nicht."))?;
                match &l.redeemed_by {
                    Some((uid, _)) if *uid == id => {
                        tx.execute("UPDATE licenses SET revoked=0 WHERE code=?1", [&code])?;
                    }
                    Some(_) => return Err(ApiError::conflict("Dieser Lizenzcode gehört schon einem anderen Nutzer.")),
                    None => {
                        if l.expires_at.is_some_and(|e| e <= t) {
                            return Err(ApiError::invalid("Dieser Lizenzcode ist abgelaufen."));
                        }
                        if let Some(old) = current.filter(|o| o.active(t)) {
                            tx.execute("UPDATE licenses SET revoked=1 WHERE code=?1", [&old.code])?;
                        }
                        assign_license(&tx, &l, &id, t)?;
                    }
                }
            } else {
                match current {
                    Some(l) if l.active(t) => {}
                    // deaktivierte, nicht abgelaufene Lizenz wieder einschalten
                    Some(l) if l.expires_at.is_none_or(|e| e > t) => {
                        tx.execute("UPDATE licenses SET revoked=0 WHERE code=?1", [&l.code])?;
                    }
                    // keine oder abgelaufene Lizenz: neuen Code erzeugen und zuweisen
                    _ => {
                        let mut made = None;
                        for _ in 0..5 {
                            let code = db::new_license_code();
                            let n = tx.execute(
                                "INSERT OR IGNORE INTO licenses(code,created,days,expires_at,quota_mb,note,revoked) VALUES(?1,?2,?3,NULL,?4,'Vom Admin vergeben',0)",
                                params![code, t, b.days, b.quota_mb],
                            )?;
                            if n == 1 {
                                made = Some(code);
                                break;
                            }
                        }
                        let code = made.ok_or_else(|| ApiError::internal("Kein freier Lizenzcode gefunden"))?;
                        let l = db::license_by_code(&tx, &code)?.ok_or_else(|| ApiError::internal("Lizenz fehlt"))?;
                        assign_license(&tx, &l, &id, t)?;
                    }
                }
            }
            let u = db::user_by_id(&tx, &id)?.ok_or_else(|| ApiError::not_found("Nutzer nicht gefunden."))?;
            let v = user_json(&tx, &u)?;
            tx.commit()?;
            Ok(v)
        })
        .await?;
    Ok(Json(v))
}

/// `DELETE /api/admin/licenses/{code}`: Lizenz endgültig löschen. Ist sie einem Nutzer zugewiesen, verliert er
/// damit sofort die Lizenzrechte (wie beim Deaktivieren); seine Boards bleiben erhalten.
pub async fn delete_license(State(s): State<AppState>, _a: AdminAuth, Path(code): Path<String>) -> ApiResult<Json<Value>> {
    let code = code.trim().to_uppercase();
    s.db.call(move |c| {
        let tx = c.transaction()?;
        db::license_by_code(&tx, &code)?.ok_or_else(|| ApiError::not_found("Lizenz nicht gefunden."))?;
        tx.execute("DELETE FROM licenses WHERE code=?1", [&code])?;
        tx.commit()?;
        Ok(())
    })
    .await?;
    Ok(Json(json!({})))
}
