//! Einfache Einstellungsdatei (Schluessel=Wert), Gegenstueck zu SharedPreferences "board".

use std::collections::BTreeMap;
use std::path::PathBuf;

pub struct Prefs {
    path: PathBuf,
    map: BTreeMap<String, String>,
}

impl Prefs {
    pub fn load(path: PathBuf) -> Prefs {
        let mut map = BTreeMap::new();
        if let Ok(s) = std::fs::read_to_string(&path) {
            for l in s.lines() {
                if let Some((k, v)) = l.split_once('=') {
                    if !k.starts_with('#') {
                        map.insert(k.trim().to_string(), v.trim().to_string());
                    }
                }
            }
        }
        Prefs { path, map }
    }

    pub fn int(&self, k: &str, def: i32) -> i32 {
        self.map.get(k).and_then(|v| v.parse().ok()).unwrap_or(def)
    }
    pub fn float(&self, k: &str, def: f32) -> f32 {
        self.map.get(k).and_then(|v| v.parse().ok()).unwrap_or(def)
    }
    pub fn str(&self, k: &str) -> Option<String> {
        self.map.get(k).cloned()
    }
    pub fn set(&mut self, k: &str, v: impl ToString) {
        self.map.insert(k.to_string(), v.to_string().replace('\n', " "));
    }

    pub fn save(&self) {
        let mut s = String::from("# OwnDraw-Einstellungen\n");
        for (k, v) in &self.map {
            s.push_str(&format!("{k}={v}\n"));
        }
        if let Some(d) = self.path.parent() {
            let _ = std::fs::create_dir_all(d);
        }
        let _ = owndraw_core::store::write_atomic(&self.path, s.as_bytes());
    }
}
