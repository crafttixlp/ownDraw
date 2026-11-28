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

/// Vektor-Befehle einer Seite (ohne Hintergrund), Koordinaten in pt.
pub fn page_ops(doc: &TextDoc, lay: &Layout, page: usize, imgs: &Images) -> Vec<VOp> {
    let mut ops = Vec::new();
    for l in lay.lines.iter().filter(|l| l.page == page && !l.pagebreak) {
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
