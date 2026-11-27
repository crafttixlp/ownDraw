//! Dokument-Sitzung: dünne Brücke zwischen Oberfläche und `owndraw_core::doc`/`xopp` (WP7).
//!
//! Die Oberfläche ruft Lesen/Schreiben/Neu/Kopie nur über diese Funktionen auf.

use crate::autosave::SaveFormat;
use crate::platform::{FileKind, FileRef, Platform};
use owndraw_core::doc::{Document, ImageStore};
use owndraw_core::pages::{BoardMode, Page, PageBg, PageFormat, PageSetup};
use std::io::{Read, Seek};
use std::path::Path;

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
    /// Textdokument (Textverarbeitung) statt Board.
    pub text: bool,
}

impl Default for NewParams {
    fn default() -> Self {
        NewParams { name: String::new(), pages: true, format: PageFormat::A4, landscape: false, bg: PageBg::Lined, text: false }
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

/// Neues leeres Dokument.
pub fn new_document(p: &NewParams) -> Document {
    if p.text {
        let mut td = owndraw_core::office::TextDoc::new();
        let (mut w, mut h) = match p.format {
            PageFormat::A5 => (419.53, 595.28),
            PageFormat::Letter => (612.0, 792.0),
            _ => (595.28, 841.89),
        };
        if p.landscape {
            std::mem::swap(&mut w, &mut h);
        }
        td.page.w = w;
        td.page.h = h;
        let mut d = Document::new_text(&p.name);
        d.text = Some(td.to_json());
        return d;
    }
    Document::new(&p.name, p.mode())
}

/// Liest eine `.owndraw` (oder `.xopp` per Import) vollständig.
pub fn open_document(platform: &dyn Platform, f: &FileRef) -> Result<Document, String> {
    let bytes = platform.read(f).map_err(|e| format!("Datei konnte nicht gelesen werden: {e}"))?;
    if FileKind::of_name(&f.display_name()) == FileKind::Xopp {
        owndraw_core::xopp::import_xopp(&bytes).map_err(|e| format!("Xournal++-Datei konnte nicht importiert werden: {e}"))
    } else {
        owndraw_core::doc::read_owndraw(&bytes).map_err(|e| format!("Board konnte nicht gelesen werden: {e}"))
    }
}

/// Serialisiert das Dokument als `.owndraw` v2.
pub fn serialize(doc: &Document) -> Result<Vec<u8>, String> {
    owndraw_core::doc::write_owndraw(doc).map_err(|e| e.to_string())
}

/// Serialisiert im gewünschten Zielformat (Autosave).
pub fn serialize_as(doc: &Document, fmt: SaveFormat) -> Result<Vec<u8>, String> {
    match fmt {
        SaveFormat::Owndraw => serialize(doc),
        SaveFormat::Xopp => owndraw_core::xopp::export_xopp(doc, Default::default()).map_err(|e| e.to_string()),
    }
}

/// Meldung für Dokumente aus einer neueren Programmversion.
pub const READ_ONLY_HINT: &str = "Dieses Board stammt aus einer neueren OwnDraw-Version und wird nicht überschrieben. Speichere eine Kopie, um es zu bearbeiten.";

/// Schreibt das Dokument atomar an `f`. Schreibgeschützte Dokumente (`meta.read_only`) werden nicht überschrieben.
pub fn write_document(platform: &dyn Platform, f: &FileRef, doc: &Document) -> Result<(), String> {
    if doc.meta.read_only {
        return Err(READ_ONLY_HINT.into());
    }
    let bytes = serialize(doc)?;
    platform.write_atomic(f, &bytes).map_err(|e| crate::autosave::describe_error(&e, &f.display_name()))
}

/// „Kopie speichern unter …“: gleiches Format, neue Board-ID; das geöffnete Dokument bleibt unverändert.
/// Die Kopie ist beschreibbar, auch wenn das Original schreibgeschützt ist.
pub fn save_copy_as(platform: &dyn Platform, doc: &Document, f: &FileRef) -> Result<(), String> {
    let mut copy = doc.clone();
    copy.meta.id = owndraw_core::doc::new_uuid();
    copy.meta.modified = rfc3339_now();
    copy.meta.read_only = false;
    copy.meta.version = owndraw_core::doc::FORMAT_VERSION;
    write_document(platform, f, &copy)
}

/// Exportiert das Dokument als Xournal++-Datei (`.xopp`).
pub fn export_xopp(platform: &dyn Platform, doc: &Document, f: &FileRef) -> Result<(), String> {
    let bytes = serialize_as(doc, SaveFormat::Xopp)?;
    platform.write_atomic(f, &bytes).map_err(|e| crate::autosave::describe_error(&e, &f.display_name()))
}

/// Legt die Bilder des Dokuments im Bildordner `dir` ab (für das Rendern, `item::set_img_dir`).
/// Im Browser (kein Dateisystem) stattdessen in den Bildspeicher des Kerns (`item::put_mem_image`).
pub fn prepare_images(doc: &Document, dir: &Path) -> Result<(), String> {
    if crate::platform::WEB {
        owndraw_core::item::clear_mem_images();
        for (n, b) in &doc.images.files {
            owndraw_core::item::put_mem_image(n, b.clone());
        }
        return Ok(());
    }
    doc.images.materialize(dir).map_err(|e| e.to_string())
}

/// Ein einzelnes Bild für das Rendern ablegen (Bildordner bzw. im Browser der Bildspeicher des Kerns).
pub fn put_image(dir: &Path, name: &str, bytes: &[u8]) -> Result<(), String> {
    if crate::platform::WEB {
        owndraw_core::item::put_mem_image(name, bytes.to_vec());
        return Ok(());
    }
    std::fs::create_dir_all(dir).map_err(|e| e.to_string())?;
    std::fs::write(dir.join(name), bytes).map_err(|e| e.to_string())
}

/// Bild importieren (png/jpg/gif/bmp/webp): auf höchstens 2048 px verkleinern, als PNG ins Dokument und in den
/// Bildordner legen. Liefert (Dateiname, Breite, Höhe).
pub fn import_image(images: &mut ImageStore, dir: &Path, bytes: &[u8]) -> Result<(String, u32, u32), String> {
    let img = image::load_from_memory(bytes).map_err(|e| format!("Bild konnte nicht geladen werden: {e}"))?;
    let (w, h) = (img.width(), img.height());
    if w == 0 || h == 0 {
        return Err("Das Bild ist leer".into());
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
    let name = images.add(png.clone(), "png");
    put_image(dir, &name, &png)?;
    Ok((name, w, h))
}

/// Liest einen einzelnen ZIP-Eintrag (höchstens `max` Bytes), ohne die übrigen zu dekodieren.
fn zip_entry<R: Read + Seek>(r: R, name: &str, max: u64) -> Option<Vec<u8>> {
    let mut z = zip::ZipArchive::new(r).ok()?;
    let e = z.by_name(name).ok()?;
    if e.size() > max {
        return None;
    }
    let mut v = Vec::new();
    e.take(max).read_to_end(&mut v).ok()?;
    Some(v)
}

/// Einzelnen Eintrag einer `.owndraw` lesen: Pfade direkt von der Platte (nur Verzeichnis + der eine Eintrag),
/// URIs über die Plattform.
fn read_zip_entry(platform: &dyn Platform, f: &FileRef, name: &str, max: u64) -> Option<Vec<u8>> {
    if FileKind::of_name(&f.display_name()) != FileKind::Owndraw {
        return None;
    }
    match f {
        FileRef::Path(p) => zip_entry(std::io::BufReader::new(std::fs::File::open(p).ok()?), name, max),
        uri => zip_entry(std::io::Cursor::new(platform.read(uri).ok()?), name, max),
    }
}

/// Vorschaubild (PNG) einer Datei für die Zuletzt-Kacheln: liest gezielt `thumb.png` aus dem ZIP.
pub fn read_thumb(platform: &dyn Platform, f: &FileRef) -> Option<Vec<u8>> {
    read_zip_entry(platform, f, "thumb.png", 8 << 20)
}

/// Name und Seitenmodus einer `.owndraw` aus dem Manifest (ohne die ganze Datei zu dekodieren).
#[cfg_attr(target_arch = "wasm32", allow(dead_code))]
pub fn read_summary(platform: &dyn Platform, f: &FileRef) -> Option<(String, bool)> {
    let m: serde_json::Value = serde_json::from_slice(&read_zip_entry(platform, f, "manifest.json", 4 << 20)?).ok()?;
    let name = m.get("name").and_then(|v| v.as_str()).filter(|s| !s.is_empty()).map(String::from).unwrap_or_else(|| f.display_name().trim_end_matches(".owndraw").to_string());
    Some((name, m.get("mode").and_then(|v| v.as_str()) == Some("pages") || m.get("kind").and_then(|v| v.as_str()) == Some("text")))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn neu_parameter_und_dokument() {
        let p = NewParams { name: "T".into(), pages: true, format: PageFormat::A5, landscape: true, bg: PageBg::Grid, text: false };
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
        assert!(!d.meta.id.is_empty());
        assert!(rfc3339_now().ends_with('Z'));
    }

    #[test]
    fn standard_ist_a4_hoch_liniert() {
        let p = NewParams::default();
        assert!(p.pages && !p.landscape && p.bg == PageBg::Lined && p.format == PageFormat::A4);
    }

    fn tmpdir(n: &str) -> std::path::PathBuf {
        let d = std::env::temp_dir().join(format!("owndraw-se-{n}-{}", owndraw_core::store::random_hex(6)));
        std::fs::create_dir_all(&d).unwrap();
        d
    }

    #[test]
    fn roundtrip_dokument_datei_dokument() {
        let d = tmpdir("rt");
        let pf = crate::platform::stub::StubPlatform::new();
        let f = FileRef::Path(d.join("t.owndraw"));
        let mut doc = new_document(&NewParams { name: "RT".into(), ..NewParams::default() });
        let png = vec![0x89, b'P', b'N', b'G', 1, 2, 3];
        let img = doc.images.add(png.clone(), "png");
        doc.board["items"] = serde_json::json!([{"t": "i", "f": img, "x": 0, "y": 0, "w": 10, "h": 10}]);
        doc.thumb = Some(b"\x89PNGthumb".to_vec());
        write_document(&pf, &f, &doc).unwrap();
        let back = open_document(&pf, &f).unwrap();
        assert_eq!(back.meta.id, doc.meta.id);
        assert_eq!(back.meta.name, "RT");
        assert_eq!(back.mode, doc.mode);
        assert_eq!(back.board, doc.board);
        assert_eq!(back.images.files.get(&img), Some(&png));
        assert_eq!(back.thumb, doc.thumb);
        // Bilder-Cache
        let cache = d.join("cache");
        prepare_images(&back, &cache).unwrap();
        assert_eq!(std::fs::read(cache.join(&img)).unwrap(), png);
        let _ = std::fs::remove_dir_all(d);
    }

    #[test]
    fn thumb_und_summary_gezielt() {
        let d = tmpdir("th");
        let pf = crate::platform::stub::StubPlatform::new();
        let f = FileRef::Path(d.join("t.owndraw"));
        let mut doc = new_document(&NewParams { name: "Mit Vorschau".into(), pages: true, ..NewParams::default() });
        doc.thumb = Some(b"PNGDATA".to_vec());
        write_document(&pf, &f, &doc).unwrap();
        assert_eq!(read_thumb(&pf, &f).unwrap(), b"PNGDATA");
        assert_eq!(read_summary(&pf, &f).unwrap(), ("Mit Vorschau".to_string(), true));
        // ohne Vorschau, falsche Endung, kaputte Datei
        doc.thumb = None;
        write_document(&pf, &f, &doc).unwrap();
        assert!(read_thumb(&pf, &f).is_none());
        let kaputt = FileRef::Path(d.join("k.owndraw"));
        std::fs::write(d.join("k.owndraw"), b"kein zip").unwrap();
        assert!(read_thumb(&pf, &kaputt).is_none());
        assert!(read_thumb(&pf, &FileRef::Path(d.join("x.xopp"))).is_none());
        let _ = std::fs::remove_dir_all(d);
    }

    #[test]
    fn read_only_wird_nicht_ueberschrieben_kopie_schon() {
        let d = tmpdir("ro");
        let pf = crate::platform::stub::StubPlatform::new();
        let f = FileRef::Path(d.join("r.owndraw"));
        let mut doc = new_document(&NewParams::default());
        doc.meta.read_only = true;
        assert!(write_document(&pf, &f, &doc).is_err());
        assert!(!pf.exists(&f));
        let g = FileRef::Path(d.join("kopie.owndraw"));
        save_copy_as(&pf, &doc, &g).unwrap();
        let c = open_document(&pf, &g).unwrap();
        assert_ne!(c.meta.id, doc.meta.id);
        assert!(!c.meta.read_only);
        let _ = std::fs::remove_dir_all(d);
    }

    #[test]
    fn xopp_export_import() {
        let d = tmpdir("xo");
        let pf = crate::platform::stub::StubPlatform::new();
        let f = FileRef::Path(d.join("t.xopp"));
        let doc = new_document(&NewParams { name: "X".into(), ..NewParams::default() });
        export_xopp(&pf, &doc, &f).unwrap();
        let back = open_document(&pf, &f).unwrap();
        assert!(matches!(back.mode, BoardMode::Pages(_)));
        let _ = std::fs::remove_dir_all(d);
    }
}
