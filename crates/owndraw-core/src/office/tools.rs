//! Werkzeuge auf dem ganzen Textdokument: Änderungen nachverfolgen (annehmen/ablehnen), Vergleichen, Versionen,
//! Seriendruck, Verzeichnisse (Literatur, Stichwörter, Abbildungen), Vorlagen, Umschläge und Etiketten.

use super::fields::Numbers;
use super::layout::Layout;
use super::model::{ordered, Align, CStyle, Kind, Para, PageSpec, Pos, Run, TextDoc};
use super::objects::{Cell, Field, Table, Version, OBJ};
use std::collections::BTreeMap;

// ---------------------------------------------------------------- Änderungen nachverfolgen

/// Text als nachverfolgte Einfügung einsetzen.
pub fn track_insert(doc: &mut TextDoc, pos: Pos, s: &str, st: &CStyle, rev: u32) -> Pos {
    let st = CStyle { ins: rev, del: 0, ..st.clone() };
    doc.insert_text(pos, s, &st)
}

/// Löschen bei nachverfolgten Änderungen: eigene Einfügungen verschwinden, anderer Text wird als gelöscht markiert.
/// Absatzgrenzen bleiben erhalten. Liefert die neue Schreibposition (Anfang des Bereichs).
pub fn track_delete(doc: &mut TextDoc, a: Pos, b: Pos, rev: u32) -> Pos {
    let (a, b) = ordered(doc.clamp(a), doc.clamp(b));
    if a == b {
        return a;
    }
    for p in (a.p..=b.p).rev() {
        let s = if p == a.p { a.o } else { 0 };
        let e = if p == b.p { b.o } else { doc.paras[p].len() };
        if s >= e {
            continue;
        }
        doc.paras[p].restyle(s, e, &|st: &mut CStyle| {
            if st.ins == 0 && st.del == 0 {
                st.del = rev;
            }
        });
        // Eingefügtes im Bereich wirklich löschen (von hinten)
        let chars = doc.paras[p].chars();
        let mut i = e;
        while i > s {
            let ri = chars[i - 1].1;
            if doc.paras[p].runs[ri].st.ins > 0 {
                let mut j = i - 1;
                while j > s && chars[j - 1].1 == ri {
                    j -= 1;
                }
                doc.paras[p].delete(j, i);
                i = j;
            } else {
                i -= 1;
            }
        }
    }
    a
}

/// Nimmt Änderungen an (`id` = eine bestimmte, None = alle).
pub fn accept(doc: &mut TextDoc, id: Option<u32>) -> usize {
    resolve_revs(doc, id, true)
}

/// Lehnt Änderungen ab.
pub fn reject(doc: &mut TextDoc, id: Option<u32>) -> usize {
    resolve_revs(doc, id, false)
}

fn resolve_revs(doc: &mut TextDoc, id: Option<u32>, accept: bool) -> usize {
    let hit = |r: u32| r > 0 && id.is_none_or(|x| x == r);
    let mut n = 0;
    let had: Vec<bool> = doc.paras.iter().map(|p| p.runs.iter().any(|r| hit(r.st.ins) || hit(r.st.del))).collect();
    for p in &mut doc.paras {
        let before = p.runs.len();
        p.runs.retain(|r| {
            let gone = if accept { hit(r.st.del) } else { hit(r.st.ins) };
            !gone
        });
        n += before - p.runs.len();
        for r in &mut p.runs {
            if hit(r.st.ins) {
                r.st.ins = 0;
                n += 1;
            }
            if hit(r.st.del) {
                r.st.del = 0;
                n += 1;
            }
        }
        p.normalize();
    }
    // Absätze, die nur aus Änderungen bestanden und nun leer sind, entfernen (mindestens einer bleibt)
    let mut i = 0;
    doc.paras.retain(|p| {
        let keep = !(had[i] && p.is_empty() && !p.kind.is_block());
        i += 1;
        keep
    });
    if doc.paras.is_empty() {
        doc.paras.push(Para::new(Kind::Body));
    }
    let used: std::collections::HashSet<u32> = doc.paras.iter().flat_map(|p| p.runs.iter().flat_map(|r| [r.st.ins, r.st.del])).collect();
    doc.ex.revs.retain(|r| used.contains(&r.id));
    n
}

/// Änderung an der Position (ID), falls vorhanden.
pub fn rev_at(doc: &TextDoc, pos: Pos) -> Option<u32> {
    let p = doc.paras.get(pos.p)?;
    let mut acc = 0;
    for r in &p.runs {
        let n = r.text.chars().count();
        if pos.o >= acc && pos.o <= acc + n && (r.st.ins > 0 || r.st.del > 0) {
            return Some(r.st.ins.max(r.st.del));
        }
        acc += n;
    }
    None
}

/// Alle Änderungen in Reihenfolge: (ID, Position, eingefügt?, Text).
pub fn changes(doc: &TextDoc) -> Vec<(u32, Pos, bool, String)> {
    let mut out: Vec<(u32, Pos, bool, String)> = Vec::new();
    for (pi, p) in doc.paras.iter().enumerate() {
        let mut acc = 0;
        for r in &p.runs {
            let id = r.st.ins.max(r.st.del);
            if id > 0 {
                let ins = r.st.ins > 0;
                match out.last_mut() {
                    Some(l) if l.0 == id && l.2 == ins && l.1.p == pi => l.3.push_str(&r.text),
                    _ => out.push((id, Pos::new(pi, acc), ins, r.text.clone())),
                }
            }
            acc += r.text.chars().count();
        }
    }
    out
}

// ---------------------------------------------------------------- Vergleichen

fn lcs<T: PartialEq>(a: &[T], b: &[T]) -> Vec<(usize, usize)> {
    let (n, m) = (a.len(), b.len());
    if n == 0 || m == 0 || n * m > 9_000_000 {
        return Vec::new();
    }
    let mut dp = vec![0u32; (n + 1) * (m + 1)];
    for i in (0..n).rev() {
        for j in (0..m).rev() {
            dp[i * (m + 1) + j] = if a[i] == b[j] { dp[(i + 1) * (m + 1) + j + 1] + 1 } else { dp[(i + 1) * (m + 1) + j].max(dp[i * (m + 1) + j + 1]) };
        }
    }
    let (mut i, mut j, mut out) = (0, 0, Vec::new());
    while i < n && j < m {
        if a[i] == b[j] {
            out.push((i, j));
            i += 1;
            j += 1;
        } else if dp[(i + 1) * (m + 1) + j] >= dp[i * (m + 1) + j + 1] {
            i += 1;
        } else {
            j += 1;
        }
    }
    out
}

fn tokens(s: &str) -> Vec<String> {
    let mut v: Vec<String> = Vec::new();
    for c in s.chars() {
        let word = c.is_alphanumeric();
        match v.last_mut() {
            Some(l) if word && l.chars().last().is_some_and(char::is_alphanumeric) => l.push(c),
            _ => v.push(c.to_string()),
        }
    }
    v
}

fn marked(text: &str, ins: bool, rev: u32) -> Run {
    Run { text: text.into(), st: CStyle { ins: if ins { rev } else { 0 }, del: if ins { 0 } else { rev }, ..CStyle::default() } }
}

fn para_marked(p: &Para, ins: bool, rev: u32) -> Para {
    let mut q = p.clone();
    for r in &mut q.runs {
        if ins {
            r.st.ins = rev;
        } else {
            r.st.del = rev;
        }
    }
    q
}

/// Vergleicht zwei Fassungen: Ergebnis ist die neue Fassung mit allen Unterschieden als nachverfolgte Änderungen.
pub fn compare(old: &TextDoc, new: &TextDoc, author: &str, time: &str) -> TextDoc {
    let mut out = new.clone();
    let rev = out.ex.rev(author, time);
    let ta: Vec<String> = old.paras.iter().map(Para::text).collect();
    let tb: Vec<String> = new.paras.iter().map(Para::text).collect();
    let pairs = lcs(&ta, &tb);
    let mut paras = Vec::new();
    let (mut i, mut j) = (0, 0);
    let flush = |i0: usize, i1: usize, j0: usize, j1: usize, paras: &mut Vec<Para>| {
        let k = (i1 - i0).min(j1 - j0);
        for d in 0..k {
            // Absatz geändert: Wortvergleich
            let (pa, pb) = (&old.paras[i0 + d], &new.paras[j0 + d]);
            if pa.kind.is_block() || pb.kind.is_block() {
                paras.push(para_marked(pa, false, rev));
                paras.push(para_marked(pb, true, rev));
                continue;
            }
            let (wa, wb) = (tokens(&ta[i0 + d]), tokens(&tb[j0 + d]));
            let m = lcs(&wa, &wb);
            let mut q = Para { runs: Vec::new(), ..pb.clone() };
            let (mut x, mut y) = (0, 0);
            let push = |x0: usize, x1: usize, y0: usize, y1: usize, q: &mut Para| {
                let del: String = wa[x0..x1].concat();
                let ins: String = wb[y0..y1].concat();
                if !del.is_empty() {
                    q.runs.push(marked(&del, false, rev));
                }
                if !ins.is_empty() {
                    q.runs.push(marked(&ins, true, rev));
                }
            };
            for (a, b) in m.iter().copied().chain(std::iter::once((wa.len(), wb.len()))) {
                push(x, a, y, b, &mut q);
                if a < wa.len() && b < wb.len() {
                    q.runs.push(Run { text: wb[b].clone(), st: CStyle::default() });
                }
                x = a + 1;
                y = b + 1;
            }
            q.normalize();
            paras.push(q);
        }
        for p in &old.paras[i0 + k..i1] {
            paras.push(para_marked(p, false, rev));
        }
        for p in &new.paras[j0 + k..j1] {
            paras.push(para_marked(p, true, rev));
        }
    };
    for (a, b) in pairs.iter().copied().chain(std::iter::once((ta.len(), tb.len()))) {
        flush(i, a, j, b, &mut paras);
        if a < ta.len() && b < tb.len() {
            paras.push(new.paras[b].clone());
        }
        i = a + 1;
        j = b + 1;
    }
    if paras.is_empty() {
        paras.push(Para::new(Kind::Body));
    }
    out.paras = paras;
    if !changes(&out).is_empty() {
        out.ex.track = true;
    } else {
        out.ex.revs.retain(|r| r.id != rev);
    }
    out
}

// ---------------------------------------------------------------- Versionen

/// Höchstzahl gespeicherter Versionen.
pub const MAX_VERSIONS: usize = 30;

/// Speichert den aktuellen Stand als Version.
pub fn save_version(doc: &mut TextDoc, name: &str, time: &str) {
    let mut copy = doc.clone();
    copy.ex.versions.clear();
    doc.ex.versions.push(Version { name: name.into(), time: time.into(), data: copy.to_json() });
    while doc.ex.versions.len() > MAX_VERSIONS {
        doc.ex.versions.remove(0);
    }
}

/// Inhalt einer Version (ohne die Versionsliste).
pub fn version_doc(doc: &TextDoc, i: usize) -> Option<TextDoc> {
    doc.ex.versions.get(i).map(|v| TextDoc::from_json(&v.data))
}

/// Stellt Version `i` wieder her; die Versionsliste bleibt, der bisherige Stand wird vorher gesichert.
pub fn restore_version(doc: &mut TextDoc, i: usize, time: &str) -> bool {
    let Some(old) = version_doc(doc, i) else { return false };
    save_version(doc, "Vor dem Wiederherstellen", time);
    let versions = std::mem::take(&mut doc.ex.versions);
    *doc = old;
    doc.ex.versions = versions;
    true
}

// ---------------------------------------------------------------- Seriendruck

/// Spaltennamen der Seriendruckfelder im Dokument.
pub fn merge_fields(doc: &TextDoc) -> Vec<String> {
    let mut v: Vec<String> = Vec::new();
    for p in &doc.paras {
        for r in &p.runs {
            if let Some(Field::Merge(n)) = &r.st.field {
                if !v.contains(n) {
                    v.push(n.clone());
                }
            }
        }
    }
    v
}

/// Dokument für Datensatz `row`: Seriendruckfelder werden durch die Werte ersetzt.
pub fn merge_row(doc: &TextDoc, row: usize) -> TextDoc {
    let mut out = doc.clone();
    let data = doc.ex.merge.clone().unwrap_or_default();
    for p in &mut out.paras {
        for r in &mut p.runs {
            if let Some(Field::Merge(n)) = r.st.field.clone() {
                let v = data.value(row, &n).unwrap_or("").to_string();
                r.text = v;
                r.st.field = None;
            }
        }
        p.normalize();
    }
    out.ex.merge = None;
    out.ex.versions.clear();
    out
}

/// Alle Datensätze hintereinander, getrennt durch Seitenumbrüche.
pub fn merge_all(doc: &TextDoc) -> TextDoc {
    let n = doc.ex.merge.as_ref().map(|m| m.rows.len()).unwrap_or(0);
    if n == 0 {
        return merge_row(doc, usize::MAX);
    }
    let mut out = merge_row(doc, 0);
    for i in 1..n {
        out.paras.push(Para::new(Kind::PageBreak));
        out.paras.extend(merge_row(doc, i).paras);
    }
    out
}

// ---------------------------------------------------------------- Verzeichnisse

/// Literaturverzeichnis aus den Quellen.
pub fn bibliography(doc: &TextDoc) -> Vec<Para> {
    let mut v = vec![Para::with_text(Kind::H1, "Literaturverzeichnis")];
    let mut src = doc.ex.sources.clone();
    src.sort_by_key(|s| s.author.to_lowercase());
    if src.is_empty() {
        v.push(Para::with_text(Kind::Body, "Noch keine Quellen – füge sie über „Einfügen › Zitat“ hinzu."));
    }
    for s in src {
        v.push(Para::with_text(Kind::Body, &s.entry()));
    }
    v.push(Para::new(Kind::Body));
    v
}

/// Stichwortverzeichnis: Begriffe mit Seitenzahlen, nach Anfangsbuchstaben gruppiert.
pub fn index(doc: &TextDoc, lay: &Layout) -> Vec<Para> {
    let mut terms: BTreeMap<String, (String, Vec<usize>)> = BTreeMap::new();
    for (pi, p) in doc.paras.iter().enumerate() {
        let mut acc = 0;
        for r in &p.runs {
            if let Some(Field::Index(t)) = &r.st.field {
                let page = lay.lines.get(lay.line_of(Pos::new(pi, acc))).map(|l| l.page + 1).unwrap_or(1);
                let e = terms.entry(t.to_lowercase()).or_insert_with(|| (t.clone(), Vec::new()));
                if !e.1.contains(&page) {
                    e.1.push(page);
                }
            }
            acc += r.text.chars().count();
        }
    }
    let mut v = vec![Para::with_text(Kind::H1, "Stichwortverzeichnis")];
    if terms.is_empty() {
        v.push(Para::with_text(Kind::Body, "Noch keine Stichwörter – markiere ein Wort und wähle „Einfügen › Stichwort“."));
    }
    let mut letter = None;
    for (k, (t, pages)) in terms {
        let l = k.chars().next().map(|c| c.to_uppercase().collect::<String>());
        if l != letter {
            v.push(Para::with_text(Kind::H3, l.as_deref().unwrap_or("")));
            letter = l;
        }
        let pages: Vec<String> = pages.iter().map(|p| p.to_string()).collect();
        v.push(Para::with_text(Kind::Body, &format!("{t}\t{}", pages.join(", "))));
    }
    v.push(Para::new(Kind::Body));
    v
}

/// Verzeichnis der Beschriftungen einer Art („Abbildung“, „Tabelle“).
pub fn figures(doc: &TextDoc, lay: &Layout, label: &str) -> Vec<Para> {
    let nums = Numbers::new(doc);
    let title = match label {
        "Tabelle" => "Tabellenverzeichnis",
        "Formel" => "Formelverzeichnis",
        _ => "Abbildungsverzeichnis",
    };
    let mut v = vec![Para::with_text(Kind::H1, title)];
    for (pi, p) in doc.paras.iter().enumerate() {
        if let Some((l, _)) = &nums.caps[pi] {
            if l == label {
                let t: String = p.runs.iter().filter(|r| r.st.field.is_none()).map(|r| r.text.as_str()).collect();
                v.push(Para::with_text(Kind::Body, &format!("{}{}\t{}", nums.caption_prefix(pi).unwrap_or_default(), t, lay.page_of_para(pi) + 1)));
            }
        }
    }
    if v.len() == 1 {
        v.push(Para::with_text(Kind::Body, &format!("Noch keine Beschriftung „{label}“ – wähle „Einfügen › Beschriftung“.")));
    }
    v.push(Para::new(Kind::Body));
    v
}

/// Datumsfelder auf `today` setzen (Felder aktualisieren).
pub fn update_fields(doc: &mut TextDoc, today: &str) -> usize {
    let mut n = 0;
    for p in &mut doc.paras {
        for r in &mut p.runs {
            if let Some(Field::Date(d)) = &mut r.st.field {
                if d != today {
                    *d = today.into();
                    n += 1;
                }
            }
        }
    }
    n
}

/// Fügt ein Feld als Platzhalter ein und liefert die Position dahinter.
pub fn insert_field(doc: &mut TextDoc, pos: Pos, f: Field, st: &CStyle) -> Pos {
    let st = CStyle { field: Some(f), link: None, ..st.clone() };
    let pos = doc.clamp(pos);
    if doc.paras[pos.p].kind.is_block() {
        doc.paras.insert(pos.p + 1, Para::new(Kind::Body));
        let p = Pos::new(pos.p + 1, 0);
        doc.paras[p.p].insert(0, &OBJ.to_string(), &st);
        return Pos::new(p.p, 1);
    }
    doc.paras[pos.p].insert(pos.o, &OBJ.to_string(), &st);
    Pos::new(pos.p, pos.o + 1)
}

/// Feld an Position (Zeichen hinter `pos`).
pub fn field_at(doc: &TextDoc, pos: Pos) -> Option<Field> {
    let p = doc.paras.get(pos.p)?;
    let mut acc = 0;
    for r in &p.runs {
        let n = r.text.chars().count();
        if pos.o >= acc && pos.o < acc + n {
            return r.st.field.clone();
        }
        acc += n;
    }
    None
}

/// Ersetzt das Feld am Zeichen hinter `pos`.
pub fn set_field_at(doc: &mut TextDoc, pos: Pos, f: Field) {
    let f2 = f.clone();
    if let Some(p) = doc.paras.get_mut(pos.p) {
        p.restyle(pos.o, pos.o + 1, &move |s: &mut CStyle| s.field = Some(f2.clone()));
    }
}

// ---------------------------------------------------------------- Vorlagen, Umschläge, Etiketten

/// Namen der Vorlagen für neue Dokumente.
pub const TEMPLATES: [&str; 7] = ["Leer", "Brief", "Bericht", "Hausarbeit", "Lebenslauf", "Protokoll", "Rechnung"];

fn p(k: Kind, t: &str) -> Para {
    Para::with_text(k, t)
}

/// Neues Dokument aus Vorlage.
pub fn template(name: &str) -> TextDoc {
    let mut d = TextDoc::new();
    let w = d.page.content_w();
    d.paras = match name {
        "Brief" => vec![
            Para { align: Align::Right, ..p(Kind::Body, "Vorname Nachname · Straße 1 · 12345 Ort") },
            p(Kind::Body, ""),
            p(Kind::Body, "Empfänger GmbH"),
            p(Kind::Body, "Frau Beispiel"),
            p(Kind::Body, "Musterweg 2"),
            p(Kind::Body, "54321 Stadt"),
            p(Kind::Body, ""),
            Para { align: Align::Right, ..p(Kind::Body, "Ort, den ") },
            p(Kind::Body, ""),
            Para { runs: vec![Run { text: "Betreff".into(), st: CStyle { b: true, ..CStyle::default() } }], ..p(Kind::Body, "") },
            p(Kind::Body, "Sehr geehrte Damen und Herren,"),
            p(Kind::Body, "hier steht Ihr Text."),
            p(Kind::Body, "Mit freundlichen Grüßen"),
            p(Kind::Body, ""),
            p(Kind::Body, "Vorname Nachname"),
        ],
        "Bericht" => vec![p(Kind::Title, "Berichtstitel"), p(Kind::Body, "Untertitel · Datum"), p(Kind::H1, "Zusammenfassung"), p(Kind::Body, "Kurz das Wichtigste."), p(Kind::H1, "Ausgangslage"), p(Kind::Body, ""), p(Kind::H1, "Ergebnisse"), p(Kind::Body, ""), p(Kind::H1, "Nächste Schritte"), p(Kind::Bullet, "")],
        "Hausarbeit" => {
            d.ex.cols = 1;
            d.footer = "Seite {seite} von {seiten}".into();
            vec![
                Para { align: Align::Center, ..p(Kind::Title, "Titel der Arbeit") },
                Para { align: Align::Center, ..p(Kind::Body, "Name · Matrikelnummer · Seminar · Semester") },
                Para::new(Kind::PageBreak),
                p(Kind::H1, "Einleitung"),
                p(Kind::Body, "Fragestellung und Aufbau der Arbeit."),
                p(Kind::H1, "Hauptteil"),
                p(Kind::H2, "Abschnitt"),
                p(Kind::Body, ""),
                p(Kind::H1, "Fazit"),
                p(Kind::Body, ""),
            ]
        }
        "Lebenslauf" => {
            let mut t = Table::new(4, 2, w);
            t.cols = vec![w * 0.28, w * 0.72];
            t.border = 0;
            for (i, (a, b)) in [("seit 2024", "Stelle, Firma, Ort"), ("2020 – 2024", "Studium, Hochschule"), ("2012 – 2020", "Abitur, Schule"), ("Sprachen", "Deutsch, Englisch")].iter().enumerate() {
                t.rows[i][0] = Cell { b: true, ..Cell::new(a) };
                t.rows[i][1] = Cell::new(b);
            }
            vec![p(Kind::Title, "Vorname Nachname"), p(Kind::Body, "Straße 1 · 12345 Ort · mail@beispiel.de · 0123 456789"), p(Kind::H1, "Lebenslauf"), Para { table: Some(t), ..Para::new(Kind::Table) }, p(Kind::Body, "")]
        }
        "Protokoll" => vec![p(Kind::Title, "Protokoll"), p(Kind::Body, "Datum: · Ort: · Leitung: · Protokoll:"), p(Kind::H2, "Teilnehmende"), p(Kind::Bullet, ""), p(Kind::H2, "Tagesordnung"), p(Kind::Number, ""), p(Kind::H2, "Beschlüsse"), p(Kind::Bullet, ""), p(Kind::H2, "Aufgaben"), p(Kind::Bullet, "")],
        "Rechnung" => {
            let mut t = Table::new(4, 4, w);
            t.cols = vec![w * 0.5, w * 0.14, w * 0.18, w * 0.18];
            t.header = true;
            for (c, h) in ["Leistung", "Menge", "Einzelpreis", "Betrag"].iter().enumerate() {
                t.rows[0][c] = Cell::new(h);
            }
            t.rows[3][0] = Cell { b: true, ..Cell::new("Summe") };
            for r in &mut t.rows {
                for c in r.iter_mut().skip(1) {
                    c.align = Align::Right;
                }
            }
            vec![p(Kind::Title, "Rechnung"), p(Kind::Body, "Rechnungsnummer: · Datum:"), p(Kind::Body, "Kunde:"), Para { table: Some(t), ..Para::new(Kind::Table) }, p(Kind::Body, "Zahlbar innerhalb von 14 Tagen ohne Abzug.")]
        }
        _ => vec![Para::new(Kind::Body)],
    };
    d
}

/// Briefumschlag DIN lang (Absender oben links, Empfänger im Fenster).
pub fn envelope(sender: &str, recipient: &str) -> TextDoc {
    let mut d = TextDoc::new();
    d.page = PageSpec { w: 623.6, h: 311.8, ml: 28.0, mr: 28.0, mt: 28.0, mb: 20.0 };
    d.family = super::fonts::Family::Sans;
    let mut v: Vec<Para> = sender.lines().map(|l| Para { runs: vec![Run { text: l.into(), st: CStyle { size: Some(8.0), ..CStyle::default() } }], ..Para::new(Kind::Body) }).collect();
    for _ in 0..4 {
        v.push(Para::new(Kind::Body));
    }
    for l in recipient.lines() {
        v.push(Para { indent: 8, runs: vec![Run { text: l.into(), st: CStyle { size: Some(12.0), ..CStyle::default() } }], ..Para::new(Kind::Body) });
    }
    d.paras = v;
    d
}

/// Etikettenbogen A4: `cols` × `rows` gleiche Etiketten.
pub fn labels(text: &str, cols: usize, rows: usize) -> TextDoc {
    let mut d = TextDoc::new();
    d.page = PageSpec { ml: 14.0, mr: 14.0, mt: 30.0, mb: 20.0, ..PageSpec::default() };
    let (cols, rows) = (cols.clamp(1, 6), rows.clamp(1, 20));
    let mut t = Table::new(rows, cols, d.page.content_w());
    t.border = 0xFFE5E7EB;
    for r in &mut t.rows {
        for c in r.iter_mut() {
            c.text = text.to_string();
            c.align = Align::Center;
        }
    }
    // Zeilen gleich hoch: Leerzeilen auffüllen
    let lines = text.lines().count().max(1);
    let want = ((d.page.h - d.page.mt - d.page.mb) / rows as f32 / 14.0).floor() as usize;
    if want > lines {
        let pad = "\n".repeat((want - lines) / 2);
        for r in &mut t.rows {
            for c in r.iter_mut() {
                c.text = format!("{pad}{}{pad}", c.text);
            }
        }
    }
    d.paras = vec![Para { table: Some(t), ..Para::new(Kind::Table) }];
    d
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::office::layout::layout;
    use crate::office::objects::{MergeData, Obj, Shape, Source, Wrap};

    #[test]
    fn nachverfolgen_annehmen_ablehnen() {
        let mut d = TextDoc::new();
        d.insert_text(Pos::default(), "Hallo Welt", &CStyle::default());
        let rev = d.ex.rev("Le", "2026-11-28T10:00:00Z");
        let p = track_insert(&mut d, Pos::new(0, 5), " schöne", &CStyle::default(), rev);
        assert_eq!(p, Pos::new(0, 12));
        track_delete(&mut d, Pos::new(0, 0), Pos::new(0, 8), rev);
        // „Hallo“ gelöscht markiert, „ sc“ wirklich weg
        assert_eq!(d.plain_text(), "Hallohöne Welt");
        assert_eq!(changes(&d).len(), 2);
        let mut a = d.clone();
        accept(&mut a, None);
        assert_eq!(a.plain_text(), "höne Welt");
        reject(&mut d, None);
        assert_eq!(d.plain_text(), "Hallo Welt");
        assert!(d.ex.revs.is_empty());
    }

    #[test]
    fn vergleichen_und_versionen() {
        let mut a = TextDoc::new();
        a.insert_text(Pos::default(), "Der Hund schläft.\nZweiter Absatz.", &CStyle::default());
        let mut b = a.clone();
        b.paras[0].runs[0].text = "Der Kater schläft.".into();
        b.paras.push(Para::with_text(Kind::Body, "Neu."));
        let c = compare(&a, &b, "Le", "t");
        let ch = changes(&c);
        assert!(ch.iter().any(|x| !x.2 && x.3 == "Hund"));
        assert!(ch.iter().any(|x| x.2 && x.3 == "Kater"));
        assert!(ch.iter().any(|x| x.2 && x.3 == "Neu."));
        let mut acc = c.clone();
        accept(&mut acc, None);
        assert_eq!(acc.plain_text(), b.plain_text());
        let mut v = a.clone();
        save_version(&mut v, "eins", "t1");
        v.paras[0].runs[0].text = "anders".into();
        assert!(restore_version(&mut v, 0, "t2"));
        assert_eq!(v.plain_text(), a.plain_text());
        assert_eq!(v.ex.versions.len(), 2);
    }

    #[test]
    fn seriendruck() {
        let mut d = TextDoc::new();
        d.insert_text(Pos::default(), "Hallo ", &CStyle::default());
        let p = insert_field(&mut d, Pos::new(0, 6), Field::Merge("Name".into()), &CStyle::default());
        d.insert_text(p, "!", &CStyle::default());
        d.ex.merge = Some(MergeData::from_csv("Name\nAnna\nBob"));
        assert_eq!(merge_fields(&d), ["Name"]);
        assert_eq!(merge_row(&d, 1).plain_text(), "Hallo Bob!");
        let all = merge_all(&d);
        assert_eq!(all.paras.len(), 3);
        assert_eq!(all.plain_text(), "Hallo Anna!\n\nHallo Bob!");
    }

    #[test]
    fn satz_mit_tabelle_objekt_fussnote_spalten() {
        let mut d = template("Rechnung");
        let w = d.page.content_w();
        d.paras.push(Para { obj: Some(Obj::shape(Shape::Star)), wrap: Wrap::Left, ..Para::new(Kind::Object) });
        d.paras.push(Para::with_text(Kind::Body, &"Text fließt neben dem Stern vorbei. ".repeat(12)));
        let n = d.paras.len() - 1;
        let o = d.paras[n].len();
        insert_field(&mut d, Pos::new(n, o), Field::Note("Eine Fußnote.".into()), &CStyle::default());
        let l = layout(&d);
        // Tabelle: 4 Zeilen
        assert_eq!(l.lines.iter().filter(|x| x.row.is_some()).count(), 4);
        // Umfluss: erste Textzeile beginnt rechts neben dem Stern
        let star = l.lines.iter().find(|x| x.obj.is_some()).unwrap();
        let (sx, sw, _) = star.obj.unwrap();
        let first = l.lines.iter().find(|x| x.para == n).unwrap();
        assert!(first.xs[0] >= sx + sw, "{} {}", first.xs[0], sx + sw);
        assert!(first.segs.last().unwrap().x1 <= d.page.ml + w + 0.5);
        // Fußnote unten auf Seite 1
        assert_eq!(l.notes.len(), 1);
        assert!(l.notes[0].y > d.page.h / 2.0);
        let pdf = crate::office::render::pdf(&d, &l, "t", &Default::default());
        assert!(pdf.starts_with(b"%PDF"));
        // Spalten
        let mut c = TextDoc::new();
        c.ex.cols = 2;
        c.insert_text(Pos::default(), &"Spaltentext mit vielen Wörtern. ".repeat(400), &CStyle::default());
        let lc = layout(&c);
        assert!(lc.lines.iter().any(|x| x.xs[0] > c.page.ml + c.page.content_w() / 2.0));
        assert!(lc.lines.iter().all(|x| x.xs.last().unwrap() - x.xs[0] <= lc.cols.1 + 0.5));
    }

    #[test]
    fn verzeichnisse_und_felder() {
        let mut d = TextDoc::new();
        d.ex.sources.push(Source { id: 1, author: "B, Bert".into(), title: "Zwei".into(), ..Source::default() });
        d.ex.sources.push(Source { id: 2, author: "A, Anna".into(), title: "Eins".into(), ..Source::default() });
        let b = bibliography(&d);
        assert!(b[1].text().starts_with("A, Anna"));
        d.insert_text(Pos::default(), "Wort", &CStyle::default());
        insert_field(&mut d, Pos::new(0, 4), Field::Index("Apfel".into()), &CStyle::default());
        insert_field(&mut d, Pos::new(0, 0), Field::Page, &CStyle::default());
        let l = layout(&d);
        let ix = index(&d, &l);
        assert_eq!(ix[2].text(), "Apfel\t1");
        assert_eq!(l.lines[0].segs[0].words[0].1, "1");
        assert_eq!(field_at(&d, Pos::new(0, 0)), Some(Field::Page));
        for t in TEMPLATES {
            assert!(!template(t).paras.is_empty());
        }
        assert_eq!(labels("A\nB", 3, 8).paras[0].table.as_ref().unwrap().rows.len(), 8);
        assert!(envelope("a", "b").page.w > 600.0);
    }
}
