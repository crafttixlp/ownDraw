//! Werkzeug-Registry: EINE Liste aller Werkzeuge mit Kennung, Name, Gruppe, Symbol, Beschreibung,
//! Standard an/aus, „Neu“-Markierung und Standard-Tastenkürzel.
//!
//! Daraus entstehen die Werkzeugleiste, „Werkzeuge anpassen“, die Einstellungsseite „Werkzeuge“, die
//! Tastenkürzel (`keys::all` legt für jedes Werkzeug eine Aktion `tool.<id>` an) und die Anleitung (Test).
//!
//! Neues Werkzeug:
//! 1. `Tool` (ui/editor/mod.rs) um eine Variante erweitern,
//! 2. hier in [`TOOLS`] eine Zeile `ToolDef::neu(Tool::X, "x", "Name", Group::…, ic::…, "Ein Satz.").key("K")`,
//! 3. Verhalten: `Editor::select_tool` (mod.rs), ggf. `Editor::active_tool`, Optionen (options.rs),
//! 4. Eintrag in `tutorial.json` (`tool:Name`, `action:tool.x`).
//!
//! Gespeichert wird nur, was vom Standard abweicht: `Settings::tool_on` (an/aus) und `Settings::tool_order`
//! (Reihenfolge der Leiste). Ein ausgeschaltetes Werkzeug verschwindet aus Leiste und Kürzeln; vorhandene
//! Objekte bleiben bearbeitbar (das Auswahl-Werkzeug ist davon unabhängig).

use super::Tool;
use crate::settings::Settings;
use owndraw_core::icons as ic;

/// Gruppe auf der Einstellungsseite „Werkzeuge“.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Group {
    Zeichnen,
    Formen,
    Ordnen,
    Extras,
}

impl Group {
    pub const ALL: [Group; 4] = [Group::Zeichnen, Group::Formen, Group::Ordnen, Group::Extras];

    pub fn label(self) -> &'static str {
        match self {
            Group::Zeichnen => "Zeichnen",
            Group::Formen => "Formen",
            Group::Ordnen => "Ordnen",
            Group::Extras => "Extras",
        }
    }
}

/// Ein Werkzeug der Registry.
#[derive(Clone, Copy, Debug)]
pub struct ToolDef {
    pub tool: Tool,
    /// Stabile Kennung (Einstellungen, Tastenkürzel `tool.<id>`); nie umbenennen.
    pub id: &'static str,
    /// Anzeigename (Leiste, Tooltip, Einstellungen, Anleitung `tool:<name>`).
    pub name: &'static str,
    /// Ein Satz Beschreibung (Einstellungsseite).
    pub desc: &'static str,
    pub group: Group,
    /// Symbol aus `owndraw_core::icons`.
    pub icon: i32,
    /// Standardmäßig eingeschaltet.
    pub default_on: bool,
    /// „Neu“-Markierung in den Einstellungen.
    pub new: bool,
    /// Standard-Tastenkürzel (Format von `keys::Chord::parse`, z. B. "P" oder "Ctrl+Shift+K").
    pub key: Option<&'static str>,
    /// Werkzeug wird über ein anderes Werkzeug erreicht (Haftnotiz über Text). Eigener Knopf in der Leiste nur,
    /// wenn das andere Werkzeug ausgeschaltet ist.
    pub parent: Option<&'static str>,
}

impl ToolDef {
    /// Vorhandenes Werkzeug: Standard an, ohne „Neu“-Markierung.
    pub const fn std(tool: Tool, id: &'static str, name: &'static str, group: Group, icon: i32, desc: &'static str) -> ToolDef {
        ToolDef { tool, id, name, desc, group, icon, default_on: true, new: false, key: None, parent: None }
    }

    /// Neues Werkzeug: Standard aus, mit „Neu“-Markierung.
    #[allow(dead_code)] // für die nächsten Werkzeuge
    pub const fn neu(tool: Tool, id: &'static str, name: &'static str, group: Group, icon: i32, desc: &'static str) -> ToolDef {
        ToolDef { default_on: false, new: true, ..ToolDef::std(tool, id, name, group, icon, desc) }
    }

    /// Standard-Tastenkürzel.
    pub const fn key(self, k: &'static str) -> ToolDef {
        ToolDef { key: Some(k), ..self }
    }

    /// Erreichbar über das Werkzeug `parent`.
    pub const fn inside(self, parent: &'static str) -> ToolDef {
        ToolDef { parent: Some(parent), ..self }
    }
}

/// Alle Werkzeuge in Standard-Reihenfolge der Leiste.
pub const TOOLS: &[ToolDef] = &[
    ToolDef::std(Tool::Pen, "pen", "Stift", Group::Zeichnen, ic::PEN, "Zeichnet freie Linien, mit einem Stift druckempfindlich.").key("P"),
    ToolDef::std(Tool::Marker, "marker", "Marker", Group::Zeichnen, ic::MARKER, "Breiter, halbdurchsichtiger Textmarker zum Hervorheben.").key("M"),
    ToolDef::std(Tool::Eraser, "eraser", "Radierer", Group::Zeichnen, ic::ERASER, "Löscht ganze Objekte oder nur die überstrichenen Stellen.").key("E"),
    ToolDef::std(Tool::Select, "select", "Auswahl", Group::Ordnen, ic::LASSO, "Wählt Objekte aus, um sie zu verschieben, zu drehen oder zu bearbeiten.").key("S"),
    ToolDef::std(Tool::Shape, "shape", "Form", Group::Formen, ic::RECT, "Rechteck, Ellipse, Linie und Pfeile durch Ziehen von Ecke zu Ecke.").key("R"),
    ToolDef::std(Tool::Text, "text", "Text", Group::Extras, ic::TEXT, "Schreibt Text direkt auf das Blatt.").key("T"),
    ToolDef::std(Tool::Note, "note", "Haftnotiz", Group::Extras, ic::NOTE, "Farbiger Zettel mit Text, erreichbar über das Text-Werkzeug.").key("N").inside("text"),
    ToolDef::std(Tool::Image, "image", "Bild", Group::Extras, ic::IMAGE, "Fügt ein Bild aus einer Datei ein.").key("I"),
    ToolDef::neu(Tool::Fill, "fill", "Füllen", Group::Zeichnen, ic::DARK, "Füllt eine geschlossene Form mit der aktuellen Farbe.").key("F"),
    ToolDef::neu(Tool::Picker, "picker", "Pipette", Group::Extras, ic::FINGER, "Übernimmt die Farbe einer Stelle auf dem Blatt.").key("K"),
    ToolDef::neu(Tool::Dim, "dim", "Maßlinie", Group::Formen, ic::LINE, "Linie mit Maßpfeilen und der Länge in der Mitte.").key("L"),
    ToolDef::neu(Tool::Stamp, "stamp", "Stempel", Group::Extras, ic::MAGIC, "Setzt Haken, Kreuz, Stern, Fragezeichen oder Ausrufezeichen.").key("J"),
    ToolDef::neu(Tool::Ruler, "ruler", "Lineal", Group::Extras, ic::RULER, "Blendet ein Lineal oder Geodreieck ein; Striche an der Kante werden gerade.").key("U"),
    ToolDef::neu(Tool::Laser, "laser", "Laserpointer", Group::Extras, ic::LASER, "Leuchtspur zum Zeigen, verblasst nach 1,5 s und wird nicht gespeichert.").key("Q"),
    ToolDef::neu(Tool::Space, "space", "Platz schaffen", Group::Ordnen, ic::SPACE, "Senkrecht ziehen schiebt alles darunter nach unten oder oben.").key("G"),
    ToolDef::neu(Tool::Connector, "connector", "Verbinder", Group::Formen, ic::CONNECTOR, "Linie oder Pfeil, der an Objekten andockt und beim Verschieben mitgeht.").key("W"),
    ToolDef::neu(Tool::Table, "table", "Tabelle", Group::Ordnen, ic::TABLE, "Setzt eine Tabelle; Zellen beschriftest du direkt auf dem Blatt.").key("B"),
    ToolDef::neu(Tool::Pin, "pin", "Kommentar", Group::Extras, ic::PIN, "Stecknadel mit Kommentar und Antworten, für alle im Board sichtbar.").key("C"),
    ToolDef::neu(Tool::Math, "math", "Formel", Group::Extras, ic::FORMULA, "Setzt Mathe-Formeln in LaTeX-Schreibweise mit Live-Vorschau.").key("Y"),
];

/// Werkzeug nach Kennung.
pub fn def(id: &str) -> Option<&'static ToolDef> {
    TOOLS.iter().find(|d| d.id == id)
}

/// Eintrag eines Werkzeugs (jedes `Tool` steht genau einmal in [`TOOLS`]).
pub fn of(t: Tool) -> &'static ToolDef {
    TOOLS.iter().find(|d| d.tool == t).expect("Werkzeug fehlt in TOOLS")
}

/// Ist das Werkzeug eingeschaltet? Unbekannte Kennungen: aus.
pub fn enabled(s: &Settings, id: &str) -> bool {
    let Some(d) = def(id) else { return false };
    s.tool_on.get(id).copied().unwrap_or(d.default_on)
}

/// Alle Werkzeuge in der Reihenfolge der Leiste (eigene Reihenfolge, danach der Rest in Registry-Reihenfolge).
pub fn ordered(s: &Settings) -> Vec<&'static ToolDef> {
    let mut v: Vec<&'static ToolDef> = Vec::new();
    for id in &s.tool_order {
        if let Some(d) = def(id) {
            if !v.iter().any(|x| x.id == d.id) {
                v.push(d);
            }
        }
    }
    for d in TOOLS {
        if !v.iter().any(|x| x.id == d.id) {
            v.push(d);
        }
    }
    v
}

/// Knöpfe der Werkzeugleiste: eingeschaltete Werkzeuge; solche mit `parent` nur, wenn das andere aus ist.
pub fn bar_tools(s: &Settings) -> Vec<Tool> {
    let v: Vec<Tool> = ordered(s).into_iter().filter(|d| enabled(s, d.id) && d.parent.map(|p| !enabled(s, p)).unwrap_or(true)).map(|d| d.tool).collect();
    if v.is_empty() {
        // kann nach `sanitize` nicht vorkommen (mindestens ein Werkzeug bleibt an)
        return TOOLS.iter().filter(|d| d.default_on && d.parent.is_none()).map(|d| d.tool).collect();
    }
    v
}

/// Schaltet ein Werkzeug ein oder aus. Das letzte eingeschaltete bleibt an (Rückgabe false).
/// Gespeichert wird nur die Abweichung vom Standard.
pub fn set_enabled(s: &mut Settings, id: &str, on: bool) -> bool {
    let Some(d) = def(id) else { return false };
    if !on && enabled(s, id) && TOOLS.iter().filter(|t| enabled(s, t.id)).count() <= 1 {
        return false;
    }
    if on == d.default_on {
        s.tool_on.remove(id);
    } else {
        s.tool_on.insert(id.to_string(), on);
    }
    true
}

/// Verschiebt das Werkzeug an Stelle `from` (Reihenfolge von [`ordered`]) nach `to`.
pub fn reorder(s: &mut Settings, from: usize, to: usize) {
    let mut ids: Vec<String> = ordered(s).iter().map(|d| d.id.to_string()).collect();
    crate::settings::reorder(&mut ids, from, to);
    let default: Vec<&str> = TOOLS.iter().map(|d| d.id).collect();
    s.tool_order = if ids.iter().map(|x| x.as_str()).eq(default.iter().copied()) { Vec::new() } else { ids };
}

/// Werkzeuge an/aus und Reihenfolge auf Standard.
pub fn reset(s: &mut Settings) {
    s.tool_on.clear();
    s.tool_order.clear();
}

/// Werkzeuge, die es vor der Registry gab (alte Einstellung `tools`, Version ≤ 4.24).
const LEGACY: [&str; 7] = ["pen", "marker", "eraser", "select", "shape", "text", "image"];

/// Übernimmt die alte Einstellung `tools` (sichtbare Werkzeuge in Reihenfolge): Reihenfolge und an/aus.
/// Haftnotiz war Teil von Text und bleibt an, solange Text sichtbar war.
pub fn migrate_legacy(s: &mut Settings, old: &[String]) {
    let mut vis: Vec<&str> = Vec::new();
    for id in old {
        let id = if id == "note" { "text" } else { id.as_str() };
        if let Some(l) = LEGACY.iter().find(|l| **l == id) {
            if !vis.contains(l) {
                vis.push(l);
            }
        }
    }
    if vis.is_empty() {
        return; // wie früher: leere Liste = Vorgaben
    }
    let mut order: Vec<String> = vis.iter().map(|x| x.to_string()).collect();
    for l in LEGACY {
        if !vis.contains(&l) {
            order.push(l.to_string());
        }
    }
    // Haftnotiz gehörte zu Text: direkt dahinter
    let ti = order.iter().position(|x| x == "text").map(|i| i + 1).unwrap_or(order.len());
    order.insert(ti, "note".into());
    s.tool_order = order;
    // an/aus unabhängig von der Mindestregel setzen (die alte Liste hatte immer mindestens ein Werkzeug)
    for l in LEGACY {
        let d = def(l).expect("Altwerkzeug in TOOLS");
        let on = vis.contains(&l);
        if on != d.default_on {
            s.tool_on.insert(l.to_string(), on);
        }
    }
    let note_on = vis.contains(&"text");
    if !note_on {
        s.tool_on.insert("note".into(), false);
    }
    // Reihenfolge, die der Vorgabe entspricht, nicht speichern
    let ids: Vec<String> = ordered(s).iter().map(|d| d.id.to_string()).collect();
    if ids.iter().map(|x| x.as_str()).eq(TOOLS.iter().map(|d| d.id)) {
        s.tool_order.clear();
    }
}

/// Nach dem Laden: unbekannte Kennungen weg, doppelte weg, mindestens ein Werkzeug an.
pub fn sanitize(s: &mut Settings) {
    let mut seen: Vec<String> = Vec::new();
    s.tool_order.retain(|id| {
        let ok = def(id).is_some() && !seen.contains(id);
        seen.push(id.clone());
        ok
    });
    // Werte gleich dem Standard nicht speichern; unbekannte Kennungen behalten (Werkzeuge neuerer Versionen)
    s.tool_on.retain(|id, on| def(id).map(|d| d.default_on != *on).unwrap_or(true));
    if !TOOLS.iter().any(|d| enabled(s, d.id)) {
        s.tool_on.retain(|id, _| def(id).is_none());
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn registry_eindeutig_und_vollstaendig() {
        for (i, d) in TOOLS.iter().enumerate() {
            assert!(TOOLS[..i].iter().all(|x| x.id != d.id && x.tool != d.tool), "doppelt: {}", d.id);
            assert!(!d.desc.is_empty() && !d.name.is_empty());
            assert_eq!(of(d.tool).id, d.id);
            assert_eq!(Tool::from_id(d.id), Some(d.tool));
            if let Some(p) = d.parent {
                assert!(def(p).is_some(), "{}: unbekanntes parent {p}", d.id);
            }
            if let Some(k) = d.key {
                assert!(super::super::keys::Chord::parse(k).is_some(), "{}: Kürzel {k}", d.id);
            }
        }
        // heutige Werkzeuge sind Standard an und nicht „neu“
        for id in ["pen", "marker", "eraser", "select", "shape", "text", "note", "image"] {
            let d = def(id).unwrap();
            assert!(d.default_on && !d.new, "{id}");
        }
    }

    #[test]
    fn standard_leiste_wie_bisher() {
        let s = Settings::default();
        assert_eq!(bar_tools(&s), vec![Tool::Pen, Tool::Marker, Tool::Eraser, Tool::Select, Tool::Shape, Tool::Text, Tool::Image]);
    }

    #[test]
    fn mindestens_ein_werkzeug_aktiv() {
        let mut s = Settings::default();
        for d in TOOLS.iter().skip(1) {
            assert!(set_enabled(&mut s, d.id, false), "{}", d.id);
        }
        assert!(!set_enabled(&mut s, "pen", false), "letztes bleibt an");
        assert!(enabled(&s, "pen"));
        assert_eq!(bar_tools(&s), vec![Tool::Pen]);
        assert!(set_enabled(&mut s, "note", true));
        // Text aus: Haftnotiz bekommt einen eigenen Knopf
        assert_eq!(bar_tools(&s), vec![Tool::Pen, Tool::Note]);
        assert!(set_enabled(&mut s, "pen", false));
        assert_eq!(bar_tools(&s), vec![Tool::Note]);
        // Laden mit allem aus: Vorgaben
        let mut s = Settings::default();
        for d in TOOLS {
            s.tool_on.insert(d.id.into(), false);
        }
        sanitize(&mut s);
        assert!(TOOLS.iter().all(|d| enabled(&s, d.id) == d.default_on));
    }

    #[test]
    fn nur_abweichungen_gespeichert() {
        let mut s = Settings::default();
        assert!(set_enabled(&mut s, "marker", false));
        assert_eq!(s.tool_on.len(), 1);
        assert!(set_enabled(&mut s, "marker", true));
        assert!(s.tool_on.is_empty());
        reorder(&mut s, 0, 2);
        assert_eq!(ordered(&s)[2].id, "pen");
        reorder(&mut s, 2, 0);
        assert!(s.tool_order.is_empty(), "Standard-Reihenfolge wird nicht gespeichert");
    }

    #[test]
    fn migration_alte_einstellung() {
        // alter Nutzer: Marker und Bild ausgeblendet, Radierer vorne
        let s = Settings::parse(br#"{"tools": ["eraser", "pen", "select", "shape", "note"]}"#).unwrap();
        assert_eq!(bar_tools(&s), vec![Tool::Eraser, Tool::Pen, Tool::Select, Tool::Shape, Tool::Text]);
        assert!(!enabled(&s, "marker") && !enabled(&s, "image") && enabled(&s, "note"));
        // neu gespeichert ohne alte Einstellung, Ergebnis bleibt
        let json = serde_json::to_string(&s).unwrap();
        assert!(!json.contains("\"tools\""), "{json}");
        let neu = Settings::parse(json.as_bytes()).unwrap();
        assert_eq!(neu, s);
        assert_eq!(bar_tools(&neu), bar_tools(&s));
        // Text ausgeblendet: Haftnotiz war damit auch weg
        let s = Settings::parse(br#"{"tools": ["pen", "eraser"]}"#).unwrap();
        assert_eq!(bar_tools(&s), vec![Tool::Pen, Tool::Eraser]);
        assert!(!enabled(&s, "note"));
        // alte Vorgabe: nichts gespeichert
        let s = Settings::parse(br#"{"tools": ["pen", "marker", "eraser", "select", "shape", "text", "image"]}"#).unwrap();
        assert!(s.tool_on.is_empty() && s.tool_order.is_empty());
        // leer bzw. unbekannt: Vorgaben
        assert_eq!(Settings::parse(br#"{"tools": []}"#).unwrap(), Settings::default());
        assert_eq!(Settings::parse(br#"{"tools": ["x"]}"#).unwrap(), Settings::default());
    }
}
