//! Integrationstests der Live-Zusammenarbeit: echter TCP-Server (WebSocket), HTTP über `oneshot` auf denselben
//! Zustand.

use axum::body::{to_bytes, Body};
use axum::http::{header, Method, Request, StatusCode};
use axum::Router;
use futures_util::{SinkExt, StreamExt};
use owndraw_core::collab::{change_to_op, ops_to_json, Change};
use owndraw_core::doc::{read_owndraw, write_owndraw, Document};
use owndraw_core::item::{Item, INK, POLY};
use owndraw_core::pages::BoardMode;
use owndraw_server::config::Config;
use owndraw_server::db::Db;
use owndraw_server::state::AppState;
use owndraw_server::{auth, build_app};
use serde_json::{json, Value};
use std::rc::Rc;
use std::time::Duration;
use tokio_tungstenite::tungstenite::client::IntoClientRequest;
use tokio_tungstenite::tungstenite::Message;
use tower::ServiceExt;

type Ws = tokio_tungstenite::WebSocketStream<tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>>;

struct Srv {
    router: Router,
    state: AppState,
    addr: std::net::SocketAddr,
    _dir: tempfile::TempDir,
}

impl Srv {
    async fn start() -> Srv {
        let dir = tempfile::tempdir().unwrap();
        let web = dir.path().join("web");
        std::fs::create_dir_all(&web).unwrap();
        let cfg = Config {
            data: dir.path().join("data"),
            web,
            bind: "127.0.0.1:0".into(),
            max_board_bytes: 8 * 1024 * 1024,
            rate_limit: 1000,
            rate_window_secs: 600,
            release_url: "http://127.0.0.1:1/none".into(),
        };
        std::fs::create_dir_all(cfg.boards_dir()).unwrap();
        let state = AppState::new(Db::memory().unwrap(), cfg);
        let router = build_app(state.clone());
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let r2 = router.clone();
        tokio::spawn(async move { axum::serve(listener, r2.into_make_service_with_connect_info::<std::net::SocketAddr>()).await.unwrap() });
        Srv { router, state, addr, _dir: dir }
    }

    async fn req(&self, method: Method, path: &str, cookie: Option<&str>, headers: &[(&str, &str)], body: Body) -> (StatusCode, axum::http::HeaderMap, Vec<u8>) {
        let mut b = Request::builder().method(method).uri(path).header("x-owndraw", "1");
        if let Some(c) = cookie {
            b = b.header(header::COOKIE, c);
        }
        for (k, v) in headers {
            b = b.header(*k, *v);
        }
        let resp = self.router.clone().oneshot(b.body(body).unwrap()).await.unwrap();
        let (parts, body) = resp.into_parts();
        (parts.status, parts.headers, to_bytes(body, 64 << 20).await.unwrap().to_vec())
    }

    async fn json(&self, method: Method, path: &str, cookie: Option<&str>, body: Value) -> (StatusCode, Value, axum::http::HeaderMap) {
        let (s, h, b) = self.req(method, path, cookie, &[("content-type", "application/json")], Body::from(body.to_string())).await;
        (s, serde_json::from_slice(&b).unwrap_or(Value::Null), h)
    }

    async fn register(&self, email: &str, name: &str) -> String {
        let (s, _, h) = self.json(Method::POST, "/api/auth/register", None, json!({"email": email, "password": "geheim1234", "name": name})).await;
        assert_eq!(s, StatusCode::OK);
        h.get(header::SET_COOKIE).unwrap().to_str().unwrap().split(';').next().unwrap().to_string()
    }

    async fn admin(&self) -> String {
        auth::create_admin(&self.state.db, "admin@example.org", "adminpasswort").await.unwrap();
        let (s, _, h) = self.json(Method::POST, "/api/auth/login", None, json!({"email": "admin@example.org", "password": "adminpasswort"})).await;
        assert_eq!(s, StatusCode::OK);
        h.get(header::SET_COOKIE).unwrap().to_str().unwrap().split(';').next().unwrap().to_string()
    }

    async fn license(&self, admin: &str, user: &str) -> String {
        let (s, v, _) = self.json(Method::POST, "/api/admin/licenses", Some(admin), json!({"count": 1, "days": null, "quota_mb": 100, "note": ""})).await;
        assert_eq!(s, StatusCode::OK);
        let code = v[0]["code"].as_str().unwrap().to_string();
        let (s, _, _) = self.json(Method::POST, "/api/license/redeem", Some(user), json!({"code": code})).await;
        assert_eq!(s, StatusCode::OK);
        code
    }

    /// Board mit einem Objekt ohne uid (altes Format) anlegen.
    async fn board(&self, owner: &str) -> String {
        let mut d = Document::new("Live", BoardMode::Infinite);
        d.board["items"] = json!([{"t":"g","c":-16777216,"w":2.0,"k":5,"h":false,"q":[0.0,0.0,10.0,0.0,10.0,10.0],"ly":1}]);
        let bytes = write_owndraw(&d).unwrap();
        let (s, _, b) = self.req(Method::POST, "/api/boards", Some(owner), &[("content-type", "application/octet-stream"), ("x-board-name", "Live")], Body::from(bytes)).await;
        assert_eq!(s, StatusCode::OK, "{}", String::from_utf8_lossy(&b));
        serde_json::from_slice::<Value>(&b).unwrap()["id"].as_str().unwrap().to_string()
    }

    async fn ws(&self, board: &str, cookie: &str) -> Ws {
        let mut req = format!("ws://{}/api/collab/{board}", self.addr).into_client_request().unwrap();
        req.headers_mut().insert("cookie", cookie.parse().unwrap());
        let (ws, _) = tokio_tungstenite::connect_async(req).await.unwrap();
        ws
    }
}

async fn recv(ws: &mut Ws) -> Value {
    loop {
        let m = tokio::time::timeout(Duration::from_secs(5), ws.next()).await.expect("Zeitüberschreitung").expect("Verbindung zu").unwrap();
        if let Message::Text(t) = m {
            return serde_json::from_str(&t).unwrap();
        }
    }
}

/// Nächste Nachricht einer Art (andere werden übersprungen).
async fn recv_t(ws: &mut Ws, t: &str) -> Value {
    loop {
        let v = recv(ws).await;
        if v["t"] == t {
            return v;
        }
    }
}

async fn send(ws: &mut Ws, v: Value) {
    ws.send(Message::Text(v.to_string().into())).await.unwrap();
}

fn upsert(x: f32) -> (u64, Value) {
    let it = Rc::new(Item::shape(POLY, vec![x, 0.0, x + 10.0, 0.0, x + 10.0, 10.0], INK, 2.0));
    (it.uid, ops_to_json(&[change_to_op(&Change::Upsert(it.clone(), None))]))
}

#[tokio::test]
async fn zwei_clients_op_kommt_an_und_wird_gespeichert() {
    let srv = Srv::start().await;
    let admin = srv.admin().await;
    let a = srv.register("a@example.org", "Anna").await;
    let b = srv.register("b@example.org", "Ben").await;
    srv.license(&admin, &a).await;
    let id = srv.board(&a).await;
    // ohne Einladung: kein Zugriff (404 vor dem Upgrade)
    let mut req = format!("ws://{}/api/collab/{id}", srv.addr).into_client_request().unwrap();
    req.headers_mut().insert("cookie", b.parse().unwrap());
    assert!(tokio_tungstenite::connect_async(req).await.is_err());
    let (s, _, _) = srv.json(Method::POST, &format!("/api/boards/{id}/members"), Some(&a), json!({"email": "b@example.org", "role": "edit"})).await;
    assert_eq!(s, StatusCode::OK);

    let mut wa = srv.ws(&id, &a).await;
    let ha = recv_t(&mut wa, "hello").await;
    assert_eq!(ha["proto"], 1);
    assert_eq!(ha["can_write"], true);
    assert_eq!(ha["you"]["role"], "owner");
    let items = ha["board"]["items"].as_array().unwrap();
    assert_eq!(items.len(), 1);
    assert_eq!(items[0]["uid"].as_str().unwrap().len(), 16, "Server vergibt fehlende uids");

    let mut wb = srv.ws(&id, &b).await;
    let hb = recv_t(&mut wb, "hello").await;
    assert_eq!(hb["can_write"], true, "eingeladen ohne eigene Lizenz: darf schreiben");
    assert_eq!(hb["peers"].as_array().unwrap().len(), 1);
    assert_eq!(hb["board"]["items"][0]["uid"], items[0]["uid"]);
    let j = recv_t(&mut wa, "join").await;
    assert_eq!(j["peer"]["name"], "Ben");

    // PUT während der Live-Sitzung: 409
    let (s, _, _) = srv.req(Method::PUT, &format!("/api/boards/{id}"), Some(&a), &[("if-match", "1")], Body::from(b"PK\x03\x04xx".to_vec())).await;
    assert_eq!(s, StatusCode::CONFLICT);

    // A zeichnet → B bekommt es, A bekommt die Bestätigung mit seiner cid
    let (uid, ops) = upsert(50.0);
    send(&mut wa, json!({"t": "ops", "cid": 7, "ops": ops})).await;
    let ob = recv_t(&mut wb, "ops").await;
    assert_eq!(ob["seq"], 1);
    assert_eq!(ob["ops"][0]["item"]["uid"], owndraw_core::item::uid_str(uid));
    let oa = recv_t(&mut wa, "ops").await;
    assert_eq!((oa["cid"].clone(), oa["peer"].clone()), (json!(7), ha["you"]["peer"].clone()));

    // Cursor geht nur an die anderen
    send(&mut wb, json!({"t": "cursor", "x": 12.5, "y": -3})).await;
    let c = recv_t(&mut wa, "cursor").await;
    assert_eq!((c["x"].as_f64(), c["peer"].clone()), (Some(12.5), hb["you"]["peer"].clone()));

    // ungültige Op → nack
    send(&mut wb, json!({"t": "ops", "cid": 3, "ops": [{"o": "up", "item": {"t": "s"}}]})).await;
    let n = recv_t(&mut wb, "nack").await;
    assert_eq!((n["cid"].clone(), n["code"].clone()), (json!(3), json!("invalid")));

    // B löscht das alte Objekt
    send(&mut wb, json!({"t": "ops", "cid": 1, "ops": [{"o": "del", "uid": items[0]["uid"]}]})).await;
    assert_eq!(recv_t(&mut wa, "ops").await["seq"], 2);

    // Bild hochladen (inhaltsadressiert) und wieder abrufen
    let png = owndraw_core::tiny_skia::Pixmap::new(2, 2).unwrap().encode_png().unwrap();
    let name = owndraw_core::doc::ImageStore::default().add(png.clone(), "png");
    let (s, _, _) = srv.req(Method::PUT, &format!("/api/boards/{id}/images/{name}"), Some(&b), &[], Body::from(png.clone())).await;
    assert_eq!(s, StatusCode::OK);
    let (s, _, _) = srv.req(Method::PUT, &format!("/api/boards/{id}/images/{}.png", "0".repeat(64)), Some(&b), &[], Body::from(png.clone())).await;
    assert_eq!(s, StatusCode::BAD_REQUEST, "Name passt nicht zum Inhalt");
    let (s, _, body) = srv.req(Method::GET, &format!("/api/boards/{id}/images/{name}"), Some(&a), &[], Body::empty()).await;
    assert_eq!((s, body == png), (StatusCode::OK, true));
    let img = json!({"t":"i","c":-16777216,"w":1.0,"x":0.0,"y":0.0,"iw":20.0,"ih":20.0,"f":name,"ly":1,"uid":"00000000000000aa"});
    send(&mut wb, json!({"t": "ops", "cid": 2, "ops": [{"o": "up", "item": img, "after": null}]})).await;
    recv_t(&mut wa, "ops").await;

    // beide verlassen → gespeichert (neue Version, Inhalt enthält die Änderungen)
    wa.close(None).await.unwrap();
    wb.close(None).await.unwrap();
    let mut doc = None;
    for _ in 0..50 {
        tokio::time::sleep(Duration::from_millis(100)).await;
        let (s, h, bytes) = srv.req(Method::GET, &format!("/api/boards/{id}"), Some(&a), &[], Body::empty()).await;
        assert_eq!(s, StatusCode::OK);
        if h.get("x-board-version").unwrap().to_str().unwrap() != "1" && !srv.state.hub.is_active(&id) {
            doc = Some(read_owndraw(&bytes).unwrap());
            break;
        }
    }
    let doc = doc.expect("nicht gespeichert");
    let items = doc.board["items"].as_array().unwrap();
    assert_eq!(items.len(), 2);
    // after: null = ganz unten → Bild vor dem Strich
    assert_eq!(items[0]["uid"], "00000000000000aa");
    assert_eq!(items[1]["uid"], owndraw_core::item::uid_str(uid));
    assert_eq!(doc.images.files.len(), 1, "hochgeladenes Bild ist im Board");
    assert!(doc.thumb.is_some());
    // nach dem Schließen geht PUT wieder
    let (s, _, _) = srv.req(Method::PUT, &format!("/api/boards/{id}"), Some(&a), &[("if-match", "2")], Body::from(write_owndraw(&doc).unwrap())).await;
    assert_eq!(s, StatusCode::OK);
}

#[tokio::test]
async fn view_darf_nicht_schreiben() {
    let srv = Srv::start().await;
    let admin = srv.admin().await;
    let a = srv.register("a@example.org", "Anna").await;
    let v = srv.register("v@example.org", "Vera").await;
    srv.license(&admin, &a).await;
    let id = srv.board(&a).await;
    srv.json(Method::POST, &format!("/api/boards/{id}/members"), Some(&a), json!({"email": "v@example.org", "role": "view"})).await;
    let mut wa = srv.ws(&id, &a).await;
    recv_t(&mut wa, "hello").await;
    let mut wv = srv.ws(&id, &v).await;
    let h = recv_t(&mut wv, "hello").await;
    assert_eq!((h["can_write"].clone(), h["you"]["role"].clone()), (json!(false), json!("view")));
    let (_, ops) = upsert(1.0);
    send(&mut wv, json!({"t": "ops", "cid": 1, "ops": ops})).await;
    let n = recv_t(&mut wv, "nack").await;
    assert_eq!(n["code"], "forbidden");
    // Besitzer schreibt: view empfängt
    let (_, ops) = upsert(2.0);
    send(&mut wa, json!({"t": "ops", "cid": 1, "ops": ops})).await;
    let o = recv_t(&mut wv, "ops").await;
    assert_eq!(o["seq"], 1, "die abgelehnte Op hat keine Sequenznummer bekommen");
    // Bild hochladen als view: verboten
    let (s, _, _) = srv.req(Method::PUT, &format!("/api/boards/{id}/images/{}.png", "a".repeat(64)), Some(&v), &[], Body::from(vec![1u8])).await;
    assert_eq!(s, StatusCode::BAD_REQUEST);
    // Rolle ändern: Verbindung wird beendet (neu verbinden bringt die neue Rolle)
    srv.json(Method::POST, &format!("/api/boards/{id}/members"), Some(&a), json!({"email": "v@example.org", "role": "edit"})).await;
    let e = recv_t(&mut wv, "error").await;
    assert_eq!(e["code"], "role_changed");
    let mut wv = srv.ws(&id, &v).await;
    assert_eq!(recv_t(&mut wv, "hello").await["can_write"], true);
}

#[tokio::test]
async fn ohne_besitzer_lizenz_kein_schreiben() {
    let srv = Srv::start().await;
    let admin = srv.admin().await;
    let a = srv.register("a@example.org", "Anna").await;
    let b = srv.register("b@example.org", "Ben").await;
    let code = srv.license(&admin, &a).await;
    let id = srv.board(&a).await;
    srv.json(Method::POST, &format!("/api/boards/{id}/members"), Some(&a), json!({"email": "b@example.org", "role": "edit"})).await;
    let (s, _, _) = srv.json(Method::PATCH, &format!("/api/admin/licenses/{code}"), Some(&admin), json!({"revoked": true})).await;
    assert_eq!(s, StatusCode::OK);
    for who in [&a, &b] {
        let mut w = srv.ws(&id, who).await;
        let h = recv_t(&mut w, "hello").await;
        assert_eq!(h["can_write"], false);
        assert_eq!(h["board"]["items"].as_array().unwrap().len(), 1, "lesen geht weiter");
        let (_, ops) = upsert(3.0);
        send(&mut w, json!({"t": "ops", "cid": 9, "ops": ops})).await;
        let n = recv_t(&mut w, "nack").await;
        assert_eq!((n["cid"].clone(), n["code"].clone()), (json!(9), json!("license_required")));
    }
}

#[tokio::test]
async fn fremde_origin_und_geloeschtes_board() {
    let srv = Srv::start().await;
    let admin = srv.admin().await;
    let a = srv.register("a@example.org", "Anna").await;
    srv.license(&admin, &a).await;
    let id = srv.board(&a).await;
    let mut req = format!("ws://{}/api/collab/{id}", srv.addr).into_client_request().unwrap();
    req.headers_mut().insert("cookie", a.parse().unwrap());
    req.headers_mut().insert("origin", "https://boese.example".parse().unwrap());
    assert!(tokio_tungstenite::connect_async(req).await.is_err(), "fremde Seite abgelehnt");
    let mut req = format!("ws://{}/api/collab/{id}", srv.addr).into_client_request().unwrap();
    req.headers_mut().insert("cookie", a.parse().unwrap());
    req.headers_mut().insert("origin", format!("http://{}", srv.addr).parse().unwrap());
    let (mut w, _) = tokio_tungstenite::connect_async(req).await.unwrap();
    recv_t(&mut w, "hello").await;
    let (s, _, _) = srv.json(Method::DELETE, &format!("/api/boards/{id}"), Some(&a), json!({})).await;
    assert_eq!(s, StatusCode::OK);
    assert_eq!(recv_t(&mut w, "error").await["code"], "not_found");
    assert!(!srv.state.hub.is_active(&id));
}
