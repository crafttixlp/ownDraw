//! Einfache Plattform für die Entwicklung der Oberfläche (WP6).
//!
//! WIRD BEI DER INTEGRATION DURCH `DesktopPlatform` (WP4) ERSETZT. Verhalten:
//! - Dialoge öffnen nichts; sie liefern sofort „abgebrochen“, außer die Umgebungsvariable
//!   `OWNDRAW_PICK` (Pfad) ist gesetzt: dann wird dieser Pfad als Ergebnis geliefert.
//! - Dateien über `std::fs`, Zwischenablage leer, Drops aus egui, `OWNDRAW_TOUCH=1` simuliert Touch.

use super::{ClipData, Drop, FileKind, FileRef, Platform, PlatformEvent, Tag};
use std::cell::RefCell;
use std::io;
use std::path::PathBuf;

#[derive(Default)]
pub struct StubPlatform {
    events: RefCell<Vec<PlatformEvent>>,
    /// Zwischenablage im Speicher (Tests/Demo).
    clip: RefCell<Option<ClipData>>,
    /// Vorgemerkte Drops (Tests/Demo), werden von `take_drops` geliefert.
    drops: RefCell<Vec<Drop>>,
}

impl StubPlatform {
    pub fn new() -> StubPlatform {
        StubPlatform::default()
    }

    /// Merkt einen Drop vor (Tests/Demo).
    pub fn push_drop(&self, d: Drop) {
        self.drops.borrow_mut().push(d);
    }

    fn answer(&self, tag: Tag) {
        let file = std::env::var("OWNDRAW_PICK")
            .ok()
            .filter(|s| !s.is_empty())
            .map(|p| FileRef::Path(PathBuf::from(p)));
        self.events
            .borrow_mut()
            .push(PlatformEvent::Picked { tag, file });
    }

    /// Ordner für Konfiguration/Cache im Stub: `OWNDRAW_DATA` bzw. `~/.local/share/OwnDraw`.
    fn base() -> PathBuf {
        crate::data_dir()
    }
}

impl Platform for StubPlatform {
    fn pick_open(&self, _kinds: &[FileKind], tag: Tag) {
        self.answer(tag);
    }
    fn pick_save(&self, _name: &str, _kind: FileKind, tag: Tag) {
        self.answer(tag);
    }
    fn pick_folder(&self, tag: Tag) {
        self.answer(tag);
    }
    fn poll(&self) -> Vec<PlatformEvent> {
        std::mem::take(&mut *self.events.borrow_mut())
    }

    fn read(&self, f: &FileRef) -> io::Result<Vec<u8>> {
        match f {
            FileRef::Path(p) => std::fs::read(p),
            FileRef::Uri(_) => Err(io::Error::new(
                io::ErrorKind::Unsupported,
                "URI nicht unterstützt",
            )),
        }
    }
    fn write_atomic(&self, f: &FileRef, data: &[u8]) -> io::Result<()> {
        use std::io::Write;
        let FileRef::Path(p) = f else {
            return Err(io::Error::new(
                io::ErrorKind::Unsupported,
                "URI nicht unterstützt",
            ));
        };
        if let Some(d) = p.parent() {
            std::fs::create_dir_all(d)?;
        }
        let t = PathBuf::from(format!("{}.tmp", p.display()));
        {
            let mut o = std::fs::File::create(&t)?;
            o.write_all(data)?;
            o.sync_all()?;
        }
        std::fs::rename(&t, p)
    }
    fn create_in(&self, dir: &FileRef, name: &str, _kind: FileKind) -> io::Result<FileRef> {
        let FileRef::Path(d) = dir else {
            return Err(io::Error::new(
                io::ErrorKind::Unsupported,
                "URI nicht unterstützt",
            ));
        };
        std::fs::create_dir_all(d)?;
        let (stem, ext) = match name.rsplit_once('.') {
            Some((s, e)) => (s.to_string(), format!(".{e}")),
            None => (name.to_string(), String::new()),
        };
        let mut p = d.join(name);
        let mut n = 2;
        while p.exists() {
            p = d.join(format!("{stem} ({n}){ext}"));
            n += 1;
        }
        Ok(FileRef::Path(p))
    }
    fn exists(&self, f: &FileRef) -> bool {
        matches!(f, FileRef::Path(p) if p.exists())
    }
    fn default_dir(&self) -> FileRef {
        FileRef::Path(Self::base().join("Dokumente"))
    }
    fn config_dir(&self) -> PathBuf {
        Self::base().join("config")
    }
    fn cache_dir(&self) -> PathBuf {
        Self::base().join("cache")
    }

    fn clipboard_get(&self) -> Option<ClipData> {
        self.clip.borrow().clone()
    }
    fn clipboard_set(&self, d: ClipData) {
        *self.clip.borrow_mut() = Some(d);
    }
    fn take_drops(&self, ctx: &egui::Context) -> Vec<Drop> {
        let pos = ctx.input(|i| i.pointer.hover_pos()).map(|p| (p.x, p.y));
        let mut queued = std::mem::take(&mut *self.drops.borrow_mut());
        queued.extend(ctx.input(|i| {
            i.raw
                .dropped_files
                .iter()
                .map(|f| {
                    let path = f.path().to_path_buf();
                    Drop {
                        name: path
                            .file_name()
                            .map(|n| n.to_string_lossy().into_owned())
                            .unwrap_or_default(),
                        bytes: None,
                        file: Some(FileRef::Path(path)),
                        pos,
                    }
                })
                .collect::<Vec<_>>()
        }));
        queued
    }
    fn drag_hover(&self, ctx: &egui::Context) -> bool {
        ctx.input(|i| !i.raw.hovered_files.is_empty())
    }
    fn open_url(&self, url: &str) {
        let _ = std::process::Command::new("xdg-open").arg(url).spawn();
    }
    fn is_touch(&self) -> bool {
        std::env::var("OWNDRAW_TOUCH")
            .map(|v| v == "1")
            .unwrap_or(false)
    }
}
