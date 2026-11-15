//! OwnDraw im Browser: dünne Hülle um `owndraw::run_web` (siehe `index.html`, `build-web.sh`, `docs/web.md`).
//!
//! JavaScript: `import init, { start } from "./owndraw_web.js"; await init(); await start("owndraw-canvas");`
#![cfg(target_arch = "wasm32")]

use wasm_bindgen::prelude::*;
use wasm_bindgen::JsCast;

/// Startet die App im Canvas mit der id `canvas_id`. Das Promise endet, sobald die App läuft
/// (Fehler beim Start: Promise wird mit einer Meldung abgelehnt).
#[wasm_bindgen]
pub async fn start(canvas_id: String) -> Result<(), JsValue> {
    eframe::WebLogger::init(log::LevelFilter::Warn).ok();
    let canvas = web_sys::window()
        .and_then(|w| w.document())
        .and_then(|d| d.get_element_by_id(&canvas_id))
        .ok_or_else(|| JsValue::from_str(&format!("Canvas „{canvas_id}“ nicht gefunden")))?
        .dyn_into::<web_sys::HtmlCanvasElement>()
        .map_err(|_| JsValue::from_str("Element ist kein <canvas>"))?;
    owndraw::run_web(canvas).await
}

/// Programmversion (für die Ladeseite).
#[wasm_bindgen]
pub fn version() -> String {
    env!("CARGO_PKG_VERSION").to_string()
}
