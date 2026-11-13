//! App-Einstellungen (WP0: Typen; WP9: Persistenz `settings.json` und Einstellungsseite).

use serde::{Deserialize, Serialize};

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
