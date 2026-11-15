//! Xournal++ (`.xopp`, gzip-XML) Import/Export (WP8), Abbildung siehe docs/konzept-features.md Abschnitt 4.
//!
//! Import: jede `<page>` wird eine Seite im Seitenmodus (1 pt = 1 Welteinheit), Koordinaten werden um die
//! Seitenlage aus [`PageSetup::rects`] verschoben. Ebene k jeder Seite wird OwnDraw-Ebene k (id k + 1).
//! Was OwnDraw nicht abbilden kann, bleibt erhalten und wird beim Export zurückgeschrieben:
//! - Seite: Hintergrund-Attribute (Farbe, Stil, `config`, PDF-/Bild-Hintergrund), Ebenenzahl und unbekannte
//!   Elemente (roh) in `Page.extra["xopp"]`;
//! - Objekt: unbekannte Attribute (`style`, `capStyle`, `fill`, `ts`, `fn`, Schriftart …) als Liste `"xa"` im
//!   Item-JSON, LaTeX-Bilder roh als `"xraw"` (mit Lage `"xg"`, nur unverändert zurückgeschrieben);
//! - Dokument: Titel und unbekannte Elemente unter `<xournal>` in `Meta.extra["xopp"]`.
//!
//! Druck: Xournal++ speichert Grundbreite B und je Strecke i die Breite w_i (≈ B × Druck). OwnDraw zeichnet
//! einen Druck-Strich mit Breite W × (0,35 + 1,3 p). Abbildung W = B / 2, p_i = (2 w_i / B − 0,35) / 1,3
//! (≥ 0): so stimmt die gezeichnete Breite überein und der Rückweg ist exakt.

use crate::doc::{DocError, Document, Result};
use crate::geom::RectF;
use crate::item::{Item, Kind};
use crate::layer::Layer;
use crate::pages::{BoardMode, Page, PageBg, PageFormat, PageSetup};
use serde_json::{json, Map, Value};
use std::fmt::Write as _;
use std::io::{Cursor, Read, Write};

/// Export-Optionen.
#[derive(Clone, Copy, Debug, Default)]
pub struct XoppExport {
    /// Unendliches Board: in A4-Seiten kacheln statt einer großen Seite.
    pub tile_a4: bool,
}

/// Rand um den Inhalt beim Export eines unendlichen Boards (pt).
const MARGIN: f32 = 20.0;
/// Größte entpackte XML-Größe.
const MAX_XML: u64 = 400 * 1024 * 1024;

fn fmt_err<E: std::fmt::Display>(what: &'static str) -> impl FnOnce(E) -> DocError {
    move |e| DocError::Format(format!("{what}: {e}"))
}

// ---------------------------------------------------------------- Import

/// Liest eine `.xopp`/`.xoj`-Datei (gzip-XML, unkomprimiertes XML oder Xournal++-ZIP mit `content.xml`).
pub fn import_xopp(bytes: &[u8]) -> Result<Document> {
    let xml = unpack(bytes)?;
    let x = roxmltree::Document::parse(&xml).map_err(fmt_err("XML"))?;
    let root = x.root_element();
    if root.tag_name().name() != "xournal" {
        return Err(DocError::Format("kein Xournal-Dokument".into()));
    }
    let mut title = String::new();
    let mut thumb = None;
    let mut top_raw = Vec::new();
    let mut pages: Vec<Page> = Vec::new();
    let mut page_nodes = Vec::new();
    for n in root.children().filter(|n| n.is_element()) {
        match n.tag_name().name() {
            "title" => title = node_text(&n),
            "preview" => thumb = b64_decode(&node_text(&n)).filter(|b| b.starts_with(b"\x89PNG")),
            "page" => page_nodes.push(n),
            _ => top_raw.push(json!(&xml[n.range()])),
        }
    }
    // Seiten zuerst anlegen, damit die Lage jeder Seite feststeht.
    for n in &page_nodes {
        let w = attr_f(n, "width")
            .filter(|v| *v > 0.0)
            .unwrap_or(PageFormat::A4.size().0);
        let h = attr_f(n, "height")
            .filter(|v| *v > 0.0)
            .unwrap_or(PageFormat::A4.size().1);
        let mut pg = Page::new(w, h, PageBg::Plain);
        if let Some(bg) = n.children().find(|c| c.has_tag_name("background")) {
            let attrs = attr_list(&bg);
            if bg.attribute("type").unwrap_or("solid") == "solid" {
                pg.bg = PageBg::from_name(bg.attribute("style").unwrap_or("plain"));
            }
            pg.extra = json!({"xopp": {"bg": attrs}});
        }
        pages.push(pg);
    }
    if pages.is_empty() {
        let (w, h) = PageFormat::A4.size();
        pages.push(Page::new(w, h, PageBg::Plain));
    }
    let first = pages[0].clone();
    let mut setup = PageSetup {
        w: first.w,
        h: first.h,
        bg: first.bg,
        pages,
    };
    let rects = setup.rects();

    let mut doc = Document::new(&doc_name(&title), BoardMode::Infinite);
    doc.thumb = thumb;
    let mut items: Vec<Value> = Vec::new();
    let mut names: Vec<Option<String>> = Vec::new();
    for (pi, n) in page_nodes.iter().enumerate() {
        let (ox, oy) = (rects[pi].left, rects[pi].top);
        let mut raw = Vec::new();
        let mut nlayers = 0usize;
        for c in n.children().filter(|c| c.is_element()) {
            match c.tag_name().name() {
                "background" => {}
                "layer" => {
                    let k = nlayers;
                    nlayers += 1;
                    if names.len() <= k {
                        names.resize(k + 1, None);
                    }
                    if names[k].is_none() {
                        names[k] = c.attribute("name").map(String::from);
                    }
                    for e in c.children().filter(|e| e.is_element()) {
                        match import_elem(&e, &xml, ox, oy, k as i32 + 1, &mut doc) {
                            Some(v) => items.push(v),
                            None => raw.push(json!({"l": k, "x": &xml[e.range()]})),
                        }
                    }
                }
                _ => raw.push(json!({"l": -1, "x": &xml[c.range()]})),
            }
        }
        let ex = &mut setup.pages[pi].extra;
        if !ex.is_object() {
            *ex = json!({"xopp": {}});
        }
        ex["xopp"]["layers"] = json!(nlayers);
        if !raw.is_empty() {
            ex["xopp"]["raw"] = Value::Array(raw);
        }
    }
    if names.is_empty() {
        names.push(None);
    }
    let layers: Vec<Value> = names
        .iter()
        .enumerate()
        .map(|(k, nm)| {
            Layer::new(
                k as i32 + 1,
                nm.as_deref().unwrap_or(&default_layer_name(k)),
            )
            .json()
        })
        .collect();
    doc.board["layers"] = Value::Array(layers);
    doc.board["items"] = Value::Array(items);
    doc.board["active"] = json!(0);
    let mut xo = Map::new();
    xo.insert("title".into(), json!(title));
    if let Some(c) = root.attribute("creator") {
        xo.insert("creator".into(), json!(c));
    }
    if !top_raw.is_empty() {
        xo.insert("raw".into(), Value::Array(top_raw));
    }
    doc.meta.extra.insert("xopp".into(), Value::Object(xo));
    doc.mode = BoardMode::Pages(setup);
    Ok(doc)
}

/// Entpackt gzip bzw. liest `content.xml` aus einem ZIP; sonst wird das XML direkt genommen.
fn unpack(bytes: &[u8]) -> Result<String> {
    let raw = if bytes.starts_with(&[0x1f, 0x8b]) {
        let mut v = Vec::new();
        flate2::read::MultiGzDecoder::new(bytes)
            .take(MAX_XML + 1)
            .read_to_end(&mut v)
            .map_err(fmt_err("gzip"))?;
        v
    } else if bytes.starts_with(b"PK\x03\x04") {
        let mut z = zip::ZipArchive::new(Cursor::new(bytes)).map_err(fmt_err("ZIP"))?;
        let e = z.by_name("content.xml").map_err(fmt_err("content.xml"))?;
        let mut v = Vec::new();
        e.take(MAX_XML + 1).read_to_end(&mut v)?;
        v
    } else {
        bytes.to_vec()
    };
    if raw.len() as u64 > MAX_XML {
        return Err(DocError::Format("Datei zu groß".into()));
    }
    let s = String::from_utf8(raw).map_err(fmt_err("kein UTF-8"))?;
    Ok(s.strip_prefix('\u{feff}').map(String::from).unwrap_or(s))
}

fn doc_name(title: &str) -> String {
    let t = title.trim();
    if t.is_empty() || t.starts_with("Xournal") {
        "Xournal++-Dokument".into()
    } else {
        t.chars().take(80).collect()
    }
}

fn default_layer_name(k: usize) -> String {
    format!("Ebene {}", k + 1)
}

fn node_text(n: &roxmltree::Node) -> String {
    n.descendants()
        .filter(|d| d.is_text())
        .filter_map(|d| d.text())
        .collect()
}

fn attr_f(n: &roxmltree::Node, k: &str) -> Option<f32> {
    n.attribute(k)?
        .trim()
        .parse::<f32>()
        .ok()
        .filter(|v| v.is_finite())
}

/// Alle Attribute als Liste `[[name, wert], …]` (Reihenfolge bleibt erhalten).
fn attr_list(n: &roxmltree::Node) -> Value {
    Value::Array(
        n.attributes()
            .map(|a| json!([a.name(), a.value()]))
            .collect(),
    )
}

/// Attribute außer `skip` als Liste `[[name, wert], …]`.
fn extra_attrs(n: &roxmltree::Node, skip: &[&str]) -> Value {
    Value::Array(
        n.attributes()
            .filter(|a| !skip.contains(&a.name()))
            .map(|a| json!([a.name(), a.value()]))
            .collect(),
    )
}

/// Ein Element einer Ebene als Item-JSON (Weltkoordinaten); None → roh erhalten.
fn import_elem(
    e: &roxmltree::Node,
    xml: &str,
    ox: f32,
    oy: f32,
    layer: i32,
    doc: &mut Document,
) -> Option<Value> {
    let mut it = match e.tag_name().name() {
        "stroke" => {
            let tool = e.attribute("tool").unwrap_or("pen");
            let hl = tool == "highlighter";
            let color = parse_color(e.attribute("color").unwrap_or("black"))?;
            let ws: Vec<f32> = e
                .attribute("width")
                .unwrap_or("1")
                .split_whitespace()
                .filter_map(|s| s.parse::<f32>().ok())
                .collect();
            let base = ws
                .first()
                .copied()
                .filter(|w| *w > 0.0 && w.is_finite())
                .unwrap_or(1.0);
            let xy: Vec<f32> = node_text(e)
                .split_whitespace()
                .map(|s| s.parse::<f32>())
                .collect::<std::result::Result<_, _>>()
                .ok()?;
            let n = xy.len() / 2;
            if n == 0 || xy.iter().any(|v| !v.is_finite()) {
                return None;
            }
            let var = ws.len() > 1;
            let width = if var { base / 2.0 } else { base };
            let mut p = Vec::with_capacity(n * 3);
            for i in 0..n {
                let pr = if var {
                    // Breite der Strecke i (letzter Punkt: letzte Breite)
                    let w = ws[1 + i.min(ws.len() - 2)];
                    ((2.0 * w / base - 0.35) / 1.3).max(0.0)
                } else {
                    0.5
                };
                p.extend_from_slice(&[xy[i * 2] + ox, xy[i * 2 + 1] + oy, pr]);
            }
            let mut it = Item::stroke(color, width, hl, var);
            if let Kind::Stroke(s) = &mut it.kind {
                s.p = p;
                // Xournal++ zeichnet gerade Strecken zwischen den Punkten
                s.sharp = true;
            }
            let mut skip = vec!["color", "width"];
            if matches!(tool, "pen" | "highlighter") {
                skip.push("tool");
            }
            (it, extra_attrs(e, &skip))
        }
        "text" => {
            let color = parse_color(e.attribute("color").unwrap_or("black"))?;
            let size = attr_f(e, "size").filter(|s| *s > 0.0).unwrap_or(12.0);
            let (x, y) = (attr_f(e, "x")?, attr_f(e, "y")?);
            let it = Item::note(&node_text(e), color, size, x + ox, y + oy, 0.0, 0.0, 0);
            (it, extra_attrs(e, &["size", "x", "y", "color"]))
        }
        "image" | "teximage" => {
            let (l, t, r, b) = (
                attr_f(e, "left")?,
                attr_f(e, "top")?,
                attr_f(e, "right")?,
                attr_f(e, "bottom")?,
            );
            let data = b64_decode(&node_text(e))?;
            let ext = image::guess_format(&data)
                .ok()
                .and_then(|f| f.extensions_str().first().copied())?;
            let file = doc.images.add(data, ext);
            let mut it = Item::new(Kind::Img(crate::item::Img {
                x: l + ox,
                y: t + oy,
                w: r - l,
                h: b - t,
                ang: 0.0,
                file,
            }));
            it.color = crate::item::INK;
            let mut v = it.json();
            v["ly"] = json!(layer);
            if e.has_tag_name("teximage") {
                v["xraw"] = json!(&xml[e.range()]);
                v["xg"] = json!([l, t, r - l, b - t]);
            }
            return Some(v);
        }
        _ => return None,
    };
    it.0.layer = layer;
    let mut v = it.0.json();
    if it.1.as_array().is_some_and(|a| !a.is_empty()) {
        v["xa"] = it.1;
    }
    Some(v)
}

/// Xournal-Farbe (`#RRGGBBAA`, `#RRGGBB` oder Name) → ARGB.
pub fn parse_color(s: &str) -> Option<u32> {
    let s = s.trim();
    if let Some(h) = s.strip_prefix('#') {
        let v = u32::from_str_radix(h, 16).ok()?;
        return match h.len() {
            8 => Some(v.rotate_right(8)),
            6 => Some(0xFF00_0000 | v),
            _ => None,
        };
    }
    let rgb = match s {
        "black" => 0x000000,
        "blue" => 0x3333cc,
        "red" => 0xff0000,
        "green" => 0x008000,
        "gray" => 0x808080,
        "lightblue" => 0x00c0ff,
        "lightgreen" => 0x00ff00,
        "magenta" => 0xff00ff,
        "orange" => 0xff8000,
        "yellow" => 0xffff00,
        "white" => 0xffffff,
        _ => return None,
    };
    Some(0xFF00_0000 | rgb)
}

/// ARGB → `#rrggbbaa`.
pub fn fmt_color(c: u32) -> String {
    format!("#{:08x}", c.rotate_left(8))
}

// ---------------------------------------------------------------- Export

/// Ein Objekt beim Export: geparstes Item + ursprüngliches JSON (Zusatzfelder).
struct Obj<'a> {
    it: Item,
    raw: &'a Value,
}

/// Seite beim Export: Größe, Lage in der Welt, Vorlage aus dem Dokument (falls Seitenmodus).
struct OutPage<'a> {
    w: f32,
    h: f32,
    ox: f32,
    oy: f32,
    page: Option<&'a Page>,
    /// Inhalt je Ebene.
    layers: Vec<String>,
}

/// Schreibt das Dokument als gzip-komprimiertes Xournal++-XML (fileversion 4).
pub fn export_xopp(doc: &Document, opt: XoppExport) -> Result<Vec<u8>> {
    let layers: Vec<Layer> = doc
        .board
        .get("layers")
        .and_then(Value::as_array)
        .map(|a| {
            a.iter()
                .filter(|v| v.is_object())
                .map(Layer::from_json)
                .collect()
        })
        .unwrap_or_default();
    let layer_idx = |id: i32| layers.iter().position(|l| l.id == id).unwrap_or(0);
    let objs: Vec<Obj> = doc
        .board
        .get("items")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(|v| Item::from_json(v).map(|it| Obj { it, raw: v }))
        .collect();

    // Seiten bestimmen und Objekte zuordnen
    let (mut pages, assign): (Vec<OutPage>, Vec<usize>) = match &doc.mode {
        BoardMode::Pages(s) if !s.pages.is_empty() => {
            let rs = s.rects();
            let pages = s
                .pages
                .iter()
                .zip(&rs)
                .map(|(p, r)| OutPage {
                    w: p.w,
                    h: p.h,
                    ox: r.left,
                    oy: r.top,
                    page: Some(p),
                    layers: Vec::new(),
                })
                .collect();
            (
                pages,
                objs.iter().map(|o| s.page_of(&o.it).unwrap_or(0)).collect(),
            )
        }
        _ => infinite_pages(&objs, opt),
    };
    for (o, &pi) in objs.iter().zip(&assign) {
        let k = layer_idx(o.it.layer);
        let pg = &mut pages[pi];
        if pg.layers.len() <= k {
            pg.layers.resize(k + 1, String::new());
        }
        let (ox, oy) = (pg.ox, pg.oy);
        emit_item(o, ox, oy, doc, &mut pg.layers[k]);
    }

    let mut out = String::with_capacity(1 << 16);
    out.push_str("<?xml version=\"1.0\" standalone=\"no\"?>\n");
    let _ = writeln!(
        out,
        "<xournal creator=\"OwnDraw {}\" fileversion=\"4\">",
        env!("CARGO_PKG_VERSION")
    );
    let xo = doc.meta.extra.get("xopp");
    let title = xo
        .and_then(|x| x.get("title"))
        .and_then(Value::as_str)
        .filter(|t| !t.is_empty())
        .unwrap_or(&doc.meta.name);
    let _ = writeln!(out, "<title>{}</title>", esc(title, false));
    if let Some(t) = &doc.thumb {
        let _ = writeln!(out, "<preview>{}</preview>", b64_encode(t));
    }
    for r in xo
        .and_then(|x| x.get("raw"))
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(Value::as_str)
    {
        out.push_str(r);
        out.push('\n');
    }
    for pg in &pages {
        write_page(&mut out, pg, &layers);
    }
    out.push_str("</xournal>\n");

    let mut gz = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::default());
    gz.write_all(out.as_bytes())?;
    Ok(gz.finish()?)
}

/// Unendliches Board: eine große Seite um den Inhalt (Rand 20 pt) oder A4-Kacheln.
fn infinite_pages<'a>(objs: &[Obj], opt: XoppExport) -> (Vec<OutPage<'a>>, Vec<usize>) {
    let (aw, ah) = PageFormat::A4.size();
    let mut b: Option<RectF> = None;
    for o in objs {
        let r = o.it.bounds();
        match &mut b {
            Some(b) => b.union(&r),
            None => b = Some(r),
        }
    }
    let Some(b) = b else {
        return (
            vec![OutPage {
                w: aw,
                h: ah,
                ox: 0.0,
                oy: 0.0,
                page: None,
                layers: Vec::new(),
            }],
            Vec::new(),
        );
    };
    let (x0, y0) = (b.left - MARGIN, b.top - MARGIN);
    let (w, h) = (b.width() + 2.0 * MARGIN, b.height() + 2.0 * MARGIN);
    if !opt.tile_a4 {
        let pg = OutPage {
            w,
            h,
            ox: x0,
            oy: y0,
            page: None,
            layers: Vec::new(),
        };
        return (vec![pg], vec![0; objs.len()]);
    }
    let cols = ((w / aw).ceil() as usize).max(1);
    let rows = ((h / ah).ceil() as usize).max(1);
    let pages = (0..rows * cols)
        .map(|i| OutPage {
            w: aw,
            h: ah,
            ox: x0 + (i % cols) as f32 * aw,
            oy: y0 + (i / cols) as f32 * ah,
            page: None,
            layers: Vec::new(),
        })
        .collect();
    let assign = objs
        .iter()
        .map(|o| {
            let r = o.it.bounds();
            let c = (((r.center_x() - x0) / aw).floor().max(0.0) as usize).min(cols - 1);
            let rr = (((r.center_y() - y0) / ah).floor().max(0.0) as usize).min(rows - 1);
            rr * cols + c
        })
        .collect();
    (pages, assign)
}

fn write_page(out: &mut String, pg: &OutPage, layers: &[Layer]) {
    let xo = pg.page.and_then(|p| p.extra.get("xopp"));
    let _ = writeln!(
        out,
        "<page width=\"{}\" height=\"{}\">",
        fnum(pg.w),
        fnum(pg.h)
    );
    // Hintergrund
    let stored: Vec<(String, String)> = xo
        .and_then(|x| x.get("bg"))
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(|a| {
            Some((
                a.get(0)?.as_str()?.to_string(),
                a.get(1)?.as_str()?.to_string(),
            ))
        })
        .collect();
    let get = |k: &str| stored.iter().find(|(n, _)| n == k).map(|(_, v)| v.as_str());
    out.push_str("<background");
    if get("type").is_some_and(|t| t != "solid") {
        // PDF-/Bild-Hintergrund: unverändert
        for (k, v) in &stored {
            let _ = write!(out, " {k}=\"{}\"", esc(v, true));
        }
    } else {
        let bg = pg.page.map(|p| p.bg).unwrap_or(PageBg::Plain);
        let old = get("style").filter(|s| PageBg::from_name(s) == bg);
        let style = old.unwrap_or(match bg {
            PageBg::Plain => "plain",
            PageBg::Lined => "lined",
            PageBg::Grid => "graph",
            PageBg::Dotted => "dotted",
        });
        let _ = write!(
            out,
            " type=\"solid\" color=\"{}\" style=\"{}\"",
            esc(get("color").unwrap_or("#ffffffff"), true),
            esc(style, true)
        );
        for (k, v) in &stored {
            // `config` gehört zum Stil: nur behalten, wenn der Stil unverändert ist
            if matches!(k.as_str(), "type" | "color" | "style") || (k == "config" && old.is_none())
            {
                continue;
            }
            let _ = write!(out, " {k}=\"{}\"", esc(v, true));
        }
    }
    out.push_str("/>\n");
    // Ebenen
    let raw: Vec<(i64, &str)> = xo
        .and_then(|x| x.get("raw"))
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(|r| Some((r.get("l")?.as_i64()?, r.get("x")?.as_str()?)))
        .collect();
    let stored_n = xo
        .and_then(|x| x.get("layers"))
        .and_then(Value::as_u64)
        .map(|n| n as usize)
        .unwrap_or(layers.len());
    let raw_n = raw
        .iter()
        .map(|(l, _)| (*l + 1).max(0) as usize)
        .max()
        .unwrap_or(0);
    let n = stored_n.max(pg.layers.len()).max(raw_n).max(1);
    for k in 0..n {
        out.push_str("<layer");
        if let Some(l) = layers.get(k).filter(|l| l.name != default_layer_name(k)) {
            let _ = write!(out, " name=\"{}\"", esc(&l.name, true));
        }
        out.push_str(">\n");
        if let Some(s) = pg.layers.get(k) {
            out.push_str(s);
        }
        for (_, x) in raw.iter().filter(|(l, _)| *l == k as i64) {
            out.push_str(x);
            out.push('\n');
        }
        out.push_str("</layer>\n");
    }
    for (_, x) in raw.iter().filter(|(l, _)| *l < 0) {
        out.push_str(x);
        out.push('\n');
    }
    out.push_str("</page>\n");
}

/// Zusatz-Attribute `"xa"` eines Items.
fn xattrs(raw: &Value) -> Vec<(&str, &str)> {
    raw.get("xa")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(|a| Some((a.get(0)?.as_str()?, a.get(1)?.as_str()?)))
        .collect()
}

/// Schreibt die Attribute `known` (Name, Wert) und danach die übrigen Zusatz-Attribute; Zusatz-Attribute
/// mit gleichem Namen ersetzen die berechneten nur bei `tool` (z. B. Radierer).
fn write_attrs(out: &mut String, known: &[(&str, String)], xa: &[(&str, &str)]) {
    for (k, v) in known {
        let v = if *k == "tool" {
            xa.iter()
                .find(|(n, _)| n == k)
                .map(|(_, v)| v.to_string())
                .unwrap_or_else(|| v.clone())
        } else {
            v.clone()
        };
        let _ = write!(out, " {k}=\"{}\"", esc(&v, true));
    }
    for (k, v) in xa {
        if known.iter().any(|(n, _)| n == k) || !valid_name(k) {
            continue;
        }
        let _ = write!(out, " {k}=\"{}\"", esc(v, true));
    }
}

fn valid_name(k: &str) -> bool {
    !k.is_empty()
        && k.chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '_' | '-' | ':' | '.'))
        && !k.starts_with(|c: char| c.is_ascii_digit())
}

/// Ein Strich: Punkte (Tripel, Weltkoordinaten), Breite, Druck ja/nein.
fn emit_stroke(
    out: &mut String,
    it: &Item,
    p: &[f32],
    hl: bool,
    var: bool,
    ox: f32,
    oy: f32,
    xa: &[(&str, &str)],
    fill: Option<u8>,
) {
    let n = p.len() / 3;
    if n == 0 {
        return;
    }
    let mut color = it.color;
    if hl && color >> 24 == 0xFF {
        color = (color & 0xFF_FFFF) | 0x7F00_0000;
    }
    let mut width = String::new();
    if var && !hl && n > 1 {
        width.push_str(&fnum(it.width * 2.0));
        for i in 0..n - 1 {
            let _ = write!(
                width,
                " {}",
                fnum(it.width * (0.35 + 1.3 * p[i * 3 + 2]).max(0.0))
            );
        }
    } else {
        width.push_str(&fnum(it.width));
    }
    let mut known = vec![
        ("tool", if hl { "highlighter" } else { "pen" }.to_string()),
        ("color", fmt_color(color)),
        ("width", width),
    ];
    if let Some(f) = fill {
        known.push(("fill", f.to_string()));
    }
    out.push_str("<stroke");
    write_attrs(out, &known, xa);
    out.push('>');
    let pts = if n == 1 { 2 } else { n };
    for i in 0..pts {
        let j = i.min(n - 1) * 3;
        if i > 0 {
            out.push(' ');
        }
        let _ = write!(out, "{} {}", fnum(p[j] - ox), fnum(p[j + 1] - oy));
    }
    out.push_str("</stroke>\n");
}

fn emit_text(
    out: &mut String,
    text: &str,
    color: u32,
    size: f32,
    x: f32,
    y: f32,
    xa: &[(&str, &str)],
) {
    let font = xa
        .iter()
        .find(|(k, _)| *k == "font")
        .map(|(_, v)| v.to_string())
        .unwrap_or_else(|| "Sans".into());
    let known = [
        ("font", font),
        ("size", fnum(size)),
        ("x", fnum(x)),
        ("y", fnum(y)),
        ("color", fmt_color(color)),
    ];
    out.push_str("<text");
    write_attrs(out, &known, xa);
    let _ = writeln!(out, ">{}</text>", esc(text, false));
}

fn emit_item(o: &Obj, ox: f32, oy: f32, doc: &Document, out: &mut String) {
    let it = &o.it;
    let xa = xattrs(o.raw);
    match &it.kind {
        Kind::Stroke(s) => emit_stroke(out, it, &s.p, s.hl, s.var, ox, oy, &xa, None),
        Kind::Shape(_) => {
            let parts = it.to_strokes();
            // Gefüllte Pfeilspitzen sind die letzten Teile; als gefülltes Dreieck schreiben.
            let heads = it.filled_heads();
            let first_head = parts.len().saturating_sub(heads.len());
            for (i, part) in parts.iter().enumerate() {
                let Kind::Stroke(s) = &part.kind else {
                    continue;
                };
                if i >= first_head {
                    // Linienbreite 0 → scharfe Spitze
                    let h = heads[i - first_head];
                    let p: Vec<f32> = [0, 1, 2, 0]
                        .iter()
                        .flat_map(|&k| [h[k * 2], h[k * 2 + 1], 0.5])
                        .collect();
                    let mut tri = part.clone();
                    tri.width = 0.01;
                    emit_stroke(out, &tri, &p, s.hl, false, ox, oy, &[], Some(255));
                } else {
                    emit_stroke(out, part, &s.p, s.hl, false, ox, oy, &[], None);
                }
            }
        }
        Kind::Note(n) => {
            if n.bg != 0 {
                // Haftnotiz: gefülltes Rechteck + Text
                let (l, t, r, b) = (n.x - ox, n.y - oy, n.x + n.w - ox, n.y + n.h - oy);
                let _ = writeln!(
                    out,
                    "<stroke tool=\"pen\" color=\"{}\" width=\"1\" fill=\"255\">{} {} {} {} {} {} {} {} {} {}</stroke>",
                    fmt_color(n.bg),
                    fnum(l),
                    fnum(t),
                    fnum(r),
                    fnum(t),
                    fnum(r),
                    fnum(b),
                    fnum(l),
                    fnum(b),
                    fnum(l),
                    fnum(t)
                );
                let pad = n.size * 0.7;
                emit_text(
                    out,
                    &n.text,
                    0xFF1B1B1F,
                    n.size,
                    n.x + pad - ox,
                    n.y + pad - oy,
                    &xa,
                );
            } else {
                emit_text(out, &n.text, it.color, n.size, n.x - ox, n.y - oy, &xa);
            }
        }
        Kind::Img(m) => {
            let (l, t) = (m.x.min(m.x + m.w), m.y.min(m.y + m.h));
            // unverändertes LaTeX-Bild roh zurückschreiben
            if let (Some(x), Some(g)) = (
                o.raw.get("xraw").and_then(Value::as_str),
                o.raw.get("xg").and_then(Value::as_array),
            ) {
                let g: Vec<f32> = g
                    .iter()
                    .filter_map(Value::as_f64)
                    .map(|v| v as f32)
                    .collect();
                let cur = [m.x - ox, m.y - oy, m.w, m.h];
                if g.len() == 4
                    && m.ang == 0.0
                    && g.iter().zip(cur).all(|(a, b)| (a - b).abs() < 0.01)
                {
                    out.push_str(x);
                    out.push('\n');
                    return;
                }
            }
            let Some(bytes) = doc.images.files.get(&m.file) else {
                return;
            };
            let png = if bytes.starts_with(b"\x89PNG") {
                bytes.clone()
            } else {
                let Ok(im) = image::load_from_memory(bytes) else {
                    return;
                };
                let mut v = Vec::new();
                if im
                    .write_to(&mut Cursor::new(&mut v), image::ImageFormat::Png)
                    .is_err()
                {
                    return;
                }
                v
            };
            let _ = writeln!(
                out,
                "<image left=\"{}\" top=\"{}\" right=\"{}\" bottom=\"{}\">{}</image>",
                fnum(l - ox),
                fnum(t - oy),
                fnum(l + m.w.abs() - ox),
                fnum(t + m.h.abs() - oy),
                b64_encode(&png)
            );
        }
    }
}

/// Zahl mit höchstens 5 Nachkommastellen, ohne überflüssige Nullen.
fn fnum(v: f32) -> String {
    if !v.is_finite() {
        return "0".into();
    }
    // kürzeste f32-Darstellung (385.6 statt 385.60001), sonst auf 5 Stellen gerundet
    let short = v.to_string();
    let s = if short.split_once('.').is_none_or(|(_, d)| d.len() <= 5) {
        short
    } else {
        format!("{:.5}", v as f64)
    };
    let s = if s.contains('.') {
        s.trim_end_matches('0').trim_end_matches('.')
    } else {
        &s
    };
    if s == "-0" {
        "0".into()
    } else {
        s.to_string()
    }
}

/// XML-Maskierung (Attribute zusätzlich `"` und Zeilenumbrüche).
fn esc(s: &str, attr: bool) -> String {
    let mut o = String::with_capacity(s.len());
    for c in s.chars() {
        match c {
            '&' => o.push_str("&amp;"),
            '<' => o.push_str("&lt;"),
            '>' => o.push_str("&gt;"),
            '"' if attr => o.push_str("&quot;"),
            '\n' if attr => o.push_str("&#10;"),
            '\r' => o.push_str("&#13;"),
            '\t' if attr => o.push_str("&#9;"),
            // in XML 1.0 nicht erlaubte Steuerzeichen weglassen
            c if (c as u32) < 0x20 && !matches!(c, '\n' | '\t') => {}
            c => o.push(c),
        }
    }
    o
}

// ---------------------------------------------------------------- Base64

const B64: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";

fn b64_encode(d: &[u8]) -> String {
    let mut o = String::with_capacity(d.len().div_ceil(3) * 4);
    for c in d.chunks(3) {
        let n = (c[0] as u32) << 16
            | (*c.get(1).unwrap_or(&0) as u32) << 8
            | *c.get(2).unwrap_or(&0) as u32;
        for i in 0..4 {
            if i <= c.len() {
                o.push(B64[(n >> (18 - 6 * i) & 63) as usize] as char);
            } else {
                o.push('=');
            }
        }
    }
    o
}

/// Dekodiert Base64 (Leerraum wird ignoriert); None bei ungültigen Zeichen oder leerem Ergebnis.
fn b64_decode(s: &str) -> Option<Vec<u8>> {
    let mut o = Vec::with_capacity(s.len() / 4 * 3);
    let (mut acc, mut bits) = (0u32, 0u32);
    for c in s.bytes() {
        let v = match c {
            b'A'..=b'Z' => c - b'A',
            b'a'..=b'z' => c - b'a' + 26,
            b'0'..=b'9' => c - b'0' + 52,
            b'+' | b'-' => 62,
            b'/' | b'_' => 63,
            b'=' => break,
            c if c.is_ascii_whitespace() => continue,
            _ => return None,
        };
        acc = acc << 6 | v as u32;
        bits += 6;
        if bits >= 8 {
            bits -= 8;
            o.push((acc >> bits) as u8);
            acc &= (1 << bits) - 1;
        }
    }
    if o.is_empty() {
        None
    } else {
        Some(o)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn farben() {
        assert_eq!(parse_color("#ff000080"), Some(0x80FF0000));
        assert_eq!(parse_color("#3333cc"), Some(0xFF3333CC));
        assert_eq!(parse_color("blue"), Some(0xFF3333CC));
        assert_eq!(parse_color("#xyz"), None);
        assert_eq!(fmt_color(0x80FF0000), "#ff000080");
        // vorzeichenbehaftet im Item-JSON
        let c = crate::item::jint(&json!(-15658735)).unwrap() as u32;
        assert_eq!(fmt_color(c), "#111111ff");
    }

    #[test]
    fn base64() {
        for n in 0..10 {
            let d: Vec<u8> = (0..n).map(|i| (i * 37 + 5) as u8).collect();
            let e = b64_encode(&d);
            assert_eq!(e.len() % 4, 0);
            if n > 0 {
                assert_eq!(b64_decode(&e).unwrap(), d);
            }
        }
        assert_eq!(b64_encode(b"Man"), "TWFu");
        assert_eq!(b64_decode("TW\nFu").unwrap(), b"Man");
    }

    #[test]
    fn zahlen_und_maskierung() {
        assert_eq!(fnum(1.5), "1.5");
        assert_eq!(fnum(2.0), "2");
        assert_eq!(fnum(-0.000001), "0");
        assert_eq!(
            esc("a<b & \"c\"\n", true),
            "a&lt;b &amp; &quot;c&quot;&#10;"
        );
        assert_eq!(esc("a\nb", false), "a\nb");
    }
}
