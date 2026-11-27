//! Dokument = eine `.owndraw`-Datei (ZIP-Container v2, siehe docs/konzept-features.md Abschnitt 3).
//!
//! WP0: Typen und Signaturen. WP1: Implementierung (lesen/schreiben, v1-Import, Bild-Deduplizierung).
//!
//! Bilder: Img-Items verweisen per Dateiname (`Img::file`) auf Bilder. Im Speicher hält [`ImageStore`] die Bytes;
//! zum Rendern werden sie mit [`ImageStore::materialize`] in den Bildordner des Kerns (`item::set_img_dir`) geschrieben.

use crate::pages::BoardMode;
use serde_json::Value;
use std::collections::BTreeMap;
use std::io::{Cursor, Read, Write};
use std::path::Path;

use crate::pages::{Page, PageBg, PageSetup};
use serde_json::{json, Map};
use sha2::{Digest, Sha256};
use zip::write::SimpleFileOptions;

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
/// Formatversion von Textdokumenten (`"kind": "text"`, Inhalt in `text.json`). Ältere Programmversionen öffnen sie
/// deshalb nur lesend und überschreiben den Text nicht.
pub const TEXT_FORMAT_VERSION: i64 = 3;

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
    pub fn add(&mut self, bytes: Vec<u8>, ext: &str) -> String {
        let name = format!("{}.{}", sha256_hex(&bytes), clean_ext(ext));
        self.files.entry(name.clone()).or_insert(bytes);
        name
    }
    /// Schreibt alle Bilder in `dir` (falls noch nicht vorhanden).
    pub fn materialize(&self, dir: &Path) -> std::io::Result<()> {
        std::fs::create_dir_all(dir)?;
        for (n, b) in &self.files {
            if !safe_name(n) {
                continue;
            }
            let p = dir.join(n);
            if !p.exists() {
                crate::store::write_atomic(&p, b)?;
            }
        }
        Ok(())
    }
}

fn sha256_hex(b: &[u8]) -> String {
    Sha256::digest(b).iter().map(|x| format!("{x:02x}")).collect()
}

fn clean_ext(e: &str) -> String {
    let e: String = e.trim_start_matches('.').to_ascii_lowercase().chars().filter(char::is_ascii_alphanumeric).take(5).collect();
    if e.is_empty() {
        "png".into()
    } else {
        e
    }
}

fn safe_name(n: &str) -> bool {
    !n.is_empty() && n.len() < 200 && !n.contains('/') && !n.contains('\\') && !n.contains("..") && !n.contains('\0') && !n.starts_with('.')
}

/// Ein Board-JSON mit einer Ebene (wie `Board::new(1.0).to_json()`).
fn empty_board_json() -> Value {
    crate::board::Board::new(1.0).to_json()
}

/// Zeitstempel (ms seit Epoche) als RFC 3339 (UTC).
pub fn rfc3339(ms: i64) -> String {
    let secs = ms.div_euclid(1000);
    let (days, rem) = (secs.div_euclid(86400), secs.rem_euclid(86400));
    // Zivildatum aus Tageszahl (Howard Hinnant)
    let z = days + 719468;
    let era = z.div_euclid(146097);
    let doe = z.rem_euclid(146097);
    let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = yoe + era * 400 + i64::from(m <= 2);
    format!("{y:04}-{m:02}-{d:02}T{:02}:{:02}:{:02}Z", rem / 3600, rem % 3600 / 60, rem % 60)
}

/// UUID v4 (Zufall aus `store::random_hex`).
pub fn new_uuid() -> String {
    let mut h: Vec<u8> = crate::store::random_hex(32).into_bytes();
    h[12] = b'4';
    h[16] = b"89ab"[(h[16] % 4) as usize];
    let s = String::from_utf8(h).unwrap_or_default();
    format!("{}-{}-{}-{}-{}", &s[0..8], &s[8..12], &s[12..16], &s[16..20], &s[20..32])
}

/// Ein Board (Image-Items: Feld "f") – Iterator über veränderbare Img-Items.
fn img_items(board: &mut Value) -> impl Iterator<Item = &mut Value> {
    board
        .get_mut("items")
        .and_then(Value::as_array_mut)
        .into_iter()
        .flatten()
        .filter(|o| o.get("t").and_then(Value::as_str) == Some("i"))
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
    /// Textdokument (Textverarbeitung): Inhalt von `text.json`; `None` = Board.
    pub text: Option<Value>,
}

impl Document {
    /// Neues leeres Dokument mit frischer UUID.
    pub fn new(name: &str, mode: BoardMode) -> Document {
        Document {
            meta: Meta {
                id: new_uuid(),
                name: name.to_string(),
                modified: rfc3339(crate::store::now_ms()),
                version: FORMAT_VERSION,
                read_only: false,
                extra: Map::new(),
            },
            mode,
            board: empty_board_json(),
            images: ImageStore::default(),
            thumb: None,
            text: None,
        }
    }

    /// Neues leeres Textdokument.
    pub fn new_text(name: &str) -> Document {
        let mut d = Document::new(name, BoardMode::Infinite);
        d.text = Some(crate::office::TextDoc::new().to_json());
        d
    }

    /// Ist das ein Textdokument?
    pub fn is_text(&self) -> bool {
        self.text.is_some()
    }
}

const MAX_TOTAL: u64 = 400 * 1024 * 1024;
const MAX_SMALL: u64 = 64 * 1024 * 1024;

fn read_limited<R: Read>(r: R, max: u64) -> Result<Vec<u8>> {
    let mut v = Vec::new();
    r.take(max + 1).read_to_end(&mut v)?;
    if v.len() as u64 > max {
        return Err(DocError::Format("Eintrag zu groß".into()));
    }
    Ok(v)
}

fn fmt_err<E: std::fmt::Display>(what: &str) -> impl FnOnce(E) -> DocError + '_ {
    move |e| DocError::Format(format!("{what}: {e}"))
}

fn num(v: Option<&Value>, d: f32) -> f32 {
    v.and_then(Value::as_f64).map(|x| x as f32).filter(|x| *x > 0.0).unwrap_or(d)
}

/// Board-Modus aus dem Manifest (Felder `mode`, `page`, `pages`; auch für [`mode_to_json`]).
pub fn mode_from_manifest(m: &Value) -> BoardMode {
    if m.get("mode").and_then(Value::as_str) != Some("pages") {
        return BoardMode::Infinite;
    }
    let def = PageSetup::default();
    let pg = m.get("page").filter(|v| v.is_object());
    let (w, h) = (num(pg.and_then(|p| p.get("w")), def.w), num(pg.and_then(|p| p.get("h")), def.h));
    let bg = pg.and_then(|p| p.get("bg")).and_then(Value::as_str).map(PageBg::from_name).unwrap_or(def.bg);
    let mut pages: Vec<Page> = m
        .get("pages")
        .and_then(Value::as_array)
        .map(|a| {
            a.iter()
                .filter(|p| p.is_object())
                .map(|p| Page {
                    id: p.get("id").and_then(Value::as_str).map(String::from).unwrap_or_else(|| crate::store::random_hex(8)),
                    w: num(p.get("w"), w),
                    h: num(p.get("h"), h),
                    bg: p.get("bg").and_then(Value::as_str).map(PageBg::from_name).unwrap_or(bg),
                    extra: p.get("extra").cloned().unwrap_or(Value::Null),
                })
                .collect()
        })
        .unwrap_or_default();
    if pages.is_empty() {
        pages.push(Page { id: crate::store::random_hex(8), w, h, bg, extra: Value::Null });
    }
    BoardMode::Pages(PageSetup { w, h, bg, pages })
}

/// Liest `.owndraw` v2 (und v1: `meta.json` + `board.json`).
pub fn read_owndraw(bytes: &[u8]) -> Result<Document> {
    let mut z = zip::ZipArchive::new(Cursor::new(bytes)).map_err(fmt_err("kein ZIP"))?;
    let (mut manifest, mut meta1, mut board, mut thumb, mut text) = (None, None, None, None, None);
    let mut images = ImageStore::default();
    let mut total = 0u64;
    for i in 0..z.len() {
        let mut e = z.by_index(i).map_err(fmt_err("ZIP-Eintrag"))?;
        let n = e.name().map_err(fmt_err("ZIP-Name"))?.to_string();
        if n.starts_with('/') || n.contains('\\') || n.contains("..") || n.contains('\0') {
            return Err(DocError::Format(format!("unsicherer Name {n}")));
        }
        if e.is_dir() {
            continue;
        }
        let img = n.strip_prefix("img/").map(String::from);
        if img.is_none() && !matches!(n.as_str(), "manifest.json" | "meta.json" | "board.json" | "thumb.png" | "text.json") {
            continue;
        }
        let d = read_limited(&mut e, if img.is_some() { MAX_TOTAL - total.min(MAX_TOTAL) } else { MAX_SMALL })?;
        total += d.len() as u64;
        if total > MAX_TOTAL {
            return Err(DocError::Format("Datei zu groß".into()));
        }
        match (img, n.as_str()) {
            (Some(f), _) => {
                if safe_name(&f) {
                    images.files.insert(f, d);
                }
            }
            (_, "manifest.json") => manifest = Some(d),
            (_, "meta.json") => meta1 = Some(d),
            (_, "board.json") => board = Some(d),
            (_, "text.json") => text = Some(d),
            _ => thumb = Some(d),
        }
    }
    let board: Value = serde_json::from_slice(&board.ok_or_else(|| DocError::Format("board.json fehlt".into()))?).map_err(fmt_err("board.json"))?;
    if !board.is_object() {
        return Err(DocError::Format("board.json ist kein Objekt".into()));
    }
    let now = rfc3339(crate::store::now_ms());
    let (meta, mode) = if let Some(mb) = manifest {
        let m: Value = serde_json::from_slice(&mb).map_err(fmt_err("manifest.json"))?;
        let obj = m.as_object().ok_or_else(|| DocError::Format("manifest.json ist kein Objekt".into()))?;
        if obj.get("format").and_then(Value::as_str).is_some_and(|f| f != "owndraw") {
            return Err(DocError::Format("kein OwnDraw-Manifest".into()));
        }
        let version = obj.get("version").and_then(Value::as_i64).unwrap_or(FORMAT_VERSION);
        let is_text = obj.get("kind").and_then(Value::as_str) == Some("text");
        let extra: Map<String, Value> =
            obj.iter().filter(|(k, _)| !matches!(k.as_str(), "format" | "version" | "id" | "name" | "mode" | "page" | "pages" | "modified" | "kind")).map(|(k, v)| (k.clone(), v.clone())).collect();
        let meta = Meta {
            id: obj.get("id").and_then(Value::as_str).filter(|s| !s.is_empty()).map(String::from).unwrap_or_else(new_uuid),
            name: obj.get("name").and_then(Value::as_str).unwrap_or("Board").to_string(),
            modified: obj.get("modified").and_then(Value::as_str).map(String::from).unwrap_or(now),
            version,
            read_only: version > if is_text { TEXT_FORMAT_VERSION } else { FORMAT_VERSION },
            extra,
        };
        (meta, mode_from_manifest(&m))
    } else {
        // v1: meta.json {"name","format":1,"app"}
        let m: Value = meta1.and_then(|b| serde_json::from_slice(&b).ok()).unwrap_or(Value::Null);
        let name = m.get("name").and_then(Value::as_str).map(str::trim).filter(|s| !s.is_empty()).unwrap_or("Importiertes Board");
        (Meta { id: new_uuid(), name: name.chars().take(80).collect(), modified: now, version: 1, read_only: false, extra: Map::new() }, BoardMode::Infinite)
    };
    let text = match text {
        Some(t) => Some(serde_json::from_slice::<Value>(&t).map_err(fmt_err("text.json"))?).filter(Value::is_object),
        None => None,
    };
    Ok(Document { meta, mode, board, images, thumb, text })
}

fn page_json(p: &Page) -> Value {
    let mut o = json!({"id": p.id, "w": p.w as f64, "h": p.h as f64, "bg": p.bg.name()});
    if !p.extra.is_null() {
        o["extra"] = p.extra.clone();
    }
    o
}

/// Board-Modus als Manifest-Ausschnitt `{"mode", "page", "pages"}` (Gegenstück: [`mode_from_manifest`]).
pub fn mode_to_json(mode: &BoardMode) -> Value {
    match mode {
        BoardMode::Infinite => json!({"mode": "infinite", "page": null, "pages": null}),
        BoardMode::Pages(s) => json!({
            "mode": "pages",
            "page": {"w": s.w as f64, "h": s.h as f64, "bg": s.bg.name()},
            "pages": Value::Array(s.pages.iter().map(page_json).collect()),
        }),
    }
}

/// Schreibt `.owndraw` v2.
pub fn write_owndraw(doc: &Document) -> Result<Vec<u8>> {
    // Bilder inhaltsadressiert umbenennen; nur referenzierte Bilder behalten.
    let mut board = doc.board.clone();
    let mut out = ImageStore::default();
    for it in img_items(&mut board) {
        let f = it.get("f").and_then(Value::as_str).unwrap_or("").to_string();
        if let Some(b) = doc.images.files.get(&f) {
            let ext = f.rsplit_once('.').map(|(_, e)| e).unwrap_or("png");
            let n = out.add(b.clone(), ext);
            it["f"] = json!(n);
        }
    }
    let mut m = doc.meta.extra.clone();
    m.insert("format".into(), json!("owndraw"));
    m.insert("version".into(), json!(if doc.text.is_some() { TEXT_FORMAT_VERSION } else { FORMAT_VERSION }));
    if doc.text.is_some() {
        m.insert("kind".into(), json!("text"));
    }
    m.entry("app").or_insert_with(|| json!(env!("CARGO_PKG_VERSION")));
    m.insert("id".into(), json!(doc.meta.id));
    m.insert("name".into(), json!(doc.meta.name));
    m.insert("modified".into(), json!(rfc3339(crate::store::now_ms())));
    if let Value::Object(mj) = mode_to_json(&doc.mode) {
        m.extend(mj);
    }
    let mut z = zip::ZipWriter::new(Cursor::new(Vec::new()));
    let opt = SimpleFileOptions::default().compression_method(zip::CompressionMethod::Deflated);
    let st = SimpleFileOptions::default().compression_method(zip::CompressionMethod::Stored);
    z.start_file("manifest.json", opt).map_err(fmt_err("ZIP schreiben"))?;
    z.write_all(Value::Object(m).to_string().as_bytes())?;
    z.start_file("board.json", opt).map_err(fmt_err("ZIP schreiben"))?;
    z.write_all(board.to_string().as_bytes())?;
    if let Some(t) = &doc.text {
        z.start_file("text.json", opt).map_err(fmt_err("ZIP schreiben"))?;
        z.write_all(t.to_string().as_bytes())?;
    }
    if let Some(t) = &doc.thumb {
        z.start_file("thumb.png", st).map_err(fmt_err("ZIP schreiben"))?;
        z.write_all(t)?;
    }
    for (n, b) in &out.files {
        z.start_file(format!("img/{n}"), st).map_err(fmt_err("ZIP schreiben"))?;
        z.write_all(b)?;
    }
    Ok(z.finish().map_err(fmt_err("ZIP schreiben"))?.into_inner())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn png(c: u8) -> Vec<u8> {
        let mut v = Vec::new();
        let mut im = image::RgbaImage::new(2, 2);
        for p in im.pixels_mut() {
            *p = image::Rgba([c, 0, 0, 255]);
        }
        image::DynamicImage::ImageRgba8(im).write_to(&mut Cursor::new(&mut v), image::ImageFormat::Png).unwrap();
        v
    }

    fn sample(mode: BoardMode) -> Document {
        let mut d = Document::new("Test", mode);
        let (p1, p2) = (png(10), png(200));
        d.images.files.insert("a.png".into(), p1.clone());
        d.images.files.insert("b.png".into(), p2);
        d.images.files.insert("a2.png".into(), p1); // gleicher Inhalt wie a.png
        d.images.files.insert("unused.png".into(), png(99));
        d.board["items"] = json!([{"t":"i","f":"a.png"},{"t":"i","f":"b.png"},{"t":"i","f":"a2.png"},{"t":"s","x":1}]);
        d.thumb = Some(png(1));
        d
    }

    fn check(d: &Document) -> Document {
        let bytes = write_owndraw(d).unwrap();
        let z = zip::ZipArchive::new(Cursor::new(&bytes)).unwrap();
        let imgs = z.file_names().filter(|n| n.as_ref().is_ok_and(|n| n.starts_with("img/"))).count();
        assert_eq!(imgs, 2);
        let r = read_owndraw(&bytes).unwrap();
        assert_eq!(r.mode, d.mode);
        assert_eq!(r.meta.id, d.meta.id);
        assert_eq!(r.thumb, d.thumb);
        assert_eq!(r.images.files.len(), 2);
        let items = r.board["items"].as_array().unwrap();
        assert_eq!(items[0]["f"], items[2]["f"]);
        assert_ne!(items[0]["f"], items[1]["f"]);
        for (i, it) in items.iter().enumerate().take(3) {
            let f = it["f"].as_str().unwrap();
            let orig = d.board["items"][i]["f"].as_str().unwrap();
            assert_eq!(r.images.files[f], d.images.files[orig]);
        }
        assert_eq!(items[3], d.board["items"][3]);
        assert_eq!(r.board["layers"], d.board["layers"]);
        r
    }

    #[test]
    fn roundtrip_unendlich() {
        check(&sample(BoardMode::Infinite));
    }

    #[test]
    fn roundtrip_seiten() {
        let s = PageSetup { pages: vec![
            Page { id: "p1".into(), w: 595.0, h: 842.0, bg: PageBg::Grid, extra: Value::Null },
            Page { id: "p2".into(), w: 420.0, h: 595.0, bg: PageBg::Dotted, extra: json!({"pdf": "x"}) },
        ], ..PageSetup::default() };
        let d = sample(BoardMode::Pages(s));
        check(&d);
    }

    #[test]
    fn unbekanntes_feld_und_version() {
        let mut d = sample(BoardMode::Infinite);
        d.meta.extra.insert("zukunft".into(), json!({"a": 1}));
        let r = read_owndraw(&write_owndraw(&d).unwrap()).unwrap();
        assert_eq!(r.meta.extra["zukunft"], json!({"a": 1}));
        assert!(!r.meta.read_only);
        // version 3
        let mut d3 = d.clone();
        d3.meta.extra.insert("version".into(), json!(3));
        let bytes = write_owndraw(&d3).unwrap();
        // version ist kein Extra-Feld: manifest von Hand auf 3 setzen
        let mut z = zip::ZipArchive::new(Cursor::new(&bytes)).unwrap();
        let mut w = zip::ZipWriter::new(Cursor::new(Vec::new()));
        for i in 0..z.len() {
            let mut e = z.by_index(i).unwrap();
            let n = e.name().unwrap().to_string();
            let mut b = Vec::new();
            e.read_to_end(&mut b).unwrap();
            if n == "manifest.json" {
                let mut m: Value = serde_json::from_slice(&b).unwrap();
                m["version"] = json!(3);
                b = m.to_string().into_bytes();
            }
            w.start_file(n, SimpleFileOptions::default()).unwrap();
            w.write_all(&b).unwrap();
        }
        let r3 = read_owndraw(&w.finish().unwrap().into_inner()).unwrap();
        assert_eq!(r3.meta.version, 3);
        assert!(r3.meta.read_only);
        assert_eq!(r3.meta.extra["zukunft"], json!({"a": 1}));
    }

    #[test]
    fn v1_datei_oeffnet() {
        let base = std::env::temp_dir().join(format!("od_v1_{}", crate::store::random_hex(10)));
        let st = crate::store::Store::new(&base);
        let id = st.create("Alt");
        let img = st.save_image_png(&png(5)).unwrap();
        st.save(&id, &json!({"bg":0,"items":[{"t":"i","f":img}],"layers":[]}).to_string(), Some(&png(6)), true);
        let mut buf = Cursor::new(Vec::new());
        assert!(st.export_board(&id, &mut buf));
        let d = read_owndraw(buf.get_ref()).unwrap();
        assert_eq!(d.meta.name, "Alt");
        assert_eq!(d.meta.version, 1);
        assert_eq!(d.mode, BoardMode::Infinite);
        assert_eq!(d.images.files.len(), 1);
        assert!(d.thumb.is_some());
        let again = read_owndraw(&write_owndraw(&d).unwrap()).unwrap();
        assert_eq!(again.meta.version, 2);
        assert_eq!(again.images.files.len(), 1);
        let _ = std::fs::remove_dir_all(&base);
    }

    #[test]
    fn uuid_und_zeit() {
        let u = new_uuid();
        assert_eq!(u.len(), 36);
        assert_ne!(u, new_uuid());
        assert_eq!(rfc3339(0), "1970-01-01T00:00:00Z");
        assert_eq!(rfc3339(1_700_000_000_000), "2023-11-14T22:13:20Z");
    }

    #[test]
    fn textdokument_rundreise_und_alte_versionen() {
        let mut d = Document::new_text("Brief");
        let mut t = crate::office::TextDoc::new();
        t.insert_text(crate::office::Pos::default(), "Hallo", &Default::default());
        d.text = Some(t.to_json());
        let bytes = write_owndraw(&d).unwrap();
        let r = read_owndraw(&bytes).unwrap();
        assert!(r.is_text() && !r.meta.read_only);
        assert_eq!(crate::office::TextDoc::from_json(r.text.as_ref().unwrap()), t);
        assert!(!r.meta.extra.contains_key("kind"));
        // Board bleibt Version 2 ohne text.json
        let b = read_owndraw(&write_owndraw(&Document::new("B", BoardMode::Infinite)).unwrap()).unwrap();
        assert!(!b.is_text() && b.meta.version == FORMAT_VERSION);
        // Ein Board mit Version 3 (ohne "kind": "text") stammt aus der Zukunft: nur lesen
        let mut z = zip::ZipWriter::new(Cursor::new(Vec::new()));
        z.start_file("manifest.json", SimpleFileOptions::default()).unwrap();
        z.write_all(br#"{"format":"owndraw","version":3}"#).unwrap();
        z.start_file("board.json", SimpleFileOptions::default()).unwrap();
        z.write_all(b"{}").unwrap();
        assert!(read_owndraw(&z.finish().unwrap().into_inner()).unwrap().meta.read_only);
    }
}
