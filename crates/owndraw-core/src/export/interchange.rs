//! Austauschformate aus den Board-Objekten: Excalidraw, draw.io, InkML, JSON und eine einzelne HTML-Datei.

use crate::board::{Board, ExportFrame};
use crate::canvas::Canvas;
use crate::item::*;
use serde_json::{json, Map, Value};
use std::collections::HashMap;
use std::rc::Rc;
use tiny_skia::Pixmap;

const B64: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";

/// Base64 (Standard-Alphabet, mit Auffüllung).
pub(crate) fn b64(d: &[u8]) -> String {
    let mut o = String::with_capacity(d.len().div_ceil(3) * 4);
    for c in d.chunks(3) {
        let n = (c[0] as u32) << 16 | (*c.get(1).unwrap_or(&0) as u32) << 8 | *c.get(2).unwrap_or(&0) as u32;
        o.push(B64[(n >> 18) as usize & 63] as char);
        o.push(B64[(n >> 12) as usize & 63] as char);
        o.push(if c.len() > 1 { B64[(n >> 6) as usize & 63] as char } else { '=' });
        o.push(if c.len() > 2 { B64[n as usize & 63] as char } else { '=' });
    }
    o
}

fn r2(x: f32) -> Value {
    let v = if x.is_finite() { x as f64 } else { 0.0 };
    json!((v * 100.0).round() / 100.0)
}

fn rgb(c: u32) -> String {
    format!("#{:06x}", c & 0xFFFFFF)
}

/// Strichfarbe (Hex) und Deckkraft 0..100 eines Objekts.
fn stroke_col(it: &Item, hl: bool, dark: bool) -> (String, u32) {
    let c = ink(it.color, dark);
    let a = if hl { 0x66 } else { (c >> 24).max(1).min(255) };
    let a = if !hl && c >> 24 == 0 { 255 } else { a };
    (rgb(c), (a * 100 + 127) / 255)
}

/// Einzelnes Objekt als PNG (transparent) samt Hülle in Weltkoordinaten; k = Auflösungsfaktor (begrenzt auf 4096 px).
fn render_png(it: &Item, dark: bool, k: f32) -> Option<(Vec<u8>, crate::geom::RectF)> {
    let r = it.bounds();
    let (w, h) = (r.width().max(1.0), r.height().max(1.0));
    let k = k.min(4096.0 / w.max(h));
    let mut pm = Pixmap::new(((w * k).round() as u32).max(1), ((h * k).round() as u32).max(1))?;
    {
        let mut c = Canvas::new(&mut pm);
        c.scale(k, k);
        c.translate(-r.left, -r.top);
        it.draw(&mut c, dark);
    }
    Some((pm.encode_png().ok()?, r))
}

/// Geometrie einer Form in nativen Grundtypen.
enum Geo {
    /// Mittelpunkt, Breite, Höhe, Drehwinkel (rad).
    Rect(f32, f32, f32, f32, f32),
    Ellipse(f32, f32, f32, f32, f32),
    Diamond(f32, f32, f32, f32, f32),
    /// Punkte, Spitzen (Anfang, Ende), geschlossen, gestrichelt.
    Line(Vec<(f32, f32)>, (bool, bool), bool, bool),
    /// Keine native Entsprechung (Herz …): als Bild.
    Raster,
    Skip,
}

fn pts(q: &[f32]) -> Vec<(f32, f32)> {
    q.chunks_exact(2).map(|c| (c[0], c[1])).collect()
}

fn geo(g: &Shape) -> Geo {
    let q = &g.q;
    if q.len() < 4 {
        return Geo::Skip;
    }
    let d = |a: (f32, f32), b: (f32, f32)| ((a.0 - b.0).powi(2) + (a.1 - b.1).powi(2)).sqrt();
    if let Some((two, _, dashed)) = arrow_spec(g.kind) {
        return Geo::Line(pts(&q[..4]), (two, true), false, dashed);
    }
    match g.kind {
        LINE => Geo::Line(pts(&q[..4]), (false, false), false, false),
        ARROW_ENG => Geo::Line(pts(&q[..4]), (false, true), false, false),
        DIM => Geo::Line(pts(&q[..4]), (true, true), false, false),
        ARC => Geo::Line(pts(q), (false, false), false, false),
        ELLIPSE => {
            let (x0, x1, y0, y1) = (q[0].min(q[2]), q[0].max(q[2]), q[1].min(q[3]), q[1].max(q[3]));
            Geo::Ellipse((x0 + x1) / 2.0, (y0 + y1) / 2.0, x1 - x0, y1 - y0, g.ang)
        }
        DIAMOND if q.len() >= 8 => {
            let p = pts(q);
            Geo::Diamond((p[1].0 + p[3].0) / 2.0, (p[0].1 + p[2].1) / 2.0, d(p[1], p[3]), d(p[0], p[2]), (p[1].1 - p[3].1).atan2(p[1].0 - p[3].0))
        }
        POLY if q.len() == 8 => {
            let p = pts(q);
            let (e0, e1) = ((p[1].0 - p[0].0, p[1].1 - p[0].1), (p[2].0 - p[1].0, p[2].1 - p[1].1));
            let (l0, l1) = (d(p[0], p[1]), d(p[1], p[2]));
            let dot = e0.0 * e1.0 + e0.1 * e1.1;
            let miss = d(p[3], (p[0].0 + e1.0, p[0].1 + e1.1));
            if l0 > 0.0 && l1 > 0.0 && dot.abs() < 0.01 * l0 * l1 && miss < 0.5 {
                Geo::Rect((p[0].0 + p[2].0) / 2.0, (p[0].1 + p[2].1) / 2.0, l0, l1, e0.1.atan2(e0.0))
            } else {
                Geo::Line(p, (false, false), true, false)
            }
        }
        k if is_poly_kind(k) => Geo::Line(pts(q), (false, false), true, false),
        _ => Geo::Raster,
    }
}

/// Excalidraw-Szene (.excalidraw, JSON). Koordinaten relativ zum Ausschnitt.
pub fn excalidraw(b: &Board, items: &[Rc<Item>], f: &ExportFrame) -> String {
    let (ox, oy) = (f.rect.left, f.rect.top);
    let mut els: Vec<Value> = Vec::new();
    let mut files = Map::new();
    let mut n = 0usize;
    // Grundgerüst eines Elements; x/y/w/h bereits im Ausschnitt-Koordinatensystem
    let mut base = |typ: &str, it: &Item, x: f32, y: f32, w: f32, h: f32, ang: f32, stroke: &str, bg: &str, op: u32, sw: f32, style: &str| -> Map<String, Value> {
        n += 1;
        let gids: Vec<String> = if it.gid != 0 { vec![format!("g{}", it.gid)] } else { vec![] };
        let seed = (it.uid.wrapping_add(n as u64) & 0x7fff_ffff) as i64;
        let mut m = Map::new();
        for (k, v) in [
            ("id", json!(format!("od{n}"))),
            ("type", json!(typ)),
            ("x", r2(x)),
            ("y", r2(y)),
            ("width", r2(w)),
            ("height", r2(h)),
            ("angle", json!((ang as f64 * 1000.0).round() / 1000.0)),
            ("strokeColor", json!(stroke)),
            ("backgroundColor", json!(bg)),
            ("fillStyle", json!("solid")),
            ("strokeWidth", r2(sw.max(0.5))),
            ("strokeStyle", json!(style)),
            ("roughness", json!(0)),
            ("opacity", json!(op)),
            ("groupIds", json!(gids)),
            ("frameId", Value::Null),
            ("roundness", Value::Null),
            ("seed", json!(seed)),
            ("version", json!(1)),
            ("versionNonce", json!(seed ^ 0x5bd1e995)),
            ("isDeleted", json!(false)),
            ("boundElements", Value::Null),
            ("updated", json!(1)),
            ("link", Value::Null),
            ("locked", json!(false)),
        ] {
            m.insert(k.into(), v);
        }
        m
    };
    for it in items {
        let it: &Item = it;
        let dark = b.dark;
        let (sc, op) = stroke_col(it, matches!(&it.kind, Kind::Shape(g) if g.hl) || matches!(&it.kind, Kind::Stroke(s) if s.hl), dark);
        let bg = it.fill.map(|c| rgb(ink(c, dark))).unwrap_or_else(|| "transparent".into());
        let mut raster = false;
        match &it.kind {
            Kind::Stroke(s) if s.n() > 0 => {
                let p: Vec<(f32, f32, f32)> = s.p.chunks_exact(3).map(|c| (c[0] - ox, c[1] - oy, c[2])).collect();
                let (x0, y0) = (p[0].0, p[0].1);
                let (mnx, mxx) = p.iter().fold((f32::MAX, f32::MIN), |a, c| (a.0.min(c.0), a.1.max(c.0)));
                let (mny, mxy) = p.iter().fold((f32::MAX, f32::MIN), |a, c| (a.0.min(c.1), a.1.max(c.1)));
                let style = match s.ps {
                    PS_DASHED => "dashed",
                    PS_DOTTED => "dotted",
                    _ => "solid",
                };
                let mut m = base("freedraw", it, x0, y0, mxx - mnx, mxy - mny, 0.0, &sc, &bg, op, it.width, style);
                m.insert("points".into(), Value::Array(p.iter().map(|c| json!([r2(c.0 - x0), r2(c.1 - y0)])).collect()));
                m.insert("pressures".into(), Value::Array(p.iter().map(|c| r2(c.2)).collect()));
                m.insert("simulatePressure".into(), json!(!s.var));
                m.insert("lastCommittedPoint".into(), Value::Null);
                els.push(Value::Object(m));
            }
            Kind::Stroke(_) => {}
            Kind::Shape(g) => match geo(g) {
                Geo::Skip => {}
                Geo::Raster => raster = true,
                Geo::Rect(cx, cy, w, h, a) | Geo::Ellipse(cx, cy, w, h, a) | Geo::Diamond(cx, cy, w, h, a) => {
                    let typ = match geo(g) {
                        Geo::Ellipse(..) => "ellipse",
                        Geo::Diamond(..) => "diamond",
                        _ => "rectangle",
                    };
                    let m = base(typ, it, cx - w / 2.0 - ox, cy - h / 2.0 - oy, w, h, a, &sc, &bg, op, it.width, "solid");
                    els.push(Value::Object(m));
                }
                Geo::Line(p, (h0, h1), closed, dashed) => {
                    let mut p: Vec<(f32, f32)> = p.iter().map(|c| (c.0 - ox, c.1 - oy)).collect();
                    if closed {
                        p.push(p[0]);
                    }
                    let (x0, y0) = p[0];
                    let (mnx, mxx) = p.iter().fold((f32::MAX, f32::MIN), |a, c| (a.0.min(c.0), a.1.max(c.0)));
                    let (mny, mxy) = p.iter().fold((f32::MAX, f32::MIN), |a, c| (a.0.min(c.1), a.1.max(c.1)));
                    let arrow = h0 || h1;
                    let mut m = base(if arrow { "arrow" } else { "line" }, it, x0, y0, mxx - mnx, mxy - mny, 0.0, &sc, &bg, op, it.width, if dashed { "dashed" } else { "solid" });
                    m.insert("points".into(), Value::Array(p.iter().map(|c| json!([r2(c.0 - x0), r2(c.1 - y0)])).collect()));
                    m.insert("lastCommittedPoint".into(), Value::Null);
                    m.insert("startBinding".into(), Value::Null);
                    m.insert("endBinding".into(), Value::Null);
                    m.insert("startArrowhead".into(), if h0 { json!("arrow") } else { Value::Null });
                    m.insert("endArrowhead".into(), if h1 { json!("arrow") } else { Value::Null });
                    els.push(Value::Object(m));
                }
            },
            Kind::Note(nt) => {
                let size = nt.size.max(1.0);
                let (x, y) = (nt.x - ox, nt.y - oy);
                let text = |m_it: &Item, x: f32, y: f32, w: f32, h: f32, base: &mut dyn FnMut(&str, &Item, f32, f32, f32, f32, f32, &str, &str, u32, f32, &str) -> Map<String, Value>| {
                    let mut m = base("text", m_it, x, y, w, h, nt.ang, &sc, "transparent", op, 1.0, "solid");
                    for (k, v) in [
                        ("text", json!(nt.text)),
                        ("originalText", json!(nt.text)),
                        ("fontSize", r2(size)),
                        ("fontFamily", json!(2)),
                        ("textAlign", json!("left")),
                        ("verticalAlign", json!("top")),
                        ("baseline", r2(size)),
                        ("containerId", Value::Null),
                        ("lineHeight", json!(1.25)),
                        ("autoResize", json!(true)),
                    ] {
                        m.insert(k.into(), v);
                    }
                    m
                };
                if nt.bg != 0 {
                    let fillc = rgb(nt.bg);
                    let mut m = base("rectangle", it, x, y, nt.w, nt.h, nt.ang, &fillc, &fillc, 100, 1.0, "solid");
                    m.insert("groupIds".into(), json!([format!("n{}", it.uid)]));
                    els.push(Value::Object(m));
                    let pd = nt.pad();
                    let mut t = text(it, x + pd, y + pd, (nt.w - 2.0 * pd).max(1.0), (nt.h - 2.0 * pd).max(1.0), &mut base);
                    t.insert("groupIds".into(), json!([format!("n{}", it.uid)]));
                    els.push(Value::Object(t));
                } else {
                    els.push(Value::Object(text(it, x, y, nt.w, nt.h, &mut base)));
                }
            }
            _ => raster = true,
        }
        if raster {
            if let Some((png, r)) = render_png(it, dark, 2.0) {
                let fid = format!("{:016x}", it.uid);
                let mut m = base("image", it, r.left - ox, r.top - oy, r.width(), r.height(), 0.0, "transparent", "transparent", 100, 1.0, "solid");
                m.insert("fileId".into(), json!(fid));
                m.insert("status".into(), json!("saved"));
                m.insert("scale".into(), json!([1, 1]));
                m.insert("crop".into(), Value::Null);
                els.push(Value::Object(m));
                files.insert(fid.clone(), json!({"mimeType":"image/png","id":fid,"dataURL":format!("data:image/png;base64,{}", b64(&png)),"created":1}));
            }
        }
    }
    let scene = json!({
        "type": "excalidraw",
        "version": 2,
        "source": "OwnDraw",
        "elements": els,
        "appState": {"viewBackgroundColor": rgb(b.bg_color()), "gridSize": Value::Null},
        "files": files,
    });
    serde_json::to_string_pretty(&scene).unwrap_or_default()
}

/// Wert eines draw.io-Attributs (XML-escaped, Zeilenumbrüche als Zeichenverweis).
fn xa(s: &str) -> String {
    super::esc(s).replace('\n', "&#10;").replace('\r', "")
}

fn fl(x: f32) -> String {
    let s = format!("{:.2}", if x.is_finite() { x } else { 0.0 });
    s.trim_end_matches('0').trim_end_matches('.').to_string()
}

/// draw.io / diagrams.net (.drawio, unkomprimiertes mxfile-XML).
pub fn drawio(b: &Board, items: &[Rc<Item>], f: &ExportFrame, title: &str) -> String {
    let (ox, oy) = (f.rect.left, f.rect.top);
    let dark = b.dark;
    let mut cells = String::new();
    let mut n = 1usize;
    for it in items {
        let it: &Item = it;
        let hl = matches!(&it.kind, Kind::Shape(g) if g.hl);
        let (sc, op) = stroke_col(it, hl, dark);
        let fill = it.fill.map(|c| format!("fillColor={};", rgb(ink(c, dark)))).unwrap_or_else(|| "fillColor=none;".into());
        let opa = if op < 100 { format!("opacity={op};") } else { String::new() };
        let sw = fl(it.width.max(0.5));
        let mut raster = false;
        match &it.kind {
            Kind::Shape(g) => match geo(g) {
                Geo::Skip => {}
                Geo::Raster => raster = true,
                Geo::Rect(cx, cy, w, h, a) | Geo::Ellipse(cx, cy, w, h, a) | Geo::Diamond(cx, cy, w, h, a) => {
                    let shape = match geo(g) {
                        Geo::Ellipse(..) => "ellipse;",
                        Geo::Diamond(..) => "rhombus;",
                        _ => "rounded=0;",
                    };
                    n += 1;
                    cells.push_str(&format!(
                        "<mxCell id=\"c{n}\" value=\"\" style=\"{shape}whiteSpace=wrap;html=1;{fill}strokeColor={sc};strokeWidth={sw};rotation={};{opa}\" vertex=\"1\" parent=\"1\"><mxGeometry x=\"{}\" y=\"{}\" width=\"{}\" height=\"{}\" as=\"geometry\"/></mxCell>\n",
                        fl(a.to_degrees()),
                        fl(cx - w / 2.0 - ox),
                        fl(cy - h / 2.0 - oy),
                        fl(w),
                        fl(h)
                    ));
                }
                Geo::Line(p, (h0, h1), closed, dashed) => {
                    if closed || p.len() != 2 {
                        raster = true;
                    } else {
                        n += 1;
                        let filled = arrow_spec(g.kind).map(|s| s.1).unwrap_or(true);
                        let head = |on: bool| if on { "classic" } else { "none" };
                        cells.push_str(&format!(
                            "<mxCell id=\"c{n}\" value=\"\" style=\"endArrow={};startArrow={};endFill={};startFill={};html=1;strokeColor={sc};strokeWidth={sw};{}{opa}\" edge=\"1\" parent=\"1\"><mxGeometry relative=\"1\" as=\"geometry\"><mxPoint x=\"{}\" y=\"{}\" as=\"sourcePoint\"/><mxPoint x=\"{}\" y=\"{}\" as=\"targetPoint\"/></mxGeometry></mxCell>\n",
                            head(h1),
                            head(h0),
                            filled as u8,
                            filled as u8,
                            if dashed { "dashed=1;" } else { "" },
                            fl(p[0].0 - ox),
                            fl(p[0].1 - oy),
                            fl(p[1].0 - ox),
                            fl(p[1].1 - oy)
                        ));
                    }
                }
            },
            Kind::Note(nt) => {
                let bgc = if nt.bg != 0 { format!("fillColor={};strokeColor=none;", rgb(nt.bg)) } else { "fillColor=none;strokeColor=none;".into() };
                let pd = nt.pad();
                n += 1;
                cells.push_str(&format!(
                    "<mxCell id=\"c{n}\" value=\"{}\" style=\"text;whiteSpace=wrap;html=0;align=left;verticalAlign=top;spacing={};fontSize={};fontColor={sc};{bgc}rotation={};{opa}\" vertex=\"1\" parent=\"1\"><mxGeometry x=\"{}\" y=\"{}\" width=\"{}\" height=\"{}\" as=\"geometry\"/></mxCell>\n",
                    xa(&nt.text),
                    fl(pd),
                    fl(nt.size),
                    fl(nt.ang.to_degrees()),
                    fl(nt.x - ox),
                    fl(nt.y - oy),
                    fl(nt.w),
                    fl(nt.h)
                ));
            }
            Kind::Stroke(s) if s.n() == 0 => {}
            _ => raster = true,
        }
        if raster {
            if let Some((png, r)) = render_png(it, dark, 2.0) {
                n += 1;
                cells.push_str(&format!(
                    "<mxCell id=\"c{n}\" value=\"\" style=\"shape=image;verticalLabelPosition=bottom;verticalAlign=top;imageAspect=0;image=data:image/png,{};\" vertex=\"1\" parent=\"1\"><mxGeometry x=\"{}\" y=\"{}\" width=\"{}\" height=\"{}\" as=\"geometry\"/></mxCell>\n",
                    b64(&png),
                    fl(r.left - ox),
                    fl(r.top - oy),
                    fl(r.width()),
                    fl(r.height())
                ));
            }
        }
    }
    format!(
        "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n<mxfile host=\"OwnDraw\" version=\"1\">\n<diagram id=\"owndraw\" name=\"{}\">\n<mxGraphModel dx=\"0\" dy=\"0\" grid=\"0\" gridSize=\"10\" guides=\"1\" tooltips=\"1\" connect=\"1\" arrows=\"1\" fold=\"1\" page=\"0\" pageScale=\"1\" pageWidth=\"{}\" pageHeight=\"{}\" background=\"{}\" math=\"0\" shadow=\"0\">\n<root>\n<mxCell id=\"0\"/>\n<mxCell id=\"1\" parent=\"0\"/>\n{cells}</root>\n</mxGraphModel>\n</diagram>\n</mxfile>\n",
        xa(title),
        fl(f.rect.width()),
        fl(f.rect.height()),
        rgb(b.bg_color())
    )
}

/// Punkte (x, y, Druck) eines Objekts für InkML; None für Nicht-Linienobjekte.
fn ink_points(it: &Item) -> Option<Vec<(f32, f32, f32)>> {
    match &it.kind {
        Kind::Stroke(s) if s.n() > 0 => Some(s.p.chunks_exact(3).map(|c| (c[0], c[1], c[2])).collect()),
        Kind::Shape(g) => match geo(g) {
            Geo::Line(p, _, closed, _) => {
                let mut v: Vec<(f32, f32, f32)> = p.iter().map(|c| (c.0, c.1, 1.0)).collect();
                if closed {
                    v.push(v[0]);
                }
                Some(v)
            }
            Geo::Rect(cx, cy, w, h, a) => {
                let c = pts(&[-w / 2.0, -h / 2.0, w / 2.0, -h / 2.0, w / 2.0, h / 2.0, -w / 2.0, h / 2.0, -w / 2.0, -h / 2.0]);
                Some(c.iter().map(|&(x, y)| { let (rx, ry) = rot_pt(x, y, 0.0, 0.0, a); (cx + rx, cy + ry, 1.0) }).collect())
            }
            Geo::Diamond(cx, cy, w, h, a) => {
                let c = pts(&[0.0, -h / 2.0, w / 2.0, 0.0, 0.0, h / 2.0, -w / 2.0, 0.0, 0.0, -h / 2.0]);
                Some(c.iter().map(|&(x, y)| { let (rx, ry) = rot_pt(x, y, 0.0, 0.0, a); (cx + rx, cy + ry, 1.0) }).collect())
            }
            Geo::Ellipse(cx, cy, w, h, a) => Some(
                (0..=48)
                    .map(|i| {
                        let t = i as f32 / 48.0 * std::f32::consts::TAU;
                        let (rx, ry) = rot_pt(w / 2.0 * t.cos(), h / 2.0 * t.sin(), 0.0, 0.0, a);
                        (cx + rx, cy + ry, 1.0)
                    })
                    .collect(),
            ),
            _ => None,
        },
        _ => None,
    }
}

/// InkML (W3C) mit allen Freihand-Strichen (Punkte, Farbe, Breite).
pub fn inkml(b: &Board, items: &[Rc<Item>], f: &ExportFrame) -> String {
    let (ox, oy) = (f.rect.left, f.rect.top);
    let mut brushes: Vec<String> = Vec::new();
    let mut bidx: HashMap<String, usize> = HashMap::new();
    // Ebenen in Reihenfolge des ersten Auftretens
    let mut groups: Vec<(i32, Vec<String>)> = Vec::new();
    for it in items {
        let Some(p) = ink_points(it) else { continue };
        let hl = matches!(&it.kind, Kind::Stroke(s) if s.hl) || matches!(&it.kind, Kind::Shape(g) if g.hl);
        let (col, op) = stroke_col(it, hl, b.dark);
        let key = format!("{col}|{}|{op}", fl(it.width));
        let i = *bidx.entry(key.clone()).or_insert_with(|| {
            brushes.push(format!(
                "<brush xml:id=\"b{}\"><brushProperty name=\"color\" value=\"{col}\"/><brushProperty name=\"width\" value=\"{w}\" units=\"cm\"/><brushProperty name=\"height\" value=\"{w}\"/><brushProperty name=\"transparency\" value=\"{}\"/><brushProperty name=\"tip\" value=\"ellipse\"/></brush>",
                brushes.len(),
                255 - (op * 255 + 50) / 100,
                w = fl(it.width)
            ));
            brushes.len() - 1
        });
        let t: Vec<String> = p.iter().map(|c| format!("{:.2} {:.2} {:.3}", c.0 - ox, c.1 - oy, c.2.clamp(0.0, 1.0))).collect();
        let tr = format!("<trace brushRef=\"#b{i}\" contextRef=\"#ctx\">{}</trace>", t.join(", "));
        match groups.iter_mut().find(|g| g.0 == it.layer) {
            Some(g) => g.1.push(tr),
            None => groups.push((it.layer, vec![tr])),
        }
    }
    let mut s = String::from("<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n<ink xmlns=\"http://www.w3.org/2003/InkML\">\n<definitions>\n");
    s.push_str("<context xml:id=\"ctx\"><inkSource xml:id=\"src\"><traceFormat><channel name=\"X\" type=\"decimal\"/><channel name=\"Y\" type=\"decimal\"/><channel name=\"F\" type=\"decimal\" min=\"0\" max=\"1\"/></traceFormat></inkSource></context>\n");
    for br in &brushes {
        s.push_str(br);
        s.push('\n');
    }
    s.push_str("</definitions>\n");
    for (lid, trs) in &groups {
        let name = b.layers.iter().find(|l| l.id == *lid).map(|l| l.name.clone()).unwrap_or_default();
        s.push_str(&format!("<traceGroup><annotation type=\"layer\">{}</annotation>\n", xa(&name)));
        for t in trs {
            s.push_str(t);
            s.push('\n');
        }
        s.push_str("</traceGroup>\n");
    }
    s.push_str("</ink>\n");
    s
}

/// Rohdaten als JSON (Board-Objekte im .owndraw-Itemformat plus Ausschnitt, Ebenen und Seiten).
pub fn json(b: &Board, items: &[Rc<Item>], f: &ExportFrame, title: &str) -> String {
    let layers: Vec<Value> = b.layers.iter().map(|l| json!({"id": l.id, "name": l.name, "visible": l.visible, "locked": l.locked, "opacity": r2(l.opacity)})).collect();
    let pages = b.pages().map(|ps| {
        let rects = ps.rects();
        Value::Array(
            ps.pages
                .iter()
                .enumerate()
                .map(|(i, p)| {
                    let r = rects.get(i).copied();
                    json!({"id": p.id, "w": r2(p.w), "h": r2(p.h), "x": r2(r.map_or(0.0, |r| r.left)), "y": r2(r.map_or(0.0, |r| r.top))})
                })
                .collect(),
        )
    });
    let v = json!({
        "format": "owndraw-export",
        "version": 1,
        "title": title,
        "frame": {"x": r2(f.rect.left), "y": r2(f.rect.top), "w": r2(f.rect.width()), "h": r2(f.rect.height())},
        "page": f.page,
        "background": rgb(b.bg_color()),
        "dark": b.dark,
        "layers": layers,
        "pages": pages.unwrap_or(Value::Null),
        "items": items.iter().map(|it| it.json()).collect::<Vec<Value>>(),
    });
    serde_json::to_string_pretty(&v).unwrap_or_default()
}

/// Einzelne HTML-Datei: Website-Export (Ebenen, Animationen) mit eingebetteten Bildern (data-URIs), ohne ZIP.
pub fn html_single(b: &Board, title: &str) -> Option<String> {
    super::html_single_impl(b, title)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::geom::RectF;

    fn board() -> Board {
        let mut b = Board::new(1.0);
        let mut s = Item::stroke(0xFFCC0000, 3.0, false, false);
        if let Kind::Stroke(st) = &mut s.kind {
            st.p = vec![10.0, 10.0, 0.5, 20.0, 15.0, 0.6, 30.0, 12.0, 0.7];
        }
        b.items.push(Rc::new(s));
        let mut r = Item::shape(POLY, vec![50.0, 50.0, 150.0, 50.0, 150.0, 120.0, 50.0, 120.0], 0xFF000000, 2.0);
        r.fill = Some(0xFFFFFF00);
        b.items.push(Rc::new(r));
        b.items.push(Rc::new(Item::shape(ARROW, vec![0.0, 200.0, 100.0, 220.0], 0xFF0000FF, 2.0)));
        b.items.push(Rc::new(Item::note("Hallo <Welt> & Co", 0xFF111111, 20.0, 200.0, 20.0, 150.0, 0.0, 0)));
        let mut png = Pixmap::new(8, 8).unwrap();
        png.fill(tiny_skia::Color::from_rgba8(0, 200, 0, 255));
        put_mem_image("t.png", png.encode_png().unwrap());
        b.items.push(Rc::new(Item::new(Kind::Img(Img { x: 200.0, y: 200.0, w: 40.0, h: 40.0, ang: 0.0, file: "t.png".into(), cr: None }))));
        b
    }

    fn frame() -> ExportFrame {
        ExportFrame { rect: RectF::new(-10.0, -10.0, 300.0, 300.0), page: None }
    }

    #[test]
    fn excalidraw_szene() {
        let b = board();
        let v: Value = serde_json::from_str(&excalidraw(&b, &b.items, &frame())).unwrap();
        assert_eq!(v["type"], "excalidraw");
        let t: Vec<&str> = v["elements"].as_array().unwrap().iter().map(|e| e["type"].as_str().unwrap()).collect();
        assert_eq!(t, ["freedraw", "rectangle", "arrow", "text", "image"]);
        assert_eq!(v["elements"][0]["x"], 20.0);
        assert_eq!(v["elements"][1]["backgroundColor"], "#ffff00");
        let fid = v["elements"][4]["fileId"].as_str().unwrap();
        assert!(v["files"][fid]["dataURL"].as_str().unwrap().starts_with("data:image/png;base64,"));
    }

    #[test]
    fn drawio_xml() {
        let b = board();
        let x = drawio(&b, &b.items, &frame(), "T & <1>");
        let d = roxmltree::Document::parse(&x).unwrap();
        assert_eq!(d.descendants().filter(|n| n.has_tag_name("mxCell")).count(), 2 + 5);
        assert!(d.descendants().any(|n| n.attribute("edge") == Some("1")));
        assert!(d.descendants().any(|n| n.attribute("style").is_some_and(|s| s.contains("image=data:image/png,") && !s.contains("base64"))));
        assert!(d.descendants().any(|n| n.attribute("value") == Some("Hallo <Welt> & Co")));
        assert!(d.descendants().any(|n| n.attribute("name") == Some("T & <1>")));
    }

    #[test]
    fn inkml_traces() {
        let b = board();
        let x = inkml(&b, &b.items, &frame());
        let d = roxmltree::Document::parse(&x).unwrap();
        assert!(d.descendants().any(|n| n.has_tag_name("brush")));
        // Strich, Rechteck, Pfeil
        assert_eq!(d.descendants().filter(|n| n.has_tag_name("trace")).count(), 3);
    }

    #[test]
    fn json_export() {
        let b = board();
        let v: Value = serde_json::from_str(&json(&b, &b.items, &frame(), "x")).unwrap();
        assert_eq!(v["format"], "owndraw-export");
        assert_eq!(v["items"].as_array().unwrap().len(), 5);
        assert_eq!(v["frame"]["w"], 310.0);
    }

    #[test]
    fn html_einzeln() {
        let b = board();
        let h = html_single(&b, "Test").unwrap();
        assert!(h.contains("data:image/png;base64,"));
        assert!(!h.contains("ebenen/"));
        assert!(html_single(&Board::new(1.0), "leer").is_none());
        assert_eq!(b64(b"Man"), "TWFu");
    }
}
