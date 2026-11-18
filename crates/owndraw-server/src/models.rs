//! Erkennungsmodelle für die Handschrift-Erkennung (Werkzeug „Umwandeln“ der Web-App).
//!
//! Der Server erkennt nichts selbst: Er legt Modell-Pakete unter `data/models/<id>/` ab (`manifest.json` + Dateien,
//! Format siehe `owndraw_core::hw::Manifest` und `tools/handschrift/pakete-bauen.py`) und liefert sie aus:
//! - `GET /api/models`: Liste der vollständigen Pakete (öffentlich, Manifeste)
//! - `GET /api/models/{id}/{datei}`: eine Datei (Range, ETag/Last-Modified, Cache-Control)
//!
//! Admin (mit 2FA): `GET /api/admin/models` (Pakete, Katalog, laufende Aufträge), `POST /api/admin/models/upload`
//! (tar, tar.gz oder zip mit `manifest.json`, Rohdaten im Body), `POST /api/admin/models/fetch` (aus URL: Manifest
//! oder Archiv, sha256 und optional minisign geprüft), `POST /api/admin/models/catalog/{id}` (Eintrag des
//! eingebauten Katalogs von Hugging Face/jsDelivr laden), `DELETE /api/admin/models/{id}`.
//! Jede Datei wird gegen Größe und sha256 aus dem Manifest geprüft; nur erlaubte Lizenzen (MIT, Apache-2.0, AFL-3.0 …).

use crate::auth::{AdminAuth, J};
use crate::error::{ApiError, ApiResult};
use crate::state::AppState;
use axum::body::Body;
use axum::extract::{Path as UrlPath, Request, State};
use axum::http::header::{CACHE_CONTROL, CONTENT_TYPE};
use axum::http::{HeaderValue, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::Json;
use futures_util::StreamExt;
use owndraw_core::hw::{safe_name, Manifest};
use serde::Deserialize;
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::time::Duration;
use tower::ServiceExt;
use tower_http::services::ServeFile;

/// Größtes Paket (Upload bzw. Archiv aus URL).
pub const MAX_PACKAGE: u64 = 4 * 1024 * 1024 * 1024;
/// Größte Einzeldatei.
const MAX_FILE: u64 = 3 * 1024 * 1024 * 1024;
/// Höchstens so viele Dateien je Paket.
const MAX_FILES: usize = 64;
/// Eingebauter Katalog (Manifeste mit Quell-URLs, direkt ladbar).
const KATALOG: &str = include_str!("models_katalog.json");

pub fn dir(data: &Path) -> PathBuf {
    data.join("models")
}

fn hex(b: &[u8]) -> String {
    b.iter().map(|x| format!("{x:02x}")).collect()
}

/// Katalog: Manifeste (mit `url` je Datei).
pub fn katalog() -> Vec<Manifest> {
    let v: Value = serde_json::from_str(KATALOG).unwrap_or(Value::Null);
    v.as_array().into_iter().flatten().filter_map(|m| Manifest::from_value(m.clone()).ok()).collect()
}

/// Liest das Manifest eines installierten Pakets und prüft, ob alle Dateien mit passender Größe da sind
/// (sha256 wird beim Installieren geprüft, nicht bei jedem Abruf).
pub fn read_package(models: &Path, id: &str) -> Result<Manifest, String> {
    if !safe_name(id) {
        return Err("ungültige ID".into());
    }
    let d = models.join(id);
    let bytes = std::fs::read(d.join("manifest.json")).map_err(|_| "manifest.json fehlt".to_string())?;
    let m = Manifest::parse(&bytes)?;
    if m.id != id {
        return Err(format!("Ordner {id} enthält das Paket {}", m.id));
    }
    for f in &m.dateien {
        let len = std::fs::metadata(d.join(&f.name)).map(|x| x.len()).map_err(|_| format!("{} fehlt", f.name))?;
        if len != f.groesse {
            return Err(format!("{}: {len} statt {} Bytes", f.name, f.groesse));
        }
    }
    Ok(m)
}

/// Alle Paketordner (ohne Zwischenordner) mit Ergebnis der Prüfung.
pub fn installed(models: &Path) -> Vec<(String, Result<Manifest, String>)> {
    let mut v: Vec<(String, Result<Manifest, String>)> = std::fs::read_dir(models)
        .into_iter()
        .flatten()
        .flatten()
        .filter(|e| e.file_type().map(|t| t.is_dir()).unwrap_or(false))
        .filter_map(|e| e.file_name().to_str().map(String::from))
        .filter(|n| !n.starts_with('.'))
        .map(|n| {
            let r = read_package(models, &n);
            (n, r)
        })
        .collect();
    v.sort_by(|a, b| a.0.cmp(&b.0));
    v
}

/// Manifest für die App: Rohdaten plus Gesamtgröße, ohne Quell-URLs der Dateien.
fn public_json(m: &Manifest) -> Value {
    let mut v = m.raw.clone();
    if let Some(ds) = v.get_mut("dateien").and_then(|d| d.as_array_mut()) {
        for d in ds {
            if let Some(o) = d.as_object_mut() {
                o.remove("url");
            }
        }
    }
    v["groesse"] = json!(m.groesse());
    v
}

// ---------------------------------------------------------------- öffentlich

pub async fn list(State(s): State<AppState>) -> Json<Value> {
    let models = dir(&s.cfg.data);
    let pakete = tokio::task::spawn_blocking(move || installed(&models)).await.unwrap_or_default();
    let v: Vec<Value> = pakete.iter().filter_map(|(_, r)| r.as_ref().ok()).map(public_json).collect();
    Json(json!({"pakete": v}))
}

fn content_type(name: &str) -> &'static str {
    match name.rsplit('.').next().unwrap_or("") {
        "mjs" | "js" => "text/javascript; charset=utf-8",
        "wasm" => "application/wasm",
        "json" => "application/json",
        "txt" | "yml" | "yaml" | "md" => "text/plain; charset=utf-8",
        _ => "application/octet-stream",
    }
}

pub async fn file(State(s): State<AppState>, UrlPath((id, name)): UrlPath<(String, String)>, req: Request) -> Response {
    let nf = || ApiError::not_found("Diese Datei gibt es nicht.").into_response();
    if !safe_name(&id) || !safe_name(&name) {
        return nf();
    }
    let models = dir(&s.cfg.data);
    let Ok(m) = read_package(&models, &id) else { return nf() };
    if name != "manifest.json" && !m.dateien.iter().any(|d| d.name == name) {
        return nf();
    }
    let mut r = match ServeFile::new(models.join(&id).join(&name)).oneshot(req).await {
        Ok(r) => r.map(Body::new),
        Err(_) => return ApiError::internal("Datei nicht lesbar").into_response(),
    };
    if r.status() == StatusCode::NOT_FOUND {
        return nf();
    }
    let h = r.headers_mut();
    h.insert(CONTENT_TYPE, HeaderValue::from_static(content_type(&name)));
    h.insert(CACHE_CONTROL, HeaderValue::from_static("public, max-age=3600"));
    r
}

// ---------------------------------------------------------------- Installieren

/// Prüft den Zwischenordner (manifest.json + Dateien mit Größe/sha256), entfernt fremde Dateien und ersetzt
/// das Paket gleicher ID. Läuft blockierend.
pub fn install_dir(models: &Path, staging: &Path) -> Result<Manifest, String> {
    let bytes = std::fs::read(staging.join("manifest.json")).map_err(|_| "Im Paket fehlt manifest.json".to_string())?;
    let m = Manifest::parse(&bytes)?;
    if m.dateien.len() > MAX_FILES {
        return Err("Zu viele Dateien im Paket".into());
    }
    for f in &m.dateien {
        let p = staging.join(&f.name);
        let mut file = std::fs::File::open(&p).map_err(|_| format!("Im Paket fehlt {}", f.name))?;
        let mut h = Sha256::new();
        let mut buf = vec![0u8; 1 << 20];
        let mut n = 0u64;
        loop {
            let k = file.read(&mut buf).map_err(|e| format!("{}: {e}", f.name))?;
            if k == 0 {
                break;
            }
            n += k as u64;
            h.update(&buf[..k]);
        }
        if n != f.groesse {
            return Err(format!("{}: {n} statt {} Bytes", f.name, f.groesse));
        }
        if hex(&h.finalize()) != f.sha256 {
            return Err(format!("{}: Prüfsumme (sha256) stimmt nicht", f.name));
        }
    }
    // nur Manifest und gelistete Dateien behalten
    for e in std::fs::read_dir(staging).map_err(|e| e.to_string())?.flatten() {
        let n = e.file_name().to_string_lossy().into_owned();
        if n != "manifest.json" && !m.dateien.iter().any(|d| d.name == n) {
            let _ = if e.path().is_dir() { std::fs::remove_dir_all(e.path()) } else { std::fs::remove_file(e.path()) };
        }
    }
    std::fs::create_dir_all(models).map_err(|e| e.to_string())?;
    let target = models.join(&m.id);
    let old = models.join(format!(".alt-{}", uuid::Uuid::new_v4()));
    if target.exists() {
        std::fs::rename(&target, &old).map_err(|e| format!("Altes Paket: {e}"))?;
    }
    if let Err(e) = std::fs::rename(staging, &target) {
        let _ = std::fs::rename(&old, &target);
        return Err(format!("Installieren: {e}"));
    }
    let _ = std::fs::remove_dir_all(&old);
    Ok(m)
}

/// Neuer Zwischenordner unter `data/models/.neu-<uuid>`.
fn staging(models: &Path) -> Result<PathBuf, String> {
    let p = models.join(format!(".neu-{}", uuid::Uuid::new_v4()));
    std::fs::create_dir_all(&p).map_err(|e| e.to_string())?;
    Ok(p)
}

/// Name einer Archiv-Datei im Paket: führendes `./` und ein gemeinsamer Oberordner fallen weg.
fn flat_name(path: &str, prefix: &str) -> Option<String> {
    let p = path.trim_start_matches("./");
    let p = p.strip_prefix(prefix).unwrap_or(p);
    (!p.is_empty() && !p.contains('/') && safe_name(p)).then(|| p.to_string())
}

/// Gemeinsamer Oberordner (`paket/`) aller Einträge, sonst leer.
fn common_prefix(names: &[String]) -> String {
    let files: Vec<&str> = names.iter().map(|n| n.trim_start_matches("./")).filter(|n| !n.ends_with('/') && !n.is_empty()).collect();
    let Some(first) = files.first() else { return String::new() };
    let Some((top, _)) = first.split_once('/') else { return String::new() };
    let pre = format!("{top}/");
    if files.iter().all(|n| n.starts_with(&pre)) {
        pre
    } else {
        String::new()
    }
}

/// Entpackt tar, tar.gz oder zip flach in `out` (nur sichere Dateinamen, Größen begrenzt).
pub fn extract(archive: &Path, out: &Path) -> Result<(), String> {
    let mut head = [0u8; 512];
    let n = std::fs::File::open(archive).and_then(|mut f| f.read(&mut head)).map_err(|e| e.to_string())?;
    let head = &head[..n];
    let mut total = 0u64;
    let mut count = 0usize;
    let mut put = |name: &str, r: &mut dyn Read| -> Result<(), String> {
        count += 1;
        if count > MAX_FILES {
            return Err("Zu viele Dateien im Archiv".into());
        }
        let mut f = std::fs::File::create(out.join(name)).map_err(|e| e.to_string())?;
        let n = std::io::copy(&mut r.take(MAX_FILE + 1), &mut f).map_err(|e| format!("{name}: {e}"))?;
        total += n;
        if n > MAX_FILE || total > MAX_PACKAGE {
            return Err("Archiv zu groß".into());
        }
        Ok(())
    };
    if head.starts_with(b"PK") {
        let f = std::fs::File::open(archive).map_err(|e| e.to_string())?;
        let mut z = zip::ZipArchive::new(f).map_err(|e| format!("ZIP unlesbar: {e}"))?;
        let names: Vec<String> = (0..z.len()).filter_map(|i| z.by_index_raw(i).ok().and_then(|e| e.name().ok().map(|n| n.to_string()))).collect();
        let pre = common_prefix(&names);
        for i in 0..z.len() {
            let mut e = z.by_index(i).map_err(|e| format!("ZIP: {e}"))?;
            if e.is_dir() {
                continue;
            }
            let Some(name) = e.name().ok().and_then(|n| flat_name(&n, &pre)) else { continue };
            put(&name, &mut e)?;
        }
        return Ok(());
    }
    let gz = head.starts_with(&[0x1f, 0x8b]);
    let is_tar = gz || (head.len() > 262 && &head[257..262] == b"ustar");
    if !is_tar {
        return Err("Unbekanntes Archiv (erlaubt: .tar, .tar.gz, .zip)".into());
    }
    let open = || -> Result<tar::Archive<Box<dyn Read>>, String> {
        let f = std::fs::File::open(archive).map_err(|e| e.to_string())?;
        let r: Box<dyn Read> = if gz { Box::new(flate2::read::GzDecoder::new(f)) } else { Box::new(f) };
        Ok(tar::Archive::new(r))
    };
    // erster Durchgang: Namen (für den gemeinsamen Oberordner)
    let mut names = Vec::new();
    for e in open()?.entries().map_err(|e| format!("TAR: {e}"))? {
        let e = e.map_err(|e| format!("TAR: {e}"))?;
        if e.header().entry_type().is_file() {
            names.push(e.path().map_err(|e| e.to_string())?.to_string_lossy().into_owned());
        }
    }
    let pre = common_prefix(&names);
    for e in open()?.entries().map_err(|e| format!("TAR: {e}"))? {
        let mut e = e.map_err(|e| format!("TAR: {e}"))?;
        if !e.header().entry_type().is_file() {
            continue;
        }
        let p = e.path().map_err(|e| e.to_string())?.to_string_lossy().into_owned();
        let Some(name) = flat_name(&p, &pre) else { continue };
        put(&name, &mut e)?;
    }
    Ok(())
}

// ---------------------------------------------------------------- Aufträge (aus URL / Katalog)

#[derive(Clone, Debug)]
pub struct Job {
    pub id: String,
    pub quelle: String,
    /// `laeuft`, `fertig`, `fehler`
    pub status: String,
    pub text: String,
    pub geladen: u64,
    pub gesamt: u64,
}

impl Job {
    fn json(&self) -> Value {
        json!({"id": self.id, "quelle": self.quelle, "status": self.status, "text": self.text, "geladen": self.geladen, "gesamt": self.gesamt})
    }
}

static JOBS: Mutex<Vec<Job>> = Mutex::new(Vec::new());

fn job_update(id: &str, f: impl FnOnce(&mut Job)) {
    let mut j = JOBS.lock().unwrap_or_else(|p| p.into_inner());
    if let Some(x) = j.iter_mut().find(|x| x.id == id) {
        f(x);
    }
}

fn job_new(quelle: &str) -> String {
    let id = uuid::Uuid::new_v4().to_string();
    let mut j = JOBS.lock().unwrap_or_else(|p| p.into_inner());
    if j.len() >= 20 {
        j.remove(0);
    }
    j.push(Job { id: id.clone(), quelle: quelle.into(), status: "laeuft".into(), text: "Startet …".into(), geladen: 0, gesamt: 0 });
    id
}

pub fn jobs() -> Vec<Job> {
    JOBS.lock().unwrap_or_else(|p| p.into_inner()).clone()
}

fn http() -> Result<reqwest::Client, String> {
    reqwest::Client::builder()
        .user_agent(format!("owndraw-server/{}", crate::update::CURRENT))
        .connect_timeout(Duration::from_secs(30))
        .timeout(Duration::from_secs(4 * 3600))
        .build()
        .map_err(|e| e.to_string())
}

/// Lädt `url` nach `dest`; liefert (Größe, sha256). `job` bekommt den Fortschritt.
async fn download(http: &reqwest::Client, url: &str, dest: &Path, limit: u64, job: &str) -> Result<(u64, String), String> {
    let mut resp = http.get(url).send().await.map_err(|e| format!("{url}: {e}"))?;
    if !resp.status().is_success() {
        return Err(format!("{url}: Status {}", resp.status()));
    }
    let mut f = std::fs::File::create(dest).map_err(|e| e.to_string())?;
    let mut h = Sha256::new();
    let mut n = 0u64;
    while let Some(c) = resp.chunk().await.map_err(|e| format!("Download abgebrochen: {e}"))? {
        n += c.len() as u64;
        if n > limit {
            return Err("Datei zu groß".into());
        }
        h.update(&c);
        f.write_all(&c).map_err(|e| e.to_string())?;
        let k = c.len() as u64;
        job_update(job, |j| j.geladen += k);
    }
    Ok((n, hex(&h.finalize())))
}

/// Paket aus einem Manifest mit Datei-URLs (absolut oder relativ zur Manifest-URL) laden und installieren.
async fn fetch_manifest(models: PathBuf, m: Manifest, base: Option<String>, job: String) -> Result<Manifest, String> {
    let http = http()?;
    let st = staging(&models)?;
    let res = async {
        job_update(&job, |j| {
            j.gesamt = m.groesse();
            j.text = format!("Lade {} ({} Dateien) …", m.name, m.dateien.len());
        });
        for d in &m.dateien {
            let url = if !d.url.is_empty() {
                d.url.clone()
            } else if let Some(b) = &base {
                reqwest::Url::parse(b).and_then(|u| u.join(&d.name)).map_err(|e| e.to_string())?.to_string()
            } else {
                return Err(format!("{}: keine Quell-URL", d.name));
            };
            if !url.starts_with("https://") && !url.starts_with("http://") {
                return Err(format!("{}: nur http(s)-Adressen", d.name));
            }
            let (n, h) = download(&http, &url, &st.join(&d.name), d.groesse.min(MAX_FILE), &job).await?;
            if n != d.groesse || h != d.sha256 {
                return Err(format!("{}: Größe oder Prüfsumme (sha256) stimmt nicht", d.name));
            }
        }
        std::fs::write(st.join("manifest.json"), serde_json::to_vec_pretty(&m.raw).unwrap_or_default()).map_err(|e| e.to_string())?;
        let (models2, st2) = (models.clone(), st.clone());
        tokio::task::spawn_blocking(move || install_dir(&models2, &st2)).await.map_err(|e| e.to_string())?
    }
    .await;
    if res.is_err() {
        let _ = std::fs::remove_dir_all(&st);
    }
    res
}

/// Archiv aus URL laden (sha256 und optional minisign prüfen), entpacken, installieren.
async fn fetch_archive(models: PathBuf, req: FetchReq, job: String) -> Result<Manifest, String> {
    let http = http()?;
    std::fs::create_dir_all(&models).map_err(|e| e.to_string())?;
    let tmp = models.join(format!(".dl-{}", uuid::Uuid::new_v4()));
    let res = async {
        job_update(&job, |j| j.text = "Lade Archiv …".into());
        let (_, h) = download(&http, &req.url, &tmp, MAX_PACKAGE, &job).await?;
        if let Some(want) = req.sha256.as_deref().map(str::trim).filter(|s| !s.is_empty()) {
            if !want.eq_ignore_ascii_case(&h) {
                return Err("Prüfsumme (sha256) des Archivs stimmt nicht".into());
            }
        }
        if let (Some(su), Some(pk)) = (req.minisig_url.as_deref().filter(|s| !s.is_empty()), req.pubkey.as_deref().filter(|s| !s.is_empty())) {
            let sig = http.get(su).send().await.map_err(|e| e.to_string())?.text().await.map_err(|e| e.to_string())?;
            let data = std::fs::read(&tmp).map_err(|e| e.to_string())?;
            crate::update::verify_signature(pk, &data, &sig)?;
        }
        job_update(&job, |j| j.text = "Entpacke und prüfe …".into());
        let (models2, tmp2) = (models.clone(), tmp.clone());
        tokio::task::spawn_blocking(move || {
            let st = staging(&models2)?;
            let r = extract(&tmp2, &st).and_then(|_| install_dir(&models2, &st));
            if r.is_err() {
                let _ = std::fs::remove_dir_all(&st);
            }
            r
        })
        .await
        .map_err(|e| e.to_string())?
    }
    .await;
    let _ = std::fs::remove_file(&tmp);
    res
}

fn spawn_job<F>(quelle: &str, fut: impl FnOnce(String) -> F) -> String
where
    F: std::future::Future<Output = Result<Manifest, String>> + Send + 'static,
{
    let id = job_new(quelle);
    let f = fut(id.clone());
    let jid = id.clone();
    tokio::spawn(async move {
        let r = f.await;
        job_update(&jid, |j| match r {
            Ok(m) => {
                j.status = "fertig".into();
                j.text = format!("{} installiert", m.name);
            }
            Err(e) => {
                j.status = "fehler".into();
                j.text = e;
            }
        });
    });
    id
}

// ---------------------------------------------------------------- Admin

pub async fn admin_list(State(s): State<AppState>, _a: AdminAuth) -> Json<Value> {
    let models = dir(&s.cfg.data);
    let pk = tokio::task::spawn_blocking(move || installed(&models)).await.unwrap_or_default();
    let pakete: Vec<Value> = pk
        .iter()
        .map(|(id, r)| match r {
            Ok(m) => json!({"id": id, "ok": true, "name": m.name, "art": m.art, "stufen": m.stufen.iter().map(|x| x.id()).collect::<Vec<_>>(),
                "version": m.version, "lizenz": m.lizenz, "lizenz_datei": m.lizenz_datei, "hinweis": m.lizenz_hinweis, "quelle": m.quelle,
                "groesse": m.groesse(), "dateien": m.dateien.len()}),
            Err(e) => json!({"id": id, "ok": false, "fehler": e}),
        })
        .collect();
    let kat: Vec<Value> = katalog()
        .iter()
        .map(|m| json!({"id": m.id, "name": m.name, "art": m.art, "stufen": m.stufen.iter().map(|x| x.id()).collect::<Vec<_>>(), "lizenz": m.lizenz,
            "hinweis": m.lizenz_hinweis, "quelle": m.quelle, "groesse": m.groesse(), "installiert": pk.iter().any(|(id, r)| *id == m.id && r.is_ok())}))
        .collect();
    let auftraege: Vec<Value> = jobs().iter().rev().map(Job::json).collect();
    Json(json!({"pakete": pakete, "katalog": kat, "auftraege": auftraege}))
}

/// Paket hochladen: Rohdaten (tar, tar.gz, zip) im Body.
pub async fn upload(State(s): State<AppState>, _a: AdminAuth, body: Body) -> ApiResult<Json<Value>> {
    let models = dir(&s.cfg.data);
    std::fs::create_dir_all(&models).map_err(ApiError::internal)?;
    let tmp = models.join(format!(".up-{}", uuid::Uuid::new_v4()));
    let res: ApiResult<Manifest> = async {
        let mut f = std::fs::File::create(&tmp).map_err(ApiError::internal)?;
        let mut st = body.into_data_stream();
        let mut n = 0u64;
        while let Some(c) = st.next().await {
            let c = c.map_err(|_| ApiError::invalid("Upload abgebrochen."))?;
            n += c.len() as u64;
            if n > MAX_PACKAGE {
                return Err(ApiError::too_large("Das Paket ist zu groß."));
            }
            f.write_all(&c).map_err(ApiError::internal)?;
        }
        drop(f);
        if n == 0 {
            return Err(ApiError::invalid("Leerer Upload."));
        }
        let (m2, t2) = (models.clone(), tmp.clone());
        tokio::task::spawn_blocking(move || {
            let st = staging(&m2)?;
            let r = extract(&t2, &st).and_then(|_| install_dir(&m2, &st));
            if r.is_err() {
                let _ = std::fs::remove_dir_all(&st);
            }
            r
        })
        .await
        .map_err(ApiError::internal)?
        .map_err(ApiError::invalid)
    }
    .await;
    let _ = std::fs::remove_file(&tmp);
    let m = res?;
    Ok(Json(json!({"ok": true, "id": m.id, "name": m.name, "groesse": m.groesse()})))
}

#[derive(Deserialize, Clone)]
pub struct FetchReq {
    pub url: String,
    /// sha256 des Archivs (optional).
    pub sha256: Option<String>,
    /// minisign: URL der `.minisig`-Datei und öffentlicher Schlüssel (optional, beide nötig).
    pub minisig_url: Option<String>,
    pub pubkey: Option<String>,
}

/// Aus URL laden: `…/manifest.json` (Dateien daneben bzw. mit eigener `url`) oder ein Archiv.
pub async fn fetch(State(s): State<AppState>, _a: AdminAuth, J(req): J<FetchReq>) -> ApiResult<Json<Value>> {
    let url = req.url.trim().to_string();
    if !(url.starts_with("https://") || url.starts_with("http://")) || url.len() > 2000 {
        return Err(ApiError::invalid("Bitte eine http(s)-Adresse angeben."));
    }
    let models = dir(&s.cfg.data);
    let is_manifest = url.split('?').next().unwrap_or("").ends_with(".json");
    let id = if is_manifest {
        let bytes = http()
            .map_err(ApiError::internal)?
            .get(&url)
            .send()
            .await
            .map_err(|e| ApiError::upstream(format!("Manifest: {e}")))?
            .error_for_status()
            .map_err(|e| ApiError::upstream(format!("Manifest: {e}")))?
            .bytes()
            .await
            .map_err(|e| ApiError::upstream(e.to_string()))?;
        if bytes.len() > 1024 * 1024 {
            return Err(ApiError::invalid("Manifest zu groß."));
        }
        let m = Manifest::parse(&bytes).map_err(ApiError::invalid)?;
        spawn_job(&url, |job| fetch_manifest(models, m, Some(url.clone()), job))
    } else {
        let r = FetchReq { url: url.clone(), ..req };
        spawn_job(&url, |job| fetch_archive(models, r, job))
    };
    Ok(Json(json!({"auftrag": id})))
}

/// Eintrag des eingebauten Katalogs laden.
pub async fn from_catalog(State(s): State<AppState>, _a: AdminAuth, UrlPath(id): UrlPath<String>) -> ApiResult<Json<Value>> {
    let m = katalog().into_iter().find(|m| m.id == id).ok_or_else(|| ApiError::not_found("Diesen Katalogeintrag gibt es nicht."))?;
    let models = dir(&s.cfg.data);
    let q = m.quelle.clone();
    let job = spawn_job(&q, |job| fetch_manifest(models, m, None, job));
    Ok(Json(json!({"auftrag": job})))
}

pub async fn delete(State(s): State<AppState>, _a: AdminAuth, UrlPath(id): UrlPath<String>) -> ApiResult<Json<Value>> {
    if !safe_name(&id) {
        return Err(ApiError::invalid("Ungültige ID."));
    }
    let d = dir(&s.cfg.data).join(&id);
    if !d.is_dir() {
        return Err(ApiError::not_found("Dieses Paket gibt es nicht."));
    }
    std::fs::remove_dir_all(&d).map_err(ApiError::internal)?;
    Ok(Json(json!({"ok": true})))
}

/// Router für den Upload: ohne die allgemeine Body-Grenze (Pakete sind bis zu einige hundert MB groß).
pub fn upload_router(state: AppState) -> axum::Router {
    axum::Router::new()
        .route("/api/admin/models/upload", axum::routing::post(upload))
        .layer(axum::middleware::from_fn(crate::auth::csrf))
        .with_state(state)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn katalog_ist_gueltig() {
        let k = katalog();
        assert!(k.len() >= 3, "{}", k.len());
        assert!(k.iter().any(|m| m.art == "laufzeit"));
        for m in &k {
            assert!(m.dateien.iter().all(|d| d.url.starts_with("https://")), "{}", m.id);
            assert!(!m.lizenz_datei.is_empty(), "{}: Lizenztext", m.id);
        }
    }

    #[test]
    fn namen_im_archiv() {
        assert_eq!(flat_name("./paket/encoder.onnx", "paket/"), Some("encoder.onnx".into()));
        assert_eq!(flat_name("encoder.onnx", ""), Some("encoder.onnx".into()));
        assert_eq!(flat_name("a/b/c.onnx", ""), None);
        assert_eq!(flat_name("../x", ""), None);
        assert_eq!(flat_name(".versteckt", ""), None);
        assert_eq!(common_prefix(&["p/a".into(), "p/b".into(), "p/".into()]), "p/");
        assert_eq!(common_prefix(&["p/a".into(), "b".into()]), "");
        assert_eq!(content_type("ort.wasm.min.mjs"), "text/javascript; charset=utf-8");
        assert_eq!(content_type("x.onnx"), "application/octet-stream");
    }
}
