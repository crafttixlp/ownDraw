//! Dokument-Sitzung: dünne Brücke zwischen Oberfläche und `owndraw_core::doc`/`xopp` (Hooks für WP7).
//!
//! Die Oberfläche ruft Lesen/Schreiben/Neu/Kopie nur über diese Funktionen auf. WP1 (Format v2) und WP8 (Xournal++)
//! laufen parallel; solange sie noch `todo!()` enthalten, fangen die Funktionen den Panic ab und melden einen Fehler
//! statt die App zu beenden. WP7 darf die Rümpfe ersetzen (Hintergrund-Thread, Rechte-Fehler …), nicht die Signaturen.

use crate::platform::{FileKind, FileRef, Platform};
use owndraw_core::board::Board;
use owndraw_core::doc::{Document, ImageStore, Meta, FORMAT_VERSION};
use owndraw_core::pages::{BoardMode, Page, PageBg, PageFormat, PageSetup};
use std::panic::{catch_unwind, AssertUnwindSafe};
use std::path::Path;

/// Führt `f` aus und wandelt einen Panic (noch nicht implementierte Teile anderer Pakete) in einen Fehler um.
pub fn guard<T>(f: impl FnOnce() -> Result<T, String>) -> Result<T, String> {
    match catch_unwind(AssertUnwindSafe(f)) {
        Ok(r) => r,
        Err(_) => Err("Funktion ist noch nicht verfügbar".into()),
    }
}

/// Aktuelle Zeit als RFC 3339 (UTC), ohne zusätzliche Abhängigkeit.
pub fn rfc3339_now() -> String {
    let secs = owndraw_core::store::now_ms() / 1000;
    let days = secs.div_euclid(86400);
    let sod = secs.rem_euclid(86400);
    let z = days + 719468;
    let era = z.div_euclid(146097);
    let doe = z - era * 146097;
    let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = yoe + era * 400 + i64::from(m <= 2);
    format!("{y:04}-{m:02}-{d:02}T{:02}:{:02}:{:02}Z", sod / 3600, sod % 3600 / 60, sod % 60)
}

/// Parameter des Neu-Dialogs (Abschnitt 5 des Konzepts).
#[derive(Clone, Debug, PartialEq)]
pub struct NewParams {
    pub name: String,
    pub pages: bool,
    pub format: PageFormat,
    pub landscape: bool,
    pub bg: PageBg,
}

impl Default for NewParams {
    fn default() -> Self {
        NewParams { name: String::new(), pages: true, format: PageFormat::A4, landscape: false, bg: PageBg::Lined }
    }
}

impl NewParams {
    pub fn mode(&self) -> BoardMode {
        if !self.pages {
            return BoardMode::Infinite;
        }
        let (mut w, mut h) = self.format.size();
        if self.landscape {
            std::mem::swap(&mut w, &mut h);
        }
        BoardMode::Pages(PageSetup { w, h, bg: self.bg, pages: vec![Page { id: owndraw_core::store::random_hex(8), w, h, bg: self.bg, extra: serde_json::Value::Null }] })
    }
}

/// Neues leeres Dokument (WP1 liefert `Document::new`; bis dahin direkt über die Felder).
pub fn new_document(p: &NewParams) -> Document {
    let mode = p.mode();
    let mut board = Board::new(1.0);
    board.board_mode = mode.clone();
    Document {
        meta: Meta { id: owndraw_core::store::random_hex(32), name: p.name.clone(), modified: rfc3339_now(), version: FORMAT_VERSION, read_only: false, extra: Default::default() },
        mode,
        board: board.to_json(),
        images: ImageStore::default(),
        thumb: None,
    }
}

/// Liest eine `.owndraw` (oder `.xopp` per Import) vollständig.
pub fn open_document(platform: &dyn Platform, f: &FileRef) -> Result<Document, String> {
    let bytes = platform.read(f).map_err(|e| format!("Datei konnte nicht gelesen werden: {e}"))?;
    guard(|| {
        if FileKind::of_name(&f.display_name()) == FileKind::Xopp {
            owndraw_core::xopp::import_xopp(&bytes).map_err(|e| e.to_string())
        } else {
            owndraw_core::doc::read_owndraw(&bytes).map_err(|e| e.to_string())
        }
    })
}

/// Serialisiert das Dokument als `.owndraw` v2.
pub fn serialize(doc: &Document) -> Result<Vec<u8>, String> {
    guard(|| owndraw_core::doc::write_owndraw(doc).map_err(|e| e.to_string()))
}

/// Schreibt das Dokument atomar an `f`.
pub fn write_document(platform: &dyn Platform, f: &FileRef, doc: &Document) -> Result<(), String> {
    let bytes = serialize(doc)?;
    platform.write_atomic(f, &bytes).map_err(|e| format!("Speichern fehlgeschlagen: {e}"))
}

/// „Kopie speichern unter …“: gleiches Format, neue Board-ID; das geöffnete Dokument bleibt unverändert.
pub fn save_copy_as(platform: &dyn Platform, doc: &Document, f: &FileRef) -> Result<(), String> {
    let mut copy = doc.clone();
    copy.meta.id = owndraw_core::store::random_hex(32);
    copy.meta.modified = rfc3339_now();
    write_document(platform, f, &copy)
}

/// Exportiert das Dokument als Xournal++-Datei (`.xopp`).
pub fn export_xopp(platform: &dyn Platform, doc: &Document, f: &FileRef) -> Result<(), String> {
    let bytes = guard(|| owndraw_core::xopp::export_xopp(doc, Default::default()).map_err(|e| e.to_string()))?;
    platform.write_atomic(f, &bytes).map_err(|e| format!("Speichern fehlgeschlagen: {e}"))
}

/// Legt die Bilder des Dokuments im Bildordner `dir` ab (für das Rendern, `item::set_img_dir`).
pub fn prepare_images(doc: &Document, dir: &Path) -> Result<(), String> {
    std::fs::create_dir_all(dir).map_err(|e| e.to_string())?;
    // Vorrang: WP1-Implementierung; Rückfall: Dateien direkt schreiben.
    if guard(|| doc.images.materialize(dir).map_err(|e| e.to_string())).is_err() {
        for (name, bytes) in &doc.images.files {
            let p = dir.join(name);
            if !p.exists() {
                std::fs::write(&p, bytes).map_err(|e| e.to_string())?;
            }
        }
    }
    Ok(())
}

/// Bild importieren (png/jpg/gif/bmp/webp): auf höchstens 2048 px verkleinern, als PNG ins Dokument und in den
/// Bildordner legen. Liefert (Dateiname, Breite, Höhe).
pub fn import_image(images: &mut ImageStore, dir: &Path, bytes: &[u8]) -> Result<(String, u32, u32), String> {
    let img = image::load_from_memory(bytes).map_err(|e| format!("Bild konnte nicht geladen werden: {e}"))?;
    let (w, h) = (img.width(), img.height());
    if w == 0 || h == 0 {
        return Err("Leeres Bild".into());
    }
    let (mut ss, mx) = (1u32, w.max(h));
    while mx.div_ceil(ss) > 2048 {
        ss *= 2;
    }
    let img = if ss > 1 { img.resize_exact(w.div_ceil(ss), h.div_ceil(ss), image::imageops::FilterType::Triangle) } else { img };
    let rgba = img.into_rgba8();
    let (w, h) = (rgba.width(), rgba.height());
    let mut png = Vec::new();
    image::DynamicImage::ImageRgba8(rgba).write_to(&mut std::io::Cursor::new(&mut png), image::ImageFormat::Png).map_err(|e| e.to_string())?;
    let name = match guard(|| Ok(images.add(png.clone(), "png"))) {
        Ok(n) => n,
        Err(_) => {
            let n = format!("{}.png", owndraw_core::store::random_hex(16));
            images.files.insert(n.clone(), png.clone());
            n
        }
    };
    std::fs::create_dir_all(dir).map_err(|e| e.to_string())?;
    std::fs::write(dir.join(&name), &png).map_err(|e| e.to_string())?;
    Ok((name, w, h))
}

/// Vorschaubild (PNG) einer Datei für die Zuletzt-Kacheln. WP7 darf das durch ein gezieltes Lesen von
/// `thumb.png` aus dem ZIP ersetzen (hier wird die ganze Datei gelesen).
pub fn read_thumb(platform: &dyn Platform, f: &FileRef) -> Option<Vec<u8>> {
    if FileKind::of_name(&f.display_name()) != FileKind::Owndraw {
        return None;
    }
    open_document(platform, f).ok().and_then(|d| d.thumb)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn neu_parameter_und_dokument() {
        let p = NewParams { name: "T".into(), pages: true, format: PageFormat::A5, landscape: true, bg: PageBg::Grid };
        match p.mode() {
            BoardMode::Pages(s) => {
                assert_eq!((s.w, s.h), (595.0, 420.0));
                assert_eq!(s.pages.len(), 1);
                assert_eq!(s.pages[0].bg, PageBg::Grid);
            }
            BoardMode::Infinite => panic!("Seitenmodus erwartet"),
        }
        let d = new_document(&NewParams { name: "Frei".into(), pages: false, ..NewParams::default() });
        assert_eq!(d.mode, BoardMode::Infinite);
        assert_eq!(d.meta.name, "Frei");
        assert_eq!(d.meta.id.len(), 32);
        assert!(rfc3339_now().ends_with('Z'));
    }

    #[test]
    fn standard_ist_a4_hoch_liniert() {
        let p = NewParams::default();
        assert!(p.pages && !p.landscape && p.bg == PageBg::Lined && p.format == PageFormat::A4);
    }
}
