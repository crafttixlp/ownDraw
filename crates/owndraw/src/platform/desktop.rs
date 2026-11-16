//! Desktop-Plattform (WP4): Linux, Windows, macOS.
//!
//! Dialoge über `rfd` (Linux: xdg-desktop-portal, kein GTK), laufen in einem Hintergrund-Thread und
//! liefern ihr Ergebnis über einen Kanal an [`Platform::poll`]. Zwischenablage über `arboard`.

use std::collections::hash_map::DefaultHasher;
use std::hash::{Hash, Hasher};
use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::mpsc::{channel, Receiver, Sender};
use std::sync::{Arc, Mutex, OnceLock};

use super::{ClipData, Drop, FileKind, FileRef, Platform, PlatformEvent, Tag};

/// Art des Dialogs für den Hintergrund-Thread.
enum DialogJob {
    Open(Vec<FileKind>),
    Save(String, FileKind),
    Folder,
}

pub struct DesktopPlatform {
    tx: Sender<PlatformEvent>,
    rx: Mutex<Receiver<PlatformEvent>>,
    /// Datei aus der Kommandozeile, erscheint beim ersten `poll()`.
    pending_open: Mutex<Option<PathBuf>>,
    /// Zuletzt benutzter Ordner (Startordner der Dialoge).
    last_dir: Arc<Mutex<Option<PathBuf>>>,
    /// Einmal erzeugt und gehalten (Wayland/X11 verlieren sonst den Inhalt).
    clipboard: Mutex<Option<arboard::Clipboard>>,
    last_pointer: Mutex<Option<(f32, f32)>>,
}

impl DesktopPlatform {
    pub fn new(open: Option<PathBuf>) -> Self {
        let (tx, rx) = channel();
        DesktopPlatform {
            tx,
            rx: Mutex::new(rx),
            pending_open: Mutex::new(open),
            last_dir: Arc::new(Mutex::new(None)),
            clipboard: Mutex::new(arboard::Clipboard::new().ok()),
            last_pointer: Mutex::new(None),
        }
    }

    fn spawn_dialog(&self, job: DialogJob, tag: Tag) {
        let tx = self.tx.clone();
        let last = self.last_dir.clone();
        let start = last.lock().ok().and_then(|g| g.clone());
        let res = std::thread::Builder::new().name("owndraw-dialog".into()).spawn(move || {
            let picked = pollster::block_on(run_dialog(job, start));
            if let Some(p) = &picked {
                let dir = if p.is_dir() { Some(p.clone()) } else { p.parent().map(Path::to_path_buf) };
                if let (Some(d), Ok(mut g)) = (dir, last.lock()) {
                    *g = Some(d);
                }
            }
            let _ = tx.send(PlatformEvent::Picked { tag, file: picked.map(FileRef::Path) });
        });
        if res.is_err() {
            let _ = self.tx.send(PlatformEvent::Picked { tag, file: None });
        }
    }
}

async fn run_dialog(job: DialogJob, start: Option<PathBuf>) -> Option<PathBuf> {
    let mut d = rfd::AsyncFileDialog::new();
    if let Some(s) = start.filter(|s| s.is_dir()) {
        d = d.set_directory(s);
    }
    match job {
        DialogJob::Open(kinds) => {
            if kinds.len() > 1 {
                let all: Vec<&str> = kinds.iter().flat_map(|k| k.extensions().iter().copied()).collect();
                if !all.is_empty() && !kinds.contains(&FileKind::Any) {
                    d = d.add_filter("Alle unterstützten Dateien", &all);
                }
            }
            for k in kinds {
                if k != FileKind::Any {
                    d = d.add_filter(filter_name(k), k.extensions());
                }
            }
            d.pick_file().await.map(|h| h.path().to_path_buf())
        }
        DialogJob::Save(name, kind) => {
            d = d.set_file_name(name);
            if kind != FileKind::Any {
                d = d.add_filter(filter_name(kind), kind.extensions());
            }
            d.save_file().await.map(|h| h.path().to_path_buf())
        }
        DialogJob::Folder => d.pick_folder().await.map(|h| h.path().to_path_buf()),
    }
}

/// Deutscher Filtername je Dateiart.
pub fn filter_name(k: FileKind) -> &'static str {
    match k {
        FileKind::Owndraw => "OwnDraw-Boards (*.owndraw)",
        FileKind::Xopp => "Xournal++-Dateien (*.xopp)",
        FileKind::Image => "Bilder",
        FileKind::Png => "PNG-Bilder (*.png)",
        FileKind::Pdf => "PDF-Dokumente (*.pdf)",
        FileKind::Zip => "ZIP-Archive (*.zip)",
        FileKind::Any => "Alle Dateien",
    }
}

fn path_of(f: &FileRef) -> io::Result<&Path> {
    match f {
        FileRef::Path(p) => Ok(p),
        FileRef::Uri(_) => Err(io::Error::new(io::ErrorKind::Unsupported, "URIs werden auf dem Desktop nicht unterstützt")),
    }
}

static TMP_COUNTER: AtomicU64 = AtomicU64::new(0);

/// Schreibt atomar: temporäre Datei im Zielordner, fsync, umbenennen. Bei Fehler wird die temporäre Datei entfernt.
pub fn write_atomic_path(path: &Path, data: &[u8]) -> io::Result<()> {
    let dir = match path.parent() {
        Some(d) if !d.as_os_str().is_empty() => d,
        _ => Path::new("."),
    };
    let name = path.file_name().ok_or_else(|| io::Error::new(io::ErrorKind::InvalidInput, "kein Dateiname"))?;
    let n = TMP_COUNTER.fetch_add(1, Ordering::Relaxed);
    let mut tmp_name = std::ffi::OsString::from(".");
    tmp_name.push(name);
    tmp_name.push(format!(".{}.{n}.tmp", std::process::id()));
    let tmp = dir.join(tmp_name);
    let res = (|| {
        let mut f = std::fs::OpenOptions::new().write(true).create_new(true).open(&tmp)?;
        f.write_all(data)?;
        f.sync_all()?;
        drop(f);
        std::fs::rename(&tmp, path)
    })();
    if res.is_err() {
        let _ = std::fs::remove_file(&tmp);
    } else {
        // Verzeichniseintrag dauerhaft machen (Unix); Fehler hier sind unkritisch.
        #[cfg(unix)]
        if let Ok(d) = std::fs::File::open(dir) {
            let _ = d.sync_all();
        }
    }
    res
}

/// Findet im Ordner einen freien Namen: "Name.owndraw", "Name (2).owndraw", …
pub fn unique_name(dir: &Path, name: &str, kind: FileKind) -> String {
    let mut name = name.to_string();
    if let Some(ext) = kind.extensions().first() {
        if !kind.extensions().iter().any(|e| name.to_ascii_lowercase().ends_with(&format!(".{e}"))) {
            name = format!("{name}.{ext}");
        }
    }
    if !dir.join(&name).exists() {
        return name;
    }
    let (stem, ext) = match name.rfind('.') {
        Some(i) if i > 0 => (&name[..i], &name[i..]),
        _ => (name.as_str(), ""),
    };
    (2u32..)
        .map(|n| format!("{stem} ({n}){ext}"))
        .find(|c| !dir.join(c).exists())
        .unwrap_or(name)
}

/// Unterordner unter `OWNDRAW_DATA` bzw. dem Systemordner. `base` ist der Wert der Umgebungsvariable.
fn resolve_dir(base: Option<PathBuf>, sub: &str, sys: Option<PathBuf>) -> PathBuf {
    let p = match base {
        Some(b) => b.join(sub),
        None => sys.unwrap_or_else(|| PathBuf::from(".")).join("OwnDraw"),
    };
    let _ = std::fs::create_dir_all(&p);
    p
}

fn data_override() -> Option<PathBuf> {
    std::env::var_os("OWNDRAW_DATA").filter(|s| !s.is_empty()).map(PathBuf::from)
}

// ---------------------------------------------------------------------------------------------
// Zwischenablage: interne Items

/// Merkt die internen Items-JSON samt Fingerprint des mitgesetzten Bildes/Textes.
#[derive(Default)]
pub struct InternalClip {
    cur: Option<(u64, String)>,
}

impl InternalClip {
    pub fn remember(&mut self, fp: u64, json: String) {
        self.cur = Some((fp, json));
    }
    pub fn forget(&mut self) {
        self.cur = None;
    }
    /// JSON, falls der Fingerprint zum gemerkten passt (Zwischenablage enthält noch unseren Inhalt).
    pub fn resolve(&self, fp: u64) -> Option<&str> {
        self.cur.as_ref().filter(|(f, _)| *f == fp).map(|(_, j)| j.as_str())
    }
}

pub fn fingerprint_image(w: u32, h: u32, rgba: &[u8]) -> u64 {
    let mut s = DefaultHasher::new();
    (0u8, w, h).hash(&mut s);
    rgba.hash(&mut s);
    s.finish()
}

pub fn fingerprint_text(t: &str) -> u64 {
    let mut s = DefaultHasher::new();
    1u8.hash(&mut s);
    t.hash(&mut s);
    s.finish()
}

fn internal() -> &'static Mutex<InternalClip> {
    static C: OnceLock<Mutex<InternalClip>> = OnceLock::new();
    C.get_or_init(|| Mutex::new(InternalClip::default()))
}

impl Platform for DesktopPlatform {
    fn pick_open(&self, kinds: &[FileKind], tag: Tag) {
        self.spawn_dialog(DialogJob::Open(kinds.to_vec()), tag);
    }
    fn pick_save(&self, name: &str, kind: FileKind, tag: Tag) {
        self.spawn_dialog(DialogJob::Save(name.to_string(), kind), tag);
    }
    fn pick_folder(&self, tag: Tag) {
        self.spawn_dialog(DialogJob::Folder, tag);
    }

    fn poll(&self) -> Vec<PlatformEvent> {
        let mut out = Vec::new();
        if let Some(p) = self.pending_open.lock().ok().and_then(|mut g| g.take()) {
            out.push(PlatformEvent::OpenFile(FileRef::Path(p)));
        }
        if let Ok(rx) = self.rx.lock() {
            out.extend(rx.try_iter());
        }
        out
    }

    fn read(&self, f: &FileRef) -> io::Result<Vec<u8>> {
        // Server-Board (`srv:<id>`): Inhalt wurde beim Öffnen geladen (crate::net)
        if let Some(id) = crate::net::srv_id(f) {
            let key = format!("{}{id}", crate::net::SRV);
            return crate::net::with(|n| n.files.get(&key).cloned()).ok_or_else(|| io::Error::new(io::ErrorKind::NotFound, "Das Board ist noch nicht vom Server geladen"));
        }
        std::fs::read(path_of(f)?)
    }
    fn write_atomic(&self, f: &FileRef, data: &[u8]) -> io::Result<()> {
        if let Some(id) = crate::net::srv_id(f) {
            // live verbunden: der Server speichert selbst; sonst Autosave per PUT (Ergebnis kommt asynchron)
            if !crate::net::with(|n| n.live.contains(id)) {
                crate::net::put_board(id, data.to_vec());
            }
            return Ok(());
        }
        write_atomic_path(path_of(f)?, data)
    }
    fn create_in(&self, dir: &FileRef, name: &str, kind: FileKind) -> io::Result<FileRef> {
        let d = path_of(dir)?;
        std::fs::create_dir_all(d)?;
        Ok(FileRef::Path(d.join(unique_name(d, name, kind))))
    }
    fn exists(&self, f: &FileRef) -> bool {
        if crate::net::srv_id(f).is_some() {
            return true;
        }
        path_of(f).map(|p| p.exists()).unwrap_or(false)
    }

    fn default_dir(&self) -> FileRef {
        let sys = dirs::document_dir().or_else(dirs::home_dir);
        FileRef::Path(resolve_dir(data_override(), "Dokumente", sys))
    }
    fn config_dir(&self) -> PathBuf {
        resolve_dir(data_override(), "config", dirs::config_dir())
    }
    fn cache_dir(&self) -> PathBuf {
        resolve_dir(data_override(), "cache", dirs::cache_dir())
    }

    fn clipboard_get(&self) -> Option<ClipData> {
        let mut g = self.clipboard.lock().ok()?;
        let cb = g.as_mut()?;
        let items = internal().lock().ok()?;
        if let Ok(img) = cb.get_image() {
            let (w, h) = (img.width as u32, img.height as u32);
            let rgba = img.bytes.into_owned();
            if let Some(json) = items.resolve(fingerprint_image(w, h, &rgba)) {
                return Some(ClipData::Items { json: json.to_string(), image: Some((w, h, rgba)) });
            }
            return Some(ClipData::Image { w, h, rgba });
        }
        let t = cb.get_text().ok()?;
        if let Some(json) = items.resolve(fingerprint_text(&t)) {
            return Some(ClipData::Items { json: json.to_string(), image: None });
        }
        Some(ClipData::Text(t))
    }

    fn clipboard_set(&self, d: ClipData) {
        let Ok(mut g) = self.clipboard.lock() else { return };
        if g.is_none() {
            *g = arboard::Clipboard::new().ok();
        }
        let Some(cb) = g.as_mut() else { return };
        let Ok(mut items) = internal().lock() else { return };
        fn img(w: u32, h: u32, rgba: &[u8]) -> arboard::ImageData<'_> {
            arboard::ImageData { width: w as usize, height: h as usize, bytes: std::borrow::Cow::Borrowed(rgba) }
        }
        match d {
            ClipData::Image { w, h, rgba } => {
                if cb.set_image(img(w, h, &rgba)).is_ok() {
                    items.forget();
                }
            }
            ClipData::Items { json, image: Some((w, h, rgba)) } => {
                if cb.set_image(img(w, h, &rgba)).is_ok() {
                    items.remember(fingerprint_image(w, h, &rgba), json);
                }
            }
            ClipData::Items { json, image: None } => {
                // Ohne Vorschaubild: JSON als Text ablegen, damit der Inhalt wiedererkannt wird.
                if cb.set_text(json.clone()).is_ok() {
                    items.remember(fingerprint_text(&json), json);
                }
            }
            ClipData::Text(t) => {
                if cb.set_text(t).is_ok() {
                    items.forget();
                }
            }
        }
    }

    fn take_drops(&self, ctx: &egui::Context) -> Vec<Drop> {
        let (files, pos) = ctx.input(|i| (i.raw.dropped_files.clone(), i.pointer.latest_pos()));
        let mut lp = self.last_pointer.lock().ok();
        if let (Some(p), Some(lp)) = (pos, lp.as_mut()) {
            **lp = Some((p.x, p.y));
        }
        let pos = lp.and_then(|g| *g);
        files
            .into_iter()
            .map(|f| {
                let p = f.path().to_path_buf();
                let name = p.file_name().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default();
                // Ohne echten Pfad (z. B. Bild aus dem Browser) Inhalt direkt lesen.
                if p.is_file() {
                    Drop { file: Some(FileRef::Path(p)), bytes: None, name, pos }
                } else {
                    Drop { file: None, bytes: f.bytes().ok(), name, pos }
                }
            })
            .collect()
    }

    fn drag_hover(&self, ctx: &egui::Context) -> bool {
        ctx.input(|i| !i.raw.hovered_files.is_empty())
    }

    fn open_url(&self, url: &str) {
        let _ = open::that_detached(url);
    }

    fn is_touch(&self) -> bool {
        std::env::var("OWNDRAW_TOUCH").map(|v| v == "1").unwrap_or(false)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tmpdir(name: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("owndraw-wp4-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        d
    }

    #[test]
    fn write_atomic_ersetzt_ohne_rest() {
        let d = tmpdir("atomic");
        let p = FileRef::Path(d.join("a.owndraw"));
        let pf = DesktopPlatform::new(None);
        pf.write_atomic(&p, b"eins").unwrap();
        pf.write_atomic(&p, b"zwei-laenger").unwrap();
        assert_eq!(pf.read(&p).unwrap(), "zwei-laenger".as_bytes());
        let names: Vec<_> = std::fs::read_dir(&d).unwrap().map(|e| e.unwrap().file_name()).collect();
        assert_eq!(names.len(), 1, "temp-Rest: {names:?}");
        assert!(pf.exists(&p));
        let _ = std::fs::remove_dir_all(&d);
    }

    #[test]
    fn write_atomic_fehler() {
        let d = tmpdir("atomic-err");
        let pf = DesktopPlatform::new(None);
        // Ordner existiert nicht
        assert!(pf.write_atomic(&FileRef::Path(d.join("fehlt/x.owndraw")), b"x").is_err());
        // Schreibgeschützter Ordner (als root nicht prüfbar)
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let ro = d.join("ro");
            std::fs::create_dir(&ro).unwrap();
            std::fs::set_permissions(&ro, std::fs::Permissions::from_mode(0o555)).unwrap();
            let is_root = std::fs::write(ro.join("probe"), b"").is_ok();
            if !is_root {
                assert!(pf.write_atomic(&FileRef::Path(ro.join("x.owndraw")), b"x").is_err());
                assert_eq!(std::fs::read_dir(&ro).unwrap().count(), 0);
            }
            std::fs::set_permissions(&ro, std::fs::Permissions::from_mode(0o755)).unwrap();
        }
        assert!(pf.write_atomic(&FileRef::Uri("content://x".into()), b"x").is_err());
        let _ = std::fs::remove_dir_all(&d);
    }

    #[test]
    fn create_in_namensfindung() {
        let d = tmpdir("create");
        let pf = DesktopPlatform::new(None);
        let dir = FileRef::Path(d.clone());
        let a = pf.create_in(&dir, "Board", FileKind::Owndraw).unwrap();
        assert_eq!(a, FileRef::Path(d.join("Board.owndraw")));
        std::fs::write(d.join("Board.owndraw"), b"").unwrap();
        let b = pf.create_in(&dir, "Board.owndraw", FileKind::Owndraw).unwrap();
        assert_eq!(b, FileRef::Path(d.join("Board (2).owndraw")));
        std::fs::write(d.join("Board (2).owndraw"), b"").unwrap();
        let c = pf.create_in(&dir, "Board", FileKind::Owndraw).unwrap();
        assert_eq!(c.display_name(), "Board (3).owndraw");
        let _ = std::fs::remove_dir_all(&d);
    }

    #[test]
    fn filekind_key_roundtrip() {
        for r in [FileRef::Path(PathBuf::from("/tmp/a b/c.owndraw")), FileRef::Uri("content://x/y%3Az".into())] {
            assert_eq!(FileRef::from_key(&r.to_key()), Some(r));
        }
        assert_eq!(FileRef::from_key("foo"), None);
    }

    #[test]
    fn items_fingerprint() {
        let rgba = vec![1u8, 2, 3, 4, 5, 6, 7, 8];
        let fp = fingerprint_image(2, 1, &rgba);
        assert_ne!(fp, fingerprint_image(1, 2, &rgba));
        assert_ne!(fp, fingerprint_image(2, 1, &[0; 8]));
        let mut c = InternalClip::default();
        assert!(c.resolve(fp).is_none());
        c.remember(fp, "{\"items\":[]}".into());
        assert_eq!(c.resolve(fp), Some("{\"items\":[]}"));
        assert!(c.resolve(fingerprint_image(2, 1, &[0; 8])).is_none());
        c.forget();
        assert!(c.resolve(fp).is_none());
        assert_ne!(fingerprint_text("a"), fingerprint_text("b"));
    }

    #[test]
    fn verzeichnisse_mit_override() {
        let d = tmpdir("dirs");
        let p = resolve_dir(Some(d.clone()), "config", None);
        assert_eq!(p, d.join("config"));
        assert!(p.is_dir());
        let s = resolve_dir(None, "x", Some(d.clone()));
        assert_eq!(s, d.join("OwnDraw"));
        let _ = std::fs::remove_dir_all(&d);
    }

    #[test]
    fn open_file_beim_ersten_poll() {
        let pf = DesktopPlatform::new(Some(PathBuf::from("/x/y.owndraw")));
        assert!(matches!(pf.poll().as_slice(), [PlatformEvent::OpenFile(FileRef::Path(_))]));
        assert!(pf.poll().is_empty());
    }

    /// Echte Zwischenablage (nur unter Xvfb/Wayland sinnvoll): `cargo test -- --ignored`.
    #[test]
    #[ignore]
    fn clipboard_roundtrip_echt() {
        let pf = DesktopPlatform::new(None);
        let rgba: Vec<u8> = (0..4 * 4 * 4).map(|i| i as u8).collect();
        pf.clipboard_set(ClipData::Items { json: "{}".into(), image: Some((4, 4, rgba.clone())) });
        match pf.clipboard_get() {
            Some(ClipData::Items { json, image: Some((4, 4, r)) }) => {
                assert_eq!(json, "{}");
                assert_eq!(r, rgba);
            }
            other => panic!("unerwartet: {other:?}"),
        }
        pf.clipboard_set(ClipData::Image { w: 4, h: 4, rgba });
        assert!(matches!(pf.clipboard_get(), Some(ClipData::Image { .. })));
    }
}
