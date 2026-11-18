//! Vom Admin gesperrte Werkzeuge und Funktionen (Katalog `owndraw_core::features`, Server `GET /api/features`).
//! Die App lädt die Liste beim Start und nach An-/Abmelden (`net::refresh_features`); für Admins liefert der
//! Server immer eine leere Liste. Ohne Server (Android, Desktop ohne Anbindung) ist nichts gesperrt.
//!
//! Gesperrte Werkzeuge verschwinden aus Leiste, „Weitere Werkzeuge“, Einstellungen und Tastenkürzeln
//! (`ui/editor/tools.rs`); gesperrte Funktionen aus Menü und Knöpfen.

use std::cell::RefCell;

thread_local! {
    static OFF: RefCell<Vec<String>> = const { RefCell::new(Vec::new()) };
}

/// Neue Liste gesperrter Kennungen übernehmen.
#[cfg_attr(target_os = "android", allow(dead_code))]
pub fn set(off: Vec<String>) {
    OFF.with(|o| *o.borrow_mut() = off);
}

/// Ist das Werkzeug bzw. die Funktion gesperrt?
pub fn locked(id: &str) -> bool {
    OFF.with(|o| o.borrow().iter().any(|x| x == id))
}

/// Anzahl gesperrter Werkzeuge (für den Hinweis in den Einstellungen).
pub fn locked_tools() -> usize {
    OFF.with(|o| o.borrow().iter().filter(|x| owndraw_core::features::get(x).is_some_and(|f| f.kind == owndraw_core::features::Kind::Tool)).count())
}
