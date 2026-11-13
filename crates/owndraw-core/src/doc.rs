//! Dokument = eine `.owndraw`-Datei (ZIP-Container v2, siehe docs/konzept-features.md Abschnitt 3).
//!
//! WP0: Typen und Signaturen. WP1: Implementierung (lesen/schreiben, v1-Import, Bild-Deduplizierung).
//!
//! Bilder: Img-Items verweisen per Dateiname (`Img::file`) auf Bilder. Im Speicher hält [`ImageStore`] die Bytes;
//! zum Rendern werden sie mit [`ImageStore::materialize`] in den Bildordner des Kerns (`item::set_img_dir`) geschrieben.

use crate::pages::BoardMode;
use serde_json::Value;
use std::collections::BTreeMap;
use std::path::Path;

pub type Result<T> = std::result::Result<T, DocError>;

#[derive(Debug)]
pub enum DocError {
    Io(std::io::Error),
    /// Kein gültiges .owndraw/.xopp.
    Format(String),
}

impl std::fmt::Display for DocError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            DocError::Io(e) => write!(f, "E/A-Fehler: {e}"),
            DocError::Format(s) => write!(f, "Ungültige Datei: {s}"),
        }
    }
}
impl std::error::Error for DocError {}
impl From<std::io::Error> for DocError {
    fn from(e: std::io::Error) -> Self {
        DocError::Io(e)
    }
}

/// Aktuelle Formatversion.
pub const FORMAT_VERSION: i64 = 2;

/// Manifest-Daten (`manifest.json`).
#[derive(Clone, Debug, Default)]
pub struct Meta {
    /// UUID des Boards.
    pub id: String,
    pub name: String,
    /// RFC 3339.
    pub modified: String,
    /// Version, mit der die Datei gelesen wurde (2 für neue Dokumente).
    pub version: i64,
    /// true, wenn `version` > FORMAT_VERSION: nur lesen.
    pub read_only: bool,
    /// Unbekannte Manifest-Felder (werden beim Schreiben erhalten).
    pub extra: serde_json::Map<String, Value>,
}

/// Bilder des Dokuments: Dateiname (z. B. `<sha256>.png`) → Bytes.
#[derive(Clone, Debug, Default)]
pub struct ImageStore {
    pub files: BTreeMap<String, Vec<u8>>,
}

impl ImageStore {
    /// Fügt ein Bild inhaltsadressiert ein und liefert den Dateinamen `<sha256>.<ext>`.
    pub fn add(&mut self, _bytes: Vec<u8>, _ext: &str) -> String {
        todo!("WP1")
    }
    /// Schreibt alle Bilder in `dir` (falls noch nicht vorhanden).
    pub fn materialize(&self, _dir: &Path) -> std::io::Result<()> {
        todo!("WP1")
    }
}

/// Ein geöffnetes Board.
#[derive(Clone, Debug, Default)]
pub struct Document {
    pub meta: Meta,
    pub mode: BoardMode,
    /// Board-JSON wie `Board::to_json` (Ebenen, Items, View-Zustand); Items optional mit Feld "page".
    pub board: Value,
    pub images: ImageStore,
    /// Vorschau (PNG, max. 512 px), optional.
    pub thumb: Option<Vec<u8>>,
}

impl Document {
    /// Neues leeres Dokument mit frischer UUID.
    pub fn new(_name: &str, _mode: BoardMode) -> Document {
        todo!("WP1")
    }
}

/// Liest `.owndraw` v2 (und v1: `meta.json` + `board.json`).
pub fn read_owndraw(_bytes: &[u8]) -> Result<Document> {
    todo!("WP1")
}

/// Schreibt `.owndraw` v2.
pub fn write_owndraw(_doc: &Document) -> Result<Vec<u8>> {
    todo!("WP1")
}
