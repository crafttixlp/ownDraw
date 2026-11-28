//! Zeichnen gesetzter Seiten als Vektor-Befehle: PDF-Export und Vorschaubild. Text wird als Umriss gezeichnet,
//! daher sieht das PDF überall gleich aus.

use super::fonts::{self, Family, FontKey};
use super::layout::{Layout, Seg};
use super::model::TextDoc;
use crate::canvas::VOp;
use std::collections::BTreeMap;
use tiny_skia::{FillRule, Paint, PathBuilder, Pixmap, PixmapPaint, Rect, Transform};

/// Bilder des Dokuments (Dateiname → Inhalt), wie `ImageStore::files`.
pub type Images = BTreeMap<String, Vec<u8>>;

/// Bild-Datei → vormultipliziertes Pixmap.
pub fn decode(bytes: &[u8]) -> Option<Pixmap> {
    let im = image::load_from_memory(bytes).ok()?.into_rgba8();
    let (w, h) = im.dimensions();
    let mut pm = Pixmap::new(w, h)?;
    for (dst, px) in pm.pixels_mut().iter_mut().zip(im.pixels()) {
        let [r, g, b, a] = px.0;
        *dst = tiny_skia::ColorU8::from_rgba(r, g, b, a).premultiply();
    }
    Some(pm)
}

fn rect(ops: &mut Vec<VOp>, x0: f32, y0: f32, x1: f32, y1: f32, color: u32) {
    if let Some(r) = Rect::from_ltrb(x0.min(x1), y0.min(y1), x0.max(x1), y0.max(y1).max(y0.min(y1) + 0.01)) {
        ops.push(VOp::Fill(PathBuilder::from_rect(r), color));
    }
}

fn text(ops: &mut Vec<VOp>, words: &[(f32, String)], font: FontKey, size: f32, base: f32, color: u32) {
    let mut pb = PathBuilder::new();
    for (x, w) in words {
        let mut pen = *x;
        for ch in w.chars() {
            fonts::outline(&mut pb, font, ch, pen, base, size);
            pen += fonts::advance(font, ch, size);
        }
    }
    if let Some(p) = pb.finish() {
        ops.push(VOp::Fill(p, color));
    }
}

fn seg(ops: &mut Vec<VOp>, s: &Seg, y: f32, h: f32, base: f32) {
    let st = &s.st;
    if st.hl != 0 {
        rect(ops, s.x0, y, s.x1, y + h, st.hl);
    }
    let bl = y + base + st.dy;
    text(ops, &s.words, st.font, st.size, bl, st.color);
    let th = (st.size * 0.06).max(0.5);
    if st.u {
        let uy = bl + st.size * 0.12;
        rect(ops, s.x0, uy, s.x1, uy + th, st.color);
    }
    if st.s {
        let sy = bl - st.size * 0.28;
        rect(ops, s.x0, sy, s.x1, sy + th, st.color);
    }
}

/// Objekt als Vektor-Befehle an (x, y) in Größe w × h.
pub fn obj_ops(obj: &super::objects::Obj, x: f32, y: f32, w: f32, h: f32) -> Vec<VOp> {
    let Some(mut pm) = Pixmap::new(1, 1) else { return Vec::new() };
    let mut c = crate::canvas::Canvas::new(&mut pm);
    c.start_recording();
    c.translate(x, y);
    super::draw::draw_obj(&mut c, obj, w, h);
    let (rec, _keep) = c.take_recording();
    rec.vector_ops(w, h)
}

/// Rahmen und Inhalt einer Tabellenzeile.
fn row_ops(ops: &mut Vec<VOp>, doc: &TextDoc, l: &super::layout::Line, rb: &super::layout::RowBox) {
    let border = doc.paras.get(l.para).and_then(|p| p.table.as_ref()).map(|t| t.border).unwrap_or(0xFFB8BCC6);
    for c in &rb.cells {
        if c.bg != 0 {
            rect(ops, c.x, l.y, c.x + c.w, l.y + l.h, c.bg);
        }
        for (x, by, t) in &c.lines {
            text(ops, &[(*x, t.clone())], c.font, c.size, l.y + by, c.color);
        }
    }
    if border != 0 {
        let t = 0.6;
        let (x0, x1) = l.block_x().unwrap_or((0.0, 0.0));
        rect(ops, x0, l.y, x1, l.y + t, border);
        rect(ops, x0, l.y + l.h - t, x1, l.y + l.h, border);
        for c in &rb.cells {
            rect(ops, c.x, l.y, c.x + t, l.y + l.h, border);
        }
        rect(ops, x1 - t, l.y, x1, l.y + l.h, border);
    }
}

/// Wasserzeichen schräg über die Seite.
fn watermark_ops(ops: &mut Vec<VOp>, s: &str, pg: &super::model::PageSpec) {
    let font = FontKey { fam: Family::Sans, bold: true, italic: false };
    let w: f32 = s.chars().map(|c| fonts::advance(font, c, 100.0)).sum();
    let size = (100.0 * (pg.w.hypot(pg.h) * 0.7) / w.max(1.0)).min(140.0);
    let tw: f32 = s.chars().map(|c| fonts::advance(font, c, size)).sum();
    let mut pb = PathBuilder::new();
    let mut pen = -tw / 2.0;
    for ch in s.chars() {
        fonts::outline(&mut pb, font, ch, pen, size * 0.35, size);
        pen += fonts::advance(font, ch, size);
    }
    let m = Transform::from_translate(pg.w / 2.0, pg.h / 2.0).pre_rotate(-pg.h.atan2(pg.w).to_degrees());
    if let Some(p) = pb.finish().and_then(|p| p.transform(m)) {
        ops.push(VOp::Fill(p, 0x28707080));
    }
}

/// Vektor-Befehle einer Seite (ohne Hintergrund), Koordinaten in pt.
pub fn page_ops(doc: &TextDoc, lay: &Layout, page: usize, imgs: &Images) -> Vec<VOp> {
    let mut ops = Vec::new();
    if !doc.ex.watermark.trim().is_empty() {
        watermark_ops(&mut ops, doc.ex.watermark.trim(), &lay.page);
    }
    for l in lay.lines.iter().filter(|l| l.page == page && !l.pagebreak) {
        if let Some(rb) = &l.row {
            row_ops(&mut ops, doc, l, rb);
            continue;
        }
        if let (Some((x, w, h)), Some(o)) = (l.obj, doc.paras.get(l.para).and_then(|p| p.obj.as_ref())) {
            ops.extend(obj_ops(o, x, l.y, w, h));
            continue;
        }
        if let Some((f, x, w, h)) = &l.img {
            match imgs.get(f).and_then(|b| decode(b)) {
                Some(pm) => {
                    let t = Transform::from_row(w / pm.width() as f32, 0.0, 0.0, h / pm.height() as f32, *x, l.y);
                    ops.push(VOp::Image(pm, t));
                }
                None => rect(&mut ops, *x, l.y, x + w, l.y + h, 0xFFE5E7EB),
            }
            continue;
        }
        if let Some(m) = &l.marker {
            seg(&mut ops, m, l.y, l.h, l.base);
        }
        for s in &l.segs {
            seg(&mut ops, s, l.y, l.h, l.base);
        }
    }
    let pg = &lay.page;
    // Fußnoten
    let nf = FontKey { fam: doc.family, bold: false, italic: false };
    let mut first = true;
    for nb in lay.notes.iter().filter(|n| n.page == page) {
        if first {
            rect(&mut ops, pg.ml, nb.y - 6.0, pg.ml + 120.0, nb.y - 5.4, 0xFF9CA3AF);
            first = false;
        }
        for (by, t) in &nb.lines {
            text(&mut ops, &[(nb.x, t.clone())], nf, super::layout::NOTE_SIZE, *by, 0xFF33333D);
        }
    }
    let hf = FontKey { fam: Family::Sans, bold: false, italic: false };
    let size = 9.0;
    for (src, y, center) in [(&doc.header, pg.mt * 0.5, false), (&doc.footer, pg.h - pg.mb * 0.5 + size * 0.35, true)] {
        if src.trim().is_empty() {
            continue;
        }
        let t = TextDoc::hf_text(src, page, lay.pages);
        let w: f32 = t.chars().map(|c| fonts::advance(hf, c, size)).sum();
        let x = if center { pg.ml + (pg.content_w() - w) / 2.0 } else { pg.ml };
        text(&mut ops, &[(x, t)], hf, size, y, 0xFF6B6B78);
    }
    ops
}

/// Objekt (Form, Zeichnung, Formel, Diagramm) als PNG mit `k` Pixel je pt (für Word/ODT/HTML-Export).
pub fn object_png(obj: &super::objects::Obj, k: f32) -> Option<Vec<u8>> {
    object_pixmap(obj, k)?.encode_png().ok()
}

/// Objekt als Pixmap (transparent) mit `k` Pixel je pt.
pub fn object_pixmap(obj: &super::objects::Obj, k: f32) -> Option<Pixmap> {
    let (w, h) = (obj.w, obj.h);
    let mut pm = Pixmap::new((w * k).ceil().clamp(1.0, 4096.0) as u32, (h * k).ceil().clamp(1.0, 4096.0) as u32)?;
    {
        let mut c = crate::canvas::Canvas::new(&mut pm);
        c.scale(k, k);
        super::draw::draw_obj(&mut c, obj, w, h);
    }
    Some(pm)
}

/// PDF aller Seiten.
pub fn pdf(doc: &TextDoc, lay: &Layout, title: &str, imgs: &Images) -> Vec<u8> {
    let mut pages = Vec::with_capacity(lay.pages);
    for i in 0..lay.pages {
        let mut ops = Vec::new();
        rect(&mut ops, 0.0, 0.0, lay.page.w, lay.page.h, 0xFFFFFFFF);
        ops.extend(page_ops(doc, lay, i, imgs));
        pages.push((ops, lay.page.w, lay.page.h));
    }
    crate::export::pdfv::pdf(&pages, false, title)
}

/// Zeichnet eine Seite in ein Pixmap mit Maßstab `k` (Pixel je pt).
pub fn raster_page(doc: &TextDoc, lay: &Layout, page: usize, k: f32, imgs: &Images) -> Option<Pixmap> {
    let w = (lay.page.w * k).ceil().max(1.0) as u32;
    let h = (lay.page.h * k).ceil().max(1.0) as u32;
    let mut pm = Pixmap::new(w, h)?;
    pm.fill(tiny_skia::Color::WHITE);
    let t = Transform::from_scale(k, k);
    for op in page_ops(doc, lay, page, imgs) {
        match op {
            VOp::Fill(p, c) => {
                let mut paint = Paint::default();
                paint.set_color(crate::canvas::sk_color(c));
                paint.anti_alias = true;
                pm.fill_path(&p, &paint, FillRule::Winding, t, None);
            }
            VOp::Image(img, m) => {
                let pp = PixmapPaint { quality: tiny_skia::FilterQuality::Bilinear, ..PixmapPaint::default() };
                pm.draw_pixmap(0, 0, img.as_ref(), &pp, t.pre_concat(m), None);
            }
            VOp::Stroke { path, color, width, round_cap, dash } => {
                let mut paint = Paint::default();
                paint.set_color(crate::canvas::sk_color(color));
                paint.anti_alias = true;
                let mut sk = tiny_skia::Stroke { width, line_join: tiny_skia::LineJoin::Round, ..Default::default() };
                if round_cap {
                    sk.line_cap = tiny_skia::LineCap::Round;
                }
                if let Some((a, b)) = dash {
                    sk.dash = tiny_skia::StrokeDash::new(vec![a, b], 0.0);
                }
                pm.stroke_path(&path, &paint, &sk, t, None);
            }
            _ => {}
        }
    }
    Some(pm)
}

/// Vorschaubild (PNG) der ersten Seite, höchstens 512 px hoch.
pub fn thumb(doc: &TextDoc, lay: &Layout, imgs: &Images) -> Option<Vec<u8>> {
    let k = (512.0 / lay.page.h.max(lay.page.w)).min(1.0);
    raster_page(doc, lay, 0, k, imgs)?.encode_png().ok()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::office::layout::layout;
    use crate::office::model::{CStyle, Pos};

    #[test]
    fn pdf_und_vorschau() {
        let mut d = TextDoc::new();
        d.insert_text(Pos::default(), "Hallo Welt, schöne Grüße! €", &CStyle { u: true, hl: 0xFFFDE68A, ..CStyle::default() });
        d.footer = "Seite {seite}".into();
        let l = layout(&d);
        let ops = page_ops(&d, &l, 0, &Images::new());
        assert!(ops.len() >= 4);
        let pdf = pdf(&d, &l, "Test", &Images::new());
        assert!(pdf.starts_with(b"%PDF"));
        let png = thumb(&d, &l, &Images::new()).unwrap();
        assert!(png.starts_with(b"\x89PNG"));
        // Text erzeugt dunkle Pixel
        let pm = raster_page(&d, &l, 0, 1.0, &Images::new()).unwrap();
        assert!(pm.pixels().iter().any(|p| p.red() < 100));
    }

    #[test]
    fn bild_im_dokument() {
        use crate::office::model::{Kind, Para};
        let mut png = Vec::new();
        let mut pm = Pixmap::new(4, 2).unwrap();
        pm.fill(tiny_skia::Color::from_rgba8(200, 0, 0, 255));
        png.extend(pm.encode_png().unwrap());
        let mut imgs = Images::new();
        imgs.insert("a.png".into(), png);
        let mut d = TextDoc::new();
        d.paras = vec![Para::with_text(Kind::Body, "Vor"), Para::image("a.png", 2000.0, 1000.0), Para::with_text(Kind::Body, "Nach")];
        let l = layout(&d);
        let li = l.lines.iter().find(|x| x.img.is_some()).unwrap();
        let (_, _, w, h) = li.img.clone().unwrap();
        assert!((w - d.page.content_w()).abs() < 0.1 && (w / h - 2.0).abs() < 0.01, "auf Inhaltsbreite verkleinert");
        let pm = raster_page(&d, &l, 0, 1.0, &imgs).unwrap();
        let (x, y) = ((d.page.ml + w / 2.0) as u32, (li.y + h / 2.0) as u32);
        let px = pm.pixel(x, y).unwrap();
        assert!(px.red() > 150 && px.green() < 50, "Bild gezeichnet");
        assert!(pdf(&d, &l, "B", &imgs).windows(6).any(|w| w == b"/Image"));
        // JSON-Rundreise
        let back = TextDoc::from_json(&d.to_json());
        assert_eq!(back.paras[1].img, d.paras[1].img);
        // Rücktaste hinter dem Bild löscht es
        let mut e = d.clone();
        let p = e.delete(Pos::new(1, 0), Pos::new(2, 0));
        assert_eq!(p, Pos::new(1, 0));
        assert!(e.paras.iter().all(|p| p.kind != Kind::Image));
        // Einfügen eines Bildes mitten im Text
        let mut e = TextDoc::new();
        e.insert_text(Pos::default(), "abcd", &Default::default());
        e.insert_paras(Pos::new(0, 2), &[Para::image("a.png", 10.0, 10.0)]);
        assert_eq!(e.paras.iter().map(|p| p.kind).collect::<Vec<_>>(), [Kind::Body, Kind::Image, Kind::Body]);
        assert_eq!(e.plain_text(), "ab\n\ncd");
    }
}
