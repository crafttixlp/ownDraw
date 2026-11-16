//! Frei belegbare Tastenkürzel: Aktions-Registry und Belegung.
//!
//! Jede Aktion hat eine stabile Kennung, einen Namen und ihre Standard-Kürzel. Die Werkzeug-Aktionen
//! (`tool.<id>`) entstehen aus der Werkzeug-Registry (`tools::TOOLS`, Feld `key`), neue Werkzeuge sind damit
//! automatisch belegbar. In `Settings::keys` steht nur, was vom Standard abweicht (Aktion → Kürzel; leere
//! Liste = kein Kürzel). Die Einrast-Taste ist eine Aktion der Art [`Kind::Modifier`] und nutzt weiterhin
//! `Settings::snap.key`.

use super::tools;
use super::Tool;
use crate::settings::Settings;
use egui::{Event, InputState, Key, Modifiers};
use std::sync::OnceLock;

/// Taste mit Umschaltern. Strg steht auch für Cmd (macOS).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct Chord {
    pub key: Key,
    pub ctrl: bool,
    pub alt: bool,
    pub shift: bool,
}

impl Chord {
    pub const fn new(key: Key) -> Chord {
        Chord { key, ctrl: false, alt: false, shift: false }
    }

    pub fn from_event(key: Key, m: &Modifiers) -> Chord {
        Chord { key, ctrl: m.ctrl || m.command || m.mac_cmd, alt: m.alt, shift: m.shift }
    }

    /// Liest die gespeicherte Form, z. B. "Ctrl+Shift+Z", "Plus", "F1".
    pub fn parse(s: &str) -> Option<Chord> {
        let parts: Vec<&str> = s.split('+').map(str::trim).collect();
        let (last, mods) = parts.split_last()?;
        let mut c = Chord::new(Key::from_name(last)?);
        for m in mods {
            match m.to_ascii_lowercase().as_str() {
                "ctrl" | "strg" | "cmd" => c.ctrl = true,
                "alt" => c.alt = true,
                "shift" | "umschalt" => c.shift = true,
                _ => return None,
            }
        }
        Some(c)
    }

    /// Gespeicherte Form (Gegenstück zu [`Chord::parse`]).
    pub fn id(self) -> String {
        let mut s = String::new();
        if self.ctrl {
            s.push_str("Ctrl+");
        }
        if self.alt {
            s.push_str("Alt+");
        }
        if self.shift {
            s.push_str("Shift+");
        }
        s.push_str(self.key.name());
        s
    }

    /// Anzeige, z. B. „Strg+Umschalt+Z“.
    pub fn label(self) -> String {
        let mut s = String::new();
        if self.ctrl {
            s.push_str("Strg+");
        }
        if self.alt {
            s.push_str("Alt+");
        }
        if self.shift {
            s.push_str("Umschalt+");
        }
        s.push_str(key_label(self.key));
        s
    }

    fn without_shift(self) -> Chord {
        Chord { shift: false, ..self }
    }
}

/// Deutscher Name einer Taste.
pub fn key_label(k: Key) -> &'static str {
    match k {
        Key::Delete => "Entf",
        Key::Backspace => "Rücktaste",
        Key::Escape => "Esc",
        Key::Space => "Leertaste",
        Key::Enter => "Enter",
        Key::Insert => "Einfg",
        Key::Home => "Pos1",
        Key::End => "Ende",
        Key::PageUp => "Bild auf",
        Key::PageDown => "Bild ab",
        Key::ArrowUp => "Pfeil hoch",
        Key::ArrowDown => "Pfeil runter",
        Key::ArrowLeft => "Pfeil links",
        Key::ArrowRight => "Pfeil rechts",
        _ => k.symbol_or_name(),
    }
}

/// Art einer Aktion.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    /// Einmal beim Drücken.
    Press,
    /// Solange gedrückt (Leertaste zum Verschieben).
    Hold,
    /// Umschalttaste, die beim Gedrückthalten etwas umkehrt (Einrasten); Belegung in `Settings::snap.key`.
    Modifier,
}

/// Wo die Aktion gilt. Gleiche Kürzel in verschiedenen Bereichen sind kein Konflikt.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Scope {
    /// Zeichenfläche.
    Editor,
    /// Während der Texteingabe.
    Text,
}

/// Eine Aktion (fest oder aus einem Werkzeug).
#[derive(Clone, Debug)]
pub struct Action {
    /// Stabile Kennung (`undo`, `tool.pen`, …).
    pub id: String,
    pub name: String,
    /// Gruppe auf der Einstellungsseite.
    pub group: &'static str,
    pub defaults: Vec<Chord>,
    /// Feuert auch bei gehaltener Taste (Tastenwiederholung).
    pub repeat: bool,
    pub kind: Kind,
    pub scope: Scope,
    /// Werkzeug-Aktion: nur aktiv, solange das Werkzeug eingeschaltet ist.
    pub tool: Option<Tool>,
}

struct Def {
    id: &'static str,
    name: &'static str,
    group: &'static str,
    keys: &'static [&'static str],
    repeat: bool,
    kind: Kind,
    scope: Scope,
}

const fn press(id: &'static str, name: &'static str, group: &'static str, keys: &'static [&'static str]) -> Def {
    Def { id, name, group, keys, repeat: false, kind: Kind::Press, scope: Scope::Editor }
}

const fn rep(d: Def) -> Def {
    Def { repeat: true, ..d }
}

/// Feste Aktionen (Werkzeug-Aktionen kommen aus der Werkzeug-Registry). Standard = Belegung bis 4.24.
const ACTIONS: &[Def] = &[
    rep(press("undo", "Rückgängig", "Bearbeiten", &["Ctrl+Z"])),
    rep(press("redo", "Wiederholen", "Bearbeiten", &["Ctrl+Y", "Ctrl+Shift+Z"])),
    press("copy", "Kopieren", "Bearbeiten", &["Ctrl+C"]),
    press("cut", "Ausschneiden", "Bearbeiten", &["Ctrl+X"]),
    press("paste", "Einfügen", "Bearbeiten", &["Ctrl+V"]),
    rep(press("duplicate", "Auswahl duplizieren", "Bearbeiten", &["Ctrl+D"])),
    press("delete", "Auswahl löschen", "Bearbeiten", &["Delete", "Backspace"]),
    press("group", "Auswahl gruppieren", "Bearbeiten", &["Ctrl+G"]),
    press("ungroup", "Gruppierung lösen", "Bearbeiten", &["Ctrl+Shift+G"]),
    press("save", "Speichern", "Datei", &["Ctrl+S"]),
    press("menu", "Menü öffnen", "Datei", &["Ctrl+E"]),
    press("help", "Anleitung & Tipps", "Datei", &["F1"]),
    press("cancel", "Menü/Optionen schließen", "Ansicht", &["Escape"]),
    press("fit", "Alles zeigen", "Ansicht", &["0"]),
    press("zoom_in", "Vergrößern", "Ansicht", &["Plus", "Equals"]),
    press("zoom_out", "Verkleinern", "Ansicht", &["Minus"]),
    Def { kind: Kind::Hold, ..press("pan", "Ansicht verschieben (halten + ziehen)", "Ansicht", &["Space"]) },
    Def { kind: Kind::Modifier, ..press("snap", "Einrasten umkehren (halten)", "Ansicht", &[]) },
    Def { scope: Scope::Text, ..press("text_done", "Texteingabe beenden", "Text", &["Ctrl+Enter", "Escape"]) },
];

/// Alle Aktionen: zuerst die Werkzeuge (Reihenfolge der Registry), dann die festen.
pub fn all() -> &'static [Action] {
    static A: OnceLock<Vec<Action>> = OnceLock::new();
    A.get_or_init(|| {
        let mut v: Vec<Action> = tools::TOOLS
            .iter()
            .map(|d| Action {
                id: tool_action(d.id),
                name: d.name.to_string(),
                group: "Werkzeuge",
                defaults: d.key.and_then(Chord::parse).into_iter().collect(),
                repeat: false,
                kind: Kind::Press,
                scope: Scope::Editor,
                tool: Some(d.tool),
            })
            .collect();
        for d in ACTIONS {
            v.push(Action {
                id: d.id.to_string(),
                name: d.name.to_string(),
                group: d.group,
                defaults: d.keys.iter().map(|k| Chord::parse(k).expect("Standard-Kürzel")).collect(),
                repeat: d.repeat,
                kind: d.kind,
                scope: d.scope,
                tool: None,
            });
        }
        v
    })
}

/// Kennung der Aktion eines Werkzeugs.
pub fn tool_action(tool_id: &str) -> String {
    format!("tool.{tool_id}")
}

pub fn find(id: &str) -> Option<&'static Action> {
    all().iter().find(|a| a.id == id)
}

/// Aktion gerade verfügbar (Werkzeug eingeschaltet)?
pub fn available(s: &Settings, a: &Action) -> bool {
    match a.tool {
        Some(t) => tools::enabled(s, tools::of(t).id),
        None => true,
    }
}

/// Aktuelle Belegung einer Aktion (ohne Rücksicht auf an/aus).
pub fn bindings(s: &Settings, a: &Action) -> Vec<Chord> {
    match s.keys.get(&a.id) {
        Some(v) => v.iter().filter_map(|k| Chord::parse(k)).collect(),
        None => a.defaults.clone(),
    }
}

/// Anzeige der Belegung (alle Kürzel), für die Einrast-Taste „Umschalt“ usw.
pub fn labels(s: &Settings, id: &str) -> Vec<String> {
    let Some(a) = find(id) else { return Vec::new() };
    if a.kind == Kind::Modifier {
        return match s.snap.key {
            crate::settings::SnapKey::Off => Vec::new(),
            k => vec![k.label().to_string()],
        };
    }
    bindings(s, a).into_iter().map(Chord::label).collect()
}

/// Erstes Kürzel für Tooltips ("" = keins oder Werkzeug aus).
pub fn label(s: &Settings, id: &str) -> String {
    match find(id) {
        Some(a) if available(s, a) => labels(s, id).into_iter().next().unwrap_or_default(),
        _ => String::new(),
    }
}

/// Aktion zur gedrückten Kombination (Art „Drücken“). Erst genau, dann ohne Umschalt (z. B. „+“ auf
/// US-Tastaturen, Umschalt+P wählt wie bisher den Stift).
pub fn lookup(s: &Settings, c: Chord, scope: Scope) -> Option<&'static Action> {
    let hit = |c: Chord| all().iter().find(|a| a.kind == Kind::Press && a.scope == scope && available(s, a) && bindings(s, a).contains(&c));
    hit(c).or_else(|| if c.shift { hit(c.without_shift()) } else { None })
}

/// Andere Aktion im selben Bereich, die `c` schon belegt (auch bei ausgeschaltetem Werkzeug).
pub fn conflict(s: &Settings, c: Chord, a: &Action) -> Option<&'static Action> {
    all().iter().find(|o| o.id != a.id && o.scope == a.scope && o.kind != Kind::Modifier && bindings(s, o).contains(&c))
}

/// Setzt die Belegung; gleich dem Standard = Eintrag entfernen.
pub fn set(s: &mut Settings, id: &str, list: Vec<Chord>) {
    let Some(a) = find(id) else { return };
    let mut uniq: Vec<Chord> = Vec::new();
    for c in list {
        if !uniq.contains(&c) {
            uniq.push(c);
        }
    }
    if uniq == a.defaults {
        s.keys.remove(id);
    } else {
        s.keys.insert(id.to_string(), uniq.into_iter().map(Chord::id).collect());
    }
}

/// Kürzel `slot` ersetzen (None = hinzufügen).
pub fn assign(s: &mut Settings, id: &str, slot: Option<usize>, c: Chord) {
    let Some(a) = find(id) else { return };
    let mut v = bindings(s, a);
    match slot {
        Some(i) if i < v.len() => v[i] = c,
        _ => v.push(c),
    }
    set(s, id, v);
}

/// Kürzel `slot` entfernen.
pub fn remove(s: &mut Settings, id: &str, slot: usize) {
    let Some(a) = find(id) else { return };
    let mut v = bindings(s, a);
    if slot < v.len() {
        v.remove(slot);
    }
    set(s, id, v);
}

/// Kürzel `c` bei Aktion `id` entfernen (Konflikt auflösen).
pub fn unbind(s: &mut Settings, id: &str, c: Chord) {
    let Some(a) = find(id) else { return };
    let v = bindings(s, a).into_iter().filter(|x| *x != c).collect();
    set(s, id, v);
}

pub fn reset(s: &mut Settings, id: &str) {
    s.keys.remove(id);
    if find(id).map(|a| a.kind == Kind::Modifier).unwrap_or(false) {
        s.snap.key = crate::settings::Snapping::default().key;
    }
}

pub fn reset_all(s: &mut Settings) {
    s.keys.clear();
    s.snap.key = crate::settings::Snapping::default().key;
}

/// Weicht die Belegung vom Standard ab?
pub fn is_custom(s: &Settings, a: &Action) -> bool {
    if a.kind == Kind::Modifier {
        return s.snap.key != crate::settings::Snapping::default().key;
    }
    s.keys.contains_key(&a.id)
}

/// Im Browser nicht belegbar (der Browser fängt sie ab: Tab schließen, neuer Tab, neu laden …).
pub fn web_blocked(c: Chord) -> bool {
    let k = c.key;
    if matches!(k, Key::F5 | Key::F11 | Key::F12) {
        return true;
    }
    if k == Key::Tab {
        return true;
    }
    if c.ctrl && !c.alt {
        if matches!(k, Key::W | Key::T | Key::N | Key::R | Key::L | Key::Q | Key::H | Key::J | Key::PageUp | Key::PageDown) {
            return true;
        }
        if c.shift && matches!(k, Key::I | Key::Delete) {
            return true;
        }
    }
    c.alt && !c.ctrl && matches!(k, Key::F4 | Key::ArrowLeft | Key::ArrowRight | Key::Home)
}

/// Grund, warum `c` nicht belegt werden kann (Browser), sonst None.
pub fn refuse(c: Chord) -> Option<String> {
    (crate::platform::WEB && web_blocked(c)).then(|| format!("{} gehört im Browser dem Browser selbst und lässt sich nicht belegen.", c.label()))
}

/// Halte-Aktion gerade gedrückt (Leertaste zum Verschieben)?
pub fn held(i: &InputState, s: &Settings, id: &str) -> bool {
    let Some(a) = find(id) else { return false };
    let m = i.modifiers;
    bindings(s, a).iter().any(|c| i.key_down(c.key) && (!c.ctrl || m.ctrl || m.command) && (!c.alt || m.alt) && (!c.shift || m.shift))
}

/// Wurde die Aktion in diesem Frame ausgelöst (ohne Tastenwiederholung)? Verbraucht nichts.
pub fn pressed(i: &InputState, s: &Settings, id: &str) -> bool {
    let Some(a) = find(id) else { return false };
    i.events.iter().any(|e| match e {
        Event::Key { key, pressed: true, repeat: false, modifiers, .. } => lookup(s, Chord::from_event(*key, modifiers), a.scope).map(|x| x.id == a.id).unwrap_or(false),
        _ => false,
    })
}

/// Wie [`pressed`], verbraucht aber das Tastenereignis.
pub fn consume(i: &mut InputState, s: &Settings, id: &str) -> bool {
    let Some(a) = find(id) else { return false };
    let pos = i.events.iter().position(|e| match e {
        Event::Key { key, pressed: true, modifiers, .. } => lookup(s, Chord::from_event(*key, modifiers), a.scope).map(|x| x.id == a.id).unwrap_or(false),
        _ => false,
    });
    match pos {
        Some(p) => {
            i.events.remove(p);
            true
        }
        None => false,
    }
}

/// Nach dem Laden: nur lesbare Kürzel, einheitliche Schreibweise, ohne Doppelte.
pub fn sanitize(s: &mut Settings) {
    for v in s.keys.values_mut() {
        let mut out: Vec<String> = Vec::new();
        for k in v.iter() {
            if let Some(c) = Chord::parse(k) {
                let id = c.id();
                if !out.contains(&id) {
                    out.push(id);
                }
            }
        }
        *v = out;
    }
    // gleich dem Standard: nicht speichern
    s.keys.retain(|id, v| match find(id) {
        Some(a) => v.iter().filter_map(|k| Chord::parse(k)).collect::<Vec<_>>() != a.defaults,
        None => true, // Aktion einer neueren Version behalten
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ch(s: &str) -> Chord {
        Chord::parse(s).unwrap()
    }

    fn hit(s: &Settings, c: &str) -> Option<String> {
        lookup(s, ch(c), Scope::Editor).map(|a| a.id.clone())
    }

    #[test]
    fn kombination_lesen_und_schreiben() {
        for k in ["Z", "Ctrl+Z", "Ctrl+Shift+Z", "Ctrl+Alt+Shift+F1", "Plus", "Equals", "Minus", "0", "Delete", "Space", "Escape"] {
            assert_eq!(ch(k).id(), k);
        }
        assert_eq!(ch("Strg+Umschalt+z"), ch("Ctrl+Shift+Z"));
        assert!(Chord::parse("Hyper+Z").is_none() && Chord::parse("").is_none() && Chord::parse("Ctrl+").is_none());
        assert_eq!(ch("Ctrl+Shift+Z").label(), "Strg+Umschalt+Z");
        assert_eq!(ch("Delete").label(), "Entf");
        assert_eq!(ch("Minus").label(), "−");
    }

    /// Standardbelegung = Belegung bis 4.24 (feste Tastenabfrage in `Editor::shortcuts`).
    #[test]
    fn standard_wie_bisher() {
        let s = Settings::default();
        let alt = [
            ("Ctrl+Z", "undo"),
            ("Ctrl+Y", "redo"),
            ("Ctrl+Shift+Z", "redo"),
            ("Ctrl+Shift+Y", "redo"),
            ("Ctrl+C", "copy"),
            ("Ctrl+X", "cut"),
            ("Ctrl+V", "paste"),
            ("Ctrl+D", "duplicate"),
            ("Ctrl+S", "save"),
            ("Ctrl+E", "menu"),
            ("F1", "help"),
            ("Delete", "delete"),
            ("Backspace", "delete"),
            ("Escape", "cancel"),
            ("P", "tool.pen"),
            ("Shift+P", "tool.pen"),
            ("M", "tool.marker"),
            ("E", "tool.eraser"),
            ("S", "tool.select"),
            ("R", "tool.shape"),
            ("T", "tool.text"),
            ("N", "tool.note"),
            ("I", "tool.image"),
            ("0", "fit"),
            ("Plus", "zoom_in"),
            ("Shift+Plus", "zoom_in"),
            ("Equals", "zoom_in"),
            ("Minus", "zoom_out"),
        ];
        for (c, id) in alt {
            assert_eq!(hit(&s, c).as_deref(), Some(id), "{c}");
        }
        for c in ["Ctrl+P", "Alt+P", "Ctrl+0", "Ctrl+Plus", "Q", "Space"] {
            assert_eq!(hit(&s, c), None, "{c}");
        }
        assert_eq!(lookup(&s, ch("Ctrl+Enter"), Scope::Text).map(|a| a.id.as_str()), Some("text_done"));
        assert_eq!(lookup(&s, ch("Escape"), Scope::Text).map(|a| a.id.as_str()), Some("text_done"));
        assert_eq!(labels(&s, "pan"), vec!["Leertaste"]);
        assert_eq!(labels(&s, "snap"), vec!["Umschalt"]);
        assert_eq!(label(&s, "undo"), "Strg+Z");
        // jede Aktion eindeutig
        for (i, a) in all().iter().enumerate() {
            assert!(all()[..i].iter().all(|b| b.id != a.id), "doppelt {}", a.id);
        }
    }

    #[test]
    fn abgeschaltetes_werkzeug_ohne_kuerzel() {
        let mut s = Settings::default();
        assert!(tools::set_enabled(&mut s, "marker", false));
        assert_eq!(hit(&s, "M"), None);
        assert_eq!(label(&s, "tool.marker"), "");
        assert_eq!(hit(&s, "P").as_deref(), Some("tool.pen"));
        // Belegung bleibt erhalten und gilt nach dem Einschalten wieder
        assert!(tools::set_enabled(&mut s, "marker", true));
        assert_eq!(hit(&s, "M").as_deref(), Some("tool.marker"));
    }

    #[test]
    fn konflikt_und_neu_belegen() {
        let mut s = Settings::default();
        let pen = find("tool.pen").unwrap();
        assert_eq!(conflict(&s, ch("Ctrl+D"), pen).map(|a| a.id.as_str()), Some("duplicate"));
        assert!(conflict(&s, ch("Ctrl+K"), pen).is_none());
        assert!(conflict(&s, ch("P"), pen).is_none(), "eigene Belegung ist kein Konflikt");
        // Escape: Editor und Texteingabe sind getrennt
        assert!(conflict(&s, ch("Escape"), find("text_done").unwrap()).is_none());
        // auch ein ausgeschaltetes Werkzeug meldet seinen Konflikt
        tools::set_enabled(&mut s, "image", false);
        assert_eq!(conflict(&s, ch("I"), pen).map(|a| a.id.as_str()), Some("tool.image"));
        // Konflikt auflösen: dort entfernen, hier zuweisen
        unbind(&mut s, "duplicate", ch("Ctrl+D"));
        assign(&mut s, "tool.pen", Some(0), ch("Ctrl+D"));
        assert_eq!(hit(&s, "Ctrl+D").as_deref(), Some("tool.pen"));
        assert_eq!(hit(&s, "P"), None);
        assert_eq!(labels(&s, "duplicate"), Vec::<String>::new());
        // Entf entfernt
        remove(&mut s, "delete", 1);
        assert_eq!(hit(&s, "Backspace"), None);
        assert_eq!(hit(&s, "Delete").as_deref(), Some("delete"));
    }

    #[test]
    fn zuruecksetzen() {
        let mut s = Settings::default();
        assign(&mut s, "undo", None, ch("Ctrl+U"));
        remove(&mut s, "fit", 0);
        s.snap.key = crate::settings::SnapKey::Alt;
        assert!(is_custom(&s, find("undo").unwrap()));
        reset(&mut s, "undo");
        assert!(!s.keys.contains_key("undo"));
        assert_eq!(hit(&s, "Ctrl+U"), None);
        assert!(s.keys.contains_key("fit"));
        reset_all(&mut s);
        assert_eq!(s, Settings::default());
        // Belegung gleich Standard wird nicht gespeichert
        assign(&mut s, "redo", Some(1), ch("Ctrl+Shift+Z"));
        assert!(s.keys.is_empty());
    }

    #[test]
    fn belegung_serialisieren() {
        let mut s = Settings::default();
        assign(&mut s, "tool.pen", Some(0), ch("Ctrl+Alt+P"));
        remove(&mut s, "fit", 0);
        let json = serde_json::to_string(&s).unwrap();
        assert!(json.contains(r#""keys":{"fit":[],"tool.pen":["Ctrl+Alt+P"]}"#), "{json}");
        let neu = Settings::parse(json.as_bytes()).unwrap();
        assert_eq!(neu, s);
        assert_eq!(hit(&neu, "Ctrl+Alt+P").as_deref(), Some("tool.pen"));
        assert_eq!(hit(&neu, "0"), None);
        // tolerant: unlesbares weg, Schreibweise vereinheitlicht, Standard nicht gespeichert, Unbekanntes bleibt
        let s = Settings::parse(br#"{"keys": {"undo": ["strg+z"], "redo": ["Kaputt+Q", "ctrl+q", "Ctrl+Q"], "zukunft": ["X"]}}"#).unwrap();
        assert!(!s.keys.contains_key("undo"));
        assert_eq!(s.keys["redo"], vec!["Ctrl+Q"]);
        assert!(s.keys.contains_key("zukunft"));
    }

    #[test]
    fn browser_sperren() {
        for c in ["Ctrl+W", "Ctrl+T", "Ctrl+N", "Ctrl+Tab", "Tab", "F5", "Ctrl+R", "Alt+F4"] {
            assert!(web_blocked(ch(c)), "{c}");
        }
        for c in ["Ctrl+Z", "P", "Ctrl+S", "Ctrl+D", "Alt+W"] {
            assert!(!web_blocked(ch(c)), "{c}");
        }
    }
}
