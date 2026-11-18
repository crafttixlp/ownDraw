//! Freigaben: Der Admin sperrt Werkzeuge und Funktionen für alle Nutzer (Katalog `owndraw_core::features`).
//! Gespeichert als JSON-Liste in `settings` unter `features_off`. Admins sind nie betroffen.
//!
//! - `GET /api/features` (öffentlich, auch Gäste): `{"off": [...]}` – für angemeldete Admins immer leer.
//! - `GET /api/admin/features`: Katalog + gesperrte Kennungen; `PUT /api/admin/features` `{"off": [...]}`.
//! - Serverseitig geprüft ([`check`]): Teilen (Mitglieder, Einladungslinks, Beitreten), Formelsammlung
//!   (eigene Formeln anlegen/abonnieren), Fehler melden. Werkzeuge blendet nur die App aus.

use crate::auth::{AdminAuth, J, MaybeAuth};
use crate::db::{self, UserRow};
use crate::error::{ApiError, ApiResult};
use crate::state::AppState;
use axum::extract::State;
use axum::http::header::CACHE_CONTROL;
use axum::http::HeaderValue;
use axum::response::{IntoResponse, Response};
use axum::Json;
use owndraw_core::features::{self, ALL};
use serde::Deserialize;
use serde_json::{json, Value};

const KEY: &str = "features_off";

/// Gesperrte Kennungen (bereinigt; unbekannte fallen weg).
pub async fn off(s: &AppState) -> ApiResult<Vec<String>> {
    let raw = s.db.call(|c| Ok(db::get_setting(c, KEY)?)).await?;
    let v: Vec<String> = raw.and_then(|r| serde_json::from_str(&r).ok()).unwrap_or_default();
    Ok(features::clean(&v).unwrap_or_default())
}

/// Fehler, wenn `id` für diesen Nutzer gesperrt ist (Admins nie).
pub async fn check(s: &AppState, user: Option<&UserRow>, id: &str) -> ApiResult<()> {
    if user.is_some_and(|u| u.admin) || !off(s).await?.iter().any(|x| x == id) {
        return Ok(());
    }
    let name = features::get(id).map(|f| f.name).unwrap_or(id);
    Err(ApiError::feature_off(format!("„{name}“ ist auf diesem Server gerade ausgeschaltet.")))
}

fn no_store(v: Value) -> Response {
    let mut r = Json(v).into_response();
    r.headers_mut().insert(CACHE_CONTROL, HeaderValue::from_static("no-store"));
    r
}

/// Öffentlich: gesperrte Kennungen für die App.
pub async fn public(State(s): State<AppState>, who: MaybeAuth) -> ApiResult<Response> {
    let admin = who.0.as_ref().is_some_and(|a| a.user.admin);
    let off = if admin { Vec::new() } else { off(&s).await? };
    Ok(no_store(json!({ "off": off })))
}

fn admin_json(off: &[String]) -> Value {
    let catalog: Vec<Value> = ALL.iter().map(|f| json!({ "id": f.id, "name": f.name, "kind": f.kind.as_str(), "desc": f.desc })).collect();
    json!({ "off": off, "catalog": catalog })
}

pub async fn admin_get(State(s): State<AppState>, _a: AdminAuth) -> ApiResult<Json<Value>> {
    Ok(Json(admin_json(&off(&s).await?)))
}

#[derive(Deserialize)]
pub struct PutBody {
    off: Vec<String>,
}

pub async fn admin_put(State(s): State<AppState>, _a: AdminAuth, J(b): J<PutBody>) -> ApiResult<Json<Value>> {
    let off = features::clean(&b.off).ok_or_else(|| ApiError::invalid("Mindestens ein Werkzeug muss für Nutzer frei bleiben."))?;
    let raw = serde_json::to_string(&off).map_err(ApiError::internal)?;
    s.db.call(move |c| Ok(db::set_setting(c, KEY, &raw)?)).await?;
    Ok(Json(admin_json(&off)))
}
