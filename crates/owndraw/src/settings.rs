//! App-Einstellungen (WP0: Typen; WP9: Persistenz `settings.json` und Einstellungsseite).

use crate::platform::{FileRef, Platform};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::io;

/// Vorgabe für [`Settings::server_url`].
pub const DEFAULT_SERVER: &str = "https://owndraw.thepepelex.at";

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

/// Andock-Zustand einer verschobenen Kachel: `edge` = angedockt an diesen Rand der
/// Zeichenfläche (None = frei schwebend), `vert` = Symbole senkrecht angeordnet.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Dock {
    pub edge: Option<BarPos>,
    pub vert: bool,
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

/// Einrasten von Winkeln (Formerkennung und Griffe).
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Snapping {
    /// Erkannte Linien/Pfeile auf das Winkelraster drehen.
    pub lines: bool,
    /// Winkelraster für erkannte Linien/Pfeile (Grad).
    pub line_step: u32,
    /// Größte Abweichung zum Einrasten (Grad, 0–15).
    pub line_tol: u32,
    /// Erkannte Rechtecke/Vielecke gerade ausrichten.
    pub shapes: bool,
    /// Pfeil-Endpunkte und Drehen einer Auswahl rasten ein.
    pub handles: bool,
    /// Winkelraster der Griffe (Grad).
    pub handle_step: u32,
    /// Größte Abweichung der Griffe (Grad, 0–15).
    pub handle_tol: u32,
    /// Taste, die beim Gedrückthalten das Einrasten umkehrt (Desktop).
    pub key: SnapKey,
}

/// Taste zum vorübergehenden Umkehren des Einrastens.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, Default)]
pub enum SnapKey {
    /// Keine Taste.
    Off,
    /// Umschalt (Shift).
    #[default]
    Shift,
    /// Alt.
    Alt,
    /// Strg/Cmd.
    Ctrl,
}

impl SnapKey {
    pub const ALL: [SnapKey; 4] = [SnapKey::Shift, SnapKey::Alt, SnapKey::Ctrl, SnapKey::Off];

    pub fn label(self) -> &'static str {
        match self {
            SnapKey::Off => "Keine",
            SnapKey::Shift => "Umschalt",
            SnapKey::Alt => "Alt",
            SnapKey::Ctrl => "Strg",
        }
    }

    /// Ist die Taste gerade gedrückt?
    pub fn held(self, m: &egui::Modifiers) -> bool {
        match self {
            SnapKey::Off => false,
            SnapKey::Shift => m.shift,
            SnapKey::Alt => m.alt,
            SnapKey::Ctrl => m.ctrl || m.command,
        }
    }
}

/// Wählbare Winkelraster (Grad).
pub const SNAP_STEPS: [u32; 5] = [5, 15, 30, 45, 90];

impl Default for Snapping {
    fn default() -> Self {
        Snapping { lines: true, line_step: 45, line_tol: 6, shapes: true, handles: true, handle_step: 15, handle_tol: 4, key: SnapKey::Shift }
    }
}

/// Einstellungen des Werkzeugs „Rechnen“ (entsprechen `owndraw_core::calc::Opts`).
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct CalcPrefs {
    /// „Rechnen erlauben“ (aus = keine Ergebnisse, z. B. für Prüfungen).
    pub on: bool,
    /// Winkel in Grad (sonst Bogenmaß).
    pub deg: bool,
    /// Gültige Stellen (2–10).
    pub digits: u8,
    /// Dezimalkomma statt Punkt.
    pub comma: bool,
    /// Rechenweg anzeigen.
    pub steps: bool,
}

impl Default for CalcPrefs {
    fn default() -> Self {
        CalcPrefs { on: true, deg: true, digits: 4, comma: true, steps: false }
    }
}

impl CalcPrefs {
    pub fn to_core(self) -> owndraw_core::calc::Opts {
        owndraw_core::calc::Opts { on: self.on, deg: self.deg, digits: self.digits.clamp(2, 10), comma: self.comma, steps: self.steps }
    }
}

impl Snapping {
    pub fn to_core(self) -> owndraw_core::recognizer::SnapCfg {
        owndraw_core::recognizer::SnapCfg {
            lines: self.lines,
            line_step: self.line_step as f32,
            line_tol: self.line_tol as f32,
            shapes: self.shapes,
            handles: self.handles,
            handle_step: self.handle_step as f32,
            handle_tol: self.handle_tol as f32,
        }
    }

    fn sanitize(&mut self) {
        let d = Snapping::default();
        if !SNAP_STEPS.contains(&self.line_step) {
            self.line_step = d.line_step;
        }
        if !SNAP_STEPS.contains(&self.handle_step) {
            self.handle_step = d.handle_step;
        }
        self.line_tol = self.line_tol.min(15);
        self.handle_tol = self.handle_tol.min(15);
    }
}

/// Werkzeug-Kennung (siehe `ui::editor::tools::TOOLS`).
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
    /// Vom Nutzer verschobene Kacheln: "<id>@<Formfaktor>" -> Anteil (0..1) der linken oberen Ecke
    /// bezogen auf (Zeichenfläche - Kachelgröße).
    pub tile_pos: BTreeMap<String, [f32; 2]>,
    /// Andock-Zustand der Kacheln (gleiche Schlüssel wie `tile_pos`); fehlt = Vorgabe.
    pub tile_dock: BTreeMap<String, Dock>,
    /// Alte Einstellung (bis 4.24): sichtbare Werkzeuge in Reihenfolge. Wird beim Laden nach `tool_on` und
    /// `tool_order` übernommen und nicht mehr geschrieben.
    #[serde(rename = "tools", skip_serializing)]
    pub legacy_tools: Option<Vec<ToolId>>,
    /// Werkzeuge an/aus, nur Abweichungen vom Standard der Registry (`ui::editor::tools`).
    pub tool_on: BTreeMap<ToolId, bool>,
    /// Reihenfolge der Werkzeugleiste; leer = Standard. Fehlende Werkzeuge folgen in Registry-Reihenfolge.
    pub tool_order: Vec<ToolId>,
    /// Tastenkürzel: Aktion → Kürzel (`Ctrl+Shift+Z`), nur Abweichungen vom Standard; leer = kein Kürzel
    /// (`ui::editor::keys`).
    pub keys: BTreeMap<String, Vec<String>>,
    pub save_choice: SaveChoice,
    /// Standard-Ort (FileRef::to_key); None = Plattform-Vorgabe.
    pub default_dir: Option<String>,
    /// Xournal++: Autosave direkt ins .xopp statt .owndraw daneben.
    pub xopp_autosave_direct: bool,
    /// Mit Lizenz (Web-App, Desktop): neue Boards standardmäßig auf dem Server (true) oder nur im Browser anlegen.
    pub new_on_server: bool,
    /// Desktop: Adresse des OwnDraw-Servers für Konto, Server-Boards und Live-Zusammenarbeit.
    pub server_url: String,
    pub update_mode: UpdateMode,
    /// Beim Start nach Updates suchen (nur Hinweis, installiert nie). Standard: aus.
    pub update_check_on_start: bool,
    /// Übersprungene Version.
    pub update_skip: Option<String>,
    /// GitHub-Repo "owner/name" für Updates.
    pub update_repo: String,
    /// Einrasten von Winkeln.
    pub snap: Snapping,
    /// Werkzeug „Rechnen“: Optionen und Schalter „Rechnen erlauben“.
    pub calc: CalcPrefs,
    /// Unbekannte/zusätzliche Werte anderer Module (z. B. Werkzeug-Vorgaben aus prefs.txt).
    pub extra: serde_json::Map<String, serde_json::Value>,
}

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

impl Default for Settings {
    fn default() -> Self {
        Settings {
            theme: Theme::System,
            accent: 0xFF7C8CFF,
            ui_scale: 100,
            toolbar_pos: None,
            options_pos: None,
            tile_pos: BTreeMap::new(),
            tile_dock: BTreeMap::new(),
            legacy_tools: None,
            tool_on: BTreeMap::new(),
            tool_order: Vec::new(),
            keys: BTreeMap::new(),
            save_choice: SaveChoice::Ask,
            default_dir: None,
            xopp_autosave_direct: false,
            new_on_server: true,
            server_url: DEFAULT_SERVER.into(),
            update_mode: UpdateMode::Package,
            update_check_on_start: false,
            update_skip: None,
            update_repo: "crafttixlp/ownDraw".into(),
            snap: Snapping::default(),
            calc: CalcPrefs::default(),
            extra: Default::default(),
        }
    }
}

impl Settings {
    /// Begrenzt Werte auf gültige Bereiche (nach dem Laden).
    pub fn sanitize(&mut self) {
        self.ui_scale = (((self.ui_scale.clamp(75, 200) + 2) / 5) * 5).clamp(75, 200);
        self.accent |= 0xFF00_0000;
        if let Some(old) = self.legacy_tools.take() {
            crate::ui::editor::tools::migrate_legacy(self, &old);
        }
        crate::ui::editor::tools::sanitize(self);
        crate::ui::editor::keys::sanitize(self);
        self.tile_pos.retain(|_, f| f.iter().all(|v| v.is_finite()));
        for f in self.tile_pos.values_mut() {
            *f = [f[0].clamp(0.0, 1.0), f[1].clamp(0.0, 1.0)];
        }
        // Seitlich angedockt = immer senkrecht, oben/unten = waagerecht
        for d in self.tile_dock.values_mut() {
            if let Some(e) = d.edge {
                d.vert = matches!(e, BarPos::Left | BarPos::Right);
            }
        }
        self.snap.sanitize();
        self.calc.digits = self.calc.digits.clamp(2, 10);
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

/// Verschiebt den Eintrag `from` an die Stelle `to` (Index in der Liste vor dem Entfernen).
pub fn reorder<T>(v: &mut Vec<T>, from: usize, to: usize) {
    if from >= v.len() || from == to {
        return;
    }
    let to = to.min(v.len() - 1);
    let it = v.remove(from);
    v.insert(to, it);
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::platform::stub::StubPlatform;

    #[test]
    fn parse_tolerant_und_begrenzt() {
        let s = Settings::parse(br#"{"ui_scale": 999, "unbekannt": 1, "tool_order": ["x","image","image"], "tool_on": {"pen": true, "marker": false}, "accent": 5}"#).unwrap();
        assert_eq!(s.ui_scale, 200);
        assert_eq!(s.tool_order, vec!["image"]);
        assert_eq!(s.tool_on.len(), 1, "Standardwerte werden nicht gespeichert");
        assert_eq!(s.accent >> 24, 0xFF);
        assert_eq!(Settings::parse(br#"{"ui_scale": 1}"#).unwrap().ui_scale, 75);
        assert_eq!(Settings::parse(br#"{"ui_scale": 103}"#).unwrap().ui_scale, 105);
        assert_eq!(Settings::parse(b"{}").unwrap(), Settings::default());
        assert!(Settings::parse(b"{kaputt").is_err());
    }

    #[test]
    fn tile_pos_roundtrip_und_altes_json() {
        let alt = Settings::parse(br#"{"ui_scale": 100}"#).unwrap();
        assert!(alt.tile_pos.is_empty());
        let mut s = Settings::default();
        s.tile_pos.insert("bar-tools@Expanded".into(), [0.25, 0.5]);
        let bytes = serde_json::to_vec(&s).unwrap();
        let neu = Settings::parse(&bytes).unwrap();
        assert_eq!(neu.tile_pos.get("bar-tools@Expanded"), Some(&[0.25, 0.5]));
        let k = Settings::parse(br#"{"tile_pos": {"a": [2.0, -1.0]}}"#).unwrap();
        assert_eq!(k.tile_pos["a"], [1.0, 0.0]);
        let mut s = neu;
        s.reset();
        assert!(s.tile_pos.is_empty());
        assert!(Settings::parse(br#"{"tile_dock": {"bar-tools@Expanded": {"edge": "Left", "vert": false}}}"#).unwrap().tile_dock["bar-tools@Expanded"] == Dock { edge: Some(BarPos::Left), vert: true });
    }

    #[test]
    fn einrasten_alt_und_begrenzt() {
        let s = Settings::parse(b"{}").unwrap();
        assert_eq!(s.snap, Snapping::default());
        let s = Settings::parse(br#"{"snap":{"line_step":7,"line_tol":99,"handles":false}}"#).unwrap();
        assert_eq!((s.snap.line_step, s.snap.line_tol, s.snap.handles, s.snap.handle_step), (45, 15, false, 15));
        assert_eq!(s.snap.key, SnapKey::Shift);
        let s = Settings::parse(br#"{"snap":{"key":"Alt"}}"#).unwrap();
        assert_eq!(s.snap.key, SnapKey::Alt);
        let m = egui::Modifiers { alt: true, ..Default::default() };
        assert!(SnapKey::Alt.held(&m) && !SnapKey::Shift.held(&m) && !SnapKey::Off.held(&m));
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

    /// Eigener Datenordner über `StubPlatform::with_base` (nicht `OWNDRAW_DATA` setzen: die Variable gilt
    /// prozessweit und würde parallel laufende Tests, z. B. `clip::tests`, in einen gelöschten Ordner umleiten).
    #[test]
    fn ueberlebt_neustart_und_kaputte_datei() {
        let dir = std::env::temp_dir().join(format!("owndraw-wp9-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let p = StubPlatform::with_base(dir.clone());
        // fehlt: Vorgaben
        assert_eq!(Settings::load(&p), Settings::default());
        // Round-Trip
        let mut s = Settings { theme: Theme::Dark, accent: ACCENTS[3].1, ui_scale: 130, toolbar_pos: Some(BarPos::Right), save_choice: SaveChoice::AlwaysDefault, default_dir: Some("/tmp/x".into()), xopp_autosave_direct: true, tool_order: vec!["eraser".into(), "pen".into()], ..Settings::default() };
        s.tool_on.insert("marker".into(), false);
        s.keys.insert("undo".into(), vec!["Ctrl+U".into()]);
        s.extra.insert("w".into(), serde_json::json!(5.0));
        s.save(&p).unwrap();
        let neu = Settings::load(&StubPlatform::with_base(dir.clone()));
        assert_eq!(neu, s);
        // kaputt: Vorgaben + Sicherung
        let file = p.config_dir().join(FILE_NAME);
        std::fs::write(&file, b"{nicht json").unwrap();
        assert_eq!(Settings::load(&p), Settings::default());
        assert_eq!(std::fs::read(p.config_dir().join("settings.json.bak")).unwrap(), b"{nicht json");
        let _ = std::fs::remove_dir_all(&dir);
    }
}
