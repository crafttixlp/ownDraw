//! Xournal++ (`.xopp`, gzip-XML) Import/Export (WP8), Abbildung siehe docs/konzept-features.md Abschnitt 4.

use crate::doc::{Document, Result};

/// Export-Optionen.
#[derive(Clone, Copy, Debug, Default)]
pub struct XoppExport {
    /// Unendliches Board: in A4-Seiten kacheln statt einer großen Seite.
    pub tile_a4: bool,
}

pub fn import_xopp(_bytes: &[u8]) -> Result<Document> {
    todo!("WP8")
}

pub fn export_xopp(_doc: &Document, _opt: XoppExport) -> Result<Vec<u8>> {
    todo!("WP8")
}
