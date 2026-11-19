//! Download-Seite `/download`: Android-App (.apk) und Linux-App (.AppImage) zum Herunterladen.
//!
//! Quelle ist das neueste GitHub-Release. Der Server lädt die Dateien samt minisign-Signatur einmal herunter,
//! prüft sie mit demselben Schlüssel wie das Self-Update und legt sie unter `data/downloads/` ab. Ausgeliefert
//! wird nur von dort; Nutzer müssen nicht zu GitHub. Fehlt eine Datei im Release (z. B. die APK ohne
//! Android-Keystore), bleibt die zuletzt geprüfte Version stehen oder die Seite zeigt „noch nicht verfügbar“.

use crate::auth::ClientIp;
use crate::db::now;
use crate::state::AppState;
use axum::body::Body;
use axum::extract::{Path as UrlPath, Request, State};
use axum::http::header::{CACHE_CONTROL, CONTENT_DISPOSITION, CONTENT_SECURITY_POLICY, CONTENT_TYPE};
use axum::http::{HeaderValue, Method, StatusCode};
use axum::response::{Html, IntoResponse, Response};
use semver::Version;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use sha2::{Digest, Sha256};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::RwLock;
use std::time::Duration;
use tower::ServiceExt;
use tower_http::services::ServeFile;

const PAGE_HTML: &str = include_str!("download.html");
/// Größte erlaubte Datei (APK/AppImage sind etwa 10 MB).
const MAX_FILE: u64 = 300 * 1024 * 1024;
/// Downloads je Client-Adresse und Fenster (auch Fortsetzungen per Range zählen).
pub const RATE_LIMIT: u32 = 30;
pub const RATE_WINDOW: Duration = Duration::from_secs(600);
/// Abstand der Prüfungen beim GitHub-Release.
const REFRESH_EVERY: Duration = Duration::from_secs(3600);

/// Angebotene Apps.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Kind {
    Android,
    Linux,
}

impl Kind {
    pub const ALL: [Kind; 2] = [Kind::Android, Kind::Linux];

    pub fn id(self) -> &'static str {
        match self {
            Kind::Android => "android",
            Kind::Linux => "linux",
        }
    }

    /// Asset-Name im Release (wie in `release.yml`).
    pub fn asset_name(self, v: &Version) -> String {
        match self {
            Kind::Android => format!("OwnDraw-{v}.apk"),
            Kind::Linux => format!("OwnDraw-{v}-x86_64.AppImage"),
        }
    }

    fn content_type(self) -> &'static str {
        match self {
            Kind::Android => "application/vnd.android.package-archive",
            Kind::Linux => "application/octet-stream",
        }
    }
}

/// Eine bereitliegende Datei.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct Entry {
    pub kind: String,
    pub version: String,
    pub name: String,
    pub size: u64,
    pub sha256: String,
}

/// Inhalt von `data/downloads/manifest.json`.
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct Manifest {
    pub files: Vec<Entry>,
    /// Letzte Prüfung (Unix-Sekunden).
    pub checked: Option<i64>,
}

impl Manifest {
    pub fn get(&self, kind: Kind) -> Option<&Entry> {
        self.files.iter().find(|e| e.kind == kind.id())
    }
}

/// Zustand im Speicher (`AppState::downloads`).
#[derive(Default)]
pub struct Downloads {
    manifest: RwLock<Option<Manifest>>,
    refreshing: AtomicBool,
}

fn dir(data: &Path) -> PathBuf {
    data.join("downloads")
}

/// Nur einfache Dateinamen (kein Pfad, kein führender Punkt).
pub fn safe_name(n: &str) -> bool {
    !n.is_empty() && n.len() <= 120 && !n.starts_with('.') && n.chars().all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '-' | '_'))
}

fn load_manifest(data: &Path) -> Manifest {
    std::fs::read(dir(data).join("manifest.json")).ok().and_then(|b| serde_json::from_slice(&b).ok()).unwrap_or_default()
}

fn save_manifest(data: &Path, m: &Manifest) -> std::io::Result<()> {
    let d = dir(data);
    let tmp = d.join("manifest.json.tmp");
    std::fs::write(&tmp, serde_json::to_vec_pretty(m).map_err(std::io::Error::other)?)?;
    std::fs::rename(tmp, d.join("manifest.json"))
}

/// Aktueller Stand (beim ersten Aufruf von der Platte gelesen).
pub fn manifest(s: &AppState) -> Manifest {
    if let Some(m) = s.downloads.manifest.read().unwrap_or_else(|p| p.into_inner()).clone() {
        return m;
    }
    let m = load_manifest(&s.cfg.data);
    *s.downloads.manifest.write().unwrap_or_else(|p| p.into_inner()) = Some(m.clone());
    m
}

// ---------- Release lesen ----------

/// Asset-URLs (Datei, Signatur) je App aus der GitHub-Release-JSON. Entwürfe/Vorabversionen ergeben `None`.
pub fn parse_release(v: &Value) -> Option<(Version, Vec<(Kind, String, String)>)> {
    if v.get("draft").and_then(Value::as_bool).unwrap_or(false) || v.get("prerelease").and_then(Value::as_bool).unwrap_or(false) {
        return None;
    }
    let version = Version::parse(v.get("tag_name")?.as_str()?.trim().trim_start_matches(['v', 'V'])).ok()?;
    let assets = v.get("assets").and_then(Value::as_array).cloned().unwrap_or_default();
    let find = |n: &str| assets.iter().find(|a| a.get("name").and_then(Value::as_str).is_some_and(|x| x.eq_ignore_ascii_case(n))).and_then(|a| a.get("browser_download_url")?.as_str().map(str::to_string));
    let files = Kind::ALL
        .iter()
        .filter_map(|&k| {
            let name = k.asset_name(&version);
            Some((k, find(&name)?, find(&format!("{name}.minisig"))?))
        })
        .collect();
    Some((version, files))
}

fn http() -> Result<reqwest::Client, String> {
    reqwest::Client::builder()
        .user_agent(format!("owndraw-server/{}", crate::update::CURRENT))
        .timeout(Duration::from_secs(600))
        .build()
        .map_err(|e| e.to_string())
}

async fn fetch(http: &reqwest::Client, url: &str, limit: u64) -> Result<Vec<u8>, String> {
    let mut resp = http.get(url).send().await.map_err(|e| format!("Download fehlgeschlagen: {e}"))?;
    if !resp.status().is_success() {
        return Err(format!("Download: Status {}", resp.status()));
    }
    let mut out = Vec::new();
    while let Some(chunk) = resp.chunk().await.map_err(|e| format!("Download abgebrochen: {e}"))? {
        if (out.len() + chunk.len()) as u64 > limit {
            return Err("Datei zu groß".into());
        }
        out.extend_from_slice(&chunk);
    }
    Ok(out)
}

/// Prüft das neueste Release und lädt neue Dateien. `verify(daten, signatur)` prüft die minisign-Signatur.
pub async fn refresh_with(s: &AppState, verify: impl Fn(&[u8], &str) -> Result<(), String>) -> Result<(), String> {
    if s.downloads.refreshing.swap(true, Ordering::SeqCst) {
        return Ok(());
    }
    let r = refresh_inner(s, verify).await;
    s.downloads.refreshing.store(false, Ordering::SeqCst);
    if let Err(e) = &r {
        eprintln!("Downloads: {e}");
    }
    r
}

/// Wie [`refresh_with`] mit dem Release-Schlüssel.
pub async fn refresh(s: &AppState) -> Result<(), String> {
    refresh_with(s, |d, sig| crate::update::verify_signature(crate::update::PUBKEY, d, sig)).await
}

async fn refresh_inner(s: &AppState, verify: impl Fn(&[u8], &str) -> Result<(), String>) -> Result<(), String> {
    let http = http()?;
    let resp = http.get(&s.cfg.release_url).header("Accept", "application/vnd.github+json").send().await.map_err(|e| format!("GitHub nicht erreichbar: {e}"))?;
    let mut m = manifest(s);
    let data = s.cfg.data.clone();
    let d = dir(&data);
    std::fs::create_dir_all(&d).map_err(|e| e.to_string())?;
    let release = if resp.status() == StatusCode::NOT_FOUND {
        None // noch kein Release
    } else if !resp.status().is_success() {
        return Err(format!("GitHub antwortet mit Status {}", resp.status()));
    } else {
        let v: Value = resp.json().await.map_err(|e| format!("Antwort von GitHub unlesbar: {e}"))?;
        parse_release(&v)
    };
    let mut errors = Vec::new();
    if let Some((version, files)) = release {
        for (kind, url, sig_url) in files {
            let name = kind.asset_name(&version);
            if !safe_name(&name) {
                continue;
            }
            let have = m.get(kind).is_some_and(|e| Version::parse(&e.version).is_ok_and(|v| v >= version) && d.join(&e.name).is_file());
            if have {
                continue;
            }
            let res: Result<Entry, String> = async {
                let bytes = fetch(&http, &url, MAX_FILE).await?;
                let sig = String::from_utf8(fetch(&http, &sig_url, 64 * 1024).await?).map_err(|_| "Signatur unlesbar".to_string())?;
                verify(&bytes, &sig)?;
                let sha256 = hex::encode(Sha256::digest(&bytes));
                let tmp = d.join(format!("{name}.part"));
                std::fs::write(&tmp, &bytes).map_err(|e| e.to_string())?;
                std::fs::rename(&tmp, d.join(&name)).map_err(|e| e.to_string())?;
                Ok(Entry { kind: kind.id().into(), version: version.to_string(), name: name.clone(), size: bytes.len() as u64, sha256 })
            }
            .await;
            match res {
                Ok(e) => {
                    m.files.retain(|x| x.kind != kind.id());
                    m.files.push(e);
                }
                Err(e) => errors.push(format!("{name}: {e}")),
            }
        }
    }
    m.checked = Some(now());
    // Alte Dateien aufräumen (alles, was nicht im Manifest steht).
    if let Ok(rd) = std::fs::read_dir(&d) {
        for f in rd.flatten() {
            let n = f.file_name().to_string_lossy().to_string();
            if n != "manifest.json" && !m.files.iter().any(|e| e.name == n) {
                let _ = std::fs::remove_file(f.path());
            }
        }
    }
    save_manifest(&data, &m).map_err(|e| e.to_string())?;
    *s.downloads.manifest.write().unwrap_or_else(|p| p.into_inner()) = Some(m);
    if errors.is_empty() {
        Ok(())
    } else {
        Err(errors.join("; "))
    }
}

/// Prüft kurz nach dem Start und danach stündlich.
pub fn spawn_refresh(state: AppState) {
    tokio::spawn(async move {
        tokio::time::sleep(Duration::from_secs(10)).await;
        loop {
            let _ = refresh(&state).await;
            tokio::time::sleep(REFRESH_EVERY).await;
        }
    });
}

// ---------- Seite und Dateien ----------

fn esc(s: &str) -> String {
    s.replace('&', "&amp;").replace('<', "&lt;").replace('>', "&gt;").replace('"', "&quot;").replace('\'', "&#39;")
}

/// Größe deutsch formatiert, z. B. „9,9 MB“.
pub fn fmt_size(b: u64) -> String {
    let mb = b as f64 / (1024.0 * 1024.0);
    if mb >= 1.0 {
        format!("{mb:.1} MB").replace('.', ",")
    } else {
        format!("{} KB", b.div_ceil(1024))
    }
}

fn card(kind: Kind, e: Option<&Entry>) -> String {
    let (title, sub, steps) = match kind {
        Kind::Android => (
            "Android",
            "APK · ab Android 8",
            "<ol><li>APK herunterladen und öffnen.</li><li>Fragt Android nach, die Installation aus dieser Quelle erlauben (Einstellungen › Apps › Unbekannte Apps installieren).</li><li>Installieren. Für ein Update die neue APK einfach darüber installieren, Boards bleiben erhalten.</li></ol>",
        ),
        Kind::Linux => (
            "Linux",
            "AppImage · x86_64",
            "<ol><li>AppImage herunterladen.</li><li>Ausführbar machen: im Dateimanager unter Eigenschaften › Berechtigungen oder im Terminal <code>chmod +x OwnDraw-*.AppImage</code>.</li><li>Per Doppelklick starten oder <code>./OwnDraw-*.AppImage</code>. Startet es nicht, fehlt meist FUSE (Paket <code>libfuse2</code>).</li></ol>",
        ),
    };
    let body = match e {
        Some(e) => format!(
            "<a class=\"btn\" href=\"/download/{n}\" download>Herunterladen</a>\
             <p class=\"meta\">Version {v} · {s}</p>\
             <p class=\"hash\"><span>SHA-256</span><code>{h}</code></p>{steps}",
            n = esc(&e.name),
            v = esc(&e.version),
            s = fmt_size(e.size),
            h = esc(&e.sha256),
        ),
        None => "<p class=\"na\">Noch nicht verfügbar.</p>".to_string(),
    };
    format!("<section class=\"card\" id=\"{id}\"><h2>{title}</h2><p class=\"sub\">{sub}</p>{body}</section>", id = kind.id())
}

/// HTML der Seite für einen Stand.
pub fn render(m: &Manifest) -> String {
    let cards: String = Kind::ALL.iter().map(|&k| card(k, m.get(k))).collect();
    PAGE_HTML.replace("__CARDS__", &cards)
}

/// Wartung aktiv und kein Admin? Dann die Antwort für Nicht-Admins.
async fn maintenance_block(s: &AppState, req: Request, page: bool) -> Result<Request, Response> {
    let Ok(m) = crate::notices::current(s).await else { return Ok(req) };
    if !m.on(now()) {
        return Ok(req);
    }
    let (mut parts, body) = req.into_parts();
    if crate::notices::is_admin(s, &mut parts).await {
        return Ok(Request::from_parts(parts, body));
    }
    Err(if page { crate::notices::maintenance_page(&m) } else { crate::error::ApiError::maintenance(m.text()).into_response() })
}

pub async fn page(State(s): State<AppState>, req: Request) -> Response {
    if let Err(r) = maintenance_block(&s, req, true).await {
        return r;
    }
    let mut r = Html(render(&manifest(&s))).into_response();
    let h = r.headers_mut();
    h.insert(CACHE_CONTROL, HeaderValue::from_static("no-cache"));
    h.insert(
        CONTENT_SECURITY_POLICY,
        HeaderValue::from_static("default-src 'none'; style-src 'unsafe-inline'; img-src 'self'; base-uri 'none'; form-action 'none'; frame-ancestors 'none'"),
    );
    r
}

fn text(status: StatusCode, msg: &'static str) -> Response {
    (status, [(CONTENT_TYPE, "text/plain; charset=utf-8")], msg).into_response()
}

pub async fn file(State(s): State<AppState>, UrlPath(name): UrlPath<String>, ClientIp(ip): ClientIp, req: Request) -> Response {
    let req = match maintenance_block(&s, req, false).await {
        Ok(r) => r,
        Err(r) => return r,
    };
    if !safe_name(&name) {
        return text(StatusCode::NOT_FOUND, "Diese Datei gibt es nicht.");
    }
    let m = manifest(&s);
    let Some(entry) = m.files.iter().find(|e| e.name == name) else {
        return text(StatusCode::NOT_FOUND, "Diese Datei gibt es nicht.");
    };
    let kind = Kind::ALL.into_iter().find(|k| k.id() == entry.kind).unwrap_or(Kind::Linux);
    if req.method() == Method::GET && !s.limiter.hit(&format!("download:{ip}"), RATE_LIMIT, RATE_WINDOW) {
        return text(StatusCode::TOO_MANY_REQUESTS, "Zu viele Downloads. Bitte in einigen Minuten erneut versuchen.");
    }
    let path = dir(&s.cfg.data).join(&entry.name);
    let mut r = match ServeFile::new(path).oneshot(req).await {
        Ok(r) => r.map(Body::new),
        Err(_) => return text(StatusCode::INTERNAL_SERVER_ERROR, "Datei nicht lesbar."),
    };
    if r.status() == StatusCode::NOT_FOUND {
        return text(StatusCode::NOT_FOUND, "Diese Datei gibt es nicht.");
    }
    let h = r.headers_mut();
    h.insert(CONTENT_TYPE, HeaderValue::from_static(kind.content_type()));
    if let Ok(v) = HeaderValue::from_str(&format!("attachment; filename=\"{}\"", entry.name)) {
        h.insert(CONTENT_DISPOSITION, v);
    }
    h.insert(CACHE_CONTROL, HeaderValue::from_static("public, max-age=86400"));
    r
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn release_lesen() {
        let rel = json!({"tag_name": "v4.17.0", "assets": [
            {"name": "OwnDraw-4.17.0-x86_64.AppImage", "browser_download_url": "http://x/a"},
            {"name": "OwnDraw-4.17.0-x86_64.AppImage.minisig", "browser_download_url": "http://x/as"},
            {"name": "OwnDraw-4.17.0.apk", "browser_download_url": "http://x/p"}]});
        let (v, files) = parse_release(&rel).unwrap();
        assert_eq!(v.to_string(), "4.17.0");
        // APK ohne Signatur wird nicht angeboten
        assert_eq!(files, vec![(Kind::Linux, "http://x/a".to_string(), "http://x/as".to_string())]);
        assert!(parse_release(&json!({"tag_name": "v1.0.0", "prerelease": true})).is_none());
    }

    #[test]
    fn namen_und_groesse() {
        assert!(safe_name("OwnDraw-4.17.0-x86_64.AppImage"));
        for bad in ["", "../x", "a/b", ".manifest", "a b", "x%2F"] {
            assert!(!safe_name(bad), "{bad}");
        }
        assert_eq!(fmt_size(10_380_902), "9,9 MB");
        assert_eq!(fmt_size(1000), "1 KB");
    }

    #[test]
    fn seite_ohne_dateien() {
        let html = render(&Manifest::default());
        assert_eq!(html.matches("Noch nicht verfügbar.").count(), 2);
        assert!(!html.contains("__CARDS__"));
    }
}
