//! Handschrift-Erkennung: Anbindung an die Browser-Seite (`crates/owndraw-web/hw.js`, `window.owndrawHw`).
//!
//! Erkannt wird nur im Browser (onnxruntime-web). Desktop und Android zeigen „nur im Browser verfügbar“; der Code
//! oberhalb dieses Moduls (Werkzeug, Fenster, Einstellungen) ist plattformneutral.

use owndraw_core::hw::{Ink, Kandidat, Manifest};
use serde_json::Value;

/// Erkennung auf dieser Plattform möglich?
pub const SUPPORTED: bool = cfg!(target_arch = "wasm32");

/// Ein Paket vom Server mit Download-Zustand im Browser.
#[derive(Clone, Debug)]
pub struct Paket {
    pub m: Manifest,
    /// `fehlt`, `laedt`, `da` oder `fehler`.
    pub zustand: String,
    pub geladen: u64,
    pub gesamt: u64,
    pub fehler: String,
}

impl Paket {
    pub fn da(&self) -> bool {
        self.zustand == "da"
    }
}

/// Zustand der Browser-Seite.
#[derive(Clone, Debug, Default)]
pub struct Info {
    /// `leer`, `laedt`, `ok`, `fehler` (Paketliste vom Server).
    pub liste: String,
    pub fehler: String,
    pub pakete: Vec<Paket>,
    /// Browser bietet WebGPU an.
    pub webgpu: bool,
    /// Cache Storage verfügbar (sonst nur bis zum Neuladen gespeichert).
    pub cache: bool,
}

impl Info {
    pub fn manifests(&self) -> Vec<Manifest> {
        self.pakete.iter().map(|p| p.m.clone()).collect()
    }
    pub fn paket(&self, id: &str) -> Option<&Paket> {
        self.pakete.iter().find(|p| p.m.id == id)
    }
}

/// Liest `owndrawHw.info()`.
#[cfg_attr(not(target_arch = "wasm32"), allow(dead_code))]
pub fn parse_info(s: &str) -> Option<Info> {
    let v: Value = serde_json::from_str(s).ok()?;
    let st = |v: &Value, k: &str| v.get(k).and_then(|x| x.as_str()).unwrap_or("").to_string();
    let mut pakete = Vec::new();
    for p in v.get("pakete").and_then(|x| x.as_array()).into_iter().flatten() {
        // Zustandsfelder hängen am Manifest; das Manifest selbst bleibt gültig
        let Ok(m) = Manifest::from_value(p.clone()) else { continue };
        let gesamt = p.get("gesamt").and_then(|x| x.as_u64()).unwrap_or_else(|| m.groesse());
        pakete.push(Paket { zustand: st(p, "zustand"), geladen: p.get("geladen").and_then(|x| x.as_u64()).unwrap_or(0), gesamt, fehler: st(p, "fehler"), m });
    }
    Some(Info {
        liste: st(&v, "liste"),
        fehler: st(&v, "fehler"),
        pakete,
        webgpu: v.get("webgpu").and_then(|x| x.as_bool()).unwrap_or(false),
        cache: v.get("cache").and_then(|x| x.as_bool()).unwrap_or(false),
    })
}

#[cfg_attr(not(target_arch = "wasm32"), allow(dead_code))]
/// Ergebnis eines Auftrags (`owndrawHw.ergebnis`): Alternativen und Rechenzeit (ms) oder Fehlermeldung.
pub fn parse_result(s: &str) -> Option<Result<(Vec<Kandidat>, u32), String>> {
    if s.is_empty() {
        return None;
    }
    let v: Value = match serde_json::from_str(s) {
        Ok(v) => v,
        Err(e) => return Some(Err(format!("Antwort nicht lesbar: {e}"))),
    };
    if !v.get("ok").and_then(|x| x.as_bool()).unwrap_or(false) {
        return Some(Err(v.get("fehler").and_then(|x| x.as_str()).unwrap_or("Erkennung fehlgeschlagen").to_string()));
    }
    let k = v
        .get("kandidaten")
        .and_then(|x| x.as_array())
        .into_iter()
        .flatten()
        .filter_map(|c| Some(Kandidat::new(c.get("text")?.as_str()?, c.get("score").and_then(|x| x.as_f64()).unwrap_or(-10.0) as f32)))
        .collect();
    Some(Ok((k, v.get("ms").and_then(|x| x.as_u64()).unwrap_or(0) as u32)))
}

#[cfg(target_arch = "wasm32")]
mod js {
    use wasm_bindgen::{JsCast, JsValue};

    fn obj() -> Option<JsValue> {
        let w = web_sys::window()?;
        js_sys::Reflect::get(&w, &JsValue::from_str("owndrawHw")).ok().filter(|o| o.is_object())
    }

    pub fn call(name: &str, args: &[JsValue]) -> Option<JsValue> {
        let o = obj()?;
        let f: js_sys::Function = js_sys::Reflect::get(&o, &JsValue::from_str(name)).ok()?.dyn_into().ok()?;
        let a = js_sys::Array::new();
        for x in args {
            a.push(x);
        }
        f.apply(&o, &a).ok()
    }
}

/// Zustand der Browser-Seite; None außerhalb des Browsers oder solange `hw.js` noch nicht geladen ist.
pub fn info() -> Option<Info> {
    #[cfg(target_arch = "wasm32")]
    {
        js::call("info", &[]).and_then(|v| v.as_string()).and_then(|s| parse_info(&s))
    }
    #[cfg(not(target_arch = "wasm32"))]
    None
}

/// Paketliste vom Server neu holen.
pub fn refresh_list() {
    #[cfg(target_arch = "wasm32")]
    js::call("liste", &[]);
}

/// Pakete herunterladen und im Browser speichern (nur nach Klick der Nutzerin).
pub fn download(_ids: &[String]) {
    #[cfg(target_arch = "wasm32")]
    js::call("laden", &[wasm_bindgen::JsValue::from_str(&serde_json::to_string(_ids).unwrap_or_default())]);
}

/// Gespeicherte Modelle löschen.
pub fn clear() {
    #[cfg(target_arch = "wasm32")]
    js::call("loeschen", &[]);
}

/// Startet die Erkennung mit dem Paket `paket`; liefert die Auftragsnummer.
pub fn start(_paket: &str, _nbest: usize, _ink: &Ink) -> Option<u32> {
    #[cfg(target_arch = "wasm32")]
    {
        use wasm_bindgen::JsValue;
        let job = serde_json::json!({"paket": _paket, "nbest": _nbest}).to_string();
        let gray = js_sys::Uint8Array::from(_ink.gray.as_slice());
        js::call("erkennen", &[JsValue::from_str(&job), JsValue::from(_ink.w), JsValue::from(_ink.h), gray.into()]).and_then(|v| v.as_f64()).map(|n| n as u32)
    }
    #[cfg(not(target_arch = "wasm32"))]
    None
}

/// Ergebnis abholen: None = läuft noch.
pub fn poll(_job: u32) -> Option<Result<(Vec<Kandidat>, u32), String>> {
    #[cfg(target_arch = "wasm32")]
    {
        match js::call("ergebnis", &[wasm_bindgen::JsValue::from(_job)]).and_then(|v| v.as_string()) {
            Some(s) => parse_result(&s),
            None => Some(Err("Die Erkennung ist nicht geladen (hw.js fehlt).".into())),
        }
    }
    #[cfg(not(target_arch = "wasm32"))]
    Some(Err("Handschrift-Erkennung ist nur im Browser verfügbar.".into()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn info_und_ergebnis_lesen() {
        let s = r#"{"verfuegbar":true,"cache":true,"webgpu":false,"liste":"ok","fehler":"","pakete":[
            {"id":"formel-p2t-int8","art":"formel","stufen":["schlecht","mittel"],"lizenz":{"spdx":"MIT"},
             "dateien":[{"name":"encoder.onnx","groesse":10,"sha256":"aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"}],
             "zustand":"laedt","geladen":4,"gesamt":10},
            {"id":"kaputt","art":"bild"}]}"#;
        let i = parse_info(s).unwrap();
        assert_eq!(i.liste, "ok");
        assert_eq!(i.pakete.len(), 1, "ungültiges Manifest fällt weg");
        let p = &i.pakete[0];
        assert_eq!((p.zustand.as_str(), p.geladen, p.gesamt, p.da()), ("laedt", 4, 10, false));
        assert!(parse_info("kein json").is_none());
        assert!(parse_result("").is_none(), "läuft noch");
        let r = parse_result(r#"{"ok":true,"kandidaten":[{"text":"a=1","score":-0.2},{"text":"a=l","score":-0.9}],"ms":512}"#).unwrap().unwrap();
        assert_eq!(r.0.len(), 2);
        assert_eq!(r.0[0].text, "a=1");
        assert_eq!(r.1, 512);
        assert_eq!(parse_result(r#"{"ok":false,"fehler":"Modell nicht geladen"}"#).unwrap().unwrap_err(), "Modell nicht geladen");
        if !SUPPORTED {
            assert!(info().is_none());
            assert!(start("x", 3, &Ink { w: 1, h: 1, gray: vec![255] }).is_none());
        }
    }
}
