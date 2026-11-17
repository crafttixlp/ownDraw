//! Lösen: Gleichung(en) nach Unbekannten – linear und quadratisch exakt, sonst numerisch;
//! Gleichungssysteme (linear exakt, nichtlinear mit Newton). Einheiten der Unbekannten werden aus den
//! Gleichungen bestimmt.

use super::eval::{self, Ctx, Env, Q, Res};
use super::parse::{Op, E};
use super::units::{self, ONE};
use super::Opts;

/// Eine Gleichung l = r.
#[derive(Clone, Debug)]
pub struct Eqn {
    pub l: E,
    pub r: E,
    /// LaTeX der Zeile (Rechenweg).
    pub tex: String,
}

/// Dimension als a + Σ k_i · d_i (d_i = Dimension der Unbekannten i).
#[derive(Clone)]
struct DA {
    a: [f64; 7],
    k: Vec<f64>,
}

impl DA {
    fn zero(n: usize) -> DA {
        DA { a: [0.0; 7], k: vec![0.0; n] }
    }
    fn lin(&self, o: &DA, s: f64) -> DA {
        let mut r = self.clone();
        for i in 0..7 {
            r.a[i] += s * o.a[i];
        }
        for i in 0..r.k.len() {
            r.k[i] += s * o.k[i];
        }
        r
    }
    fn scale(&self, s: f64) -> DA {
        DA { a: self.a.map(|x| x * s), k: self.k.iter().map(|x| x * s).collect() }
    }
}

fn has_unknown(e: &E, unk: &[String]) -> bool {
    let mut v = Vec::new();
    e.vars(&mut v);
    v.iter().any(|x| unk.contains(x))
}

fn infer(e: &E, c: &Ctx, unk: &[String], cons: &mut Vec<DA>) -> Res<DA> {
    let n = unk.len();
    Ok(match e {
        E::Num(..) | E::Hole => DA::zero(n),
        E::Unit(u) => DA { a: u.d.map(|x| x as f64), k: vec![0.0; n] },
        E::Var(name) => match unk.iter().position(|x| x == name) {
            Some(i) => {
                let mut d = DA::zero(n);
                d.k[i] = 1.0;
                d
            }
            None => {
                let q = eval::eval(e, c)?;
                DA { a: q.d.map(|x| x as f64), k: vec![0.0; n] }
            }
        },
        E::Neg(a) | E::Par(a) | E::Abs(a) => infer(a, c, unk, cons)?,
        E::Fact(a) => {
            let t = infer(a, c, unk, cons)?;
            cons.push(t);
            DA::zero(n)
        }
        E::Bin(op, a, b) => {
            let x = infer(a, c, unk, cons)?;
            match op {
                Op::Add | Op::Sub => {
                    let y = infer(b, c, unk, cons)?;
                    cons.push(x.lin(&y, -1.0));
                    x
                }
                Op::Mul | Op::IMul | Op::UMul => x.lin(&infer(b, c, unk, cons)?, 1.0),
                Op::Div | Op::Frac => x.lin(&infer(b, c, unk, cons)?, -1.0),
                Op::Pow => {
                    if has_unknown(b, unk) {
                        cons.push(x);
                        let t = infer(b, c, unk, cons)?;
                        cons.push(t);
                        DA::zero(n)
                    } else {
                        let p = eval::eval(b, c)?;
                        x.scale(p.v)
                    }
                }
            }
        }
        E::Root(a, k) => {
            let x = infer(a, c, unk, cons)?;
            let k = match k {
                Some(k) if !has_unknown(k, unk) => eval::eval(k, c)?.v,
                Some(_) => return Err("Wurzelgrad darf nicht unbekannt sein".into()),
                None => 2.0,
            };
            x.scale(1.0 / k)
        }
        E::Call(_, a, b) => {
            let t = infer(a, c, unk, cons)?;
            cons.push(t);
            if let Some(b) = b {
                let t = infer(b, c, unk, cons)?;
                        cons.push(t);
            }
            DA::zero(n)
        }
    })
}

/// Löst A·x = b (m × n) nach Gauß; freie Unbekannte = 0. None = widersprüchlich.
pub fn lin_solve(mut a: Vec<Vec<f64>>, mut b: Vec<f64>, n: usize) -> Option<Vec<f64>> {
    let m = a.len();
    let mut piv_col = Vec::new();
    let mut row = 0;
    let scale = a.iter().flatten().fold(0.0f64, |s, x| s.max(x.abs())).max(1e-300);
    for col in 0..n {
        if row >= m {
            break;
        }
        let (p, best) = (row..m).map(|r| (r, a[r][col].abs())).fold((row, -1.0), |acc, x| if x.1 > acc.1 { x } else { acc });
        if best <= 1e-12 * scale {
            continue;
        }
        a.swap(row, p);
        b.swap(row, p);
        let pv = a[row][col];
        for r in 0..m {
            if r != row {
                let f = a[r][col] / pv;
                if f != 0.0 {
                    for c2 in 0..n {
                        a[r][c2] -= f * a[row][c2];
                    }
                    b[r] -= f * b[row];
                }
            }
        }
        piv_col.push(col);
        row += 1;
    }
    let bs = b.iter().fold(0.0f64, |s, x| s.max(x.abs())).max(1e-300);
    for r in row..m {
        if b[r].abs() > 1e-9 * bs.max(scale) {
            return None;
        }
    }
    let mut x = vec![0.0; n];
    for (r, &col) in piv_col.iter().enumerate() {
        x[col] = b[r] / a[r][col];
    }
    Some(x)
}

/// Dimensionen der Unbekannten aus den Gleichungen.
fn dims(eqs: &[&Eqn], unk: &[String], c: &Ctx) -> Res<Vec<units::Dim>> {
    let n = unk.len();
    let mut cons = Vec::new();
    for e in eqs {
        let l = infer(&e.l, c, unk, &mut cons)?;
        let r = infer(&e.r, c, unk, &mut cons)?;
        cons.push(l.lin(&r, -1.0));
    }
    let mut out = vec![ONE; n];
    for comp in 0..7 {
        let a: Vec<Vec<f64>> = cons.iter().map(|d| d.k.clone()).collect();
        let b: Vec<f64> = cons.iter().map(|d| -d.a[comp]).collect();
        let Some(x) = lin_solve(a, b, n) else { return Err("Einheiten passen nicht zusammen".into()) };
        for i in 0..n {
            if (x[i] - x[i].round()).abs() > 1e-6 {
                return Err(format!("Einheit von {} nicht bestimmbar", unk[i]));
            }
            out[i][comp] = x[i].round() as i8;
        }
    }
    Ok(out)
}

/// Summe der Beträge der Summanden (für relative Genauigkeit).
fn mag(e: &E, c: &Ctx) -> f64 {
    match e {
        E::Bin(Op::Add | Op::Sub, a, b) => mag(a, c) + mag(b, c),
        E::Par(a) | E::Neg(a) => mag(a, c),
        _ => eval::eval(e, c).map(|q| q.v.abs()).unwrap_or(0.0),
    }
}

struct Sys<'a> {
    eqs: Vec<&'a Eqn>,
    unk: &'a [String],
    d: Vec<units::Dim>,
    env: &'a Env,
    o: &'a Opts,
}

impl Sys<'_> {
    fn qs(&self, x: &[f64]) -> Vec<(String, Q)> {
        self.unk.iter().zip(x).zip(&self.d).map(|((n, v), d)| (n.clone(), Q::with_dim(*v, *d))).collect()
    }
    /// Rest je Gleichung (l − r) und Größenordnung max(|l|, |r|).
    fn f(&self, x: &[f64]) -> Option<Vec<(f64, f64)>> {
        let extra = self.qs(x);
        let c = Ctx { env: self.env, extra: &extra, o: self.o };
        let mut out = Vec::new();
        for e in &self.eqs {
            let l = eval::eval(&e.l, &c).ok()?;
            let r = eval::eval(&e.r, &c).ok()?;
            if l.d != r.d {
                return None;
            }
            let conv = |q: &Q, o: &Q| if !q.ang && o.ang && self.o.deg { q.v.to_radians() } else { q.v };
            let (lv, rv) = (conv(&l, &r), conv(&r, &l));
            if !lv.is_finite() || !rv.is_finite() {
                return None;
            }
            // Größenordnung: Summe der Beträge der Summanden (x + y − 10 = 0 hat Größe 10, nicht 0)
            let sc = mag(&e.l, &c).max(mag(&e.r, &c)).max(lv.abs()).max(rv.abs());
            out.push((lv - rv, sc));
        }
        Some(out)
    }
    fn ok(&self, x: &[f64]) -> bool {
        self.f(x).is_some_and(|f| f.iter().all(|(r, s)| r.abs() <= 1e-9 * s.max(1e-300) || *r == 0.0))
    }
}

/// Lösungen: je Lösung ein Wert pro Unbekannter (Reihenfolge wie `unk`).
pub fn solve(eqs: &[&Eqn], unk: &[String], env: &Env, o: &Opts) -> Res<Vec<Vec<Q>>> {
    let extra: Vec<(String, Q)> = Vec::new();
    let c = Ctx { env, extra: &extra, o };
    let d = dims(eqs, unk, &c)?;
    let s = Sys { eqs: eqs.to_vec(), unk, d, env, o };
    let sols = if unk.len() == 1 && eqs.len() == 1 { solve1(&s)? } else { solve_n(&s)? };
    // Einheit wie geschrieben: steht die Unbekannte allein auf einer Seite, die Einheit der anderen Seite (M = F·r → N·m)
    Ok(sols
        .into_iter()
        .map(|x| {
            let extra = s.qs(&x);
            let c = Ctx { env, extra: &extra, o };
            x.iter()
                .zip(&s.d)
                .enumerate()
                .map(|(i, (v, d))| {
                    let me = E::Var(unk[i].clone());
                    let u = eqs
                        .iter()
                        .find_map(|e| {
                            let other = if e.l == me { &e.r } else if e.r == me { &e.l } else { return None };
                            eval::eval(other, &c).ok().filter(|q| q.d == *d && !q.u.is_empty() && units::disp_dim(&q.u) == Some(*d)).map(|q| q.u)
                        })
                        .unwrap_or_else(|| units::disp_for(*d));
                    Q { v: *v, d: *d, ang: false, u }
                })
                .collect()
        })
        .collect())
}

fn solve1(s: &Sys) -> Res<Vec<Vec<f64>>> {
    let f = |x: f64| s.f(&[x]).map(|v| v[0]);
    // Polynom bis Grad 2?
    if let (Some(fm), Some(f0), Some(fp)) = (f(-1.0), f(0.0), f(1.0)) {
        let c = f0.0;
        let a = (fp.0 + fm.0) / 2.0 - c;
        let b = (fp.0 - fm.0) / 2.0;
        let poly = [2.0, -2.0, 0.5, 3.0, -7.0].iter().all(|&x| {
            f(x).is_some_and(|(r, sc)| {
                let p = a * x * x + b * x + c;
                (r - p).abs() <= 1e-9 * (sc + (a * x * x).abs() + (b * x).abs() + c.abs()).max(1e-300)
            })
        });
        if poly {
            let mag = a.abs() + b.abs() + c.abs();
            let sc = f0.1.max(fp.1).max(fm.1);
            if a.abs() <= 1e-12 * mag {
                if b.abs() <= 1e-12 * sc.max(1e-300) {
                    return Err(if c.abs() <= 1e-12 * sc.max(1e-300) { "jeder Wert erfüllt die Gleichung".into() } else { "keine Lösung".into() });
                }
                return Ok(vec![vec![-c / b]]);
            }
            let disc = b * b - 4.0 * a * c;
            if disc < -1e-12 * (b * b).max((4.0 * a * c).abs()) {
                return Err("keine reelle Lösung".into());
            }
            if disc.abs() <= 1e-12 * (b * b).max((4.0 * a * c).abs()) {
                return Ok(vec![vec![-b / (2.0 * a)]]);
            }
            let q = -0.5 * (b + b.signum() * disc.sqrt());
            let (mut x1, mut x2) = (q / a, c / q);
            if x1 > x2 {
                std::mem::swap(&mut x1, &mut x2);
            }
            return Ok(vec![vec![x1], vec![x2]]);
        }
    }
    // Numerisch: Vorzeichenwechsel auf einem logarithmischen Raster suchen, dann halbieren
    let mut xs: Vec<f64> = (-120..=120).map(|k| -(10f64.powf(-k as f64 / 8.0))).collect();
    xs.push(0.0);
    xs.extend((-120..=120).rev().map(|k| 10f64.powf(-k as f64 / 8.0)));
    xs.sort_by(|a, b| a.partial_cmp(b).unwrap());
    let vals: Vec<Option<f64>> = xs.iter().map(|&x| f(x).map(|v| v.0)).collect();
    let mut roots: Vec<f64> = Vec::new();
    let add = |r: f64, roots: &mut Vec<f64>| {
        if s.ok(&[r]) && !roots.iter().any(|x| (x - r).abs() <= 1e-7 * x.abs().max(r.abs()).max(1e-300)) {
            roots.push(r);
        }
    };
    for i in 0..xs.len() {
        if vals[i] == Some(0.0) {
            add(xs[i], &mut roots);
        }
        if i + 1 == xs.len() {
            break;
        }
        let (Some(fa), Some(fb)) = (vals[i], vals[i + 1]) else { continue };
        if fa.signum() == fb.signum() || fa == 0.0 || fb == 0.0 {
            continue;
        }
        let (mut lo, mut hi, mut flo) = (xs[i], xs[i + 1], fa);
        for _ in 0..200 {
            let mid = 0.5 * (lo + hi);
            if mid == lo || mid == hi {
                break;
            }
            let Some(fm) = f(mid).map(|v| v.0) else { break };
            if fm == 0.0 {
                lo = mid;
                hi = mid;
                break;
            }
            if fm.signum() == flo.signum() {
                lo = mid;
                flo = fm;
            } else {
                hi = mid;
            }
        }
        add(0.5 * (lo + hi), &mut roots);
    }
    if roots.is_empty() {
        // Newton als Ausweg (Berührpunkte)
        for &x0 in &[1.0, -1.0, 1e-3, 1e3] {
            if let Some(r) = newton(s, vec![x0]) {
                add(r[0], &mut roots);
            }
        }
    }
    if roots.is_empty() {
        return Err("keine Lösung gefunden".into());
    }
    roots.sort_by(|a, b| a.partial_cmp(b).unwrap());
    roots.truncate(4);
    Ok(roots.into_iter().map(|r| vec![r]).collect())
}

fn jac(s: &Sys, x: &[f64], f0: &[(f64, f64)]) -> Option<Vec<Vec<f64>>> {
    let n = x.len();
    let mut j = vec![vec![0.0; n]; f0.len()];
    for k in 0..n {
        let h = 1e-7 * x[k].abs().max(1e-7);
        let mut x2 = x.to_vec();
        x2[k] += h;
        let f1 = s.f(&x2)?;
        for r in 0..f0.len() {
            j[r][k] = (f1[r].0 - f0[r].0) / h;
        }
    }
    Some(j)
}

/// Gauß-Newton-Schritt: (JᵀJ) Δ = −Jᵀ f (bei quadratischem J direkt).
fn step(j: &[Vec<f64>], f: &[(f64, f64)], n: usize) -> Option<Vec<f64>> {
    if j.len() == n {
        return lin_solve(j.to_vec(), f.iter().map(|x| -x.0).collect(), n);
    }
    let mut a = vec![vec![0.0; n]; n];
    let mut b = vec![0.0; n];
    for r in 0..j.len() {
        for p in 0..n {
            b[p] -= j[r][p] * f[r].0;
            for q in 0..n {
                a[p][q] += j[r][p] * j[r][q];
            }
        }
    }
    lin_solve(a, b, n)
}

fn newton(s: &Sys, mut x: Vec<f64>) -> Option<Vec<f64>> {
    let n = x.len();
    let norm = |f: &[(f64, f64)]| f.iter().map(|v| v.0 * v.0).sum::<f64>();
    for _ in 0..200 {
        let f0 = s.f(&x)?;
        if s.ok(&x) {
            return Some(x);
        }
        let j = jac(s, &x, &f0)?;
        let dx = step(&j, &f0, n)?;
        // gedämpft
        let n0 = norm(&f0);
        let mut t = 1.0;
        loop {
            let x2: Vec<f64> = x.iter().zip(&dx).map(|(a, b)| a + t * b).collect();
            if let Some(f2) = s.f(&x2) {
                if norm(&f2) < n0 || t < 1e-3 {
                    x = x2;
                    break;
                }
            }
            t *= 0.5;
            if t < 1e-6 {
                return None;
            }
        }
    }
    s.ok(&x).then_some(x)
}

fn solve_n(s: &Sys) -> Res<Vec<Vec<f64>>> {
    let n = s.unk.len();
    if s.eqs.len() < n {
        return Err(format!("zu wenige Gleichungen für {}", s.unk.join(", ")));
    }
    // linear? Dann exakt.
    let x0 = vec![1.0; n];
    let x1: Vec<f64> = (0..n).map(|i| 2.0 + i as f64 * 0.37).collect();
    if let (Some(f0), Some(f1)) = (s.f(&x0), s.f(&x1)) {
        if let (Some(j0), Some(j1)) = (jac(s, &x0, &f0), jac(s, &x1, &f1)) {
            let same = j0.iter().flatten().zip(j1.iter().flatten()).all(|(a, b)| (a - b).abs() <= 1e-6 * a.abs().max(b.abs()).max(1e-300));
            if same {
                // Jacobi-Matrix exakt aus Differenzen mit Schritt 1
                let mut j = vec![vec![0.0; n]; f0.len()];
                let z = vec![0.0; n];
                if let Some(fz) = s.f(&z) {
                    for k in 0..n {
                        let mut e = z.clone();
                        e[k] = 1.0;
                        if let Some(fe) = s.f(&e) {
                            for r in 0..f0.len() {
                                j[r][k] = fe[r].0 - fz[r].0;
                            }
                        }
                    }
                    let rank_ok = {
                        // eindeutig lösbar?
                        let mut m = j.clone();
                        let mut rank = 0;
                        for col in 0..n {
                            if let Some(p) = (rank..m.len()).max_by(|&a, &b| m[a][col].abs().partial_cmp(&m[b][col].abs()).unwrap()) {
                                let sc = m.iter().flatten().fold(0.0f64, |s, x| s.max(x.abs())).max(1e-300);
                                if m[p][col].abs() > 1e-10 * sc {
                                    m.swap(rank, p);
                                    for r in 0..m.len() {
                                        if r != rank {
                                            let f = m[r][col] / m[rank][col];
                                            for c in 0..n {
                                                m[r][c] -= f * m[rank][c];
                                            }
                                        }
                                    }
                                    rank += 1;
                                }
                            }
                        }
                        rank == n
                    };
                    if !rank_ok {
                        return Err("Gleichungen legen die Unbekannten nicht eindeutig fest".into());
                    }
                    let b: Vec<f64> = fz.iter().map(|v| -v.0).collect();
                    if let Some(x) = step(&j, &fz.iter().map(|v| (v.0, v.1)).collect::<Vec<_>>(), n).or_else(|| lin_solve(j.clone(), b, n)) {
                        if s.ok(&x) {
                            return Ok(vec![x]);
                        }
                        if let Some(x) = newton(s, x) {
                            return Ok(vec![x]);
                        }
                        return Err("Gleichungen widersprechen sich".into());
                    }
                    return Err("Gleichungen widersprechen sich".into());
                }
            }
        }
    }
    for start in [1.0, 0.5, 10.0, -1.0, 1e-3, 1e3, 0.1, 100.0] {
        let x0: Vec<f64> = (0..n).map(|i| start * (1.0 + 0.13 * i as f64)).collect();
        if let Some(x) = newton(s, x0) {
            return Ok(vec![x]);
        }
    }
    Err("keine Lösung gefunden".into())
}
