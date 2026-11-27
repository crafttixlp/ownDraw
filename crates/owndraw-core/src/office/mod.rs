//! Textverarbeitung (OwnDraw Office): Dokumentmodell, Schriften, Satz mit Seiten, Zeichnen/PDF und Word-Dateien.
//! Ohne Oberflächen-Abhängigkeit; der Editor liegt in `crates/owndraw/src/ui/writer`.
//!
//! Ein Textdokument ist eine `.owndraw`-Datei mit `"kind": "text"` im Manifest und dem Inhalt in `text.json`
//! (siehe [`crate::doc`]). So funktionieren Speicherorte, Server, Teilen und „Zuletzt“ wie bei Boards.

pub mod docx;
pub mod fonts;
pub mod layout;
pub mod model;
pub mod render;

pub use layout::{layout, Layout};
pub use model::{Align, CStyle, Kind, PageSpec, Para, Pos, Run, TextDoc, VAlign};
