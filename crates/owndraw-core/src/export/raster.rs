//! Rasterformate außer PNG: JPEG, WebP, AVIF, TIFF, BMP, GIF sowie animiertes GIF und APNG.

use tiny_skia::Pixmap;

/// Zielformat für [`encode`].
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Raster {
    /// Qualität 1..100.
    Jpeg { quality: u8 },
    Webp,
    Avif,
    Tiff,
    Bmp,
    Gif,
}

/// Kodiert das (vormultiplizierte) Bild. Formate ohne Transparenz (JPEG, BMP) werden auf `bg` (0xAARRGGBB, deckend) gelegt.
pub fn encode(pm: &Pixmap, kind: Raster, bg: u32) -> Result<Vec<u8>, String> {
    let _ = (pm, kind, bg);
    Err("noch nicht umgesetzt".into())
}

/// Animiertes GIF (Endlosschleife), jedes Bild `delay_ms` lang; alle Bilder gleich groß.
pub fn gif_anim(frames: &[Pixmap], delay_ms: u32) -> Result<Vec<u8>, String> {
    let _ = (frames, delay_ms);
    Err("noch nicht umgesetzt".into())
}

/// Animiertes PNG (APNG, Endlosschleife), jedes Bild `delay_ms` lang; alle Bilder gleich groß.
pub fn apng(frames: &[Pixmap], delay_ms: u32) -> Result<Vec<u8>, String> {
    let _ = (frames, delay_ms);
    Err("noch nicht umgesetzt".into())
}
