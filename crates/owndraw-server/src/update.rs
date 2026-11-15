//! Server-Self-Update: GitHub-Release prüfen → herunterladen → minisign prüfen → DB-Backup →
//! nach `data/versions/<v>/` entpacken → Symlink `data/current` umstellen → Prozess beenden
//! (Docker startet neu, `packaging/server/entrypoint.sh` wählt `data/current`).
//!
//! Auto-Update (Standard AUS): einmal täglich zur eingestellten Stunde, Uhrzeit in UTC.

use crate::auth::{AdminAuth, J};
use crate::db::{get_setting, now, set_setting};
use crate::error::{ApiError, ApiResult};
use crate::state::AppState;
use axum::extract::State;
use axum::Json;
use rusqlite::Connection;
use semver::Version;
use serde::Deserialize;
use serde_json::{json, Value};
use std::path::Path;
use std::sync::atomic::Ordering;
use std::time::Duration;

/// Öffentlicher Schlüssel der Releases (gleicher minisign-Schlüssel wie die Desktop-App).
pub const PUBKEY: &str = include_str!("../../owndraw/src/update/minisign.pub");
pub const CURRENT: &str = env!("CARGO_PKG_VERSION");
const MAX_DOWNLOAD: u64 = 400 * 1024 * 1024;
const LOG_KEEP: usize = 100;

// ---------- Reine Hilfen (getestet) ----------

#[derive(Debug, PartialEq)]
pub struct ReleaseInfo {
    pub version: Version,
    pub tar_url: Option<String>,
    pub sig_url: Option<String>,
}

pub fn asset_name(v: &Version) -> String {
    format!("owndraw-server-{v}-x86_64-linux.tar.gz")
}

/// Liest die GitHub-Release-JSON; Entwürfe, Vorabversionen und Tags ohne semver ergeben `None`.
pub fn parse_release(v: &Value) -> Option<ReleaseInfo> {
    if v.get("draft").and_then(Value::as_bool).unwrap_or(false) || v.get("prerelease").and_then(Value::as_bool).unwrap_or(false) {
        return None;
    }
    let tag = v.get("tag_name")?.as_str()?;
    let version = Version::parse(tag.trim().trim_start_matches(['v', 'V'])).ok()?;
    let name = asset_name(&version);
    let sig_name = format!("{name}.minisig");
    let find = |n: &str| {
        v.get("assets")?
            .as_array()?
            .iter()
            .find(|a| a.get("name").and_then(Value::as_str) == Some(n))
            .and_then(|a| a.get("browser_download_url")?.as_str().map(str::to_string))
    };
    Some(ReleaseInfo { tar_url: find(&name), sig_url: find(&sig_name), version })
}

/// Prüft `data` gegen eine minisign-Signatur (Inhalt der `.minisig`-Datei).
pub fn verify_signature(pubkey: &str, data: &[u8], sig_text: &str) -> Result<(), String> {
    let pk = minisign_verify::PublicKey::decode(pubkey).map_err(|e| format!("Öffentlicher Schlüssel unlesbar: {e}"))?;
    let sig = minisign_verify::Signature::decode(sig_text).map_err(|e| format!("Signatur unlesbar: {e}"))?;
    pk.verify(data, &sig, true).map_err(|e| format!("Signatur ungültig: {e}"))
}

#[cfg(unix)]
fn symlink(target: &str, link: &Path) -> std::io::Result<()> {
    std::os::unix::fs::symlink(target, link)
}
#[cfg(not(unix))]
fn symlink(_: &str, _: &Path) -> std::io::Result<()> {
    Err(std::io::Error::other("Symlinks werden nur unter Linux unterstützt"))
}

fn replace_symlink(link: &Path, target: &str) -> std::io::Result<()> {
    let tmp = link.with_extension("new");
    let _ = std::fs::remove_file(&tmp);
    symlink(target, &tmp)?;
    std::fs::rename(&tmp, link)
}

/// Stellt `data/current` auf `versions/<version>` um (und merkt die vorige Version in `data/previous`).
pub fn switch_current(data: &Path, version: &Version) -> std::io::Result<()> {
    let cur = data.join("current");
    if let Ok(prev) = std::fs::read_link(&cur) {
        replace_symlink(&data.join("previous"), &prev.to_string_lossy())?;
    }
    replace_symlink(&cur, &format!("versions/{version}"))
}

/// Entpackt ein `.tar.gz` (Binary `owndraw-server` + `web/`) nach `dest` und prüft, dass das Binary da ist.
pub fn extract_tar_gz(archive: &Path, dest: &Path) -> std::io::Result<()> {
    let _ = std::fs::remove_dir_all(dest);
    std::fs::create_dir_all(dest)?;
    let file = std::fs::File::open(archive)?;
    let mut ar = tar::Archive::new(flate2::read::GzDecoder::new(file));
    ar.set_preserve_permissions(true);
    ar.unpack(dest)?;
    let bin = dest.join("owndraw-server");
    if !bin.is_file() {
        return Err(std::io::Error::other("Das Paket enthält kein Programm owndraw-server"));
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&bin, std::fs::Permissions::from_mode(0o755))?;
    }
    Ok(())
}

// ---------- Zustand in der DB ----------

fn log_push(conn: &Connection, msg: &str) {
    let mut v: Vec<Value> = get_setting(conn, "update.log").ok().flatten().and_then(|s| serde_json::from_str(&s).ok()).unwrap_or_default();
    v.push(json!({"t": now(), "msg": msg}));
    if v.len() > LOG_KEEP {
        v.drain(0..v.len() - LOG_KEEP);
    }
    let _ = set_setting(conn, "update.log", &Value::Array(v).to_string());
    eprintln!("Update: {msg}");
}

async fn log(state: &AppState, msg: impl Into<String>) {
    let msg = msg.into();
    let _ = state
        .db
        .call(move |c| {
            log_push(c, &msg);
            Ok(())
        })
        .await;
}

fn is_available(conn: &Connection) -> bool {
    let latest = get_setting(conn, "update.latest").ok().flatten().and_then(|s| Version::parse(&s).ok());
    let has_asset = get_setting(conn, "update.tar_url").ok().flatten().is_some_and(|s| !s.is_empty());
    let cur = Version::parse(CURRENT).ok();
    matches!((latest, cur), (Some(l), Some(c)) if l > c) && has_asset
}

fn state_json(conn: &Connection) -> ApiResult<Value> {
    let get = |k: &str| get_setting(conn, k).ok().flatten();
    let hour: i64 = get("update.auto_hour").and_then(|s| s.parse().ok()).unwrap_or(3);
    Ok(json!({
        "current": CURRENT,
        "latest": get("update.latest").filter(|s| !s.is_empty()),
        "available": is_available(conn),
        "auto": get("update.auto").as_deref() == Some("1"),
        "auto_hour": hour,
        "last_check": get("update.last_check").and_then(|s| s.parse::<i64>().ok()),
        "log": get("update.log").and_then(|s| serde_json::from_str::<Value>(&s).ok()).unwrap_or_else(|| json!([])),
    }))
}

// ---------- Prüfen & Installieren ----------

fn client(timeout: Duration) -> ApiResult<reqwest::Client> {
    reqwest::Client::builder().user_agent(format!("owndraw-server/{CURRENT}")).timeout(timeout).build().map_err(ApiError::internal)
}

pub async fn check(state: &AppState) -> ApiResult<()> {
    let res = async {
        let resp = client(Duration::from_secs(30))
            .map_err(|e| e.message)?
            .get(&state.cfg.release_url)
            .header("Accept", "application/vnd.github+json")
            .send()
            .await
            .map_err(|e| format!("GitHub nicht erreichbar: {e}"))?;
        if !resp.status().is_success() {
            return Err(format!("GitHub antwortet mit Status {}", resp.status()));
        }
        let v: Value = resp.json().await.map_err(|e| format!("Antwort von GitHub unlesbar: {e}"))?;
        parse_release(&v).ok_or_else(|| "Kein passendes Release gefunden".to_string())
    }
    .await;
    match res {
        Ok(info) => {
            let msg = match (&info.tar_url, &info.sig_url) {
                (Some(_), Some(_)) => format!("Prüfung: neueste Version {} (installiert: {CURRENT})", info.version),
                _ => format!("Prüfung: Version {} hat kein Server-Paket mit Signatur", info.version),
            };
            let ok = info.tar_url.is_some() && info.sig_url.is_some();
            state
                .db
                .call(move |c| {
                    set_setting(c, "update.latest", &info.version.to_string())?;
                    set_setting(c, "update.tar_url", if ok { info.tar_url.as_deref().unwrap_or("") } else { "" })?;
                    set_setting(c, "update.sig_url", if ok { info.sig_url.as_deref().unwrap_or("") } else { "" })?;
                    set_setting(c, "update.last_check", &now().to_string())?;
                    log_push(c, &msg);
                    Ok(())
                })
                .await
        }
        Err(e) => {
            log(state, format!("Prüfung fehlgeschlagen: {e}")).await;
            let _ = state.db.call(|c| Ok(set_setting(c, "update.last_check", &now().to_string())?)).await;
            Err(ApiError::upstream(e))
        }
    }
}

async fn download(url: &str, dest: &Path) -> Result<(), String> {
    use tokio::io::AsyncWriteExt;
    let mut resp = client(Duration::from_secs(900))
        .map_err(|_| "HTTP-Client".to_string())?
        .get(url)
        .send()
        .await
        .map_err(|e| format!("Download fehlgeschlagen: {e}"))?;
    if !resp.status().is_success() {
        return Err(format!("Download: Status {}", resp.status()));
    }
    let mut f = tokio::fs::File::create(dest).await.map_err(|e| e.to_string())?;
    let mut total = 0u64;
    while let Some(chunk) = resp.chunk().await.map_err(|e| format!("Download abgebrochen: {e}"))? {
        total += chunk.len() as u64;
        if total > MAX_DOWNLOAD {
            return Err("Download zu groß".into());
        }
        f.write_all(&chunk).await.map_err(|e| e.to_string())?;
    }
    f.flush().await.map_err(|e| e.to_string())
}

/// Installiert die neueste Version; bei Erfolg beendet sich der Prozess kurz darauf.
pub async fn install(state: &AppState) -> ApiResult<String> {
    if state.installing.swap(true, Ordering::SeqCst) {
        return Err(ApiError::conflict("Es läuft bereits eine Installation."));
    }
    let res = install_inner(state).await;
    match &res {
        Ok(_) => {} // Prozess endet; `installing` bleibt gesetzt
        Err(e) => {
            state.installing.store(false, Ordering::SeqCst);
            log(state, format!("Installation fehlgeschlagen: {}", e.message)).await;
        }
    }
    res
}

async fn install_inner(state: &AppState) -> ApiResult<String> {
    if state.db.call(|c| Ok(!is_available(c))).await? {
        check(state).await?;
    }
    let (latest, tar_url, sig_url) = state
        .db
        .call(|c| {
            if !is_available(c) {
                return Err(ApiError::conflict("Es ist kein neueres Server-Paket verfügbar."));
            }
            let g = |k: &str| get_setting(c, k).ok().flatten().unwrap_or_default();
            Ok((g("update.latest"), g("update.tar_url"), g("update.sig_url")))
        })
        .await?;
    let version = Version::parse(&latest).map_err(|_| ApiError::internal("Version unlesbar"))?;
    let data = state.cfg.data.clone();
    log(state, format!("Installiere {version}: lade {}", asset_name(&version))).await;

    let tmp_dir = data.join("tmp");
    tokio::fs::create_dir_all(&tmp_dir).await?;
    let tar_path = tmp_dir.join(asset_name(&version));
    let sig_path = tmp_dir.join(format!("{}.minisig", asset_name(&version)));
    download(&tar_url, &tar_path).await.map_err(ApiError::upstream)?;
    download(&sig_url, &sig_path).await.map_err(ApiError::upstream)?;

    // Signatur prüfen (ohne gültige Signatur wird nichts installiert).
    let (tp, sp) = (tar_path.clone(), sig_path.clone());
    tokio::task::spawn_blocking(move || -> Result<(), String> {
        let data = std::fs::read(&tp).map_err(|e| e.to_string())?;
        let sig = std::fs::read_to_string(&sp).map_err(|e| e.to_string())?;
        verify_signature(PUBKEY, &data, &sig)
    })
    .await
    .map_err(ApiError::internal)?
    .map_err(|e| ApiError::invalid(format!("Signaturprüfung fehlgeschlagen, nichts installiert: {e}")))?;
    log(state, "Signatur gültig").await;

    // DB-Backup (VACUUM INTO).
    let backups = data.join("backups");
    tokio::fs::create_dir_all(&backups).await?;
    let backup = backups.join(format!("owndraw-{CURRENT}-{}.db", now()));
    let b2 = backup.clone();
    state
        .db
        .call(move |c| {
            c.execute("VACUUM INTO ?1", [b2.to_string_lossy().as_ref()])?;
            Ok(())
        })
        .await?;
    log(state, format!("Datenbank gesichert: {}", backup.display())).await;

    // Entpacken und umstellen.
    let (d2, v2, tp) = (data.clone(), version.clone(), tar_path.clone());
    tokio::task::spawn_blocking(move || -> std::io::Result<()> {
        let versions = d2.join("versions");
        std::fs::create_dir_all(&versions)?;
        let staging = versions.join(format!("{v2}.partial"));
        extract_tar_gz(&tp, &staging)?;
        let target = versions.join(v2.to_string());
        let _ = std::fs::remove_dir_all(&target);
        std::fs::rename(&staging, &target)?;
        switch_current(&d2, &v2)
    })
    .await
    .map_err(ApiError::internal)??;
    let _ = tokio::fs::remove_file(&tar_path).await;
    let _ = tokio::fs::remove_file(&sig_path).await;
    log(state, format!("Version {version} installiert, Server startet neu")).await;

    let st = state.clone();
    tokio::spawn(async move {
        tokio::time::sleep(Duration::from_secs(2)).await;
        // offene Live-Sitzungen noch speichern
        st.hub.flush_all(&st).await;
        std::process::exit(0);
    });
    Ok(version.to_string())
}

/// Hintergrundaufgabe: prüft jede Minute, ob die eingestellte Stunde (UTC) erreicht ist.
pub fn spawn_auto(state: AppState) {
    tokio::spawn(async move {
        loop {
            tokio::time::sleep(Duration::from_secs(60)).await;
            let t = now();
            let (day, hour) = (t / 86400, (t % 86400) / 3600);
            let due = state
                .db
                .call(move |c| {
                    let g = |k: &str| get_setting(c, k).ok().flatten();
                    let auto = g("update.auto").as_deref() == Some("1");
                    let h: i64 = g("update.auto_hour").and_then(|s| s.parse().ok()).unwrap_or(3);
                    let last: i64 = g("update.auto_day").and_then(|s| s.parse().ok()).unwrap_or(-1);
                    let due = auto && h == hour && last != day;
                    if due {
                        set_setting(c, "update.auto_day", &day.to_string())?;
                        log_push(c, "Automatische Prüfung gestartet");
                    }
                    Ok(due)
                })
                .await
                .unwrap_or(false);
            if due && check(&state).await.is_ok() && state.db.call(|c| Ok(is_available(c))).await.unwrap_or(false) {
                let _ = install(&state).await;
            }
        }
    });
}

// ---------- Routen ----------

async fn full_state(s: &AppState) -> ApiResult<Json<Value>> {
    let mut v = s.db.call(|c| state_json(c)).await?;
    v["installing"] = json!(s.installing.load(Ordering::SeqCst));
    Ok(Json(v))
}

pub async fn get_state(State(s): State<AppState>, _a: AdminAuth) -> ApiResult<Json<Value>> {
    full_state(&s).await
}

pub async fn post_check(State(s): State<AppState>, _a: AdminAuth) -> ApiResult<Json<Value>> {
    check(&s).await?;
    full_state(&s).await
}

pub async fn post_install(State(s): State<AppState>, _a: AdminAuth) -> ApiResult<Json<Value>> {
    let v = install(&s).await?;
    Ok(Json(json!({"ok": true, "version": v, "restarting": true})))
}

#[derive(Deserialize)]
pub struct PatchBody {
    auto: Option<bool>,
    auto_hour: Option<i64>,
}

pub async fn patch_state(State(s): State<AppState>, _a: AdminAuth, J(b): J<PatchBody>) -> ApiResult<Json<Value>> {
    if b.auto_hour.is_some_and(|h| !(0..=23).contains(&h)) {
        return Err(ApiError::invalid("Die Stunde muss zwischen 0 und 23 liegen."));
    }
    s.db.call(move |c| {
        if let Some(a) = b.auto {
            set_setting(c, "update.auto", if a { "1" } else { "0" })?;
            log_push(c, if a { "Auto-Update eingeschaltet" } else { "Auto-Update ausgeschaltet" });
        }
        if let Some(h) = b.auto_hour {
            set_setting(c, "update.auto_hour", &h.to_string())?;
        }
        Ok(())
    })
    .await?;
    full_state(&s).await
}

#[cfg(test)]
mod tests {
    use super::*;

    fn release(tag: &str, with_assets: bool) -> Value {
        let v = tag.trim_start_matches('v');
        let assets = if with_assets {
            json!([
                {"name": format!("owndraw-server-{v}-x86_64-linux.tar.gz"), "browser_download_url": "https://x/t.tar.gz"},
                {"name": format!("owndraw-server-{v}-x86_64-linux.tar.gz.minisig"), "browser_download_url": "https://x/t.minisig"},
                {"name": "OwnDraw-x86_64.AppImage", "browser_download_url": "https://x/app"}
            ])
        } else {
            json!([{"name": "OwnDraw-x86_64.AppImage", "browser_download_url": "https://x/app"}])
        };
        json!({"tag_name": tag, "draft": false, "prerelease": false, "assets": assets})
    }

    #[test]
    fn release_wird_gelesen() {
        let r = parse_release(&release("v4.6.0", true)).unwrap();
        assert_eq!(r.version, Version::new(4, 6, 0));
        assert_eq!(r.tar_url.as_deref(), Some("https://x/t.tar.gz"));
        assert_eq!(r.sig_url.as_deref(), Some("https://x/t.minisig"));
        let r = parse_release(&release("4.6.1", false)).unwrap();
        assert!(r.tar_url.is_none() && r.sig_url.is_none());
        assert!(parse_release(&json!({"tag_name": "nightly", "assets": []})).is_none());
        assert!(parse_release(&json!({"tag_name": "v5.0.0", "draft": true})).is_none());
    }

    #[test]
    fn eingebauter_schluessel_ist_gueltig() {
        assert!(minisign_verify::PublicKey::decode(PUBKEY).is_ok());
        assert!(verify_signature(PUBKEY, b"x", "kaputt").is_err());
    }

    #[test]
    fn entpacken_und_umstellen() {
        let dir = tempfile::tempdir().unwrap();
        // Paket bauen: owndraw-server + web/index.html
        let tgz = dir.path().join("p.tar.gz");
        {
            let gz = flate2::write::GzEncoder::new(std::fs::File::create(&tgz).unwrap(), flate2::Compression::fast());
            let mut b = tar::Builder::new(gz);
            for (name, data, mode) in [("owndraw-server", &b"#!/bin/sh\n"[..], 0o644), ("web/index.html", &b"<html>"[..], 0o644)] {
                let mut h = tar::Header::new_gnu();
                h.set_size(data.len() as u64);
                h.set_mode(mode);
                h.set_cksum();
                b.append_data(&mut h, name, data).unwrap();
            }
            b.into_inner().unwrap().finish().unwrap();
        }
        let data = dir.path().join("data");
        let v1 = Version::new(4, 6, 0);
        let v2 = Version::new(4, 7, 0);
        for v in [&v1, &v2] {
            extract_tar_gz(&tgz, &data.join("versions").join(v.to_string())).unwrap();
            switch_current(&data, v).unwrap();
        }
        assert!(data.join("current/owndraw-server").is_file());
        assert!(data.join("current/web/index.html").is_file());
        assert_eq!(std::fs::read_link(data.join("current")).unwrap().to_str(), Some("versions/4.7.0"));
        assert_eq!(std::fs::read_link(data.join("previous")).unwrap().to_str(), Some("versions/4.6.0"));
    }

    #[test]
    fn paket_ohne_programm_wird_abgelehnt() {
        let dir = tempfile::tempdir().unwrap();
        let tgz = dir.path().join("leer.tar.gz");
        let gz = flate2::write::GzEncoder::new(std::fs::File::create(&tgz).unwrap(), flate2::Compression::fast());
        tar::Builder::new(gz).into_inner().unwrap().finish().unwrap();
        assert!(extract_tar_gz(&tgz, &dir.path().join("x")).is_err());
    }
}
