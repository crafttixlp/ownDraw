//! Handschrift-Erkennung (Werkzeug „Umwandeln“): plattformneutrale Teile.
//!
//! Die Erkennung selbst läuft nur im Browser (onnxruntime-web, `crates/owndraw-web/hw.js`). Hier liegt alles,
//! was sich ohne Modell prüfen lässt:
//! - [`Manifest`]: Beschreibung eines Modell-Pakets (`data/models/<id>/manifest.json` am Server, auch die App liest es)
//! - [`render`]: ausgewählte Striche → Graustufenbild mit normierter Strichdicke und Rand
//! - [`Kontext`], [`neu_bewerten`], [`vorauswahl`]: Alternativen mit dem Wissen der Seite (bekannte Variablen,
//!   Einheiten, Wörter) neu ordnen und den passenden Reiter (Text / Formel / Zahl) vorwählen
//!
//! Auswahl (Lasso/Antippen) und Ersetzen der Striche sind Methoden von [`crate::board::Board`] (`board/hw.rs`).

use crate::geom::RectF;
use crate::item::{Item, Kind};
use serde_json::Value;
use tiny_skia::{LineCap, LineJoin, Paint, PathBuilder, Pixmap, Stroke, Transform};

/// Art der Erkennung (Reiter im Fenster).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Art {
    Text,
    Formel,
    Zahl,
}

impl Art {
    pub const ALL: [Art; 3] = [Art::Text, Art::Formel, Art::Zahl];

    /// Kennung im Manifest (`"art"`).
    pub fn id(self) -> &'static str {
        match self {
            Art::Text => "text",
            Art::Formel => "formel",
            Art::Zahl => "zahl",
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Art::Text => "Text",
            Art::Formel => "Formel",
            Art::Zahl => "Zahl",
        }
    }

    pub fn from_id(s: &str) -> Option<Art> {
        Art::ALL.into_iter().find(|a| a.id() == s)
    }
}

/// Stufe der Erkennung (Einstellung „Handschrift-Erkennung“). Höher = größere Modelle, bessere Erkennung.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Stufe {
    Aus,
    Schlecht,
    Mittel,
    Hoch,
}

impl Stufe {
    pub const ALL: [Stufe; 4] = [Stufe::Aus, Stufe::Schlecht, Stufe::Mittel, Stufe::Hoch];

    pub fn id(self) -> &'static str {
        match self {
            Stufe::Aus => "aus",
            Stufe::Schlecht => "schlecht",
            Stufe::Mittel => "mittel",
            Stufe::Hoch => "hoch",
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Stufe::Aus => "Aus",
            Stufe::Schlecht => "Schlecht",
            Stufe::Mittel => "Mittel",
            Stufe::Hoch => "Hoch",
        }
    }

    pub fn from_id(s: &str) -> Option<Stufe> {
        Stufe::ALL.into_iter().find(|a| a.id() == s)
    }
}

// ---------------------------------------------------------------- Manifest

/// Lizenzen, die Pakete haben dürfen (keine NC-/Copyleft-Modelle).
pub const LIZENZEN: [&str; 6] = ["MIT", "Apache-2.0", "AFL-3.0", "BSD-2-Clause", "BSD-3-Clause", "CC0-1.0"];

/// Eine Datei des Pakets.
#[derive(Clone, Debug, PartialEq)]
pub struct Datei {
    pub name: String,
    pub groesse: u64,
    /// SHA-256 (64 Hex-Zeichen, klein).
    pub sha256: String,
    /// Herkunft zum Nachladen (Admin: „aus URL laden“), sonst leer.
    pub url: String,
}

/// Beschreibung eines Modell-Pakets (`manifest.json`). Unbekannte Felder (Vorverarbeitung, Modell, Tokenizer) liest
/// nur die Browser-Seite; sie bleiben in [`Manifest::raw`].
#[derive(Clone, Debug, PartialEq)]
pub struct Manifest {
    pub id: String,
    pub name: String,
    /// `text`, `formel`, `zahl` oder `laufzeit` (onnxruntime-web).
    pub art: String,
    pub stufen: Vec<Stufe>,
    pub version: String,
    pub dateien: Vec<Datei>,
    pub lizenz: String,
    /// Lizenztext im Paket (Dateiname), sonst leer.
    pub lizenz_datei: String,
    pub lizenz_hinweis: String,
    pub quelle: String,
    pub raw: Value,
}

/// Erlaubte Namen für Paket-IDs und Dateien: Buchstaben, Ziffern, `-`, `_`, `.`; nicht mit `.` beginnend.
pub fn safe_name(s: &str) -> bool {
    !s.is_empty() && s.len() <= 100 && !s.starts_with('.') && s.chars().all(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.'))
}

impl Manifest {
    /// Liest und prüft ein Manifest.
    pub fn parse(bytes: &[u8]) -> Result<Manifest, String> {
        let v: Value = serde_json::from_slice(bytes).map_err(|e| format!("manifest.json ist kein gültiges JSON: {e}"))?;
        Manifest::from_value(v)
    }

    pub fn from_value(v: Value) -> Result<Manifest, String> {
        let s = |k: &str| v.get(k).and_then(|x| x.as_str()).unwrap_or("").trim().to_string();
        let id = s("id");
        if !safe_name(&id) {
            return Err(format!("Ungültige Paket-ID „{id}“ (erlaubt: Buchstaben, Ziffern, - _ .)"));
        }
        let art = s("art");
        if Art::from_id(&art).is_none() && art != "laufzeit" {
            return Err(format!("Unbekannte Art „{art}“ (text, formel, zahl oder laufzeit)"));
        }
        // "stufen": ["mittel", "hoch"] oder "stufe": "mittel"
        let mut stufen: Vec<Stufe> = match v.get("stufen").or_else(|| v.get("stufe")) {
            Some(Value::Array(a)) => a.iter().filter_map(|x| x.as_str().and_then(Stufe::from_id)).collect(),
            Some(Value::String(x)) => Stufe::from_id(x).into_iter().collect(),
            _ => Vec::new(),
        };
        stufen.retain(|x| *x != Stufe::Aus);
        stufen.sort();
        stufen.dedup();
        if stufen.is_empty() {
            return Err("Keine Stufe angegeben (schlecht, mittel oder hoch)".into());
        }
        let lic = v.get("lizenz");
        let lizenz = match lic {
            Some(Value::String(x)) => x.trim().to_string(),
            Some(o) => o.get("spdx").and_then(|x| x.as_str()).unwrap_or("").trim().to_string(),
            None => String::new(),
        };
        if !LIZENZEN.contains(&lizenz.as_str()) {
            return Err(format!("Lizenz „{lizenz}“ ist nicht erlaubt (nur {})", LIZENZEN.join(", ")));
        }
        let lget = |k: &str| lic.and_then(|o| o.get(k)).and_then(|x| x.as_str()).unwrap_or("").to_string();
        let mut dateien = Vec::new();
        for d in v.get("dateien").and_then(|x| x.as_array()).ok_or("Liste „dateien“ fehlt")? {
            let name = d.get("name").and_then(|x| x.as_str()).unwrap_or("").to_string();
            if !safe_name(&name) || name == "manifest.json" {
                return Err(format!("Ungültiger Dateiname „{name}“"));
            }
            let sha = d.get("sha256").and_then(|x| x.as_str()).unwrap_or("").to_ascii_lowercase();
            if sha.len() != 64 || !sha.chars().all(|c| c.is_ascii_hexdigit()) {
                return Err(format!("{name}: sha256 fehlt oder ist ungültig"));
            }
            let groesse = d.get("groesse").and_then(|x| x.as_u64()).ok_or_else(|| format!("{name}: Größe fehlt"))?;
            if dateien.iter().any(|x: &Datei| x.name == name) {
                return Err(format!("{name}: doppelt"));
            }
            let url = d.get("url").and_then(|x| x.as_str()).unwrap_or("").to_string();
            dateien.push(Datei { name, groesse, sha256: sha, url });
        }
        if dateien.is_empty() {
            return Err("Das Paket enthält keine Dateien".into());
        }
        let lizenz_datei = lget("datei");
        if !lizenz_datei.is_empty() && !dateien.iter().any(|d| d.name == lizenz_datei) {
            return Err(format!("Lizenzdatei {lizenz_datei} fehlt in „dateien“"));
        }
        let quelle = match v.get("quelle") {
            Some(Value::String(x)) => x.clone(),
            Some(o) => o.get("url").and_then(|x| x.as_str()).unwrap_or("").to_string(),
            None => String::new(),
        };
        Ok(Manifest {
            name: Some(s("name")).filter(|n| !n.is_empty()).unwrap_or_else(|| id.clone()),
            id,
            art,
            stufen,
            version: s("version"),
            dateien,
            lizenz,
            lizenz_datei,
            lizenz_hinweis: lget("hinweis"),
            quelle,
            raw: v,
        })
    }

    /// Summe der Dateigrößen.
    pub fn groesse(&self) -> u64 {
        self.dateien.iter().map(|d| d.groesse).sum()
    }

    pub fn art(&self) -> Option<Art> {
        Art::from_id(&self.art)
    }
}

/// Paket für Art und Stufe: eines mit genau dieser Stufe, sonst das nächstniedrigere, sonst das nächsthöhere.
pub fn paket_fuer(pakete: &[Manifest], art: Art, stufe: Stufe) -> Option<&Manifest> {
    if stufe == Stufe::Aus {
        return None;
    }
    let mit = |s: Stufe| pakete.iter().find(|m| m.art() == Some(art) && m.stufen.contains(&s));
    if let Some(m) = mit(stufe) {
        return Some(m);
    }
    let rang = |s: Stufe| s as i32;
    let mut best: Option<(i32, &Manifest)> = None;
    for m in pakete.iter().filter(|m| m.art() == Some(art)) {
        for s in &m.stufen {
            // niedriger zuerst (kleiner Abstand, niedrigere Stufe bevorzugt)
            let d = rang(*s) - rang(stufe);
            let key = if d < 0 { -d * 2 } else { d * 2 + 1 };
            if best.is_none_or(|(k, _)| key < k) {
                best = Some((key, m));
            }
        }
    }
    best.map(|(_, m)| m)
}

/// Laufzeit-Paket (onnxruntime-web).
pub fn laufzeit(pakete: &[Manifest]) -> Option<&Manifest> {
    pakete.iter().find(|m| m.art == "laufzeit")
}

/// Pakete, die für eine Stufe gebraucht werden (Laufzeit + je Art eines).
pub fn pakete_fuer_stufe(pakete: &[Manifest], stufe: Stufe) -> Vec<&Manifest> {
    if stufe == Stufe::Aus {
        return Vec::new();
    }
    let mut v: Vec<&Manifest> = laufzeit(pakete).into_iter().collect();
    for a in Art::ALL {
        if let Some(m) = paket_fuer(pakete, a, stufe) {
            if !v.iter().any(|x| x.id == m.id) {
                v.push(m);
            }
        }
    }
    v
}

/// Größe als Text („32,1 MB“).
pub fn fmt_mb(bytes: u64) -> String {
    let mb = bytes as f64 / 1e6;
    if mb >= 100.0 {
        format!("{} MB", mb.round() as u64)
    } else {
        format!("{:.1} MB", mb).replace('.', ",")
    }
}

// ---------------------------------------------------------------- Striche → Bild

/// Gerendertes Graustufenbild der Striche (weißer Grund, schwarze Tinte), Zeile für Zeile, 1 Byte je Pixel.
#[derive(Clone, Debug, PartialEq)]
pub struct Ink {
    pub w: u32,
    pub h: u32,
    pub gray: Vec<u8>,
}

/// Typische Zeichenhöhe im Bild (Pixel) und Strichdicke dazu.
pub const CHAR_PX: f32 = 40.0;
pub const PEN_PX: f32 = 3.2;
/// Rand um die Tinte (Pixel).
pub const PAD_PX: f32 = 14.0;
/// Größte Bildkante.
pub const MAX_PX: f32 = 2048.0;

/// Polylinien (Weltkoordinaten) eines Objekts: Freihand-Striche direkt, Formen über `to_strokes`.
pub fn polylines(it: &Item) -> Vec<Vec<(f32, f32)>> {
    let one = |p: &[f32]| p.chunks_exact(3).map(|c| (c[0], c[1])).collect::<Vec<_>>();
    match &it.kind {
        Kind::Stroke(s) => vec![one(&s.p)],
        Kind::Shape(_) => it.to_strokes().iter().filter_map(|x| x.as_stroke().map(|s| one(&s.p))).collect(),
        _ => Vec::new(),
    }
}

/// Hülle mehrerer Polylinien.
pub fn hull(lines: &[Vec<(f32, f32)>]) -> Option<RectF> {
    let mut r: Option<RectF> = None;
    for &(x, y) in lines.iter().flatten() {
        let p = RectF::new(x, y, x, y);
        match r.as_mut() {
            Some(r) => r.union(&p),
            None => r = Some(p),
        }
    }
    r
}

/// Typische Zeichenhöhe: Median der Strichhöhen ohne Punkte und lange Linien (Bruchstriche, Unterstreichungen).
pub fn typical_height(lines: &[Vec<(f32, f32)>]) -> Option<f32> {
    let all = hull(lines)?;
    let mut hs: Vec<f32> = lines
        .iter()
        .filter_map(|l| hull(std::slice::from_ref(l)))
        .map(|r| r.height().max(r.width().min(r.height() * 3.0) * 0.6))
        .filter(|h| *h > all.height().max(all.width()) * 0.02)
        .collect();
    if hs.is_empty() {
        return Some(all.height().max(1.0));
    }
    hs.sort_by(|a, b| a.total_cmp(b));
    Some(hs[hs.len() / 2].max(1e-3))
}

/// Rendert die Objekte als Graustufenbild: Zeichenhöhe ≈ [`CHAR_PX`], Strichdicke [`PEN_PX`], Rand [`PAD_PX`].
/// Farben, Marker-Transparenz und Stiftart spielen keine Rolle.
pub fn render(items: &[&Item]) -> Option<Ink> {
    let lines: Vec<Vec<(f32, f32)>> = items.iter().flat_map(|it| polylines(it)).filter(|l| !l.is_empty()).collect();
    let b = hull(&lines)?;
    let ch = typical_height(&lines)?;
    let mut k = CHAR_PX / ch;
    let big = b.width().max(b.height()) * k;
    if big > MAX_PX - 2.0 * PAD_PX {
        k *= (MAX_PX - 2.0 * PAD_PX) / big;
    }
    // sehr kleine Auswahl (ein Punkt, ein Strich): nicht ins Riesige aufblasen
    k = k.min(400.0);
    let w = (b.width() * k + 2.0 * PAD_PX).ceil().max(8.0) as u32;
    let h = (b.height() * k + 2.0 * PAD_PX).ceil().max(8.0) as u32;
    let mut pm = Pixmap::new(w, h)?;
    pm.fill(tiny_skia::Color::WHITE);
    let mut paint = Paint::default();
    paint.set_color_rgba8(0, 0, 0, 255);
    paint.anti_alias = true;
    let st = Stroke { width: PEN_PX, line_cap: LineCap::Round, line_join: LineJoin::Round, ..Stroke::default() };
    let tf = |x: f32, y: f32| ((x - b.left) * k + PAD_PX, (y - b.top) * k + PAD_PX);
    for l in &lines {
        let mut pb = PathBuilder::new();
        let (x0, y0) = tf(l[0].0, l[0].1);
        pb.move_to(x0, y0);
        if l.len() == 1 {
            pb.line_to(x0 + 0.01, y0);
        }
        for &(x, y) in &l[1..] {
            let (px, py) = tf(x, y);
            pb.line_to(px, py);
        }
        if let Some(p) = pb.finish() {
            pm.stroke_path(&p, &paint, &st, Transform::identity(), None);
        }
    }
    let gray = pm.data().chunks_exact(4).map(|p| p[0]).collect();
    Some(Ink { w, h, gray })
}

/// Punkt im Polygon (gerade/ungerade-Regel).
pub fn in_poly(poly: &[(f32, f32)], x: f32, y: f32) -> bool {
    let mut inside = false;
    let n = poly.len();
    if n < 3 {
        return false;
    }
    let mut j = n - 1;
    for i in 0..n {
        let (xi, yi) = poly[i];
        let (xj, yj) = poly[j];
        if (yi > y) != (yj > y) && x < (xj - xi) * (y - yi) / (yj - yi) + xi {
            inside = !inside;
        }
        j = i;
    }
    inside
}

// ---------------------------------------------------------------- Alternativen und Kontext

/// Eine Alternative der Erkennung. `score` = mittlere Log-Wahrscheinlichkeit je Zeichen/Token (≤ 0, höher = besser).
#[derive(Clone, Debug, PartialEq)]
pub struct Kandidat {
    pub text: String,
    pub score: f32,
}

impl Kandidat {
    pub fn new(text: &str, score: f32) -> Kandidat {
        Kandidat { text: text.to_string(), score }
    }
    /// Zuversicht 0..1.
    pub fn conf(&self) -> f32 {
        self.score.min(0.0).exp()
    }
}

/// Wissen der Seite: Variablen und Einheiten aus Rechenzeilen/Formeln, Wörter aus Texten.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Kontext {
    pub vars: Vec<String>,
    pub einheiten: Vec<String>,
    pub woerter: Vec<String>,
}

fn push_uniq(v: &mut Vec<String>, s: &str) {
    if !s.is_empty() && !v.iter().any(|x| x == s) {
        v.push(s.to_string());
    }
}

fn is_word_char(c: char) -> bool {
    c.is_alphabetic() || matches!(c, 'µ' | 'Ω' | '°')
}

impl Kontext {
    /// Nimmt eine Rechen-/Formelzeile auf (Klartext wie `a: 2 kN`, `M = a·b`).
    pub fn add_calc_line(&mut self, plain: &str) {
        let mut prev_num = false;
        let mut first = true;
        let chars: Vec<char> = plain.chars().collect();
        let mut i = 0;
        while i < chars.len() {
            let c = chars[i];
            if c.is_ascii_digit() {
                while i < chars.len() && (chars[i].is_ascii_digit() || chars[i] == ',' || chars[i] == '.') {
                    i += 1;
                }
                prev_num = true;
                first = false;
                continue;
            }
            if is_word_char(c) {
                let st = i;
                while i < chars.len() && (is_word_char(chars[i]) || chars[i].is_ascii_digit() || chars[i] == '_') {
                    i += 1;
                }
                let w: String = chars[st..i].iter().collect();
                let rest = chars[i..].iter().collect::<String>();
                let assign = first && (rest.trim_start().starts_with(':') || rest.trim_start().starts_with('='));
                let func = rest.starts_with('(') && matches!(w.as_str(), "sqrt" | "sin" | "cos" | "tan" | "ln" | "log" | "exp");
                if func {
                } else if prev_num && !assign && crate::calc::units::lookup(&w).is_some() {
                    push_uniq(&mut self.einheiten, &w);
                } else if assign || w.chars().count() <= 3 {
                    push_uniq(&mut self.vars, &w);
                }
                first = false;
                // Einheiten-Produkte wie kN·m: Folge-Einheit nach · oder /
                prev_num = prev_num && i < chars.len() && matches!(chars[i], '·' | '/' | '*');
                continue;
            }
            if !c.is_whitespace() && !matches!(c, '·' | '/' | '*' | '^') {
                prev_num = false;
            }
            if !c.is_whitespace() {
                first = false;
            }
            i += 1;
        }
    }

    /// Nimmt Wörter eines Texts auf (ab 3 Buchstaben).
    pub fn add_text(&mut self, text: &str) {
        for w in text.split(|c: char| !c.is_alphanumeric()) {
            if w.chars().count() >= 3 && self.woerter.len() < 2000 {
                push_uniq(&mut self.woerter, &w.to_lowercase());
            }
        }
    }

    pub fn is_empty(&self) -> bool {
        self.vars.is_empty() && self.einheiten.is_empty() && self.woerter.is_empty()
    }
}

/// LaTeX-Befehle, die in Schul-/Technikformeln üblich sind (andere werden leicht abgewertet).
const OK_CMDS: [&str; 40] = [
    "frac", "cdot", "sqrt", "cos", "sin", "tan", "ln", "log", "ldots", "dots", "circ", "mathrm", "text", "times", "pm", "left", "right",
    "alpha", "beta", "gamma", "delta", "Delta", "epsilon", "eta", "lambda", "mu", "pi", "rho", "sigma", "tau", "varphi", "phi", "omega",
    "Omega", "le", "ge", "approx", "infty", "sum", "int",
];

/// Einzelbuchstaben-Bezeichner einer LaTeX-Formel (ohne Befehle und `\mathrm{…}`/`\text{…}`-Inhalt).
pub fn formel_buchstaben(latex: &str) -> (Vec<String>, Vec<String>) {
    let mut cmds = Vec::new();
    let mut letters = Vec::new();
    let ch: Vec<char> = latex.chars().collect();
    let mut i = 0;
    let mut skip_group = false;
    while i < ch.len() {
        let c = ch[i];
        if c == '\\' {
            let st = i + 1;
            i += 1;
            while i < ch.len() && ch[i].is_ascii_alphabetic() {
                i += 1;
            }
            let name: String = ch[st..i].iter().collect();
            if name.is_empty() {
                i += 1;
                continue;
            }
            skip_group = matches!(name.as_str(), "mathrm" | "text" | "operatorname" | "mathbf");
            cmds.push(name);
            continue;
        }
        if c == '{' && skip_group {
            let mut depth = 0;
            while i < ch.len() {
                if ch[i] == '{' {
                    depth += 1;
                } else if ch[i] == '}' {
                    depth -= 1;
                    if depth == 0 {
                        break;
                    }
                }
                i += 1;
            }
            skip_group = false;
            i += 1;
            continue;
        }
        skip_group = false;
        if c.is_alphabetic() {
            letters.push(c.to_string());
        }
        i += 1;
    }
    (cmds, letters)
}

fn zahl_form(s: &str) -> Option<(String, String)> {
    let t = s.trim();
    let mut i = 0;
    let b: Vec<char> = t.chars().collect();
    if i < b.len() && matches!(b[i], '-' | '+' | '−') {
        i += 1;
    }
    let st = i;
    while i < b.len() && (b[i].is_ascii_digit() || matches!(b[i], ',' | '.' | ' ' | '\'')) {
        i += 1;
    }
    let num: String = b[st..i].iter().collect();
    if !num.chars().any(|c| c.is_ascii_digit()) {
        return None;
    }
    Some((num.trim().to_string(), b[i..].iter().collect::<String>().trim().to_string()))
}

/// Bewertet die Alternativen einer Art mit dem Kontext neu (Bonus für bekannte Bezeichner/Einheiten/Wörter,
/// Abzug für fremde) und sortiert absteigend. Doppelte Texte fallen weg.
pub fn neu_bewerten(art: Art, mut k: Vec<Kandidat>, ctx: &Kontext, lexikon: &[String]) -> Vec<Kandidat> {
    for c in k.iter_mut() {
        c.text = match art {
            Art::Formel => c.text.replace("\\displaystyle", "").trim().to_string(),
            _ => c.text.split_whitespace().collect::<Vec<_>>().join(" "),
        };
        let mut adj = 0.0f32;
        match art {
            Art::Formel => {
                let (cmds, letters) = formel_buchstaben(&c.text);
                adj -= 0.25 * cmds.iter().filter(|x| !OK_CMDS.contains(&x.as_str())).count() as f32;
                if !ctx.vars.is_empty() || !ctx.einheiten.is_empty() {
                    let known = |l: &str| ctx.vars.iter().any(|v| v == l) || ctx.einheiten.iter().any(|v| v == l);
                    let unknown = letters.iter().filter(|l| !known(l)).count() as f32;
                    let hits = letters.iter().filter(|l| known(l)).count() as f32;
                    adj += 0.15 * hits - 0.35 * unknown;
                }
            }
            Art::Zahl => match zahl_form(&c.text) {
                Some((_, unit)) if unit.is_empty() => adj += 0.3,
                Some((_, unit)) => {
                    let u = unit.replace(' ', "");
                    if ctx.einheiten.iter().any(|x| *x == u) {
                        adj += 0.6;
                    } else if lexikon.iter().any(|x| *x == u) || crate::calc::units::lookup(&u).is_some() {
                        adj += 0.4;
                    } else {
                        adj -= 0.3;
                    }
                }
                None => adj -= 0.8,
            },
            Art::Text => {
                for w in c.text.split(|ch: char| !ch.is_alphanumeric()).filter(|w| w.chars().count() >= 3) {
                    if ctx.woerter.iter().any(|x| *x == w.to_lowercase()) {
                        adj += 0.2;
                    }
                }
            }
        }
        if c.text.is_empty() {
            adj -= 5.0;
        }
        c.score += adj;
    }
    k.sort_by(|a, b| b.score.total_cmp(&a.score));
    let mut out: Vec<Kandidat> = Vec::new();
    for c in k {
        if !out.iter().any(|x| x.text == c.text) {
            out.push(c);
        }
    }
    out
}

/// Sieht die Formel nach Mathematik aus (Operatoren, Brüche, Hoch-/Tiefstellung, Zahlen mit Rechenzeichen)?
pub fn mathe_struktur(latex: &str) -> bool {
    let has = |s: &str| latex.contains(s);
    has("\\frac") || has("\\sqrt") || has("^") || has("_") || has("=") || has("\\cdot") || has("+") || has("\\int") || has("\\sum")
        || (latex.chars().any(|c| c.is_ascii_digit()) && latex.chars().any(|c| matches!(c, '-' | '*' | '/' | '<' | '>')))
}

/// Reiter, der beim Öffnen gewählt ist: Zahl bei eindeutigem Zahlwert, Formel bei Mathe-Struktur, sonst Text.
/// `best` = beste Alternative je Art (nach [`neu_bewerten`]), fehlende Art = nicht erkannt.
pub fn vorauswahl(best: &[(Art, Option<&Kandidat>)]) -> Art {
    let get = |a: Art| best.iter().find(|x| x.0 == a).and_then(|x| x.1);
    let (t, f, z) = (get(Art::Text), get(Art::Formel), get(Art::Zahl));
    if let Some(z) = z {
        let num_like = zahl_form(&z.text).is_some_and(|(_, u)| u.chars().count() <= 6);
        if num_like && z.conf() >= 0.55 {
            // Text, der „nur“ eine Zahl ist, und Formel ohne Struktur verlieren gegen die Zahl
            let f_struct = f.is_some_and(|f| mathe_struktur(&f.text) && f.score > z.score + 0.1 && f.text.contains('='));
            if !f_struct {
                return Art::Zahl;
            }
        }
    }
    match (t, f) {
        (Some(t), Some(f)) => {
            if mathe_struktur(&f.text) && f.score >= t.score - 0.15 {
                Art::Formel
            } else if f.score > t.score + 0.4 {
                Art::Formel
            } else {
                Art::Text
            }
        }
        (None, Some(_)) => Art::Formel,
        (Some(_), None) => Art::Text,
        (None, None) if z.is_some() => Art::Zahl,
        _ => Art::Text,
    }
}

/// Zahl-Ergebnis als Rechen-Klartext (`12,5 kN`): Einheit mit Leerzeichen abgesetzt.
pub fn zahl_klartext(s: &str) -> String {
    match zahl_form(s) {
        Some((n, u)) if u.is_empty() => n,
        Some((n, u)) => format!("{n} {u}"),
        None => s.trim().to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn mf(id: &str, art: &str, stufen: &[&str]) -> Manifest {
        Manifest::from_value(json!({"id": id, "art": art, "stufen": stufen, "version": "1", "lizenz": {"spdx": "MIT"},
            "dateien": [{"name": "a.onnx", "groesse": 3, "sha256": "a".repeat(64)}]}))
        .unwrap()
    }

    #[test]
    fn manifest_lesen_und_pruefen() {
        let v = json!({"format": 1, "id": "formel-p2t-int8", "name": "P2T", "art": "formel", "stufen": ["schlecht", "mittel"],
            "version": "1.5", "lizenz": {"spdx": "MIT", "datei": "LICENSE.txt", "hinweis": "x"},
            "quelle": {"url": "https://huggingface.co/breezedeus/pix2text-mfr-1.5"},
            "dateien": [{"name": "encoder.onnx", "groesse": 10, "sha256": "AB".repeat(32)}, {"name": "LICENSE.txt", "groesse": 5, "sha256": "0".repeat(64)}],
            "modell": {"typ": "ved"}});
        let m = Manifest::parse(v.to_string().as_bytes()).unwrap();
        assert_eq!(m.id, "formel-p2t-int8");
        assert_eq!(m.art(), Some(Art::Formel));
        assert_eq!(m.stufen, vec![Stufe::Schlecht, Stufe::Mittel]);
        assert_eq!(m.groesse(), 15);
        assert_eq!(m.dateien[0].sha256, "ab".repeat(32), "klein geschrieben");
        assert_eq!(m.quelle, "https://huggingface.co/breezedeus/pix2text-mfr-1.5");
        assert_eq!(m.raw["modell"]["typ"], "ved");
        // Fehlerfälle
        let bad = |f: &dyn Fn(&mut Value)| {
            let mut x = v.clone();
            f(&mut x);
            Manifest::from_value(x).unwrap_err()
        };
        assert!(bad(&|x| x["id"] = json!("../x")).contains("Paket-ID"));
        assert!(bad(&|x| x["art"] = json!("bild")).contains("Art"));
        assert!(bad(&|x| x["lizenz"]["spdx"] = json!("CC-BY-NC-SA-4.0")).contains("nicht erlaubt"));
        assert!(bad(&|x| x["dateien"][0]["name"] = json!("../../etc/passwd")).contains("Dateiname"));
        assert!(bad(&|x| x["dateien"][0]["sha256"] = json!("xyz")).contains("sha256"));
        assert!(bad(&|x| x["stufen"] = json!([])).contains("Stufe"));
        assert!(bad(&|x| x["lizenz"]["datei"] = json!("COPYING")).contains("Lizenzdatei"));
        assert!(Manifest::parse(b"{").is_err());
        // Kurzform: "stufe" als Text, "lizenz" als Text
        let m = Manifest::from_value(json!({"id": "x", "art": "laufzeit", "stufe": "hoch", "lizenz": "Apache-2.0",
            "dateien": [{"name": "o.mjs", "groesse": 1, "sha256": "f".repeat(64)}]}))
        .unwrap();
        assert_eq!((m.stufen.as_slice(), m.lizenz.as_str(), m.name.as_str()), (&[Stufe::Hoch][..], "Apache-2.0", "x"));
    }

    #[test]
    fn paket_nach_stufe() {
        let p = vec![mf("rt", "laufzeit", &["schlecht", "mittel", "hoch"]), mf("p2t", "formel", &["schlecht", "mittel"]), mf("tt", "formel", &["hoch"]), mf("de", "text", &["mittel"])];
        assert_eq!(paket_fuer(&p, Art::Formel, Stufe::Mittel).unwrap().id, "p2t");
        assert_eq!(paket_fuer(&p, Art::Formel, Stufe::Hoch).unwrap().id, "tt");
        // Text nur „mittel“: bei „hoch“ das niedrigere, bei „schlecht“ das höhere
        assert_eq!(paket_fuer(&p, Art::Text, Stufe::Hoch).unwrap().id, "de");
        assert_eq!(paket_fuer(&p, Art::Text, Stufe::Schlecht).unwrap().id, "de");
        assert!(paket_fuer(&p, Art::Zahl, Stufe::Mittel).is_none());
        assert!(paket_fuer(&p, Art::Formel, Stufe::Aus).is_none());
        let ids: Vec<&str> = pakete_fuer_stufe(&p, Stufe::Hoch).iter().map(|m| m.id.as_str()).collect();
        assert_eq!(ids, vec!["rt", "de", "tt"]);
        assert!(pakete_fuer_stufe(&p, Stufe::Aus).is_empty());
        assert_eq!(fmt_mb(32_026_253), "32,0 MB");
        assert_eq!(fmt_mb(620_000_000), "620 MB");
    }

    fn stroke(pts: &[(f32, f32)]) -> Item {
        let mut it = Item::stroke(0xFF000000, 2.0, false, false);
        for &(x, y) in pts {
            it.add_point(x, y, 0.5);
        }
        it
    }

    #[test]
    fn striche_rendern_normiert() {
        // „1“ und „=“, 20 Einheiten hoch
        let a = stroke(&[(10.0, 10.0), (10.0, 30.0)]);
        let b = stroke(&[(20.0, 18.0), (30.0, 18.0)]);
        let c = stroke(&[(20.0, 24.0), (30.0, 24.0)]);
        let ink = render(&[&a, &b, &c]).unwrap();
        assert_eq!(ink.gray.len(), (ink.w * ink.h) as usize);
        // Höhe: typische Zeichenhöhe 20 → CHAR_PX, plus Rand
        let expect_h = (20.0 * CHAR_PX / typical_height(&[polylines(&a), polylines(&b), polylines(&c)].concat()).unwrap() + 2.0 * PAD_PX).ceil() as u32;
        assert!(ink.h.abs_diff(expect_h) <= 1, "{} vs {expect_h}", ink.h);
        // Rand weiß, Strich schwarz
        assert_eq!(ink.gray[0], 255);
        let px = |x: u32, y: u32| ink.gray[(y * ink.w + x) as usize];
        assert!(px(PAD_PX as u32, ink.h / 2) < 80, "Strich der 1");
        // doppelt so groß gezeichnet → gleiche Bildgröße (Strichdicke/Größe normiert)
        let s = |it: &Item| {
            let mut x = it.dup();
            x.xf(2.0, 0.0, 0.0, 0.0, 0.0);
            x
        };
        let ink2 = render(&[&s(&a), &s(&b), &s(&c)]).unwrap();
        assert!(ink2.w.abs_diff(ink.w) <= 2 && ink2.h.abs_diff(ink.h) <= 2);
        // riesige Auswahl wird begrenzt
        let long = stroke(&[(0.0, 0.0), (100000.0, 10.0)]);
        let ink3 = render(&[&long, &a]).unwrap();
        assert!(ink3.w as f32 <= MAX_PX + 1.0);
        assert!(render(&[]).is_none());
    }

    #[test]
    fn polygon() {
        let sq = [(0.0, 0.0), (10.0, 0.0), (10.0, 10.0), (0.0, 10.0)];
        assert!(in_poly(&sq, 5.0, 5.0));
        assert!(!in_poly(&sq, 15.0, 5.0));
        assert!(!in_poly(&sq[..2], 5.0, 5.0));
    }

    #[test]
    fn kontext_aus_rechenzeilen() {
        let mut k = Kontext::default();
        k.add_calc_line("a: 2 kN");
        k.add_calc_line("b = 3 m");
        k.add_calc_line("M = a·b");
        k.add_calc_line("v = 12 m/s");
        k.add_calc_line("x = sqrt(2)");
        assert_eq!(k.vars, vec!["a", "b", "M", "v", "x"]);
        assert_eq!(k.einheiten, vec!["kN", "m", "s"]);
        k.add_text("Die Kraft wirkt am Hebel.");
        assert!(k.woerter.contains(&"kraft".to_string()) && !k.woerter.contains(&"am".to_string()));
    }

    #[test]
    fn alternativen_mit_kontext() {
        let mut k = Kontext::default();
        k.add_calc_line("a: 1 N");
        k.add_calc_line("b: 2 m");
        // Formel: „M=a\cdot b“ schlägt die etwas wahrscheinlichere „M=\alpha\cdot6“ nicht, wohl aber „H=a\colon b“
        let c = vec![Kandidat::new("H=a\\colon b", -0.20), Kandidat::new("M=a\\cdot b", -0.30), Kandidat::new("M=a\\cdot b", -0.4)];
        let r = neu_bewerten(Art::Formel, c, &k, &[]);
        assert_eq!(r[0].text, "M=a\\cdot b");
        assert_eq!(r.len(), 2, "doppelte fallen weg");
        // ohne Kontext bleibt die Reihenfolge der Modell-Bewertung
        let c = vec![Kandidat::new("H=a b", -0.20), Kandidat::new("M=a b", -0.30)];
        assert_eq!(neu_bewerten(Art::Formel, c, &Kontext::default(), &[])[0].text, "H=a b");
        // Zahl: bekannte Einheit gewinnt, Nicht-Zahl verliert
        let c = vec![Kandidat::new("l2 m", -0.1), Kandidat::new("12 mn", -0.15), Kandidat::new("12 m", -0.3)];
        let r = neu_bewerten(Art::Zahl, c, &k, &["m".to_string()]);
        assert_eq!(r[0].text, "12 m");
        // Text: Wörter der Seite
        let mut k2 = Kontext::default();
        k2.add_text("Hebelgesetz und Drehmoment");
        let c = vec![Kandidat::new("Drehnoment", -0.2), Kandidat::new("Drehmoment", -0.3)];
        assert_eq!(neu_bewerten(Art::Text, c, &k2, &[])[0].text, "Drehmoment");
    }

    #[test]
    fn reiter_vorauswahl() {
        let k = |t: &str, s: f32| Kandidat::new(t, s);
        let (t, f, z) = (k("12 kN", -0.1), k("12\\mathrm{kN}", -0.2), k("12 kN", -0.05));
        assert_eq!(vorauswahl(&[(Art::Text, Some(&t)), (Art::Formel, Some(&f)), (Art::Zahl, Some(&z))]), Art::Zahl);
        let (t, f, z) = (k("M = a b", -0.5), k("M=a\\cdot b", -0.3), k("Ma", -1.5));
        assert_eq!(vorauswahl(&[(Art::Text, Some(&t)), (Art::Formel, Some(&f)), (Art::Zahl, Some(&z))]), Art::Formel);
        let (t, f) = (k("Hallo Welt", -0.1), k("H a l l o", -0.9));
        assert_eq!(vorauswahl(&[(Art::Text, Some(&t)), (Art::Formel, Some(&f)), (Art::Zahl, None)]), Art::Text);
        assert_eq!(vorauswahl(&[(Art::Text, None), (Art::Formel, None), (Art::Zahl, None)]), Art::Text);
        assert_eq!(zahl_klartext(" 12,5kN"), "12,5 kN");
        assert_eq!(zahl_klartext("abc"), "abc");
    }
}
