//! Vektorformate aus [`VOp`]-Listen: SVG, EPS und EMF.

use crate::canvas::VOp;

/// SVG-Dokument der Größe w × h (Einheit px, viewBox 0 0 w h).
pub fn svg(ops: &[VOp], w: f32, h: f32) -> String {
    let _ = (ops, w, h);
    String::new()
}

/// Encapsulated PostScript (EPSF-3.0), BoundingBox 0 0 w h in Punkt.
pub fn eps(ops: &[VOp], w: f32, h: f32) -> String {
    let _ = (ops, w, h);
    String::new()
}

/// Windows Enhanced Metafile (EMF), Größe w × h in Ausgabeeinheiten (96 dpi).
pub fn emf(ops: &[VOp], w: f32, h: f32) -> Vec<u8> {
    let _ = (ops, w, h);
    Vec::new()
}
