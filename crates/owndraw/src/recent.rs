//! Zuletzt-geöffnet-Liste (WP7).
//!
//! Vertrag: `Recent::load(dir)`, `add`, `remove`, `list`. Die Liste liegt als `recent.json` im Konfigurationsordner
//! (`Platform::config_dir`). Vorschaubilder stehen nicht hier, sondern in den Dateien (`session::read_thumb`).

use crate::platform::FileRef;
use serde_json::{json, Value};
use std::collections::HashSet;
use std::path::{Path, PathBuf};

/// Höchstzahl gemerkter Einträge.
pub const MAX_RECENT: usize = 40;

#[derive(Clone, Debug, PartialEq)]
pub struct RecentEntry {
    pub file: FileRef,
    pub name: String,
    /// Zeitpunkt des letzten Öffnens/Speicherns (ms seit 1970).
    pub modified_ms: i64,
    /// Seitenmodus (für das Symbol der Kachel).
    pub pages: bool,
}

pub struct Recent {
    path: PathBuf,
    items: Vec<RecentEntry>,
    /// Schlüssel (`FileRef::to_key`) der Einträge, deren Datei nicht mehr existiert (nur im Speicher).
    missing: HashSet<String>,
}

impl Recent {
    /// Lädt `<dir>/recent.json` (fehlt sie oder ist sie kaputt: leere Liste).
    pub fn load(dir: &Path) -> Recent {
        let path = dir.join("recent.json");
        let items = crate::kv::read(&path)
            .ok()
            .and_then(|b| serde_json::from_slice::<Value>(&b).ok())
            .and_then(|v| v.as_array().cloned())
            .map(|a| {
                a.iter()
                    .filter_map(|e| {
                        Some(RecentEntry {
                            file: FileRef::from_key(e.get("file")?.as_str()?)?,
                            name: e.get("name")?.as_str()?.to_string(),
                            modified_ms: e.get("modified")?.as_i64()?,
                            pages: e.get("pages").and_then(Value::as_bool).unwrap_or(false),
                        })
                    })
                    .collect()
            })
            .unwrap_or_default();
        Recent { path, items, missing: HashSet::new() }
    }

    fn save(&self) {
        let v: Vec<Value> = self.items.iter().map(|e| json!({"file": e.file.to_key(), "name": e.name, "modified": e.modified_ms, "pages": e.pages})).collect();
        let _ = crate::kv::write(&self.path, Value::Array(v).to_string().as_bytes());
    }

    /// Fügt vorn ein (ersetzt einen vorhandenen Eintrag derselben Datei) und speichert.
    pub fn add(&mut self, e: RecentEntry) {
        self.items.retain(|x| x.file != e.file);
        self.items.insert(0, e);
        self.items.truncate(MAX_RECENT);
        self.save();
    }

    pub fn remove(&mut self, f: &FileRef) {
        self.items.retain(|x| &x.file != f);
        self.missing.remove(&f.to_key());
        self.save();
    }

    /// Prüft alle Einträge mit `exists` und merkt sich die fehlenden (für das Ausgrauen).
    pub fn check_missing(&mut self, exists: impl Fn(&FileRef) -> bool) {
        self.missing = self.items.iter().filter(|e| !exists(&e.file)).map(|e| e.file.to_key()).collect();
    }

    /// Datei nicht mehr vorhanden (Stand der letzten `check_missing`)?
    pub fn is_missing(&self, f: &FileRef) -> bool {
        self.missing.contains(&f.to_key())
    }

    /// Einträge, neueste zuerst.
    pub fn list(&self) -> &[RecentEntry] {
        &self.items
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn roundtrip() {
        let dir = std::env::temp_dir().join(format!("owndraw-rc-{}", owndraw_core::store::random_hex(6)));
        let mut r = Recent::load(&dir);
        let a = FileRef::Path(PathBuf::from("/x/a.owndraw"));
        let b = FileRef::Path(PathBuf::from("/x/b.owndraw"));
        r.add(RecentEntry { file: a.clone(), name: "A".into(), modified_ms: 1, pages: false });
        r.add(RecentEntry { file: b.clone(), name: "B".into(), modified_ms: 2, pages: true });
        r.add(RecentEntry { file: a.clone(), name: "A2".into(), modified_ms: 3, pages: false });
        let r2 = Recent::load(&dir);
        assert_eq!(r2.list().len(), 2);
        assert_eq!(r2.list()[0].name, "A2");
        let mut r2 = r2;
        r2.remove(&a);
        assert_eq!(Recent::load(&dir).list().len(), 1);
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn fehlende_dateien_und_kaputte_json() {
        let dir = std::env::temp_dir().join(format!("owndraw-rc2-{}", owndraw_core::store::random_hex(6)));
        std::fs::create_dir_all(&dir).unwrap();
        let da = dir.join("da.owndraw");
        std::fs::write(&da, b"x").unwrap();
        let weg = FileRef::Path(dir.join("weg.owndraw"));
        let da = FileRef::Path(da);
        let mut r = Recent::load(&dir);
        r.add(RecentEntry { file: da.clone(), name: "da".into(), modified_ms: 1, pages: false });
        r.add(RecentEntry { file: weg.clone(), name: "weg".into(), modified_ms: 2, pages: true });
        r.check_missing(|f| matches!(f, FileRef::Path(p) if p.exists()));
        assert!(r.is_missing(&weg) && !r.is_missing(&da));
        r.remove(&weg);
        assert!(!r.is_missing(&weg));
        // Höchstzahl und kaputte Datei
        for i in 0..(MAX_RECENT + 5) {
            r.add(RecentEntry { file: FileRef::Uri(format!("content://x/{i}")), name: i.to_string(), modified_ms: i as i64, pages: false });
        }
        assert_eq!(Recent::load(&dir).list().len(), MAX_RECENT);
        std::fs::write(dir.join("recent.json"), b"{kaputt").unwrap();
        assert!(Recent::load(&dir).list().is_empty());
        let _ = std::fs::remove_dir_all(dir);
    }
}
