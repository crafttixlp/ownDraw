//! Formelsammlungen der App: Grundsammlung, eigene Sammlungen (im Konto am Server, ohne Konto auf dem Gerät),
//! abonnierte (per Code geteilte) und vom Admin für alle bereitgestellte Sammlungen.
//!
//! Zustand thread-lokal wie `net`; Antworten des Servers kommen asynchron. Server-API: `/api/formulas…`
//! (siehe `crates/owndraw-server/src/formulas.rs`). Ohne Anmeldung liegen eigene Sammlungen in `formeln.json`
//! (Konfigurationsordner bzw. localStorage). Ausgeschaltete Sammlungen: Einstellung `formulas_off` (Liste der Schlüssel).

#[cfg(not(target_os = "android"))]
use crate::net::{self, Body};
#[cfg(target_os = "android")]
use net::Body;
use crate::settings::Settings;
use owndraw_core::calc::library::{self, Collection};
use owndraw_core::calc::Prelude;
use serde_json::{json, Value};
use std::cell::RefCell;
use std::path::Path;

/// Herkunft einer Sammlung.
#[derive(Clone, Debug, PartialEq)]
pub enum Src {
    Builtin,
    /// Auf diesem Gerät (ohne Konto).
    Local,
    /// Im Konto am Server (`mine` = eigene, sonst abonniert bzw. für alle bereitgestellt).
    Server { id: String, mine: bool, owner: String, code: Option<String>, global: bool },
}

#[derive(Clone, Debug)]
pub struct Set {
    pub src: Src,
    pub col: Collection,
}

impl Set {
    /// Dauerhafter Schlüssel (Einstellung `formulas_off`).
    pub fn key(&self) -> String {
        match &self.src {
            Src::Builtin => "builtin".into(),
            Src::Local => format!("loc:{}", self.col.name),
            Src::Server { id, .. } => format!("srv:{id}"),
        }
    }
    pub fn editable(&self) -> bool {
        matches!(self.src, Src::Local | Src::Server { mine: true, .. })
    }
    /// Kurzer Herkunftstext für die Liste.
    pub fn origin(&self) -> String {
        match &self.src {
            Src::Builtin => "Mitgeliefert".into(),
            Src::Local => "Auf diesem Gerät".into(),
            Src::Server { mine: true, global: true, .. } => "Im Konto · für alle bereitgestellt".into(),
            Src::Server { mine: true, code: Some(_), .. } => "Im Konto · geteilt".into(),
            Src::Server { mine: true, .. } => "Im Konto".into(),
            Src::Server { global: true, owner, .. } => format!("Für alle bereitgestellt von {owner}"),
            Src::Server { owner, .. } => format!("Geteilt von {owner}"),
        }
    }
}

/// Vorschau einer per Code geteilten Sammlung (vor dem Abonnieren).
#[derive(Clone, Debug)]
pub struct Preview {
    pub code: String,
    pub owner: String,
    pub col: Collection,
}

#[derive(Default)]
pub struct Lib {
    pub sets: Vec<Set>,
    /// Zählt jede Änderung (Board übernimmt Konstanten/Einheiten neu).
    pub rev: u64,
    pub loaded_for: Option<String>,
    pub busy: bool,
    pub msg: Option<(String, bool)>,
    pub preview: Option<Preview>,
    local_loaded: bool,
}

thread_local! {
    static LIB: RefCell<Lib> = RefCell::new(Lib::default());
}

pub fn with<R>(f: impl FnOnce(&mut Lib) -> R) -> R {
    LIB.with(|l| f(&mut l.borrow_mut()))
}

const FILE: &str = "formeln.json";

fn note(msg: impl Into<String>, err: bool) {
    with(|l| {
        l.msg = Some((msg.into(), err));
        l.busy = false;
    });
}

/// Eigene Sammlungen am Server speichern? (angemeldet, kein Gast)
#[cfg(not(target_os = "android"))]
pub fn account() -> bool {
    net::with(|n| n.me.as_ref().is_some_and(|m| !m.guest))
}

#[cfg(not(target_os = "android"))]
fn me_id() -> Option<String> {
    net::with(|n| n.me.as_ref().map(|m| m.id.clone()))
}

/// Android: noch ohne Server-Anbindung (nur Gerät).
#[cfg(not(target_os = "android"))]
pub fn is_admin() -> bool {
    net::is_admin()
}

#[cfg(not(target_os = "android"))]
pub fn logged_in() -> bool {
    net::logged_in()
}

#[cfg(target_os = "android")]
pub fn is_admin() -> bool {
    false
}

#[cfg(target_os = "android")]
pub fn logged_in() -> bool {
    false
}

#[cfg(target_os = "android")]
pub fn account() -> bool {
    false
}

#[cfg(target_os = "android")]
fn me_id() -> Option<String> {
    None
}

#[cfg(target_os = "android")]
#[allow(dead_code)]
mod net {
    pub enum Body {
        Json(serde_json::Value),
    }
    pub struct Resp {
        pub status: u16,
    }
    impl Resp {
        pub fn ok(&self) -> bool {
            false
        }
        pub fn json(&self) -> serde_json::Value {
            serde_json::Value::Null
        }
        pub fn message(&self) -> String {
            String::new()
        }
    }
    pub fn request(_m: &'static str, _p: String, _b: Option<Body>, _h: Vec<(&'static str, String)>, done: impl FnOnce(Result<Resp, String>) + 'static) {
        done(Err("ohne Server".into()))
    }
}

/// Pro Frame: Grundsammlung + Gerät laden, nach An-/Abmeldung Serverliste holen.
pub fn tick(config_dir: &Path) {
    let first = with(|l| !std::mem::replace(&mut l.local_loaded, true));
    if first {
        let mut sets = vec![Set { src: Src::Builtin, col: library::builtin() }];
        if let Ok(b) = crate::kv::read(&config_dir.join(FILE)) {
            if let Ok(Value::Array(a)) = serde_json::from_slice::<Value>(&b) {
                for v in a {
                    if let Ok(c) = Collection::from_value(v) {
                        sets.push(Set { src: Src::Local, col: c });
                    }
                }
            }
        }
        with(|l| {
            l.sets = sets;
            l.rev += 1;
        });
    }
    let who = me_id();
    let stale = with(|l| l.loaded_for != who && !l.busy);
    if stale {
        with(|l| l.loaded_for = who.clone());
        if who.is_some() {
            refresh();
        } else {
            // abgemeldet: Server-Sammlungen entfernen
            with(|l| {
                l.sets.retain(|s| !matches!(s.src, Src::Server { .. }));
                l.rev += 1;
            });
        }
    }
}

/// Eigene Sammlungen auf dem Gerät speichern.
pub fn save_local(config_dir: &Path) {
    let a: Vec<Value> = with(|l| l.sets.iter().filter(|s| s.src == Src::Local).map(|s| s.col.to_value()).collect());
    if let Err(e) = crate::kv::write(&config_dir.join(FILE), serde_json::to_string(&a).unwrap_or_default().as_bytes()) {
        note(format!("Speichern auf dem Gerät fehlgeschlagen: {e}"), true);
    }
}

/// Serverliste neu laden.
pub fn refresh() {
    with(|l| l.busy = true);
    net::request("GET", "/api/formulas".into(), None, vec![], |r| match r {
        Ok(r) if r.ok() => {
            let mut srv = Vec::new();
            for v in r.json().get("sets").and_then(Value::as_array).cloned().unwrap_or_default() {
                let Some(id) = v.get("id").and_then(Value::as_str) else { continue };
                let mut col = match v.get("data").cloned().map(Collection::from_value) {
                    Some(Ok(c)) => c,
                    _ => continue,
                };
                if let Some(n) = v.get("name").and_then(Value::as_str) {
                    col.name = n.to_string();
                }
                srv.push(Set {
                    src: Src::Server {
                        id: id.to_string(),
                        mine: v.get("mine").and_then(Value::as_bool).unwrap_or(false),
                        owner: v.get("owner_name").and_then(Value::as_str).unwrap_or("").to_string(),
                        code: v.get("share_code").and_then(Value::as_str).map(String::from),
                        global: v.get("global").and_then(Value::as_bool).unwrap_or(false),
                    },
                    col,
                });
            }
            with(|l| {
                l.sets.retain(|s| !matches!(s.src, Src::Server { .. }));
                l.sets.extend(srv);
                l.busy = false;
                l.rev += 1;
            });
        }
        // Server ohne Formel-API (älter) oder offline: nur Gerät
        _ => with(|l| l.busy = false),
    });
}

/// Sammlung speichern (neu oder geändert). `i` = Index in `sets`.
pub fn save(i: usize, config_dir: &Path) {
    let Some(set) = with(|l| l.sets.get(i).cloned()) else { return };
    with(|l| l.rev += 1);
    match &set.src {
        Src::Builtin => {}
        Src::Local => save_local(config_dir),
        Src::Server { id, mine: true, .. } => {
            let body = json!({"name": set.col.name, "data": set.col.to_value()});
            net::request("PUT", format!("/api/formulas/{id}"), Some(Body::Json(body)), vec![], |r| match r {
                Ok(r) if r.ok() => {}
                Ok(r) => note(r.message(), true),
                Err(e) => note(format!("Server nicht erreichbar: {e}"), true),
            });
        }
        Src::Server { .. } => {}
    }
}

/// Neue Sammlung anlegen: mit Konto am Server, sonst auf dem Gerät. Liefert den Index (am Server erst nach Antwort).
pub fn create(col: Collection, config_dir: &Path) -> Option<usize> {
    if account() {
        upload(col, None, config_dir);
        None
    } else {
        let i = with(|l| {
            l.sets.push(Set { src: Src::Local, col });
            l.rev += 1;
            l.sets.len() - 1
        });
        save_local(config_dir);
        Some(i)
    }
}

/// Sammlung ins Konto hochladen (`from_local`: Index einer Gerät-Sammlung, die danach entfernt wird).
pub fn upload(col: Collection, from_local: Option<usize>, config_dir: &Path) {
    with(|l| l.busy = true);
    let body = json!({"name": col.name, "data": col.to_value()});
    let dir = config_dir.to_path_buf();
    let name = col.name.clone();
    net::request("POST", "/api/formulas".into(), Some(Body::Json(body)), vec![], move |r| match r {
        Ok(r) if r.ok() => {
            if let Some(i) = from_local {
                with(|l| {
                    if l.sets.get(i).is_some_and(|s| s.src == Src::Local && s.col.name == name) {
                        l.sets.remove(i);
                    }
                });
                save_local(&dir);
            }
            note(format!("„{name}“ im Konto gespeichert"), false);
            refresh();
        }
        Ok(r) => note(r.message(), true),
        Err(e) => note(format!("Server nicht erreichbar: {e}"), true),
    });
}

/// Löschen (eigene) bzw. Abo beenden (geteilte).
pub fn delete(i: usize, config_dir: &Path) {
    let Some(set) = with(|l| l.sets.get(i).cloned()) else { return };
    match set.src {
        Src::Builtin => {}
        Src::Local => {
            with(|l| {
                l.sets.remove(i);
                l.rev += 1;
            });
            save_local(config_dir);
        }
        Src::Server { id, .. } => {
            with(|l| {
                l.sets.remove(i);
                l.rev += 1;
            });
            net::request("DELETE", format!("/api/formulas/{id}"), None, vec![], |r| match r {
                Ok(r) if r.ok() => {}
                Ok(r) => {
                    note(r.message(), true);
                    refresh();
                }
                Err(e) => note(format!("Server nicht erreichbar: {e}"), true),
            });
        }
    }
}

/// Teilen ein/aus (Code für die Klasse).
pub fn share(i: usize, on: bool) {
    let Some(Src::Server { id, .. }) = with(|l| l.sets.get(i).map(|s| s.src.clone())) else { return };
    with(|l| l.busy = true);
    let method = if on { "POST" } else { "DELETE" };
    net::request(method, format!("/api/formulas/{id}/share"), None, vec![], move |r| match r {
        Ok(r) if r.ok() => {
            let code = r.json().get("code").and_then(Value::as_str).map(String::from);
            with(|l| {
                if let Some(Set { src: Src::Server { id: sid, code: c, .. }, .. }) = l.sets.get_mut(i) {
                    if *sid == id {
                        *c = if on { code } else { None };
                    }
                }
                l.busy = false;
                l.rev += 1;
            });
        }
        Ok(r) => note(r.message(), true),
        Err(e) => note(format!("Server nicht erreichbar: {e}"), true),
    });
}

/// Admin: für alle Nutzer bereitstellen.
pub fn set_global(i: usize, on: bool) {
    let Some(Src::Server { id, .. }) = with(|l| l.sets.get(i).map(|s| s.src.clone())) else { return };
    net::request("POST", format!("/api/formulas/{id}/global"), Some(Body::Json(json!({"on": on}))), vec![], |r| match r {
        Ok(r) if r.ok() => refresh(),
        Ok(r) => note(r.message(), true),
        Err(e) => note(format!("Server nicht erreichbar: {e}"), true),
    });
}

/// Code normalisieren (Leerzeichen/Bindestriche weg, groß).
pub fn norm_code(s: &str) -> String {
    s.chars().filter(|c| c.is_ascii_alphanumeric()).map(|c| c.to_ascii_uppercase()).collect()
}

/// Geteilte Sammlung per Code ansehen.
pub fn preview(code: &str) {
    let code = norm_code(code);
    if code.is_empty() {
        return;
    }
    with(|l| {
        l.busy = true;
        l.preview = None;
    });
    let c2 = code.clone();
    net::request("GET", format!("/api/formulas/code/{code}"), None, vec![], move |r| match r {
        Ok(r) if r.ok() => {
            let v = r.json();
            match v.get("data").cloned().map(Collection::from_value) {
                Some(Ok(mut col)) => {
                    if let Some(n) = v.get("name").and_then(Value::as_str) {
                        col.name = n.to_string();
                    }
                    let owner = v.get("owner_name").and_then(Value::as_str).unwrap_or("").to_string();
                    with(|l| {
                        l.preview = Some(Preview { code: c2, owner, col });
                        l.busy = false;
                        l.msg = None;
                    });
                }
                _ => note("Die geteilte Sammlung ist nicht lesbar.", true),
            }
        }
        Ok(r) if r.status == 404 => note("Kein Sammlung mit diesem Code gefunden.", true),
        Ok(r) => note(r.message(), true),
        Err(e) => note(format!("Server nicht erreichbar: {e}"), true),
    });
}

/// Geteilte Sammlung abonnieren (erscheint dann bei allen Geräten im Konto).
pub fn subscribe(code: &str) {
    let code = norm_code(code);
    with(|l| l.busy = true);
    net::request("POST", format!("/api/formulas/code/{code}"), None, vec![], |r| match r {
        Ok(r) if r.ok() => {
            with(|l| l.preview = None);
            note("Sammlung hinzugefügt", false);
            refresh();
        }
        Ok(r) => note(r.message(), true),
        Err(e) => note(format!("Server nicht erreichbar: {e}"), true),
    });
}

// ---------------------------------------------------------------- aktiv / Rechnen

fn off_list(s: &Settings) -> Vec<String> {
    s.extra.get("formulas_off").and_then(|v| v.as_array()).map(|a| a.iter().filter_map(|x| x.as_str().map(String::from)).collect()).unwrap_or_default()
}

pub fn is_active(s: &Settings, key: &str) -> bool {
    !off_list(s).iter().any(|k| k == key)
}

pub fn set_active(s: &mut Settings, key: &str, on: bool) {
    let mut l = off_list(s);
    l.retain(|k| k != key);
    if !on {
        l.push(key.to_string());
    }
    s.extra.insert("formulas_off".into(), json!(l));
    with(|l| l.rev += 1);
}

/// Aktive Sammlungen (Reihenfolge der Liste).
pub fn active(s: &Settings) -> Vec<Collection> {
    let off = off_list(s);
    with(|l| l.sets.iter().filter(|x| !off.contains(&x.key())).map(|x| x.col.clone()).collect())
}

/// Konstanten und Einheiten der aktiven Sammlungen.
pub fn prelude(s: &Settings) -> Prelude {
    library::prelude(active(s).iter())
}

/// Stand für den Zwischenspeicher (Änderungen an Sammlungen oder Auswahl).
pub fn rev() -> u64 {
    with(|l| l.rev)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn aktiv_schalten() {
        let mut s = Settings::default();
        assert!(is_active(&s, "builtin"));
        set_active(&mut s, "builtin", false);
        assert!(!is_active(&s, "builtin"));
        set_active(&mut s, "builtin", true);
        assert!(is_active(&s, "builtin"));
        assert_eq!(norm_code(" ab-cd 12 "), "ABCD12");
    }
}
