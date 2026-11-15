//! Formerkennung: macht aus einem Freihand-Strich Linie, Pfeil, Doppelpfeil, Ellipse/Kreis, Dreieck, Viereck,
//! Fuenfeck, Sechseck, Stern oder Herz.
//!
//! Zwei Darstellungen des Strichs:
//!  - S: nach Bogenlaenge neu abgetastet und geglaettet, nur fuer die Topologie (geschlossen?, Ecken, Pfeilspitzen).
//!  - U: nach Bogenlaenge neu abgetastet, NICHT geglaettet, fuer die Passung, damit die Form deckungsgleich auf dem
//!    Strich liegt.
//!
//! Jedes Modell wird geometrisch an U angepasst (Seiten = Ausgleichsgeraden, Ecken = Schnittpunkte, Ellipse per
//! Gauss-Newton mit orthogonalen Abstaenden); entschieden wird ueber das normierte Residuum mit Komplexitaetsstrafe
//! und Eck-Statistik. 1:1-Port der Java-Fassung.

use crate::item::{self, Item, Kind};
use std::f64::consts::PI;

const N: usize = 128; // geglaettete Punkte (Topologie)
const M: usize = 160; // ungeglaettete Punkte (Passung)

/// Ergebnisform (wird am Ende zu einem Item).
struct G {
    kind: i32,
    q: Vec<f32>,
    ang: f32,
}

impl G {
    fn new(kind: i32, q: Vec<f32>) -> G {
        G { kind, q, ang: 0.0 }
    }
}

/// Einrast-Einstellungen (Formerkennung und Griffe), in den App-Einstellungen änderbar.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct SnapCfg {
    /// Erkannte Linien/Pfeile auf ein Winkelraster drehen.
    pub lines: bool,
    /// Winkelraster für erkannte Linien/Pfeile (Grad).
    pub line_step: f32,
    /// Größte Abweichung, bei der eingerastet wird (Grad).
    pub line_tol: f32,
    /// Erkannte Rechtecke/Vielecke waagerecht/senkrecht ausrichten.
    pub shapes: bool,
    /// Pfeil-Endpunkte und Drehen einer Auswahl rasten ein.
    pub handles: bool,
    /// Winkelraster der Griffe (Grad).
    pub handle_step: f32,
    /// Größte Abweichung der Griffe (Grad).
    pub handle_tol: f32,
}

impl Default for SnapCfg {
    fn default() -> Self {
        SnapCfg { lines: true, line_step: 45.0, line_tol: 6.0, shapes: true, handles: true, handle_step: 15.0, handle_tol: 4.0 }
    }
}

impl SnapCfg {
    /// Einrasten umgekehrt (Taste gedrückt): Ein wird aus, Aus wird ein.
    pub fn inverted(self) -> SnapCfg {
        SnapCfg { lines: !self.lines, shapes: !self.shapes, handles: !self.handles, ..self }
    }

    /// Rastet `deg` auf das Raster `step` ein, wenn die Abweichung höchstens `tol` beträgt.
    pub fn snap_deg(deg: f32, step: f32, tol: f32) -> Option<f32> {
        if step <= 0.0 || tol <= 0.0 {
            return None;
        }
        let r = (deg / step).round() * step;
        ((deg - r).abs() <= tol).then_some(r)
    }
}

thread_local! {
    static CFG: std::cell::Cell<SnapCfg> = std::cell::Cell::new(SnapCfg::default());
}

fn cfg() -> SnapCfg {
    CFG.with(|c| c.get())
}

/// Erkennt eine Form im Strich `s`; `sc` = Zoomfaktor. Liefert die Form oder None.
pub fn run(s: &Item, sc: f32) -> Option<Item> {
    run_cfg(s, sc, SnapCfg::default())
}

/// Wie `run`, aber mit eigenen Einrast-Einstellungen.
pub fn run_cfg(s: &Item, sc: f32, snap: SnapCfg) -> Option<Item> {
    CFG.with(|c| c.set(snap));
    let st = s.as_stroke()?;
    let hl = st.hl;
    let p = st.p.clone();
    let g = std::panic::catch_unwind(move || recognize(&p, hl, sc as f64)).ok().flatten()?;
    if g.q.iter().any(|v| !v.is_finite()) || !g.ang.is_finite() {
        return None;
    }
    let mut it = Item::shape(g.kind, g.q, s.color, s.width);
    it.layer = s.layer;
    if let Kind::Shape(sh) = &mut it.kind {
        sh.hl = hl;
        sh.ang = g.ang;
    }
    it.touch();
    Some(it)
}

#[inline]
fn hyp(a: f64, b: f64) -> f64 {
    (a * a + b * b).sqrt()
}

/// Java Math.round(double).
#[inline]
fn jround(x: f64) -> i64 {
    (x + 0.5).floor() as i64
}

#[inline]
fn clamp(v: f64, lo: f64, hi: f64) -> f64 {
    v.max(lo).min(hi)
}

fn recognize(p: &[f32], hl: bool, sc: f64) -> Option<G> {
    let n0 = p.len() / 3;
    if n0 < 3 {
        return None;
    }
    // Doppelte Punkte entfernen
    let mut rx = vec![0.0f64; n0];
    let mut ry = vec![0.0f64; n0];
    let mut m = 0usize;
    let (mut min_x, mut min_y, mut max_x, mut max_y) = (1e30f64, 1e30f64, -1e30f64, -1e30f64);
    for i in 0..n0 {
        let (px, py) = (p[i * 3] as f64, p[i * 3 + 1] as f64);
        if m > 0 && hyp(px - rx[m - 1], py - ry[m - 1]) < 1e-4 {
            continue;
        }
        rx[m] = px;
        ry[m] = py;
        m += 1;
        min_x = min_x.min(px);
        max_x = max_x.max(px);
        min_y = min_y.min(py);
        max_y = max_y.max(py);
    }
    let dg = hyp(max_x - min_x, max_y - min_y);
    if m < 3 || dg * sc < 24.0 {
        return None;
    }
    // Zittern beim Aufsetzen / beim Halten am Ende entfernen
    m = collapse_ends(&mut rx, &mut ry, m, (18.0 / sc).min(0.15 * dg));
    if m < 3 {
        return None;
    }
    rx.truncate(m);
    ry.truncate(m);

    // S: neu abtasten + glaetten
    let (mut ax, mut ay) = resample(&rx, &ry, m, 192, false);
    let sr = if m >= 80 { 3 } else if m >= 40 { 2 } else if m >= 24 { 1 } else { 0 };
    if sr > 0 {
        smooth(&mut ax, &mut ay, sr, 2, false);
    }
    let (mut x, mut y) = resample(&ax, &ay, 192, N, false);
    if m >= 24 {
        smooth(&mut x, &mut y, 1, 1, false);
    }
    // U: ungeglaettet
    let (ux, uy) = resample(&rx, &ry, m, M, false);

    let mut len = 0.0;
    for i in 1..N {
        len += hyp(x[i] - x[i - 1], y[i] - y[i - 1]);
    }
    let chord = hyp(x[N - 1] - x[0], y[N - 1] - y[0]);

    // Linie: leichtes Zittern und minimaler Bogen sind erlaubt
    if chord > 0.8 * dg && len < 1.12 * chord && dev(&x, &y, 0, N - 1) < 0.075 * chord {
        return Some(G::new(item::LINE, snap_line(x[0], y[0], x[N - 1], y[N - 1])));
    }
    if hl {
        return None;
    }

    // Schliesst der Strich?
    let mut j: usize = 0;
    let mut dmin = 1e30;
    for i in N / 2..N {
        let dd = hyp(x[i] - x[0], y[i] - y[0]);
        if dd <= dmin + 1e-9 {
            dmin = dd;
            j = i;
        }
    }
    let mut closed = dmin <= 0.22 * dg && ((N - 1 - j) as f64) <= 0.4 * N as f64;
    if closed && j < N - 1 {
        // Der Ueberhang muss dem Anfang des Strichs folgen (Ueberzeichnen), sonst ist es ein Schwanz
        let c = N - 1 - j;
        let e = (N - 1).min(2 * c + 6);
        let mut worst: f64 = 0.0;
        for i in j + 1..N {
            let mut best: f64 = 1e30;
            for t in 0..e {
                best = best.min(seg_d(x[i], y[i], x[t], y[t], x[t + 1], y[t + 1]));
            }
            worst = worst.max(best);
        }
        if worst > 0.12 * dg {
            closed = false;
        }
    }

    if !closed {
        if len < 1.0e-6 {
            return None;
        }
        if let Some(g) = arrow2(&x, &y, &ux, &uy, sc) {
            return Some(g);
        }
        return arrow(&x, &y, &ux, &uy, sc);
    }
    // Schleife: Rohpunkte bis zur Schliessstelle
    let mut sj = 0.0;
    for i in 1..=j {
        sj += hyp(x[i] - x[i - 1], y[i] - y[i - 1]);
    }
    let (lx, ly) = cut_raw(&rx, &ry, m, if len > 0.0 { sj / len } else { 1.0 });
    let (ulx, uly) = resample(&lx, &ly, lx.len(), M, true);
    closed_shape(&x, &y, j, &ulx, &uly, &lx, &ly)
}

/// Zittern am Ende und beim Aufsetzen: eine Punktwolke mit kleinem Durchmesser, langem Weg und vielen
/// Richtungsumkehrungen wird durch ihren Schwerpunkt ersetzt.
fn collapse_ends(x: &mut [f64], y: &mut [f64], mut m: usize, rad: f64) -> usize {
    if m < 8 || rad <= 0.0 {
        return m;
    }
    let k = cluster(x, y, m, m as i64 - 1, -1, rad);
    if k >= 2 {
        let k = k as usize;
        let (mut cx, mut cy) = (0.0, 0.0);
        for i in k..m {
            cx += x[i];
            cy += y[i];
        }
        x[k] = cx / (m - k) as f64;
        y[k] = cy / (m - k) as f64;
        m = k + 1;
    }
    let b = cluster(x, y, m, 0, 1, rad);
    if b >= 0 && m as i64 - b >= 4 {
        let b = b as usize;
        let (mut cx, mut cy) = (0.0, 0.0);
        for i in 0..=b {
            cx += x[i];
            cy += y[i];
        }
        x[0] = cx / (b + 1) as f64;
        y[0] = cy / (b + 1) as f64;
        let mut o = 1;
        for i in b + 1..m {
            x[o] = x[i];
            y[o] = y[i];
            o += 1;
        }
        m = o;
    }
    m
}

/// Vom Index s in Richtung dir: letzter Index der Wolke oder -1.
fn cluster(x: &[f64], y: &[f64], m: usize, s: i64, dir: i64, rad: f64) -> i64 {
    let (mut k, mut best, mut rev) = (s, -1i64, 0);
    let su = s as usize;
    let (mut l, mut r, mut t, mut b) = (x[su], x[su], y[su], y[su]);
    let (mut path, mut px, mut py) = (0.0, 0.0, 0.0);
    loop {
        let n = k + dir;
        if n < 0 || n >= m as i64 {
            break;
        }
        let (nu, ku) = (n as usize, k as usize);
        let (l2, r2, t2, b2) = (l.min(x[nu]), r.max(x[nu]), t.min(y[nu]), b.max(y[nu]));
        if hyp(r2 - l2, b2 - t2) > rad {
            break;
        }
        l = l2;
        r = r2;
        t = t2;
        b = b2;
        let (dx, dy) = (x[nu] - x[ku], y[nu] - y[ku]);
        path += hyp(dx, dy);
        if k != s && dx * px + dy * py < -0.34 * hyp(dx, dy) * hyp(px, py) {
            rev += 1;
        }
        px = dx;
        py = dy;
        k = n;
        if (k - s).abs() >= 5 && rev >= 5 && path > 3.5 * hyp(r - l, b - t) {
            best = k;
        }
    }
    best
}

/// Rohpunkte vom Anfang bis zum Bogenlaengen-Anteil f (letzter Punkt interpoliert).
fn cut_raw(x: &[f64], y: &[f64], m: usize, f: f64) -> (Vec<f64>, Vec<f64>) {
    let mut tot = 0.0;
    for i in 1..m {
        tot += hyp(x[i] - x[i - 1], y[i] - y[i - 1]);
    }
    let lim = clamp(f, 0.0, 1.0) * tot;
    let mut acc = 0.0;
    let mut k = 1;
    let (mut ex, mut ey) = (x[m - 1], y[m - 1]);
    while k < m {
        let d = hyp(x[k] - x[k - 1], y[k] - y[k - 1]);
        if acc + d >= lim {
            let t = if d > 0.0 { (lim - acc) / d } else { 0.0 };
            ex = x[k - 1] + (x[k] - x[k - 1]) * t;
            ey = y[k - 1] + (y[k] - y[k - 1]) * t;
            break;
        }
        acc += d;
        k += 1;
    }
    let k = k.min(m - 1);
    let mut ox = x[..k].to_vec();
    let mut oy = y[..k].to_vec();
    ox.push(ex);
    oy.push(ey);
    (ox, oy)
}

// ------------------------------------------------------------------ Pfeil

fn arrow(x: &[f64], y: &[f64], ux: &[f64], uy: &[f64], sc: f64) -> Option<G> {
    let mut far: f64 = 0.0;
    for i in 1..N {
        far = far.max(hyp(x[i] - x[0], y[i] - y[0]));
    }
    if far * sc < 40.0 {
        return None;
    }
    let mut k1 = 1;
    while k1 < N - 1 && hyp(x[k1] - x[0], y[k1] - y[0]) < 0.93 * far {
        k1 += 1;
    }
    if N - 1 - k1 < 4 {
        return None;
    }
    if dev(x, y, 0, k1) > 0.08 * far {
        return None;
    }
    let (mut vx, mut vy) = (x[k1] - x[0], y[k1] - y[0]);
    let vl = hyp(vx, vy);
    if vl < 1e-9 {
        return None;
    }
    vx /= vl;
    vy /= vl;
    // Spitze = weitester Punkt auf der Schaftachse
    let mut lt: f64 = 0.0;
    for i in 0..N {
        lt = lt.max((x[i] - x[0]) * vx + (y[i] - y[0]) * vy);
    }
    let (tx, ty) = (x[0] + vx * lt, y[0] + vy * lt);
    let (mut head, mut hx, mut hy) = (0.0, 0.0, 0.0);
    for i in k1 + 1..N {
        let dd = hyp(x[i] - tx, y[i] - ty);
        if dd > 0.45 * far {
            return None;
        }
        if dd > head {
            head = dd;
            hx = x[i] - tx;
            hy = y[i] - ty;
        }
    }
    if head < 0.07 * far || head > 0.42 * far {
        return None;
    }
    let cos_a = (-vx * hx - vy * hy) / head;
    let ang = clamp(cos_a, -1.0, 1.0).acos().to_degrees();
    if !(8.0..=72.0).contains(&ang) {
        return None;
    }
    let ku = (k1 as f64 * (M - 1) as f64 / (N - 1) as f64 * 0.92) as i64;
    Some(G::new(item::ARROW, fit_segment(ux, uy, 0, ku.max(2), x[0], y[0], tx, ty)))
}

/// Doppelpfeil in einem Strich: Haken – Spitze – Schaft – Spitze – Haken.
fn arrow2(x: &[f64], y: &[f64], ux: &[f64], uy: &[f64], sc: f64) -> Option<G> {
    let (mut pa, mut pb) = (0usize, N - 1);
    let mut far = 0.0;
    let mut i = 0;
    while i < N {
        let mut k = i + 2;
        while k < N {
            let d = (x[i] - x[k]) * (x[i] - x[k]) + (y[i] - y[k]) * (y[i] - y[k]);
            if d > far {
                far = d;
                pa = i;
                pb = k;
            }
            k += 2;
        }
        i += 2;
    }
    for _ in 0..2 {
        let (mut ba, mut bb) = (pa, pb);
        for i in pa.saturating_sub(2)..=(pa + 2).min(N - 1) {
            for k in pb.saturating_sub(2)..=(pb + 2).min(N - 1) {
                let d = (x[i] - x[k]) * (x[i] - x[k]) + (y[i] - y[k]) * (y[i] - y[k]);
                if d > far && i < k {
                    far = d;
                    ba = i;
                    bb = k;
                }
            }
        }
        pa = ba;
        pb = bb;
    }
    let far = far.sqrt();
    let (vx, vy) = ((x[pb] - x[pa]) / far, (y[pb] - y[pa]) / far);
    let (ax0, ay0, bx0, by0) = (x[pa], y[pa], x[pb], y[pb]);
    for i in pa + 1..pb {
        if hyp(x[i] - ax0, y[i] - ay0) < 0.05 * far && (x[i] - ax0) * vx + (y[i] - ay0) * vy < 0.03 * far {
            pa = i;
        }
    }
    let mut i = pb as i64 - 1;
    while i > pa as i64 {
        let iu = i as usize;
        if hyp(x[iu] - bx0, y[iu] - by0) < 0.05 * far && (bx0 - x[iu]) * vx + (by0 - y[iu]) * vy < 0.03 * far {
            pb = iu;
        }
        i -= 1;
    }
    if far * sc < 50.0 || pa < 3 || (N as i64 - 1 - pb as i64) < 3 {
        return None;
    }
    if dev(x, y, pa, pb) > 0.1 * far {
        return None;
    }
    let h1 = head_at(x, y, 0, pa, pa, vx, vy, far);
    let h2 = head_at(x, y, pb + 1, N, pb, -vx, -vy, far);
    if h1 <= 0.0 || h2 <= 0.0 {
        return None;
    }
    if h1.max(h2) > 3.0 * h1.min(h2) {
        return None;
    }
    let ia = jround(pa as f64 * (M - 1) as f64 / (N - 1) as f64);
    let ib = jround(pb as f64 * (M - 1) as f64 / (N - 1) as f64);
    let sa = ia + (ib - ia) / 10;
    let sb = ib - (ib - ia) / 10;
    let ln = tls(ux, uy, sa, sb, M, false)?;
    let (mut dx, mut dy) = (ln[2], ln[3]);
    if (x[pb] - x[pa]) * dx + (y[pb] - y[pa]) * dy < 0.0 {
        dx = -dx;
        dy = -dy;
    }
    let (mut t0, mut t1) = (1e30f64, -1e30f64);
    for i in 0..M {
        let t = (ux[i] - ln[0]) * dx + (uy[i] - ln[1]) * dy;
        t0 = t0.min(t);
        t1 = t1.max(t);
    }
    Some(G::new(item::ARROW2, snap_line(ln[0] + dx * t0, ln[1] + dy * t0, ln[0] + dx * t1, ln[1] + dy * t1)))
}

/// Prueft, ob die Punkte [from, to) einen Pfeilhaken an der Spitze tip bilden. Liefert die Hakenlaenge oder 0.
#[allow(clippy::too_many_arguments)]
fn head_at(x: &[f64], y: &[f64], from: usize, to: usize, tip: usize, vx: f64, vy: f64, far: f64) -> f64 {
    let (tx, ty) = (x[tip], y[tip]);
    let (mut head, mut hx, mut hy, mut pl) = (0.0, 0.0, 0.0, 0.0);
    for i in from..to {
        let dd = hyp(x[i] - tx, y[i] - ty);
        if dd > 0.42 * far {
            return 0.0;
        }
        if dd > head {
            head = dd;
            hx = x[i] - tx;
            hy = y[i] - ty;
        }
        if i > from {
            pl += hyp(x[i] - x[i - 1], y[i] - y[i - 1]);
        }
    }
    if head < 0.07 * far || head > 0.4 * far || pl > 4.5 * head {
        return 0.0;
    }
    let cs = (hx * vx + hy * vy) / head;
    let ang = clamp(cs, -1.0, 1.0).acos().to_degrees();
    if !(10.0..=68.0).contains(&ang) {
        return 0.0;
    }
    head
}

// ------------------------------------------------------------------ geschlossene Formen

fn closed_shape(x0: &[f64], y0: &[f64], j: usize, ux: &[f64], uy: &[f64], lx: &[f64], ly: &[f64]) -> Option<G> {
    let (mut x, mut y) = resample(x0, y0, j + 1, N, true);
    smooth(&mut x, &mut y, 2, 1, true);
    let (mut min_x, mut min_y, mut max_x, mut max_y) = (1e30f64, 1e30f64, -1e30f64, -1e30f64);
    for i in 0..M {
        min_x = min_x.min(ux[i]);
        max_x = max_x.max(ux[i]);
        min_y = min_y.min(uy[i]);
        max_y = max_y.max(uy[i]);
    }
    let (w, h) = (max_x - min_x, max_y - min_y);
    let dg = hyp(w, h);
    if w < 1e-3 || h < 1e-3 {
        return None;
    }

    // Gesamtdrehung: 360 Grad = einfache Schleife, 720 Grad = Pentagramm
    let (mut gx, mut gy) = (x.clone(), y.clone());
    smooth(&mut gx, &mut gy, 3, 3, true);
    let mut tot = 0.0;
    for i in 0..N {
        let (a, b) = ((i + N - 1) % N, (i + 1) % N);
        tot += turn(gx[a], gy[a], gx[i], gy[i], gx[b], gy[b]);
    }
    let sg = if tot < 0.0 { -1.0 } else { 1.0 };
    let wind = jround(tot.abs() / 360.0);
    let t_b = turns(&x, &y, N / 24, sg);
    if wind == 2 {
        return pentagram(&t_b, ux, uy, dg);
    }
    if wind != 1 {
        return None;
    }

    // Umfang, Flaeche
    let (mut per, mut area) = (0.0, 0.0);
    for i in 0..N {
        let k = (i + 1) % N;
        per += hyp(x[k] - x[i], y[k] - y[i]);
        area += x[i] * y[k] - x[k] * y[i];
    }
    area = area.abs() / 2.0;
    if per <= 0.0 || 4.0 * PI * area / (per * per) < 0.2 {
        return None;
    }

    // Modell 1: Ellipse
    let el = fit_ellipse(ux, uy);
    let e_e = el.as_ref().map_or(1e9, |e| e[5] / dg);

    // Modell 2: konvexe Polygone mit 3..6 Ecken
    let t_a = turns(ux, uy, M / 16, sg);
    let pk = peaks(&t_a, 20.0, M / 24);
    let mut pv: [Option<Vec<[f64; 2]>>; 7] = Default::default();
    let mut e_p = [0.0f64; 7];
    let mut conc = [0.0f64; 7];
    let mut kb: i32 = -1;
    for k in 3..=6usize {
        e_p[k] = 1e9;
        let Some(idx) = pick(&pk, &t_a, k, if k <= 4 { M / 10 } else { M / 13 }) else { continue };
        let mut mt: f64 = 1e9;
        for &ii in &idx {
            mt = mt.min(t_a[ii]);
        }
        if mt < if k <= 4 { 30.0 } else { 26.0 } {
            continue;
        }
        let Some(v) = fit_poly(ux, uy, &idx, M, true) else { continue };
        if !convex(&v, dg) {
            continue;
        }
        let mut q = 0.0;
        for &ii in &idx {
            q += t_a[ii].min(180.0) / 360.0;
        }
        e_p[k] = poly_err(ux, uy, &v) / dg;
        conc[k] = q;
        if kb < 0 {
            kb = k as i32;
        } else {
            let kbu = kb as usize;
            if e_p[k] < e_p[kbu] * 0.7
                && e_p[kbu] - e_p[k] > 0.003
                && min_side(&v) > if k >= 5 { 0.4 } else { 0.25 }
                && (e_p[kbu] > 0.016 || min_side(&v) > 0.6)
                && supported(&v, lx, ly, dg)
            {
                kb = k as i32;
            }
        }
        pv[k] = Some(v);
    }
    // Einbuchtungen?
    let mut cave: f64 = 0.0;
    {
        let ng: Vec<f64> = t_a.iter().map(|v| -v).collect();
        let cv = peaks(&ng, 22.0, M / 24);
        let mut cnt = 0;
        for &i in &cv {
            if i.min(M - i) < M / 10 {
                continue;
            }
            cnt += 1;
            cave = cave.min(t_a[i]);
        }
        if cnt >= 2 {
            cave = cave.min(-100.0);
        }
    }

    // Modell 3: Stern-Umriss
    let star = star_outline(&t_b, ux, uy, dg);
    let e_s = star.as_ref().map_or(1e9, |s| s[6]);
    // Modell 4: Herz
    let heart = heart(&x, &y, &t_b, ux, uy, dg);
    let e_h = heart.as_ref().map_or(1e9, |h| h[5]);

    let e_kb = if kb > 0 { e_p[kb as usize] } else { 1e9 };
    let e_best = e_e.min(e_kb);
    if let Some(s) = &star {
        if e_s < 0.05 && e_s < 0.8 * e_best {
            return Some(G::new(item::POLY, star_pts(s[0], s[1], s[2], s[3], s[4])));
        }
    }
    if let Some(hp) = &heart {
        if e_h < 0.045 && e_h < 0.8 * e_e && e_h < 1.1 * e_kb {
            let q = vec![
                (hp[0] - hp[2] / 2.0) as f32,
                (hp[1] - hp[3] / 2.0) as f32,
                (hp[0] + hp[2] / 2.0) as f32,
                (hp[1] + hp[3] / 2.0) as f32,
            ];
            return Some(G { kind: item::HEART, q, ang: hp[4] as f32 });
        }
    }
    if cave < -40.0 {
        return None;
    }
    let mut k = kb;
    while k >= 3 {
        let ku = k as usize;
        k -= 1;
        let Some(v) = &pv[ku] else { continue };
        if e_p[ku] >= 0.05 {
            continue;
        }
        if (ku as i32) < kb && e_p[ku] > 1.6 * e_p[kb as usize] + 0.004 {
            continue;
        }
        let r = e_p[ku] / e_e.max(1e-9);
        let lim = if ku <= 4 { 0.5 } else if ku == 5 { 0.42 } else { 0.36 };
        let cl = if ku <= 4 { 0.6 } else if ku == 5 { 0.68 } else { 0.75 };
        let mut ok = r <= lim || (r <= 0.8 && conc[ku] >= cl);
        if ku >= 5 && lx.len() < 5 * ku {
            ok = r <= 0.35 || (r <= 0.8 && raw_conc(lx, ly, ku, dg) >= 0.72);
        }
        if !ok {
            continue;
        }
        let q = match ku {
            3 => triangle(v),
            4 => quad(v, ux, uy),
            _ => regular(v),
        };
        return Some(G::new(item::POLY, q));
    }
    if let Some(e) = el {
        if e_e < 0.05 {
            let q = vec![(e[0] - e[2]) as f32, (e[1] - e[3]) as f32, (e[0] + e[2]) as f32, (e[1] + e[3]) as f32];
            return Some(G { kind: item::ELLIPSE, q, ang: e[4] as f32 });
        }
    }
    None
}

// ------------------------------------------------------------------ Ellipse

/// Ellipse frei gedreht: [cx, cy, a, b, ang, rms].
fn fit_ellipse(x: &[f64], y: &[f64]) -> Option<[f64; 6]> {
    let n = x.len();
    let nf = n as f64;
    let (mut mx, mut my) = (0.0, 0.0);
    for i in 0..n {
        mx += x[i] / nf;
        my += y[i] / nf;
    }
    let (mut sxx, mut syy, mut sxy) = (0.0, 0.0, 0.0);
    for i in 0..n {
        let (dx, dy) = (x[i] - mx, y[i] - my);
        sxx += dx * dx;
        syy += dy * dy;
        sxy += dx * dy;
    }
    let th = 0.5 * (2.0 * sxy).atan2(sxx - syy);
    let (c, s) = (th.cos(), th.sin());
    let (mut u0, mut u1, mut v0, mut v1) = (1e30f64, -1e30f64, 1e30f64, -1e30f64);
    for i in 0..n {
        let (dx, dy) = (x[i] - mx, y[i] - my);
        let (u, v) = (dx * c + dy * s, -dx * s + dy * c);
        u0 = u0.min(u);
        u1 = u1.max(u);
        v0 = v0.min(v);
        v1 = v1.max(v);
    }
    let mut p = [
        mx + c * (u0 + u1) / 2.0 - s * (v0 + v1) / 2.0,
        my + s * (u0 + u1) / 2.0 + c * (v0 + v1) / 2.0,
        ((u1 - u0) / 2.0).max(1e-3),
        ((v1 - v0) / 2.0).max(1e-3),
        th,
    ];
    let e = ell_fit(x, y, &mut p, false, 5);
    if e < 0.0 {
        return None;
    }
    if p[3] > p[2] {
        p.swap(2, 3);
        p[4] += PI / 2.0;
    }
    while p[4] > PI / 2.0 {
        p[4] -= PI;
    }
    while p[4] <= -PI / 2.0 {
        p[4] += PI;
    }
    if p[2] / p[3] < 1.12 {
        let mut cc = [p[0], p[1], (p[2] * p[3]).sqrt()];
        let ec = circ_fit(x, y, &mut cc, 5);
        if ec >= 0.0 && ec < e * 1.25 + 1e-9 {
            return Some([cc[0], cc[1], cc[2], cc[2], 0.0, ec]);
        }
    }
    let deg = p[4].to_degrees();
    if deg.abs() < 10.0 || deg.abs() > 80.0 {
        let mut q = p;
        let vert = deg.abs() > 80.0;
        q[4] = if vert { PI / 2.0 } else { 0.0 };
        let e2 = ell_fit(x, y, &mut q, true, 3);
        if e2 >= 0.0 {
            if vert {
                return Some([q[0], q[1], q[3], q[2], 0.0, e2]);
            }
            return Some([q[0], q[1], q[2], q[3], 0.0, e2]);
        }
    }
    Some([p[0], p[1], p[2], p[3], p[4], e])
}

/// Gauss-Newton mit orthogonalen Abstaenden; p = [cx, cy, a, b, th]. Liefert rms-Abstand oder -1.
fn ell_fit(x: &[f64], y: &[f64], p: &mut [f64; 5], fix_th: bool, iters: usize) -> f64 {
    let n = x.len();
    let np = if fix_th { 4 } else { 5 };
    let mut jx = [0.0f64; 5];
    let mut jy = [0.0f64; 5];
    for it in 0..=iters {
        let (c, s, a, b) = (p[4].cos(), p[4].sin(), p[2], p[3]);
        let mut am = vec![vec![0.0f64; np + 1]; np];
        let mut ss = 0.0;
        for i in 0..n {
            let (dx, dy) = (x[i] - p[0], y[i] - p[1]);
            let (u, v) = (dx * c + dy * s, -dx * s + dy * c);
            let mut t = (a * v).atan2(b * u);
            for _ in 0..2 {
                let (ct, st) = (t.cos(), t.sin());
                let f = (a * a - b * b) * st * ct - u * a * st + v * b * ct;
                let fp = (a * a - b * b) * (ct * ct - st * st) - u * a * ct - v * b * st;
                if fp.abs() < 1e-12 {
                    break;
                }
                t -= clamp(f / fp, -0.5, 0.5);
            }
            let (ct, st) = (t.cos(), t.sin());
            let rxx = x[i] - (p[0] + a * c * ct - b * s * st);
            let ryy = y[i] - (p[1] + a * s * ct + b * c * st);
            ss += rxx * rxx + ryy * ryy;
            if it == iters {
                continue;
            }
            jx = [1.0, 0.0, c * ct, -s * st, -a * s * ct - b * c * st];
            jy = [0.0, 1.0, s * ct, c * st, a * c * ct - b * s * st];
            for r in 0..np {
                for q in 0..np {
                    am[r][q] += jx[r] * jx[q] + jy[r] * jy[q];
                }
                am[r][np] += jx[r] * rxx + jy[r] * ryy;
            }
        }
        if it == iters {
            return (ss / n as f64).sqrt();
        }
        for r in 0..np {
            am[r][r] = am[r][r] * 1.001 + 1e-9;
        }
        let Some(d) = solve(am) else { return -1.0 };
        for r in 0..np {
            p[r] += d[r];
        }
        p[2] = p[2].abs();
        p[3] = p[3].abs();
        if p[2] < 1e-3 || p[3] < 1e-3 {
            return -1.0;
        }
    }
    let _ = (jx, jy);
    -1.0
}

/// Geometrische Kreispassung (Gauss-Newton); p = [cx, cy, r]. Liefert rms-Abstand oder -1.
fn circ_fit(x: &[f64], y: &[f64], p: &mut [f64; 3], iters: usize) -> f64 {
    let n = x.len();
    for it in 0..=iters {
        let mut am = vec![vec![0.0f64; 4]; 3];
        let mut ss = 0.0;
        for i in 0..n {
            let (dx, dy) = (x[i] - p[0], y[i] - p[1]);
            let d = hyp(dx, dy);
            if d < 1e-9 {
                continue;
            }
            let rr = d - p[2];
            ss += rr * rr;
            let jj = [dx / d, dy / d, 1.0];
            for r in 0..3 {
                for q in 0..3 {
                    am[r][q] += jj[r] * jj[q];
                }
                am[r][3] += jj[r] * rr;
            }
        }
        if it == iters {
            return (ss / n as f64).sqrt();
        }
        let Some(d) = solve(am) else { return -1.0 };
        for r in 0..3 {
            p[r] += d[r];
        }
        if p[2] < 1e-3 {
            return -1.0;
        }
    }
    -1.0
}

// ------------------------------------------------------------------ Polygone

/// Vorzeichenbehafteter Abbiegewinkel (Grad) je Punkt ueber ein Fenster von w Punkten; sg = Umlaufsinn.
fn turns(x: &[f64], y: &[f64], w: usize, sg: f64) -> Vec<f64> {
    let n = x.len();
    (0..n)
        .map(|i| {
            let a = (i + n * 4 - w) % n;
            let b = (i + w) % n;
            sg * turn(x[a], y[a], x[i], y[i], x[b], y[b])
        })
        .collect()
}

fn turn(ax: f64, ay: f64, bx: f64, by: f64, cx: f64, cy: f64) -> f64 {
    let (ux, uy, vx, vy) = (bx - ax, by - ay, cx - bx, cy - by);
    (ux * vy - uy * vx).atan2(ux * vx + uy * vy).to_degrees()
}

/// Lokale Maxima von t (>= thr) im Umkreis von sep Punkten, nach Staerke absteigend.
fn peaks(t: &[f64], thr: f64, sep: usize) -> Vec<usize> {
    let n = t.len() as i64;
    let sep = sep as i64;
    let mut o: Vec<usize> = Vec::new();
    for i in 0..n {
        let ti = t[i as usize];
        if ti < thr {
            continue;
        }
        let mut top = true;
        let mut d = -sep;
        while d <= sep && top {
            let q = ((i + d + n) % n) as usize;
            if d != 0 && (t[q] > ti || (t[q] == ti && d < 0)) {
                top = false;
            }
            d += 1;
        }
        if top {
            o.push(i as usize);
        }
    }
    // Einfuegesortierung (stabil, wie Java)
    for i in 1..o.len() {
        let v = o[i];
        let mut k = i as i64 - 1;
        while k >= 0 && t[o[k as usize]] < t[v] {
            o[k as usize + 1] = o[k as usize];
            k -= 1;
        }
        o[(k + 1) as usize] = v;
    }
    o
}

/// Die k staerksten Spitzen mit Mindestabstand sep, aufsteigend nach Index; None, wenn es zu wenige gibt.
fn pick(pk: &[usize], t: &[f64], k: usize, sep: usize) -> Option<Vec<usize>> {
    let n = t.len();
    let mut idx: Vec<usize> = Vec::with_capacity(k);
    for &ci in pk {
        if idx.len() >= k {
            break;
        }
        let ok = idx.iter().all(|&i| {
            let dd = i.abs_diff(ci);
            dd.min(n - dd) >= sep
        });
        if ok {
            idx.push(ci);
        }
    }
    if idx.len() < k {
        return None;
    }
    idx.sort_unstable();
    Some(idx)
}

/// Polygon durch die Ecken idx: Seiten = Ausgleichsgeraden der ungeglaetteten Punkte, Ecken = Schnittpunkte.
fn fit_poly(ux: &[f64], uy: &[f64], idx: &[usize], base: usize, refine: bool) -> Option<Vec<[f64; 2]>> {
    let (k, n) = (idx.len(), ux.len());
    let nf = n as f64;
    let mut ln: Vec<[f64; 4]> = Vec::with_capacity(k);
    for i in 0..k {
        let fa = idx[i] as f64 * nf / base as f64;
        let mut fb = idx[(i + 1) % k] as f64 * nf / base as f64;
        if fb <= fa {
            fb += nf;
        }
        let mut from = jround(fa + (fb - fa) * 0.2);
        let mut to = jround(fb - (fb - fa) * 0.2);
        if to - from < 2 {
            from = (fa + (fb - fa) * 0.1).floor() as i64;
            to = (fb - (fb - fa) * 0.1).ceil() as i64;
        }
        ln.push(tls(ux, uy, from, to, n, true)?);
    }
    let mut v = corners_of(&ln, ux, uy, idx, n, base);
    if !refine {
        return Some(v);
    }
    for _ in 0..2 {
        let mut acc = vec![[0.0f64; 6]; k];
        for i in 0..n {
            let mut bs: usize = 0;
            let (mut bd, mut bt) = (1e30f64, 0.0f64);
            for s in 0..k {
                let (a, b) = (v[s], v[(s + 1) % k]);
                let (dx, dy) = (b[0] - a[0], b[1] - a[1]);
                let l2 = dx * dx + dy * dy;
                let t = if l2 == 0.0 { 0.0 } else { ((ux[i] - a[0]) * dx + (uy[i] - a[1]) * dy) / l2 };
                let tc = clamp(t, 0.0, 1.0);
                let d = hyp(ux[i] - a[0] - tc * dx, uy[i] - a[1] - tc * dy);
                if d < bd {
                    bd = d;
                    bs = s;
                    bt = t;
                }
            }
            if !(0.1..=0.9).contains(&bt) {
                continue;
            }
            let q = &mut acc[bs];
            q[0] += 1.0;
            q[1] += ux[i];
            q[2] += uy[i];
            q[3] += ux[i] * ux[i];
            q[4] += uy[i] * uy[i];
            q[5] += ux[i] * uy[i];
        }
        for s in 0..k {
            let q = acc[s];
            if q[0] < 3.0 {
                continue;
            }
            let (mx, my) = (q[1] / q[0], q[2] / q[0]);
            let sxx = q[3] / q[0] - mx * mx;
            let syy = q[4] / q[0] - my * my;
            let sxy = q[5] / q[0] - mx * my;
            let th = 0.5 * (2.0 * sxy).atan2(sxx - syy);
            ln[s] = [mx, my, th.cos(), th.sin()];
        }
        v = corners_of(&ln, ux, uy, idx, n, base);
    }
    Some(v)
}

/// Ecke i = Schnitt der Seitengeraden i-1 und i; bei fast parallelen Geraden der Eckpunkt des Strichs.
fn corners_of(ln: &[[f64; 4]], ux: &[f64], uy: &[f64], idx: &[usize], n: usize, base: usize) -> Vec<[f64; 2]> {
    let k = ln.len();
    let mut v = vec![[0.0f64; 2]; k];
    for i in 0..k {
        let (l1, l2) = (ln[(i + k - 1) % k], ln[i]);
        let pi = (jround(idx[i] as f64 * n as f64 / base as f64) as usize) % n;
        let (mut vx, mut vy) = (ux[pi], uy[pi]);
        let cross = l1[2] * l2[3] - l1[3] * l2[2];
        if cross.abs() > 0.17 {
            let t = ((l2[0] - l1[0]) * l2[3] - (l2[1] - l1[1]) * l2[2]) / cross;
            vx = l1[0] + t * l1[2];
            vy = l1[1] + t * l1[3];
        } else {
            let t1 = (vx - l1[0]) * l1[2] + (vy - l1[1]) * l1[3];
            let t2 = (vx - l2[0]) * l2[2] + (vy - l2[1]) * l2[3];
            vx = (l1[0] + t1 * l1[2] + l2[0] + t2 * l2[2]) / 2.0;
            vy = (l1[1] + t1 * l1[3] + l2[1] + t2 * l2[3]) / 2.0;
        }
        v[i] = [vx, vy];
    }
    v
}

/// Konvexes, einfaches Polygon mit deutlichen Ecken und ohne winzige Seiten?
fn convex(v: &[[f64; 2]], dg: f64) -> bool {
    let k = v.len();
    let mut sign = 0;
    let mut sum = 0.0;
    for i in 0..k {
        let (a, b, c) = (v[(i + k - 1) % k], v[i], v[(i + 1) % k]);
        let lu = hyp(b[0] - a[0], b[1] - a[1]);
        let lw = hyp(c[0] - b[0], c[1] - b[1]);
        if lu < 0.07 * dg || lw < 0.07 * dg {
            return false;
        }
        let tn = turn(a[0], a[1], b[0], b[1], c[0], c[1]);
        let sg = if tn > 0.0 { 1 } else { -1 };
        if sign == 0 {
            sign = sg;
        } else if sg != sign {
            return false;
        }
        if tn.abs() < 18.0 {
            return false;
        }
        sum += tn.abs();
    }
    (sum - 360.0).abs() < 30.0
}

/// Jede kurze Seite muss von Rohpunkten getragen sein (mind. 2 Punkte im Inneren).
fn supported(v: &[[f64; 2]], x: &[f64], y: &[f64], dg: f64) -> bool {
    let (k, n) = (v.len(), x.len());
    let mut ml = 0.0;
    for s in 0..k {
        ml += hyp(v[(s + 1) % k][0] - v[s][0], v[(s + 1) % k][1] - v[s][1]) / k as f64;
    }
    for s in 0..k {
        let (a, b) = (v[s], v[(s + 1) % k]);
        let (dx, dy) = (b[0] - a[0], b[1] - a[1]);
        let l2 = dx * dx + dy * dy;
        if l2 <= 0.0 {
            return false;
        }
        if l2.sqrt() > 0.6 * ml {
            continue;
        }
        let mut c = 0;
        let mut i = 0;
        while i < n && c < 2 {
            let t = ((x[i] - a[0]) * dx + (y[i] - a[1]) * dy) / l2;
            if (0.15..=0.85).contains(&t) && ((x[i] - a[0]) * dy - (y[i] - a[1]) * dx).abs() / l2.sqrt() < 0.04 * dg {
                c += 1;
            }
            i += 1;
        }
        if c < 2 {
            return false;
        }
    }
    true
}

/// Bei wenigen Rohpunkten: Anteil der k staerksten Knicke an der Gesamtdrehung.
fn raw_conc(x: &[f64], y: &[f64], k: usize, dg: f64) -> f64 {
    let mut px: Vec<f64> = Vec::with_capacity(x.len());
    let mut py: Vec<f64> = Vec::with_capacity(x.len());
    for i in 0..x.len() {
        if let (Some(&lx), Some(&ly)) = (px.last(), py.last()) {
            if hyp(x[i] - lx, y[i] - ly) < 0.02 * dg {
                continue;
            }
        }
        px.push(x[i]);
        py.push(y[i]);
    }
    let mut m = px.len();
    if m > 2 && hyp(px[m - 1] - px[0], py[m - 1] - py[0]) < 0.02 * dg {
        m -= 1;
    }
    if m < k {
        return 0.0;
    }
    let mut t = vec![0.0f64; m];
    let mut tot = 0.0;
    for i in 0..m {
        let (a, b) = ((i + m - 1) % m, (i + 1) % m);
        t[i] = turn(px[a], py[a], px[i], py[i], px[b], py[b]).abs();
        tot += t[i];
    }
    t.sort_by(|a, b| a.total_cmp(b));
    let top: f64 = (0..k).map(|i| t[m - 1 - i]).sum();
    top / tot.max(360.0)
}

/// Kuerzeste Seite relativ zur mittleren Seitenlaenge.
fn min_side(v: &[[f64; 2]]) -> f64 {
    let k = v.len();
    let (mut mn, mut sum) = (1e30f64, 0.0);
    for i in 0..k {
        let l = hyp(v[(i + 1) % k][0] - v[i][0], v[(i + 1) % k][1] - v[i][1]);
        mn = mn.min(l);
        sum += l;
    }
    mn / (sum / k as f64)
}

/// rms-Abstand der Punkte zum geschlossenen Polygon v.
fn poly_err(x: &[f64], y: &[f64], v: &[[f64; 2]]) -> f64 {
    let (k, n) = (v.len(), x.len());
    let mut sum = 0.0;
    for i in 0..n {
        let mut best: f64 = 1e30;
        for j in 0..k {
            let (a, b) = (v[j], v[(j + 1) % k]);
            best = best.min(seg_d(x[i], y[i], a[0], a[1], b[0], b[1]));
        }
        sum += best * best;
    }
    (sum / n as f64).sqrt()
}

fn to_q(v: &[[f64; 2]]) -> Vec<f32> {
    v.iter().flat_map(|p| [p[0] as f32, p[1] as f32]).collect()
}

/// Polygon um seinen Schwerpunkt drehen, wenn eine laengere Seite fast waagrecht/senkrecht ist (< 7 Grad).
fn axis_snap(v: &[[f64; 2]]) -> Vec<[f64; 2]> {
    if !cfg().shapes {
        return v.to_vec();
    }
    let k = v.len();
    let kf = k as f64;
    let (mut cx, mut cy, mut best, mut bl) = (0.0, 0.0, 1e9f64, 0.0);
    for p in v {
        cx += p[0] / kf;
        cy += p[1] / kf;
    }
    let mut max_l: f64 = 0.0;
    for i in 0..k {
        max_l = max_l.max(hyp(v[(i + 1) % k][0] - v[i][0], v[(i + 1) % k][1] - v[i][1]));
    }
    for i in 0..k {
        let (dx, dy) = (v[(i + 1) % k][0] - v[i][0], v[(i + 1) % k][1] - v[i][1]);
        let l = hyp(dx, dy);
        if l < 0.5 * max_l {
            continue;
        }
        let a = dy.atan2(dx).to_degrees();
        let d = a - jround(a / 90.0) as f64 * 90.0;
        if d.abs() < 7.0 && (d.abs() < best.abs() || (d.abs() == best.abs() && l > bl)) {
            best = d;
            bl = l;
        }
    }
    if best > 1e8 {
        return v.to_vec();
    }
    let r = -best.to_radians();
    let (cs, sn) = (r.cos(), r.sin());
    v.iter()
        .map(|p| {
            let (dx, dy) = (p[0] - cx, p[1] - cy);
            [cx + dx * cs - dy * sn, cy + dx * sn + dy * cs]
        })
        .collect()
}

fn triangle(v: &[[f64; 2]]) -> Vec<f32> {
    let mut eq = true;
    for i in 0..3 {
        let (a, b, c) = (v[(i + 2) % 3], v[i], v[(i + 1) % 3]);
        let ang = 180.0 - turn(a[0], a[1], b[0], b[1], c[0], c[1]).abs();
        if (ang - 60.0).abs() > 7.0 {
            eq = false;
        }
    }
    if eq {
        return regular(v);
    }
    to_q(&axis_snap(v))
}

/// Regelmaessiges k-Eck, wenn das Polygon nahezu regelmaessig ist, sonst frei.
fn regular(v: &[[f64; 2]]) -> Vec<f32> {
    let k = v.len();
    let kf = k as f64;
    let (mut cx, mut cy, mut ml) = (0.0, 0.0, 0.0);
    for i in 0..k {
        cx += v[i][0] / kf;
        cy += v[i][1] / kf;
        ml += hyp(v[(i + 1) % k][0] - v[i][0], v[(i + 1) % k][1] - v[i][1]) / kf;
    }
    let ideal = 360.0 / kf;
    let mut reg = true;
    for i in 0..k {
        let l = hyp(v[(i + 1) % k][0] - v[i][0], v[(i + 1) % k][1] - v[i][1]);
        if (l / ml - 1.0).abs() > 0.2 {
            reg = false;
        }
        let (a, b, c) = (v[(i + k - 1) % k], v[i], v[(i + 1) % k]);
        if (turn(a[0], a[1], b[0], b[1], c[0], c[1]).abs() - ideal).abs() > 14.0 {
            reg = false;
        }
    }
    if !reg {
        return to_q(&axis_snap(v));
    }
    let (mut rad, mut sc, mut ss) = (0.0, 0.0, 0.0);
    for p in v {
        let (dx, dy) = (p[0] - cx, p[1] - cy);
        rad += hyp(dx, dy) / kf;
        let a = dy.atan2(dx) * kf;
        sc += a.cos();
        ss += a.sin();
    }
    let phi = snap_phase(ss.atan2(sc) / kf, k);
    let d0 = (v[0][1] - cy).atan2(v[0][0] - cx);
    let ccw = turn(v[0][0], v[0][1], v[1][0], v[1][1], v[2][0], v[2][1]) < 0.0;
    let st = 2.0 * PI / kf;
    let i0 = jround((d0 - phi) / st) as f64;
    let mut q = Vec::with_capacity(k * 2);
    for i in 0..k {
        let fi = i as f64;
        let a = phi + st * (i0 + if ccw { -fi } else { fi });
        q.push((cx + rad * a.cos()) as f32);
        q.push((cy + rad * a.sin()) as f32);
    }
    q
}

/// Drehung eines k-fach symmetrischen Koerpers auf "Spitze oben" bzw. "Seite oben" ausrichten (< 7 Grad).
fn snap_phase(phi: f64, k: usize) -> f64 {
    if !cfg().shapes {
        return phi;
    }
    let st = 2.0 * PI / k as f64;
    for cand in [-PI / 2.0, -PI / 2.0 + st / 2.0] {
        let mut d = phi - cand;
        d -= jround(d / st) as f64 * st;
        if d.abs() < 7f64.to_radians() {
            return phi - d;
        }
    }
    phi
}

fn quad(v: &[[f64; 2]], ux: &[f64], uy: &[f64]) -> Vec<f32> {
    let mut rect = true;
    let (mut sc, mut ss) = (0.0, 0.0);
    for i in 0..4 {
        let (a, b, c) = (v[i], v[(i + 1) % 4], v[(i + 2) % 4]);
        let (px, py, wx, wy) = (b[0] - a[0], b[1] - a[1], c[0] - b[0], c[1] - b[1]);
        let (lu, lw) = (hyp(px, py), hyp(wx, wy));
        if (px * wx + py * wy).abs() / (lu * lw) > 0.34 {
            rect = false;
        }
        let ang = py.atan2(px);
        sc += lu * (4.0 * ang).cos();
        ss += lu * (4.0 * ang).sin();
        if i < 2 {
            let lo = hyp(v[(i + 3) % 4][0] - v[(i + 2) % 4][0], v[(i + 3) % 4][1] - v[(i + 2) % 4][1]);
            if lu.min(lo) / lu.max(lo) < 0.6 {
                rect = false;
            }
        }
    }
    if rect {
        let mut th = (ss.atan2(sc) / 4.0).to_degrees();
        if th.abs() <= 16.0 {
            th = 0.0;
        } else if (th.abs() - 45.0).abs() <= 6.0 {
            th = if th > 0.0 { 45.0 } else { -45.0 };
        }
        return rect_fit(v, ux, uy, th.to_radians());
    }
    // Raute
    let mut dia = true;
    let (mut l, mut t, mut r, mut b) = (1e30f64, 1e30f64, -1e30f64, -1e30f64);
    for i in 0..4 {
        let (a, c) = (v[i], v[(i + 1) % 4]);
        let ang = (c[1] - a[1]).atan2(c[0] - a[0]).to_degrees().abs() % 90.0;
        if ang.min(90.0 - ang) < 22.0 {
            dia = false;
        }
        l = l.min(a[0]);
        r = r.max(a[0]);
        t = t.min(a[1]);
        b = b.max(a[1]);
    }
    if dia {
        let (mx, my) = (((l + r) / 2.0) as f32, ((t + b) / 2.0) as f32);
        return vec![mx, t as f32, r as f32, my, mx, b as f32, l as f32, my];
    }
    to_q(&axis_snap(v))
}

/// Rechteck mit fester Drehung th an die ungeglaetteten Punkte anpassen. Fast quadratisch -> Quadrat.
fn rect_fit(v: &[[f64; 2]], ux: &[f64], uy: &[f64], th: f64) -> Vec<f32> {
    let (mut cx, mut cy) = (0.0, 0.0);
    for p in v.iter().take(4) {
        cx += p[0] / 4.0;
        cy += p[1] / 4.0;
    }
    let (c, s) = (th.cos(), th.sin());
    let mut us = [0.0f64; 4];
    let mut vs = [0.0f64; 4];
    for i in 0..4 {
        us[i] = (v[i][0] - cx) * c + (v[i][1] - cy) * s;
        vs[i] = -(v[i][0] - cx) * s + (v[i][1] - cy) * c;
    }
    us.sort_by(|a, b| a.total_cmp(b));
    vs.sort_by(|a, b| a.total_cmp(b));
    let (mut u0, mut u1, mut v0, mut v1) = ((us[0] + us[1]) / 2.0, (us[2] + us[3]) / 2.0, (vs[0] + vs[1]) / 2.0, (vs[2] + vs[3]) / 2.0);
    let n = ux.len();
    for _ in 0..3 {
        let mut sum = [0.0f64; 4];
        let mut cnt = [0.0f64; 4];
        let (mu, mv) = ((u1 - u0) * 0.12, (v1 - v0) * 0.12);
        for i in 0..n {
            let pu = (ux[i] - cx) * c + (uy[i] - cy) * s;
            let pv = -(ux[i] - cx) * s + (uy[i] - cy) * c;
            let (d0, d1, d2, d3) = ((pu - u0).abs(), (pu - u1).abs(), (pv - v0).abs(), (pv - v1).abs());
            let md = d0.min(d1).min(d2.min(d3));
            if md == d0 || md == d1 {
                if pv < v0 + mv || pv > v1 - mv {
                    continue;
                }
                let k = if md == d0 { 0 } else { 1 };
                sum[k] += pu;
                cnt[k] += 1.0;
            } else {
                if pu < u0 + mu || pu > u1 - mu {
                    continue;
                }
                let k = if md == d2 { 2 } else { 3 };
                sum[k] += pv;
                cnt[k] += 1.0;
            }
        }
        if cnt[0] >= 2.0 {
            u0 = sum[0] / cnt[0];
        }
        if cnt[1] >= 2.0 {
            u1 = sum[1] / cnt[1];
        }
        if cnt[2] >= 2.0 {
            v0 = sum[2] / cnt[2];
        }
        if cnt[3] >= 2.0 {
            v1 = sum[3] / cnt[3];
        }
    }
    let (w, h) = (u1 - u0, v1 - v0);
    if (w - h).abs() / w.max(h) < 0.09 {
        let (sd, mu, mv) = ((w + h) / 2.0, (u0 + u1) / 2.0, (v0 + v1) / 2.0);
        u0 = mu - sd / 2.0;
        u1 = mu + sd / 2.0;
        v0 = mv - sd / 2.0;
        v1 = mv + sd / 2.0;
    }
    let uv = [[u0, v0], [u1, v0], [u1, v1], [u0, v1]];
    let mut q = Vec::with_capacity(8);
    for p in uv {
        q.push((cx + p[0] * c - p[1] * s) as f32);
        q.push((cy + p[0] * s + p[1] * c) as f32);
    }
    q
}

// ------------------------------------------------------------------ Stern

/// Fuenfzackiger Stern als Umriss: 10 Punkte, Spitzen bei Radius big_r, Kerben bei r, eine Spitze bei Winkel phi.
fn star_pts(cx: f64, cy: f64, big_r: f64, r: f64, phi: f64) -> Vec<f32> {
    let mut q = Vec::with_capacity(20);
    for i in 0..10 {
        let a = phi + PI * i as f64 / 5.0;
        let rr = if i % 2 == 0 { big_r } else { r };
        q.push((cx + rr * a.cos()) as f32);
        q.push((cy + rr * a.sin()) as f32);
    }
    q
}

/// Spitzen (5 Punkte) pruefen: gleicher Radius, gleichmaessig verteilt; liefert [cx, cy, R, phi].
fn tips(t: &[[f64; 2]], step: i32) -> Option<[f64; 4]> {
    let (mut cx, mut cy) = (0.0, 0.0);
    for p in t.iter().take(5) {
        cx += p[0] / 5.0;
        cy += p[1] / 5.0;
    }
    let (mut big_r, mut sc, mut ss) = (0.0, 0.0, 0.0);
    let mut rr = [0.0f64; 5];
    let mut an = [0.0f64; 5];
    for i in 0..5 {
        rr[i] = hyp(t[i][0] - cx, t[i][1] - cy);
        an[i] = (t[i][1] - cy).atan2(t[i][0] - cx);
        big_r += rr[i] / 5.0;
        sc += (5.0 * an[i]).cos();
        ss += (5.0 * an[i]).sin();
    }
    if big_r <= 0.0 {
        return None;
    }
    let mut dir = 0;
    for i in 0..5 {
        if (rr[i] / big_r - 1.0).abs() > 0.3 {
            return None;
        }
        let mut d = (an[(i + 1) % 5] - an[i]).to_degrees();
        while d > 180.0 {
            d -= 360.0;
        }
        while d <= -180.0 {
            d += 360.0;
        }
        let sg = if d > 0.0 { 1 } else { -1 };
        if dir == 0 {
            dir = sg;
        } else if sg != dir {
            return None;
        }
        if (d.abs() - 72.0 * step as f64).abs() > 24.0 {
            return None;
        }
    }
    let phi = snap_phase(ss.atan2(sc) / 5.0, 5);
    Some([cx, cy, big_r, phi])
}

/// Pentagramm in einem Strich (5 Geraden, Spitze zu Spitze, Gesamtdrehung 720 Grad).
fn pentagram(t_b: &[f64], ux: &[f64], uy: &[f64], dg: f64) -> Option<G> {
    let pk = peaks(t_b, 70.0, N / 12);
    let idx = pick(&pk, t_b, 5, N / 9)?;
    let mut mt = 0.0;
    for &i in &idx {
        if t_b[i] < 65.0 {
            return None;
        }
        mt += t_b[i] / 5.0;
    }
    if mt < 100.0 {
        return None;
    }
    if pk.len() > 5 && t_b[pk[5]] > 0.6 * t_b[idx[0]] {
        let c = pk.iter().filter(|&&q| t_b[q] > 95.0).count();
        if c > 5 {
            return None;
        }
    }
    let v = fit_poly(ux, uy, &idx, N, false)?;
    let tp = tips(&v, 2)?;
    if poly_err(ux, uy, &v) / dg > 0.05 {
        return None;
    }
    Some(G::new(item::POLY, star_pts(tp[0], tp[1], tp[2], tp[2] * 0.381966, tp[3])))
}

/// Stern-Umriss: 5 starke aeussere und 5 innere Ecken im Wechsel. [cx, cy, R, r, phi, -, rms/dg].
fn star_outline(t_b: &[f64], ux: &[f64], uy: &[f64], dg: f64) -> Option<[f64; 7]> {
    let n = t_b.len();
    let po = peaks(t_b, 55.0, N / 20);
    if po.len() < 5 {
        return None;
    }
    let o = pick(&po, t_b, 5, N / 14)?;
    let mut ts = 0.0;
    for &i in &o {
        ts += t_b[i] / 5.0;
    }
    if ts < 85.0 {
        return None;
    }
    for &c in &po {
        if !o.contains(&c) && t_b[c] > 0.7 * ts {
            return None;
        }
    }
    // Kerben = staerkste Einbuchtung zwischen zwei Zacken
    let mut all = [0usize; 10];
    let mut cv = 0.0;
    for i in 0..5 {
        let a = o[i];
        let mut b = o[(i + 1) % 5];
        if b <= a {
            b += n;
        }
        let mut bi: Option<usize> = None;
        let mut t = a + 2;
        while t + 2 <= b {
            if bi.is_none_or(|bv| t_b[t % n] < t_b[bv % n]) {
                bi = Some(t);
            }
            t += 1;
        }
        let bi = bi?;
        if t_b[bi % n] > -5.0 {
            return None;
        }
        cv += t_b[bi % n] / 5.0;
        all[i * 2] = a;
        all[i * 2 + 1] = bi % n;
    }
    if cv > -25.0 {
        return None;
    }
    all.sort_unstable();
    let first = o.contains(&all[0]);
    let v = fit_poly(ux, uy, &all, N, true)?;
    let mut t = [[0.0f64; 2]; 5];
    let mut ins = [[0.0f64; 2]; 5];
    for i in 0..5 {
        t[i] = v[if first { i * 2 } else { i * 2 + 1 }];
        ins[i] = v[if first { i * 2 + 1 } else { i * 2 }];
    }
    let tp = tips(&t, 1)?;
    let mut r = 0.0;
    for p in &ins {
        r += hyp(p[0] - tp[0], p[1] - tp[1]) / 5.0;
    }
    for p in &ins {
        if (hyp(p[0] - tp[0], p[1] - tp[1]) / r - 1.0).abs() > 0.35 {
            return None;
        }
    }
    let ratio = r / tp[2];
    if !(0.2..=0.65).contains(&ratio) {
        return None;
    }
    let sq = star_pts(tp[0], tp[1], tp[2], r, tp[3]);
    let sv: Vec<[f64; 2]> = (0..10).map(|i| [sq[i * 2] as f64, sq[i * 2 + 1] as f64]).collect();
    Some([tp[0], tp[1], tp[2], r, tp[3], 0.0, poly_err(ux, uy, &sv) / dg])
}

// ------------------------------------------------------------------ Herz

/// Herz: eine tiefe Kerbe und gegenueber eine Spitze. [cx, cy, w, h, ang, rms/dg].
fn heart(x: &[f64], y: &[f64], t_b: &[f64], ux: &[f64], uy: &[f64], dg: f64) -> Option<[f64; 6]> {
    let n = t_b.len();
    let mut ic = 0;
    for i in 1..n {
        if t_b[i] < t_b[ic] {
            ic = i;
        }
    }
    if t_b[ic] > -45.0 {
        return None;
    }
    // keine zweite deutliche Einbuchtung
    for i in 0..n {
        let d = i.abs_diff(ic);
        if d.min(n - d) > n / 10 && t_b[i] < (-45f64).min(0.45 * t_b[ic]) {
            return None;
        }
    }
    let mut it: Option<usize> = None;
    let (d0, d1) = ((n as f64 * 0.35) as usize, (n as f64 * 0.65) as usize);
    for d in d0..=d1 {
        let i = (ic + d) % n;
        if it.is_none_or(|iv| t_b[i] > t_b[iv]) {
            it = Some(i);
        }
    }
    let it = it?;
    if t_b[it] < 30.0 {
        return None;
    }
    let (ax, ay) = (x[it] - x[ic], y[it] - y[ic]);
    if hyp(ax, ay) < 0.25 * dg {
        return None;
    }
    let mut ang = ay.atan2(ax) - PI / 2.0;
    while ang > PI {
        ang -= 2.0 * PI;
    }
    while ang <= -PI {
        ang += 2.0 * PI;
    }
    if ang.abs() < 12f64.to_radians() {
        ang = 0.0;
    }
    let (c, s) = (ang.cos(), ang.sin());
    let (mut u0, mut u1, mut v0, mut v1) = (1e30f64, -1e30f64, 1e30f64, -1e30f64);
    for i in 0..ux.len() {
        let pu = ux[i] * c + uy[i] * s;
        let pv = -ux[i] * s + uy[i] * c;
        u0 = u0.min(pu);
        u1 = u1.max(pu);
        v0 = v0.min(pv);
        v1 = v1.max(pv);
    }
    let (w, h) = (u1 - u0, v1 - v0);
    if w < 0.25 * h || h < 0.25 * w {
        return None;
    }
    let (mu, mv) = ((u0 + u1) / 2.0, (v0 + v1) / 2.0);
    let mut p = [mu * c - mv * s, mu * s + mv * c, w, h, ang, 0.0];
    p[5] = heart_err(ux, uy, &p, 3);
    // Feinanpassung (Koordinatensuche ueber Mittelpunkt, Groesse und Drehung)
    let mut step = [0.02 * w, 0.02 * h, 0.03 * w, 0.03 * h, 0.06];
    for _ in 0..3 {
        for k in 0..5 {
            for sg in [-1.0, 1.0] {
                let mut q = p;
                let (cc, ss) = (p[4].cos(), p[4].sin());
                if k < 2 {
                    let du = if k == 0 { step[0] * sg } else { 0.0 };
                    let dv = if k == 1 { step[1] * sg } else { 0.0 };
                    q[0] += du * cc - dv * ss;
                    q[1] += du * ss + dv * cc;
                } else {
                    q[k] += step[k] * sg;
                }
                q[5] = heart_err(ux, uy, &q, 3);
                if q[5] < p[5] {
                    p = q;
                    break;
                }
            }
        }
        for s in step.iter_mut() {
            *s /= 2.0;
        }
    }
    // fast aufrecht -> aufrecht
    if p[4] != 0.0 && p[4].abs() < 8f64.to_radians() {
        let mut q = p;
        q[4] = 0.0;
        q[5] = heart_err(ux, uy, &q, 3);
        if q[5] < 1.1 * p[5] {
            p = q;
        }
    }
    p[5] = heart_err(ux, uy, &p, 1) / dg;
    Some(p)
}

/// rms-Abstand zur Herzkontur; step > 1 wertet nur jeden step-ten Punkt aus.
fn heart_err(x: &[f64], y: &[f64], p: &[f64; 6], step: usize) -> f64 {
    let q = [(p[0] - p[2] / 2.0) as f32, (p[1] - p[3] / 2.0) as f32, (p[0] + p[2] / 2.0) as f32, (p[1] + p[3] / 2.0) as f32];
    let hp = item::heart_pts(&q, p[4] as f32, if step > 1 { 16 } else { 24 });
    let k = hp.len() / 2;
    let h = |j: usize| (hp[j * 2] as f64, hp[j * 2 + 1] as f64);
    let (mut c, mut sum) = (0usize, 0.0);
    let mut i = 0;
    while i < x.len() {
        c += 1;
        let (mut bj, mut bd) = (0usize, 1e30f64);
        for j in 0..k {
            let (hx, hy) = h(j);
            let (dx, dy) = (x[i] - hx, y[i] - hy);
            let d = dx * dx + dy * dy;
            if d < bd {
                bd = d;
                bj = j;
            }
        }
        let (j0, j2) = ((bj + k - 1) % k, (bj + 1) % k);
        let (a, b, e) = (h(j0), h(bj), h(j2));
        let best = seg_d(x[i], y[i], a.0, a.1, b.0, b.1).min(seg_d(x[i], y[i], b.0, b.1, e.0, e.1));
        sum += best * best;
        i += step;
    }
    (sum / c as f64).sqrt()
}

// ------------------------------------------------------------------ Hilfsfunktionen

/// Ausgleichsgerade (total least squares) der Punkte from..=to (Indizes ggf. modulo n): [mx, my, dx, dy].
fn tls(x: &[f64], y: &[f64], from: i64, to: i64, n: usize, wrap: bool) -> Option<[f64; 4]> {
    let ni = n as i64;
    let ix = |t: i64| -> usize {
        if wrap {
            (((t % ni) + ni) % ni) as usize
        } else {
            t.clamp(0, ni - 1) as usize
        }
    };
    let (mut sx, mut sy, mut cnt) = (0.0, 0.0, 0.0);
    for t in from..=to {
        let i = ix(t);
        sx += x[i];
        sy += y[i];
        cnt += 1.0;
    }
    if cnt < 2.0 {
        return None;
    }
    sx /= cnt;
    sy /= cnt;
    let (mut sxx, mut syy, mut sxy) = (0.0, 0.0, 0.0);
    for t in from..=to {
        let i = ix(t);
        let (dx, dy) = (x[i] - sx, y[i] - sy);
        sxx += dx * dx;
        syy += dy * dy;
        sxy += dx * dy;
    }
    let th = 0.5 * (2.0 * sxy).atan2(sxx - syy);
    Some([sx, sy, th.cos(), th.sin()])
}

/// Strecke deckungsgleich zum Strich: Ausgleichsgerade der Punkte from..to, Enden = Projektion von a und b.
#[allow(clippy::too_many_arguments)]
fn fit_segment(x: &[f64], y: &[f64], from: i64, to: i64, ax: f64, ay: f64, bx: f64, by: f64) -> Vec<f32> {
    let Some(ln) = tls(x, y, from, to, x.len(), false) else { return snap_line(ax, ay, bx, by) };
    let (mut dx, mut dy) = (ln[2], ln[3]);
    if (bx - ax) * dx + (by - ay) * dy < 0.0 {
        dx = -dx;
        dy = -dy;
    }
    let t0 = (ax - ln[0]) * dx + (ay - ln[1]) * dy;
    let t1 = (bx - ln[0]) * dx + (by - ln[1]) * dy;
    snap_line(ln[0] + dx * t0, ln[1] + dy * t0, ln[0] + dx * t1, ln[1] + dy * t1)
}

/// Gauss-Jordan mit Pivotsuche auf der erweiterten Matrix (n x n+1).
fn solve(mut m: Vec<Vec<f64>>) -> Option<Vec<f64>> {
    let n = m.len();
    for c in 0..n {
        let mut p = c;
        for r in c + 1..n {
            if m[r][c].abs() > m[p][c].abs() {
                p = r;
            }
        }
        if m[p][c].abs() < 1e-12 {
            return None;
        }
        m.swap(c, p);
        for r in 0..n {
            if r == c {
                continue;
            }
            let f = m[r][c] / m[c][c];
            for k in c..=n {
                m[r][k] -= f * m[c][k];
            }
        }
    }
    Some((0..n).map(|i| m[i][n] / m[i][i]).collect())
}

/// Gleichmaessig nach Bogenlaenge neu abtasten. closed: Schleife (Schlusskante letzter->erster Punkt).
fn resample(x: &[f64], y: &[f64], m: usize, cnt: usize, closed: bool) -> (Vec<f64>, Vec<f64>) {
    let segs = if closed { m } else { m - 1 };
    let mut cum = vec![0.0f64; segs + 1];
    for i in 0..segs {
        let k = (i + 1) % m;
        cum[i + 1] = cum[i] + hyp(x[k] - x[i], y[k] - y[i]);
    }
    let tot = cum[segs];
    let mut ox = vec![0.0f64; cnt];
    let mut oy = vec![0.0f64; cnt];
    if tot <= 0.0 {
        ox.fill(x[0]);
        oy.fill(y[0]);
        return (ox, oy);
    }
    let mut sgi = 0usize;
    for i in 0..cnt {
        let d = if closed { tot * i as f64 / cnt as f64 } else { tot * i as f64 / (cnt - 1) as f64 };
        while sgi + 1 < segs && cum[sgi + 1] < d {
            sgi += 1;
        }
        let k = (sgi + 1) % m;
        let f = if cum[sgi + 1] > cum[sgi] { (d - cum[sgi]) / (cum[sgi + 1] - cum[sgi]) } else { 0.0 };
        let f = clamp(f, 0.0, 1.0);
        ox[i] = x[sgi] + (x[k] - x[sgi]) * f;
        oy[i] = y[sgi] + (y[k] - y[sgi]) * f;
    }
    (ox, oy)
}

/// Gleitender Mittelwert (Radius r Punkte), mehrere Durchgaenge; Enden offener Striche bleiben fest.
fn smooth(x: &mut [f64], y: &mut [f64], r: usize, passes: usize, closed: bool) {
    let n = x.len();
    let mut tx = vec![0.0f64; n];
    let mut ty = vec![0.0f64; n];
    for _ in 0..passes {
        for i in 0..n {
            if !closed && (i == 0 || i == n - 1) {
                tx[i] = x[i];
                ty[i] = y[i];
                continue;
            }
            let (mut sx, mut sy, mut c) = (0.0, 0.0, 0.0);
            let rr = if closed { r } else { r.min(i.min(n - 1 - i)) } as i64;
            for d in -rr..=rr {
                let k = if closed { ((i as i64 + d + n as i64 * 8) % n as i64) as usize } else { (i as i64 + d) as usize };
                sx += x[k];
                sy += y[k];
                c += 1.0;
            }
            tx[i] = sx / c;
            ty[i] = sy / c;
        }
        x.copy_from_slice(&tx);
        y.copy_from_slice(&ty);
    }
}

fn seg_d(px: f64, py: f64, ax: f64, ay: f64, bx: f64, by: f64) -> f64 {
    let (dx, dy) = (bx - ax, by - ay);
    let l = dx * dx + dy * dy;
    let t = if l == 0.0 { 0.0 } else { ((px - ax) * dx + (py - ay) * dy) / l };
    let t = clamp(t, 0.0, 1.0);
    hyp(px - ax - t * dx, py - ay - t * dy)
}

/// Groesste Abweichung der Punkte a..b von der Sehne a-b.
fn dev(x: &[f64], y: &[f64], a: usize, b: usize) -> f64 {
    let mut m: f64 = 0.0;
    for i in a + 1..b {
        m = m.max(seg_d(x[i], y[i], x[a], y[a], x[b], y[b]));
    }
    m
}

/// Ausrichten aufs Winkelraster (Vorgabe 45 Grad) bei weniger als `line_tol` Abweichung (Vorgabe 6 Grad) (Drehung um die Mitte, Laenge bleibt).
fn snap_line(mut x0: f64, mut y0: f64, mut x1: f64, mut y1: f64) -> Vec<f32> {
    let (dx, dy) = (x1 - x0, y1 - y0);
    let l = hyp(dx, dy);
    if l > 0.0 {
        let ang = dy.atan2(dx).to_degrees();
        let c = cfg();
        let step = c.line_step.max(1.0) as f64;
        let snap = jround(ang / step) as f64 * step;
        if c.lines && (ang - snap).abs() < c.line_tol as f64 {
            let (mx, my) = ((x0 + x1) / 2.0, (y0 + y1) / 2.0);
            let (cx, cy) = (snap.to_radians().cos() * l / 2.0, snap.to_radians().sin() * l / 2.0);
            x0 = mx - cx;
            y0 = my - cy;
            x1 = mx + cx;
            y1 = my + cy;
        }
    }
    vec![x0 as f32, y0 as f32, x1 as f32, y1 as f32]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn einrasten_einstellbar() {
        assert_eq!(SnapCfg::snap_deg(13.0, 15.0, 4.0), Some(15.0));
        let i = SnapCfg { shapes: false, ..SnapCfg::default() }.inverted();
        assert!(!i.lines && i.shapes && !i.handles && i.line_step == 45.0);
        assert_eq!(SnapCfg::snap_deg(10.0, 15.0, 4.0), None);
        assert_eq!(SnapCfg::snap_deg(10.0, 15.0, 0.0), None);
        let ang = |q: &[f32]| ((q[3] - q[1]) as f64).atan2((q[2] - q[0]) as f64).to_degrees();
        CFG.with(|c| c.set(SnapCfg::default()));
        assert!((ang(&snap_line(0.0, 0.0, 100.0, 8.0)) - 0.0).abs() < 1e-3);
        CFG.with(|c| c.set(SnapCfg { lines: false, ..SnapCfg::default() }));
        assert!(ang(&snap_line(0.0, 0.0, 100.0, 8.0)) > 4.0);
        CFG.with(|c| c.set(SnapCfg { line_step: 15.0, line_tol: 10.0, ..SnapCfg::default() }));
        assert!((ang(&snap_line(0.0, 0.0, 100.0, 22.0)) - 15.0).abs() < 1e-3);
        CFG.with(|c| c.set(SnapCfg::default()));
    }

    fn stroke(pts: &[(f64, f64)]) -> Item {
        let mut it = Item::stroke(0xFF000000, 4.0, false, false);
        if let Kind::Stroke(s) = &mut it.kind {
            for &(x, y) in pts {
                s.p.extend_from_slice(&[x as f32, y as f32, 0.5]);
            }
        }
        it
    }

    fn poly_path(v: &[(f64, f64)], per_side: usize) -> Vec<(f64, f64)> {
        let mut o = Vec::new();
        for i in 0..=v.len() {
            let (a, b) = (v[i % v.len()], v[(i + 1) % v.len()]);
            if i == v.len() {
                o.push(a);
                break;
            }
            for k in 0..per_side {
                let t = k as f64 / per_side as f64;
                o.push((a.0 + (b.0 - a.0) * t, a.1 + (b.1 - a.1) * t));
            }
        }
        o
    }

    fn kind_of(it: &Item) -> (i32, usize) {
        match &it.kind {
            Kind::Shape(s) => (s.kind, s.q.len()),
            _ => (-1, 0),
        }
    }

    #[test]
    fn line() {
        let pts: Vec<_> = (0..40).map(|i| (100.0 + i as f64 * 5.0, 100.0 + (i % 3) as f64 * 0.5)).collect();
        let r = run(&stroke(&pts), 1.0).expect("Linie");
        assert_eq!(kind_of(&r).0, item::LINE);
    }

    #[test]
    fn circle() {
        let pts: Vec<_> = (0..=90)
            .map(|i| {
                let a = i as f64 / 90.0 * 2.0 * PI;
                (300.0 + 100.0 * a.cos(), 300.0 + 100.0 * a.sin())
            })
            .collect();
        let r = run(&stroke(&pts), 1.0).expect("Kreis");
        assert_eq!(kind_of(&r).0, item::ELLIPSE);
    }

    #[test]
    fn rectangle() {
        let pts = poly_path(&[(100.0, 100.0), (400.0, 100.0), (400.0, 300.0), (100.0, 300.0)], 25);
        let r = run(&stroke(&pts), 1.0).expect("Rechteck");
        assert_eq!(kind_of(&r), (item::POLY, 8));
    }

    #[test]
    fn triangle_shape() {
        let pts = poly_path(&[(200.0, 100.0), (350.0, 360.0), (50.0, 360.0)], 30);
        let r = run(&stroke(&pts), 1.0).expect("Dreieck");
        assert_eq!(kind_of(&r), (item::POLY, 6));
    }

    #[test]
    fn arrow_shape() {
        let mut pts: Vec<_> = (0..=40).map(|i| (100.0 + i as f64 * 7.5, 200.0)).collect();
        for i in 1..=10 {
            pts.push((400.0 - i as f64 * 4.0, 200.0 - i as f64 * 3.0));
        }
        let r = run(&stroke(&pts), 1.0).expect("Pfeil");
        assert_eq!(kind_of(&r).0, item::ARROW);
    }

    #[test]
    fn scribble_none() {
        let pts: Vec<_> = (0..120)
            .map(|i| {
                let t = i as f64 * 0.31;
                (200.0 + 80.0 * (t * 1.7).sin() + 30.0 * (t * 5.3).cos(), 200.0 + 60.0 * (t * 2.9).cos() + i as f64)
            })
            .collect();
        assert!(run(&stroke(&pts), 1.0).is_none());
    }
}
