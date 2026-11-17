//! Formelsammlungen: Sets mit beliebigem JSON (der Server interpretiert es nicht), Teilen per Code, Abos und globale Sets.
//! Endpunkte unter `/api/formulas`. Gäste dürfen lesen und abonnieren, aber nichts anlegen oder ändern.

use crate::auth::{read_limited, Auth, AdminAuth};
use crate::db::{self, now};
use crate::error::{ApiError, ApiResult};
use crate::state::AppState;
use axum::body::Body;
use axum::extract::{FromRequest, Path, Request, State};
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::Json;
use rusqlite::{params, Connection, OptionalExtension};
use serde::de::DeserializeOwned;
use serde::Deserialize;
use serde_json::{json, value::RawValue, Value};

/// Höchstgröße der Daten eines Sets (serialisiertes JSON).
pub const MAX_DATA_BYTES: usize = 1024 * 1024;
/// Höchstens so viele eigene Sets je Nutzer.
pub const MAX_SETS: i64 = 50;
const MAX_NAME_CHARS: usize = 100;

/// JSON-Body bis 1 MiB Daten plus Rahmen (der allgemeine `J`-Extraktor ist auf 256 KiB begrenzt).
pub struct Big<T>(pub T);

impl<S: Send + Sync, T: DeserializeOwned> FromRequest<S> for Big<T> {
    type Rejection = ApiError;
    async fn from_request(req: Request, _: &S) -> Result<Self, ApiError> {
        let bytes = read_limited(Body::new(req.into_body()), (MAX_DATA_BYTES + 64 * 1024) as u64)
            .await
            .map_err(|_| ApiError::too_large("Die Daten sind größer als 1 MB."))?;
        serde_json::from_slice(&bytes).map(Big).map_err(|_| ApiError::invalid("Ungültige JSON-Daten."))
    }
}

#[derive(Deserialize)]
pub struct SetBody {
    name: String,
    data: Box<RawValue>,
}

fn check_body(b: &SetBody) -> ApiResult<String> {
    let name = b.name.trim().to_string();
    if name.is_empty() || name.chars().count() > MAX_NAME_CHARS {
        return Err(ApiError::invalid(format!("Der Name muss 1 bis {MAX_NAME_CHARS} Zeichen lang sein.")));
    }
    let data = b.data.get().to_string();
    if data.len() > MAX_DATA_BYTES {
        return Err(ApiError::too_large("Die Formelsammlung ist größer als 1 MB."));
    }
    Ok(data)
}

fn no_guests(a: &Auth) -> ApiResult<()> {
    if a.user.guest {
        return Err(ApiError::forbidden("Gäste können keine Formelsammlungen anlegen oder ändern."));
    }
    Ok(())
}

fn owner_of(c: &Connection, id: &str) -> ApiResult<String> {
    c.query_row("SELECT owner FROM formula_sets WHERE id=?1", [id], |r| r.get(0))
        .optional()?
        .ok_or_else(|| ApiError::not_found("Diese Formelsammlung gibt es nicht."))
}

fn parse(data: &str) -> Value {
    serde_json::from_str(data).unwrap_or(Value::Null)
}

pub async fn list(State(s): State<AppState>, a: Auth) -> ApiResult<Json<Value>> {
    let uid = a.user.id;
    let sets = s
        .db
        .call(move |c| {
            let mut st = c.prepare(
                "SELECT f.id, f.name, u.name, f.owner=?1, f.global, f.share_code, f.updated, f.data,
                        CASE WHEN f.owner=?1 THEN 0 WHEN EXISTS (SELECT 1 FROM formula_subs s WHERE s.set_id=f.id AND s.user_id=?1) THEN 1 ELSE 2 END AS rank
                 FROM formula_sets f JOIN users u ON u.id=f.owner
                 WHERE f.owner=?1 OR f.global=1 OR EXISTS (SELECT 1 FROM formula_subs s WHERE s.set_id=f.id AND s.user_id=?1)
                 ORDER BY rank, f.name COLLATE NOCASE, f.id",
            )?;
            let rows = st
                .query_map([&uid], |r| {
                    let mine: bool = r.get(3)?;
                    let code: Option<String> = r.get(5)?;
                    Ok(json!({
                        "id": r.get::<_, String>(0)?,
                        "name": r.get::<_, String>(1)?,
                        "owner_name": r.get::<_, String>(2)?,
                        "mine": mine,
                        "global": r.get::<_, i64>(4)? != 0,
                        "share_code": if mine { code.map(Value::String).unwrap_or(Value::Null) } else { Value::Null },
                        "updated": r.get::<_, i64>(6)?,
                        "data": parse(&r.get::<_, String>(7)?),
                    }))
                })?
                .collect::<Result<Vec<_>, _>>()?;
            Ok(rows)
        })
        .await?;
    Ok(Json(json!({ "sets": sets })))
}

pub async fn create(State(s): State<AppState>, a: Auth, Big(b): Big<SetBody>) -> ApiResult<Json<Value>> {
    no_guests(&a)?;
    let (name, data) = (b.name.trim().to_string(), check_body(&b)?);
    let uid = a.user.id;
    let (id, t) = s
        .db
        .call(move |c| {
            let n: i64 = c.query_row("SELECT COUNT(*) FROM formula_sets WHERE owner=?1", [&uid], |r| r.get(0))?;
            if n >= MAX_SETS {
                return Err(ApiError::too_many(format!("Höchstens {MAX_SETS} Formelsammlungen je Nutzer – lösche zuerst eine alte.")));
            }
            let id = db::random_hex(8);
            let t = now();
            c.execute(
                "INSERT INTO formula_sets(id,owner,name,data,global,created,updated) VALUES(?1,?2,?3,?4,0,?5,?5)",
                params![id, uid, name, data, t],
            )?;
            Ok((id, t))
        })
        .await?;
    Ok(Json(json!({"id": id, "updated": t})))
}

pub async fn update(State(s): State<AppState>, a: Auth, Path(id): Path<String>, Big(b): Big<SetBody>) -> ApiResult<Json<Value>> {
    let (name, data) = (b.name.trim().to_string(), check_body(&b)?);
    let uid = a.user.id;
    let t = s
        .db
        .call(move |c| {
            if owner_of(c, &id)? != uid {
                return Err(ApiError::forbidden("Nur die Besitzerin oder der Besitzer darf diese Formelsammlung ändern."));
            }
            let t = now();
            c.execute("UPDATE formula_sets SET name=?2, data=?3, updated=?4 WHERE id=?1", params![id, name, data, t])?;
            Ok(t)
        })
        .await?;
    Ok(Json(json!({"updated": t})))
}

/// Besitzer: Set löschen (Abos fallen per Fremdschlüssel mit weg). Sonst: Abo entfernen.
pub async fn delete(State(s): State<AppState>, a: Auth, Path(id): Path<String>) -> ApiResult<Response> {
    let uid = a.user.id;
    s.db
        .call(move |c| {
            if owner_of(c, &id)? == uid {
                c.execute("DELETE FROM formula_subs WHERE set_id=?1", [&id])?;
                c.execute("DELETE FROM formula_sets WHERE id=?1", [&id])?;
            } else {
                c.execute("DELETE FROM formula_subs WHERE set_id=?1 AND user_id=?2", params![id, uid])?;
            }
            Ok(())
        })
        .await?;
    Ok(StatusCode::NO_CONTENT.into_response())
}

fn new_share_code() -> String {
    (0..8).map(|_| db::random_code_char()).collect()
}

pub async fn share(State(s): State<AppState>, a: Auth, Path(id): Path<String>) -> ApiResult<Json<Value>> {
    no_guests(&a)?;
    let uid = a.user.id;
    let code = s
        .db
        .call(move |c| {
            if owner_of(c, &id)? != uid {
                return Err(ApiError::forbidden("Nur die Besitzerin oder der Besitzer darf diese Formelsammlung teilen."));
            }
            let cur: Option<String> = c.query_row("SELECT share_code FROM formula_sets WHERE id=?1", [&id], |r| r.get(0))?;
            if let Some(code) = cur {
                return Ok(code);
            }
            loop {
                let code = new_share_code();
                match c.execute("UPDATE formula_sets SET share_code=?2 WHERE id=?1", params![id, code]) {
                    Ok(_) => return Ok(code),
                    Err(rusqlite::Error::SqliteFailure(e, _)) if e.code == rusqlite::ErrorCode::ConstraintViolation => continue,
                    Err(e) => return Err(e.into()),
                }
            }
        })
        .await?;
    Ok(Json(json!({ "code": code })))
}

pub async fn unshare(State(s): State<AppState>, a: Auth, Path(id): Path<String>) -> ApiResult<Json<Value>> {
    let uid = a.user.id;
    s.db
        .call(move |c| {
            if owner_of(c, &id)? != uid {
                return Err(ApiError::forbidden("Nur die Besitzerin oder der Besitzer darf das Teilen beenden."));
            }
            c.execute("UPDATE formula_sets SET share_code=NULL WHERE id=?1", [&id])?;
            Ok(())
        })
        .await?;
    Ok(Json(json!({})))
}

/// (id, name, owner_name, data, owner_id) zum Code.
fn by_code(c: &Connection, code: &str) -> ApiResult<(String, String, String, String, String)> {
    c.query_row(
        "SELECT f.id, f.name, u.name, f.data, f.owner FROM formula_sets f JOIN users u ON u.id=f.owner WHERE f.share_code=?1",
        [code.trim().to_ascii_uppercase()],
        |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?, r.get(4)?)),
    )
    .optional()?
    .ok_or_else(|| ApiError::not_found("Diesen Code gibt es nicht."))
}

pub async fn preview(State(s): State<AppState>, _a: Auth, Path(code): Path<String>) -> ApiResult<Json<Value>> {
    let (id, name, owner_name, data, _) = s.db.call(move |c| by_code(c, &code)).await?;
    Ok(Json(json!({"id": id, "name": name, "owner_name": owner_name, "data": parse(&data)})))
}

pub async fn subscribe(State(s): State<AppState>, a: Auth, Path(code): Path<String>) -> ApiResult<Json<Value>> {
    let uid = a.user.id;
    let id = s
        .db
        .call(move |c| {
            let (id, _, _, _, owner) = by_code(c, &code)?;
            if owner != uid {
                c.execute("INSERT OR IGNORE INTO formula_subs(user_id,set_id,created) VALUES(?1,?2,?3)", params![uid, id, now()])?;
            }
            Ok(id)
        })
        .await?;
    Ok(Json(json!({ "id": id })))
}

#[derive(Deserialize)]
pub struct GlobalBody {
    on: bool,
}

pub async fn set_global(State(s): State<AppState>, _a: AdminAuth, Path(id): Path<String>, Big(b): Big<GlobalBody>) -> ApiResult<Json<Value>> {
    s.db
        .call(move |c| {
            owner_of(c, &id)?;
            c.execute("UPDATE formula_sets SET global=?2 WHERE id=?1", params![id, b.on as i64])?;
            Ok(())
        })
        .await?;
    Ok(Json(json!({"global": b.on})))
}
