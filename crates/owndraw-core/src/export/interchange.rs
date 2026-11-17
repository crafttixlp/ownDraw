//! Austauschformate aus den Board-Objekten: Excalidraw, draw.io, InkML, JSON und eine einzelne HTML-Datei.

use crate::board::{Board, ExportFrame};
use crate::item::Item;
use std::rc::Rc;

/// Excalidraw-Szene (.excalidraw, JSON). Koordinaten relativ zum Ausschnitt.
pub fn excalidraw(b: &Board, items: &[Rc<Item>], f: &ExportFrame) -> String {
    let _ = (b, items, f);
    String::new()
}

/// draw.io / diagrams.net (.drawio, unkomprimiertes mxfile-XML).
pub fn drawio(b: &Board, items: &[Rc<Item>], f: &ExportFrame, title: &str) -> String {
    let _ = (b, items, f, title);
    String::new()
}

/// InkML (W3C) mit allen Freihand-Strichen (Punkte, Farbe, Breite).
pub fn inkml(b: &Board, items: &[Rc<Item>], f: &ExportFrame) -> String {
    let _ = (b, items, f);
    String::new()
}

/// Rohdaten als JSON (Board-Objekte im .owndraw-Itemformat plus Ausschnitt, Ebenen und Seiten).
pub fn json(b: &Board, items: &[Rc<Item>], f: &ExportFrame, title: &str) -> String {
    let _ = (b, items, f, title);
    String::new()
}

/// Einzelne HTML-Datei: Website-Export (Ebenen, Animationen) mit eingebetteten Bildern (data-URIs), ohne ZIP.
pub fn html_single(b: &Board, title: &str) -> Option<String> {
    let _ = (b, title);
    None
}
