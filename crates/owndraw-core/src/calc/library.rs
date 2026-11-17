//! Formelsammlung: Sammlungen mit Formeln, Konstanten und eigenen Einheiten; Grundsammlung; Vorschläge.
//!
//! Formeln stehen in getippter Schreibweise des Rechenkerns (`P = U·I`), damit sie direkt als Rechenzeile
//! eingefügt werden können. Konstanten und Einheiten aktiver Sammlungen gelten auf jeder Seite (`Prelude`).
//! Dateiformat (Export/Import, Server): JSON einer `Collection` mit `"format": "owndraw-formeln"`.

use super::{line_vars, Prelude};
use serde::{Deserialize, Serialize};

/// Kennung im Dateiformat.
pub const FORMAT: &str = "owndraw-formeln";
/// Dateiendung beim Export.
pub const EXT: &str = "formeln.json";

/// Eine Größe einer Formel (Formelzeichen, Einheit, Bedeutung).
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct Var {
    pub sym: String,
    #[serde(default)]
    pub unit: String,
    #[serde(default)]
    pub desc: String,
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct Formula {
    pub name: String,
    /// Getippte Formel, z. B. `P = U·I`.
    pub expr: String,
    #[serde(default)]
    pub vars: Vec<Var>,
    #[serde(default)]
    pub desc: String,
    /// Thema/Kapitel, z. B. `Mechanik – Kinematik`.
    #[serde(default)]
    pub topic: String,
}

/// Konstante (`g` = `9,81 m/s²`) oder eigene Einheit (`kp` = `9,80665 N`).
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct Named {
    pub sym: String,
    pub value: String,
    #[serde(default)]
    pub desc: String,
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct Collection {
    #[serde(default = "format_tag")]
    pub format: String,
    pub name: String,
    #[serde(default)]
    pub desc: String,
    #[serde(default)]
    pub formulas: Vec<Formula>,
    #[serde(default)]
    pub consts: Vec<Named>,
    #[serde(default)]
    pub units: Vec<Named>,
}

fn format_tag() -> String {
    FORMAT.into()
}

impl Collection {
    pub fn new(name: &str) -> Collection {
        Collection { format: FORMAT.into(), name: name.into(), ..Default::default() }
    }

    /// Liest eine Sammlung (Datei/Server). Akzeptiert auch eine Liste von Formeln.
    pub fn from_json(s: &str) -> Result<Collection, String> {
        let v: serde_json::Value = serde_json::from_str(s).map_err(|e| format!("keine gültige Datei ({e})"))?;
        Collection::from_value(v)
    }

    pub fn from_value(v: serde_json::Value) -> Result<Collection, String> {
        if v.is_array() {
            let formulas: Vec<Formula> = serde_json::from_value(v).map_err(|e| format!("Formeln nicht lesbar ({e})"))?;
            return Ok(Collection { formulas, ..Collection::new("Importiert") });
        }
        if let Some(f) = v.get("format").and_then(|f| f.as_str()) {
            if f != FORMAT {
                return Err(format!("unbekanntes Format „{f}“"));
            }
        }
        let mut c: Collection = serde_json::from_value(v).map_err(|e| format!("Sammlung nicht lesbar ({e})"))?;
        c.format = FORMAT.into();
        Ok(c)
    }

    pub fn to_json(&self) -> String {
        serde_json::to_string_pretty(self).unwrap_or_default()
    }

    pub fn to_value(&self) -> serde_json::Value {
        serde_json::to_value(self).unwrap_or_default()
    }

    /// Themen in Reihenfolge des ersten Vorkommens.
    pub fn topics(&self) -> Vec<&str> {
        let mut t: Vec<&str> = Vec::new();
        for f in &self.formulas {
            if !t.contains(&f.topic.as_str()) {
                t.push(&f.topic);
            }
        }
        t
    }
}

impl Formula {
    /// Formelzeichen der Formel (aus `expr`, sonst aus `vars`).
    pub fn syms(&self) -> Vec<String> {
        let v = line_vars(&self.expr);
        if v.is_empty() {
            self.vars.iter().map(|v| v.sym.clone()).collect()
        } else {
            v
        }
    }

    /// Ergänzt fehlende Größen aus der Formel (leere Einheit/Beschreibung), entfernt nicht mehr vorkommende leere.
    pub fn sync_vars(&mut self) {
        let syms = line_vars(&self.expr);
        if syms.is_empty() {
            return;
        }
        self.vars.retain(|v| syms.contains(&v.sym) || !v.unit.is_empty() || !v.desc.is_empty());
        for s in syms {
            if !self.vars.iter().any(|v| v.sym == s) {
                self.vars.push(Var { sym: s, ..Default::default() });
            }
        }
    }

    pub fn var(&self, sym: &str) -> Option<&Var> {
        self.vars.iter().find(|v| v.sym == sym)
    }
}

/// Mitgelieferte Grundsammlung (Mechanik, Elektrotechnik).
pub fn builtin() -> Collection {
    Collection::from_json(include_str!("grundsammlung.json")).expect("Grundsammlung lesbar")
}

/// Konstanten und Einheiten der Sammlungen für den Rechenkern (spätere überschreiben frühere nicht).
pub fn prelude<'a>(cols: impl IntoIterator<Item = &'a Collection>) -> Prelude {
    let mut p = Prelude::default();
    for c in cols {
        for k in &c.consts {
            if !p.consts.iter().any(|x| x.0 == k.sym) {
                p.consts.push((k.sym.clone(), k.value.clone()));
            }
        }
        for u in &c.units {
            if !p.units.iter().any(|x| x.0 == u.sym) {
                p.units.push((u.sym.clone(), u.value.clone()));
            }
        }
    }
    p
}

/// Ein Treffer (Sammlung, Formel) mit Bewertung.
#[derive(Clone, Debug, PartialEq)]
pub struct Hit {
    pub col: usize,
    pub idx: usize,
    pub score: i32,
    /// Formelzeichen, die auf der Seite noch fehlen.
    pub missing: Vec<String>,
}

fn norm(s: &str) -> String {
    s.to_lowercase().replace(['ä'], "a").replace('ö', "o").replace('ü', "u").replace('ß', "ss")
}

/// Textsuche über Name, Beschreibung, Thema, Formel und Größen. Leere Suche = alle.
pub fn search(cols: &[&Collection], q: &str) -> Vec<Hit> {
    let words: Vec<String> = norm(q).split_whitespace().map(|s| s.to_string()).collect();
    let mut out = Vec::new();
    for (ci, c) in cols.iter().enumerate() {
        for (fi, f) in c.formulas.iter().enumerate() {
            let mut hay = norm(&format!("{} {} {} {}", f.name, f.desc, f.topic, f.expr));
            for v in &f.vars {
                hay.push(' ');
                hay.push_str(&norm(&format!("{} {} {}", v.sym, v.unit, v.desc)));
            }
            if words.iter().all(|w| hay.contains(w.as_str())) {
                let score = words.iter().map(|w| if norm(&f.name).contains(w.as_str()) { 3 } else { 1 }).sum();
                out.push(Hit { col: ci, idx: fi, score, missing: Vec::new() });
            }
        }
    }
    out.sort_by(|a, b| b.score.cmp(&a.score));
    out
}

/// Vorschläge für das Rechnen-Werkzeug: Formeln, die eine gefragte Größe (`asked`, z. B. aus `P =`) enthalten
/// und möglichst nur bekannte Größen (`known`) brauchen. `typed`: Zeile, die gerade getippt wird.
pub fn suggest(cols: &[&Collection], known: &[String], asked: &[String], typed: &str, max: usize) -> Vec<Hit> {
    let mut want: Vec<String> = asked.to_vec();
    // `P =` / `P = ?` / `P` in der Eingabezeile
    let t = typed.trim().trim_end_matches('?').trim_end_matches('=').trim();
    if !t.is_empty() && !t.contains(['=', ':', ' ', '+', '*', '/', '·']) && !want.iter().any(|w| w == t) {
        want.insert(0, t.to_string());
    }
    if want.is_empty() {
        return Vec::new();
    }
    let mut out: Vec<Hit> = Vec::new();
    for (ci, c) in cols.iter().enumerate() {
        for (fi, f) in c.formulas.iter().enumerate() {
            let syms = f.syms();
            let Some(wi) = want.iter().position(|w| syms.contains(w)) else { continue };
            let others: Vec<&String> = syms.iter().filter(|s| **s != want[wi]).collect();
            let missing: Vec<String> = others.iter().filter(|s| !known.contains(s)).map(|s| (*s).clone()).collect();
            let have = others.len() - missing.len();
            let mut score = 20 - wi as i32 * 3 + have as i32 * 4 - missing.len() as i32 * 3;
            if missing.is_empty() {
                score += 10;
            }
            // gleiche Formel aus mehreren Sammlungen nur einmal
            if out.iter().any(|h| cols[h.col].formulas[h.idx].expr == f.expr) {
                continue;
            }
            out.push(Hit { col: ci, idx: fi, score, missing });
        }
    }
    out.sort_by(|a, b| b.score.cmp(&a.score).then(a.missing.len().cmp(&b.missing.len())));
    out.truncate(max);
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::calc::{self, Opts, Out};

    #[test]
    fn grundsammlung_lesbar_und_rechenbar() {
        let c = builtin();
        assert!(c.formulas.len() >= 50, "nur {} Formeln", c.formulas.len());
        let o = Opts::default();
        for f in &c.formulas {
            assert!(calc::is_calc_line(&f.expr), "nicht lesbar: {} ({})", f.expr, f.name);
            let syms = f.syms();
            assert!(syms.len() >= 2, "zu wenige Größen: {}", f.expr);
            for s in &syms {
                let v = f.var(s).unwrap_or_else(|| panic!("Größe {s} fehlt in {} ({})", f.expr, f.name));
                assert!(!v.desc.is_empty(), "Beschreibung fehlt: {s} in {}", f.name);
                // Einheit prüfbar: Formel mit Einheiten-Beispielwerten muss dimensionsrichtig sein
                if !v.unit.is_empty() && v.unit != "1" {
                    calc::quantity(&format!("1 {}", v.unit), &o).unwrap_or_else(|e| panic!("Einheit {} von {s} in {}: {e}", v.unit, f.name));
                }
            }
            // Dimensionsprobe: alle Größen bis auf die erste mit Einheit 1 belegen und nach der ersten fragen
            let first = &syms[0];
            let mut lines: Vec<String> = Vec::new();
            for s in syms.iter().skip(1) {
                let v = f.var(s).unwrap();
                let u = if v.unit == "1" { String::new() } else { v.unit.clone() };
                lines.push(format!("{s}: 2 {u}"));
            }
            lines.push(f.expr.clone());
            let fu = &f.var(first).unwrap().unit;
            lines.push(if fu.is_empty() || fu == "1" { format!("{first} =") } else { format!("{first} = ? {fu}") });
            let refs: Vec<&str> = lines.iter().map(|s| s.as_str()).collect();
            let r = calc::page(&refs, &o);
            match r.last().unwrap() {
                Some(Out::Result { err: None, .. }) => {}
                other => panic!("Formel {} ({}) nicht rechenbar: {:?}\n{}", f.name, f.expr, other, lines.join("\n")),
            }
        }
    }

    #[test]
    fn datei_hin_und_zurueck() {
        let mut c = Collection::new("Meine");
        c.formulas.push(Formula { name: "Leistung".into(), expr: "P = U·I".into(), ..Default::default() });
        c.formulas[0].sync_vars();
        assert_eq!(c.formulas[0].vars.len(), 3);
        c.consts.push(Named { sym: "g".into(), value: "9,81 m/s²".into(), desc: String::new() });
        let s = c.to_json();
        assert!(s.contains("owndraw-formeln"));
        assert_eq!(Collection::from_json(&s).unwrap(), c);
        assert!(Collection::from_json(r#"{"format":"anders","name":"x"}"#).is_err());
        let l = Collection::from_json(r#"[{"name":"A","expr":"A = a·b"}]"#).unwrap();
        assert_eq!(l.formulas.len(), 1);
    }

    #[test]
    fn vorschlaege() {
        let c = builtin();
        let cols = [&c];
        let lines = ["U: 230 V", "I: 2 A", "P ="];
        let (known, asked) = calc::page_names(&lines, &Opts::default(), &Prelude::default());
        assert!(known.contains(&"U".to_string()) && known.contains(&"I".to_string()), "{known:?}");
        assert_eq!(asked, vec!["P".to_string()]);
        let h = suggest(&cols, &known, &asked, "", 5);
        let first = &c.formulas[h[0].idx];
        assert!(first.expr.replace(' ', "").starts_with("P=U·I"), "{}", first.expr);
        assert!(h[0].missing.is_empty());
        // getippt
        let h = suggest(&cols, &[], &[], "F =", 3);
        assert!(!h.is_empty());
        assert!(search(&cols, "ohm").iter().any(|h| c.formulas[h.idx].name.contains("Ohm")));
    }

    #[test]
    fn konstanten_und_einheiten() {
        let mut c = Collection::new("K");
        c.consts.push(Named { sym: "rho_Cu".into(), value: "0,0178 Ω·mm²/m".into(), desc: String::new() });
        c.units.push(Named { sym: "kp".into(), value: "9,80665 N".into(), desc: String::new() });
        let p = prelude([&c]);
        let o = Opts::default();
        let r = calc::page_with(&["F: 2 kp", "F = ? N", "R = rho_Cu·10 m/(1 mm²)", "R ="], &o, &p);
        assert_eq!(r.len(), 4);
        assert!(r[1].as_ref().unwrap().text().starts_with("19{,}61"), "{:?}", r[1]);
        assert!(r[3].as_ref().unwrap().text().starts_with("0{,}178"), "{:?}", r[3]);
        // ohne Vorgaben unbekannt
        let r = calc::page(&["R = rho_Cu·10 m", "R ="], &o);
        assert!(matches!(&r[1], Some(Out::Result { err: Some(_), .. })));
    }
}
