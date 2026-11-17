//! Rechenkern (Werkzeug „Rechnen“): rechnet Formelzeilen einer Seite mit Variablen und Einheiten aus.
//!
//! Zeilenarten (getippt oder als LaTeX eines Formel-Objekts):
//! - Zuweisung `a: 1 N` oder `a = 1 N`, Formel `M = a·b` (rechte Seite bekannt → `M` bekannt)
//! - Gleichung mit Unbekannten `p = F/A`, `x + y = 10` (wird beim Fragen gelöst)
//! - Ergebniszeile `M =`, `M = ?`, `M = kN·m` (Wunsch-Einheit), `3·4 + 2 =`, `M = a·b =`
//!
//! Variablen gelten von oben nach unten; spätere Zuweisungen überschreiben frühere.

mod eval;
mod parse;
mod solve;
pub mod units;

use eval::{Ctx, Env, Q};
use parse::{Tok, E};
use solve::Eqn;
use std::collections::{HashMap, HashSet};

/// Farbe von Fehlern.
pub const ERR: u32 = crate::math::ERR_COLOR;

/// Einstellungen des Rechenkerns.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Opts {
    /// Rechnen eingeschaltet (aus = keine Ergebnisse, z. B. für Prüfungen).
    pub on: bool,
    /// Winkel ohne Einheit in Grad (sonst Bogenmaß).
    pub deg: bool,
    /// Gültige Stellen im Ergebnis (2–10).
    pub digits: u8,
    /// Dezimalkomma statt Punkt.
    pub comma: bool,
    /// Rechenweg unter Ergebniszeilen anzeigen.
    pub steps: bool,
}

impl Default for Opts {
    fn default() -> Opts {
        Opts { on: true, deg: true, digits: 4, comma: true, steps: false }
    }
}

/// Ergebnis einer Zeile.
#[derive(Clone, Debug, PartialEq)]
pub enum Out {
    /// Ergebniszeile: `left` (LaTeX bis einschließlich „=“), Wert (LaTeX, mit Einheit, wenn keine Wunsch-Einheit),
    /// Wunsch-Einheit (LaTeX, sonst leer), Rechenweg (LaTeX-Zeilen), Fehler/Warnung.
    Result { left: String, value: String, unit: String, steps: Vec<String>, err: Option<String> },
    /// Rechenzeile mit Fehler (z. B. „Einheiten passen nicht: N + m“).
    Error(String),
}

impl Out {
    /// Zeile als Klartext (Tests, Barrierefreiheit).
    pub fn text(&self) -> String {
        match self {
            Out::Result { value, unit, err, .. } => match err {
                Some(e) => format!("? ({e})"),
                None if unit.is_empty() => value.clone(),
                None => format!("{value} {unit}"),
            },
            Out::Error(e) => format!("Fehler: {e}"),
        }
    }
}

/// Getippte Zeile (z. B. `a: 1,5 kN`, `M = a·b`, `x = sqrt(2)/3`) → LaTeX-Quelltext für ein Formel-Objekt.
/// Nicht lesbare Zeilen bleiben unverändert.
pub fn to_latex(plain: &str) -> String {
    let Ok(t) = parse::lex(plain) else { return plain.to_string() };
    let parts = parse::split_eq(&t);
    let colon = t.len() > 1 && matches!(t[0], Tok::Id(_)) && t[1] == Tok::Sym(':');
    let mut out = Vec::new();
    for (i, p) in parts.iter().enumerate() {
        match parse::part_latex(p, i > 0) {
            Some(s) => out.push(s),
            None => return plain.to_string(),
        }
    }
    join(out, colon, "=")
}

/// LaTeX-Quelltext eines Formel-Objekts → getippte Schreibweise (zum Bearbeiten).
pub fn to_plain(latex: &str) -> String {
    let Ok(t) = parse::lex(latex) else { return latex.to_string() };
    let parts = parse::split_eq(&t);
    let colon = t.len() > 1 && matches!(t[0], Tok::Id(_)) && t[1] == Tok::Sym(':');
    let mut out = Vec::new();
    for (i, p) in parts.iter().enumerate() {
        match parse::part_plain(p, i > 0) {
            Some(s) => out.push(s),
            None => return latex.to_string(),
        }
    }
    join(out, colon, "=")
}

fn join(parts: Vec<String>, colon: bool, eq: &str) -> String {
    let mut s = String::new();
    for (i, p) in parts.iter().enumerate() {
        if i > 0 {
            s.push_str(if i == 1 && colon { ":" } else { " " });
            if !(i == 1 && colon) {
                s.push_str(eq);
            }
            if !p.is_empty() {
                s.push(' ');
            }
        }
        s.push_str(p);
    }
    s.trim_end().to_string()
}

/// Ist die Zeile eine Rechenzeile (enthält „=“ bzw. „name:“ und ist lesbar)?
pub fn is_calc_line(src: &str) -> bool {
    parse::lex(src).is_ok_and(|t| parse::split_eq(&t).len() > 1)
}

struct Def {
    /// LaTeX der Formel (rechte Seite) und der Ausdruck selbst (für den Rechenweg)
    e: E,
}

/// Rechnet die Zeilen einer Seite (in Lesereihenfolge) aus. Ein Eintrag je Zeile, None = nichts anzuzeigen.
pub fn page(lines: &[&str], o: &Opts) -> Vec<Option<Out>> {
    let toks: Vec<Option<Vec<Tok>>> = lines.iter().map(|s| parse::lex(s).ok()).collect();
    // Namen, die auf der Seite zugewiesen werden (gegen Verwechslung mit Einheiten in Ergebniszeilen)
    let mut names: HashSet<String> = HashSet::new();
    for t in toks.iter().flatten() {
        let parts = parse::split_eq(t);
        if parts.len() > 1 {
            if let [Tok::Id(n)] = parts[0] {
                names.insert(n.clone());
            }
            if let Ok(e) = parse::expr(parts[0]) {
                let mut v = Vec::new();
                e.vars(&mut v);
                names.extend(v);
            }
        }
    }
    let mut env: Env = HashMap::new();
    let mut defs: HashMap<String, Def> = HashMap::new();
    let mut rels: Vec<Eqn> = Vec::new();
    let mut out = Vec::new();
    for t in &toks {
        let Some(t) = t else {
            out.push(None);
            continue;
        };
        out.push(line(t, o, &names, &mut env, &mut defs, &mut rels));
    }
    out
}

fn is_query_tail(t: &[Tok], names: &HashSet<String>) -> Option<Option<E>> {
    if t.iter().all(|x| *x == Tok::Hole) {
        return Some(None);
    }
    let u = parse::unit_only(t)?;
    let mut ids = Vec::new();
    for x in t {
        if let Tok::Id(s) = x {
            ids.push(s);
        }
    }
    // Hat die Seite Variablen mit diesen Namen, ist es eine Formel (v = s/t), keine Einheit – außer mit Platzhalter
    if t[0] != Tok::Hole && ids.iter().any(|s| names.contains(*s)) {
        return None;
    }
    Some(Some(u))
}

fn line(t: &[Tok], o: &Opts, names: &HashSet<String>, env: &mut Env, defs: &mut HashMap<String, Def>, rels: &mut Vec<Eqn>) -> Option<Out> {
    let parts = parse::split_eq(t);
    if parts.len() < 2 {
        return None;
    }
    let last = parts[parts.len() - 1];
    let query = is_query_tail(last, names);
    let eq_parts = if query.is_some() { &parts[..parts.len() - 1] } else { &parts[..] };
    if eq_parts.iter().any(|p| p.is_empty()) {
        return None;
    }
    let mut exprs = Vec::new();
    for p in eq_parts {
        match parse::expr(p) {
            Ok(e) if !e.has_hole() => exprs.push(e),
            Ok(_) => return None,
            Err(e) => {
                return query.is_some().then(|| Out::Result {
                    left: String::new(),
                    value: "?".into(),
                    unit: String::new(),
                    steps: Vec::new(),
                    err: Some(format!("nicht lesbar ({e})")),
                })
            }
        }
    }
    let mut err = None;
    for w in exprs.windows(2) {
        if let Err(e) = equation(&w[0], &w[1], o, env, defs, rels) {
            err.get_or_insert(e);
        }
    }
    let Some(wish) = query else {
        return err.map(Out::Error);
    };
    let target = exprs.last().unwrap();
    let left = format!("{} =", exprs.iter().map(|e| parse::latex(e, None)).collect::<Vec<_>>().join(" = "));
    let unit = wish.as_ref().map(parse::unit_latex).unwrap_or_default();
    let fail = |e: String, left: String, unit: String| Out::Result { left, value: "?".into(), unit, steps: Vec::new(), err: Some(e) };
    if let Some(e) = err {
        return Some(fail(e, left, unit));
    }
    match query_value(target, o, env, defs, rels) {
        Ok((vals, mut steps)) => {
            let mut parts = Vec::new();
            for q in &vals {
                match show(q, wish.as_ref(), o, env) {
                    Ok(s) => parts.push(s),
                    Err(e) => return Some(fail(e, left, unit)),
                }
            }
            let value = parts.join("\\;\\text{oder}\\;");
            if o.steps && !steps.is_empty() {
                let lt = parse::latex(target, None);
                steps.push(format!("{lt} = {value}{}", if unit.is_empty() { String::new() } else { format!("\\,{unit}") }));
            } else {
                steps.clear();
            }
            Some(Out::Result { left, value, unit, steps, err: None })
        }
        Err(e) => Some(fail(e, left, unit)),
    }
}

/// Wert in der Wunsch-Einheit (nur Zahl) bzw. mit passender Einheit.
fn show(q: &Q, wish: Option<&E>, o: &Opts, env: &Env) -> Result<String, String> {
    let Some(w) = wish else { return Ok(eval::q_latex(q, None, o)) };
    let c = Ctx { env, extra: &[], o };
    let wq = eval::eval(w, &c)?;
    if wq.d != q.d {
        return Err(format!("Einheit passt nicht: Ergebnis hat {}", eval::q_plain(&Q { v: 1.0, ..q.clone() }, o).trim_start_matches("1 ")));
    }
    let (v, _) = eval::shown(q, &wq.u, o);
    let v = if wq.u.len() == 1 && units::lookup(&wq.u[0].0).is_some_and(|u| u.ang) {
        v
    } else if wq.u.len() == 1 && wq.u[0].0 == "°C" {
        v
    } else {
        q.v / wq.v
    };
    Ok(eval::num_latex(v, o))
}

fn equation(l: &E, r: &E, o: &Opts, env: &mut Env, defs: &mut HashMap<String, Def>, rels: &mut Vec<Eqn>) -> Result<(), String> {
    let c = Ctx { env, extra: &[], o };
    let rv = eval::eval(r, &c);
    let lv = eval::eval(l, &c);
    let unknown = |e: &Result<Q, String>| matches!(e, Err(s) if s.starts_with("unbekannt"));
    if let E::Var(n) = l {
        match &rv {
            Ok(q) => {
                env.insert(n.clone(), q.clone());
                defs.insert(n.clone(), Def { e: r.clone() });
                return Ok(());
            }
            Err(e) if !unknown(&rv) => return Err(e.clone()),
            _ => {}
        }
    }
    if let (E::Var(n), Ok(q)) = (r, &lv) {
        if !env.contains_key(n) {
            env.insert(n.clone(), q.clone());
            return Ok(());
        }
    }
    match (&lv, &rv) {
        (Err(e), _) if !unknown(&lv) => return Err(e.clone()),
        (_, Err(e)) if !unknown(&rv) => return Err(e.clone()),
        (Ok(a), Ok(b)) => {
            if a.d != b.d {
                return Err("Einheiten der beiden Seiten passen nicht".into());
            }
            return Ok(());
        }
        _ => {}
    }
    // Definition vergessen, falls neu als Gleichung
    rels.push(Eqn { l: l.clone(), r: r.clone(), tex: format!("{} = {}", parse::latex(l, None), parse::latex(r, None)) });
    Ok(())
}

/// Rechenweg-Ersatz: Variable → Wert.
fn subst_tex<'a>(env: &'a Env, o: &Opts) -> impl Fn(&str) -> Option<(String, u8)> + 'a {
    let o = *o;
    move |n: &str| {
        let q = env.get(n)?;
        let s = eval::q_latex(q, None, &o);
        let pr = if q.v < 0.0 { 2 } else if s.contains("\\,") || s.contains("cdot") || s.contains("circ") { 3 } else { 5 };
        Some((s, pr))
    }
}

fn unknowns(e: &E, env: &Env) -> Vec<String> {
    let mut v = Vec::new();
    e.vars(&mut v);
    v.retain(|n| !env.contains_key(n) && eval::constant(n).is_none());
    v
}

fn query_value(target: &E, o: &Opts, env: &mut Env, defs: &HashMap<String, Def>, rels: &[Eqn]) -> Result<(Vec<Q>, Vec<String>), String> {
    let mut steps = Vec::new();
    let need = unknowns(target, env);
    if need.is_empty() {
        let c = Ctx { env, extra: &[], o };
        let q = eval::eval(target, &c)?;
        // Rechenweg: Definition, eingesetzt
        let sub = subst_tex(env, o);
        match target {
            E::Var(n) => {
                if let Some(d) = defs.get(n) {
                    let f = parse::latex(&d.e, None);
                    let s = parse::latex(&d.e, Some(&sub));
                    let nl = parse::latex(target, None);
                    steps.push(format!("{nl} = {f}"));
                    if s != f {
                        steps.push(format!("{nl} = {s}"));
                    }
                }
            }
            _ => {
                let s = parse::latex(target, Some(&sub));
                if s != parse::latex(target, None) {
                    steps.push(s);
                }
            }
        }
        return Ok((vec![q], steps));
    }
    // Lösen
    let (sol, used) = solve_for(&need, o, env, rels)?;
    for u in &used {
        steps.push(format!("\\text{{aus }} {}", rels[*u].tex));
    }
    steps.push(format!("\\text{{gelöst nach }} {}", need.iter().map(|n| parse::name_latex(n)).collect::<Vec<_>>().join(", ")));
    if sol.len() == 1 {
        for (n, q) in need.iter().zip(&sol[0]) {
            env.insert(n.clone(), q.clone());
        }
        let c = Ctx { env, extra: &[], o };
        return Ok((vec![eval::eval(target, &c)?], steps));
    }
    // mehrere Lösungen: nicht übernehmen
    let mut vals = Vec::new();
    for s in &sol {
        let extra: Vec<(String, Q)> = need.iter().cloned().zip(s.iter().cloned()).collect();
        let c = Ctx { env, extra: &extra, o };
        vals.push(eval::eval(target, &c)?);
    }
    Ok((vals, steps))
}

/// Sucht passende Gleichungen und löst nach `need`. Liefert Lösungen (Werte je `need`) und benutzte Gleichungen.
fn solve_for(need: &[String], o: &Opts, env: &mut Env, rels: &[Eqn]) -> Result<(Vec<Vec<Q>>, Vec<usize>), String> {
    let mut used = Vec::new();
    // 1. Gleichungen mit genau einer Unbekannten nacheinander lösen
    let mut done: HashSet<usize> = HashSet::new();
    let mut local = env.clone();
    loop {
        if need.iter().all(|n| local.contains_key(n)) {
            break;
        }
        let mut progress = false;
        for (i, r) in rels.iter().enumerate() {
            if done.contains(&i) {
                continue;
            }
            let mut u = unknowns(&r.l, &local);
            for x in unknowns(&r.r, &local) {
                if !u.contains(&x) {
                    u.push(x);
                }
            }
            if u.len() != 1 {
                continue;
            }
            done.insert(i);
            let Ok(sol) = solve::solve(&[r], &u, &local, o) else { continue };
            if sol.len() == 1 {
                local.insert(u[0].clone(), sol[0][0].clone());
                used.push(i);
                progress = true;
            } else if need.len() == 1 && u[0] == need[0] {
                used.push(i);
                return Ok((sol, used));
            }
        }
        if !progress {
            break;
        }
    }
    if need.iter().all(|n| local.contains_key(n)) {
        // Zwischenergebnisse übernehmen
        for (k, v) in &local {
            env.entry(k.clone()).or_insert_with(|| v.clone());
        }
        return Ok((vec![need.iter().map(|n| local[n].clone()).collect()], used));
    }
    // 2. Gleichungssystem: kleinste Auswahl, deren Unbekannte genau so viele sind wie Gleichungen
    let cand: Vec<(usize, Vec<String>)> = rels
        .iter()
        .enumerate()
        .filter(|(i, _)| !done.contains(i) || !used.contains(i))
        .map(|(i, r)| {
            let mut u = unknowns(&r.l, &local);
            for x in unknowns(&r.r, &local) {
                if !u.contains(&x) {
                    u.push(x);
                }
            }
            (i, u)
        })
        .filter(|(_, u)| !u.is_empty())
        .collect();
    // nur Gleichungen, die (über gemeinsame Unbekannte) mit den gesuchten zusammenhängen
    let mut reach: Vec<String> = need.iter().filter(|n| !local.contains_key(*n)).cloned().collect();
    let mut comp: Vec<usize> = Vec::new();
    loop {
        let mut grew = false;
        for (k, (_, u)) in cand.iter().enumerate() {
            if !comp.contains(&k) && u.iter().any(|x| reach.contains(x)) {
                comp.push(k);
                for x in u {
                    if !reach.contains(x) {
                        reach.push(x.clone());
                    }
                }
                grew = true;
            }
        }
        if !grew {
            break;
        }
    }
    let missing: Vec<String> = need.iter().filter(|n| !local.contains_key(*n)).cloned().collect();
    if comp.is_empty() {
        return Err(format!("unbekannt: {}", missing.join(", ")));
    }
    let comp: Vec<usize> = comp.into_iter().take(14).collect();
    let mut best: Option<(Vec<usize>, Vec<String>)> = None;
    for mask in 1u32..(1 << comp.len()) {
        let sel: Vec<usize> = (0..comp.len()).filter(|b| mask & (1 << b) != 0).collect();
        if best.as_ref().is_some_and(|b| b.0.len() <= sel.len()) {
            continue;
        }
        let mut u: Vec<String> = Vec::new();
        for &k in &sel {
            for x in &cand[comp[k]].1 {
                if !u.contains(x) {
                    u.push(x.clone());
                }
            }
        }
        if u.len() == sel.len() && missing.iter().all(|m| u.contains(m)) {
            best = Some((sel.iter().map(|&k| cand[comp[k]].0).collect(), u));
        }
    }
    let Some((sel, u)) = best else {
        let mut all: Vec<String> = Vec::new();
        for k in &comp {
            for x in &cand[*k].1 {
                if !all.contains(x) {
                    all.push(x.clone());
                }
            }
        }
        return Err(format!("zu wenige Angaben: unbekannt {}", all.join(", ")));
    };
    let eqs: Vec<&Eqn> = sel.iter().map(|&i| &rels[i]).collect();
    let sol = solve::solve(&eqs, &u, &local, o)?;
    used.extend(sel);
    let pick = |s: &Vec<Q>| -> Vec<Q> { need.iter().map(|n| local.get(n).cloned().unwrap_or_else(|| s[u.iter().position(|x| x == n).unwrap()].clone())).collect() };
    if sol.len() == 1 {
        for (k, v) in &local {
            env.entry(k.clone()).or_insert_with(|| v.clone());
        }
        for (n, q) in u.iter().zip(&sol[0]) {
            env.insert(n.clone(), q.clone());
        }
    }
    Ok((sol.iter().map(pick).collect(), used))
}

#[cfg(test)]
mod tests;
