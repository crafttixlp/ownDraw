//! Integrationstests: der axum-Router läuft im Speicher (`tower::ServiceExt::oneshot`), die DB ist `:memory:`.

use axum::body::{to_bytes, Body};
use axum::http::{header, Method, Request, StatusCode};
use axum::Router;
use owndraw_server::config::Config;
use owndraw_server::db::{now, Db};
use owndraw_server::state::AppState;
use owndraw_server::{auth, build_app};
use serde_json::{json, Value};
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
    fn code(&self) -> String {
        self.json()["error"].as_str().unwrap_or("").to_string()
    }
}

fn zip_bytes(extra: usize) -> Vec<u8> {
    let mut v = b"PK\x03\x04".to_vec();
    v.extend(std::iter::repeat_n(7u8, 100 + extra));
    v
}

impl App {
    fn new() -> Self {
        Self::with(|_| {})
    }

    fn with(tweak: impl FnOnce(&mut Config)) -> Self {
        let dir = tempfile::tempdir().unwrap();
        let web = dir.path().join("web");
        std::fs::create_dir_all(&web).unwrap();
        std::fs::write(web.join("index.html"), "<html>OwnDraw Web</html>").unwrap();
        let mut cfg = Config {
            data: dir.path().join("data"),
            web,
            bind: "127.0.0.1:0".into(),
            max_board_bytes: 1024 * 1024,
            rate_limit: 1000,
            rate_window_secs: 600,
            release_url: "http://127.0.0.1:1/none".into(),
        };
        tweak(&mut cfg);
        std::fs::create_dir_all(cfg.boards_dir()).unwrap();
        let state = AppState::new(Db::memory().unwrap(), cfg);
        App { router: build_app(state.clone()), state, _dir: dir }
    }

    async fn send(&self, method: Method, path: &str, cookie: Option<&str>, headers: &[(&str, &str)], body: Body, csrf: bool) -> Resp {
        let mut b = Request::builder().method(method).uri(path);
        if csrf {
            b = b.header("x-owndraw", "1");
        }
        if let Some(c) = cookie {
            b = b.header(header::COOKIE, c);
        }
        for (k, v) in headers {
            b = b.header(*k, *v);
        }
        let resp = self.router.clone().oneshot(b.body(body).unwrap()).await.unwrap();
        let (parts, body) = resp.into_parts();
        let bytes = to_bytes(body, 64 * 1024 * 1024).await.unwrap().to_vec();
        Resp { status: parts.status, headers: parts.headers, bytes }
    }

    async fn api(&self, method: Method, path: &str, cookie: Option<&str>, body: Option<Value>) -> Resp {
        let (hdrs, b): (Vec<(&str, &str)>, Body) = match body {
            Some(v) => (vec![("content-type", "application/json")], Body::from(v.to_string())),
            None => (vec![], Body::empty()),
        };
        self.send(method, path, cookie, &hdrs, b, true).await
    }

    async fn get(&self, path: &str, cookie: Option<&str>) -> Resp {
        self.api(Method::GET, path, cookie, None).await
    }
    async fn post(&self, path: &str, cookie: Option<&str>, body: Value) -> Resp {
        self.api(Method::POST, path, cookie, Some(body)).await
    }
    async fn patch(&self, path: &str, cookie: Option<&str>, body: Value) -> Resp {
        self.api(Method::PATCH, path, cookie, Some(body)).await
    }

    /// Registriert ein Konto und liefert das Sitzungs-Cookie.
    async fn register(&self, email: &str) -> String {
        let r = self.post("/api/auth/register", None, json!({"email": email, "password": "geheim1234", "name": "Test"})).await;
        assert_eq!(r.status, StatusCode::OK, "{}", String::from_utf8_lossy(&r.bytes));
        cookie_of(&r)
    }

    async fn login(&self, email: &str, pw: &str) -> Resp {
        self.post("/api/auth/login", None, json!({"email": email, "password": pw})).await
    }

    async fn admin(&self) -> String {
        auth::create_admin(&self.state.db, "admin@example.org", "adminpasswort").await.unwrap();
        let r = self.login("admin@example.org", "adminpasswort").await;
        assert_eq!(r.status, StatusCode::OK);
        cookie_of(&r)
    }

    /// Erzeugt eine Lizenz (als Admin) und löst sie für das Konto ein.
    async fn give_license(&self, admin: &str, user: &str, days: Value, quota_mb: i64) -> String {
        let r =
            self.post("/api/admin/licenses", Some(admin), json!({"count": 1, "days": days, "quota_mb": quota_mb, "note": "Test"})).await;
        assert_eq!(r.status, StatusCode::OK);
        let code = r.json()[0]["code"].as_str().unwrap().to_string();
        let r = self.post("/api/license/redeem", Some(user), json!({"code": code})).await;
        assert_eq!(r.status, StatusCode::OK, "{}", String::from_utf8_lossy(&r.bytes));
        code
    }

    async fn upload(&self, cookie: &str, name: &str, bytes: Vec<u8>) -> Resp {
        self.send(
            Method::POST,
            "/api/boards",
            Some(cookie),
            &[("content-type", "application/octet-stream"), ("x-board-name", name)],
            Body::from(bytes),
            true,
        )
        .await
    }

    async fn put_board(&self, cookie: &str, id: &str, version: &str, bytes: Vec<u8>) -> Resp {
        self.send(
            Method::PUT,
            &format!("/api/boards/{id}"),
            Some(cookie),
            &[("if-match", version), ("content-type", "application/octet-stream")],
            Body::from(bytes),
            true,
        )
        .await
    }
}

fn cookie_of(r: &Resp) -> String {
    let sc = r.headers.get(header::SET_COOKIE).expect("Set-Cookie fehlt").to_str().unwrap();
    sc.split(';').next().unwrap().to_string()
}

#[tokio::test]
async fn registrieren_anmelden_me() {
    let app = App::new();
    let r = app.post("/api/auth/register", None, json!({"email": "Anna@Example.org", "password": "geheim1234", "name": "Anna"})).await;
    assert_eq!(r.status, StatusCode::OK);
    let sc = r.headers.get(header::SET_COOKIE).unwrap().to_str().unwrap().to_string();
    for part in ["owndraw_session=", "HttpOnly", "Secure", "SameSite=Lax", "Max-Age=2592000"] {
        assert!(sc.contains(part), "Cookie ohne {part}: {sc}");
    }
    let me = r.json();
    assert_eq!(me["email"], "anna@example.org");
    assert_eq!(me["name"], "Anna");
    assert_eq!(me["admin"], false);
    assert!(me["license"].is_null());
    let cookie = cookie_of(&r);

    // /api/me mit und ohne Cookie
    let r = app.get("/api/me", Some(&cookie)).await;
    assert_eq!(r.status, StatusCode::OK);
    assert_eq!(r.json()["email"], "anna@example.org");
    let r = app.get("/api/me", None).await;
    assert_eq!(r.status, StatusCode::UNAUTHORIZED);
    assert_eq!(r.code(), "unauthorized");

    // Doppelt, zu kurzes Passwort, ungültige E-Mail
    let r = app.post("/api/auth/register", None, json!({"email": "anna@example.org", "password": "geheim1234", "name": "x"})).await;
    assert_eq!((r.status, r.code().as_str()), (StatusCode::CONFLICT, "conflict"));
    let r = app.post("/api/auth/register", None, json!({"email": "b@example.org", "password": "kurz", "name": "x"})).await;
    assert_eq!((r.status, r.code().as_str()), (StatusCode::BAD_REQUEST, "invalid"));
    let r = app.post("/api/auth/register", None, json!({"email": "keine-mail", "password": "geheim1234", "name": "x"})).await;
    assert_eq!(r.status, StatusCode::BAD_REQUEST);

    // Anmelden: richtig und falsch
    let r = app.login("anna@example.org", "falsch12345").await;
    assert_eq!((r.status, r.code().as_str()), (StatusCode::UNAUTHORIZED, "unauthorized"));
    let r = app.login("niemand@example.org", "geheim1234").await;
    assert_eq!(r.status, StatusCode::UNAUTHORIZED);
    let r = app.login("ANNA@example.org", "geheim1234").await;
    assert_eq!(r.status, StatusCode::OK);
    let c2 = cookie_of(&r);

    // Passwort ändern
    let r = app.post("/api/me/password", Some(&c2), json!({"old": "falsch", "new": "neuespasswort"})).await;
    assert_eq!(r.status, StatusCode::FORBIDDEN);
    let r = app.post("/api/me/password", Some(&c2), json!({"old": "geheim1234", "new": "neuespasswort"})).await;
    assert_eq!(r.status, StatusCode::OK);
    assert_eq!(app.login("anna@example.org", "geheim1234").await.status, StatusCode::UNAUTHORIZED);
    assert_eq!(app.login("anna@example.org", "neuespasswort").await.status, StatusCode::OK);
    // andere Sitzungen sind beendet, die aktuelle bleibt
    assert_eq!(app.get("/api/me", Some(&cookie)).await.status, StatusCode::UNAUTHORIZED);
    assert_eq!(app.get("/api/me", Some(&c2)).await.status, StatusCode::OK);

    // Abmelden
    let r = app.post("/api/auth/logout", Some(&c2), json!({})).await;
    assert_eq!(r.status, StatusCode::OK);
    assert!(r.headers.get(header::SET_COOKIE).unwrap().to_str().unwrap().contains("Max-Age=0"));
    assert_eq!(app.get("/api/me", Some(&c2)).await.status, StatusCode::UNAUTHORIZED);
}

#[tokio::test]
async fn csrf_header_ist_pflicht() {
    let app = App::new();
    let body = json!({"email": "a@example.org", "password": "geheim1234", "name": "A"}).to_string();
    let r =
        app.send(Method::POST, "/api/auth/register", None, &[("content-type", "application/json")], Body::from(body.clone()), false).await;
    assert_eq!((r.status, r.code().as_str()), (StatusCode::FORBIDDEN, "forbidden"));
    // falscher Wert zählt nicht
    let r = app.send(Method::POST, "/api/auth/register", None, &[("x-owndraw", "0")], Body::from(body.clone()), false).await;
    assert_eq!(r.status, StatusCode::FORBIDDEN);
    // mit Header klappt es; lesende Anfragen brauchen ihn nicht
    let r = app.send(Method::POST, "/api/auth/register", None, &[("content-type", "application/json")], Body::from(body), true).await;
    assert_eq!(r.status, StatusCode::OK);
    let r = app.send(Method::GET, "/api/health", None, &[], Body::empty(), false).await;
    assert_eq!(r.status, StatusCode::OK);
    assert_eq!(r.json()["ok"], true);
    assert!(r.json()["version"].is_string());
    // auch Board-Upload ohne Header wird abgelehnt
    let c = cookie_of(&app.login("a@example.org", "geheim1234").await);
    let r = app.send(Method::POST, "/api/boards", Some(&c), &[], Body::from(zip_bytes(0)), false).await;
    assert_eq!(r.status, StatusCode::FORBIDDEN);
}

#[tokio::test]
async fn lizenz_einloesen_und_ablauf() {
    let app = App::new();
    let admin = app.admin().await;
    let user = app.register("u@example.org").await;

    let r = app.post("/api/admin/licenses", Some(&admin), json!({"count": 2, "days": 30, "quota_mb": 5, "note": "Test"})).await;
    assert_eq!(r.status, StatusCode::OK);
    let list = r.json();
    assert_eq!(list.as_array().unwrap().len(), 2);
    let code = list[0]["code"].as_str().unwrap().to_string();
    assert!(code.starts_with("OWND-") && code.len() == 19, "{code}");
    assert!(list[0]["redeemed_by"].is_null() && list[0]["expires_at"].is_null());
    assert_eq!(list[0]["days"], 30);

    // unbekannter Code
    let r = app.post("/api/license/redeem", Some(&user), json!({"code": "OWND-AAAA-AAAA-AAAA"})).await;
    assert_eq!((r.status, r.code().as_str()), (StatusCode::NOT_FOUND, "not_found"));

    // einlösen (Kleinschreibung und Leerzeichen sind egal): Laufzeit beginnt jetzt
    let t0 = now();
    let r = app.post("/api/license/redeem", Some(&user), json!({"code": format!(" {} ", code.to_lowercase())})).await;
    assert_eq!(r.status, StatusCode::OK);
    let lic = &r.json()["license"];
    assert_eq!(lic["active"], true);
    assert_eq!(lic["quota_mb"], 5);
    let exp = lic["expires_at"].as_i64().unwrap();
    assert!((t0 + 30 * 86400..=now() + 30 * 86400).contains(&exp), "expires_at {exp}");

    // zweites Konto kann denselben Code nicht einlösen
    let other = app.register("o@example.org").await;
    let r = app.post("/api/license/redeem", Some(&other), json!({"code": code})).await;
    assert_eq!((r.status, r.code().as_str()), (StatusCode::CONFLICT, "conflict"));

    // Lizenz abgelaufen (Admin setzt Ablauf in die Vergangenheit) → inaktiv, Upload verweigert
    let r = app.patch(&format!("/api/admin/licenses/{code}"), Some(&admin), json!({"expires_at": now() - 10})).await;
    assert_eq!(r.status, StatusCode::OK);
    let me = app.get("/api/me", Some(&user)).await.json();
    assert_eq!(me["license"]["active"], false);
    let r = app.upload(&user, "Test", zip_bytes(0)).await;
    assert_eq!((r.status, r.code().as_str()), (StatusCode::FORBIDDEN, "license_required"));

    // Ablauf entfernen (null = unbegrenzt) → wieder aktiv; Widerruf → inaktiv
    let r = app.patch(&format!("/api/admin/licenses/{code}"), Some(&admin), json!({"expires_at": null})).await;
    assert!(r.json()["expires_at"].is_null());
    assert_eq!(app.get("/api/me", Some(&user)).await.json()["license"]["active"], true);
    let r = app.patch(&format!("/api/admin/licenses/{code}"), Some(&admin), json!({"revoked": true, "note": "weg"})).await;
    assert_eq!((r.json()["revoked"].clone(), r.json()["note"].clone()), (json!(true), json!("weg")));
    assert_eq!(app.get("/api/me", Some(&user)).await.json()["license"]["active"], false);

    // Liste zeigt Einlöser; unbegrenzte Lizenz (days = null)
    let all = app.get("/api/admin/licenses", Some(&admin)).await.json();
    let mine = all.as_array().unwrap().iter().find(|l| l["code"] == code.as_str()).unwrap();
    assert_eq!(mine["redeemed_by"]["email"], "u@example.org");
    let r = app.post("/api/admin/licenses", Some(&admin), json!({"count": 1, "days": null, "quota_mb": 10, "note": ""})).await;
    assert!(r.json()[0]["days"].is_null());
    // ungültige Anzahl
    let r = app.post("/api/admin/licenses", Some(&admin), json!({"count": 0, "days": null, "quota_mb": 10, "note": ""})).await;
    assert_eq!(r.status, StatusCode::BAD_REQUEST);
}

#[tokio::test]
async fn board_upload_nur_mit_lizenz() {
    let app = App::new();
    let admin = app.admin().await;
    let user = app.register("u@example.org").await;

    let r = app.upload(&user, "Mein%20Board%20%C3%A4", zip_bytes(0)).await;
    assert_eq!((r.status, r.code().as_str()), (StatusCode::FORBIDDEN, "license_required"));
    let r = app.upload("owndraw_session=ungueltig", "x", zip_bytes(0)).await;
    assert_eq!(r.status, StatusCode::UNAUTHORIZED);

    app.give_license(&admin, &user, json!(null), 10).await;
    let content = zip_bytes(500);
    let r = app.upload(&user, "Mein%20Board%20%C3%A4", content.clone()).await;
    assert_eq!(r.status, StatusCode::OK, "{}", String::from_utf8_lossy(&r.bytes));
    let info = r.json();
    assert_eq!(info["name"], "Mein Board ä");
    assert_eq!(info["role"], "owner");
    assert_eq!(info["version"], 1);
    assert_eq!(info["size"], content.len());
    assert_eq!(info["owner"]["name"], "Test");
    let id = info["id"].as_str().unwrap().to_string();

    // Lesen: Bytes + Version
    let r = app.get(&format!("/api/boards/{id}"), Some(&user)).await;
    assert_eq!(r.status, StatusCode::OK);
    assert_eq!(r.bytes, content);
    assert_eq!(r.headers.get("x-board-version").unwrap(), "1");
    assert!(app.state.cfg.boards_dir().join(format!("{id}.owndraw")).is_file());

    // Liste, Verbrauch in Me, Umbenennen
    let list = app.get("/api/boards", Some(&user)).await.json();
    assert_eq!(list.as_array().unwrap().len(), 1);
    assert!(app.get("/api/me", Some(&user)).await.json()["license"]["used_mb"].as_f64().is_some());
    let r = app.patch(&format!("/api/boards/{id}"), Some(&user), json!({"name": "Neu"})).await;
    assert_eq!(r.json()["name"], "Neu");

    // keine ZIP-Datei, zu groß (Limit im Test 1 MB), Quote
    let r = app.upload(&user, "x", b"kein zip".to_vec()).await;
    assert_eq!((r.status, r.code().as_str()), (StatusCode::BAD_REQUEST, "invalid"));
    let r = app.upload(&user, "x", zip_bytes(1024 * 1024 + 10)).await;
    assert_eq!((r.status, r.code().as_str()), (StatusCode::PAYLOAD_TOO_LARGE, "too_large"));

    // Löschen entfernt Datei und Eintrag
    let r = app.api(Method::DELETE, &format!("/api/boards/{id}"), Some(&user), None).await;
    assert_eq!(r.status, StatusCode::OK);
    assert!(!app.state.cfg.boards_dir().join(format!("{id}.owndraw")).exists());
    assert_eq!(app.get(&format!("/api/boards/{id}"), Some(&user)).await.status, StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn speicherquote() {
    let app = App::new();
    let admin = app.admin().await;
    let user = app.register("u@example.org").await;
    app.give_license(&admin, &user, json!(null), 1).await; // 1 MB
    let r = app.upload(&user, "a", zip_bytes(700 * 1024)).await;
    assert_eq!(r.status, StatusCode::OK);
    let r = app.upload(&user, "b", zip_bytes(700 * 1024)).await;
    assert_eq!((r.status, r.code().as_str()), (StatusCode::INSUFFICIENT_STORAGE, "quota"));
}

#[tokio::test]
async fn versionskonflikt_409() {
    let app = App::new();
    let admin = app.admin().await;
    let user = app.register("u@example.org").await;
    app.give_license(&admin, &user, json!(null), 10).await;
    let id = app.upload(&user, "b", zip_bytes(10)).await.json()["id"].as_str().unwrap().to_string();

    let r = app.put_board(&user, &id, "1", zip_bytes(20)).await;
    assert_eq!(r.status, StatusCode::OK);
    assert_eq!(r.json()["version"], 2);
    assert_eq!(r.json()["size"], 124);

    // veraltete Version
    let r = app.put_board(&user, &id, "1", zip_bytes(30)).await;
    assert_eq!((r.status, r.code().as_str()), (StatusCode::CONFLICT, "conflict"));
    assert_eq!(r.json()["version"], 2);
    // Inhalt blieb unverändert
    assert_eq!(app.get(&format!("/api/boards/{id}"), Some(&user)).await.bytes.len(), 124);
    // If-Match in Anführungszeichen ist erlaubt, fehlender Header nicht
    assert_eq!(app.put_board(&user, &id, "\"2\"", zip_bytes(5)).await.json()["version"], 3);
    let r = app.send(Method::PUT, &format!("/api/boards/{id}"), Some(&user), &[], Body::from(zip_bytes(1)), true).await;
    assert_eq!(r.status, StatusCode::BAD_REQUEST);
}

#[tokio::test]
async fn mitglieder_und_rollen() {
    let app = App::new();
    let admin = app.admin().await;
    let owner = app.register("owner@example.org").await;
    let editor = app.register("edit@example.org").await;
    let viewer = app.register("view@example.org").await;
    let stranger = app.register("fremd@example.org").await;
    let code = app.give_license(&admin, &owner, json!(null), 10).await;
    let id = app.upload(&owner, "geteilt", zip_bytes(10)).await.json()["id"].as_str().unwrap().to_string();
    let members = format!("/api/boards/{id}/members");

    // Teilen: nur Besitzer; Fremde sehen das Board nicht
    assert_eq!(app.get(&format!("/api/boards/{id}"), Some(&stranger)).await.status, StatusCode::NOT_FOUND);
    let r = app.post(&members, Some(&editor), json!({"email": "view@example.org", "role": "view"})).await;
    assert_eq!(r.status, StatusCode::NOT_FOUND);
    let r = app.post(&members, Some(&owner), json!({"email": "gibtsnicht@example.org", "role": "edit"})).await;
    assert_eq!((r.status, r.code().as_str()), (StatusCode::NOT_FOUND, "not_found"));
    let r = app.post(&members, Some(&owner), json!({"email": "edit@example.org", "role": "admin"})).await;
    assert_eq!(r.status, StatusCode::BAD_REQUEST);
    let r = app.post(&members, Some(&owner), json!({"email": "edit@example.org", "role": "edit"})).await;
    assert_eq!(r.status, StatusCode::OK);
    let r = app.post(&members, Some(&owner), json!({"email": "view@example.org", "role": "view"})).await;
    let list = r.json();
    assert_eq!(list.as_array().unwrap().len(), 3);
    assert_eq!(list[0]["role"], "owner");
    assert!(list.as_array().unwrap().iter().any(|m| m["email"] == "edit@example.org" && m["role"] == "edit"));

    // Eingeladene sehen das Board (auch ohne eigene Lizenz) mit ihrer Rolle
    let l = app.get("/api/boards", Some(&editor)).await.json();
    assert_eq!(l[0]["role"], "edit");
    assert_eq!(l[0]["owner"]["name"], "Test");
    assert_eq!(app.get("/api/boards", Some(&viewer)).await.json()[0]["role"], "view");
    assert_eq!(app.get(&format!("/api/boards/{id}"), Some(&viewer)).await.status, StatusCode::OK);
    assert_eq!(app.get(&members, Some(&viewer)).await.status, StatusCode::OK);

    // Bearbeiter darf speichern, Betrachter nicht; Löschen/Teilen/Umbenennen nur Besitzer
    let r = app.put_board(&editor, &id, "1", zip_bytes(40)).await;
    assert_eq!(r.status, StatusCode::OK);
    assert_eq!(r.json()["role"], "edit");
    let r = app.put_board(&viewer, &id, "2", zip_bytes(40)).await;
    assert_eq!((r.status, r.code().as_str()), (StatusCode::FORBIDDEN, "forbidden"));
    assert_eq!(app.api(Method::DELETE, &format!("/api/boards/{id}"), Some(&editor), None).await.status, StatusCode::FORBIDDEN);
    assert_eq!(app.patch(&format!("/api/boards/{id}"), Some(&editor), json!({"name": "x"})).await.status, StatusCode::FORBIDDEN);
    let r = app.post(&members, Some(&editor), json!({"email": "fremd@example.org", "role": "view"})).await;
    assert_eq!(r.status, StatusCode::FORBIDDEN);
    // Eingeladene ohne Lizenz dürfen selbst keine Boards anlegen
    let r = app.upload(&editor, "x", zip_bytes(1)).await;
    assert_eq!(r.code(), "license_required");

    // Läuft die Lizenz des Besitzers ab, ist das Board für alle nur noch lesbar
    let r = app.patch(&format!("/api/admin/licenses/{code}"), Some(&admin), json!({"expires_at": now() - 1})).await;
    assert_eq!(r.status, StatusCode::OK);
    let r = app.put_board(&editor, &id, "2", zip_bytes(40)).await;
    assert_eq!((r.status, r.code().as_str()), (StatusCode::FORBIDDEN, "license_required"));
    let r = app.post(&members, Some(&owner), json!({"email": "fremd@example.org", "role": "view"})).await;
    assert_eq!(r.code(), "license_required");
    assert_eq!(app.get(&format!("/api/boards/{id}"), Some(&owner)).await.status, StatusCode::OK);

    // Mitglied entfernen (Besitzer) bzw. selbst gehen
    let uid_view = app.get("/api/me", Some(&viewer)).await.json()["id"].as_str().unwrap().to_string();
    let uid_edit = app.get("/api/me", Some(&editor)).await.json()["id"].as_str().unwrap().to_string();
    let r = app.api(Method::DELETE, &format!("{members}/{uid_view}"), Some(&editor), None).await;
    assert_eq!(r.status, StatusCode::FORBIDDEN);
    let r = app.api(Method::DELETE, &format!("{members}/{uid_view}"), Some(&owner), None).await;
    assert_eq!(r.status, StatusCode::OK);
    assert_eq!(r.json().as_array().unwrap().len(), 2);
    assert_eq!(app.get(&format!("/api/boards/{id}"), Some(&viewer)).await.status, StatusCode::NOT_FOUND);
    let r = app.api(Method::DELETE, &format!("{members}/{uid_edit}"), Some(&editor), None).await;
    assert_eq!(r.status, StatusCode::OK);
    assert_eq!(app.get(&format!("/api/boards/{id}"), Some(&editor)).await.status, StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn admin_rechte() {
    let app = App::new();
    let admin = app.admin().await;
    let user = app.register("u@example.org").await;

    for (m, p) in [
        (Method::GET, "/api/admin/users"),
        (Method::GET, "/api/admin/licenses"),
        (Method::GET, "/api/admin/update"),
        (Method::POST, "/api/admin/update/check"),
        (Method::POST, "/api/admin/update/install"),
    ] {
        let r = app.api(m.clone(), p, Some(&user), None).await;
        assert_eq!((r.status, r.code().as_str()), (StatusCode::FORBIDDEN, "forbidden"), "{m} {p}");
        let r = app.api(m.clone(), p, None, None).await;
        assert_eq!(r.status, StatusCode::UNAUTHORIZED, "{m} {p}");
    }
    let r = app.post("/api/admin/licenses", Some(&user), json!({"count": 1, "days": null, "quota_mb": 1, "note": ""})).await;
    assert_eq!(r.status, StatusCode::FORBIDDEN);
    let r = app.patch("/api/admin/update", Some(&user), json!({"auto": true})).await;
    assert_eq!(r.status, StatusCode::FORBIDDEN);

    // Admin sieht Nutzer mit Lizenz und Verbrauch
    app.give_license(&admin, &user, json!(7), 3).await;
    let users = app.get("/api/admin/users", Some(&admin)).await.json();
    let u = users.as_array().unwrap().iter().find(|u| u["email"] == "u@example.org").unwrap().clone();
    assert_eq!(u["license"]["quota_mb"], 3);
    assert_eq!(u["boards"], 0);
    assert_eq!(u["disabled"], false);
    let uid = u["id"].as_str().unwrap().to_string();

    // Admin-Recht vergeben und entziehen
    let r = app.patch(&format!("/api/admin/users/{uid}"), Some(&admin), json!({"admin": true})).await;
    assert_eq!(r.json()["admin"], true);
    assert_eq!(app.get("/api/admin/users", Some(&user)).await.status, StatusCode::OK);
    app.patch(&format!("/api/admin/users/{uid}"), Some(&admin), json!({"admin": false})).await;
    assert_eq!(app.get("/api/admin/users", Some(&user)).await.status, StatusCode::FORBIDDEN);

    // Sperren beendet Sitzungen und verhindert die Anmeldung
    let r = app.patch(&format!("/api/admin/users/{uid}"), Some(&admin), json!({"disabled": true})).await;
    assert_eq!(r.json()["disabled"], true);
    assert_eq!(app.get("/api/me", Some(&user)).await.status, StatusCode::UNAUTHORIZED);
    assert_eq!(app.login("u@example.org", "geheim1234").await.status, StatusCode::FORBIDDEN);
    app.patch(&format!("/api/admin/users/{uid}"), Some(&admin), json!({"disabled": false})).await;
    assert_eq!(app.login("u@example.org", "geheim1234").await.status, StatusCode::OK);

    // Sich selbst sperren/löschen geht nicht
    let me = app.get("/api/me", Some(&admin)).await.json()["id"].as_str().unwrap().to_string();
    assert_eq!(app.patch(&format!("/api/admin/users/{me}"), Some(&admin), json!({"disabled": true})).await.status, StatusCode::BAD_REQUEST);
    assert_eq!(app.api(Method::DELETE, &format!("/api/admin/users/{me}"), Some(&admin), None).await.status, StatusCode::BAD_REQUEST);

    // Löschen entfernt Nutzer samt Boards; Lizenz wird widerrufen
    let c = cookie_of(&app.login("u@example.org", "geheim1234").await);
    let bid = app.upload(&c, "b", zip_bytes(5)).await.json()["id"].as_str().unwrap().to_string();
    let file = app.state.cfg.boards_dir().join(format!("{bid}.owndraw"));
    assert!(file.is_file());
    let r = app.api(Method::DELETE, &format!("/api/admin/users/{uid}"), Some(&admin), None).await;
    assert_eq!(r.status, StatusCode::OK);
    assert!(!file.exists());
    assert_eq!(app.get("/api/me", Some(&c)).await.status, StatusCode::UNAUTHORIZED);
    let lic = app.get("/api/admin/licenses", Some(&admin)).await.json();
    assert_eq!(lic[0]["revoked"], true);
    assert!(lic[0]["redeemed_by"].is_null());

    // Update-Einstellungen: Auto-Update standardmäßig aus
    let st = app.get("/api/admin/update", Some(&admin)).await.json();
    assert_eq!((st["auto"].clone(), st["auto_hour"].clone(), st["available"].clone()), (json!(false), json!(3), json!(false)));
    assert!(st["current"].is_string());
    let st = app.patch("/api/admin/update", Some(&admin), json!({"auto": true, "auto_hour": 5})).await.json();
    assert_eq!((st["auto"].clone(), st["auto_hour"].clone()), (json!(true), json!(5)));
    assert_eq!(app.patch("/api/admin/update", Some(&admin), json!({"auto_hour": 24})).await.status, StatusCode::BAD_REQUEST);
    // Prüfen ohne Netz: Fehler, kein Absturz
    let r = app.post("/api/admin/update/check", Some(&admin), json!({})).await;
    assert_eq!(r.status, StatusCode::BAD_GATEWAY);
    let st = app.get("/api/admin/update", Some(&admin)).await.json();
    assert!(!st["log"].as_array().unwrap().is_empty());
}

#[tokio::test]
async fn rate_limit_fuer_login() {
    let app = App::with(|c| c.rate_limit = 3);
    for _ in 0..3 {
        let r = app.login("x@example.org", "irgendwas1").await;
        assert_eq!(r.status, StatusCode::UNAUTHORIZED);
    }
    let r = app.login("x@example.org", "irgendwas1").await;
    assert_eq!((r.status, r.code().as_str()), (StatusCode::TOO_MANY_REQUESTS, "rate_limited"));
    // andere Client-Adresse (X-Forwarded-For) ist nicht betroffen
    let r = app
        .send(
            Method::POST,
            "/api/auth/login",
            None,
            &[("x-forwarded-for", "203.0.113.9"), ("content-type", "application/json")],
            Body::from(json!({"email": "x@example.org", "password": "irgendwas1"}).to_string()),
            true,
        )
        .await;
    assert_eq!(r.status, StatusCode::UNAUTHORIZED);
}

#[tokio::test]
async fn statisches_admin_und_platzhalter() {
    let app = App::new();
    let r = app.send(Method::GET, "/", None, &[], Body::empty(), false).await;
    assert_eq!(r.status, StatusCode::OK);
    assert!(String::from_utf8_lossy(&r.bytes).contains("OwnDraw Web"));
    let r = app.send(Method::GET, "/admin", None, &[], Body::empty(), false).await;
    assert_eq!(r.status, StatusCode::OK);
    assert!(r.headers.get(header::CONTENT_TYPE).unwrap().to_str().unwrap().starts_with("text/html"));
    assert!(String::from_utf8_lossy(&r.bytes).contains("OwnDraw"));
    // unbekannter API-Pfad: JSON-404; Kollaboration ohne Anmeldung: 401
    let r = app.get("/api/gibtsnicht", None).await;
    assert_eq!((r.status, r.code().as_str()), (StatusCode::NOT_FOUND, "not_found"));
    let r = app.get("/api/collab/abc", None).await;
    assert_eq!(r.status, StatusCode::UNAUTHORIZED);
    // ungültiges JSON
    let r = app.send(Method::POST, "/api/auth/login", None, &[], Body::from("{kaputt"), true).await;
    assert_eq!((r.status, r.code().as_str()), (StatusCode::BAD_REQUEST, "invalid"));
}

/// Kleiner GitHub-Ersatz: `/repos/o/r` gibt es, `/repos/o/r/releases/latest` je nach `release`.
async fn fake_github(release: Option<Value>) -> String {
    use axum::routing::get;
    let rel = release.clone();
    let r = Router::new().route("/repos/o/r", get(|| async { axum::Json(json!({"full_name": "o/r"})) })).route(
        "/repos/o/r/releases/latest",
        get(move || {
            let rel = rel.clone();
            async move {
                match rel {
                    Some(v) => (StatusCode::OK, axum::Json(v)),
                    None => (StatusCode::NOT_FOUND, axum::Json(json!({"message": "Not Found"}))),
                }
            }
        }),
    );
    let l = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = l.local_addr().unwrap();
    tokio::spawn(async move { axum::serve(l, r).await.unwrap() });
    format!("http://{addr}")
}

#[tokio::test]
async fn update_pruefung_ohne_release_und_ohne_repo() {
    // Repository vorhanden, aber noch kein Release: kein Fehler, verständlicher Zustand
    let base = fake_github(None).await;
    let url = format!("{base}/repos/o/r/releases/latest");
    let app = App::with(|c| c.release_url = url);
    let admin = app.admin().await;
    let r = app.post("/api/admin/update/check", Some(&admin), json!({})).await;
    assert_eq!(r.status, StatusCode::OK);
    let st = r.json();
    assert_eq!((st["no_release"].clone(), st["available"].clone(), st["latest"].clone()), (json!(true), json!(false), Value::Null));
    assert!(st["log"].to_string().contains("noch kein Release"));
    assert_eq!(app.post("/api/admin/update/install", Some(&admin), json!({})).await.status, StatusCode::CONFLICT);

    // Repository falsch: klare Fehlermeldung
    let url = format!("{base}/repos/o/falsch/releases/latest");
    let app = App::with(|c| c.release_url = url);
    let admin = app.admin().await;
    let r = app.post("/api/admin/update/check", Some(&admin), json!({})).await;
    assert_eq!(r.status, StatusCode::BAD_GATEWAY);
    assert!(r.json()["message"].to_string().contains("nicht gefunden"), "{}", r.json());

    // Release mit Server-Paket: verfügbar, Hinweis verschwindet
    let rel = json!({"tag_name": "v99.0.0", "draft": false, "prerelease": false, "assets": [
        {"name": "owndraw-server-99.0.0-x86_64-linux.tar.gz", "browser_download_url": "http://x/t"},
        {"name": "owndraw-server-99.0.0-x86_64-linux.tar.gz.minisig", "browser_download_url": "http://x/s"}]});
    let base = fake_github(Some(rel)).await;
    let url = format!("{base}/repos/o/r/releases/latest");
    let app = App::with(|c| c.release_url = url);
    let admin = app.admin().await;
    let st = app.post("/api/admin/update/check", Some(&admin), json!({})).await.json();
    assert_eq!((st["no_release"].clone(), st["available"].clone(), st["latest"].clone()), (json!(false), json!(true), json!("99.0.0")));
}

#[tokio::test]
async fn einladungslink_gast_mit_eigener_id() {
    let app = App::new();
    let admin = app.admin().await;
    let owner = app.register("owner@example.org").await;
    let other = app.register("andere@example.org").await;
    let code = app.give_license(&admin, &owner, json!(null), 10).await;
    let id = app.upload(&owner, "Gemeinsam", zip_bytes(10)).await.json()["id"].as_str().unwrap().to_string();
    let invites = format!("/api/boards/{id}/invites");

    // Links verwalten nur Besitzer; Rolle/Laufzeit werden geprüft
    assert_eq!(app.post(&invites, Some(&other), json!({"role": "edit"})).await.status, StatusCode::NOT_FOUND);
    assert_eq!(app.post(&invites, Some(&owner), json!({"role": "owner"})).await.status, StatusCode::BAD_REQUEST);
    assert_eq!(app.post(&invites, Some(&owner), json!({"role": "edit", "days": 0})).await.status, StatusCode::BAD_REQUEST);
    let r = app.post(&invites, Some(&owner), json!({"role": "edit"})).await;
    assert_eq!(r.status, StatusCode::OK);
    let edit = r.json();
    let token = edit["token"].as_str().unwrap().to_string();
    assert_eq!(token.len(), 48);
    assert!(edit["expires_at"].as_i64().unwrap() > now() + 6 * 86400);
    // Laufzeit frei: Stunden oder Tage, Maximum 365 Tage
    assert_eq!(app.post(&invites, Some(&owner), json!({"role": "view", "hours": 0})).await.status, StatusCode::BAD_REQUEST);
    assert_eq!(app.post(&invites, Some(&owner), json!({"role": "view", "hours": 365 * 24 + 1})).await.status, StatusCode::BAD_REQUEST);
    assert_eq!(app.post(&invites, Some(&owner), json!({"role": "view", "days": 366})).await.status, StatusCode::BAD_REQUEST);
    let h3 = app.post(&invites, Some(&owner), json!({"role": "view", "hours": 3})).await.json();
    let left = h3["expires_at"].as_i64().unwrap() - h3["created"].as_i64().unwrap();
    assert_eq!(left, 3 * 3600);
    let d30 = app.post(&invites, Some(&owner), json!({"role": "view", "days": 30})).await.json();
    assert_eq!(d30["expires_at"].as_i64().unwrap() - d30["created"].as_i64().unwrap(), 30 * 86400);
    for x in [&h3, &d30] {
        app.api(Method::DELETE, &format!("{invites}/{}", x["id"].as_str().unwrap()), Some(&owner), None).await;
    }
    let view = app.post(&invites, Some(&owner), json!({"role": "view", "days": null})).await.json();
    assert!(view["expires_at"].is_null());
    let list = app.get(&invites, Some(&owner)).await.json();
    assert_eq!(list.as_array().unwrap().len(), 2);
    assert!(list[0].get("token").is_none(), "Token nur beim Anlegen");

    // Linkinfo ohne Anmeldung; falsche Tokens → 404
    let r = app.get(&format!("/api/invites/{token}"), None).await;
    assert_eq!(r.status, StatusCode::OK);
    assert_eq!(r.json()["board"]["name"], "Gemeinsam");
    assert_eq!(r.json()["role"], "edit");
    assert_eq!(app.get(&format!("/api/invites/{}", "0".repeat(48)), None).await.status, StatusCode::NOT_FOUND);
    assert_eq!(app.get("/api/invites/kurz", None).await.status, StatusCode::NOT_FOUND);

    // Gast tritt bei: Name nötig, bekommt eigene ID + Cookie
    let join = format!("/api/invites/{token}/join");
    assert_eq!(app.post(&join, None, json!({"name": "  "})).await.status, StatusCode::BAD_REQUEST);
    let r = app.post(&join, None, json!({"name": "Mila"})).await;
    assert_eq!(r.status, StatusCode::OK, "{}", String::from_utf8_lossy(&r.bytes));
    let guest = cookie_of(&r);
    let me = r.json()["me"].clone();
    let gid = me["name"].as_str().unwrap().to_string();
    assert!(gid.starts_with("Mila#") && gid.len() == 9, "{gid}");
    assert_eq!((me["guest"].as_bool(), me["email"].as_str()), (Some(true), Some("")));
    assert_eq!(r.json()["board"]["role"], "edit");
    let me2 = app.get("/api/me", Some(&guest)).await.json();
    assert_eq!(me2["name"], gid.as_str());

    // Zweiter Gast mit gleichem Namen bekommt eine andere ID; per Ansehen-Link nur view
    let r = app.post(&format!("/api/invites/{}/join", view["token"].as_str().unwrap()), None, json!({"name": "Mila"})).await;
    let guest2 = cookie_of(&r);
    assert_ne!(r.json()["me"]["name"], gid.as_str());
    assert_eq!(r.json()["board"]["role"], "view");

    // Gast sieht/bearbeitet das Board, darf aber nichts Eigenes
    let l = app.get("/api/boards", Some(&guest)).await.json();
    assert_eq!((l[0]["id"].as_str(), l[0]["role"].as_str()), (Some(id.as_str()), Some("edit")));
    assert_eq!(app.put_board(&guest, &id, "1", zip_bytes(20)).await.status, StatusCode::OK);
    assert_eq!(app.put_board(&guest2, &id, "2", zip_bytes(20)).await.code(), "forbidden");
    assert_eq!(app.upload(&guest, "eigenes", zip_bytes(1)).await.code(), "license_required");
    assert_eq!(app.post("/api/license/redeem", Some(&guest), json!({"code": code})).await.status, StatusCode::FORBIDDEN);
    assert_eq!(app.post(&invites, Some(&guest), json!({"role": "edit"})).await.status, StatusCode::FORBIDDEN);
    assert_eq!(app.post("/api/me/password", Some(&guest), json!({"old": "", "new": "geheim12345"})).await.status, StatusCode::FORBIDDEN);
    // per E-Mail sind Gäste nicht einladbar, in der Mitgliederliste ohne Adresse
    let members = app.get(&format!("/api/boards/{id}/members"), Some(&owner)).await.json();
    let g = members.as_array().unwrap().iter().find(|m| m["name"] == gid.as_str()).unwrap();
    assert_eq!((g["guest"].as_bool(), g["email"].as_str()), (Some(true), Some("")));
    let gmail = app.state.db.call(|c| Ok(c.query_row("SELECT email FROM users WHERE guest=1 LIMIT 1", [], |r| r.get::<_, String>(0))?)).await.unwrap();
    assert_eq!(app.post(&format!("/api/boards/{id}/members"), Some(&owner), json!({"email": gmail, "role": "edit"})).await.status, StatusCode::NOT_FOUND);
    assert_eq!(app.login(&gmail, "!").await.status, StatusCode::UNAUTHORIZED);

    // Konto per Link: wird Mitglied; edit bleibt bei späterem view-Link
    let r = app.post(&join, Some(&other), json!({})).await;
    assert_eq!(r.json()["board"]["role"], "edit");
    let r = app.post(&format!("/api/invites/{}/join", view["token"].as_str().unwrap()), Some(&other), json!({})).await;
    assert_eq!(r.json()["board"]["role"], "edit");
    assert!(r.headers.get(header::SET_COOKIE).is_none());
    assert_eq!(app.get(&invites, Some(&owner)).await.json().as_array().unwrap().iter().map(|i| i["uses"].as_i64().unwrap()).sum::<i64>(), 4);

    // Gast legt ein Konto an: Mitgliedschaft wandert mit, Gast ist weg
    let r = app.api(Method::POST, "/api/auth/register", Some(&guest2), Some(json!({"email": "mila@example.org", "password": "geheim1234", "name": "Mila"}))).await;
    assert_eq!(r.status, StatusCode::OK);
    let mila = cookie_of(&r);
    assert_eq!(app.get("/api/boards", Some(&mila)).await.json()[0]["role"], "view");
    assert_eq!(app.get("/api/me", Some(&guest2)).await.status, StatusCode::UNAUTHORIZED);

    // Abmelden löscht den Gast samt Zugriff
    assert_eq!(app.post("/api/auth/logout", Some(&guest), json!({})).await.status, StatusCode::OK);
    let n: i64 = app.state.db.call(|c| Ok(c.query_row("SELECT COUNT(*) FROM users WHERE guest=1", [], |r| r.get(0))?)).await.unwrap();
    assert_eq!(n, 0);

    // Link löschen → ungültig; abgelaufene Lizenz → kein Beitritt
    let r = app.api(Method::DELETE, &format!("{invites}/{}", edit["id"].as_str().unwrap()), Some(&owner), None).await;
    assert_eq!(r.json().as_array().unwrap().len(), 1);
    assert_eq!(app.post(&join, None, json!({"name": "Ben"})).await.status, StatusCode::NOT_FOUND);
    app.patch(&format!("/api/admin/licenses/{code}"), Some(&admin), json!({"expires_at": now() - 1})).await;
    let r = app.post(&format!("/api/invites/{}/join", view["token"].as_str().unwrap()), None, json!({"name": "Ben"})).await;
    assert_eq!(r.code(), "license_required");
}

#[tokio::test]
async fn gast_beitritt_hat_rate_limit() {
    let app = App::with(|c| c.rate_limit = 3);
    let admin = app.admin().await;
    let owner = app.register("owner@example.org").await;
    app.give_license(&admin, &owner, json!(null), 10).await;
    let id = app.upload(&owner, "x", zip_bytes(1)).await.json()["id"].as_str().unwrap().to_string();
    let token = app.post(&format!("/api/boards/{id}/invites"), Some(&owner), json!({"role": "view"})).await.json()["token"].as_str().unwrap().to_string();
    let join = format!("/api/invites/{token}/join");
    // Registrierungen (owner oben) und neue Gäste teilen sich das Limit je IP
    assert_eq!(app.post(&join, None, json!({"name": "A"})).await.status, StatusCode::OK);
    assert_eq!(app.post(&join, None, json!({"name": "B"})).await.status, StatusCode::OK);
    assert_eq!(app.post(&join, None, json!({"name": "C"})).await.status, StatusCode::TOO_MANY_REQUESTS);
}

#[tokio::test]
async fn mitteilungen_anlegen_zeitraum_und_rechte() {
    let app = App::new();
    let admin = app.admin().await;
    let user = app.register("u@example.org").await;

    // Ohne Mitteilungen: leere Liste, auch ohne Anmeldung
    let r = app.get("/api/notices", None).await;
    assert_eq!(r.status, StatusCode::OK);
    assert_eq!(r.json(), json!([]));

    // Nur Admins dürfen anlegen
    let r = app.post("/api/admin/notices", Some(&user), json!({"title": "x"})).await;
    assert_eq!(r.status, StatusCode::FORBIDDEN);
    let r = app.post("/api/admin/notices", Some(&admin), json!({"title": " ", "body": ""})).await;
    assert_eq!((r.status, r.code().as_str()), (StatusCode::BAD_REQUEST, "invalid"));
    let r = app.post("/api/admin/notices", Some(&admin), json!({"title": "x", "kind": "bunt"})).await;
    assert_eq!(r.status, StatusCode::BAD_REQUEST);
    let r = app.post("/api/admin/notices", Some(&admin), json!({"title": "x", "starts_at": 200, "ends_at": 100})).await;
    assert_eq!(r.status, StatusCode::BAD_REQUEST);

    let t = now();
    let r = app.post("/api/admin/notices", Some(&admin), json!({"title": "Neu in 4.8", "body": "Mitteilungen\n<b>und</b> Wartung", "kind": "info"})).await;
    assert_eq!(r.status, StatusCode::OK, "{}", String::from_utf8_lossy(&r.bytes));
    let n = r.json();
    let id = n["id"].as_str().unwrap().to_string();
    assert_eq!((n["active"].clone(), n["visible"].clone()), (json!(true), json!(true)));
    // Geplant (Zukunft), abgelaufen und inaktiv: nicht öffentlich
    app.post("/api/admin/notices", Some(&admin), json!({"title": "Später", "starts_at": t + 3600})).await;
    app.post("/api/admin/notices", Some(&admin), json!({"title": "Vorbei", "ends_at": t - 10})).await;
    app.post("/api/admin/notices", Some(&admin), json!({"title": "Aus", "active": false, "kind": "warning"})).await;

    let r = app.get("/api/notices", None).await;
    let list = r.json();
    assert_eq!(list.as_array().unwrap().len(), 1);
    assert_eq!(list[0]["title"], "Neu in 4.8");
    assert_eq!(list[0]["body"], "Mitteilungen\n<b>und</b> Wartung");
    assert!(list[0].get("active").is_none(), "öffentlich nur das Nötige");
    assert_eq!(app.get("/api/admin/notices", Some(&admin)).await.json().as_array().unwrap().len(), 4);
    assert_eq!(app.get("/api/admin/notices", Some(&user)).await.status, StatusCode::FORBIDDEN);

    // Bearbeiten: Zeitraum entfernen, Typ ändern; deaktivieren blendet aus
    let r = app.patch(&format!("/api/admin/notices/{id}"), Some(&admin), json!({"kind": "warning", "ends_at": null, "title": "Geändert"})).await;
    assert_eq!(r.status, StatusCode::OK);
    assert_eq!(r.json()["kind"], "warning");
    assert_eq!(app.get("/api/notices", None).await.json()[0]["title"], "Geändert");
    app.patch(&format!("/api/admin/notices/{id}"), Some(&admin), json!({"active": false})).await;
    assert_eq!(app.get("/api/notices", None).await.json(), json!([]));

    // Löschen
    let r = app.api(Method::DELETE, &format!("/api/admin/notices/{id}"), Some(&admin), None).await;
    assert_eq!(r.status, StatusCode::OK);
    let r = app.api(Method::DELETE, &format!("/api/admin/notices/{id}"), Some(&admin), None).await;
    assert_eq!(r.status, StatusCode::NOT_FOUND);
    assert_eq!(app.get("/api/admin/notices", Some(&admin)).await.json().as_array().unwrap().len(), 3);
}

#[tokio::test]
async fn wartungsmodus_sperrt_nutzer_aber_nicht_admins() {
    let app = App::new();
    let admin = app.admin().await;
    let user = app.register("u@example.org").await;
    app.give_license(&admin, &user, json!(null), 100).await;
    let r = app.upload(&user, "Vorher", zip_bytes(0)).await;
    assert_eq!(r.status, StatusCode::OK);
    let id = r.json()["id"].as_str().unwrap().to_string();

    assert_eq!(app.get("/api/maintenance", None).await.json()["active"], false);
    // Nur Admins schalten
    let r = app.api(Method::PUT, "/api/admin/maintenance", Some(&user), Some(json!({"active": true}))).await;
    assert_eq!(r.status, StatusCode::FORBIDDEN);
    let r = app.api(Method::PUT, "/api/admin/maintenance", Some(&admin), Some(json!({"active": true, "until": now() - 5}))).await;
    assert_eq!(r.status, StatusCode::BAD_REQUEST, "Endzeit in der Vergangenheit");
    let r = app.api(Method::PUT, "/api/admin/maintenance", Some(&admin), Some(json!({"active": true, "message": "Umzug auf neuen Server"}))).await;
    assert_eq!(r.status, StatusCode::OK);
    assert_eq!(r.json()["active"], true);

    // Öffentlicher Zustand, Health und Mitteilungen bleiben erreichbar
    let m = app.get("/api/maintenance", None).await.json();
    assert_eq!((m["active"].clone(), m["message"].clone()), (json!(true), json!("Umzug auf neuen Server")));
    let h = app.get("/api/health", None).await;
    assert_eq!((h.status, h.json()["maintenance"].clone()), (StatusCode::OK, json!(true)));
    assert_eq!(app.get("/api/notices", None).await.status, StatusCode::OK);

    // Nutzer: Lesen, Speichern, Hochladen, Registrieren -> 503 maintenance mit Nachricht
    let r = app.get("/api/boards", Some(&user)).await;
    assert_eq!((r.status, r.code().as_str()), (StatusCode::SERVICE_UNAVAILABLE, "maintenance"));
    assert_eq!(r.json()["message"], "Umzug auf neuen Server");
    assert!(r.headers.get(header::RETRY_AFTER).is_some());
    assert_eq!(app.put_board(&user, &id, "1", zip_bytes(1)).await.status, StatusCode::SERVICE_UNAVAILABLE);
    assert_eq!(app.upload(&user, "Neu", zip_bytes(0)).await.status, StatusCode::SERVICE_UNAVAILABLE);
    let r = app.post("/api/auth/register", None, json!({"email": "n@example.org", "password": "geheim1234", "name": "N"})).await;
    assert_eq!(r.status, StatusCode::SERVICE_UNAVAILABLE);
    assert_eq!(app.get(&format!("/api/collab/{id}"), Some(&user)).await.status, StatusCode::SERVICE_UNAVAILABLE);
    // Anmelden und /api/me bleiben möglich (Admins müssen sich anmelden können)
    assert_eq!(app.get("/api/me", Some(&user)).await.status, StatusCode::OK);

    // Admins arbeiten weiter
    assert_eq!(app.get("/api/boards", Some(&admin)).await.status, StatusCode::OK);
    assert_eq!(app.get("/api/admin/users", Some(&admin)).await.status, StatusCode::OK);
    // Nicht-Admin auf Admin-Routen weiterhin 403 (nicht 503)
    assert_eq!(app.get("/api/admin/users", Some(&user)).await.status, StatusCode::FORBIDDEN);

    // Web-App: Nutzer und Gäste bekommen die Wartungsseite, Admins die App; andere Dateien bleiben
    let r = app.send(Method::GET, "/", None, &[], Body::empty(), false).await;
    assert_eq!(r.status, StatusCode::SERVICE_UNAVAILABLE);
    let html = String::from_utf8_lossy(&r.bytes).to_string();
    assert!(html.contains("Wartungsarbeiten") && html.contains("Umzug auf neuen Server"), "{html}");
    let r = app.send(Method::GET, "/index.html", Some(&user), &[], Body::empty(), false).await;
    assert_eq!(r.status, StatusCode::SERVICE_UNAVAILABLE);
    let r = app.send(Method::GET, "/", Some(&admin), &[], Body::empty(), false).await;
    assert_eq!(r.status, StatusCode::OK);
    assert!(String::from_utf8_lossy(&r.bytes).contains("OwnDraw Web"));
    let r = app.send(Method::GET, "/admin", None, &[], Body::empty(), false).await;
    assert_eq!(r.status, StatusCode::OK);

    // HTML in der Nachricht wird maskiert
    app.api(Method::PUT, "/api/admin/maintenance", Some(&admin), Some(json!({"active": true, "message": "<script>x</script>"}))).await;
    let r = app.send(Method::GET, "/", None, &[], Body::empty(), false).await;
    let html = String::from_utf8_lossy(&r.bytes).to_string();
    assert!(html.contains("&lt;script&gt;x&lt;/script&gt;") && !html.contains("<script>x"));

    // Ausschalten: alles wieder normal; Zustand überlebt einen Neustart (DB)
    let r = app.api(Method::PUT, "/api/admin/maintenance", Some(&admin), Some(json!({"active": false}))).await;
    assert_eq!(r.json()["active"], false);
    assert_eq!(app.get("/api/boards", Some(&user)).await.status, StatusCode::OK);
    assert_eq!(app.send(Method::GET, "/", None, &[], Body::empty(), false).await.status, StatusCode::OK);

    app.api(Method::PUT, "/api/admin/maintenance", Some(&admin), Some(json!({"active": true}))).await;
    *app.state.maintenance.write().unwrap() = None; // Zwischenspeicher verwerfen wie nach Neustart
    let r = app.get("/api/boards", Some(&user)).await;
    assert_eq!(r.status, StatusCode::SERVICE_UNAVAILABLE);
    assert!(r.json()["message"].as_str().unwrap().contains("gewartet"), "Standardtext ohne eigene Nachricht");
}

#[tokio::test]
async fn wartungsmodus_endet_automatisch() {
    let app = App::new();
    let admin = app.admin().await;
    let user = app.register("u@example.org").await;
    let r = app.api(Method::PUT, "/api/admin/maintenance", Some(&admin), Some(json!({"active": true, "until": now() + 2}))).await;
    assert_eq!(r.status, StatusCode::OK);
    assert_eq!(app.get("/api/boards", Some(&user)).await.status, StatusCode::SERVICE_UNAVAILABLE);
    tokio::time::sleep(std::time::Duration::from_millis(2100)).await;
    assert_eq!(app.get("/api/boards", Some(&user)).await.status, StatusCode::OK);
    assert_eq!(app.get("/api/maintenance", None).await.json()["active"], false);
}
