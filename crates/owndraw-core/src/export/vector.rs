//! Vektorformate aus [`VOp`]-Listen: SVG, EPS und EMF.

use crate::canvas::VOp;
use std::fmt::Write as _;
use tiny_skia::{Path, PathSegment, Pixmap, Transform};

// ---------------------------------------------------------------- gemeinsame Hilfen (auch für pdfv)

/// Pfadsegment mit Quadratik bereits als Kubik.
#[derive(Clone, Copy, Debug)]
pub(crate) enum Seg {
    M(f32, f32),
    L(f32, f32),
    C([f32; 6]),
    Z,
}

/// Segmente eines Pfads; Segmente mit NaN/∞ werden übersprungen (ungültiges MoveTo verwirft den Teilpfad).
pub(crate) fn segs(p: &Path) -> Vec<Seg> {
    let fin = |v: &[f32]| v.iter().all(|x| x.is_finite());
    let mut v = Vec::new();
    let (mut cur, mut start, mut ok) = ((0.0f32, 0.0f32), (0.0f32, 0.0f32), false);
    for s in p.segments() {
        match s {
            PathSegment::MoveTo(a) => {
                ok = fin(&[a.x, a.y]);
                if ok {
                    v.push(Seg::M(a.x, a.y));
                    cur = (a.x, a.y);
                    start = cur;
                }
            }
            PathSegment::LineTo(a) if ok && fin(&[a.x, a.y]) => {
                v.push(Seg::L(a.x, a.y));
                cur = (a.x, a.y);
            }
            PathSegment::QuadTo(b, a) if ok && fin(&[a.x, a.y, b.x, b.y]) => {
                let k = 2.0 / 3.0;
                v.push(Seg::C([cur.0 + k * (b.x - cur.0), cur.1 + k * (b.y - cur.1), a.x + k * (b.x - a.x), a.y + k * (b.y - a.y), a.x, a.y]));
                cur = (a.x, a.y);
            }
            PathSegment::CubicTo(b, c, a) if ok && fin(&[a.x, a.y, b.x, b.y, c.x, c.y]) => {
                v.push(Seg::C([b.x, b.y, c.x, c.y, a.x, a.y]));
                cur = (a.x, a.y);
            }
            PathSegment::Close if ok => {
                v.push(Seg::Z);
                cur = start;
            }
            _ => {}
        }
    }
    // Pfad ohne Zeichensegment ist leer
    if v.iter().all(|s| matches!(s, Seg::M(..) | Seg::Z)) {
        v.clear();
    }
    v
}

/// Kompakte Zahl mit höchstens `dec` Nachkommastellen (ohne Nullen am Ende, kein "-0").
pub(crate) fn num(v: f32, dec: usize) -> String {
    if !v.is_finite() {
        return "0".into();
    }
    let mut s = format!("{:.*}", dec, v);
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
    s
}

/// Gültige Seitengröße (NaN/≤0 → 1).
pub(crate) fn dim(v: f32) -> f32 {
    if v.is_finite() && v > 0.0 { v } else { 1.0 }
}

/// RGB einer 0xAARRGGBB-Farbe als Bytes.
pub(crate) fn rgb(c: u32) -> (u8, u8, u8) {
    ((c >> 16) as u8, (c >> 8) as u8, c as u8)
}

/// Farbe mit Deckkraft `a` (0..1) gegen Weiß gemischt.
fn on_white(c: u32, a: f32) -> (u8, u8, u8) {
    let (r, g, b) = rgb(c);
    let m = |x: u8| (x as f32 * a + 255.0 * (1.0 - a)).round().clamp(0.0, 255.0) as u8;
    (m(r), m(g), m(b))
}

/// Deckkraft einer 0xAARRGGBB-Farbe (0..1).
pub(crate) fn alpha(c: u32) -> f32 {
    (c >> 24) as f32 / 255.0
}

/// Vormultiplizierte Pixel auf Weiß (Deckkraft-Faktor `ga`), Zeile für Zeile RGB.
fn rgb_on_white(pm: &Pixmap, ga: f32) -> Vec<u8> {
    let mut o = Vec::with_capacity(pm.width() as usize * pm.height() as usize * 3);
    for p in pm.data().chunks_exact(4) {
        let a = p[3] as f32 * ga;
        for &c in &p[..3] {
            o.push((c as f32 * ga + 255.0 - a).round().clamp(0.0, 255.0) as u8);
        }
    }
    o
}

const B64: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";

fn b64(d: &[u8]) -> String {
    let mut o = String::with_capacity(d.len().div_ceil(3) * 4);
    for c in d.chunks(3) {
        let n = (c[0] as u32) << 16 | (*c.get(1).unwrap_or(&0) as u32) << 8 | *c.get(2).unwrap_or(&0) as u32;
        for i in 0..4 {
            o.push(if i <= c.len() { B64[(n >> (18 - 6 * i) & 63) as usize] as char } else { '=' });
        }
    }
    o
}

/// Hülle (l, t, r, b) eines Rechtecks unter einer Transformation.
pub(crate) fn hull(m: &Transform, w: f32, h: f32) -> (f32, f32, f32, f32) {
    let pts = [(0.0, 0.0), (w, 0.0), (0.0, h), (w, h)].map(|(x, y)| (m.sx * x + m.kx * y + m.tx, m.ky * x + m.sy * y + m.ty));
    let mut r = (f32::MAX, f32::MAX, f32::MIN, f32::MIN);
    for (x, y) in pts {
        r = (r.0.min(x), r.1.min(y), r.2.max(x), r.3.max(y));
    }
    r
}

fn transform_ok(m: &Transform) -> bool {
    [m.sx, m.kx, m.ky, m.sy, m.tx, m.ty].iter().all(|v| v.is_finite())
}

// ---------------------------------------------------------------- SVG

fn svg_d(s: &[Seg]) -> String {
    let n = |v: f32| num(v, 2);
    let mut d = String::new();
    for g in s {
        match *g {
            Seg::M(x, y) => {
                let _ = write!(d, "M{} {}", n(x), n(y));
            }
            Seg::L(x, y) => {
                let _ = write!(d, "L{} {}", n(x), n(y));
            }
            Seg::C(c) => {
                let _ = write!(d, "C{} {} {} {} {} {}", n(c[0]), n(c[1]), n(c[2]), n(c[3]), n(c[4]), n(c[5]));
            }
            Seg::Z => d.push('Z'),
        }
    }
    d
}

fn hex(c: u32) -> String {
    format!("#{:06x}", c & 0xFF_FFFF)
}

/// SVG-Dokument der Größe w × h (Einheit px, viewBox 0 0 w h).
pub fn svg(ops: &[VOp], w: f32, h: f32) -> String {
    let (w, h) = (dim(w), dim(h));
    let mut o = String::new();
    let _ = writeln!(o, "<?xml version=\"1.0\" encoding=\"UTF-8\"?>");
    let _ = writeln!(
        o,
        "<svg xmlns=\"http://www.w3.org/2000/svg\" xmlns:xlink=\"http://www.w3.org/1999/xlink\" version=\"1.1\" width=\"{0}\" height=\"{1}\" viewBox=\"0 0 {0} {1}\">",
        num(w, 2),
        num(h, 2)
    );
    // je Ebene: Zahl offener <g> (Save/Group selbst + Clips)
    let mut stack: Vec<usize> = vec![0];
    let mut clip_id = 0;
    for op in ops {
        match op {
            VOp::Fill(p, c) => {
                let s = segs(p);
                if s.is_empty() || c >> 24 == 0 {
                    continue;
                }
                let _ = write!(o, "<path d=\"{}\" fill=\"{}\" fill-rule=\"nonzero\"", svg_d(&s), hex(*c));
                if c >> 24 != 255 {
                    let _ = write!(o, " fill-opacity=\"{}\"", num(alpha(*c), 3));
                }
                o.push_str("/>\n");
            }
            VOp::Stroke { path, color, width, round_cap, dash } => {
                let s = segs(path);
                if s.is_empty() || color >> 24 == 0 {
                    continue;
                }
                let _ = write!(
                    o,
                    "<path d=\"{}\" fill=\"none\" stroke=\"{}\" stroke-width=\"{}\" stroke-linecap=\"{}\" stroke-linejoin=\"round\"",
                    svg_d(&s),
                    hex(*color),
                    num(width.max(0.0), 2),
                    if *round_cap { "round" } else { "butt" }
                );
                if color >> 24 != 255 {
                    let _ = write!(o, " stroke-opacity=\"{}\"", num(alpha(*color), 3));
                }
                if let Some((a, b)) = dash {
                    let _ = write!(o, " stroke-dasharray=\"{} {}\"", num(a.max(0.0), 2), num(b.max(0.0), 2));
                }
                o.push_str("/>\n");
            }
            VOp::Image(pm, m) => {
                if !transform_ok(m) {
                    continue;
                }
                let Ok(png) = pm.encode_png() else { continue };
                let n = |v: f32| num(v, 6);
                let _ = writeln!(
                    o,
                    "<image width=\"{}\" height=\"{}\" preserveAspectRatio=\"none\" transform=\"matrix({} {} {} {} {} {})\" xlink:href=\"data:image/png;base64,{}\"/>",
                    pm.width(),
                    pm.height(),
                    n(m.sx),
                    n(m.ky),
                    n(m.kx),
                    n(m.sy),
                    n(m.tx),
                    n(m.ty),
                    b64(&png)
                );
            }
            VOp::Save => {
                o.push_str("<g>\n");
                stack.push(1);
            }
            VOp::Group(a) => {
                let _ = writeln!(o, "<g opacity=\"{}\">", num(*a as f32 / 255.0, 3));
                stack.push(1);
            }
            VOp::Restore => {
                if stack.len() > 1 {
                    for _ in 0..stack.pop().unwrap_or(0) {
                        o.push_str("</g>\n");
                    }
                }
            }
            VOp::Clip(p) => {
                clip_id += 1;
                let _ = writeln!(o, "<defs><clipPath id=\"c{clip_id}\"><path d=\"{}\"/></clipPath></defs>", svg_d(&segs(p)));
                let _ = writeln!(o, "<g clip-path=\"url(#c{clip_id})\">");
                if let Some(t) = stack.last_mut() {
                    *t += 1;
                }
            }
        }
    }
    for n in stack {
        for _ in 0..n {
            o.push_str("</g>\n");
        }
    }
    o.push_str("</svg>\n");
    o
}

// ---------------------------------------------------------------- EPS

fn ps_path(o: &mut String, s: &[Seg]) {
    let n = |v: f32| num(v, 3);
    o.push_str("newpath\n");
    for g in s {
        let _ = match *g {
            Seg::M(x, y) => writeln!(o, "{} {} moveto", n(x), n(y)),
            Seg::L(x, y) => writeln!(o, "{} {} lineto", n(x), n(y)),
            Seg::C(c) => writeln!(o, "{} {} {} {} {} {} curveto", n(c[0]), n(c[1]), n(c[2]), n(c[3]), n(c[4]), n(c[5])),
            Seg::Z => writeln!(o, "closepath"),
        };
    }
}

fn ps_color(o: &mut String, (r, g, b): (u8, u8, u8)) {
    let c = |v: u8| num(v as f32 / 255.0, 4);
    let _ = writeln!(o, "{} {} {} setrgbcolor", c(r), c(g), c(b));
}

/// Encapsulated PostScript (EPSF-3.0), BoundingBox 0 0 w h in Punkt.
pub fn eps(ops: &[VOp], w: f32, h: f32) -> String {
    let (w, h) = (dim(w), dim(h));
    let mut o = String::new();
    o.push_str("%!PS-Adobe-3.0 EPSF-3.0\n");
    let _ = writeln!(o, "%%BoundingBox: 0 0 {} {}", w.ceil() as i64, h.ceil() as i64);
    let _ = writeln!(o, "%%HiResBoundingBox: 0 0 {} {}", num(w, 3), num(h, 3));
    o.push_str("%%Creator: OwnDraw\n%%LanguageLevel: 2\n%%Pages: 1\n%%EndComments\n");
    o.push_str("% Hinweis: PostScript kennt keine Transparenz - Farben und Bilder sind gegen Weiss gemischt.\n");
    o.push_str("%%Page: 1 1\nsave\n");
    let _ = writeln!(o, "0 {} translate 1 -1 scale", num(h, 3));
    o.push_str("1 setlinejoin\n");
    // Deckkraft-Faktor je Ebene (Gruppen multiplizieren)
    let mut ga: Vec<f32> = vec![1.0];
    for op in ops {
        let g = *ga.last().unwrap_or(&1.0);
        match op {
            VOp::Fill(p, c) => {
                let s = segs(p);
                if s.is_empty() || c >> 24 == 0 {
                    continue;
                }
                ps_path(&mut o, &s);
                ps_color(&mut o, on_white(*c, alpha(*c) * g));
                o.push_str("fill\n");
            }
            VOp::Stroke { path, color, width, round_cap, dash } => {
                let s = segs(path);
                if s.is_empty() || color >> 24 == 0 {
                    continue;
                }
                ps_path(&mut o, &s);
                ps_color(&mut o, on_white(*color, alpha(*color) * g));
                let _ = writeln!(o, "{} setlinewidth {} setlinecap 1 setlinejoin", num(width.max(0.0), 3), if *round_cap { 1 } else { 0 });
                match dash {
                    Some((a, b)) if *a > 0.0 || *b > 0.0 => {
                        let _ = writeln!(o, "[{} {}] 0 setdash", num(a.max(0.0), 3), num(b.max(0.0), 3));
                    }
                    _ => o.push_str("[] 0 setdash\n"),
                }
                o.push_str("stroke\n");
            }
            VOp::Image(pm, m) => {
                if !transform_ok(m) {
                    continue;
                }
                let (iw, ih) = (pm.width(), pm.height());
                let n = |v: f32| num(v, 6);
                let _ = writeln!(o, "gsave\n[{} {} {} {} {} {}] concat", n(m.sx), n(m.ky), n(m.kx), n(m.sy), n(m.tx), n(m.ty));
                let _ = writeln!(o, "/picstr {} string def", iw * 3);
                let _ = writeln!(o, "{iw} {ih} 8 [1 0 0 1 0 0] {{currentfile picstr readhexstring pop}} false 3 colorimage");
                let data = rgb_on_white(pm, g);
                for line in data.chunks(40) {
                    for b in line {
                        let _ = write!(o, "{b:02x}");
                    }
                    o.push('\n');
                }
                o.push_str("grestore\n");
            }
            VOp::Save => {
                o.push_str("gsave\n");
                ga.push(g);
            }
            VOp::Group(a) => {
                o.push_str("gsave\n");
                ga.push(g * *a as f32 / 255.0);
            }
            VOp::Restore => {
                if ga.len() > 1 {
                    ga.pop();
                    o.push_str("grestore\n");
                }
            }
            VOp::Clip(p) => {
                let s = segs(p);
                if s.is_empty() {
                    o.push_str("newpath 0 0 moveto 0 0 lineto\n");
                } else {
                    ps_path(&mut o, &s);
                }
                o.push_str("clip newpath\n");
            }
        }
    }
    for _ in 1..ga.len() {
        o.push_str("grestore\n");
    }
    o.push_str("restore\nshowpage\n%%Trailer\n%%EOF\n");
    o
}

// ---------------------------------------------------------------- EMF

/// Logische EMF-Einheiten je px (Unterpixel-Genauigkeit über MM_ANISOTROPIC).
const EMF_K: f32 = 16.0;

struct Emf {
    b: Vec<u8>,
    n: u32,
}

impl Emf {
    fn rec(&mut self, t: u32, d: &[u8]) {
        let len = 8 + d.len();
        let pad = (4 - len % 4) % 4;
        self.b.extend_from_slice(&t.to_le_bytes());
        self.b.extend_from_slice(&((len + pad) as u32).to_le_bytes());
        self.b.extend_from_slice(d);
        self.b.extend(std::iter::repeat_n(0u8, pad));
        self.n += 1;
    }
    fn rec_u32(&mut self, t: u32, v: &[u32]) {
        let d: Vec<u8> = v.iter().flat_map(|x| x.to_le_bytes()).collect();
        self.rec(t, &d);
    }
    fn rec_i32(&mut self, t: u32, v: &[i32]) {
        let d: Vec<u8> = v.iter().flat_map(|x| x.to_le_bytes()).collect();
        self.rec(t, &d);
    }
    /// Pfad zwischen BEGINPATH und ENDPATH; liefert Hülle in logischen Einheiten.
    fn path(&mut self, s: &[Seg]) -> [i32; 4] {
        let q = |v: f32| (v * EMF_K).round().clamp(-1e9, 1e9) as i32;
        let mut bb = [i32::MAX, i32::MAX, i32::MIN, i32::MIN];
        let mut grow = |x: i32, y: i32| {
            bb = [bb[0].min(x), bb[1].min(y), bb[2].max(x), bb[3].max(y)];
        };
        self.rec(59, &[]); // EMR_BEGINPATH
        let mut i = 0;
        while i < s.len() {
            match s[i] {
                Seg::M(x, y) => {
                    grow(q(x), q(y));
                    self.rec_i32(27, &[q(x), q(y)]); // EMR_MOVETOEX
                    i += 1;
                }
                Seg::Z => {
                    self.rec(61, &[]); // EMR_CLOSEFIGURE
                    i += 1;
                }
                Seg::L(..) | Seg::C(..) => {
                    // gleichartige Segmente bündeln: EMR_POLYLINETO (6) bzw. EMR_POLYBEZIERTO (5)
                    let line = matches!(s[i], Seg::L(..));
                    let mut pts: Vec<i32> = Vec::new();
                    while i < s.len() {
                        match s[i] {
                            Seg::L(x, y) if line => pts.extend([q(x), q(y)]),
                            Seg::C(c) if !line => pts.extend(c.map(q)),
                            _ => break,
                        }
                        i += 1;
                    }
                    let mut pb = [i32::MAX, i32::MAX, i32::MIN, i32::MIN];
                    for p in pts.chunks(2) {
                        grow(p[0], p[1]);
                        pb = [pb[0].min(p[0]), pb[1].min(p[1]), pb[2].max(p[0]), pb[3].max(p[1])];
                    }
                    let mut d = pb.to_vec();
                    d.push((pts.len() / 2) as i32);
                    d.extend(pts);
                    self.rec_i32(if line { 6 } else { 5 }, &d);
                }
            }
        }
        self.rec(60, &[]); // EMR_ENDPATH
        if bb[0] > bb[2] { [0; 4] } else { bb }
    }
}

fn colorref((r, g, b): (u8, u8, u8)) -> u32 {
    r as u32 | (g as u32) << 8 | (b as u32) << 16
}

/// Windows Enhanced Metafile (EMF), Größe w × h in Ausgabeeinheiten (96 dpi).
pub fn emf(ops: &[VOp], w: f32, h: f32) -> Vec<u8> {
    let (w, h) = (dim(w), dim(h));
    let (wi, hi) = (w.ceil().min(1e6) as i32, h.ceil().min(1e6) as i32);
    let mut e = Emf { b: Vec::new(), n: 0 };
    // EMR_HEADER (Referenzgerät 9600 px = 2540 mm → genau 96 dpi)
    let mut hd: Vec<u8> = Vec::new();
    let mut put = |v: i32| hd.extend_from_slice(&v.to_le_bytes());
    for v in [0, 0, wi - 1, hi - 1] {
        put(v); // rclBounds (Geräte-px, inklusiv)
    }
    let mm = |v: f32| (v * 2540.0 / 96.0).round() as i32;
    for v in [0, 0, mm(w), mm(h)] {
        put(v); // rclFrame (0,01 mm)
    }
    put(0x464D_4520); // " EMF"
    put(0x0001_0000); // nVersion
    put(0); // nBytes (später)
    put(0); // nRecords (später)
    put(3); // nHandles (u16) + sReserved (u16)
    put(0); // nDescription
    put(0); // offDescription
    put(0); // nPalEntries
    put(9600);
    put(9600); // szlDevice
    put(2540);
    put(2540); // szlMillimeters
    put(0);
    put(0);
    put(0); // cbPixelFormat, offPixelFormat, bOpenGL
    put(2_540_000);
    put(2_540_000); // szlMicrometers
    e.rec(1, &hd);
    e.rec_u32(17, &[8]); // EMR_SETMAPMODE MM_ANISOTROPIC
    e.rec_i32(10, &[0, 0]); // EMR_SETWINDOWORGEX
    e.rec_i32(9, &[wi * EMF_K as i32, hi * EMF_K as i32]); // EMR_SETWINDOWEXTEX
    e.rec_i32(12, &[0, 0]); // EMR_SETVIEWPORTORGEX
    e.rec_i32(11, &[wi, hi]); // EMR_SETVIEWPORTEXTEX
    e.rec_u32(18, &[1]); // EMR_SETBKMODE TRANSPARENT
    e.rec_u32(19, &[2]); // EMR_SETPOLYFILLMODE WINDING
    let dev = |v: i32| (v as f32 / EMF_K).floor() as i32;
    let devb = |b: [i32; 4]| [dev(b[0]), dev(b[1]), dev(b[2]) + 1, dev(b[3]) + 1];
    // je Ebene: Deckkraft-Faktor, Clip aktiv?
    let mut st: Vec<(f32, bool)> = vec![(1.0, false)];
    for op in ops {
        let (g, clipped) = *st.last().unwrap_or(&(1.0, false));
        match op {
            VOp::Fill(p, c) => {
                let s = segs(p);
                if s.is_empty() || c >> 24 == 0 {
                    continue;
                }
                e.rec_u32(39, &[1, 0, colorref(on_white(*c, alpha(*c) * g)), 0]); // EMR_CREATEBRUSHINDIRECT
                e.rec_u32(37, &[1]); // EMR_SELECTOBJECT
                let bb = e.path(&s);
                e.rec_i32(62, &devb(bb)); // EMR_FILLPATH
                e.rec_u32(37, &[0x8000_0000]); // WHITE_BRUSH
                e.rec_u32(40, &[1]); // EMR_DELETEOBJECT
            }
            VOp::Stroke { path, color, width, round_cap, dash } => {
                let s = segs(path);
                if s.is_empty() || color >> 24 == 0 {
                    continue;
                }
                // PS_GEOMETRIC | Endkappe | PS_JOIN_ROUND (0); gestrichelt: PS_DASH (Muster relativ zur Breite) –
                // PS_USERSTYLE wäre exakt, aber LibreOffice verliert dabei die Strichbreite
                let mut style = 0x0001_0000u32 | if *round_cap { 0 } else { 0x200 };
                if matches!(dash, Some((a, b)) if *a > 0.0 && *b > 0.0) {
                    style |= 1;
                }
                let pw = (width.max(0.0) * EMF_K).round().clamp(1.0, 1e9) as u32;
                let d = [2, 0, 0, 0, 0, style, pw, 0, colorref(on_white(*color, alpha(*color) * g)), 0, 0];
                e.rec_u32(95, &d); // EMR_EXTCREATEPEN
                e.rec_u32(37, &[2]);
                let bb = e.path(&s);
                e.rec_i32(64, &devb(bb)); // EMR_STROKEPATH
                e.rec_u32(37, &[0x8000_0007]); // BLACK_PEN
                e.rec_u32(40, &[2]);
            }
            VOp::Image(pm, m) => {
                if !transform_ok(m) {
                    continue;
                }
                let (iw, ih) = (pm.width(), pm.height());
                let (l, t, r, b) = hull(m, iw as f32, ih as f32);
                let q = |v: f32| (v * EMF_K).round().clamp(-1e9, 1e9) as i32;
                let (x, y, cx, cy) = (q(l), q(t), (q(r) - q(l)).max(1), (q(b) - q(t)).max(1));
                // 32-bit BI_RGB, Zeilen von unten nach oben, BGRX
                let rgbv = rgb_on_white(pm, g);
                let mut bits = Vec::with_capacity(iw as usize * ih as usize * 4);
                for row in rgbv.chunks_exact(iw as usize * 3).rev() {
                    for p in row.chunks_exact(3) {
                        bits.extend_from_slice(&[p[2], p[1], p[0], 255]);
                    }
                }
                let mut d: Vec<u8> = Vec::new();
                let mut p = |v: i32| d.extend_from_slice(&v.to_le_bytes());
                for v in devb([x, y, x + cx, y + cy]) {
                    p(v);
                }
                for v in [x, y, 0, 0, iw as i32, ih as i32, 80, 40, 120, bits.len() as i32, 0, 0x00CC_0020, cx, cy] {
                    p(v);
                }
                // BITMAPINFOHEADER
                for v in [40, iw as i32, ih as i32] {
                    p(v);
                }
                d.extend_from_slice(&1u16.to_le_bytes());
                d.extend_from_slice(&32u16.to_le_bytes());
                let mut p = |v: i32| d.extend_from_slice(&v.to_le_bytes());
                for v in [0, bits.len() as i32, 3780, 3780, 0, 0] {
                    p(v);
                }
                d.extend_from_slice(&bits);
                e.rec(81, &d); // EMR_STRETCHDIBITS
            }
            VOp::Save => {
                e.rec(33, &[]); // EMR_SAVEDC
                st.push((g, clipped));
            }
            VOp::Group(a) => {
                e.rec(33, &[]);
                st.push((g * *a as f32 / 255.0, clipped));
            }
            VOp::Restore => {
                if st.len() > 1 {
                    st.pop();
                    e.rec_i32(34, &[-1]); // EMR_RESTOREDC
                }
            }
            VOp::Clip(p) => {
                let mut s = segs(p);
                if s.is_empty() {
                    s = vec![Seg::M(0.0, 0.0), Seg::L(0.0, 0.0), Seg::Z];
                }
                e.path(&s);
                e.rec_u32(67, &[if clipped { 1 } else { 5 }]); // EMR_SELECTCLIPPATH RGN_AND / RGN_COPY
                if let Some(t) = st.last_mut() {
                    t.1 = true;
                }
            }
        }
    }
    for _ in 1..st.len() {
        e.rec_i32(34, &[-1]);
    }
    e.rec_u32(14, &[0, 16, 20]); // EMR_EOF
    let len = e.b.len() as u32;
    let n = e.n;
    e.b[48..52].copy_from_slice(&len.to_le_bytes());
    e.b[52..56].copy_from_slice(&n.to_le_bytes());
    e.b
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use tiny_skia::{Color, PathBuilder, Rect};

    pub(crate) fn sample_ops() -> Vec<VOp> {
        let r = |x, y, w, h| PathBuilder::from_rect(Rect::from_xywh(x, y, w, h).unwrap());
        let mut pb = PathBuilder::new();
        pb.move_to(10.0, 10.0);
        pb.quad_to(50.0, 0.0, 90.0, 10.0);
        pb.cubic_to(90.0, 40.0, 60.0, 60.0, 10.0, 80.0);
        let curve = pb.finish().unwrap();
        let mut pm = Pixmap::new(3, 2).unwrap();
        pm.fill(Color::from_rgba8(200, 0, 0, 128));
        vec![
            VOp::Fill(r(0.0, 0.0, 100.0, 80.0), 0xFFFFFFFF),
            VOp::Fill(r(5.0, 5.0, 20.5, 10.25), 0x80FF0000),
            VOp::Stroke { path: curve.clone(), color: 0xFF0000FF, width: 2.5, round_cap: true, dash: Some((4.0, 2.0)) },
            VOp::Group(128),
            VOp::Clip(r(20.0, 20.0, 40.0, 40.0)),
            VOp::Fill(r(0.0, 0.0, 100.0, 100.0), 0xFF00FF00),
            VOp::Save,
            VOp::Stroke { path: curve, color: 0xFF000000, width: 1.0, round_cap: false, dash: None },
            VOp::Restore,
            VOp::Restore,
            VOp::Image(pm, Transform::from_row(10.0, 0.0, 0.0, 10.0, 50.0, 50.0)),
        ]
    }

    #[test]
    fn svg_struktur() {
        let s = svg(&sample_ops(), 100.0, 80.0);
        let doc = roxmltree::Document::parse(&s).expect("SVG ist gültiges XML");
        let root = doc.root_element();
        assert_eq!(root.tag_name().name(), "svg");
        assert_eq!(root.attribute("viewBox"), Some("0 0 100 80"));
        let tag = |n: &str| doc.descendants().filter(|x| x.tag_name().name() == n).count();
        assert_eq!(tag("path"), 6);
        assert_eq!(tag("clipPath"), 1);
        assert_eq!(tag("image"), 1);
        assert!(doc.descendants().any(|x| x.attribute("opacity") == Some("0.502")));
        assert!(doc.descendants().any(|x| x.attribute("clip-path") == Some("url(#c1)")));
        assert!(doc.descendants().any(|x| x.attribute("fill-opacity") == Some("0.502")));
        assert!(doc.descendants().any(|x| x.attribute("stroke-dasharray") == Some("4 2")));
        let img = doc.descendants().find(|x| x.tag_name().name() == "image").unwrap();
        let href = img.attribute(("http://www.w3.org/1999/xlink", "href")).unwrap();
        assert!(href.starts_with("data:image/png;base64,iVBORw0KGgo"));
        assert_eq!(img.attribute("transform"), Some("matrix(10 0 0 10 50 50)"));
        assert!(s.contains("d=\"M5 5L25.5 5L25.5 15.25L5 15.25Z\""));
        // leer
        let e = svg(&[], 10.0, 10.0);
        roxmltree::Document::parse(&e).unwrap();
    }

    #[test]
    fn zahlen_und_base64() {
        assert_eq!(num(1.234, 2), "1.23");
        assert_eq!(num(-0.001, 2), "0");
        assert_eq!(num(3.0, 2), "3");
        assert_eq!(num(f32::NAN, 2), "0");
        assert_eq!(b64(b"Ma"), "TWE=");
        assert_eq!(b64(b"Man"), "TWFu");
    }

    #[test]
    fn nan_wird_uebersprungen() {
        let mut pb = PathBuilder::new();
        pb.move_to(0.0, 0.0);
        pb.line_to(5.0, 5.0);
        let p = pb.finish().unwrap();
        assert_eq!(segs(&p).len(), 2);
        let s = svg(&[VOp::Stroke { path: p, color: 0xFF000000, width: f32::NAN, round_cap: false, dash: None }], f32::NAN, 5.0);
        assert!(s.contains("stroke-width=\"0\""));
        roxmltree::Document::parse(&s).unwrap();
    }

    #[test]
    fn eps_struktur() {
        let s = eps(&sample_ops(), 100.4, 80.0);
        assert!(s.starts_with("%!PS-Adobe-3.0 EPSF-3.0\n"));
        assert!(s.lines().any(|l| l == "%%BoundingBox: 0 0 101 80"));
        assert!(s.contains("0 80 translate 1 -1 scale"));
        assert!(s.contains("colorimage"));
        assert!(s.contains("clip newpath"));
        assert_eq!(s.matches("gsave").count(), s.matches("grestore").count());
        assert!(eps(&[], 1.0, 1.0).contains("%%EOF"));
    }

    pub(crate) fn emf_records(b: &[u8]) -> Vec<(u32, usize)> {
        let rd = |o: usize| u32::from_le_bytes(b[o..o + 4].try_into().unwrap());
        let mut v = Vec::new();
        let mut o = 0;
        while o + 8 <= b.len() {
            let (t, n) = (rd(o), rd(o + 4) as usize);
            assert!(n >= 8 && n % 4 == 0 && o + n <= b.len(), "Record {t} bei {o} ungültig");
            v.push((t, o));
            o += n;
        }
        assert_eq!(o, b.len());
        v
    }

    #[test]
    fn emf_struktur() {
        for ops in [sample_ops(), Vec::new()] {
            let b = emf(&ops, 100.0, 80.0);
            let rd = |o: usize| u32::from_le_bytes(b[o..o + 4].try_into().unwrap());
            assert_eq!(rd(0), 1);
            assert_eq!(rd(40), 0x464D4520);
            assert_eq!(rd(48) as usize, b.len());
            let recs = emf_records(&b);
            assert_eq!(rd(52) as usize, recs.len());
            assert_eq!(recs.last().unwrap().0, 14);
            if !ops.is_empty() {
                let has = |t: u32| recs.iter().any(|r| r.0 == t);
                for t in [39, 62, 95, 64, 67, 81, 33, 34, 5, 6] {
                    assert!(has(t), "Record {t} fehlt");
                }
                let saves = recs.iter().filter(|r| r.0 == 33).count();
                assert_eq!(saves, recs.iter().filter(|r| r.0 == 34).count());
            }
        }
    }
}
