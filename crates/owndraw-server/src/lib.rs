//! OwnDraw-Server: JSON-API, Admin-Panel und Auslieferung der Web-App (Vertrag: `docs/web-api.md`).

pub mod admin;
pub mod auth;
pub mod bugreports;
pub mod boards;
pub mod collab;
pub mod config;
pub mod db;
pub mod downloads;
pub mod error;
pub mod invites;
pub mod notices;
pub mod state;
pub mod totp;
pub mod stats;
pub mod update;

use axum::extract::{DefaultBodyLimit, Request, State};
use axum::http::header::{CACHE_CONTROL, CONTENT_SECURITY_POLICY, CONTENT_TYPE};
use axum::http::HeaderValue;
use axum::middleware::{self, Next};
use axum::response::{Html, IntoResponse, Response};
use axum::routing::{get, patch, post};
use axum::{Json, Router};
use error::ApiError;
use serde_json::json;
use state::AppState;
use tower_http::compression::predicate::{DefaultPredicate, NotForContentType, Predicate};
use tower_http::compression::CompressionLayer;
use tower_http::limit::RequestBodyLimitLayer;
use tower_http::services::ServeDir;

/// Admin-Panel (eingebettet, kein Build-Schritt, keine externen Ressourcen).
const ADMIN_HTML: &str = include_str!("admin.html");

/// Legt Datenordner und Datenbank an und liefert den Zustand.
pub fn open_state(cfg: config::Config) -> Result<AppState, String> {
    std::fs::create_dir_all(cfg.boards_dir()).map_err(|e| format!("Datenordner {}: {e}", cfg.data.display()))?;
    let db = db::Db::open(&cfg.db_path()).map_err(|e| format!("Datenbank: {e}"))?;
    Ok(AppState::new(db, cfg))
}

/// Legt beim Start einen Admin an, wenn es noch keinen gibt und `OWNDRAW_ADMIN_EMAIL`/`_PASSWORD` gesetzt sind.
pub async fn ensure_admin_from_env(state: &AppState) -> Result<(), String> {
    let (Ok(email), Ok(pw)) = (std::env::var("OWNDRAW_ADMIN_EMAIL"), std::env::var("OWNDRAW_ADMIN_PASSWORD")) else {
        return Ok(());
    };
    if email.trim().is_empty() || pw.is_empty() {
        return Ok(());
    }
    let has_admin = state
        .db
        .call(|c| Ok(c.query_row("SELECT COUNT(*) FROM users WHERE admin=1", [], |r| r.get::<_, i64>(0))? > 0))
        .await
        .map_err(|e| e.message)?;
    if has_admin {
        return Ok(());
    }
    auth::create_admin(&state.db, &email, &pw).await.map_err(|e| e.message)?;
    eprintln!("Admin {email} angelegt.");
    Ok(())
}

async fn health(State(state): State<AppState>) -> Json<serde_json::Value> {
    // Version der ausgelieferten Web-App (von build-web.sh in version.txt geschrieben; fehlt bei alten Builds).
    let web = std::fs::read_to_string(state.cfg.web.join("version.txt")).map(|v| v.trim().to_string()).unwrap_or_else(|_| "unbekannt".into());
    let maintenance = notices::current(&state).await.is_ok_and(|m| m.on(db::now()));
    Json(json!({"ok": true, "version": update::CURRENT, "web": web, "maintenance": maintenance}))
}

/// Web-App-Dateien (index.html, .js, .wasm) haben feste Namen: der Browser muss sie bei jedem Laden
/// neu prüfen (ETag/Last-Modified -> 304), sonst bleibt nach einem Update die alte Version im Cache.
async fn static_no_cache(req: Request, next: Next) -> Response {
    let mut r = next.run(req).await;
    r.headers_mut().entry(CACHE_CONTROL).or_insert(HeaderValue::from_static("no-cache"));
    r
}

async fn api_not_found() -> ApiError {
    ApiError::not_found("Diesen Pfad gibt es nicht.")
}

async fn admin_page() -> Response {
    let mut r = Html(ADMIN_HTML).into_response();
    let h = r.headers_mut();
    h.insert(CACHE_CONTROL, HeaderValue::from_static("no-store"));
    h.insert(
        CONTENT_SECURITY_POLICY,
        HeaderValue::from_static("default-src 'none'; script-src 'unsafe-inline'; style-src 'unsafe-inline'; connect-src 'self'; img-src data: 'self'; base-uri 'none'; form-action 'none'; frame-ancestors 'none'"),
    );
    r
}

async fn security_headers(req: Request, next: Next) -> Response {
    let mut r = next.run(req).await;
    let h = r.headers_mut();
    h.insert("x-content-type-options", HeaderValue::from_static("nosniff"));
    h.insert("referrer-policy", HeaderValue::from_static("same-origin"));
    r
}

/// Übergroße Anfragen (von der Body-Begrenzung) im JSON-Fehlerformat beantworten.
async fn json_413(res: Response) -> Response {
    let is_json = res.headers().get(CONTENT_TYPE).and_then(|v| v.to_str().ok()).is_some_and(|v| v.starts_with("application/json"));
    if res.status() == axum::http::StatusCode::PAYLOAD_TOO_LARGE && !is_json {
        return ApiError::too_large("Die Anfrage ist zu groß.").into_response();
    }
    res
}

pub fn build_app(state: AppState) -> Router {
    let api = Router::new()
        .route("/health", get(health))
        .route("/auth/register", post(auth::register))
        .route("/auth/login", post(auth::login))
        .route("/auth/logout", post(auth::logout))
        .route("/auth/2fa/setup", post(totp::setup))
        .route("/auth/2fa/enable", post(totp::enable))
        .route("/auth/2fa/verify", post(totp::verify))
        .route("/admin/2fa/recovery-codes", post(totp::regenerate))
        .route("/me", get(auth::me))
        .route("/me/password", post(auth::change_password))
        .route("/license/redeem", post(auth::redeem))
        .route("/boards", get(boards::list).post(boards::create))
        .route("/boards/{id}", get(boards::get).put(boards::put).patch(boards::rename).delete(boards::delete))
        .route("/boards/{id}/members", get(boards::members).post(boards::add_member))
        .route("/boards/{id}/members/{user_id}", axum::routing::delete(boards::remove_member))
        .route("/boards/{id}/invites", get(invites::list).post(invites::create))
        .route("/boards/{id}/invites/{invite_id}", axum::routing::delete(invites::delete))
        .route("/invites/{token}", get(invites::show))
        .route("/invites/{token}/join", post(invites::join))
        .route("/admin/users", get(admin::users))
        .route("/admin/users/{id}", patch(admin::patch_user).delete(admin::delete_user))
        .route("/admin/users/{id}/license", post(admin::user_license))
        .route("/admin/licenses", get(admin::licenses).post(admin::create_licenses))
        .route("/admin/licenses/{code}", patch(admin::patch_license).delete(admin::delete_license))
        .route("/admin/stats", get(stats::get))
        .route("/notices", get(notices::public_list))
        .route("/notices/{id}/board.png", get(notices::public_board))
        .route("/maintenance", get(notices::public_state))
        .route("/admin/boards", get(notices::admin_boards))
        .route("/admin/boards/{id}/preview.png", get(notices::admin_board_preview))
        .route("/admin/notices", get(notices::admin_list).post(notices::create))
        .route("/admin/notices/order", axum::routing::put(notices::reorder))
        .route("/admin/notices/{id}", patch(notices::patch).delete(notices::delete))
        .route("/admin/maintenance", get(notices::admin_state).put(notices::put_state))
        .route("/admin/update", get(update::get_state).patch(update::patch_state))
        .route("/admin/update/check", post(update::post_check))
        .route("/admin/update/install", post(update::post_install))
        .route("/bugreports", post(bugreports::create).layer(DefaultBodyLimit::max(bugreports::MAX_REQUEST_BYTES)))
        .route("/admin/bugreports", get(bugreports::admin_list))
        .route("/admin/bugreports/{id}", patch(bugreports::patch).delete(bugreports::delete))
        .route("/admin/bugreports/{id}/images/{n}", get(bugreports::image))
        .route("/boards/{id}/images/{name}", get(collab::get_image).put(collab::put_image))
        .route("/collab/{board_id}", get(collab::ws))
        .fallback(api_not_found)
        .layer(middleware::from_fn_with_state(state.clone(), notices::api_guard))
        .layer(middleware::from_fn(auth::csrf))
        .with_state(state.clone());

    let body_limit = (state.cfg.max_board_bytes as usize + 1024 * 1024).max(bugreports::MAX_REQUEST_BYTES);
    Router::new()
        .nest("/api", api)
        .route("/admin", get(admin_page))
        .route("/admin/", get(admin_page))
        // Anleitung der Web-App (öffnet die App direkt auf der Seite „Anleitung & Tipps“)
        .route("/tutorial", get(|| async { axum::response::Redirect::to("/#tutorial") }))
        .route("/anleitung", get(|| async { axum::response::Redirect::to("/#tutorial") }))
        .merge(
            Router::new()
                .route("/download", get(downloads::page))
                .route("/download/", get(downloads::page))
                .route("/download/{name}", get(downloads::file))
                .with_state(state.clone()),
        )
        .fallback_service(
            Router::new()
                .fallback_service(ServeDir::new(&state.cfg.web).append_index_html_on_directories(true))
                .layer(middleware::from_fn(static_no_cache))
                .layer(middleware::from_fn_with_state(state.clone(), notices::static_guard)),
        )
        .layer(RequestBodyLimitLayer::new(body_limit))
        .layer(middleware::map_response(json_413))
        .layer(CompressionLayer::new().compress_when(DefaultPredicate::new().and(NotForContentType::new("application/octet-stream")).and(NotForContentType::new("application/vnd.android.package-archive"))))
        .layer(middleware::from_fn(security_headers))
}
