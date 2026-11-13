//! Autosave-Dienst (WP7).
//!
//! Vertrag mit der Oberfläche: Der Editor ruft pro Frame `tick` auf und bei jeder Änderung `mark_changed`;
//! beim Schließen/Zurück/Pause/Beenden `flush`. Die Dokument-Bytes werden nur bei Bedarf über die Closure erzeugt
//! (leerer Vektor = Serialisierung fehlgeschlagen) und immer auf dem UI-Thread (das Board ist nicht `Send`).
//! Anzeige: `status()`.
//!
//! Ablauf: 2 s nach der letzten Änderung wird serialisiert; das Schreiben (temporäre Datei → fsync → umbenennen)
//! läuft für `FileRef::Path` in einem Hintergrund-Thread, für `FileRef::Uri` synchron über die Plattform
//! (`Platform` ist nicht `Send`). Es läuft höchstens ein Schreibvorgang gleichzeitig; `flush` wartet darauf und
//! schreibt dann synchron.

use crate::platform::{FileRef, Platform};
use std::io::{self, Write};
use std::path::Path;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::mpsc::{channel, Receiver, TryRecvError};

/// Verzögerung nach der letzten Änderung (Sekunden).
pub const DEBOUNCE_SECS: f64 = 2.0;

/// Dateiformat des Ziels.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum SaveFormat {
    #[default]
    Owndraw,
    /// Direkt ins Xournal++-Format (Einstellung `xopp_autosave_direct`).
    Xopp,
}

#[derive(Clone, Debug, PartialEq)]
pub enum AutosaveStatus {
    /// Nichts zu tun, noch nie gespeichert.
    Idle,
    /// Ungespeicherte Änderungen vorhanden.
    Dirty,
    /// Schreibvorgang läuft.
    Saving,
    /// Zuletzt gespeichert (Zeitpunkt in Sekunden der UI-Uhr).
    Saved(f64),
    /// Speichern fehlgeschlagen (Text für die Anzeige).
    Error(String),
}

static TMP_COUNTER: AtomicU64 = AtomicU64::new(0);

/// Schreibt atomar: temporäre Datei im Zielordner, fsync, umbenennen (gleiche Logik wie `desktop.rs`, aber ohne
/// `Platform`, damit sie in einem Hintergrund-Thread laufen kann). Nach einem Abbruch (SIGKILL, Stromausfall)
/// ist die Zieldatei entweder komplett alt oder komplett neu.
pub fn write_path_atomic(path: &Path, data: &[u8]) -> io::Result<()> {
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
        #[cfg(unix)]
        if let Ok(d) = std::fs::File::open(dir) {
            let _ = d.sync_all();
        }
    }
    res
}

/// Verständlicher Fehlertext für die Anzeige.
pub fn describe_error(e: &io::Error, name: &str) -> String {
    match e.kind() {
        io::ErrorKind::PermissionDenied | io::ErrorKind::ReadOnlyFilesystem => {
            format!("Keine Schreibrechte für „{name}“. Das Board ist nicht gesichert – speichere eine Kopie an einem anderen Ort.")
        }
        io::ErrorKind::NotFound => format!("Der Ordner von „{name}“ ist nicht mehr erreichbar (Laufwerk getrennt oder Ordner gelöscht?)."),
        io::ErrorKind::StorageFull => format!("Auf dem Speicher ist kein Platz mehr frei („{name}“)."),
        _ => format!("Speichern fehlgeschlagen: {e}"),
    }
}

pub struct Autosave {
    target: Option<FileRef>,
    format: SaveFormat,
    dirty_since: Option<f64>,
    status: AutosaveStatus,
    /// Ergebnis des laufenden Hintergrund-Schreibens.
    inflight: Option<Receiver<io::Result<()>>>,
    /// Letzter Fehler war ein Rechteproblem (für die Aktion „Kopie speichern unter …“).
    perm_error: bool,
    /// Dokument aus neuerer Programmversion: nie schreiben.
    read_only: bool,
}

impl Autosave {
    /// `target`: Zieldatei; `None` = (noch) kein Speicherort, es wird nichts geschrieben.
    pub fn new(target: Option<FileRef>) -> Autosave {
        Autosave { target, format: SaveFormat::Owndraw, dirty_since: None, status: AutosaveStatus::Idle, inflight: None, perm_error: false, read_only: false }
    }

    pub fn target(&self) -> Option<&FileRef> {
        self.target.as_ref()
    }

    #[allow(dead_code)]
    pub fn set_target(&mut self, t: Option<FileRef>) {
        self.target = t;
    }

    pub fn set_read_only(&mut self, r: bool) {
        self.read_only = r;
    }

    pub fn format(&self) -> SaveFormat {
        self.format
    }

    pub fn set_format(&mut self, f: SaveFormat) {
        self.format = f;
    }

    /// Meldet eine Änderung (`now` = UI-Uhr in Sekunden). Startet/verlängert die Wartezeit.
    pub fn mark_changed(&mut self, now: f64) {
        self.dirty_since = Some(now);
        if !matches!(self.status, AutosaveStatus::Error(_)) {
            self.status = AutosaveStatus::Dirty;
        }
    }

    #[allow(dead_code)]
    pub fn is_dirty(&self) -> bool {
        self.dirty_since.is_some()
    }

    /// Sekunden bis zum nächsten fälligen Speichern (für `request_repaint_after`), None wenn nichts ansteht.
    pub fn next_due(&self, now: f64) -> Option<f64> {
        if self.inflight.is_some() {
            return Some(0.05);
        }
        self.dirty_since.map(|t| (t + DEBOUNCE_SECS - now).max(0.0))
    }

    /// Pro Frame aufrufen: wertet ein beendetes Schreiben aus und startet ein neues, wenn die Wartezeit abgelaufen ist.
    pub fn tick(&mut self, now: f64, doc_bytes: impl FnOnce() -> Vec<u8>, platform: &dyn Platform) {
        self.poll(now);
        if self.inflight.is_none() {
            if let Some(t) = self.dirty_since {
                if now - t >= DEBOUNCE_SECS {
                    self.write(now, doc_bytes, platform, false);
                }
            }
        }
    }

    /// Sofort und synchron speichern, falls Änderungen anstehen (Schließen, Zurück, App im Hintergrund, Beenden).
    pub fn flush(&mut self, now: f64, doc_bytes: impl FnOnce() -> Vec<u8>, platform: &dyn Platform) {
        self.wait_idle(now);
        if self.dirty_since.is_some() {
            self.write(now, doc_bytes, platform, true);
        }
    }

    /// Wartet auf ein laufendes Hintergrund-Schreiben und wertet es aus.
    pub fn wait_idle(&mut self, now: f64) {
        if let Some(rx) = self.inflight.take() {
            let r = rx.recv().unwrap_or_else(|_| Err(io::Error::other("Schreib-Thread abgebrochen")));
            self.finish(now, r);
        }
    }

    fn poll(&mut self, now: f64) {
        let Some(rx) = &self.inflight else { return };
        let r = match rx.try_recv() {
            Ok(r) => r,
            Err(TryRecvError::Empty) => return,
            Err(TryRecvError::Disconnected) => Err(io::Error::other("Schreib-Thread abgebrochen")),
        };
        self.inflight = None;
        self.finish(now, r);
    }

    fn finish(&mut self, now: f64, r: io::Result<()>) {
        match r {
            Ok(()) => {
                self.perm_error = false;
                // Während des Schreibens kam eine neue Änderung: bleibt „ungespeichert“.
                self.status = if self.dirty_since.is_some() { AutosaveStatus::Dirty } else { AutosaveStatus::Saved(now) };
            }
            Err(e) => {
                let name = self.target.as_ref().map(FileRef::display_name).unwrap_or_default();
                self.perm_error = matches!(e.kind(), io::ErrorKind::PermissionDenied | io::ErrorKind::ReadOnlyFilesystem);
                self.dirty_since.get_or_insert(now);
                self.status = AutosaveStatus::Error(describe_error(&e, &name));
            }
        }
    }

    fn write(&mut self, now: f64, doc_bytes: impl FnOnce() -> Vec<u8>, platform: &dyn Platform, sync: bool) {
        let Some(t) = self.target.clone().filter(|_| !self.read_only) else {
            self.dirty_since = None;
            self.status = AutosaveStatus::Idle;
            return;
        };
        let bytes = doc_bytes();
        if bytes.is_empty() {
            // Beim nächsten Versuch erneut, Fehler anzeigen.
            self.status = AutosaveStatus::Error("Speichern fehlgeschlagen (Format)".into());
            self.dirty_since = Some(now);
            return;
        }
        self.status = AutosaveStatus::Saving;
        self.dirty_since = None;
        match t {
            FileRef::Path(p) if !sync => {
                let (tx, rx) = channel();
                self.inflight = Some(rx);
                let spawned = std::thread::Builder::new().name("owndraw-autosave".into()).spawn(move || {
                    let _ = tx.send(write_path_atomic(&p, &bytes));
                });
                if spawned.is_err() {
                    self.inflight = None;
                    self.finish(now, Err(io::Error::other("Schreib-Thread nicht startbar")));
                }
            }
            FileRef::Path(p) => {
                let r = write_path_atomic(&p, &bytes);
                self.finish(now, r);
            }
            uri => {
                let r = platform.write_atomic(&uri, &bytes);
                self.finish(now, r);
            }
        }
    }

    pub fn status(&self) -> &AutosaveStatus {
        &self.status
    }

    /// Fehlertext zur Anzeige, falls das letzte Speichern fehlschlug.
    pub fn error(&self) -> Option<&str> {
        match &self.status {
            AutosaveStatus::Error(s) => Some(s),
            _ => None,
        }
    }

    /// Letzter Fehler: fehlende Schreibrechte (Anzeige mit Aktion „Kopie speichern unter …“).
    pub fn permission_error(&self) -> bool {
        self.perm_error && self.error().is_some()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::platform::stub::StubPlatform;
    use owndraw_core::doc::{read_owndraw, write_owndraw, Document};
    use owndraw_core::pages::BoardMode;

    fn tmpdir(n: &str) -> std::path::PathBuf {
        let d = std::env::temp_dir().join(format!("owndraw-as-{n}-{}", owndraw_core::store::random_hex(6)));
        std::fs::create_dir_all(&d).unwrap();
        d
    }

    #[test]
    fn debounce_zeit_injiziert() {
        let d = tmpdir("deb");
        let f = FileRef::Path(d.join("a.owndraw"));
        let pf = StubPlatform::new();
        let mut a = Autosave::new(Some(f.clone()));
        assert_eq!(a.next_due(0.0), None);
        a.mark_changed(10.0);
        assert_eq!(a.status(), &AutosaveStatus::Dirty);
        assert!((a.next_due(11.0).unwrap() - 1.0).abs() < 1e-9);
        a.tick(11.9, || b"x".to_vec(), &pf);
        assert!(!pf.exists(&f), "vor Ablauf der Wartezeit nichts schreiben");
        // weitere Änderung verlängert die Wartezeit
        a.mark_changed(11.9);
        a.tick(13.0, || b"x".to_vec(), &pf);
        assert!(!pf.exists(&f));
        a.tick(14.0, || b"eins".to_vec(), &pf);
        assert_eq!(a.status(), &AutosaveStatus::Saving);
        a.wait_idle(14.0);
        assert_eq!(a.status(), &AutosaveStatus::Saved(14.0));
        assert_eq!(pf.read(&f).unwrap(), b"eins");
        assert!(!a.is_dirty());
        assert_eq!(a.next_due(15.0), None);
        // flush schreibt sofort und synchron
        a.mark_changed(20.0);
        a.flush(20.0, || b"zwei".to_vec(), &pf);
        assert_eq!(pf.read(&f).unwrap(), b"zwei");
        assert_eq!(a.status(), &AutosaveStatus::Saved(20.0));
        // flush ohne Änderung schreibt nicht
        a.flush(21.0, || panic!("nicht aufrufen"), &pf);
        let _ = std::fs::remove_dir_all(d);
    }

    #[test]
    fn aenderung_waehrend_schreiben_bleibt_dirty() {
        let d = tmpdir("dirty");
        let f = FileRef::Path(d.join("a.owndraw"));
        let pf = StubPlatform::new();
        let mut a = Autosave::new(Some(f));
        a.mark_changed(0.0);
        a.tick(2.0, || b"a".to_vec(), &pf);
        a.mark_changed(2.1);
        a.wait_idle(2.2);
        assert_eq!(a.status(), &AutosaveStatus::Dirty);
        assert!(a.is_dirty());
        let _ = std::fs::remove_dir_all(d);
    }

    #[test]
    fn fehler_rechte_und_wiederholung() {
        let pf = StubPlatform::new();
        let f = FileRef::Path(std::path::PathBuf::from("/proc/owndraw-nicht-schreibbar/a.owndraw"));
        let mut a = Autosave::new(Some(f));
        a.mark_changed(0.0);
        a.flush(5.0, || b"x".to_vec(), &pf);
        assert!(a.error().is_some());
        assert!(a.is_dirty(), "nach Fehler erneut versuchen");
        let e = io::Error::from(io::ErrorKind::PermissionDenied);
        assert!(describe_error(&e, "b.owndraw").contains("Schreibrechte"));
    }

    #[test]
    fn leeres_ergebnis_ist_fehler() {
        let pf = StubPlatform::new();
        let mut a = Autosave::new(Some(FileRef::Path("/tmp/x.owndraw".into())));
        a.mark_changed(0.0);
        a.flush(1.0, Vec::new, &pf);
        assert!(a.error().is_some());
        assert!(a.is_dirty());
    }

    /// Kindprozess des Kill-Tests: schreibt endlos große Dokumente (nur aktiv mit `OWNDRAW_KILLTEST_FILE`).
    #[test]
    fn kill_child() {
        let Ok(path) = std::env::var("OWNDRAW_KILLTEST_FILE") else { return };
        let path = std::path::PathBuf::from(path);
        let mut doc = Document::new("Kill", BoardMode::Infinite);
        for i in 0u64.. {
            // pro Durchlauf andere, schlecht komprimierbare Bilder (einige MB)
            let mut x = i.wrapping_mul(0x9E37_79B9_7F4A_7C15) | 1;
            let bytes: Vec<u8> = (0..3_000_000).map(|_| { x ^= x << 13; x ^= x >> 7; x ^= x << 17; x as u8 }).collect();
            doc.images.add(bytes, "png");
            if doc.images.files.len() > 3 {
                let k = doc.images.files.keys().next().cloned().unwrap();
                doc.images.files.remove(&k);
            }
            doc.meta.name = format!("Kill {i}");
            let data = write_owndraw(&doc).unwrap();
            write_path_atomic(&path, &data).unwrap();
        }
    }

    /// Kill-Test: Kindprozess schreibt große Dokumente, wird per SIGKILL mitten drin beendet; die Datei muss
    /// danach jedes Mal lesbar sein.
    #[test]
    fn kill_test_datei_bleibt_lesbar() {
        if std::env::var("OWNDRAW_KILLTEST_FILE").is_ok() {
            return;
        }
        let d = tmpdir("kill");
        let path = d.join("k.owndraw");
        write_path_atomic(&path, &write_owndraw(&Document::new("Start", BoardMode::Infinite)).unwrap()).unwrap();
        let exe = std::env::current_exe().unwrap();
        let mut gesehen = std::collections::BTreeSet::new();
        for run in 0..20u64 {
            let mut child = std::process::Command::new(&exe)
                .args(["--exact", "autosave::tests::kill_child", "--nocapture", "--test-threads=1"])
                .env("OWNDRAW_KILLTEST_FILE", &path)
                .stdout(std::process::Stdio::null())
                .stderr(std::process::Stdio::null())
                .spawn()
                .unwrap();
            // Warten, bis das Kind mindestens einmal geschrieben hat, dann an zufälliger Stelle abbrechen.
            let t0 = std::time::Instant::now();
            let before = std::fs::metadata(&path).unwrap().modified().unwrap();
            while std::fs::metadata(&path).unwrap().modified().unwrap() == before && t0.elapsed().as_secs() < 30 {
                std::thread::sleep(std::time::Duration::from_millis(5));
            }
            std::thread::sleep(std::time::Duration::from_millis(3 + (run * 37) % 90));
            child.kill().unwrap(); // SIGKILL
            child.wait().unwrap();
            let bytes = std::fs::read(&path).unwrap();
            let doc = read_owndraw(&bytes).unwrap_or_else(|e| panic!("Durchlauf {run}: Datei nicht lesbar: {e}"));
            gesehen.insert(doc.meta.name.clone());
        }
        assert!(gesehen.len() > 1, "Kind hat nichts geschrieben: {gesehen:?}");
        let _ = std::fs::remove_dir_all(d);
    }
}
