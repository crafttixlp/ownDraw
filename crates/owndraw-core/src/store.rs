//! Lokale Ablage: boards/index.json, boards/<id>.json, boards/<id>.png (Vorschau), boards/img/ (Bilder)
//! und Board-Austausch als .owndraw-Datei (ZIP). Dateiformat identisch mit der Java-Version.

use serde_json::{json, Value};
use std::collections::{HashMap, HashSet};
use std::fs;
use std::io::{self, Read, Seek, Write};
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};
use zip::write::SimpleFileOptions;

const MAX_TOTAL: u64 = 200 * 1024 * 1024;
const MAX_SMALL: u64 = 32 * 1024 * 1024;

pub fn now_ms() -> i64 {
    SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_millis() as i64).unwrap_or(0)
}

/// Zahl in Basis 36 (wie Long.toString(v, 36)).
pub fn base36(mut v: i64) -> String {
    if v == 0 {
        return "0".into();
    }
    let neg = v < 0;
    let mut s = Vec::new();
    while v != 0 {
        let d = (v % 36).unsigned_abs() as u8;
        s.push(if d < 10 { b'0' + d } else { b'a' + d - 10 });
        v /= 36;
    }
    if neg {
        s.push(b'-');
    }
    s.reverse();
    String::from_utf8(s).unwrap()
}

/// Zufaellige Hex-Zeichen (ohne zusaetzliche Abhaengigkeit).
pub fn random_hex(n: usize) -> String {
    use std::collections::hash_map::RandomState;
    use std::hash::{BuildHasher, Hasher};
    let mut s = String::new();
    while s.len() < n {
        let mut h = RandomState::new().build_hasher();
        h.write_i64(now_ms());
        h.write_usize(s.len());
        s.push_str(&format!("{:016x}", h.finish()));
    }
    s.truncate(n);
    s
}

/// Schreibt atomar (temporaere Datei, fsync, umbenennen).
pub fn write_atomic(f: &Path, data: &[u8]) -> io::Result<()> {
    let t = PathBuf::from(format!("{}.tmp", f.display()));
    {
        let mut o = fs::File::create(&t)?;
        o.write_all(data)?;
        o.sync_all()?;
    }
    fs::rename(&t, f)
}

/// Ein Board-Eintrag des Index.
#[derive(Clone, Debug, PartialEq)]
pub struct Entry {
    pub id: String,
    pub name: String,
    /// Letzte Aenderung (ms seit 1970).
    pub t: i64,
}

/// Ablage mit dem Board-Ordner `<files>/boards`.
#[derive(Clone, Debug)]
pub struct Store {
    pub dir: PathBuf,
}

impl Store {
    /// files_dir = Android getFilesDir() bzw. Desktop ~/.local/share/OwnDraw/files.
    pub fn new(files_dir: &Path) -> Store {
        let dir = files_dir.join("boards");
        let _ = fs::create_dir_all(dir.join("img"));
        Store { dir }
    }

    pub fn img_dir(&self) -> PathBuf {
        let d = self.dir.join("img");
        let _ = fs::create_dir_all(&d);
        d
    }

    pub fn thumb(&self, id: &str) -> PathBuf {
        self.dir.join(format!("{id}.png"))
    }

    /// Speichert PNG-Daten unter zufaelligem Namen im Bildordner.
    pub fn save_image_png(&self, png: &[u8]) -> Option<String> {
        let name = format!("{}.png", random_hex(16));
        write_atomic(&self.img_dir().join(&name), png).ok()?;
        Some(name)
    }

    /// Laedt eine Bilddatei (png, jpg, gif, bmp, webp), verkleinert sie auf hoechstens 2048 px Kantenlaenge
    /// und legt sie als PNG im Bildordner ab. Liefert (Dateiname, Breite, Hoehe).
    pub fn import_image_file(&self, path: &Path) -> Option<(String, u32, u32)> {
        let img = image::open(path).ok()?;
        let (w, h) = (img.width(), img.height());
        if w == 0 || h == 0 {
            return None;
        }
        // wie BitmapFactory.inSampleSize: in Zweierpotenzen verkleinern, bis die lange Kante <= 2048 ist
        let (mut ss, mx) = (1u32, w.max(h));
        while mx.div_ceil(ss) > 2048 {
            ss *= 2;
        }
        let img = if ss > 1 { img.resize_exact(w.div_ceil(ss), h.div_ceil(ss), image::imageops::FilterType::Triangle) } else { img };
        let rgba = img.into_rgba8();
        let (w, h) = (rgba.width(), rgba.height());
        let mut png = Vec::new();
        image::DynamicImage::ImageRgba8(rgba).write_to(&mut std::io::Cursor::new(&mut png), image::ImageFormat::Png).ok()?;
        let name = self.save_image_png(&png)?;
        Some((name, w, h))
    }

    pub fn index(&self) -> Vec<Value> {
        index_in(&self.dir)
    }

    /// Index als Liste, neueste zuerst.
    pub fn entries(&self) -> Vec<Entry> {
        let mut v: Vec<Entry> = self
            .index()
            .iter()
            .filter_map(|o| {
                Some(Entry {
                    id: o.get("id")?.as_str()?.to_string(),
                    name: o.get("name").and_then(Value::as_str).unwrap_or("Board").to_string(),
                    t: o.get("t").and_then(Value::as_i64).unwrap_or(0),
                })
            })
            .collect();
        v.sort_by(|a, b| b.t.cmp(&a.t));
        v
    }

    fn put_index(&self, a: &[Value]) {
        let _ = write_atomic(&self.dir.join("index.json"), Value::Array(a.to_vec()).to_string().as_bytes());
    }

    pub fn name(&self, id: &str) -> String {
        for o in self.index() {
            if o.get("id").and_then(Value::as_str) == Some(id) {
                return o.get("name").and_then(Value::as_str).unwrap_or("").to_string();
            }
        }
        "Board".into()
    }

    pub fn meta(&self, id: &str, name: Option<&str>, touch: bool) {
        let mut a = self.index();
        for o in a.iter_mut() {
            if o.get("id").and_then(Value::as_str) == Some(id) {
                if let Some(obj) = o.as_object_mut() {
                    if let Some(n) = name {
                        obj.insert("name".into(), json!(n));
                    }
                    if touch {
                        obj.insert("t".into(), json!(now_ms()));
                    }
                }
                self.put_index(&a);
                return;
            }
        }
        a.push(json!({"id": id, "name": name.unwrap_or("Board"), "t": now_ms()}));
        self.put_index(&a);
    }

    pub fn create(&self, name: &str) -> String {
        let ids: HashSet<String> = self.index().iter().filter_map(|o| o.get("id")?.as_str().map(String::from)).collect();
        let mut id = base36(now_ms());
        while ids.contains(&id) {
            id.push('x');
        }
        self.meta(&id, Some(name), true);
        id
    }

    pub fn load(&self, id: &str) -> Option<Value> {
        let s = fs::read(self.dir.join(format!("{id}.json"))).ok()?;
        serde_json::from_slice(&s).ok()
    }

    /// Board-JSON (bereits serialisiert) und optional Vorschau-PNG speichern.
    pub fn save(&self, id: &str, board_json: &str, thumb_png: Option<&[u8]>, touch: bool) -> bool {
        if write_atomic(&self.dir.join(format!("{id}.json")), board_json.as_bytes()).is_err() {
            return false;
        }
        if let Some(t) = thumb_png {
            if write_atomic(&self.thumb(id), t).is_err() {
                return false;
            }
        }
        if touch {
            self.meta(id, None, true);
        }
        true
    }

    pub fn delete(&self, id: &str) {
        let _ = fs::remove_file(self.dir.join(format!("{id}.json")));
        let _ = fs::remove_file(self.thumb(id));
        let b: Vec<Value> = self.index().into_iter().filter(|o| o.get("id").and_then(Value::as_str) != Some(id)).collect();
        self.put_index(&b);
    }

    pub fn duplicate(&self, id: &str) -> String {
        let n = self.create(&format!("{} (Kopie)", self.name(id)));
        if let Ok(d) = fs::read(self.dir.join(format!("{id}.json"))) {
            let _ = write_atomic(&self.dir.join(format!("{n}.json")), &d);
        }
        if let Ok(d) = fs::read(self.thumb(id)) {
            let _ = write_atomic(&self.thumb(&n), &d);
        }
        n
    }

    // ---------------------------------------------------------------- Board-Austausch (.owndraw = ZIP)

    /// Exportiert ein Board als ZIP.
    pub fn export_board<W: Write + Seek>(&self, id: &str, os: W) -> bool {
        export_board_files(&self.dir, &self.img_dir(), id, &self.name(id), os)
    }

    /// Importiert ein .owndraw-ZIP als neues Board; liefert die neue id.
    pub fn import_board<R: Read + Seek>(&self, r: R) -> Option<String> {
        import_board_files(&self.dir, &self.img_dir(), r)
    }
}

pub fn index_in(boards_dir: &Path) -> Vec<Value> {
    fs::read(boards_dir.join("index.json"))
        .ok()
        .and_then(|s| serde_json::from_slice::<Value>(&s).ok())
        .and_then(|v| v.as_array().cloned())
        .unwrap_or_default()
}

fn safe_name(n: &str) -> bool {
    !n.is_empty() && n.chars().count() < 200 && !n.contains('/') && !n.contains('\\') && !n.contains("..") && !n.contains('\0') && !n.starts_with('.')
}

fn image_files(board: &Value) -> Vec<String> {
    board
        .get("items")
        .and_then(Value::as_array)
        .map(|a| {
            a.iter()
                .filter(|o| o.get("t").and_then(Value::as_str) == Some("i"))
                .filter_map(|o| o.get("f").and_then(Value::as_str))
                .filter(|f| !f.is_empty())
                .map(String::from)
                .collect()
        })
        .unwrap_or_default()
}

pub fn export_board_files<W: Write + Seek>(boards_dir: &Path, img_dir: &Path, id: &str, name: &str, os: W) -> bool {
    (|| -> Option<()> {
        if !safe_name(id) {
            return None;
        }
        let board_bytes = fs::read(boards_dir.join(format!("{id}.json"))).ok()?;
        let board: Value = serde_json::from_slice(&board_bytes).ok()?;
        let mut z = zip::ZipWriter::new(os);
        let opt = SimpleFileOptions::default().compression_method(zip::CompressionMethod::Deflated);
        let meta = json!({"name": name, "format": 1, "app": "OwnDraw"}).to_string();
        z.start_file("meta.json", opt).ok()?;
        z.write_all(meta.as_bytes()).ok()?;
        z.start_file("board.json", opt).ok()?;
        z.write_all(&board_bytes).ok()?;
        if let Ok(t) = fs::read(boards_dir.join(format!("{id}.png"))) {
            z.start_file("thumb.png", opt).ok()?;
            z.write_all(&t).ok()?;
        }
        let mut done = HashSet::new();
        for f in image_files(&board) {
            if !safe_name(&f) || !done.insert(f.clone()) {
                continue;
            }
            let p = img_dir.join(&f);
            if p.is_file() {
                let d = fs::read(&p).ok()?;
                z.start_file(format!("img/{f}"), opt).ok()?;
                z.write_all(&d).ok()?;
            }
        }
        z.finish().ok()?;
        Some(())
    })()
    .is_some()
}

fn read_limited<R: Read>(r: R, max: u64) -> Option<Vec<u8>> {
    let mut v = Vec::new();
    r.take(max + 1).read_to_end(&mut v).ok()?;
    if v.len() as u64 > max {
        None
    } else {
        Some(v)
    }
}

pub fn import_board_files<R: Read + Seek>(boards_dir: &Path, img_dir: &Path, r: R) -> Option<String> {
    let _ = fs::create_dir_all(boards_dir);
    let _ = fs::create_dir_all(img_dir);
    let stage = boards_dir.join(format!(".import_{}", random_hex(12)));
    fs::create_dir_all(&stage).ok()?;
    let mut created: Vec<PathBuf> = Vec::new();
    let res = import_inner(boards_dir, img_dir, r, &stage, &mut created);
    let _ = fs::remove_dir_all(&stage);
    if res.is_none() {
        for f in created {
            let _ = fs::remove_file(f);
        }
    }
    res
}

fn import_inner<R: Read + Seek>(boards_dir: &Path, img_dir: &Path, r: R, stage: &Path, created: &mut Vec<PathBuf>) -> Option<String> {
    let mut z = zip::ZipArchive::new(r).ok()?;
    let (mut meta_b, mut board_b, mut thumb_b): (Option<Vec<u8>>, Option<Vec<u8>>, Option<Vec<u8>>) = (None, None, None);
    let mut staged: Vec<String> = Vec::new();
    let mut total: u64 = 0;
    for i in 0..z.len() {
        let mut e = z.by_index(i).ok()?;
        let n = e.name().ok()?.to_string();
        if n.starts_with('/') || n.contains('\\') || n.contains("..") || n.contains('\0') {
            return None;
        }
        if e.is_dir() {
            continue;
        }
        if n == "meta.json" || n == "board.json" || n == "thumb.png" {
            let d = read_limited(&mut e, MAX_SMALL)?;
            total += d.len() as u64;
            if total > MAX_TOTAL {
                return None;
            }
            match n.as_str() {
                "meta.json" => meta_b = Some(d),
                "board.json" => board_b = Some(d),
                _ => thumb_b = Some(d),
            }
        } else if let Some(f) = n.strip_prefix("img/") {
            if !safe_name(f) || staged.iter().any(|s| s == f) {
                return None;
            }
            let d = read_limited(&mut e, MAX_TOTAL - total)?;
            total += d.len() as u64;
            fs::write(stage.join(f), d).ok()?;
            staged.push(f.to_string());
        }
        // andere Eintraege werden ignoriert
    }
    let mut board: Value = serde_json::from_slice(&board_b?).ok()?;
    board.get("items")?.as_array()?;

    let mut name = "Importiertes Board".to_string();
    if let Some(mb) = meta_b {
        if let Ok(m) = serde_json::from_slice::<Value>(&mb) {
            let mn = m.get("name").and_then(Value::as_str).unwrap_or("").trim().to_string();
            if !mn.is_empty() {
                name = mn.chars().take(80).collect();
            }
        }
    }

    // Bilder in das Bildverzeichnis uebernehmen
    let mut map: HashMap<String, String> = HashMap::new();
    for f in &staged {
        let src = stage.join(f);
        let mut dst = img_dir.join(f);
        let mut usen = f.clone();
        if dst.exists() {
            if fs::read(&src).ok()? == fs::read(&dst).ok()? {
                map.insert(f.clone(), f.clone());
                continue;
            }
            let ext = match f.rfind('.') {
                Some(d) if d > 0 && f.len() - d <= 6 => f[d..].to_string(),
                _ => ".png".into(),
            };
            loop {
                usen = format!("{}{ext}", random_hex(16));
                dst = img_dir.join(&usen);
                if !dst.exists() {
                    break;
                }
            }
        }
        fs::copy(&src, &dst).ok()?;
        created.push(dst);
        map.insert(f.clone(), usen);
    }
    if let Some(items) = board.get_mut("items").and_then(Value::as_array_mut) {
        for o in items.iter_mut() {
            if o.get("t").and_then(Value::as_str) != Some("i") {
                continue;
            }
            let f = o.get("f").and_then(Value::as_str).unwrap_or("").to_string();
            if f.is_empty() {
                continue;
            }
            if let Some(m) = map.get(&f) {
                o["f"] = json!(m);
            } else if !safe_name(&f) {
                o["f"] = json!("");
            }
        }
    }

    // Name eindeutig machen
    let mut idx = index_in(boards_dir);
    let names: HashSet<String> = idx.iter().filter_map(|o| o.get("name")?.as_str().map(String::from)).collect();
    let ids: HashSet<String> = idx.iter().filter_map(|o| o.get("id")?.as_str().map(String::from)).collect();
    let mut final_name = name.clone();
    let mut k = 2;
    while names.contains(&final_name) {
        final_name = format!("{name} ({k})");
        k += 1;
    }
    let mut id = base36(now_ms());
    while ids.contains(&id) || boards_dir.join(format!("{id}.json")).exists() {
        id.push('x');
    }

    let bf = boards_dir.join(format!("{id}.json"));
    write_atomic(&bf, board.to_string().as_bytes()).ok()?;
    created.push(bf);
    if let Some(t) = thumb_b {
        let tf = boards_dir.join(format!("{id}.png"));
        write_atomic(&tf, &t).ok()?;
        created.push(tf);
    }
    idx.push(json!({"id": id, "name": final_name, "t": now_ms()}));
    write_atomic(&boards_dir.join("index.json"), Value::Array(idx).to_string().as_bytes()).ok()?;
    Some(id)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Cursor;

    fn tmp(name: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("owndraw-test-{name}-{}", random_hex(8)));
        let _ = fs::remove_dir_all(&d);
        d
    }

    #[test]
    fn base36_matches_java() {
        assert_eq!(base36(1731535200000), "m3gfcn40");
        assert_eq!(base36(35), "z");
    }

    #[test]
    fn create_save_export_import() {
        let root = tmp("a");
        let s = Store::new(&root);
        let id = s.create("Mein Board");
        fs::write(s.img_dir().join("bild.png"), b"PNGDATA").unwrap();
        let board = json!({"items": [{"t":"i","c":-1,"w":4,"x":0,"y":0,"iw":10,"ih":10,"f":"bild.png","ly":1}], "layers": []});
        assert!(s.save(&id, &board.to_string(), Some(b"thumb"), true));
        let mut buf = Cursor::new(Vec::new());
        assert!(s.export_board(&id, &mut buf));

        // Import in denselben Speicher: gleicher Bildinhalt -> gleicher Name, Board-Name eindeutig
        let id2 = s.import_board(Cursor::new(buf.get_ref().clone())).unwrap();
        assert_ne!(id, id2);
        assert_eq!(s.name(&id2), "Mein Board (2)");
        assert_eq!(s.load(&id2).unwrap()["items"][0]["f"], "bild.png");

        // anderes Bild gleichen Namens -> Umbenennung
        fs::write(s.img_dir().join("bild.png"), b"ANDERS").unwrap();
        let id3 = s.import_board(Cursor::new(buf.get_ref().clone())).unwrap();
        let f = s.load(&id3).unwrap()["items"][0]["f"].as_str().unwrap().to_string();
        assert_ne!(f, "bild.png");
        assert_eq!(fs::read(s.img_dir().join(&f)).unwrap(), b"PNGDATA");
        assert_eq!(s.entries().len(), 3);

        let d = s.duplicate(&id);
        assert_eq!(s.name(&d), "Mein Board (Kopie)");
        s.delete(&d);
        assert_eq!(s.entries().len(), 3);
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn rejects_bad_zip() {
        let root = tmp("b");
        let s = Store::new(&root);
        let mut buf = Cursor::new(Vec::new());
        {
            let mut z = zip::ZipWriter::new(&mut buf);
            z.start_file("img/../boese", SimpleFileOptions::default()).unwrap();
            z.write_all(b"x").unwrap();
            z.start_file("board.json", SimpleFileOptions::default()).unwrap();
            z.write_all(b"{\"items\":[]}").unwrap();
            z.finish().unwrap();
        }
        assert!(s.import_board(Cursor::new(buf.into_inner())).is_none());
        assert!(s.entries().is_empty());
        let _ = fs::remove_dir_all(root);
    }
}
