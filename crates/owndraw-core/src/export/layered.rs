//! Formate mit Ebenen: OpenRaster (.ora, Krita/GIMP) und Photoshop (.psd).

use tiny_skia::Pixmap;

/// Eine Ebene (alle gleich groß wie `merged`), von hinten nach vorne.
pub struct LayerImg {
    pub name: String,
    pub pm: Pixmap,
    pub opacity: f32,
    pub visible: bool,
}

pub fn ora(layers: &[LayerImg], merged: &Pixmap) -> Result<Vec<u8>, String> {
    let _ = (layers, merged);
    Err("noch nicht umgesetzt".into())
}

pub fn psd(layers: &[LayerImg], merged: &Pixmap) -> Result<Vec<u8>, String> {
    let _ = (layers, merged);
    Err("noch nicht umgesetzt".into())
}
