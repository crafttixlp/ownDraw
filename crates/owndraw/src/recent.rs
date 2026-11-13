//! Zuletzt-geöffnet-Liste (Hook für WP7).
//!
//! Vertrag: `Recent::load(dir)`, `add`, `remove`, `list`. Die Liste liegt als `recent.json` im Konfigurationsordner
//! (`Platform::config_dir`). Vorschaubilder stehen nicht hier, sondern in den Dateien (`session::read_thumb`).

use crate::platform::FileRef;
use serde_json::{json, Value};
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
}

impl Recent {
    /// Lädt `<dir>/recent.json` (fehlt sie oder ist sie kaputt: leere Liste).
    pub fn load(dir: &Path) -> Recent {
        let path = dir.join("recent.json");
        let items = std::fs::read(&path)
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
        Recent { path, items }
    }

    fn save(&self) {
        let v: Vec<Value> = self.items.iter().map(|e| json!({"file": e.file.to_key(), "name": e.name, "modified": e.modified_ms, "pages": e.pages})).collect();
        if let Some(d) = self.path.parent() {
            let _ = std::fs::create_dir_all(d);
        }
        let _ = std::fs::write(&self.path, Value::Array(v).to_string());
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
        self.save();
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
}
