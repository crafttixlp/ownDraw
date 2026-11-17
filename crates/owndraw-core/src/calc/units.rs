//! Einheiten: SI-Dimensionen, Einheitentabelle mit Vorsätzen, Anzeige.

/// Exponenten der SI-Basisgrößen: m, kg, s, A, K, mol, cd.
pub type Dim = [i8; 7];

pub const ONE: Dim = [0; 7];

const BASE: [&str; 7] = ["m", "kg", "s", "A", "K", "mol", "cd"];

/// Eine Einheit der Tabelle.
pub struct Def {
    pub sym: &'static str,
    /// Faktor zur SI-Einheit.
    pub f: f64,
    pub d: Dim,
    /// Vorsätze erlaubt (k, M, m, µ …).
    pub pre: bool,
    /// Winkeleinheit (° und rad).
    pub ang: bool,
}

const fn d(m: i8, kg: i8, s: i8, a: i8, k: i8) -> Dim {
    [m, kg, s, a, k, 0, 0]
}

macro_rules! u {
    ($s:expr, $f:expr, $d:expr, $p:expr) => {
        Def { sym: $s, f: $f, d: $d, pre: $p, ang: false }
    };
}

pub static DEFS: &[Def] = &[
    // Basis
    u!("m", 1.0, d(1, 0, 0, 0, 0), true),
    u!("g", 1e-3, d(0, 1, 0, 0, 0), true),
    u!("s", 1.0, d(0, 0, 1, 0, 0), true),
    u!("A", 1.0, d(0, 0, 0, 1, 0), true),
    u!("K", 1.0, d(0, 0, 0, 0, 1), true),
    u!("mol", 1.0, [0, 0, 0, 0, 0, 1, 0], true),
    u!("cd", 1.0, [0, 0, 0, 0, 0, 0, 1], true),
    // Mechanik
    u!("N", 1.0, d(1, 1, -2, 0, 0), true),
    u!("Nm", 1.0, d(2, 1, -2, 0, 0), true),
    u!("J", 1.0, d(2, 1, -2, 0, 0), true),
    u!("W", 1.0, d(2, 1, -3, 0, 0), true),
    u!("Pa", 1.0, d(-1, 1, -2, 0, 0), true),
    u!("bar", 1e5, d(-1, 1, -2, 0, 0), true),
    u!("atm", 101325.0, d(-1, 1, -2, 0, 0), false),
    u!("mmHg", 133.322387415, d(-1, 1, -2, 0, 0), false),
    u!("t", 1000.0, d(0, 1, 0, 0, 0), false),
    u!("l", 1e-3, d(3, 0, 0, 0, 0), true),
    u!("L", 1e-3, d(3, 0, 0, 0, 0), true),
    u!("ha", 1e4, d(2, 0, 0, 0, 0), false),
    u!("min", 60.0, d(0, 0, 1, 0, 0), false),
    u!("h", 3600.0, d(0, 0, 1, 0, 0), false),
    u!("d", 86400.0, d(0, 0, 1, 0, 0), false),
    u!("Hz", 1.0, d(0, 0, -1, 0, 0), true),
    u!("rpm", 1.0 / 60.0, d(0, 0, -1, 0, 0), false),
    u!("U", 1.0, ONE, false),
    u!("PS", 735.49875, d(2, 1, -3, 0, 0), false),
    u!("Wh", 3600.0, d(2, 1, -2, 0, 0), true),
    u!("eV", 1.602176634e-19, d(2, 1, -2, 0, 0), true),
    u!("cal", 4.184, d(2, 1, -2, 0, 0), true),
    // Elektrotechnik
    u!("V", 1.0, d(2, 1, -3, -1, 0), true),
    u!("Ω", 1.0, d(2, 1, -3, -2, 0), true),
    u!("Ohm", 1.0, d(2, 1, -3, -2, 0), true),
    u!("S", 1.0, d(-2, -1, 3, 2, 0), true),
    u!("C", 1.0, d(0, 0, 1, 1, 0), true),
    u!("Ah", 3600.0, d(0, 0, 1, 1, 0), true),
    u!("F", 1.0, d(-2, -1, 4, 2, 0), true),
    u!("H", 1.0, d(2, 1, -2, -2, 0), true),
    u!("T", 1.0, d(0, 1, -2, -1, 0), true),
    u!("Wb", 1.0, d(2, 1, -2, -1, 0), true),
    u!("VA", 1.0, d(2, 1, -3, 0, 0), true),
    u!("var", 1.0, d(2, 1, -3, 0, 0), true),
    // Wärme (°C: Anzeige mit Nullpunkt, siehe eval)
    u!("°C", 1.0, d(0, 0, 0, 0, 1), false),
    // ohne Dimension
    u!("%", 0.01, ONE, false),
    u!("‰", 0.001, ONE, false),
    u!("ppm", 1e-6, ONE, false),
    Def { sym: "°", f: std::f64::consts::PI / 180.0, d: ONE, pre: false, ang: true },
    Def { sym: "rad", f: 1.0, d: ONE, pre: true, ang: true },
];

const PREFIX: [(&str, f64); 21] = [
    ("da", 1e1),
    ("Y", 1e24),
    ("Z", 1e21),
    ("E", 1e18),
    ("P", 1e15),
    ("T", 1e12),
    ("G", 1e9),
    ("M", 1e6),
    ("k", 1e3),
    ("h", 1e2),
    ("d", 1e-1),
    ("c", 1e-2),
    ("m", 1e-3),
    ("µ", 1e-6),
    ("μ", 1e-6),
    ("u", 1e-6),
    ("n", 1e-9),
    ("p", 1e-12),
    ("f", 1e-15),
    ("a", 1e-18),
    ("z", 1e-21),
];

thread_local! {
    /// Eigene Einheiten aus der Formelsammlung (Zeichen, Faktor zu SI, Dimension); gelten während `with_custom`.
    static CUSTOM: std::cell::RefCell<Vec<(String, f64, Dim)>> = const { std::cell::RefCell::new(Vec::new()) };
}

/// Führt `f` mit zusätzlichen eigenen Einheiten aus (ohne Vorsätze). Eingebaute Einheiten haben Vorrang.
pub fn with_custom<R>(units: Vec<(String, f64, Dim)>, f: impl FnOnce() -> R) -> R {
    let old = CUSTOM.with(|c| std::mem::replace(&mut *c.borrow_mut(), units));
    let r = f();
    CUSTOM.with(|c| *c.borrow_mut() = old);
    r
}

/// Fügt eine eigene Einheit hinzu (nur innerhalb von `with_custom`).
pub fn push_custom(sym: &str, f: f64, d: Dim) {
    CUSTOM.with(|c| c.borrow_mut().push((sym.to_string(), f, d)));
}

/// Eine gefundene Einheit (mit Vorsatz).
#[derive(Clone, Debug, PartialEq)]
pub struct Unit {
    /// Anzeige, z. B. "kN" (Vorsatz µ einheitlich als "µ").
    pub sym: String,
    pub f: f64,
    pub d: Dim,
    pub ang: bool,
    /// °C (Nullpunkt verschoben).
    pub celsius: bool,
}

/// Einheit zu einem Zeichen wie "kN", "mm", "µF", "°C". None = keine Einheit.
pub fn lookup(s: &str) -> Option<Unit> {
    if s.is_empty() {
        return None;
    }
    let mk = |def: &Def, pre: &str, pf: f64| Unit {
        sym: format!("{}{}", if pre == "u" || pre == "μ" { "µ" } else { pre }, if def.sym == "Ohm" { "Ω" } else { def.sym }),
        f: def.f * pf,
        d: def.d,
        ang: def.ang,
        celsius: def.sym == "°C",
    };
    if let Some(def) = DEFS.iter().find(|u| u.sym == s) {
        return Some(mk(def, "", 1.0));
    }
    if let Some(u) = CUSTOM.with(|c| c.borrow().iter().find(|u| u.0 == s).cloned()) {
        return Some(Unit { sym: u.0, f: u.1, d: u.2, ang: false, celsius: false });
    }
    for (p, pf) in PREFIX {
        if let Some(rest) = s.strip_prefix(p) {
            if let Some(def) = DEFS.iter().find(|u| u.pre && u.sym == rest) {
                // "uf" o. Ä. ist eher ein Name: u nur vor bekannten Einheiten mit Großbuchstaben oder m/s/g/l
                return Some(mk(def, p, pf));
            }
        }
    }
    None
}

/// Anzeige-Einheit: Liste von (Zeichen, Exponent), z. B. N·m = [("N",1),("m",1)].
pub type Disp = Vec<(String, i8)>;

/// Dimension einer Anzeige-Einheit.
pub fn disp_dim(u: &Disp) -> Option<Dim> {
    let mut r = ONE;
    for (s, e) in u {
        let x = lookup(s)?;
        for i in 0..7 {
            r[i] += x.d[i] * e;
        }
    }
    Some(r)
}

/// SI-Faktor einer Anzeige-Einheit.
pub fn disp_factor(u: &Disp) -> f64 {
    u.iter().map(|(s, e)| lookup(s).map(|x| x.f).unwrap_or(1.0).powi(*e as i32)).product()
}

pub fn dim_mul(a: Dim, b: Dim, k: i8) -> Dim {
    let mut r = a;
    for i in 0..7 {
        r[i] += b[i] * k;
    }
    r
}

/// Multipliziert zwei Anzeige-Einheiten (k = 1) bzw. teilt (k = -1). Gleichartige Einheiten werden in die
/// erste umgerechnet (km/h · min → km), dimensionslose Hilfseinheiten (%, U) fallen neben echten weg.
pub fn disp_mul(a: &Disp, b: &Disp, k: i8) -> Disp {
    let mut r = a.clone();
    for (s, e) in b {
        let e = e * k;
        if let Some(x) = r.iter_mut().find(|(t, _)| t == s) {
            x.1 += e;
            continue;
        }
        let ub = lookup(s);
        let same = ub.as_ref().and_then(|ub| {
            r.iter_mut().find(|(t, te)| *te != 0 && lookup(t).is_some_and(|ut| ut.d == ub.d && ut.d != ONE && !ut.celsius))
        });
        match same {
            Some(x) => x.1 += e,
            None => r.push((s.clone(), e)),
        }
    }
    r.retain(|(_, e)| *e != 0);
    // °C in zusammengesetzten Einheiten = K
    if r.len() > 1 || r.first().is_some_and(|x| x.1 != 1) {
        for x in r.iter_mut() {
            if x.0 == "°C" {
                x.0 = "K".into();
            }
        }
    }
    let real = r.iter().any(|(s, _)| lookup(s).is_some_and(|u| u.d != ONE));
    if real {
        r.retain(|(s, _)| lookup(s).is_some_and(|u| u.d != ONE));
    }
    nice(r)
}

/// Bekannte Kombinationen ohne Vorsatz zusammenfassen (V·A → W, V/A → Ω …).
fn nice(r: Disp) -> Disp {
    let key = disp_key(&r);
    let rep = match key.as_str() {
        "A·V" => "W",
        "A·Ω" => "V",
        "V/A" => "Ω",
        "V/Ω" => "A",
        "W/V" => "A",
        "W/A" => "V",
        "J/s" => "W",
        "W·s" => "J",
        "V·s" => "Wb",
        "A·s" => "C",
        "N/m²" => "Pa",
        "kg·m/s²" => "N",
        "C/V" => "F",
        "1/Ω" => "S",
        _ => return r,
    };
    vec![(rep.to_string(), 1)]
}

/// Sortierter Schlüssel ("A·V", "V/A", "kg·m/s²").
fn disp_key(r: &Disp) -> String {
    let mut num: Vec<String> = Vec::new();
    let mut den: Vec<String> = Vec::new();
    for (s, e) in r {
        let t = if e.abs() == 1 { s.clone() } else { format!("{s}{}", sup(e.abs() as i32)) };
        if *e > 0 {
            num.push(t)
        } else {
            den.push(t)
        }
    }
    num.sort();
    den.sort();
    let n = if num.is_empty() { "1".to_string() } else { num.join("·") };
    if den.is_empty() {
        n
    } else {
        format!("{n}/{}", den.join("·"))
    }
}

pub fn sup(n: i32) -> String {
    let s = n.to_string();
    s.chars()
        .map(|c| match c {
            '-' => '⁻',
            '0' => '⁰',
            '1' => '¹',
            '2' => '²',
            '3' => '³',
            '4' => '⁴',
            '5' => '⁵',
            '6' => '⁶',
            '7' => '⁷',
            '8' => '⁸',
            _ => '⁹',
        })
        .collect()
}

/// Passende Anzeige-Einheit für eine Dimension ohne Vorgabe (N, J, W, Pa … oder Basiseinheiten).
pub fn disp_for(d: Dim) -> Disp {
    if d == ONE {
        return Vec::new();
    }
    for s in ["N", "J", "W", "Pa", "V", "Ω", "C", "F", "H", "T", "Wb", "S"] {
        if lookup(s).is_some_and(|u| u.d == d) {
            return vec![(s.to_string(), 1)];
        }
    }
    let mut r = Vec::new();
    for i in [1, 0, 2, 3, 4, 5, 6] {
        if d[i] != 0 {
            r.push((BASE[i].to_string(), d[i]));
        }
    }
    r
}

/// Einheit als LaTeX: \mathrm{N}\cdot\mathrm{m}, \mathrm{m}/\mathrm{s}^{2}, \mathrm{J}/\left(\mathrm{kg}\cdot\mathrm{K}\right).
pub fn disp_latex(u: &Disp) -> String {
    let one = |s: &str, e: i8| {
        let b = format!("\\mathrm{{{}}}", if s == "%" { "\\%" } else { s });
        if e == 1 {
            b
        } else {
            format!("{b}^{{{e}}}")
        }
    };
    let num: Vec<String> = u.iter().filter(|x| x.1 > 0).map(|(s, e)| one(s, *e)).collect();
    let den: Vec<String> = u.iter().filter(|x| x.1 < 0).map(|(s, e)| one(s, -*e)).collect();
    if den.is_empty() {
        return num.join("\\cdot ");
    }
    let n = if num.is_empty() { "1".to_string() } else { num.join("\\cdot ") };
    if den.len() == 1 {
        format!("{n}/{}", den[0])
    } else {
        format!("{n}/\\left({}\\right)", den.join("\\cdot "))
    }
}

/// Einheit als Klartext: N·m, m/s², J/(kg·K).
pub fn disp_plain(u: &Disp) -> String {
    let one = |s: &str, e: i8| if e == 1 { s.to_string() } else { format!("{s}{}", sup(e as i32)) };
    let num: Vec<String> = u.iter().filter(|x| x.1 > 0).map(|(s, e)| one(s, *e)).collect();
    let den: Vec<String> = u.iter().filter(|x| x.1 < 0).map(|(s, e)| one(s, -*e)).collect();
    if den.is_empty() {
        return num.join("·");
    }
    let n = if num.is_empty() { "1".to_string() } else { num.join("·") };
    if den.len() == 1 {
        format!("{n}/{}", den[0])
    } else {
        format!("{n}/({})", den.join("·"))
    }
}

/// Klartext einer Dimension (für Meldungen).
pub fn dim_plain(d: Dim) -> String {
    if d == ONE {
        return "ohne Einheit".into();
    }
    disp_plain(&disp_for(d))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn einheiten() {
        let k = lookup("kN").unwrap();
        assert_eq!(k.f, 1000.0);
        assert_eq!(lookup("mm").unwrap().f, 1e-3);
        assert_eq!(lookup("min").unwrap().f, 60.0);
        assert!((lookup("kg").unwrap().f - 1.0).abs() < 1e-12);
        assert_eq!(lookup("µF").unwrap().sym, "µF");
        assert_eq!(lookup("uF").unwrap().sym, "µF");
        assert_eq!(lookup("kΩ").unwrap().f, 1000.0);
        assert_eq!(lookup("MPa").unwrap().d, lookup("Pa").unwrap().d);
        assert_eq!(lookup("kWh").unwrap().f, 3.6e6);
        assert!(lookup("x").is_none());
        assert!(lookup("Fres").is_none());
        let nm = disp_mul(&vec![("N".into(), 1)], &vec![("m".into(), 1)], 1);
        assert_eq!(disp_plain(&nm), "N·m");
        let w = disp_mul(&vec![("V".into(), 1)], &vec![("A".into(), 1)], 1);
        assert_eq!(disp_plain(&w), "W");
        let kmh = disp_mul(&vec![("km".into(), 1), ("h".into(), -1)], &vec![("min".into(), 1)], 1);
        assert_eq!(disp_plain(&kmh), "km");
        assert_eq!(disp_plain(&disp_for(d(1, 0, -2, 0, 0))), "m/s²");
        assert_eq!(disp_plain(&disp_for(d(2, 1, -2, 0, 0))), "J");
    }
}
