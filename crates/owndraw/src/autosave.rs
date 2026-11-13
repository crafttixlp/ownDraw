//! Autosave-Dienst (Hook für WP7).
//!
//! Vertrag mit der Oberfläche: Der Editor ruft pro Frame `tick` auf und bei jeder Änderung `mark_changed`;
//! beim Schließen/Zurück/Pause `flush`. Die Dokument-Bytes werden nur bei Bedarf über die Closure erzeugt
//! (leerer Vektor = Serialisierung fehlgeschlagen). Anzeige: `status()`.
//!
//! Dieser Stub schreibt synchron 2 s nach der letzten Änderung. WP7 ersetzt das durch Hintergrund-Thread,
//! Rechte-Fehleranzeige usw., ohne die Signaturen zu ändern.

use crate::platform::{FileRef, Platform};

/// Verzögerung nach der letzten Änderung (Sekunden).
pub const DEBOUNCE_SECS: f64 = 2.0;

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

pub struct Autosave {
    target: Option<FileRef>,
    dirty_since: Option<f64>,
    status: AutosaveStatus,
}

impl Autosave {
    /// `target`: Zieldatei; `None` = (noch) kein Speicherort, es wird nichts geschrieben.
    pub fn new(target: Option<FileRef>) -> Autosave {
        Autosave { target, dirty_since: None, status: AutosaveStatus::Idle }
    }

    pub fn target(&self) -> Option<&FileRef> {
        self.target.as_ref()
    }

    #[allow(dead_code)]
    pub fn set_target(&mut self, t: Option<FileRef>) {
        self.target = t;
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
        self.dirty_since.map(|t| (t + DEBOUNCE_SECS - now).max(0.0))
    }

    /// Pro Frame aufrufen: schreibt, wenn die Wartezeit abgelaufen ist.
    pub fn tick(&mut self, now: f64, doc_bytes: impl FnOnce() -> Vec<u8>, platform: &dyn Platform) {
        if let Some(t) = self.dirty_since {
            if now - t >= DEBOUNCE_SECS {
                self.write(now, doc_bytes, platform);
            }
        }
    }

    /// Sofort speichern, falls Änderungen anstehen (Schließen, Zurück, App im Hintergrund).
    pub fn flush(&mut self, now: f64, doc_bytes: impl FnOnce() -> Vec<u8>, platform: &dyn Platform) {
        if self.dirty_since.is_some() {
            self.write(now, doc_bytes, platform);
        }
    }

    fn write(&mut self, now: f64, doc_bytes: impl FnOnce() -> Vec<u8>, platform: &dyn Platform) {
        let Some(t) = self.target.clone() else {
            self.dirty_since = None;
            self.status = AutosaveStatus::Idle;
            return;
        };
        self.status = AutosaveStatus::Saving;
        let bytes = doc_bytes();
        if bytes.is_empty() {
            // Beim nächsten Frame erneut versuchen, Fehler anzeigen.
            self.status = AutosaveStatus::Error("Speichern fehlgeschlagen (Format)".into());
            self.dirty_since = Some(now);
            return;
        }
        match platform.write_atomic(&t, &bytes) {
            Ok(()) => {
                self.dirty_since = None;
                self.status = AutosaveStatus::Saved(now);
            }
            Err(e) => {
                self.dirty_since = Some(now);
                self.status = AutosaveStatus::Error(format!("Speichern fehlgeschlagen: {e}"));
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
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::platform::stub::StubPlatform;

    #[test]
    fn debounce_und_flush() {
        let dir = std::env::temp_dir().join(format!("owndraw-as-{}", owndraw_core::store::random_hex(6)));
        let f = FileRef::Path(dir.join("a.owndraw"));
        let pf = StubPlatform::new();
        let mut a = Autosave::new(Some(f.clone()));
        a.mark_changed(10.0);
        a.tick(11.0, || b"x".to_vec(), &pf);
        assert!(!pf.exists(&f));
        a.tick(12.1, || b"x".to_vec(), &pf);
        assert!(pf.exists(&f));
        assert_eq!(a.status(), &AutosaveStatus::Saved(12.1));
        a.mark_changed(13.0);
        a.flush(13.0, || b"yy".to_vec(), &pf);
        assert_eq!(pf.read(&f).unwrap(), b"yy");
        let _ = std::fs::remove_dir_all(dir);
    }
}
