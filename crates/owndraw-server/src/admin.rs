//! Admin-API: Nutzer und Lizenzen (`/api/admin/users`, `/api/admin/licenses`).

use crate::auth::{AdminAuth, J};
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
    admin: Option<bool>,
}

pub async fn patch_user(State(s): State<AppState>, a: AdminAuth, Path(id): Path<String>, J(b): J<PatchUser>) -> ApiResult<Json<Value>> {
    let me = a.0.user.id;
    let v =
        s.db.call(move |c| {
            let u = db::user_by_id(c, &id)?.ok_or_else(|| ApiError::not_found("Nutzer nicht gefunden."))?;
            if u.id == me && (b.disabled == Some(true) || b.admin == Some(false)) {
                return Err(ApiError::invalid("Du kannst dich nicht selbst sperren oder deine Admin-Rechte entziehen."));
            }
            if let Some(d) = b.disabled {
                c.execute("UPDATE users SET disabled=?2 WHERE id=?1", params![id, d as i64])?;
                if d {
                    c.execute("DELETE FROM sessions WHERE user_id=?1", [&id])?;
                }
            }
            if u.guest && b.admin == Some(true) {
                return Err(ApiError::invalid("Gäste können keine Admins werden."));
            }
            if let Some(ad) = b.admin {
                c.execute("UPDATE users SET admin=?2 WHERE id=?1", params![id, ad as i64])?;
            }
            let u = db::user_by_id(c, &id)?.ok_or_else(|| ApiError::not_found("Nutzer nicht gefunden."))?;
            user_json(c, &u)
        })
        .await?;
    Ok(Json(v))
}

pub async fn delete_user(State(s): State<AppState>, a: AdminAuth, Path(id): Path<String>) -> ApiResult<Json<Value>> {
    let me = a.0.user.id;
    let dir = s.cfg.boards_dir();
    s.db.call(move |c| {
        if id == me {
            return Err(ApiError::invalid("Du kannst dein eigenes Konto nicht löschen."));
        }
        db::user_by_id(c, &id)?.ok_or_else(|| ApiError::not_found("Nutzer nicht gefunden."))?;
        let mut st = c.prepare("SELECT id FROM boards WHERE owner_id=?1")?;
        let boards = st.query_map([&id], |r| r.get::<_, String>(0))?.collect::<Result<Vec<_>, _>>()?;
        drop(st);
        // Lizenzen des Nutzers werden widerrufen (Code bleibt zur Nachvollziehbarkeit erhalten).
        c.execute("UPDATE licenses SET revoked=1, note=note || ' (Nutzer gelöscht)' WHERE redeemed_by=?1", [&id])?;
        c.execute("DELETE FROM users WHERE id=?1", [&id])?;
        for b in boards {
            let _ = std::fs::remove_file(dir.join(format!("{b}.owndraw")));
        }
        Ok(())
    })
    .await?;
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
