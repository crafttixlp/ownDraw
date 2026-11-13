//! Eine Ebene des Boards. Objekte verweisen ueber Item.layer auf Layer.id. Index 0 in Board.layers = ganz hinten.

use crate::item::{jbool, jf, jint, num};
use serde_json::{json, Value};

/// Animationsarten fuer den Website-Export.
pub const A_NONE: i32 = 0;
pub const A_PARALLAX: i32 = 1;
pub const A_DRIFT: i32 = 2;
pub const A_FLOAT: i32 = 3;
pub const A_FADE_IN: i32 = 4;
pub const A_SLIDE_UP: i32 = 5;
pub const A_HUE: i32 = 6;
pub const A_RISE: i32 = 7;
pub const ANIM_NAMES: [&str; 8] = [
    "Keine",
    "Parallax (Scrollen)",
    "Seitwärts treiben",
    "Schweben",
    "Einblenden",
    "Von unten einfahren",
    "Farbverlauf wandern",
    "Partikel steigen auf",
];

#[derive(Clone, Debug, PartialEq)]
pub struct Layer {
    pub id: i32,
    pub name: String,
    pub visible: bool,
    pub locked: bool,
    pub opacity: f32,
    pub anim: i32,
    /// Parallax: Faktor 0..1 (0,2 = langsam). Sonst: Tempo-Faktor 0,25..3 (1 = normal).
    pub speed: f32,
    /// Hover-Effekt (leichtes Anheben/Vergroessern) bei Mauszeiger.
    pub hover: bool,
}

impl Layer {
    pub fn new(id: i32, name: &str) -> Layer {
        Layer { id, name: name.to_string(), visible: true, locked: false, opacity: 1.0, anim: A_NONE, speed: 1.0, hover: false }
    }

    /// Dateiname ohne Leerzeichen/Umlaute, z. B. "02-berge-hinten".
    pub fn file_name(&self) -> String {
        let s = self.name.to_lowercase().replace('ä', "ae").replace('ö', "oe").replace('ü', "ue").replace('ß', "ss");
        let mut out = String::new();
        for ch in s.chars() {
            if ch.is_ascii_lowercase() || ch.is_ascii_digit() || ch == '_' || ch == '-' {
                out.push(ch);
            } else {
                out.push('-');
            }
        }
        let mut dedup = String::new();
        for ch in out.chars() {
            if ch == '-' && dedup.ends_with('-') {
                continue;
            }
            dedup.push(ch);
        }
        // wie die Java-Regex "^-|-$": genau ein Bindestrich am Anfang und am Ende
        let t = dedup.strip_prefix('-').unwrap_or(&dedup);
        let t = t.strip_suffix('-').unwrap_or(t);
        if t.is_empty() {
            format!("ebene-{}", self.id)
        } else {
            t.to_string()
        }
    }

    pub fn json(&self) -> Value {
        json!({"id": self.id, "n": self.name, "v": self.visible, "l": self.locked, "o": num(self.opacity),
               "a": self.anim, "sp": num(self.speed), "hv": self.hover})
    }

    pub fn from_json(o: &Value) -> Layer {
        Layer {
            id: o.get("id").and_then(jint).unwrap_or(1),
            name: o.get("n").and_then(Value::as_str).unwrap_or("Ebene").to_string(),
            visible: o.get("v").and_then(Value::as_bool).unwrap_or(true),
            locked: jbool(o, "l"),
            opacity: jf(o, "o").unwrap_or(1.0),
            anim: o.get("a").and_then(jint).unwrap_or(A_NONE),
            speed: jf(o, "sp").unwrap_or(1.0),
            hover: jbool(o, "hv"),
        }
    }

    /// Vorlage aus dem Konzept "Layer-Design zu animierter Website", hinten nach vorne.
    pub fn template(first_id: i32) -> Vec<Layer> {
        let t: [(&str, i32, f32, bool); 7] = [
            ("01-hintergrund", A_HUE, 1.0, false),
            ("02-berge-hinten", A_PARALLAX, 0.2, false),
            ("03-wolken", A_DRIFT, 1.0, false),
            ("04-berge-vorne", A_PARALLAX, 0.6, false),
            ("05-figur", A_FLOAT, 1.0, false),
            ("06-deko", A_RISE, 1.0, false),
            ("07-text", A_SLIDE_UP, 1.0, true),
        ];
        t.iter()
            .enumerate()
            .map(|(i, (n, a, sp, hv))| {
                let mut l = Layer::new(first_id + i as i32, n);
                l.anim = *a;
                l.speed = *sp;
                l.hover = *hv;
                l
            })
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn file_names() {
        assert_eq!(Layer::new(3, "Berge Hinten!").file_name(), "berge-hinten");
        assert_eq!(Layer::new(3, "Größe").file_name(), "groesse");
        assert_eq!(Layer::new(3, "???").file_name(), "ebene-3");
    }

    #[test]
    fn json_roundtrip() {
        let l = Layer::template(5).remove(1);
        assert_eq!(Layer::from_json(&l.json()), l);
        assert_eq!(Layer::from_json(&serde_json::json!({})).name, "Ebene");
    }
}
