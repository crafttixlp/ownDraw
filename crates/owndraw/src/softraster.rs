//! CPU-Rasterizer für egui: Software-Fallback im Browser ohne WebGL (siehe `platform/web/soft`).
//!
//! Verhält sich wie `egui_glow`: Vertexfarben und Texturen im Gamma-Raum (sRGB-Werte ohne Umrechnung),
//! vormultipliziertes Alpha, Mischen `ONE, ONE_MINUS_SRC_ALPHA`, Abtastung in Pixelmitten mit Top-Left-Regel,
//! Clip-Rechtecke gerundet wie `glScissor`, Texturfilter (nächster/bilinear) und Wrap-Modi wie in `TextureOptions`
//! (ohne Mipmaps, ohne Dithering).
//!
//! Schnell gehalten durch:
//! - nur geänderte Bereiche neu rastern: Primitive werden mit dem letzten Frame verglichen (Hash je Primitiv,
//!   gemeinsame Teilfolge), dazu die Bildschirmbereiche, in denen sich Texturen geändert haben
//!   (Teil-Updates werden über die UV-Abbildung jedes Dreiecks auf den Bildschirm umgerechnet);
//! - achsenparallele Rechtecke (zwei Dreiecke, UV achsenparallel, eine Farbe) ohne Dreiecks-Rasterung,
//!   bei 1:1-Abbildung (Board-Pixmap, Schrift) als direkte Kopie;
//! - Dreiecke mit konstanter UV (egui zeichnet Flächen mit dem weißen Texel der Schrifttextur) ohne Abtastung.
//!
//! Ergebnis: RGBA8 (deckend, damit vormultipliziert = nicht vormultipliziert) für `putImageData`.

use egui::epaint::{ClippedPrimitive, ImageData, Mesh, Primitive, Vertex};
use egui::{Color32, TextureFilter, TextureId, TextureWrapMode, TexturesDelta};
use std::collections::HashMap;

/// Ganzzahliges Rechteck in Bildschirmpixeln (`x1`/`y1` exklusiv).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub struct IRect {
    pub x0: i32,
    pub y0: i32,
    pub x1: i32,
    pub y1: i32,
}

impl IRect {
    pub fn new(x0: i32, y0: i32, x1: i32, y1: i32) -> IRect {
        IRect { x0, y0, x1, y1 }
    }
    pub fn is_empty(&self) -> bool {
        self.x0 >= self.x1 || self.y0 >= self.y1
    }
    pub fn width(&self) -> i32 {
        (self.x1 - self.x0).max(0)
    }
    pub fn height(&self) -> i32 {
        (self.y1 - self.y0).max(0)
    }
    fn area(&self) -> i64 {
        self.width() as i64 * self.height() as i64
    }
    fn intersect(&self, o: &IRect) -> IRect {
        IRect { x0: self.x0.max(o.x0), y0: self.y0.max(o.y0), x1: self.x1.min(o.x1), y1: self.y1.min(o.y1) }
    }
    fn union(&self, o: &IRect) -> IRect {
        IRect { x0: self.x0.min(o.x0), y0: self.y0.min(o.y0), x1: self.x1.max(o.x1), y1: self.y1.max(o.y1) }
    }
    /// Kleinstes Pixelrechteck um ein Gleitkomma-Rechteck (Pixel, deren Mitte darin liegen kann).
    fn around(x0: f32, y0: f32, x1: f32, y1: f32) -> IRect {
        let c = |v: f32| v.clamp(-1.0e6, 1.0e6);
        IRect { x0: c(x0).floor() as i32, y0: c(y0).floor() as i32, x1: c(x1).ceil() as i32, y1: c(y1).ceil() as i32 }
    }
}

struct Tex {
    w: usize,
    h: usize,
    px: Vec<[u8; 4]>,
    mag: TextureFilter,
    min: TextureFilter,
    wrap: TextureWrapMode,
}

impl Tex {
    #[inline]
    fn wrap(&self, i: i32, n: usize) -> usize {
        let n = n as i32;
        (match self.wrap {
            TextureWrapMode::ClampToEdge => i.clamp(0, n - 1),
            TextureWrapMode::Repeat => i.rem_euclid(n),
            TextureWrapMode::MirroredRepeat => {
                let p = i.rem_euclid(2 * n);
                if p < n {
                    p
                } else {
                    2 * n - 1 - p
                }
            }
        }) as usize
    }
    #[inline]
    fn at(&self, x: usize, y: usize) -> [u8; 4] {
        self.px[y * self.w + x]
    }
    /// Abtastung bei UV (normiert), Filter `f`.
    #[inline]
    fn sample(&self, u: f32, v: f32, f: TextureFilter) -> [u8; 4] {
        let (x, y) = (u * self.w as f32, v * self.h as f32);
        match f {
            TextureFilter::Nearest => self.at(self.wrap(x.floor() as i32, self.w), self.wrap(y.floor() as i32, self.h)),
            TextureFilter::Linear => {
                let (x, y) = (x - 0.5, y - 0.5);
                let (fx0, fy0) = (x.floor(), y.floor());
                let (fx, fy) = (x - fx0, y - fy0);
                let (i0, j0) = (fx0 as i32, fy0 as i32);
                let (xa, xb) = (self.wrap(i0, self.w), self.wrap(i0 + 1, self.w));
                let (ya, yb) = (self.wrap(j0, self.h), self.wrap(j0 + 1, self.h));
                bilerp(self.at(xa, ya), self.at(xb, ya), self.at(xa, yb), self.at(xb, yb), fx, fy)
            }
        }
    }
}

#[inline]
fn bilerp(a: [u8; 4], b: [u8; 4], c: [u8; 4], d: [u8; 4], fx: f32, fy: f32) -> [u8; 4] {
    let mut o = [0u8; 4];
    for k in 0..4 {
        let top = a[k] as f32 + (b[k] as f32 - a[k] as f32) * fx;
        let bot = c[k] as f32 + (d[k] as f32 - c[k] as f32) * fx;
        o[k] = (top + (bot - top) * fy + 0.5) as u8;
    }
    o
}

/// a·b/255, gerundet.
#[inline]
fn mul255(a: u32, b: u32) -> u32 {
    let t = a * b + 128;
    (t + (t >> 8)) >> 8
}

#[inline]
fn mul_color(c: [u8; 4], t: [u8; 4]) -> [u8; 4] {
    [mul255(c[0] as u32, t[0] as u32) as u8, mul255(c[1] as u32, t[1] as u32) as u8, mul255(c[2] as u32, t[2] as u32) as u8, mul255(c[3] as u32, t[3] as u32) as u8]
}

/// Vormultipliziertes „über“: d = s + d·(1 − αs).
#[inline]
fn blend(d: &mut [u8; 4], s: [u8; 4]) {
    let ia = 255 - s[3] as u32;
    if ia == 0 {
        *d = s;
    } else if s != [0; 4] {
        for k in 0..4 {
            d[k] = (s[k] as u32 + mul255(d[k] as u32, ia)).min(255) as u8;
        }
    }
}

#[inline]
fn rgba(c: Color32) -> [u8; 4] {
    c.to_array()
}

/// Signatur eines gezeichneten Primitivs (zum Vergleich mit dem letzten Frame).
#[derive(Clone, Copy)]
struct Sig {
    hash: u64,
    bbox: IRect,
}

/// Ein zu zeichnendes Primitiv dieses Frames.
struct Item<'a> {
    mesh: &'a Mesh,
    /// Dreiecke (Indizes) dieses Stücks.
    idx: &'a [u32],
    clip: IRect,
    sig: Sig,
}

#[inline]
fn mix(h: u64, v: u64) -> u64 {
    (h.rotate_left(5) ^ v).wrapping_mul(0x517c_c1b7_2722_0a95)
}

#[inline]
fn hash_vertex(h: u64, v: &Vertex) -> u64 {
    let h = mix(h, ((v.pos.x.to_bits() as u64) << 32) | v.pos.y.to_bits() as u64);
    let h = mix(h, ((v.uv.x.to_bits() as u64) << 32) | v.uv.y.to_bits() as u64);
    mix(h, u32::from_le_bytes(v.color.to_array()) as u64)
}

/// Zerlegt ein Mesh in Stücke von im Mittel ~32 Dreiecken. Die Schnitte hängen nur vom Inhalt der Dreiecke ab
/// (nicht von ihrer Lage im Mesh): egui fasst die ganze Oberfläche oft zu einem einzigen Mesh zusammen, und
/// nach einer Änderung (z. B. Hover-Farbe eines Knopfs) finden die Stücke danach wieder zum alten Raster
/// zurück; so wird nur der geänderte Bereich neu gezeichnet.
fn chunk_mesh<'a>(m: &'a Mesh, clip: IRect, ppp: f32, out: &mut Vec<Item<'a>>) {
    let tid = match m.texture_id {
        TextureId::Managed(i) => i,
        TextureId::User(i) => i ^ (1 << 63),
    };
    let seed = mix(mix(mix(0x1234_5678, tid), ((clip.x0 as u32 as u64) << 32) | clip.y0 as u32 as u64), ((clip.x1 as u32 as u64) << 32) | clip.y1 as u32 as u64);
    let tris = m.indices.len() / 3;
    let mut start = 0;
    let mut h = seed;
    let (mut x0, mut y0, mut x1, mut y1) = (f32::INFINITY, f32::INFINITY, f32::NEG_INFINITY, f32::NEG_INFINITY);
    for t in 0..tris {
        let mut th = 0u64;
        for k in 0..3 {
            let v = &m.vertices[m.indices[t * 3 + k] as usize];
            th = hash_vertex(th, v);
            x0 = x0.min(v.pos.x);
            y0 = y0.min(v.pos.y);
            x1 = x1.max(v.pos.x);
            y1 = y1.max(v.pos.y);
        }
        h = mix(h, th);
        let n = t + 1 - start;
        if (th >> 40).is_multiple_of(32) || n >= 512 || t + 1 == tris {
            let bbox = IRect::around(x0 * ppp, y0 * ppp, x1 * ppp, y1 * ppp).intersect(&clip);
            if !bbox.is_empty() {
                out.push(Item { mesh: m, idx: &m.indices[start * 3..(t + 1) * 3], clip, sig: Sig { hash: mix(h, n as u64), bbox } });
            }
            start = t + 1;
            h = seed;
            (x0, y0, x1, y1) = (f32::INFINITY, f32::INFINITY, f32::NEG_INFINITY, f32::NEG_INFINITY);
        }
    }
}

/// Software-Painter: hält den Bildpuffer und die Texturen.
#[derive(Default)]
pub struct SoftPainter {
    w: usize,
    h: usize,
    buf: Vec<[u8; 4]>,
    tex: HashMap<TextureId, Tex>,
    prev: Vec<Sig>,
    prev_ppp: f32,
    full: bool,
}

/// Statistik eines `paint`-Aufrufs.
#[derive(Clone, Copy, Debug, Default)]
pub struct PaintInfo {
    /// Neu gerasterte Pixel.
    pub pixels: u64,
    /// Primitive im Frame.
    pub primitives: usize,
}

impl SoftPainter {
    pub fn new() -> SoftPainter {
        SoftPainter { full: true, ..SoftPainter::default() }
    }

    /// Bildpuffer (RGBA8, Zeile für Zeile, Breite × Höhe aus [`Self::size`]).
    pub fn pixels(&self) -> &[[u8; 4]] {
        &self.buf
    }
    #[cfg(test)]
    pub fn size(&self) -> (usize, usize) {
        (self.w, self.h)
    }
    /// Nächstes `paint` zeichnet alles neu (z. B. nachdem der Canvas geleert wurde).
    pub fn invalidate(&mut self) {
        self.full = true;
    }

    /// Wendet `delta.set` an, rastert die geänderten Bereiche und gibt Texturen frei (`delta.free`); `delta` ist danach leer.
    /// Liefert die neu gezeichneten Rechtecke (Bildschirmpixel), die auf den Canvas kopiert werden müssen.
    pub fn paint(&mut self, w: usize, h: usize, clear: Color32, prims: &[ClippedPrimitive], ppp: f32, delta: &mut TexturesDelta) -> (Vec<IRect>, PaintInfo) {
        if w != self.w || h != self.h {
            self.w = w;
            self.h = h;
            self.buf = vec![[0, 0, 0, 255]; w * h];
            self.full = true;
        }
        if ppp != self.prev_ppp {
            self.prev_ppp = ppp;
            self.full = true;
        }
        let screen = IRect::new(0, 0, w as i32, h as i32);

        // 1. Texturen setzen, geänderte Texelbereiche merken
        let mut changes: Vec<(TextureId, [f32; 4])> = Vec::new();
        for (id, deltas) in &delta.set {
            for d in deltas {
                let ImageData::Color(img) = &d.image;
                let [iw, ih] = img.size;
                let px: Vec<[u8; 4]> = img.pixels.iter().map(|c| c.to_array()).collect();
                match d.pos {
                    None => {
                        self.tex.insert(*id, Tex { w: iw, h: ih, px, mag: d.options.magnification, min: d.options.minification, wrap: d.options.wrap_mode });
                        changes.push((*id, [0.0, 0.0, iw as f32, ih as f32]));
                    }
                    Some([x, y]) => {
                        let Some(t) = self.tex.get_mut(id) else { continue };
                        t.mag = d.options.magnification;
                        t.min = d.options.minification;
                        t.wrap = d.options.wrap_mode;
                        let (cw, ch) = (iw.min(t.w.saturating_sub(x)), ih.min(t.h.saturating_sub(y)));
                        for r in 0..ch {
                            let o = (y + r) * t.w + x;
                            t.px[o..o + cw].copy_from_slice(&px[r * iw..r * iw + cw]);
                        }
                        changes.push((*id, [x as f32, y as f32, (x + cw) as f32, (y + ch) as f32]));
                    }
                }
            }
        }

        // 2. Primitive dieses Frames
        let mut items: Vec<Item> = Vec::with_capacity(prims.len());
        for p in prims {
            let Primitive::Mesh(m) = &p.primitive else { continue };
            if m.indices.is_empty() || m.vertices.is_empty() {
                continue;
            }
            let c = p.clip_rect;
            let clip = IRect::new((c.min.x * ppp).round() as i32, (c.min.y * ppp).round() as i32, (c.max.x * ppp).round() as i32, (c.max.y * ppp).round() as i32).intersect(&screen);
            if clip.is_empty() {
                continue;
            }
            chunk_mesh(m, clip, ppp, &mut items);
        }

        // 3. Geänderte Bereiche
        let damage = if self.full {
            vec![screen]
        } else {
            let mut d = Vec::new();
            diff(&self.prev, &items, &mut d);
            for (id, r) in &changes {
                if let Some(t) = self.tex.get(id) {
                    for it in items.iter().filter(|i| i.mesh.texture_id == *id) {
                        tex_damage(it, t, *r, ppp, &mut d);
                    }
                }
            }
            merge(d, screen)
        };

        // 4. Rastern
        let mut info = PaintInfo { pixels: 0, primitives: items.len() };
        let clear = {
            let mut c = rgba(clear);
            c[3] = 255;
            c
        };
        for d in &damage {
            info.pixels += d.area() as u64;
            for y in d.y0..d.y1 {
                let o = y as usize * w;
                self.buf[o + d.x0 as usize..o + d.x1 as usize].fill(clear);
            }
            for it in &items {
                if it.sig.bbox.intersect(d).is_empty() {
                    continue;
                }
                let Some(t) = self.tex.get(&it.mesh.texture_id) else { continue };
                let clip = it.clip.intersect(d);
                draw_mesh(&mut self.buf, w, it.mesh, it.idx, t, ppp, clip);
            }
        }

        // 5. Aufräumen
        for id in &delta.free {
            self.tex.remove(id);
        }
        self.prev = items.iter().map(|i| i.sig).collect();
        self.full = false;
        delta.clear();
        (damage, info)
    }
}

/// Bereiche der Primitive, die nicht in einer gemeinsamen Teilfolge von altem und neuem Frame liegen.
/// (Pixel außerhalb davon werden von denselben Primitiven in derselben Reihenfolge bedeckt wie zuvor.)
fn diff(prev: &[Sig], cur: &[Item], out: &mut Vec<IRect>) {
    let (mut a, mut pe, mut ce) = (0, prev.len(), cur.len());
    while a < pe && a < ce && prev[a].hash == cur[a].sig.hash {
        a += 1;
    }
    while pe > a && ce > a && prev[pe - 1].hash == cur[ce - 1].sig.hash {
        pe -= 1;
        ce -= 1;
    }
    let pm = &prev[a..pe];
    let mut pos: HashMap<u64, Vec<usize>> = HashMap::new();
    for (i, s) in pm.iter().enumerate() {
        pos.entry(s.hash).or_default().push(i);
    }
    let mut matched = vec![false; pm.len()];
    let mut next = 0usize;
    for it in &cur[a..ce] {
        if let Some(v) = pos.get(&it.sig.hash) {
            let k = v.partition_point(|&i| i < next);
            // nur nahe Treffer übernehmen, sonst würden viele Primitive dazwischen als geändert gelten
            if let Some(&i) = v.get(k).filter(|&&i| i - next <= 64) {
                matched[i] = true;
                next = i + 1;
                continue;
            }
        }
        out.push(it.sig.bbox);
    }
    for (i, s) in pm.iter().enumerate() {
        if !matched[i] {
            out.push(s.bbox);
        }
    }
}

/// Bildschirmbereich, in dem das Primitiv `it` den geänderten Texelbereich `r` zeigt.
fn tex_damage(it: &Item, t: &Tex, r: [f32; 4], ppp: f32, out: &mut Vec<IRect>) {
    let (tw, th) = (t.w as f32, t.h as f32);
    // ein Texel Rand für bilineare Filter
    let r = [r[0] - 1.0, r[1] - 1.0, r[2] + 1.0, r[3] + 1.0];
    let full_tex = r[0] <= 0.0 && r[1] <= 0.0 && r[2] >= tw && r[3] >= th;
    if full_tex || t.wrap != TextureWrapMode::ClampToEdge {
        out.push(it.sig.bbox);
        return;
    }
    let m = it.mesh;
    for tri in it.idx.chunks_exact(3) {
        let v: [&Vertex; 3] = [&m.vertices[tri[0] as usize], &m.vertices[tri[1] as usize], &m.vertices[tri[2] as usize]];
        let uv: [(f32, f32); 3] = [0, 1, 2].map(|k| (v[k].uv.x * tw, v[k].uv.y * th));
        let (umin, umax) = (uv[0].0.min(uv[1].0).min(uv[2].0), uv[0].0.max(uv[1].0).max(uv[2].0));
        let (vmin, vmax) = (uv[0].1.min(uv[1].1).min(uv[2].1), uv[0].1.max(uv[1].1).max(uv[2].1));
        if umax < r[0] || umin > r[2] || vmax < r[1] || vmin > r[3] {
            continue;
        }
        let p: [(f32, f32); 3] = [0, 1, 2].map(|k| (v[k].pos.x * ppp, v[k].pos.y * ppp));
        let tb = IRect::around(p[0].0.min(p[1].0).min(p[2].0), p[0].1.min(p[1].1).min(p[2].1), p[0].0.max(p[1].0).max(p[2].0), p[0].1.max(p[1].1).max(p[2].1)).intersect(&it.clip);
        if tb.is_empty() {
            continue;
        }
        // affine Abbildung Texel → Bildschirm aus den drei Ecken
        let (du1, dv1, du2, dv2) = (uv[1].0 - uv[0].0, uv[1].1 - uv[0].1, uv[2].0 - uv[0].0, uv[2].1 - uv[0].1);
        let det = du1 * dv2 - du2 * dv1;
        if det.abs() < 1e-6 {
            out.push(tb);
            continue;
        }
        let (dx1, dy1, dx2, dy2) = (p[1].0 - p[0].0, p[1].1 - p[0].1, p[2].0 - p[0].0, p[2].1 - p[0].1);
        // [x y] = P0 + M · (t − T0), M = [dP1 dP2] · inv([dT1 dT2])
        let (i00, i01, i10, i11) = (dv2 / det, -du2 / det, -dv1 / det, du1 / det);
        let (m00, m01) = (dx1 * i00 + dx2 * i10, dx1 * i01 + dx2 * i11);
        let (m10, m11) = (dy1 * i00 + dy2 * i10, dy1 * i01 + dy2 * i11);
        let map = |tu: f32, tv: f32| (p[0].0 + m00 * (tu - uv[0].0) + m01 * (tv - uv[0].1), p[0].1 + m10 * (tu - uv[0].0) + m11 * (tv - uv[0].1));
        let cs = [map(r[0], r[1]), map(r[2], r[1]), map(r[0], r[3]), map(r[2], r[3])];
        let (x0, x1) = cs.iter().fold((f32::INFINITY, f32::NEG_INFINITY), |(a, b), c| (a.min(c.0), b.max(c.0)));
        let (y0, y1) = cs.iter().fold((f32::INFINITY, f32::NEG_INFINITY), |(a, b), c| (a.min(c.1), b.max(c.1)));
        let d = IRect::around(x0 - 1.0, y0 - 1.0, x1 + 1.0, y1 + 1.0).intersect(&tb);
        if !d.is_empty() {
            out.push(d);
        }
    }
}

/// Fasst Rechtecke zusammen (wenige, nicht zu viel Verschnitt); bei sehr vielen ein einziges.
fn merge(v: Vec<IRect>, screen: IRect) -> Vec<IRect> {
    let mut v: Vec<IRect> = v.into_iter().map(|r| r.intersect(&screen)).filter(|r| !r.is_empty()).collect();
    if v.len() > 256 {
        let u = v.iter().fold(v[0], |a, b| a.union(b));
        return vec![u];
    }
    loop {
        let mut merged = false;
        'outer: for i in 0..v.len() {
            for j in i + 1..v.len() {
                let u = v[i].union(&v[j]);
                if u.area() <= (v[i].area() + v[j].area()) * 5 / 4 + 4096 {
                    v[i] = u;
                    v.swap_remove(j);
                    merged = true;
                    break 'outer;
                }
            }
        }
        if !merged {
            break;
        }
    }
    if v.len() > 24 {
        let u = v.iter().fold(v[0], |a, b| a.union(b));
        return vec![u];
    }
    v
}

/// Achsenparalleles Rechteck aus zwei Dreiecken (Pixelkoordinaten, UV linear in x bzw. y).
struct Quad {
    x0: f32,
    y0: f32,
    x1: f32,
    y1: f32,
    u0: f32,
    u1: f32,
    v0: f32,
    v1: f32,
    color: [u8; 4],
}

/// Erkennt zwei Dreiecke, die zusammen genau ein achsenparalleles Rechteck bilden (gemeinsame Diagonale),
/// mit einer Farbe und achsenparalleler UV.
fn as_quad(m: &Mesh, idx: &[u32], ppp: f32) -> Option<Quad> {
    let (t0, t1) = (&idx[0..3], &idx[3..6]);
    let only0: Vec<u32> = t0.iter().copied().filter(|i| !t1.contains(i)).collect();
    let only1: Vec<u32> = t1.iter().copied().filter(|i| !t0.contains(i)).collect();
    if only0.len() != 1 || only1.len() != 1 || t0[0] == t0[1] || t0[1] == t0[2] || t0[0] == t0[2] {
        return None;
    }
    let shared: Vec<u32> = t0.iter().copied().filter(|i| t1.contains(i)).collect();
    if shared.len() != 2 {
        return None;
    }
    let vs = [&m.vertices[only0[0] as usize], &m.vertices[only1[0] as usize], &m.vertices[shared[0] as usize], &m.vertices[shared[1] as usize]];
    let (a, b) = (vs[0], vs[1]);
    if a.pos.x == b.pos.x || a.pos.y == b.pos.y {
        return None;
    }
    let col = a.color;
    for s in &vs[2..] {
        let corner = (s.pos.x == a.pos.x && s.pos.y == b.pos.y) || (s.pos.x == b.pos.x && s.pos.y == a.pos.y);
        if !corner {
            return None;
        }
    }
    if vs[2].pos == vs[3].pos {
        return None;
    }
    for p in &vs {
        if p.color != col {
            return None;
        }
        for q in &vs {
            if (p.pos.x == q.pos.x && p.uv.x != q.uv.x) || (p.pos.y == q.pos.y && p.uv.y != q.uv.y) {
                return None;
            }
        }
    }
    let (lo_x, hi_x) = if a.pos.x < b.pos.x { (a, b) } else { (b, a) };
    let (lo_y, hi_y) = if a.pos.y < b.pos.y { (a, b) } else { (b, a) };
    Some(Quad {
        x0: lo_x.pos.x * ppp,
        x1: hi_x.pos.x * ppp,
        y0: lo_y.pos.y * ppp,
        y1: hi_y.pos.y * ppp,
        u0: lo_x.uv.x,
        u1: hi_x.uv.x,
        v0: lo_y.uv.y,
        v1: hi_y.uv.y,
        color: rgba(col),
    })
}

fn draw_mesh(buf: &mut [[u8; 4]], w: usize, m: &Mesh, idx: &[u32], t: &Tex, ppp: f32, clip: IRect) {
    let mut i = 0;
    while i + 3 <= idx.len() {
        if i + 6 <= idx.len() {
            if let Some(q) = as_quad(m, &idx[i..i + 6], ppp) {
                draw_quad(buf, w, &q, t, clip);
                i += 6;
                continue;
            }
        }
        let v = [&m.vertices[idx[i] as usize], &m.vertices[idx[i + 1] as usize], &m.vertices[idx[i + 2] as usize]];
        draw_tri(buf, w, v, t, ppp, clip);
        i += 3;
    }
}

/// Erster Pixel, dessen Mitte ≥ `x` liegt.
#[inline]
fn first_px(x: f32) -> i32 {
    (x - 0.5).clamp(-1.0e6, 1.0e6).ceil() as i32
}

fn fill_span(row: &mut [[u8; 4]], c: [u8; 4]) {
    if c[3] == 255 {
        row.fill(c);
    } else if c != [0; 4] {
        for d in row {
            blend(d, c);
        }
    }
}

fn draw_quad(buf: &mut [[u8; 4]], w: usize, q: &Quad, t: &Tex, clip: IRect) {
    let r = IRect::new(first_px(q.x0), first_px(q.y0), first_px(q.x1), first_px(q.y1)).intersect(&clip);
    if r.is_empty() {
        return;
    }
    let white = q.color == [255; 4];
    let (tw, th) = (t.w as f32, t.h as f32);
    // Texel je Pixel → Vergrößern oder Verkleinern
    let sx = (q.u1 - q.u0) * tw / (q.x1 - q.x0);
    let sy = (q.v1 - q.v0) * th / (q.y1 - q.y0);
    let filter = if sx.abs() > 1.0001 || sy.abs() > 1.0001 { t.min } else { t.mag };
    let u_at = |px: i32| q.u0 + (px as f32 + 0.5 - q.x0) * (q.u1 - q.u0) / (q.x1 - q.x0);
    let v_at = |py: i32| q.v0 + (py as f32 + 0.5 - q.y0) * (q.v1 - q.v0) / (q.y1 - q.y0);

    if q.u0 == q.u1 && q.v0 == q.v1 {
        let c = mul_color(q.color, t.sample(q.u0, q.v0, filter));
        for y in r.y0..r.y1 {
            let o = y as usize * w;
            fill_span(&mut buf[o + r.x0 as usize..o + r.x1 as usize], c);
        }
        return;
    }
    // 1:1 mit Texelmitten auf Pixelmitten: bilinear = nächster Texel
    let aligned = |s: f32, c: f32| (s - 1.0).abs() < 1e-4 && ((c - c.floor()) - 0.5).abs() < 1e-3;
    let nearest = filter == TextureFilter::Nearest || (aligned(sx, u_at(r.x0) * tw) && aligned(sy, v_at(r.y0) * th));
    if nearest {
        let cols: Vec<usize> = (r.x0..r.x1).map(|px| t.wrap((u_at(px) * tw).floor() as i32, t.w)).collect();
        for y in r.y0..r.y1 {
            let ty = t.wrap((v_at(y) * th).floor() as i32, t.h);
            let trow = &t.px[ty * t.w..(ty + 1) * t.w];
            let o = y as usize * w;
            let row = &mut buf[o + r.x0 as usize..o + r.x1 as usize];
            // zusammenhängende Texelzeile 1:1: direkt kopieren, wenn deckend
            for (d, &tx) in row.iter_mut().zip(&cols) {
                let s = if white { trow[tx] } else { mul_color(q.color, trow[tx]) };
                if s[3] == 255 {
                    *d = s;
                } else {
                    blend(d, s);
                }
            }
        }
        return;
    }
    // bilinear, getrennt nach Achsen vorberechnet
    let cols: Vec<(usize, usize, f32)> = (r.x0..r.x1)
        .map(|px| {
            let x = u_at(px) * tw - 0.5;
            let f = x.floor();
            (t.wrap(f as i32, t.w), t.wrap(f as i32 + 1, t.w), x - f)
        })
        .collect();
    for y in r.y0..r.y1 {
        let yy = v_at(y) * th - 0.5;
        let f = yy.floor();
        let (ya, yb, fy) = (t.wrap(f as i32, t.h), t.wrap(f as i32 + 1, t.h), yy - f);
        let o = y as usize * w;
        let row = &mut buf[o + r.x0 as usize..o + r.x1 as usize];
        for (d, &(xa, xb, fx)) in row.iter_mut().zip(&cols) {
            let s = bilerp(t.at(xa, ya), t.at(xb, ya), t.at(xa, yb), t.at(xb, yb), fx, fy);
            let s = if white { s } else { mul_color(q.color, s) };
            blend(d, s);
        }
    }
}

/// Grenze einer Kante für eine Zeile: Pixelbereich [lo, hi) bzw. ganze Zeile/keine.
struct Edge {
    // E(x, y) = a·(x − rx) + b·(y − ry), innen: E > 0 oder (E = 0 und tie)
    a: f32,
    b: f32,
    rx: f32,
    ry: f32,
    tie: bool,
}

impl Edge {
    fn new(p: (f32, f32), q: (f32, f32)) -> Edge {
        let (a, b) = (-(q.1 - p.1), q.0 - p.0);
        // Bezugspunkt unabhängig von der Richtung wählen, damit Nachbardreiecke exakt dieselbe Grenze rechnen
        let r = if (p.0, p.1) < (q.0, q.1) { p } else { q };
        Edge { a, b, rx: r.0, ry: r.1, tie: a > 0.0 || (a == 0.0 && b > 0.0) }
    }
    /// Schränkt [lo, hi) der Zeile mit Pixelmitte y ein.
    #[inline]
    fn clip_row(&self, yc: f32, lo: &mut i32, hi: &mut i32) {
        let k = self.b * (yc - self.ry);
        if self.a == 0.0 {
            if !(k > 0.0 || (k == 0.0 && self.tie)) {
                *hi = *lo;
            }
            return;
        }
        // a·(x − rx) + k ≥ 0  ⇔  x ≥ rx − k/a (a > 0)  bzw.  x ≤ rx − k/a (a < 0)
        let xb = (self.rx - k / self.a - 0.5).clamp(-1.0e6, 1.0e6);
        if self.a > 0.0 {
            // px ≥ xb (mit Gleichheit nur bei tie)
            let l = if self.tie { xb.ceil() as i32 } else { xb.floor() as i32 + 1 };
            *lo = (*lo).max(l);
        } else {
            let h = if self.tie { xb.floor() as i32 + 1 } else { xb.ceil() as i32 };
            *hi = (*hi).min(h);
        }
    }
}

fn draw_tri(buf: &mut [[u8; 4]], w: usize, v: [&Vertex; 3], t: &Tex, ppp: f32, clip: IRect) {
    let mut p = [0, 1, 2].map(|k| (v[k].pos.x * ppp, v[k].pos.y * ppp));
    let mut v = v;
    let mut area = (p[1].0 - p[0].0) * (p[2].1 - p[0].1) - (p[1].1 - p[0].1) * (p[2].0 - p[0].0);
    if area.abs() < 1e-8 || !area.is_finite() {
        return;
    }
    if area < 0.0 {
        p.swap(1, 2);
        v.swap(1, 2);
        area = -area;
    }
    let ys = (p[0].1.min(p[1].1).min(p[2].1), p[0].1.max(p[1].1).max(p[2].1));
    let xs = (p[0].0.min(p[1].0).min(p[2].0), p[0].0.max(p[1].0).max(p[2].0));
    let r = IRect::new(first_px(xs.0), first_px(ys.0), first_px(xs.1) + 1, first_px(ys.1) + 1).intersect(&clip);
    if r.is_empty() {
        return;
    }
    let edges = [Edge::new(p[1], p[2]), Edge::new(p[2], p[0]), Edge::new(p[0], p[1])];

    // Attribute: Farbe (4) und UV (2), linear über Baryzentrische Koordinaten
    let col = [0, 1, 2].map(|k| v[k].color.to_array().map(|c| c as f32));
    let uv = [0, 1, 2].map(|k| [v[k].uv.x, v[k].uv.y]);
    let const_col = v[0].color == v[1].color && v[1].color == v[2].color;
    let const_uv = uv[0] == uv[1] && uv[1] == uv[2];
    // λ1 = E(p2→p0)/area, λ2 = E(p0→p1)/area als Funktion von x, y
    let l = |pa: (f32, f32), pb: (f32, f32)| {
        let (a, b) = (-(pb.1 - pa.1) / area, (pb.0 - pa.0) / area);
        (a, b, -(a * pa.0 + b * pa.1))
    };
    let (l1a, l1b, l1c) = l(p[2], p[0]);
    let (l2a, l2b, l2c) = l(p[0], p[1]);
    let (tw, th) = (t.w as f32, t.h as f32);
    let filter = if const_uv {
        t.mag
    } else {
        let du = ((uv[1][0] - uv[0][0]) * l1a + (uv[2][0] - uv[0][0]) * l2a).abs() * tw;
        let dv = ((uv[1][1] - uv[0][1]) * l1b + (uv[2][1] - uv[0][1]) * l2b).abs() * th;
        if du > 1.0001 || dv > 1.0001 {
            t.min
        } else {
            t.mag
        }
    };
    let tex_const = const_uv.then(|| t.sample(uv[0][0], uv[0][1], filter));
    let flat = match (const_col, tex_const) {
        (true, Some(tc)) => Some(mul_color(rgba(v[0].color), tc)),
        _ => None,
    };

    for y in r.y0..r.y1 {
        let yc = y as f32 + 0.5;
        let (mut lo, mut hi) = (r.x0, r.x1);
        for e in &edges {
            e.clip_row(yc, &mut lo, &mut hi);
        }
        if lo >= hi {
            continue;
        }
        let o = y as usize * w;
        let row = &mut buf[o + lo as usize..o + hi as usize];
        if let Some(c) = flat {
            fill_span(row, c);
            continue;
        }
        for (i, d) in row.iter_mut().enumerate() {
            let xc = (lo + i as i32) as f32 + 0.5;
            let l1 = l1a * xc + l1b * yc + l1c;
            let l2 = l2a * xc + l2b * yc + l2c;
            let c = if const_col {
                rgba(v[0].color)
            } else {
                let mut c = [0u8; 4];
                for k in 0..4 {
                    c[k] = (col[0][k] + l1 * (col[1][k] - col[0][k]) + l2 * (col[2][k] - col[0][k]) + 0.5).clamp(0.0, 255.0) as u8;
                }
                c
            };
            let tc = match tex_const {
                Some(tc) => tc,
                None => {
                    let u = uv[0][0] + l1 * (uv[1][0] - uv[0][0]) + l2 * (uv[2][0] - uv[0][0]);
                    let vv = uv[0][1] + l1 * (uv[1][1] - uv[0][1]) + l2 * (uv[2][1] - uv[0][1]);
                    t.sample(u, vv, filter)
                }
            };
            blend(d, mul_color(c, tc));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use egui::epaint::ImageDelta;
    use egui::{pos2, ColorImage, Rect, TextureOptions};

    const FONT: TextureId = TextureId::Managed(0);
    const IMG: TextureId = TextureId::Managed(7);

    fn white_tex() -> (TextureId, ImageDelta) {
        (FONT, ImageDelta::full(ColorImage::new([2, 2], vec![Color32::WHITE; 4]), TextureOptions::LINEAR))
    }

    fn delta(set: Vec<(TextureId, ImageDelta)>) -> TexturesDelta {
        let mut d = TexturesDelta::default();
        for (id, x) in set {
            d.set.entry(id).or_default().push(x);
        }
        d
    }

    fn prim(m: Mesh) -> ClippedPrimitive {
        ClippedPrimitive { clip_rect: Rect::EVERYTHING, primitive: Primitive::Mesh(m) }
    }

    fn rect_mesh(r: Rect, uv: Rect, c: Color32, tex: TextureId) -> Mesh {
        let mut m = Mesh::with_texture(tex);
        m.add_rect_with_uv(r, uv, c);
        m
    }

    fn px(p: &SoftPainter, x: usize, y: usize) -> [u8; 4] {
        p.pixels()[y * p.size().0 + x]
    }

    #[test]
    fn fills_rect_and_blends() {
        let mut p = SoftPainter::new();
        let uv = Rect::from_min_max(pos2(0.25, 0.25), pos2(0.25, 0.25));
        let prims = vec![
            prim(rect_mesh(Rect::from_min_max(pos2(2.0, 2.0), pos2(6.0, 5.0)), uv, Color32::from_rgb(200, 10, 10), FONT)),
            prim(rect_mesh(Rect::from_min_max(pos2(4.0, 0.0), pos2(8.0, 8.0)), uv, Color32::from_black_alpha(128), FONT)),
        ];
        let (dmg, _) = p.paint(10, 10, Color32::WHITE, &prims, 1.0, &mut delta(vec![white_tex()]));
        assert_eq!(dmg, vec![IRect::new(0, 0, 10, 10)]);
        assert_eq!(px(&p, 0, 0), [255, 255, 255, 255]);
        assert_eq!(px(&p, 2, 2), [200, 10, 10, 255]);
        assert_eq!(px(&p, 5, 4), [100, 5, 5, 255]); // halb schwarz darüber
        assert_eq!(px(&p, 6, 4), [127, 127, 127, 255]);
        assert_eq!(px(&p, 6, 5), [127, 127, 127, 255]);
        assert_eq!(px(&p, 2, 5), [255, 255, 255, 255]); // y1 exklusiv
    }

    #[test]
    fn unchanged_frame_has_no_damage_and_changes_are_local() {
        let mut p = SoftPainter::new();
        let uv = Rect::from_min_max(pos2(0.5, 0.5), pos2(0.5, 0.5));
        let a = rect_mesh(Rect::from_min_max(pos2(1.0, 1.0), pos2(3.0, 3.0)), uv, Color32::RED, FONT);
        let b = rect_mesh(Rect::from_min_max(pos2(20.0, 20.0), pos2(25.0, 22.0)), uv, Color32::BLUE, FONT);
        p.paint(100, 100, Color32::WHITE, &[prim(a.clone()), prim(b.clone())], 1.0, &mut delta(vec![white_tex()]));
        let (dmg, _) = p.paint(100, 100, Color32::WHITE, &[prim(a.clone()), prim(b.clone())], 1.0, &mut TexturesDelta::default());
        assert!(dmg.is_empty());
        let b2 = rect_mesh(Rect::from_min_max(pos2(20.0, 20.0), pos2(25.0, 22.0)), uv, Color32::GREEN, FONT);
        let (dmg, _) = p.paint(100, 100, Color32::WHITE, &[prim(a.clone()), prim(b2)], 1.0, &mut TexturesDelta::default());
        assert_eq!(dmg, vec![IRect::new(20, 20, 25, 22)]);
        assert_eq!(px(&p, 21, 21), Color32::GREEN.to_array());
        assert_eq!(px(&p, 1, 1), Color32::RED.to_array());
        // Reihenfolge getauscht: das nicht in der gemeinsamen Teilfolge liegende Primitiv wird neu gezeichnet
        p.paint(100, 100, Color32::WHITE, &[prim(a.clone()), prim(b.clone())], 1.0, &mut TexturesDelta::default());
        let (dmg, _) = p.paint(100, 100, Color32::WHITE, &[prim(b.clone()), prim(a.clone())], 1.0, &mut TexturesDelta::default());
        assert_eq!(dmg, vec![IRect::new(1, 1, 3, 3)]);
    }

    #[test]
    fn partial_texture_update_damages_only_mapped_region() {
        let mut p = SoftPainter::new();
        let img = ColorImage::new([64, 64], vec![Color32::from_rgb(1, 2, 3); 64 * 64]);
        let board = rect_mesh(Rect::from_min_max(pos2(0.0, 0.0), pos2(64.0, 64.0)), Rect::from_min_max(pos2(0.0, 0.0), pos2(1.0, 1.0)), Color32::WHITE, IMG);
        p.paint(64, 64, Color32::WHITE, &[prim(board.clone())], 1.0, &mut delta(vec![(IMG, ImageDelta::full(img, TextureOptions::NEAREST))]));
        assert_eq!(px(&p, 10, 10), [1, 2, 3, 255]);
        let patch = ColorImage::new([4, 3], vec![Color32::from_rgb(9, 9, 9); 12]);
        let (dmg, info) = p.paint(64, 64, Color32::WHITE, &[prim(board)], 1.0, &mut delta(vec![(IMG, ImageDelta::partial([10, 20], patch, TextureOptions::NEAREST))]));
        assert!(info.pixels < 200, "{dmg:?}");
        assert_eq!(px(&p, 10, 20), [9, 9, 9, 255]);
        assert_eq!(px(&p, 13, 22), [9, 9, 9, 255]);
        assert_eq!(px(&p, 14, 22), [1, 2, 3, 255]);
    }

    /// Schnellweg für Rechtecke und allgemeine Dreiecks-Rasterung liefern dasselbe Bild.
    #[test]
    fn quad_fast_path_matches_triangles() {
        let mut pixels = Vec::new();
        for i in 0..16 * 16 {
            pixels.push(Color32::from_rgba_premultiplied((i * 7 % 256) as u8, (i * 13 % 256) as u8, (i % 200) as u8, 255));
        }
        let img = ColorImage::new([16, 16], pixels);
        for (opts, r) in [
            (TextureOptions::LINEAR, Rect::from_min_max(pos2(3.3, 2.7), pos2(40.6, 33.2))),
            (TextureOptions::NEAREST, Rect::from_min_max(pos2(3.3, 2.7), pos2(40.6, 33.2))),
            (TextureOptions::LINEAR, Rect::from_min_max(pos2(5.0, 6.0), pos2(21.0, 22.0))),
        ] {
            let c = Color32::from_rgba_premultiplied(200, 180, 100, 220);
            let m = rect_mesh(r, Rect::from_min_max(pos2(0.0, 0.0), pos2(1.0, 1.0)), c, IMG);
            // gleiche Dreiecke, Indizes so umgestellt, dass kein Rechteck erkannt wird (Dreiecke einzeln)
            let mut m2 = m.clone();
            m2.indices = vec![0, 1, 2, 0, 0, 0, 2, 1, 3];
            assert!(as_quad(&m, &m.indices, 1.0).is_some());
            let run = |m: Mesh| {
                let mut p = SoftPainter::new();
                p.paint(48, 40, Color32::WHITE, &[prim(m)], 1.0, &mut delta(vec![(IMG, ImageDelta::full(img.clone(), opts))]));
                p.pixels().to_vec()
            };
            let (a, b) = (run(m), run(m2));
            let maxdiff = a.iter().zip(&b).map(|(x, y)| (0..4).map(|k| x[k].abs_diff(y[k])).max().unwrap()).max().unwrap();
            assert!(maxdiff <= 1, "max. Abweichung {maxdiff} bei {opts:?}");
        }
    }

    /// Zwei Dreiecke mit gemeinsamer schräger Kante: kein Pixel doppelt, keiner ausgelassen.
    #[test]
    fn shared_edges_cover_once() {
        let mut m = Mesh::with_texture(FONT);
        let c = Color32::from_black_alpha(100);
        for (x, y) in [(0.0, 0.0), (17.3, 1.1), (3.7, 15.9), (19.0, 18.0)] {
            m.vertices.push(Vertex { pos: pos2(x, y), uv: pos2(0.5, 0.5), color: c });
        }
        m.indices = vec![0, 1, 2, 2, 1, 3, 0, 0, 0];
        let mut p = SoftPainter::new();
        p.paint(20, 20, Color32::WHITE, &[prim(m)], 1.0, &mut delta(vec![white_tex()]));
        let once = {
            let mut d = [255u8; 4];
            blend(&mut d, c.to_array());
            d
        };
        for q in p.pixels() {
            assert!(*q == [255; 4] || *q == once, "{q:?}");
        }
    }

    /// Echte egui-Oberfläche über mehrere Frames: nur geänderte Bereiche neu zeichnen ergibt dasselbe Bild
    /// wie jedes Mal alles neu zu zeichnen.
    #[test]
    fn incremental_equals_full_redraw_with_real_ui() {
        let ctx = egui::Context::default();
        let (mut inc, mut full) = (SoftPainter::new(), SoftPainter::new());
        let ppp = 1.5;
        let (w, h) = (450usize, 300usize);
        for frame in 0..12 {
            let mut raw = egui::RawInput { screen_rect: Some(Rect::from_min_size(pos2(0.0, 0.0), egui::vec2(w as f32 / ppp, h as f32 / ppp))), ..Default::default() };
            raw.viewports.entry(egui::ViewportId::ROOT).or_default().native_pixels_per_point = Some(ppp);
            raw.time = Some(frame as f64 * 0.1);
            if frame == 5 {
                raw.events.push(egui::Event::PointerMoved(pos2(40.0, 60.0)));
            }
            let out = ctx.run_ui(raw, |ui| {
                ui.heading("OwnDraw");
                ui.label(format!("Frame {frame}: äöü ß €"));
                let _ = ui.button("Neues Board");
                ui.add(egui::Slider::new(&mut (frame as f32), 0.0..=20.0));
                egui::Window::new("Fenster").default_pos(pos2(80.0 + frame as f32 * 3.0, 90.0)).current_pos(pos2(80.0 + frame as f32 * 3.0, 90.0)).show(ui.ctx(), |ui| {
                    ui.label("Text im Fenster");
                    ui.checkbox(&mut (frame % 2 == 0), "Haken");
                });
            });
            let prims = ctx.tessellate(out.shapes, out.pixels_per_point);
            let mut d1 = out.textures_delta;
            let mut d2 = TexturesDelta::default();
            d2.set = d1.set.clone();
            d2.free = d1.free.clone();
            let (dmg, _) = inc.paint(w, h, Color32::from_gray(30), &prims, ppp, &mut d1);
            full.invalidate();
            full.paint(w, h, Color32::from_gray(30), &prims, ppp, &mut d2);
            if frame > 0 {
                assert!(dmg.iter().map(|r| r.area()).sum::<i64>() < (w * h) as i64, "Frame {frame}: alles neu");
            }
            let diff = inc.pixels().iter().zip(full.pixels()).filter(|(a, b)| a != b).count();
            assert_eq!(diff, 0, "Frame {frame}: {diff} Pixel verschieden");
        }
        // es wurde tatsächlich etwas gezeichnet (Text, Flächen)
        let distinct: std::collections::HashSet<[u8; 4]> = full.pixels().iter().copied().collect();
        assert!(distinct.len() > 20);
    }
}
