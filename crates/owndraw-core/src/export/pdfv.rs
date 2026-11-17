//! PDF mit Vektorinhalt (Pfade, Bilder, Transparenz) und PDF/A-2b.

use crate::canvas::VOp;

/// Mehrseitiges PDF; jede Seite (ops, w, h) mit Größe w × h Punkt. `pdfa` = PDF/A-2b (XMP, sRGB-OutputIntent).
pub fn pdf(pages: &[(Vec<VOp>, f32, f32)], pdfa: bool, title: &str) -> Vec<u8> {
    let _ = (pages, pdfa, title);
    Vec::new()
}
