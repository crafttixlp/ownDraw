//! Transport im Browser: `fetch` (gleiche Origin, Cookie vom Browser) und `WebSocket`.

use super::{repaint, Body, Resp, SockEv};
use serde_json::Value;
use std::cell::RefCell;
use std::rc::Rc;
use wasm_bindgen::prelude::*;
use wasm_bindgen::JsCast;
use wasm_bindgen_futures::{spawn_local, JsFuture};

/// Beim Start: Einladung aus der Adresse (`#invite=<token>`) holen.
pub fn init(_ctx: &egui::Context) -> Option<String> {
    take_invite_from_url()
}

pub fn encode(s: &str) -> String {
    String::from(js_sys::encode_uri_component(s))
}

/// Adresse des Servers für Einladungslinks.
pub fn origin() -> String {
    web_sys::window().and_then(|w| w.location().origin().ok()).unwrap_or_default()
}

pub fn request(method: &'static str, path: String, body: Option<Body>, headers: Vec<(&'static str, String)>, done: Box<dyn FnOnce(Result<Resp, String>)>) {
    spawn_local(async move {
        let r = fetch(method, &path, body, &headers).await;
        done(r);
        repaint();
    });
}

fn js_text(e: JsValue) -> String {
    e.as_string().or_else(|| js_sys::Reflect::get(&e, &"message".into()).ok().and_then(|m| m.as_string())).unwrap_or_else(|| "Netzwerkfehler".into())
}

async fn fetch(method: &str, path: &str, body: Option<Body>, headers: &[(&str, String)]) -> Result<Resp, String> {
    let w = web_sys::window().ok_or("kein window")?;
    let init = web_sys::RequestInit::new();
    init.set_method(method);
    init.set_credentials(web_sys::RequestCredentials::SameOrigin);
    let h = web_sys::Headers::new().map_err(js_text)?;
    if method != "GET" {
        h.set("X-OwnDraw", "1").map_err(js_text)?;
    }
    match body {
        Some(Body::Json(v)) => {
            h.set("Content-Type", "application/json").map_err(js_text)?;
            init.set_body(&JsValue::from_str(&v.to_string()));
        }
        Some(Body::Bytes(b)) => {
            h.set("Content-Type", "application/octet-stream").map_err(js_text)?;
            init.set_body(&js_sys::Uint8Array::from(b.as_slice()));
        }
        None => {}
    }
    for (k, v) in headers {
        h.set(k, v).map_err(js_text)?;
    }
    init.set_headers(&h);
    let req = web_sys::Request::new_with_str_and_init(path, &init).map_err(js_text)?;
    let resp: web_sys::Response = JsFuture::from(w.fetch_with_request(&req)).await.map_err(js_text)?.dyn_into().map_err(js_text)?;
    let version = resp.headers().get("x-board-version").ok().flatten().and_then(|v| v.parse().ok());
    let buf = JsFuture::from(resp.array_buffer().map_err(js_text)?).await.map_err(js_text)?;
    Ok(Resp { status: resp.status(), version, body: js_sys::Uint8Array::new(&buf).to_vec() })
}


/// Liest `#invite=<token>` aus der Adresse, merkt die Einladung vor und entfernt das Fragment aus der Adresszeile.
fn take_invite_from_url() -> Option<String> {
    let w = web_sys::window()?;
    let loc = w.location();
    let hash = loc.hash().ok()?;
    let token = hash.strip_prefix("#invite=")?;
    let token = token.trim().to_string();
    if token.is_empty() {
        return None;
    }
    if let (Ok(path), Ok(search), Ok(h)) = (loc.pathname(), loc.search(), w.history()) {
        let _ = h.replace_state_with_url(&JsValue::NULL, "", Some(&format!("{path}{search}")));
    }
    Some(token)
}

/// WebSocket zu `/api/collab/{id}` (Cookie geht automatisch mit).
pub struct Socket {
    ws: web_sys::WebSocket,
    inbox: Rc<RefCell<Vec<SockEv>>>,
    _cbs: Vec<Closure<dyn FnMut(JsValue)>>,
}

impl Socket {
    pub fn connect(board: &str) -> Result<Socket, String> {
        let loc = web_sys::window().ok_or("kein window")?.location();
        let proto = if loc.protocol().map_err(js_text)? == "https:" { "wss" } else { "ws" };
        let url = format!("{proto}://{}/api/collab/{board}", loc.host().map_err(js_text)?);
        let ws = web_sys::WebSocket::new(&url).map_err(js_text)?;
        let inbox: Rc<RefCell<Vec<SockEv>>> = Rc::default();
        let mut cbs: Vec<Closure<dyn FnMut(JsValue)>> = Vec::new();
        let i = inbox.clone();
        cbs.push(Closure::new(move |_e: JsValue| {
            i.borrow_mut().push(SockEv::Open);
            repaint();
        }));
        ws.set_onopen(Some(cbs[0].as_ref().unchecked_ref()));
        let i = inbox.clone();
        cbs.push(Closure::new(move |e: JsValue| {
            let Ok(e) = e.dyn_into::<web_sys::MessageEvent>() else { return };
            if let Some(t) = e.data().as_string() {
                if let Ok(v) = serde_json::from_str(&t) {
                    i.borrow_mut().push(SockEv::Msg(v));
                    repaint();
                }
            }
        }));
        ws.set_onmessage(Some(cbs[1].as_ref().unchecked_ref()));
        let i = inbox.clone();
        cbs.push(Closure::new(move |e: JsValue| {
            let (code, reason) = e.dyn_into::<web_sys::CloseEvent>().map(|c| (c.code(), c.reason())).unwrap_or((1006, String::new()));
            i.borrow_mut().push(SockEv::Closed { code, reason });
            repaint();
        }));
        ws.set_onclose(Some(cbs[2].as_ref().unchecked_ref()));
        Ok(Socket { ws, inbox, _cbs: cbs })
    }

    pub fn send(&self, v: &Value) -> bool {
        self.ws.ready_state() == web_sys::WebSocket::OPEN && self.ws.send_with_str(&v.to_string()).is_ok()
    }

    pub fn take(&self) -> Vec<SockEv> {
        std::mem::take(&mut self.inbox.borrow_mut())
    }
}

impl Drop for Socket {
    fn drop(&mut self) {
        self.ws.set_onopen(None);
        self.ws.set_onmessage(None);
        self.ws.set_onclose(None);
        let _ = self.ws.close();
    }
}
