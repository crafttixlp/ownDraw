//! Kleine Ablage für Konfigurationsdateien (`settings.json`, `recent.json`).
//!
//! Desktop/Android: Datei unter dem Pfad (Ordner anlegen, atomar schreiben). Browser: `localStorage`, Schlüssel =
//! `owndraw:` + Pfad; Daten, die kein UTF-8 sind, als `b64:` + Base64.

use std::io;
use std::path::Path;

#[cfg(not(target_arch = "wasm32"))]
pub fn read(path: &Path) -> io::Result<Vec<u8>> {
    std::fs::read(path)
}

#[cfg(not(target_arch = "wasm32"))]
pub fn write(path: &Path, data: &[u8]) -> io::Result<()> {
    if let Some(d) = path.parent() {
        let _ = std::fs::create_dir_all(d);
    }
    owndraw_core::store::write_atomic(path, data)
}

#[cfg(target_arch = "wasm32")]
fn storage() -> io::Result<web_sys::Storage> {
    web_sys::window()
        .and_then(|w| w.local_storage().ok().flatten())
        .ok_or_else(|| io::Error::new(io::ErrorKind::Unsupported, "localStorage nicht verfügbar"))
}

#[cfg(target_arch = "wasm32")]
fn key(path: &Path) -> String {
    format!("owndraw:{}", path.display())
}

#[cfg(target_arch = "wasm32")]
pub fn read(path: &Path) -> io::Result<Vec<u8>> {
    let v = storage()?.get_item(&key(path)).ok().flatten().ok_or_else(|| io::Error::from(io::ErrorKind::NotFound))?;
    match v.strip_prefix("b64:") {
        Some(b) => crate::ui::editor::clip::b64_decode(b).ok_or_else(|| io::Error::from(io::ErrorKind::InvalidData)),
        None => Ok(v.into_bytes()),
    }
}

#[cfg(target_arch = "wasm32")]
pub fn write(path: &Path, data: &[u8]) -> io::Result<()> {
    let v = match std::str::from_utf8(data) {
        Ok(s) if !s.starts_with("b64:") => s.to_string(),
        _ => format!("b64:{}", crate::ui::editor::clip::b64_encode(data)),
    };
    storage()?.set_item(&key(path), &v).map_err(|_| io::Error::new(io::ErrorKind::StorageFull, "Browser-Speicher (localStorage) voll"))
}

#[cfg(target_arch = "wasm32")]
pub fn exists(path: &Path) -> bool {
    storage().ok().and_then(|s| s.get_item(&key(path)).ok().flatten()).is_some()
}
