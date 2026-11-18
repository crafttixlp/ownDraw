//! Fehlerberichte (ab 4.10.0): Nutzer und Gäste senden Text + Screenshots (`POST /api/bugreports`, multipart),
//! Admins sehen sie im Admin-Panel (`/api/admin/bugreports`).
//!
//! Bilder liegen unter `data/bugreports/<id>/<n>.<ext>` (wird mit `data/` gesichert). Nur PNG, JPEG und WebP,
//! erkannt am Inhalt (nicht am angegebenen Typ); ausgeliefert nur an Admins, mit `nosniff` und CSP `sandbox`.

use crate::auth::{rate_check_n, AdminAuth, ClientIp, MaybeAuth, J};
use crate::db::now;
use crate::error::{ApiError, ApiResult};
use crate::state::AppState;
use axum::extract::{Multipart, Path, State};
use axum::http::header::{CACHE_CONTROL, CONTENT_DISPOSITION, CONTENT_SECURITY_POLICY, CONTENT_TYPE};
use axum::http::HeaderValue;
use axum::response::{IntoResponse, Response};
use axum::Json;
use rusqlite::{params, Connection, OptionalExtension, Row};
use serde::Deserialize;
use serde_json::{json, Value};
use std::path::PathBuf;

pub const MAX_TEXT: usize = 5000;
pub const MAX_IMAGES: usize = 5;
pub const MAX_IMAGE_BYTES: usize = 5 * 1024 * 1024;
/// Obergrenze für eine ganze Meldung (Bilder + Textfelder + Multipart-Rahmen).
pub const MAX_REQUEST_BYTES: usize = MAX_IMAGES * MAX_IMAGE_BYTES + 256 * 1024;
/// Meldungen je Client-Adresse und Zeitfenster (`rate_window_secs`, Standard 10 min).
pub const RATE_LIMIT: u32 = 5;
/// Höchstens so viele Meldungen insgesamt (danach muss ein Admin aufräumen).
pub const MAX_REPORTS: i64 = 1000;
/// Höchstens so viel Speicher für alle Screenshots zusammen.
pub const MAX_TOTAL_BYTES: i64 = 1024 * 1024 * 1024;
const MAX_META: usize = 300;

pub fn dir(s: &AppState) -> PathBuf {
    s.cfg.data.join("bugreports")
}

/// Bildtyp am Inhalt erkennen (Magic Bytes): `(mime, endung)`.
pub fn sniff(b: &[u8]) -> Option<(&'static str, &'static str)> {
    if b.starts_with(b"\x89PNG\r\n\x1a\n") {
        Some(("image/png", "png"))
    } else if b.starts_with(&[0xFF, 0xD8, 0xFF]) {
        Some(("image/jpeg", "jpg"))
    } else if b.len() >= 12 && &b[0..4] == b"RIFF" && &b[8..12] == b"WEBP" {
        Some(("image/webp", "webp"))
    } else {
        None
    }
}

fn clean(s: &str, max: usize, multiline: bool) -> String {
    s.trim().chars().filter(|c| (multiline && (*c == '\n' || *c == '\t')) || !c.is_control()).take(max).collect()
}

struct Report {
    id: String,
    created: i64,
    user_id: Option<String>,
    sender: String,
    guest: bool,
    text: String,
    version: String,
    user_agent: String,
    page: String,
    screen: String,
    images: Value,
    status: String,
}

const COLS: &str = "id,created,user_id,sender,guest,text,version,user_agent,page,screen,images,status";

impl Report {
    fn from_row(r: &Row) -> rusqlite::Result<Self> {
        Ok(Self {
            id: r.get(0)?,
            created: r.get(1)?,
            user_id: r.get(2)?,
            sender: r.get(3)?,
            guest: r.get::<_, i64>(4)? != 0,
            text: r.get(5)?,
            version: r.get(6)?,
            user_agent: r.get(7)?,
            page: r.get(8)?,
            screen: r.get(9)?,
            images: serde_json::from_str(&r.get::<_, String>(10)?).unwrap_or(Value::Array(vec![])),
            status: r.get(11)?,
        })
    }
    fn json(&self) -> Value {
        json!({
            "id": self.id, "created": self.created, "user_id": self.user_id, "sender": self.sender, "guest": self.guest,
            "text": self.text, "version": self.version, "user_agent": self.user_agent, "page": self.page,
            "screen": self.screen, "images": self.images, "status": self.status,
        })
    }
}

fn by_id(c: &Connection, id: &str) -> rusqlite::Result<Option<Report>> {
    c.query_row(&format!("SELECT {COLS} FROM bug_reports WHERE id=?1"), [id], Report::from_row).optional()
}

struct Img {
    bytes: Vec<u8>,
    mime: &'static str,
    ext: &'static str,
}

/// `POST /api/bugreports` (multipart): Felder `text` (Pflicht), `version`, `user_agent`, `page`, `screen`
/// und bis zu fünf Dateien `image`. Ohne Anmeldung möglich.
pub async fn create(State(s): State<AppState>, ClientIp(ip): ClientIp, who: MaybeAuth, mut mp: Multipart) -> ApiResult<Json<Value>> {
    rate_check_n(&s, "bugreport", &ip, RATE_LIMIT)?;
    crate::features::check(&s, who.0.as_ref().map(|a| &a.user), owndraw_core::features::BUGREPORT).await?;
    let (mut text, mut version, mut ua, mut page, mut screen) = (String::new(), String::new(), String::new(), String::new(), String::new());
    let mut imgs: Vec<Img> = Vec::new();
    let bad = |_| ApiError::invalid("Die Meldung konnte nicht gelesen werden.");
    while let Some(mut f) = mp.next_field().await.map_err(bad)? {
        let name = f.name().unwrap_or("").to_string();
        if name == "image" {
            if imgs.len() >= MAX_IMAGES {
                return Err(ApiError::invalid(format!("Höchstens {MAX_IMAGES} Bilder pro Meldung.")));
            }
            let mut bytes = Vec::new();
            while let Some(ch) = f.chunk().await.map_err(bad)? {
                if bytes.len() + ch.len() > MAX_IMAGE_BYTES {
                    return Err(ApiError::too_large(format!("Ein Bild ist größer als {} MB.", MAX_IMAGE_BYTES / 1024 / 1024)));
                }
                bytes.extend_from_slice(&ch);
            }
            if bytes.is_empty() {
                continue;
            }
            let (mime, ext) = sniff(&bytes).ok_or_else(|| ApiError::invalid("Nur PNG-, JPEG- und WebP-Bilder sind möglich."))?;
            imgs.push(Img { bytes, mime, ext });
        } else {
            let mut raw = Vec::new();
            while let Some(ch) = f.chunk().await.map_err(bad)? {
                if raw.len() + ch.len() > 64 * 1024 {
                    return Err(ApiError::too_large("Der Text ist zu lang."));
                }
                raw.extend_from_slice(&ch);
            }
            let v = String::from_utf8_lossy(&raw);
            match name.as_str() {
                "text" => text = clean(&v, MAX_TEXT, true),
                "version" => version = clean(&v, 40, false),
                "user_agent" => ua = clean(&v, MAX_META, false),
                "page" => page = clean(&v, MAX_META, false),
                "screen" => screen = clean(&v, 40, false),
                _ => {}
            }
        }
    }
    if text.is_empty() {
        return Err(ApiError::invalid("Bitte beschreibe den Fehler."));
    }
    let size: i64 = imgs.iter().map(|i| i.bytes.len() as i64).sum();
    let (user_id, sender, guest) = match who.0 {
        Some(a) if a.user.guest => (Some(a.user.id), a.user.name, true),
        Some(a) => (Some(a.user.id), if a.user.name.is_empty() { a.user.email.clone() } else { format!("{} <{}>", a.user.name, a.user.email) }, false),
        None => (None, "Ohne Konto".to_string(), true),
    };
    // Platz prüfen und Eintrag reservieren
    let id = uuid::Uuid::new_v4().to_string();
    let meta: Vec<Value> = imgs.iter().enumerate().map(|(n, i)| json!({"n": n, "mime": i.mime, "size": i.bytes.len()})).collect();
    let meta = Value::Array(meta).to_string();
    let base = dir(&s);
    let folder = base.join(&id);
    let names: Vec<String> = imgs.iter().enumerate().map(|(n, i)| format!("{n}.{}", i.ext)).collect();
    let id2 = id.clone();
    s.db.call(move |c| {
        let (n, total): (i64, i64) = c.query_row("SELECT COUNT(*), COALESCE(SUM(size),0) FROM bug_reports", [], |r| Ok((r.get(0)?, r.get(1)?)))?;
        if n >= MAX_REPORTS || total + size > MAX_TOTAL_BYTES {
            return Err(ApiError::quota("Der Speicher für Fehlerberichte ist voll. Bitte später erneut versuchen."));
        }
        c.execute(
            &format!("INSERT INTO bug_reports({COLS},size) VALUES(?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11,'open',?12)"),
            params![id2, now(), user_id, sender, guest as i64, text, version, ua, page, screen, meta, size],
        )?;
        Ok(())
    })
    .await?;
    let written = tokio::task::spawn_blocking(move || -> std::io::Result<()> {
        if imgs.is_empty() {
            return Ok(());
        }
        std::fs::create_dir_all(&folder)?;
        for (img, name) in imgs.iter().zip(&names) {
            std::fs::write(folder.join(name), &img.bytes)?;
        }
        Ok(())
    })
    .await
    .map_err(ApiError::internal)?;
    if let Err(e) = written {
        let id3 = id.clone();
        let _ = s.db.call(move |c| Ok(c.execute("DELETE FROM bug_reports WHERE id=?1", [&id3])?)).await;
        let _ = std::fs::remove_dir_all(base.join(&id));
        return Err(ApiError::internal(e));
    }
    Ok(Json(json!({"id": id})))
}

/// `GET /api/admin/bugreports`: alle Meldungen, neueste zuerst.
pub async fn admin_list(State(s): State<AppState>, _a: AdminAuth) -> ApiResult<Json<Value>> {
    let v = s.db.call(|c| {
        let mut st = c.prepare(&format!("SELECT {COLS} FROM bug_reports ORDER BY created DESC, id"))?;
        let v = st.query_map([], Report::from_row)?.collect::<Result<Vec<_>, _>>()?;
        Ok(Value::Array(v.iter().map(Report::json).collect()))
    })
    .await?;
    Ok(Json(v))
}

#[derive(Deserialize)]
pub struct PatchReport {
    status: String,
}

/// `PATCH /api/admin/bugreports/{id}` `{status:"open"|"done"}`.
pub async fn patch(State(s): State<AppState>, _a: AdminAuth, Path(id): Path<String>, J(b): J<PatchReport>) -> ApiResult<Json<Value>> {
    if !matches!(b.status.as_str(), "open" | "done") {
        return Err(ApiError::invalid("Der Status muss „open“ oder „done“ sein."));
    }
    let v = s.db.call(move |c| {
        if c.execute("UPDATE bug_reports SET status=?2 WHERE id=?1", params![id, b.status])? == 0 {
            return Err(ApiError::not_found("Meldung nicht gefunden."));
        }
        Ok(by_id(c, &id)?.ok_or_else(|| ApiError::not_found("Meldung nicht gefunden."))?.json())
    })
    .await?;
    Ok(Json(v))
}

fn valid_id(id: &str) -> ApiResult<()> {
    if uuid::Uuid::parse_str(id).is_ok() {
        Ok(())
    } else {
        Err(ApiError::not_found("Meldung nicht gefunden."))
    }
}

/// `DELETE /api/admin/bugreports/{id}`: Meldung samt Screenshots löschen.
pub async fn delete(State(s): State<AppState>, _a: AdminAuth, Path(id): Path<String>) -> ApiResult<Json<Value>> {
    valid_id(&id)?;
    let id2 = id.clone();
    s.db.call(move |c| {
        if c.execute("DELETE FROM bug_reports WHERE id=?1", [&id2])? == 0 {
            return Err(ApiError::not_found("Meldung nicht gefunden."));
        }
        Ok(())
    })
    .await?;
    let _ = tokio::fs::remove_dir_all(dir(&s).join(&id)).await;
    Ok(Json(json!({})))
}

/// `GET /api/admin/bugreports/{id}/images/{n}`: Screenshot (nur Admins).
pub async fn image(State(s): State<AppState>, _a: AdminAuth, Path((id, n)): Path<(String, usize)>) -> ApiResult<Response> {
    valid_id(&id)?;
    let id2 = id.clone();
    let r = s.db.call(move |c| Ok(by_id(c, &id2)?)).await?.ok_or_else(|| ApiError::not_found("Meldung nicht gefunden."))?;
    let entry = r.images.as_array().and_then(|a| a.iter().find(|i| i["n"].as_u64() == Some(n as u64))).ok_or_else(|| ApiError::not_found("Bild nicht gefunden."))?;
    let (mime, ext) = match entry["mime"].as_str() {
        Some("image/png") => ("image/png", "png"),
        Some("image/jpeg") => ("image/jpeg", "jpg"),
        Some("image/webp") => ("image/webp", "webp"),
        _ => return Err(ApiError::not_found("Bild nicht gefunden.")),
    };
    let bytes = tokio::fs::read(dir(&s).join(&id).join(format!("{n}.{ext}"))).await.map_err(|_| ApiError::not_found("Bild nicht gefunden."))?;
    let mut resp = bytes.into_response();
    let h = resp.headers_mut();
    h.insert(CONTENT_TYPE, HeaderValue::from_static(mime));
    h.insert(CONTENT_DISPOSITION, HeaderValue::from_str(&format!("inline; filename=\"fehlerbericht-{n}.{ext}\"")).map_err(ApiError::internal)?);
    h.insert(CONTENT_SECURITY_POLICY, HeaderValue::from_static("default-src 'none'; sandbox"));
    h.insert(CACHE_CONTROL, HeaderValue::from_static("private, max-age=3600"));
    Ok(resp)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bildtyp_am_inhalt() {
        assert_eq!(sniff(b"\x89PNG\r\n\x1a\nxxxx"), Some(("image/png", "png")));
        assert_eq!(sniff(&[0xFF, 0xD8, 0xFF, 0xE0]), Some(("image/jpeg", "jpg")));
        assert_eq!(sniff(b"RIFF\0\0\0\0WEBPVP8 "), Some(("image/webp", "webp")));
        assert_eq!(sniff(b"<svg xmlns="), None);
        assert_eq!(sniff(b"GIF89a"), None);
    }

    #[test]
    fn text_bereinigen() {
        assert_eq!(clean(" a\u{1b}b\nc ", 10, true), "ab\nc");
        assert_eq!(clean("a\nb", 10, false), "ab");
    }
}
