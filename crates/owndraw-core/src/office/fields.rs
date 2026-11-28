//! Anzeige von Feldern: Beschriftungsnummern, Querverweise, Zitate, Formularfelder, Fußnotenzeichen.
//! Der Satz, die Exporte und die Textausgabe nutzen dieselben Regeln.

use super::model::{Kind, Para, TextDoc};
use super::objects::{Field, RefShow, OBJ};
use std::collections::HashMap;

/// Nummern im Dokument (einmal je Satz berechnet).
#[derive(Clone, Debug, Default)]
pub struct Numbers {
    /// Je Absatz: Beschriftungs-Art und Nummer.
    pub caps: Vec<Option<(String, u32)>>,
    /// Absatz-ID → Absatzindex.
    pub ids: HashMap<u32, usize>,
}

impl Numbers {
    pub fn new(doc: &TextDoc) -> Numbers {
        let mut caps = Vec::with_capacity(doc.paras.len());
        let mut count: HashMap<String, u32> = HashMap::new();
        let mut ids = HashMap::new();
        for (i, p) in doc.paras.iter().enumerate() {
            if p.id > 0 {
                ids.insert(p.id, i);
            }
            caps.push((p.kind == Kind::Caption).then(|| {
                let label = if p.cap.trim().is_empty() { "Abbildung".to_string() } else { p.cap.trim().to_string() };
                let n = count.entry(label.clone()).or_insert(0);
                *n += 1;
                (label, *n)
            }));
        }
        Numbers { caps, ids }
    }

    /// „Abbildung 2“ für Beschriftungs-Absatz `pi`.
    pub fn caption(&self, pi: usize) -> Option<String> {
        self.caps.get(pi)?.as_ref().map(|(l, n)| format!("{l} {n}"))
    }

    /// Vorspann einer Beschriftung „Abbildung 2: “.
    pub fn caption_prefix(&self, pi: usize) -> Option<String> {
        self.caption(pi).map(|c| format!("{c}: "))
    }
}

/// Umgebung für die Anzeige eines Feldes.
pub struct FieldCx<'a> {
    pub doc: &'a TextDoc,
    pub nums: &'a Numbers,
    /// Seite (0-basiert) und Seitenzahl, falls bekannt.
    pub page: usize,
    pub pages: usize,
    /// Seite eines Absatzes (für Seitenverweise).
    pub para_page: &'a dyn Fn(usize) -> usize,
    /// Nummer der Fußnote (für `Field::Note`).
    pub note: u32,
}

/// Text, den ein Feld zeigt.
pub fn field_text(f: &Field, cx: &FieldCx) -> String {
    match f {
        Field::Page => (cx.page + 1).to_string(),
        Field::Pages => cx.pages.max(1).to_string(),
        Field::Date(d) => d.clone(),
        Field::Ref(id, show) => match cx.nums.ids.get(id) {
            None => "[Verweis fehlt]".into(),
            Some(&pi) => match show {
                RefShow::Page => ((cx.para_page)(pi) + 1).to_string(),
                RefShow::Text => para_plain(cx.doc, cx.nums, pi),
                RefShow::Label => cx.nums.caption(pi).unwrap_or_else(|| cx.doc.paras[pi].text().replace(OBJ, "").trim().to_string()),
            },
        },
        Field::Cite(id) => cx.doc.ex.source(*id).map(|s| s.cite()).unwrap_or_else(|| "(Quelle fehlt)".into()),
        Field::Merge(n) => format!("«{n}»"),
        Field::Check(b) => if *b { "☒" } else { "☐" }.into(),
        Field::Choice(o, i) => format!("{} ▾", o.get(*i).map(String::as_str).unwrap_or("Auswahl")),
        Field::Index(_) => String::new(),
        Field::Note(_) => cx.note.to_string(),
    }
}

/// Beschriftung samt Text bzw. Absatztext ohne Felder.
fn para_plain(doc: &TextDoc, nums: &Numbers, pi: usize) -> String {
    let p = &doc.paras[pi];
    let t: String = p.runs.iter().filter(|r| r.st.field.is_none() && r.st.del == 0).map(|r| r.text.as_str()).collect();
    match nums.caption_prefix(pi) {
        Some(pre) => format!("{pre}{t}"),
        None => t,
    }
}

/// Absatz als lesbarer Text (Felder ausgefüllt, gelöschte Änderungen weggelassen), für Exporte.
/// `note` zählt Fußnoten über das Dokument; Fußnotentexte landen in `notes`.
pub fn para_display(doc: &TextDoc, nums: &Numbers, pi: usize, note: &mut u32, notes: &mut Vec<(u32, String)>) -> String {
    let p: &Para = &doc.paras[pi];
    let mut s = nums.caption_prefix(pi).unwrap_or_default();
    let pp = |_: usize| 0usize;
    for r in &p.runs {
        if r.st.del > 0 {
            continue;
        }
        match &r.st.field {
            None => s.push_str(&r.text),
            Some(f) => {
                for _ in r.text.chars() {
                    if let Field::Note(t) = f {
                        *note += 1;
                        notes.push((*note, t.clone()));
                    }
                    let cx = FieldCx { doc, nums, page: 0, pages: 1, para_page: &pp, note: *note };
                    match f {
                        Field::Page | Field::Pages => s.push('#'),
                        _ => s.push_str(&field_text(f, &cx)),
                    }
                }
            }
        }
    }
    s
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::office::model::{CStyle, Pos};
    use crate::office::objects::Source;

    #[test]
    fn beschriftung_verweis_zitat() {
        let mut d = TextDoc::new();
        d.paras = vec![Para::with_text(Kind::Caption, "Haus"), Para::with_text(Kind::Caption, "Baum"), Para::with_text(Kind::Body, "Siehe ")];
        d.paras[1].id = 7;
        d.ex.sources.push(Source { id: 3, author: "Eva Kern".into(), year: "2021".into(), title: "T".into(), ..Source::default() });
        let o = d.paras[2].len();
        d.paras[2].insert(o, &OBJ.to_string(), &CStyle { field: Some(Field::Ref(7, RefShow::Label)), ..CStyle::default() });
        d.paras[2].insert(o + 1, &OBJ.to_string(), &CStyle { field: Some(Field::Cite(3)), ..CStyle::default() });
        d.paras[2].insert(o + 2, &OBJ.to_string(), &CStyle { field: Some(Field::Note("Fuß".into())), ..CStyle::default() });
        let n = Numbers::new(&d);
        assert_eq!(n.caption_prefix(1).as_deref(), Some("Abbildung 2: "));
        let (mut k, mut notes) = (0, Vec::new());
        assert_eq!(para_display(&d, &n, 2, &mut k, &mut notes), "Siehe Abbildung 2(Kern 2021)1");
        assert_eq!(notes, [(1, "Fuß".to_string())]);
        let _ = Pos::default();
    }
}
