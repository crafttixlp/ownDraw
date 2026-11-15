//! Konto: Passwörter (Argon2), Sitzungen (Zufalls-Token, SHA-256 in der DB), CSRF-Schutz,
//! Extraktoren und die Konto-Routen (`/api/auth/*`, `/api/me*`, `/api/license/redeem`).

use crate::db::{self, now, UserRow};
use crate::error::{ApiError, ApiResult};
use crate::state::AppState;
use argon2::password_hash::{PasswordHasher, PasswordVerifier};
use argon2::Argon2;
use axum::body::{to_bytes, Body};
use axum::extract::{FromRequest, FromRequestParts, Request, State};
use axum::http::header::{COOKIE, SET_COOKIE};
use axum::http::request::Parts;
use axum::http::{HeaderMap, HeaderValue, Method};
use axum::middleware::Next;
use axum::response::{IntoResponse, Response};
use axum::Json;
use rusqlite::{params, OptionalExtension};
use serde::de::DeserializeOwned;
use serde::Deserialize;
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::net::SocketAddr;
use std::sync::OnceLock;
use std::time::Duration;

pub const COOKIE_NAME: &str = "owndraw_session";
pub const SESSION_SECS: i64 = 30 * 24 * 3600;
pub const MIN_PASSWORD: usize = 8;
const MAX_PASSWORD: usize = 1024;
const JSON_LIMIT: usize = 256 * 1024;

// ---------- Hilfen ----------

pub fn hash_token(token: &str) -> String {
    hex::encode(Sha256::digest(token.as_bytes()))
}

pub async fn hash_password(pw: String) -> ApiResult<String> {
    tokio::task::spawn_blocking(move || Argon2::default().hash_password(pw.as_bytes()).map(|h| h.to_string()).map_err(ApiError::internal))
        .await
        .map_err(ApiError::internal)?
}

pub async fn verify_password(pw: String, hash: String) -> bool {
    tokio::task::spawn_blocking(move || Argon2::default().verify_password(pw.as_bytes(), hash.as_str()).is_ok()).await.unwrap_or(false)
}

fn dummy_hash() -> &'static str {
    static H: OnceLock<String> = OnceLock::new();
    H.get_or_init(|| Argon2::default().hash_password(b"dummy-passwort").map(|h| h.to_string()).unwrap_or_default())
}

pub fn normalize_email(raw: &str) -> ApiResult<String> {
    let e = raw.trim().to_lowercase();
    let ok = e.len() <= 254
        && !e.chars().any(|c| c.is_whitespace() || c.is_control())
        && e.split_once('@')
            .is_some_and(|(l, d)| !l.is_empty() && d.contains('.') && !d.starts_with('.') && !d.ends_with('.') && !d.contains('@'));
    if ok {
        Ok(e)
    } else {
        Err(ApiError::invalid("Bitte eine gültige E-Mail-Adresse angeben."))
    }
}

pub fn check_password(pw: &str) -> ApiResult<()> {
    if pw.chars().count() < MIN_PASSWORD {
        return Err(ApiError::invalid(format!("Das Passwort muss mindestens {MIN_PASSWORD} Zeichen lang sein.")));
    }
    if pw.len() > MAX_PASSWORD {
        return Err(ApiError::invalid("Das Passwort ist zu lang."));
    }
    Ok(())
}

fn session_cookie(h: &HeaderMap) -> Option<String> {
    h.get_all(COOKIE)
        .iter()
        .filter_map(|v| v.to_str().ok())
        .flat_map(|s| s.split(';'))
        .filter_map(|p| p.trim().split_once('='))
        .find(|(k, v)| *k == COOKIE_NAME && !v.is_empty())
        .map(|(_, v)| v.to_string())
}

fn cookie_header(token: &str, max_age: i64) -> HeaderValue {
    HeaderValue::from_str(&format!("{COOKIE_NAME}={token}; HttpOnly; Secure; SameSite=Lax; Path=/; Max-Age={max_age}"))
        .expect("Cookie-Header ist gültig")
}

fn with_cookie(token: &str, max_age: i64, body: Value) -> Response {
    let mut r = Json(body).into_response();
    r.headers_mut().append(SET_COOKIE, cookie_header(token, max_age));
    r
}

pub async fn create_session(state: &AppState, user_id: String) -> ApiResult<String> {
    let token = db::random_hex(32);
    let th = hash_token(&token);
    state
        .db
        .call(move |c| {
            let t = now();
            c.execute("DELETE FROM sessions WHERE expires<?1", [t])?;
            c.execute(
                "INSERT INTO sessions(token_hash,user_id,created,expires) VALUES(?1,?2,?3,?4)",
                params![th, user_id, t, t + SESSION_SECS],
            )?;
            Ok(())
        })
        .await?;
    Ok(token)
}

/// Legt einen Admin an (oder macht einen vorhandenen Nutzer zum Admin und setzt das Passwort).
pub async fn create_admin(db: &db::Db, email: &str, password: &str) -> ApiResult<()> {
    let email = normalize_email(email)?;
    check_password(password)?;
    let hash = hash_password(password.to_string()).await?;
    db.call(move |c| {
        match db::user_by_email(c, &email)? {
            Some(u) => {
                c.execute("UPDATE users SET admin=1, disabled=0, pw_hash=?2 WHERE id=?1", params![u.id, hash])?;
            }
            None => {
                let name = email.split('@').next().unwrap_or("Admin").to_string();
                db::insert_user(c, &email, &name, &hash, true)?;
            }
        }
        Ok(())
    })
    .await
}

// ---------- Extraktoren ----------

/// JSON-Body mit eigener Fehlerbehandlung (`invalid` / `too_large`), unabhängig vom Content-Type.
pub struct J<T>(pub T);

impl<S: Send + Sync, T: DeserializeOwned> FromRequest<S> for J<T> {
    type Rejection = ApiError;
    async fn from_request(req: Request, _: &S) -> Result<Self, ApiError> {
        let bytes = to_bytes(req.into_body(), JSON_LIMIT).await.map_err(|_| ApiError::too_large("Anfrage zu groß."))?;
        serde_json::from_slice(&bytes).map(J).map_err(|_| ApiError::invalid("Ungültige JSON-Daten."))
    }
}

/// Liest den Body bis `limit` Bytes; größer → `too_large`.
pub async fn read_limited(body: Body, limit: u64) -> ApiResult<axum::body::Bytes> {
    to_bytes(body, limit as usize)
        .await
        .map_err(|_| ApiError::too_large(format!("Die Datei ist größer als das Limit von {} MB.", limit / 1024 / 1024)))
}

/// Client-Adresse (hinter Caddy aus `X-Forwarded-For`, sonst die Verbindungsadresse).
pub struct ClientIp(pub String);

impl<S: Send + Sync> FromRequestParts<S> for ClientIp {
    type Rejection = std::convert::Infallible;
    async fn from_request_parts(parts: &mut Parts, _: &S) -> Result<Self, Self::Rejection> {
        let fwd = parts
            .headers
            .get("x-forwarded-for")
            .and_then(|v| v.to_str().ok())
            .and_then(|v| v.split(',').next())
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty());
        let ip = fwd
            .or_else(|| parts.extensions.get::<axum::extract::ConnectInfo<SocketAddr>>().map(|c| c.0.ip().to_string()))
            .unwrap_or_else(|| "local".into());
        Ok(ClientIp(ip))
    }
}

/// Angemeldeter Nutzer (gültige, nicht abgelaufene Sitzung, Konto nicht gesperrt).
pub struct Auth {
    pub user: UserRow,
    pub token_hash: String,
}

impl FromRequestParts<AppState> for Auth {
    type Rejection = ApiError;
    async fn from_request_parts(parts: &mut Parts, state: &AppState) -> Result<Self, ApiError> {
        let token = session_cookie(&parts.headers).ok_or_else(ApiError::unauthorized)?;
        let th = hash_token(&token);
        let th2 = th.clone();
        let user = state
            .db
            .call(move |c| {
                let uid: Option<String> = c
                    .query_row("SELECT user_id FROM sessions WHERE token_hash=?1 AND expires>?2", params![th2, now()], |r| r.get(0))
                    .optional()?;
                match uid {
                    Some(uid) => Ok(db::user_by_id(c, &uid)?.filter(|u| !u.disabled)),
                    None => Ok(None),
                }
            })
            .await?
            .ok_or_else(ApiError::unauthorized)?;
        Ok(Auth { user, token_hash: th })
    }
}

/// Angemeldeter Admin.
pub struct AdminAuth(pub Auth);

impl FromRequestParts<AppState> for AdminAuth {
    type Rejection = ApiError;
    async fn from_request_parts(parts: &mut Parts, state: &AppState) -> Result<Self, ApiError> {
        let a = Auth::from_request_parts(parts, state).await?;
        if !a.user.admin {
            return Err(ApiError::forbidden("Nur für Administratoren."));
        }
        Ok(AdminAuth(a))
    }
}

/// CSRF-Schutz: schreibende Anfragen brauchen `X-OwnDraw: 1`.
pub async fn csrf(req: Request, next: Next) -> Response {
    let safe = matches!(*req.method(), Method::GET | Method::HEAD | Method::OPTIONS);
    if !safe && req.headers().get("x-owndraw").and_then(|v| v.to_str().ok()) != Some("1") {
        return ApiError::forbidden("Der Header X-OwnDraw: 1 fehlt.").into_response();
    }
    next.run(req).await
}

fn rate_check(state: &AppState, kind: &str, ip: &str) -> ApiResult<()> {
    let key = format!("{kind}:{ip}");
    if state.limiter.hit(&key, state.cfg.rate_limit, Duration::from_secs(state.cfg.rate_window_secs)) {
        Ok(())
    } else {
        Err(ApiError::rate_limited())
    }
}

// ---------- Routen ----------

#[derive(Deserialize)]
pub struct RegisterBody {
    email: String,
    password: String,
    #[serde(default)]
    name: String,
}

pub async fn register(State(s): State<AppState>, ClientIp(ip): ClientIp, J(b): J<RegisterBody>) -> ApiResult<Response> {
    rate_check(&s, "register", &ip)?;
    let email = normalize_email(&b.email)?;
    check_password(&b.password)?;
    let name = b.name.trim().chars().take(100).collect::<String>();
    let name = if name.is_empty() { email.split('@').next().unwrap_or("").to_string() } else { name };
    let hash = hash_password(b.password).await?;
    let (user, me) =
        s.db.call(move |c| {
            let u = db::insert_user(c, &email, &name, &hash, false)?;
            let me = db::me_json(c, &u)?;
            Ok((u, me))
        })
        .await?;
    let token = create_session(&s, user.id).await?;
    Ok(with_cookie(&token, SESSION_SECS, me))
}

#[derive(Deserialize)]
pub struct LoginBody {
    email: String,
    password: String,
}

pub async fn login(State(s): State<AppState>, ClientIp(ip): ClientIp, J(b): J<LoginBody>) -> ApiResult<Response> {
    rate_check(&s, "login", &ip)?;
    let email = b.email.trim().to_lowercase();
    let e2 = email.clone();
    let user = s.db.call(move |c| Ok(db::user_by_email(c, &e2)?)).await?;
    let bad = || ApiError::unauthorized_with("E-Mail oder Passwort ist falsch.");
    let Some(user) = user else {
        // Gleiche Rechenzeit wie bei einem echten Konto (kein Hinweis, ob die Adresse existiert).
        verify_password(b.password, dummy_hash().to_string()).await;
        return Err(bad());
    };
    if !verify_password(b.password, user.pw_hash.clone()).await {
        return Err(bad());
    }
    if user.disabled {
        return Err(ApiError::forbidden("Dieses Konto ist gesperrt."));
    }
    let u2 = user.clone();
    let me = s.db.call(move |c| db::me_json(c, &u2)).await?;
    let token = create_session(&s, user.id).await?;
    Ok(with_cookie(&token, SESSION_SECS, me))
}

pub async fn logout(State(s): State<AppState>, headers: HeaderMap) -> ApiResult<Response> {
    if let Some(token) = session_cookie(&headers) {
        let th = hash_token(&token);
        s.db.call(move |c| Ok(c.execute("DELETE FROM sessions WHERE token_hash=?1", [th])?)).await?;
    }
    Ok(with_cookie("", 0, json!({})))
}

pub async fn me(State(s): State<AppState>, a: Auth) -> ApiResult<Json<Value>> {
    Ok(Json(s.db.call(move |c| db::me_json(c, &a.user)).await?))
}

#[derive(Deserialize)]
pub struct PasswordBody {
    old: String,
    new: String,
}

pub async fn change_password(State(s): State<AppState>, a: Auth, J(b): J<PasswordBody>) -> ApiResult<Json<Value>> {
    check_password(&b.new)?;
    if !verify_password(b.old, a.user.pw_hash.clone()).await {
        return Err(ApiError::forbidden("Das bisherige Passwort ist falsch."));
    }
    let hash = hash_password(b.new).await?;
    let (uid, th) = (a.user.id.clone(), a.token_hash.clone());
    s.db.call(move |c| {
        c.execute("UPDATE users SET pw_hash=?2 WHERE id=?1", params![uid, hash])?;
        // Alle anderen Sitzungen beenden.
        c.execute("DELETE FROM sessions WHERE user_id=?1 AND token_hash<>?2", params![uid, th])?;
        Ok(())
    })
    .await?;
    Ok(Json(json!({})))
}

#[derive(Deserialize)]
pub struct RedeemBody {
    code: String,
}

pub async fn redeem(State(s): State<AppState>, ClientIp(ip): ClientIp, a: Auth, J(b): J<RedeemBody>) -> ApiResult<Json<Value>> {
    rate_check(&s, "redeem", &ip)?;
    let code = b.code.trim().to_uppercase();
    let user = a.user;
    let me =
        s.db.call(move |c| {
            let t = now();
            let tx = c.transaction()?;
            let lic = db::license_by_code(&tx, &code)?.ok_or_else(|| ApiError::not_found("Diesen Lizenzcode gibt es nicht."))?;
            match &lic.redeemed_by {
                Some((id, _)) if *id == user.id => {} // schon eingelöst: Me zurückgeben
                Some(_) => return Err(ApiError::conflict("Dieser Lizenzcode wurde bereits eingelöst.")),
                None => {
                    if lic.revoked {
                        return Err(ApiError::forbidden("Dieser Lizenzcode wurde widerrufen."));
                    }
                    if lic.expires_at.is_some_and(|e| e <= t) {
                        return Err(ApiError::forbidden("Dieser Lizenzcode ist abgelaufen."));
                    }
                    if db::active_license_of(&tx, &user.id, t)?.is_some() {
                        return Err(ApiError::conflict("Du hast bereits eine gültige Lizenz."));
                    }
                    let expires = match lic.days {
                        Some(d) => Some(t + d * 86400),
                        None => lic.expires_at,
                    };
                    tx.execute(
                        "UPDATE licenses SET redeemed_by=?2, redeemed_at=?3, expires_at=?4 WHERE code=?1",
                        params![lic.code, user.id, t, expires],
                    )?;
                }
            }
            let me = db::me_json(&tx, &user)?;
            tx.commit()?;
            Ok(me)
        })
        .await?;
    Ok(Json(me))
}
