//! Ausrechnen: Größen (Wert in SI + Dimension + Anzeige-Einheit) und Funktionen.

use super::parse::{Op, E};
use super::units::{self, Dim, Disp, ONE};
use super::Opts;
use std::collections::HashMap;
use std::f64::consts::PI;

/// Eine Größe: Wert in SI-Einheiten, Dimension, Winkel-Kennung, gewünschte Anzeige.
#[derive(Clone, Debug, PartialEq)]
pub struct Q {
    pub v: f64,
    pub d: Dim,
    /// Winkel mit Einheit (° oder rad); Wert in rad.
    pub ang: bool,
    pub u: Disp,
}

impl Q {
    pub fn num(v: f64) -> Q {
        Q { v, d: ONE, ang: false, u: Vec::new() }
    }
    pub fn with_dim(v: f64, d: Dim) -> Q {
        Q { v, d, ang: false, u: Vec::new() }
    }
}

pub type Env = HashMap<String, Q>;

pub type Res<T> = Result<T, String>;

/// Konstanten (gelten nur, solange es keine Variable mit diesem Namen gibt).
pub fn constant(n: &str) -> Option<Q> {
    let q = |v: f64, d: Dim, u: &[(&str, i8)]| Q { v, d, ang: false, u: u.iter().map(|(s, e)| (s.to_string(), *e)).collect() };
    Some(match n {
        "π" | "pi" => Q::num(PI),
        "e" => Q::num(std::f64::consts::E),
        "g" => q(9.80665, [1, 0, -2, 0, 0, 0, 0], &[("m", 1), ("s", -2)]),
        "ε_0" | "epsilon_0" => q(8.8541878128e-12, [-3, -1, 4, 2, 0, 0, 0], &[("F", 1), ("m", -1)]),
        "μ_0" | "µ_0" | "mu_0" => q(1.25663706212e-6, [1, 1, -2, -2, 0, 0, 0], &[("H", 1), ("m", -1)]),
        "N_A" => q(6.02214076e23, [0, 0, 0, 0, 0, -1, 0], &[("mol", -1)]),
        "k_B" => q(1.380649e-23, [2, 1, -2, 0, -1, 0, 0], &[("J", 1), ("K", -1)]),
        "c_0" => q(299792458.0, [1, 0, -1, 0, 0, 0, 0], &[("m", 1), ("s", -1)]),
        _ => return None,
    })
}

pub struct Ctx<'a> {
    pub env: &'a Env,
    /// Zusätzliche Werte (Unbekannte beim Lösen), vor `env`.
    pub extra: &'a [(String, Q)],
    pub o: &'a Opts,
}

fn same_dim(a: &Q, b: &Q, op: &str) -> Res<()> {
    if a.d != b.d {
        return Err(format!("Einheiten passen nicht: {} {op} {}", show_dim(a), show_dim(b)));
    }
    Ok(())
}

fn show_dim(q: &Q) -> String {
    if !q.u.is_empty() && units::disp_dim(&q.u) == Some(q.d) {
        units::disp_plain(&q.u)
    } else {
        units::dim_plain(q.d)
    }
}

/// Winkel in rad (Zahl ohne Einheit je nach Einstellung Grad/Bogenmaß).
fn to_rad(q: &Q, o: &Opts, f: &str) -> Res<f64> {
    if q.d != ONE {
        return Err(format!("{f} braucht einen Winkel oder eine Zahl, nicht {}", show_dim(q)));
    }
    Ok(if q.ang || !o.deg { q.v } else { q.v.to_radians() })
}

fn plain_num(q: &Q, f: &str) -> Res<f64> {
    if q.d != ONE {
        return Err(format!("{f} braucht eine Zahl ohne Einheit, nicht {}", show_dim(q)));
    }
    Ok(q.v)
}

fn finite(q: Q) -> Res<Q> {
    if q.v.is_finite() {
        Ok(q)
    } else if q.v.is_nan() {
        Err("Ergebnis nicht definiert".into())
    } else {
        Err("Ergebnis unendlich (Division durch 0?)".into())
    }
}

pub fn add(a: Q, b: Q, sub: bool, o: &Opts) -> Res<Q> {
    same_dim(&a, &b, if sub { "−" } else { "+" })?;
    // Winkel mit und ohne Einheit mischen
    let conv = |q: &Q, other: &Q| if !q.ang && other.ang && o.deg { q.v.to_radians() } else { q.v };
    let (av, bv) = (conv(&a, &b), conv(&b, &a));
    let v = if sub { av - bv } else { av + bv };
    let cel = |q: &Q| q.u.len() == 1 && q.u[0].0 == "°C";
    let u = if sub && cel(&a) && cel(&b) {
        vec![("K".to_string(), 1)]
    } else if a.u.is_empty() {
        b.u.clone()
    } else {
        a.u.clone()
    };
    Ok(Q { v, d: a.d, ang: a.ang || b.ang, u })
}

pub fn mul(a: Q, b: Q, k: i8) -> Res<Q> {
    let v = if k > 0 { a.v * b.v } else { a.v / b.v };
    if k < 0 && b.v == 0.0 {
        return Err("Division durch 0".into());
    }
    let ang = if a.ang && b.ang { false } else { a.ang || (b.ang && k > 0) };
    // %, ‰, ppm sind nur Schreibweise einer Zahl: beim Rechnen verschwinden sie
    let mut u = units::disp_mul(&a.u, &b.u, k);
    u.retain(|(s, _)| !matches!(s.as_str(), "%" | "‰" | "ppm"));
    finite(Q { v, d: units::dim_mul(a.d, b.d, k), ang, u })
}

pub fn pow(a: Q, p: Q) -> Res<Q> {
    if p.d != ONE {
        return Err(format!("Hochzahl muss ohne Einheit sein, nicht {}", show_dim(&p)));
    }
    let e = p.v;
    let mut d = ONE;
    let mut u = Vec::new();
    if a.d != ONE {
        for i in 0..7 {
            let x = a.d[i] as f64 * e;
            if (x - x.round()).abs() > 1e-9 {
                return Err(format!("{} hoch {} ergibt keine Einheit", show_dim(&a), fmt_plain(e)));
            }
            d[i] = x.round() as i8;
        }
        for (s, k) in &a.u {
            let x = *k as f64 * e;
            if (x - x.round()).abs() > 1e-9 {
                u.clear();
                break;
            }
            u.push((s.clone(), x.round() as i8));
        }
        if units::disp_dim(&u) != Some(d) {
            u = Vec::new();
        }
    }
    if a.v < 0.0 && (e - e.round()).abs() > 1e-12 {
        return Err("Wurzel aus negativer Zahl".into());
    }
    finite(Q { v: a.v.powf(e), d, ang: false, u })
}

fn fmt_plain(v: f64) -> String {
    let s = format!("{}", (v * 1e6).round() / 1e6);
    s.replace('.', ",")
}

fn call(f: &str, a: Q, base: Option<Q>, o: &Opts) -> Res<Q> {
    let ang_out = |v: f64| Q { v, d: ONE, ang: true, u: Vec::new() };
    let num = |v: f64| finite(Q::num(v));
    match f {
        "sin" => num(to_rad(&a, o, f)?.sin()),
        "cos" => num(to_rad(&a, o, f)?.cos()),
        "tan" => {
            let r = to_rad(&a, o, f)?;
            if (r / PI - 0.5).rem_euclid(1.0).abs() < 1e-12 {
                return Err("tan ist hier nicht definiert".into());
            }
            num(r.tan())
        }
        "cot" => num(1.0 / to_rad(&a, o, f)?.tan()),
        "arcsin" | "asin" | "arccos" | "acos" => {
            let x = plain_num(&a, f)?;
            if !(-1.0 - 1e-12..=1.0 + 1e-12).contains(&x) {
                return Err(format!("{f} braucht eine Zahl zwischen −1 und 1"));
            }
            let x = x.clamp(-1.0, 1.0);
            Ok(ang_out(if f.ends_with("sin") { x.asin() } else { x.acos() }))
        }
        "arctan" | "atan" => Ok(ang_out(plain_num(&a, f)?.atan())),
        "arccot" => Ok(ang_out(PI / 2.0 - plain_num(&a, f)?.atan())),
        "sinh" => num(plain_num(&a, f)?.sinh()),
        "cosh" => num(plain_num(&a, f)?.cosh()),
        "tanh" => num(plain_num(&a, f)?.tanh()),
        "exp" => num(plain_num(&a, f)?.exp()),
        "ln" | "lg" | "ld" | "log" => {
            let x = plain_num(&a, f)?;
            if x <= 0.0 {
                return Err("Logarithmus braucht eine positive Zahl".into());
            }
            let b = match (f, base) {
                ("ln", _) => std::f64::consts::E,
                ("ld", _) => 2.0,
                (_, Some(b)) => {
                    let b = plain_num(&b, "Basis")?;
                    if b <= 0.0 || b == 1.0 {
                        return Err("ungültige Basis".into());
                    }
                    b
                }
                _ => 10.0,
            };
            num(x.ln() / b.ln())
        }
        "sgn" => Ok(Q::num(a.v.signum())),
        "round" | "floor" | "ceil" => {
            let x = plain_num(&a, f)?;
            num(match f {
                "round" => x.round(),
                "floor" => x.floor(),
                _ => x.ceil(),
            })
        }
        _ => Err(format!("unbekannte Funktion {f}")),
    }
}

/// Wert eines Ausdrucks. Unbekannte Variablen → Fehler „unbekannt: x“.
pub fn eval(e: &E, c: &Ctx) -> Res<Q> {
    match e {
        E::Num(v, _) => Ok(Q::num(*v)),
        E::Var(n) => {
            if let Some((_, q)) = c.extra.iter().find(|x| x.0 == *n) {
                return Ok(q.clone());
            }
            if let Some(q) = c.env.get(n) {
                return Ok(q.clone());
            }
            constant(n).ok_or_else(|| format!("unbekannt: {n}"))
        }
        E::Unit(u) => Ok(Q { v: u.f, d: u.d, ang: u.ang, u: vec![(u.sym.clone(), 1)] }),
        E::Hole => Err("Platzhalter".into()),
        E::Neg(a) => {
            let mut q = eval(a, c)?;
            q.v = -q.v;
            Ok(q)
        }
        E::Par(a) => eval(a, c),
        E::Abs(a) => {
            let mut q = eval(a, c)?;
            q.v = q.v.abs();
            Ok(q)
        }
        E::Fact(a) => {
            let q = eval(a, c)?;
            let x = plain_num(&q, "!")?;
            if x < 0.0 || x.fract() != 0.0 || x > 170.0 {
                return Err("Fakultät nur für ganze Zahlen 0 … 170".into());
            }
            Ok(Q::num((1..=x as u64).map(|k| k as f64).product()))
        }
        E::Bin(op, a, b) => {
            // 20 °C: Nullpunkt verschieben
            if *op == Op::UMul {
                if let E::Unit(u) = &**b {
                    if u.celsius {
                        let x = eval(a, c)?;
                        let x = plain_num(&x, "°C")?;
                        return Ok(Q { v: x + 273.15, d: u.d, ang: false, u: vec![("°C".into(), 1)] });
                    }
                }
            }
            let x = eval(a, c)?;
            let y = eval(b, c)?;
            match op {
                Op::Add => add(x, y, false, c.o),
                Op::Sub => add(x, y, true, c.o),
                Op::Mul | Op::IMul | Op::UMul => mul(x, y, 1),
                Op::Div | Op::Frac => mul(x, y, -1),
                Op::Pow => pow(x, y),
            }
        }
        E::Root(a, n) => {
            let x = eval(a, c)?;
            let k = match n {
                Some(n) => plain_num(&eval(n, c)?, "Wurzelgrad")?,
                None => 2.0,
            };
            if k == 0.0 {
                return Err("Wurzelgrad 0".into());
            }
            // ungerade Wurzel aus negativer Zahl
            if x.v < 0.0 && (k.round() - k).abs() < 1e-12 && (k.round() as i64) % 2 != 0 {
                let mut q = pow(Q { v: -x.v, ..x }, Q::num(1.0 / k))?;
                q.v = -q.v;
                return Ok(q);
            }
            if x.v < 0.0 {
                return Err("Wurzel aus negativer Zahl".into());
            }
            pow(x, Q::num(1.0 / k))
        }
        E::Call(f, a, base) => {
            let x = eval(a, c)?;
            let b = match base {
                Some(b) => Some(eval(b, c)?),
                None => None,
            };
            call(f, x, b, c.o)
        }
    }
}

// ---------------------------------------------------------------- Anzeige

/// Zahl mit n gültigen Stellen als (Mantisse, Zehnerpotenz) – Potenz nur bei sehr großen/kleinen Werten.
pub fn fmt_num(v: f64, o: &Opts) -> (String, Option<i32>) {
    let n = o.digits.clamp(1, 15) as i32;
    if v == 0.0 {
        return ("0".into(), None);
    }
    let ex = v.abs().log10().floor() as i32;
    let sci = !(-4..7).contains(&ex);
    let (m, ex) = if sci {
        let mut m = v / 10f64.powi(ex);
        let mut ex = ex;
        let r = (m * 10f64.powi(n - 1)).round() / 10f64.powi(n - 1);
        if r.abs() >= 10.0 {
            m = r / 10.0;
            ex += 1;
        } else {
            m = r;
        }
        (m, Some(ex))
    } else {
        (v, None)
    };
    let e0 = if m == 0.0 { 0 } else { m.abs().log10().floor() as i32 };
    let dec = (n - 1 - e0).max(0) as usize;
    let mut s = format!("{:.*}", dec, m);
    if s.contains('.') {
        while s.ends_with('0') {
            s.pop();
        }
        if s.ends_with('.') {
            s.pop();
        }
    }
    if s == "-0" {
        s = "0".into();
    }
    // Rundung auf ganze Zahl mit mehr Stellen als n (z. B. 123456 bei 4 Stellen → 123500)
    if dec == 0 && ex.is_none() && e0 >= n {
        let k = 10f64.powi(e0 - n + 1);
        s = format!("{}", (m / k).round() * k);
    }
    (s, ex)
}

pub fn num_latex(v: f64, o: &Opts) -> String {
    let (m, ex) = fmt_num(v, o);
    let m = m.replace('-', "-").replace('.', if o.comma { "{,}" } else { "." });
    match ex {
        Some(e) if m == "1" => format!("10^{{{e}}}"),
        Some(e) => format!("{m}\\cdot 10^{{{e}}}"),
        None => m,
    }
}

pub fn num_plain(v: f64, o: &Opts) -> String {
    let (m, ex) = fmt_num(v, o);
    let m = m.replace('.', if o.comma { "," } else { "." });
    match ex {
        Some(e) => format!("{m}·10{}", units::sup(e)),
        None => m,
    }
}

/// Anzeige-Einheit einer Größe (Vorgabe aus der Rechnung, sonst passend zur Dimension).
pub fn auto_disp(q: &Q) -> Disp {
    if !q.u.is_empty() && units::disp_dim(&q.u) == Some(q.d) {
        return q.u.clone();
    }
    units::disp_for(q.d)
}

/// Wert in einer Anzeige-Einheit; Winkel ohne Einheit je nach Einstellung in ° bzw. rad.
pub fn shown(q: &Q, u: &Disp, o: &Opts) -> (f64, Disp) {
    if q.ang && q.d == ONE && u.iter().all(|(s, _)| units::lookup(s).is_some_and(|x| x.ang)) {
        let want_deg = if u.is_empty() { o.deg } else { u[0].0 == "°" };
        return if want_deg { (q.v.to_degrees(), vec![("°".into(), 1)]) } else { (q.v, if u.is_empty() { Vec::new() } else { vec![("rad".into(), 1)] }) };
    }
    if u.len() == 1 && u[0].0 == "°C" && u[0].1 == 1 {
        return (q.v - 273.15, u.clone());
    }
    let f = units::disp_factor(u);
    (q.v / f, u.clone())
}

/// LaTeX einer Größe: 2\,\mathrm{N}\cdot\mathrm{m}, 30^{\circ}.
pub fn q_latex(q: &Q, u: Option<&Disp>, o: &Opts) -> String {
    let au = match u {
        Some(u) => u.clone(),
        None => auto_disp(q),
    };
    let (v, du) = shown(q, &au, o);
    let n = num_latex(v, o);
    if du.is_empty() {
        return n;
    }
    if du.len() == 1 && du[0].0 == "°" {
        return format!("{n}^{{\\circ}}");
    }
    if du.len() == 1 && du[0].0 == "°C" {
        return format!("{n}\\,^{{\\circ}}\\mathrm{{C}}");
    }
    format!("{n}\\,{}", units::disp_latex(&du))
}

pub fn q_plain(q: &Q, o: &Opts) -> String {
    let au = auto_disp(q);
    let (v, du) = shown(q, &au, o);
    let n = num_plain(v, o);
    if du.is_empty() {
        n
    } else if du[0].0.starts_with('°') && du.len() == 1 {
        format!("{n}{}", du[0].0)
    } else {
        format!("{n} {}", units::disp_plain(&du))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn zahlen() {
        let o = Opts::default();
        assert_eq!(fmt_num(2.0, &o).0, "2");
        assert_eq!(fmt_num(1.0 / 3.0, &o).0, "0.3333");
        assert_eq!(fmt_num(123456.0, &o).0, "123500");
        assert_eq!(fmt_num(9.80665, &o).0, "9.807");
        assert_eq!(fmt_num(-0.00012346, &o).0, "-0.0001235");
        assert_eq!(fmt_num(1.5e-7, &o), ("1.5".into(), Some(-7)));
        assert_eq!(fmt_num(9.99996e9, &o), ("1".into(), Some(10)));
        assert_eq!(num_latex(1.5, &o), "1{,}5");
        assert_eq!(num_plain(2.5e9, &o), "2,5·10⁹");
    }
}
