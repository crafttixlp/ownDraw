//! Eigenes Konto (Einstellungen → „Konto“): Name und E-Mail ändern, Sitzungen auf anderen Geräten beenden,
//! Daten herunterladen (ZIP mit allen eigenen Boards) und Konto löschen. Ändern der E-Mail und Löschen verlangen
//! das aktuelle Passwort; Fehlversuche zählen je Konto (wie beim Anmelden).

use crate::auth::{verify_password, with_cookie, Auth, ClientIp, J};
use crate::db::{self, now};
use crate::error::{ApiError, ApiResult};
use crate::state::AppState;
use axum::extract::State;
use axum::http::header::{CONTENT_DISPOSITION, CONTENT_TYPE};
use axum::http::HeaderValue;
use axum::response::{IntoResponse, Response};
use axum::Json;
use rusqlite::params;
use serde::Deserialize;
use serde_json::{json, Value};
use std::io::Write;
use std::time::Duration;

/// Falsche Passwörter je Konto im Rate-Limit-Fenster (E-Mail ändern, Konto löschen, Passwort ändern).
pub(crate) const PW_FAILS_PER_ACCOUNT: u32 = 10;

fn no_guest(a: &Auth) -> ApiResult<()> {
    if a.user.guest {
        return Err(ApiError::forbidden("Als Gast hast du kein Konto. Lege dafür ein Konto an."));
    }
    Ok(())
}

/// Prüft das aktuelle Passwort; zu viele Fehlversuche → `rate_limited` (auch mit richtigem Passwort).
pub(crate) async fn check_current(s: &AppState, a: &Auth, ip: &str, pw: String) -> ApiResult<()> {
    crate::auth::rate_check(s, "account", ip)?;
    let key = format!("pw-fail:{}", a.user.id);
    let window = Duration::from_secs(s.cfg.rate_window_secs);
    if s.limiter.count(&key, window) >= PW_FAILS_PER_ACCOUNT {
        return Err(ApiError::rate_limited());
    }
    if !verify_password(pw, a.user.pw_hash.clone()).await {
        s.limiter.hit(&key, u32::MAX, window);
        return Err(ApiError::forbidden("Das aktuelle Passwort ist falsch."));
    }
    s.limiter.clear(&key);
    Ok(())
}

#[derive(Deserialize)]
pub struct NameBody {
    name: String,
}

/// `PATCH /api/me`: Anzeigename ändern.
pub async fn set_name(State(s): State<AppState>, a: Auth, J(b): J<NameBody>) -> ApiResult<Json<Value>> {
    no_guest(&a)?;
    let name = b.name.trim().chars().filter(|c| !c.is_control()).take(100).collect::<String>();
    if name.is_empty() {
        return Err(ApiError::invalid("Bitte einen Namen angeben."));
    }
    let mut u = a.user;
    let me =
        s.db.call(move |c| {
            c.execute("UPDATE users SET name=?2 WHERE id=?1", params![u.id, name])?;
            u.name = name;
            db::me_json(c, &u)
        })
        .await?;
    Ok(Json(me))
}

#[derive(Deserialize)]
pub struct EmailBody {
    email: String,
    password: String,
}

/// `POST /api/me/email`: E-Mail-Adresse ändern (mit aktuellem Passwort). Andere Sitzungen enden.
pub async fn set_email(State(s): State<AppState>, ClientIp(ip): ClientIp, a: Auth, J(b): J<EmailBody>) -> ApiResult<Json<Value>> {
    no_guest(&a)?;
    let email = crate::auth::normalize_email(&b.email)?;
    check_current(&s, &a, &ip, b.password).await?;
    let th = a.token_hash.clone();
    let mut u = a.user;
    let me =
        s.db.call(move |c| {
            if let Some(o) = db::user_by_email(c, &email)? {
                if o.id != u.id {
                    return Err(ApiError::conflict("Diese E-Mail-Adresse gehört schon zu einem anderen Konto."));
                }
            }
            c.execute("UPDATE users SET email=?2 WHERE id=?1", params![u.id, email])?;
            c.execute("DELETE FROM sessions WHERE user_id=?1 AND token_hash<>?2", params![u.id, th])?;
            u.email = email;
            db::me_json(c, &u)
        })
        .await?;
    Ok(Json(me))
}

/// `GET /api/me/sessions`: Anzahl der angemeldeten Geräte (Sitzungen), inklusive dieser.
pub async fn sessions(State(s): State<AppState>, a: Auth) -> ApiResult<Json<Value>> {
    let uid = a.user.id;
    let (n, since) =
        s.db.call(move |c| {
            let n: i64 = c.query_row("SELECT COUNT(*) FROM sessions WHERE user_id=?1 AND expires>?2", params![uid, now()], |r| r.get(0))?;
            let since: i64 = c.query_row("SELECT created FROM users WHERE id=?1", [&uid], |r| r.get(0))?;
            Ok((n, since))
        })
        .await?;
    Ok(Json(json!({"count": n, "created": since})))
}

/// `POST /api/me/sessions/logout-others`: alle anderen Sitzungen beenden (diese bleibt).
pub async fn logout_others(State(s): State<AppState>, a: Auth) -> ApiResult<Json<Value>> {
    let (uid, th) = (a.user.id.clone(), a.token_hash.clone());
    let n = s.db.call(move |c| Ok(c.execute("DELETE FROM sessions WHERE user_id=?1 AND token_hash<>?2", params![uid, th])?)).await?;
    Ok(Json(json!({"ended": n})))
}

/// Dateiname für ein Board im ZIP: ohne Pfad- und Steuerzeichen, eindeutig.
fn zip_name(name: &str, used: &mut std::collections::HashSet<String>) -> String {
    let base: String = name.chars().map(|c| if c.is_control() || "/\\:*?\"<>|".contains(c) { '_' } else { c }).collect::<String>().trim().trim_matches('.').chars().take(80).collect();
    let base = if base.is_empty() { "Board".to_string() } else { base };
    let mut n = format!("{base}.owndraw");
    let mut i = 2;
    while !used.insert(n.to_lowercase()) {
        n = format!("{base} ({i}).owndraw");
        i += 1;
    }
    n
}

/// `GET /api/me/export`: ZIP mit `konto.json` und allen eigenen Boards (`boards/<Name>.owndraw`).
pub async fn export(State(s): State<AppState>, a: Auth) -> ApiResult<Response> {
    no_guest(&a)?;
    let dir = s.cfg.boards_dir();
    let u = a.user;
    let (info, boards) =
        s.db.call(move |c| {
            let mut me = db::me_json(c, &u)?;
            me["created"] = json!(u.created);
            let mut st = c.prepare("SELECT id, name, modified FROM boards WHERE owner_id=?1 ORDER BY modified DESC")?;
            let own = st.query_map([&u.id], |r| Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?, r.get::<_, i64>(2)?)))?.collect::<Result<Vec<_>, _>>()?;
            drop(st);
            let mut st = c.prepare("SELECT b.name, m.role FROM members m JOIN boards b ON b.id=m.board_id WHERE m.user_id=?1 ORDER BY b.name")?;
            let shared = st.query_map([&u.id], |r| Ok(json!({"name": r.get::<_, String>(0)?, "role": r.get::<_, String>(1)?})))?.collect::<Result<Vec<_>, _>>()?;
            me["shared_with_me"] = Value::Array(shared);
            Ok((me, own))
        })
        .await?;
    let bytes = tokio::task::spawn_blocking(move || -> std::io::Result<Vec<u8>> {
        use zip::write::SimpleFileOptions;
        let mut z = zip::ZipWriter::new(std::io::Cursor::new(Vec::new()));
        let stored = SimpleFileOptions::default().compression_method(zip::CompressionMethod::Stored);
        let defl = SimpleFileOptions::default().compression_method(zip::CompressionMethod::Deflated);
        let mut used = std::collections::HashSet::new();
        let mut list = Vec::new();
        for (id, name, modified) in &boards {
            let Ok(data) = std::fs::read(crate::boards::board_path(&dir, id)) else { continue };
            let file = zip_name(name, &mut used);
            z.start_file(format!("boards/{file}"), stored).map_err(std::io::Error::other)?;
            z.write_all(&data)?;
            list.push(json!({"name": name, "file": format!("boards/{file}"), "modified": modified}));
        }
        let mut info = info;
        info["boards"] = Value::Array(list);
        info["exported"] = json!(now());
        z.start_file("konto.json", defl).map_err(std::io::Error::other)?;
        z.write_all(serde_json::to_string_pretty(&info).unwrap_or_default().as_bytes())?;
        Ok(z.finish().map_err(std::io::Error::other)?.into_inner())
    })
    .await
    .map_err(ApiError::internal)?
    .map_err(ApiError::internal)?;
    let mut r = bytes.into_response();
    let h = r.headers_mut();
    h.insert(CONTENT_TYPE, HeaderValue::from_static("application/zip"));
    h.insert(CONTENT_DISPOSITION, HeaderValue::from_static("attachment; filename=\"OwnDraw-Konto.zip\""));
    h.insert("cache-control", HeaderValue::from_static("no-store"));
    Ok(r)
}

#[derive(Deserialize)]
pub struct DeleteBody {
    password: String,
}

/// `POST /api/me/delete`: eigenes Konto mit allen eigenen Boards löschen (mit aktuellem Passwort). Der einzige
/// Admin kann sich nicht selbst löschen. Alle Sitzungen enden, das Cookie wird gelöscht.
pub async fn delete(State(s): State<AppState>, ClientIp(ip): ClientIp, a: Auth, J(b): J<DeleteBody>) -> ApiResult<Response> {
    no_guest(&a)?;
    check_current(&s, &a, &ip, b.password).await?;
    let dir = s.cfg.boards_dir();
    let u = a.user;
    let uid = u.id.clone();
    let boards =
        s.db.call(move |c| {
            if u.admin {
                let admins: i64 = c.query_row("SELECT COUNT(*) FROM users WHERE admin=1 AND disabled=0 AND guest=0", [], |r| r.get(0))?;
                if admins <= 1 {
                    return Err(ApiError::conflict("Du bist der einzige Admin. Ernenne zuerst einen anderen Admin, bevor du dein Konto löschst."));
                }
            }
            let tx = c.transaction()?;
            let boards = crate::admin::remove_user(&tx, &dir, &u.id)?;
            tx.commit()?;
            Ok(boards)
        })
        .await?;
    for b in &boards {
        s.hub.kick(b, None, "not_found", "Das Board wurde gelöscht.");
    }
    s.hub.kick_user(&uid, "unauthorized", "Dein Konto wurde gelöscht.");
    Ok(with_cookie("", 0, json!({})))
}
