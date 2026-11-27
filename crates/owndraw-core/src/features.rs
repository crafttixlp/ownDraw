//! Freigaben durch den Admin: Katalog aller Werkzeuge und Funktionen, die der Admin im Admin-Panel
//! (Tab „Funktionen“) für Nutzer sperren kann. Der Server speichert die gesperrten Kennungen
//! (`GET /api/features`), die App blendet sie aus. Admins sind nie betroffen.
//!
//! Werkzeuge haben dieselbe Kennung wie in der Werkzeug-Registry der App (`ui/editor/tools.rs`); ein Test
//! dort prüft, dass beide Listen übereinstimmen. Neues Werkzeug → hier eine Zeile `tool(…)`.

/// Art eines Eintrags.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    Tool,
    Feature,
}

impl Kind {
    pub fn as_str(self) -> &'static str {
        match self {
            Kind::Tool => "tool",
            Kind::Feature => "feature",
        }
    }
}

/// Ein sperrbares Werkzeug bzw. eine sperrbare Funktion.
#[derive(Clone, Copy, Debug)]
pub struct Feature {
    /// Stabile Kennung (gespeichert am Server); nie umbenennen.
    pub id: &'static str,
    pub name: &'static str,
    pub kind: Kind,
    /// Kurze Erklärung für das Admin-Panel.
    pub desc: &'static str,
}

const fn tool(id: &'static str, name: &'static str) -> Feature {
    Feature { id, name, kind: Kind::Tool, desc: "" }
}

const fn feature(id: &'static str, name: &'static str, desc: &'static str) -> Feature {
    Feature { id, name, kind: Kind::Feature, desc }
}

/// Funktion „Teilen“ (Mitglieder und Einladungslinks).
pub const SHARE: &str = "share";
/// Export-Fenster.
pub const EXPORT: &str = "export";
/// Formelsammlung.
pub const FORMULAS: &str = "formulas";
/// Ebenen.
pub const LAYERS: &str = "layers";
/// „Fehler melden“.
pub const BUGREPORT: &str = "bugreport";
/// Office: Textverarbeitung (Textdokumente anlegen und öffnen).
pub const OFFICE: &str = "office";
/// Office: Word-Dateien öffnen, einfügen und speichern.
pub const OFFICE_WORD: &str = "office_word";

/// Alle sperrbaren Einträge (Werkzeuge in Leisten-Reihenfolge, danach Funktionen).
pub const ALL: &[Feature] = &[
    tool("pen", "Stift"),
    tool("marker", "Marker"),
    tool("eraser", "Radierer"),
    tool("select", "Auswahl"),
    tool("shape", "Form"),
    tool("text", "Text"),
    tool("note", "Haftnotiz"),
    tool("image", "Bild"),
    tool("fill", "Füllen"),
    tool("picker", "Pipette"),
    tool("dim", "Maßlinie"),
    tool("stamp", "Stempel"),
    tool("ruler", "Lineal"),
    tool("laser", "Laserpointer"),
    tool("space", "Platz schaffen"),
    tool("connector", "Verbinder"),
    tool("table", "Tabelle"),
    tool("pin", "Kommentar"),
    tool("math", "Formel"),
    tool("calc", "Rechnen"),
    tool("convert", "Umwandeln"),
    tool("penmode", "Stift-Modus"),
    feature(SHARE, "Teilen", "Mitglieder hinzufügen, Einladungslinks erstellen und über Links beitreten. Bestehende Mitglieder bleiben."),
    feature(EXPORT, "Exportieren", "Export-Fenster mit allen Formaten, Teilen, Drucken und Kopieren."),
    feature(FORMULAS, "Formelsammlung", "Fenster „Formelsammlung“ und eigene Formeln am Server."),
    feature(LAYERS, "Ebenen", "Ebenen-Liste im Menü des Editors."),
    feature(BUGREPORT, "Fehler melden", "Menüpunkt „Fehler melden“ in der Web-App."),
    feature(OFFICE, "Office: Textverarbeitung", "Textdokumente (wie Word) anlegen und öffnen. Gesperrt: Karte „Textdokument“ fehlt, vorhandene öffnen nicht."),
    feature(OFFICE_WORD, "Office: Word-Dateien", "Word-Dateien (.docx) öffnen, in Textdokumente einfügen und als Word-Datei speichern."),
];

/// Eintrag nach Kennung.
pub fn get(id: &str) -> Option<&'static Feature> {
    ALL.iter().find(|f| f.id == id)
}

/// Bereinigt eine Liste gesperrter Kennungen: nur bekannte, ohne doppelte, in Katalog-Reihenfolge.
/// Mindestens ein Werkzeug bleibt frei (sonst `None`).
pub fn clean(ids: &[String]) -> Option<Vec<String>> {
    let out: Vec<String> = ALL.iter().filter(|f| ids.iter().any(|i| i == f.id)).map(|f| f.id.to_string()).collect();
    let free = ALL.iter().any(|f| f.kind == Kind::Tool && !out.iter().any(|o| o == f.id));
    free.then_some(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn eindeutig() {
        for (i, f) in ALL.iter().enumerate() {
            assert!(ALL[..i].iter().all(|x| x.id != f.id), "doppelt: {}", f.id);
            assert!(!f.name.is_empty());
            assert!(f.kind == Kind::Tool || !f.desc.is_empty(), "{}", f.id);
        }
    }

    #[test]
    fn bereinigen() {
        let v = |s: &[&str]| s.iter().map(|x| x.to_string()).collect::<Vec<_>>();
        assert_eq!(clean(&v(&["export", "x", "laser", "laser"])), Some(v(&["laser", "export"])));
        let all_tools: Vec<String> = ALL.iter().filter(|f| f.kind == Kind::Tool).map(|f| f.id.to_string()).collect();
        assert_eq!(clean(&all_tools), None, "mindestens ein Werkzeug bleibt frei");
    }
}
