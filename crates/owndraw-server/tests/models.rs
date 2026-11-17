//! Integrationstests: Erkennungsmodelle (`/api/models`, Admin-Abschnitt „Erkennungsmodelle“).

use axum::body::{to_bytes, Body};
use axum::http::{header, Method, Request, StatusCode};
use axum::Router;
use owndraw_server::config::Config;
use owndraw_server::db::Db;
use owndraw_server::state::AppState;
use owndraw_server::{auth, build_app, totp};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::time::Duration;
use tower::ServiceExt;

struct App {
    router: Router,
    state: AppState,
    _dir: tempfile::TempDir,
}

struct Resp {
    status: StatusCode,
    headers: axum::http::HeaderMap,
    bytes: Vec<u8>,
}

impl Resp {
    fn json(&self) -> Value {
        serde_json::from_slice(&self.bytes).unwrap_or_else(|_| panic!("kein JSON: {}", String::from_utf8_lossy(&self.bytes)))
    }
}

impl App {
    fn new() -> Self {
        let dir = tempfile::tempdir().unwrap();
        let web = dir.path().join("web");
        std::fs::create_dir_all(&web).unwrap();
        std::fs::write(web.join("index.html"), "<html>OwnDraw Web</html>").unwrap();
        let cfg = Config {
            data: dir.path().join("data"),
            web,
            bind: "127.0.0.1:0".into(),
            // klein: der Upload von Modell-Paketen muss trotzdem durchgehen
            max_board_bytes: 1024,
            rate_limit: 1000,
            rate_window_secs: 600,
            release_url: "http://127.0.0.1:1/none".into(),
        };
        std::fs::create_dir_all(cfg.boards_dir()).unwrap();
        let state = AppState::new(Db::memory().unwrap(), cfg);
        App { router: build_app(state.clone()), state, _dir: dir }
    }

    async fn send(&self, method: Method, path: &str, cookie: Option<&str>, headers: &[(&str, &str)], body: Body) -> Resp {
        let mut b = Request::builder().method(method).uri(path).header("x-owndraw", "1");
        if let Some(c) = cookie {
            b = b.header(header::COOKIE, c);
        }
        for (k, v) in headers {
            b = b.header(*k, *v);
        }
        let resp = self.router.clone().oneshot(b.body(body).unwrap()).await.unwrap();
        let (parts, body) = resp.into_parts();
        let bytes = to_bytes(body, 256 * 1024 * 1024).await.unwrap().to_vec();
        Resp { status: parts.status, headers: parts.headers, bytes }
    }

    async fn get(&self, path: &str, cookie: Option<&str>) -> Resp {
        self.send(Method::GET, path, cookie, &[], Body::empty()).await
    }

    async fn post_json(&self, path: &str, cookie: Option<&str>, v: Value) -> Resp {
        self.send(Method::POST, path, cookie, &[("content-type", "application/json")], Body::from(v.to_string())).await
    }

    async fn admin(&self) -> String {
        auth::create_admin(&self.state.db, "admin@example.org", "adminpasswort").await.unwrap();
        let r = self.post_json("/api/auth/login", None, json!({"email": "admin@example.org", "password": "adminpasswort"})).await;
        let c = r.headers.get(header::SET_COOKIE).unwrap().to_str().unwrap().split(';').next().unwrap().to_string();
        let secret = self.post_json("/api/auth/2fa/setup", Some(&c), json!({})).await.json()["secret"].as_str().unwrap().to_string();
        let r = self.post_json("/api/auth/2fa/enable", Some(&c), json!({"code": totp::code_now(&secret)})).await;
        assert_eq!(r.status, StatusCode::OK);
        c
    }

    fn models(&self) -> std::path::PathBuf {
        self.state.cfg.data.join("models")
    }
}

fn sha(b: &[u8]) -> String {
    hex::encode(Sha256::digest(b))
}

/// Paket mit zwei Dateien (Modell + Lizenz); liefert (Manifest, Dateien).
fn package(id: &str, model: &[u8]) -> (Value, Vec<(String, Vec<u8>)>) {
    let lic = b"MIT License\n".to_vec();
    let files = vec![("rec.onnx".to_string(), model.to_vec()), ("LICENSE.txt".to_string(), lic)];
    let m = json!({"format": 1, "id": id, "name": "Testmodell", "art": "zahl", "stufen": ["mittel"], "version": "1",
        "lizenz": {"spdx": "MIT", "datei": "LICENSE.txt"}, "quelle": {"url": "https://example.org/modell"},
        "modell": {"typ": "ctc", "datei": "rec.onnx"},
        "dateien": files.iter().map(|(n, b)| json!({"name": n, "groesse": b.len(), "sha256": sha(b)})).collect::<Vec<_>>()});
    (m, files)
}

fn tar_of(manifest: &Value, files: &[(String, Vec<u8>)], prefix: &str) -> Vec<u8> {
    let mut t = tar::Builder::new(Vec::new());
    let mut add = |name: &str, data: &[u8]| {
        let mut h = tar::Header::new_gnu();
        h.set_size(data.len() as u64);
        h.set_mode(0o644);
        h.set_cksum();
        t.append_data(&mut h, format!("{prefix}{name}"), data).unwrap();
    };
    add("manifest.json", manifest.to_string().as_bytes());
    for (n, b) in files {
        add(n, b);
    }
    t.into_inner().unwrap()
}

#[tokio::test]
async fn pakete_ausliefern_mit_range_und_etag() {
    let app = App::new();
    // ohne Ordner: leere Liste
    let r = app.get("/api/models", None).await;
    assert_eq!(r.status, StatusCode::OK);
    assert_eq!(r.json()["pakete"].as_array().unwrap().len(), 0);
    // Paket von Hand ablegen
    let model = vec![7u8; 5000];
    let (m, files) = package("zahl-test", &model);
    let d = app.models().join("zahl-test");
    std::fs::create_dir_all(&d).unwrap();
    std::fs::write(d.join("manifest.json"), m.to_string()).unwrap();
    for (n, b) in &files {
        std::fs::write(d.join(n), b).unwrap();
    }
    // unvollständiges Paket wird nicht angeboten
    let d2 = app.models().join("kaputt");
    std::fs::create_dir_all(&d2).unwrap();
    std::fs::write(d2.join("manifest.json"), package("kaputt", b"x").0.to_string()).unwrap();
    let r = app.get("/api/models", None).await;
    let p = r.json()["pakete"].as_array().unwrap().clone();
    assert_eq!(p.len(), 1);
    assert_eq!(p[0]["id"], "zahl-test");
    assert_eq!(p[0]["groesse"], 5000 + 12);
    assert_eq!(p[0]["modell"]["typ"], "ctc", "Manifest wird vollständig weitergegeben");
    // Datei, mit Cache-Headern
    let r = app.get("/api/models/zahl-test/rec.onnx", None).await;
    assert_eq!(r.status, StatusCode::OK);
    assert_eq!(r.bytes, model);
    assert!(r.headers["cache-control"].to_str().unwrap().contains("max-age"));
    let etag = r.headers.get("etag").or(r.headers.get("last-modified")).cloned();
    assert!(etag.is_some(), "ETag oder Last-Modified");
    // Teil-Abruf (Range)
    let r = app.send(Method::GET, "/api/models/zahl-test/rec.onnx", None, &[("range", "bytes=10-19")], Body::empty()).await;
    assert_eq!(r.status, StatusCode::PARTIAL_CONTENT);
    assert_eq!(r.bytes.len(), 10);
    // Revalidierung
    if let Some(lm) = r.headers.get("last-modified") {
        let r = app.send(Method::GET, "/api/models/zahl-test/rec.onnx", None, &[("if-modified-since", lm.to_str().unwrap())], Body::empty()).await;
        assert_eq!(r.status, StatusCode::NOT_MODIFIED);
    }
    // Lizenz als Text, Manifest erreichbar; fremde Dateien und Pfade nicht
    let r = app.get("/api/models/zahl-test/LICENSE.txt", None).await;
    assert!(r.headers["content-type"].to_str().unwrap().starts_with("text/plain"));
    assert_eq!(app.get("/api/models/zahl-test/manifest.json", None).await.status, StatusCode::OK);
    std::fs::write(d.join("geheim.txt"), "x").unwrap();
    for p in ["/api/models/zahl-test/geheim.txt", "/api/models/kaputt/rec.onnx", "/api/models/..%2Fowndraw.db/x", "/api/models/zahl-test/..%2F..%2Fowndraw.db", "/api/models/fehlt/rec.onnx"] {
        assert_eq!(app.get(p, None).await.status, StatusCode::NOT_FOUND, "{p}");
    }
}

#[tokio::test]
async fn admin_hochladen_pruefen_loeschen() {
    let app = App::new();
    let model = vec![3u8; 200_000];
    let (m, files) = package("zahl-up", &model);
    let tar = tar_of(&m, &files, "zahl-up/");
    // ohne Admin: abgelehnt
    let r = app.send(Method::POST, "/api/admin/models/upload", None, &[], Body::from(tar.clone())).await;
    assert_eq!(r.status, StatusCode::UNAUTHORIZED);
    assert_eq!(app.get("/api/admin/models", None).await.status, StatusCode::UNAUTHORIZED);
    let admin = app.admin().await;
    // ohne CSRF-Header: abgelehnt
    let mut req = Request::builder().method(Method::POST).uri("/api/admin/models/upload").header(header::COOKIE, &admin).body(Body::from(tar.clone())).unwrap();
    req.headers_mut().remove("x-owndraw");
    assert_eq!(app.router.clone().oneshot(req).await.unwrap().status(), StatusCode::FORBIDDEN);
    // Upload größer als die allgemeine Grenze (1 KB + 1 MB) geht durch
    let r = app.send(Method::POST, "/api/admin/models/upload", Some(&admin), &[], Body::from(tar)).await;
    assert_eq!(r.status, StatusCode::OK, "{}", String::from_utf8_lossy(&r.bytes));
    assert_eq!(r.json()["id"], "zahl-up");
    assert_eq!(std::fs::read(app.models().join("zahl-up/rec.onnx")).unwrap(), model);
    let l = app.get("/api/admin/models", Some(&admin)).await.json();
    let p = &l["pakete"][0];
    assert_eq!((p["id"].as_str(), p["ok"].as_bool(), p["lizenz"].as_str()), (Some("zahl-up"), Some(true), Some("MIT")));
    assert!(l["katalog"].as_array().unwrap().iter().any(|k| k["art"] == "laufzeit"), "eingebauter Katalog");
    assert_eq!(app.get("/api/models", None).await.json()["pakete"].as_array().unwrap().len(), 1);
    // falsche Prüfsumme: abgelehnt, altes Paket bleibt
    let (mut m2, files2) = package("zahl-up", &[1u8; 100]);
    m2["dateien"][0]["sha256"] = json!("0".repeat(64));
    let r = app.send(Method::POST, "/api/admin/models/upload", Some(&admin), &[], Body::from(tar_of(&m2, &files2, ""))).await;
    assert_eq!(r.status, StatusCode::BAD_REQUEST);
    assert!(String::from_utf8_lossy(&r.bytes).contains("Prüfsumme"), "{}", String::from_utf8_lossy(&r.bytes));
    assert_eq!(std::fs::read(app.models().join("zahl-up/rec.onnx")).unwrap(), model);
    // NC-Lizenz und Unsinn: abgelehnt
    let (mut m3, f3) = package("nc", b"x");
    m3["lizenz"]["spdx"] = json!("CC-BY-NC-SA-4.0");
    let r = app.send(Method::POST, "/api/admin/models/upload", Some(&admin), &[], Body::from(tar_of(&m3, &f3, ""))).await;
    assert_eq!(r.status, StatusCode::BAD_REQUEST);
    let r = app.send(Method::POST, "/api/admin/models/upload", Some(&admin), &[], Body::from(b"kein archiv".to_vec())).await;
    assert_eq!(r.status, StatusCode::BAD_REQUEST);
    // neue Version ersetzt die alte (zip)
    let (m4, f4) = package("zahl-up", &[9u8; 50]);
    let mut z = zip::ZipWriter::new(std::io::Cursor::new(Vec::new()));
    let o = zip::write::SimpleFileOptions::default();
    z.start_file("manifest.json", o).unwrap();
    std::io::Write::write_all(&mut z, m4.to_string().as_bytes()).unwrap();
    for (n, b) in &f4 {
        z.start_file(n.as_str(), o).unwrap();
        std::io::Write::write_all(&mut z, b).unwrap();
    }
    let zip = z.finish().unwrap().into_inner();
    let r = app.send(Method::POST, "/api/admin/models/upload", Some(&admin), &[], Body::from(zip)).await;
    assert_eq!(r.status, StatusCode::OK, "{}", String::from_utf8_lossy(&r.bytes));
    assert_eq!(std::fs::read(app.models().join("zahl-up/rec.onnx")).unwrap(), vec![9u8; 50]);
    // keine Zwischenordner übrig
    let rest: Vec<String> = std::fs::read_dir(app.models()).unwrap().flatten().map(|e| e.file_name().to_string_lossy().into_owned()).collect();
    assert_eq!(rest, vec!["zahl-up".to_string()]);
    // löschen
    let r = app.send(Method::DELETE, "/api/admin/models/zahl-up", Some(&admin), &[], Body::empty()).await;
    assert_eq!(r.status, StatusCode::OK);
    assert_eq!(app.get("/api/models", None).await.json()["pakete"].as_array().unwrap().len(), 0);
    assert_eq!(app.send(Method::DELETE, "/api/admin/models/zahl-up", Some(&admin), &[], Body::empty()).await.status, StatusCode::NOT_FOUND);
}

/// Kleiner HTTP-Server für „aus URL laden“.
async fn fake_host(files: Vec<(String, Vec<u8>)>) -> String {
    use axum::extract::Path;
    use axum::routing::get;
    use std::collections::HashMap;
    let l = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let base = format!("http://{}", l.local_addr().unwrap());
    let map: std::sync::Arc<HashMap<String, Vec<u8>>> = std::sync::Arc::new(files.into_iter().collect());
    let r = Router::new().route("/p/{name}", get(move |Path(n): Path<String>| async move { map.get(&n).cloned().map(|b| (StatusCode::OK, b)).unwrap_or((StatusCode::NOT_FOUND, vec![])) }));
    tokio::spawn(async move { axum::serve(l, r).await.unwrap() });
    format!("{base}/p/")
}

async fn wait_job(app: &App, admin: &str, id: &str) -> Value {
    for _ in 0..200 {
        let l = app.get("/api/admin/models", Some(admin)).await.json();
        if let Some(j) = l["auftraege"].as_array().unwrap().iter().find(|j| j["id"] == id) {
            if j["status"] != "laeuft" {
                return j.clone();
            }
        }
        tokio::time::sleep(Duration::from_millis(25)).await;
    }
    panic!("Auftrag {id} endet nicht");
}

#[tokio::test]
async fn admin_aus_url_laden() {
    let app = App::new();
    let admin = app.admin().await;
    let (m, files) = package("zahl-url", &[5u8; 3000]);
    let tar = tar_of(&m, &files, "");
    let mut host = files.clone();
    host.push(("manifest.json".into(), m.to_string().into_bytes()));
    host.push(("paket.tar".into(), tar.clone()));
    let base = fake_host(host).await;
    // Manifest-URL: Dateien liegen daneben
    let r = app.post_json("/api/admin/models/fetch", Some(&admin), json!({"url": format!("{base}manifest.json")})).await;
    assert_eq!(r.status, StatusCode::OK, "{}", String::from_utf8_lossy(&r.bytes));
    let j = wait_job(&app, &admin, r.json()["auftrag"].as_str().unwrap()).await;
    assert_eq!(j["status"], "fertig", "{j}");
    assert_eq!(j["geladen"], 3000 + 12);
    assert!(app.models().join("zahl-url/rec.onnx").exists());
    // Archiv mit falscher sha256: Fehler
    let r = app.post_json("/api/admin/models/fetch", Some(&admin), json!({"url": format!("{base}paket.tar"), "sha256": "0".repeat(64)})).await;
    let j = wait_job(&app, &admin, r.json()["auftrag"].as_str().unwrap()).await;
    assert_eq!(j["status"], "fehler");
    assert!(j["text"].as_str().unwrap().contains("Prüfsumme"), "{j}");
    // Archiv mit richtiger sha256
    std::fs::remove_dir_all(app.models().join("zahl-url")).unwrap();
    let r = app.post_json("/api/admin/models/fetch", Some(&admin), json!({"url": format!("{base}paket.tar"), "sha256": sha(&tar)})).await;
    let j = wait_job(&app, &admin, r.json()["auftrag"].as_str().unwrap()).await;
    assert_eq!(j["status"], "fertig", "{j}");
    // Datei fehlt am Host: Fehler, nichts installiert
    let (m2, _) = package("zahl-fehlt", &[1u8; 10]);
    let base2 = fake_host(vec![("manifest.json".into(), m2.to_string().into_bytes())]).await;
    let r = app.post_json("/api/admin/models/fetch", Some(&admin), json!({"url": format!("{base2}manifest.json")})).await;
    let j = wait_job(&app, &admin, r.json()["auftrag"].as_str().unwrap()).await;
    assert_eq!(j["status"], "fehler");
    assert!(!app.models().join("zahl-fehlt").exists());
    // ungültige Eingaben
    assert_eq!(app.post_json("/api/admin/models/fetch", Some(&admin), json!({"url": "file:///etc/passwd"})).await.status, StatusCode::BAD_REQUEST);
    assert_eq!(app.post_json("/api/admin/models/catalog/gibt-es-nicht", Some(&admin), json!({})).await.status, StatusCode::NOT_FOUND);
}
