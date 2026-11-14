//! App-Einstellungen (WP0: Typen; WP9: Persistenz `settings.json` und Einstellungsseite).

use crate::platform::{FileRef, Platform};
use serde::{Deserialize, Serialize};
use std::io;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, Default)]
pub enum Theme {
    #[default]
    System,
    Light,
    Dark,
}

/// Position einer Leiste.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum BarPos {
    Top,
    Bottom,
    Left,
    Right,
}

/// Wo neue Boards gespeichert werden.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, Default)]
pub enum SaveChoice {
    /// Beim Anlegen fragen (Standard-Ort oder selbst wählen).
    #[default]
    Ask,
    /// Immer Standard-Ort.
    AlwaysDefault,
}

/// Update-Modus (WP11).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, Default)]
pub enum UpdateMode {
    /// Fertiges Release-Paket laden (Standard).
    #[default]
    Package,
    /// Quell-Tarball laden und mit cargo bauen (nur Desktop).
    Source,
}

/// Werkzeug-Kennungen der Hauptleiste (Reihenfolge = Anzeige-Reihenfolge).
/// Bekannt: "pen", "marker", "eraser", "select", "shape", "note", "text", "image", "undo", "redo", "layers", "pages".
pub type ToolId = String;

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Settings {
    pub theme: Theme,
    /// Akzentfarbe ARGB.
    pub accent: u32,
    /// UI-Größe in Prozent (75–200), multipliziert mit System-DPI.
    pub ui_scale: u32,
    /// Position der Werkzeugleiste; None = automatisch nach Formfaktor.
    pub toolbar_pos: Option<BarPos>,
    /// Position der Optionsleiste; None = automatisch.
    pub options_pos: Option<BarPos>,
    /// Sichtbare Werkzeuge in Reihenfolge.
    pub tools: Vec<ToolId>,
    pub save_choice: SaveChoice,
    /// Standard-Ort (FileRef::to_key); None = Plattform-Vorgabe.
    pub default_dir: Option<String>,
    /// Xournal++: Autosave direkt ins .xopp statt .owndraw daneben.
    pub xopp_autosave_direct: bool,
    pub update_mode: UpdateMode,
    /// Beim Start nach Updates suchen (nur Hinweis, installiert nie). Standard: aus.
    pub update_check_on_start: bool,
    /// Übersprungene Version.
    pub update_skip: Option<String>,
    /// GitHub-Repo "owner/name" für Updates.
    pub update_repo: String,
    /// Unbekannte/zusätzliche Werte anderer Module (z. B. Werkzeug-Vorgaben aus prefs.txt).
    pub extra: serde_json::Map<String, serde_json::Value>,
}

/// Alle Werkzeuge, die die Leiste kann (Reihenfolge = Vorgabe beim Anzeigen ausgeblendeter Werkzeuge im Bearbeiten-Modus).
pub const ALL_TOOLS: [&str; 7] = ["pen", "marker", "eraser", "select", "shape", "text", "image"];

/// Auswahl der Akzentfarben (ARGB) in den Einstellungen.
pub const ACCENTS: [(&str, u32); 8] = [
    ("Indigo", 0xFF7C8CFF),
    ("Blau", 0xFF3E8EF7),
    ("Türkis", 0xFF2BB5B0),
    ("Grün", 0xFF30A46C),
    ("Gelb", 0xFFF5B800),
    ("Orange", 0xFFF76B15),
    ("Rot", 0xFFE5484D),
    ("Pink", 0xFFE93D82),
];

/// Dateiname der Einstellungen im Konfigurationsordner.
pub const FILE_NAME: &str = "settings.json";

pub const DEFAULT_TOOLS: [&str; 7] = ["pen", "marker", "eraser", "select", "shape", "text", "image"];

impl Default for Settings {
    fn default() -> Self {
        Settings {
            theme: Theme::System,
            accent: 0xFF7C8CFF,
            ui_scale: 100,
            toolbar_pos: None,
            options_pos: None,
            tools: DEFAULT_TOOLS.iter().map(|s| s.to_string()).collect(),
            save_choice: SaveChoice::Ask,
            default_dir: None,
            xopp_autosave_direct: false,
            update_mode: UpdateMode::Package,
            update_check_on_start: false,
            update_skip: None,
            update_repo: "crafttixlp/owndraw".into(),
            extra: Default::default(),
        }
    }
}

impl Settings {
    /// Begrenzt Werte auf gültige Bereiche (nach dem Laden).
    pub fn sanitize(&mut self) {
        self.ui_scale = (((self.ui_scale.clamp(75, 200) + 2) / 5) * 5).clamp(75, 200);
        self.accent |= 0xFF00_0000;
        let mut tools: Vec<ToolId> = Vec::new();
        for id in &self.tools {
            let id = if id == "note" { "text" } else { id.as_str() };
            if ALL_TOOLS.contains(&id) && !tools.iter().any(|t| t == id) {
                tools.push(id.to_string());
            }
        }
        if tools.is_empty() {
            tools = DEFAULT_TOOLS.iter().map(|s| s.to_string()).collect();
        }
        self.tools = tools;
        if self.update_repo.trim().is_empty() {
            self.update_repo = Settings::default().update_repo;
        }
    }

    /// Liest Einstellungen aus JSON-Bytes (tolerant gegenüber unbekannten/fehlenden Feldern) und begrenzt die Werte.
    pub fn parse(bytes: &[u8]) -> Result<Settings, serde_json::Error> {
        let mut s: Settings = serde_json::from_slice(bytes)?;
        s.sanitize();
        Ok(s)
    }

    /// Lädt `settings.json` aus dem Konfigurationsordner. Fehlt die Datei: Vorgaben. Ist sie kaputt: Vorgaben,
    /// die Datei wird als `settings.json.bak` gesichert.
    pub fn load(platform: &dyn Platform) -> Settings {
        let path = FileRef::Path(platform.config_dir().join(FILE_NAME));
        let Ok(bytes) = platform.read(&path) else { return Settings::default() };
        match Settings::parse(&bytes) {
            Ok(s) => s,
            Err(_) => {
                let bak = FileRef::Path(platform.config_dir().join(format!("{FILE_NAME}.bak")));
                let _ = platform.write_atomic(&bak, &bytes);
                Settings::default()
            }
        }
    }

    /// Speichert atomar als `settings.json` im Konfigurationsordner.
    pub fn save(&self, platform: &dyn Platform) -> io::Result<()> {
        let path = FileRef::Path(platform.config_dir().join(FILE_NAME));
        let bytes = serde_json::to_vec_pretty(self).map_err(io::Error::other)?;
        platform.write_atomic(&path, &bytes)
    }

    /// Setzt alles auf die Vorgaben zurück.
    pub fn reset(&mut self) {
        *self = Settings::default();
    }
}

/// Eintrag der Werkzeugliste im Bearbeiten-Modus: Kennung + sichtbar?
pub type ToolEntry = (ToolId, bool);

/// Vollständige Liste für den Bearbeiten-Modus: sichtbare in Reihenfolge, danach die ausgeblendeten.
pub fn edit_list(visible: &[ToolId]) -> Vec<ToolEntry> {
    let mut v: Vec<ToolEntry> = visible.iter().filter(|t| ALL_TOOLS.contains(&t.as_str())).map(|t| (t.clone(), true)).collect();
    for id in ALL_TOOLS {
        if !v.iter().any(|(t, _)| t == id) {
            v.push((id.to_string(), false));
        }
    }
    v
}

/// Sichtbare Kennungen aus der Bearbeiten-Liste.
pub fn tools_from_edit(list: &[ToolEntry]) -> Vec<ToolId> {
    list.iter().filter(|(_, on)| *on).map(|(t, _)| t.clone()).collect()
}

/// Verschiebt den Eintrag `from` an die Stelle `to` (Index in der Liste vor dem Entfernen).
pub fn reorder<T>(v: &mut Vec<T>, from: usize, to: usize) {
    if from >= v.len() || from == to {
        return;
    }
    let to = to.min(v.len() - 1);
    let it = v.remove(from);
    v.insert(to, it);
}

/// Schaltet die Sichtbarkeit von Eintrag `i` um. Das letzte sichtbare Werkzeug bleibt sichtbar (Rückgabe false).
pub fn toggle_visible(list: &mut [ToolEntry], i: usize) -> bool {
    let Some(on) = list.get(i).map(|e| e.1) else { return false };
    if on && list.iter().filter(|e| e.1).count() <= 1 {
        return false;
    }
    list[i].1 = !on;
    true
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::platform::stub::StubPlatform;

    #[test]
    fn parse_tolerant_und_begrenzt() {
        let s = Settings::parse(br#"{"ui_scale": 999, "unbekannt": 1, "tools": ["x","pen","pen","note"], "accent": 5}"#).unwrap();
        assert_eq!(s.ui_scale, 200);
        assert_eq!(s.tools, vec!["pen", "text"]);
        assert_eq!(s.accent >> 24, 0xFF);
        assert_eq!(Settings::parse(br#"{"ui_scale": 1}"#).unwrap().ui_scale, 75);
        assert_eq!(Settings::parse(br#"{"ui_scale": 103}"#).unwrap().ui_scale, 105);
        assert_eq!(Settings::parse(br#"{"tools": []}"#).unwrap().tools.len(), DEFAULT_TOOLS.len());
        assert_eq!(Settings::parse(b"{}").unwrap(), Settings::default());
        assert!(Settings::parse(b"{kaputt").is_err());
    }

    #[test]
    fn reset_setzt_vorgaben() {
        let mut s = Settings { ui_scale: 150, theme: Theme::Dark, ..Settings::default() };
        s.reset();
        assert_eq!(s, Settings::default());
    }

    #[test]
    fn reorder_logik() {
        let mut v = vec![1, 2, 3, 4];
        reorder(&mut v, 0, 2);
        assert_eq!(v, vec![2, 3, 1, 4]);
        reorder(&mut v, 3, 0);
        assert_eq!(v, vec![4, 2, 3, 1]);
        reorder(&mut v, 1, 99);
        assert_eq!(v, vec![4, 3, 1, 2]);
        reorder(&mut v, 9, 0);
        assert_eq!(v, vec![4, 3, 1, 2]);
    }

    #[test]
    fn mindestens_ein_werkzeug() {
        let mut l = edit_list(&["pen".to_string(), "eraser".to_string()]);
        assert_eq!(l.len(), ALL_TOOLS.len());
        assert!(toggle_visible(&mut l, 0));
        assert_eq!(tools_from_edit(&l), vec!["eraser"]);
        assert!(!toggle_visible(&mut l, 1), "letztes sichtbares bleibt");
        assert!(toggle_visible(&mut l, 0));
        assert_eq!(tools_from_edit(&l), vec!["pen", "eraser"]);
    }

    /// Alle Plattform-Tests in einem Test, weil `OWNDRAW_DATA` prozessweit gilt.
    #[test]
    fn ueberlebt_neustart_und_kaputte_datei() {
        let dir = std::env::temp_dir().join(format!("owndraw-wp9-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::env::set_var("OWNDRAW_DATA", &dir);
        let p = StubPlatform::new();
        // fehlt: Vorgaben
        assert_eq!(Settings::load(&p), Settings::default());
        // Round-Trip
        let mut s = Settings { theme: Theme::Dark, accent: ACCENTS[3].1, ui_scale: 130, toolbar_pos: Some(BarPos::Right), save_choice: SaveChoice::AlwaysDefault, default_dir: Some("/tmp/x".into()), xopp_autosave_direct: true, tools: vec!["eraser".into(), "pen".into()], ..Settings::default() };
        s.extra.insert("w".into(), serde_json::json!(5.0));
        s.save(&p).unwrap();
        let neu = Settings::load(&StubPlatform::new());
        assert_eq!(neu, s);
        // kaputt: Vorgaben + Sicherung
        let file = p.config_dir().join(FILE_NAME);
        std::fs::write(&file, b"{nicht json").unwrap();
        assert_eq!(Settings::load(&p), Settings::default());
        assert_eq!(std::fs::read(p.config_dir().join("settings.json.bak")).unwrap(), b"{nicht json");
        let _ = std::fs::remove_dir_all(&dir);
    }
}
