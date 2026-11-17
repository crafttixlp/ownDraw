//! PDF mit Vektorinhalt (Pfade, Bilder, Transparenz) und PDF/A-2b.

use super::vector::{dim, num, rgb, segs, Seg};
use crate::canvas::VOp;
use sha2::{Digest, Sha256};
use std::collections::{BTreeSet, HashMap};
use std::fmt::Write as _;
use tiny_skia::{Pixmap, Transform};

/// Objekt-Schreiber mit Offset-Tabelle (Index = Objektnummer − 1, 0 = noch nicht geschrieben).
struct W {
    out: Vec<u8>,
    offs: Vec<usize>,
}

impl W {
    fn alloc(&mut self) -> usize {
        self.offs.push(0);
        self.offs.len()
    }
    fn obj(&mut self, id: usize, body: &str) {
        self.offs[id - 1] = self.out.len();
        self.out.extend_from_slice(format!("{id} 0 obj\n{body}\nendobj\n").as_bytes());
    }
    /// Stream-Objekt; `dict` ohne << >> und ohne /Length.
    fn stream(&mut self, id: usize, dict: &str, data: &[u8], deflate: bool) {
        let z;
        let (data, filt) = if deflate {
            z = miniz_oxide::deflate::compress_to_vec_zlib(data, 6);
            (&z[..], " /Filter /FlateDecode")
        } else {
            (data, "")
        };
        self.offs[id - 1] = self.out.len();
        self.out.extend_from_slice(format!("{id} 0 obj\n<< {dict} /Length {}{filt} >>\nstream\n", data.len()).as_bytes());
        self.out.extend_from_slice(data);
        self.out.extend_from_slice(b"\nendstream\nendobj\n");
    }
}

/// Gemeinsamer Zustand aller Seiten (eine Ressourcen-Tabelle für Seiten und Formulare).
struct Doc {
    w: W,
    res: usize,
    alphas: BTreeSet<u8>,
    xobj: Vec<(String, usize)>,
    images: HashMap<[u8; 32], usize>,
}

fn n3(v: f32) -> String {
    num(v, 3)
}

fn path_ops(o: &mut String, s: &[Seg]) {
    for g in s {
        let _ = match *g {
            Seg::M(x, y) => writeln!(o, "{} {} m", n3(x), n3(y)),
            Seg::L(x, y) => writeln!(o, "{} {} l", n3(x), n3(y)),
            Seg::C(c) => writeln!(o, "{} {} {} {} {} {} c", n3(c[0]), n3(c[1]), n3(c[2]), n3(c[3]), n3(c[4]), n3(c[5])),
            Seg::Z => writeln!(o, "h"),
        };
    }
}

fn color(c: u32) -> String {
    let (r, g, b) = rgb(c);
    let f = |v: u8| num(v as f32 / 255.0, 3);
    format!("{} {} {}", f(r), f(g), f(b))
}

impl Doc {
    /// ExtGState-Name für Deckkraft a (setzt /ca und /CA).
    fn gs(&mut self, a: u8) -> String {
        self.alphas.insert(a);
        format!("/GA{a} gs")
    }

    /// Bild-XObject (RGB + SMask bei Transparenz), gleiche Bilder nur einmal.
    fn image(&mut self, pm: &Pixmap) -> usize {
        let key: [u8; 32] = {
            let mut h = Sha256::new();
            h.update(pm.width().to_le_bytes());
            h.update(pm.height().to_le_bytes());
            h.update(pm.data());
            h.finalize().into()
        };
        if let Some(&id) = self.images.get(&key) {
            return id;
        }
        let (iw, ih) = (pm.width(), pm.height());
        let mut rgbv = Vec::with_capacity(iw as usize * ih as usize * 3);
        let mut av = Vec::with_capacity(iw as usize * ih as usize);
        for p in pm.data().chunks_exact(4) {
            let a = p[3];
            for &c in &p[..3] {
                // demultiplizieren
                rgbv.push(if a == 0 { 0 } else { ((c as u32 * 255 + a as u32 / 2) / a as u32).min(255) as u8 });
            }
            av.push(a);
        }
        let smask = if av.iter().any(|&a| a != 255) {
            let id = self.w.alloc();
            self.w.stream(id, &format!("/Type /XObject /Subtype /Image /Width {iw} /Height {ih} /ColorSpace /DeviceGray /BitsPerComponent 8"), &av, true);
            format!(" /SMask {id} 0 R")
        } else {
            String::new()
        };
        let id = self.w.alloc();
        self.w.stream(id, &format!("/Type /XObject /Subtype /Image /Width {iw} /Height {ih} /ColorSpace /DeviceRGB /BitsPerComponent 8{smask}"), &rgbv, true);
        self.xobj.push((format!("Im{id}"), id));
        self.images.insert(key, id);
        id
    }

    /// Inhaltsstrom einer Seite (ohne y-Umkehr); Gruppen werden zu Form-XObjects.
    fn content(&mut self, ops: &[VOp], w: f32, h: f32) -> String {
        // Puffer-Stapel: neuer Puffer je Gruppe; Rahmen: None = Save, Some(a) = Gruppe
        let mut bufs: Vec<String> = vec![String::new()];
        let mut frames: Vec<Option<u8>> = Vec::new();
        let close = |d: &mut Doc, bufs: &mut Vec<String>, f: Option<u8>| match f {
            None => bufs.last_mut().unwrap().push_str("Q\n"),
            Some(a) => {
                let body = bufs.pop().unwrap_or_default();
                let id = d.w.alloc();
                d.w.stream(
                    id,
                    &format!("/Type /XObject /Subtype /Form /BBox [0 0 {} {}] /Group << /S /Transparency /CS /DeviceRGB >> /Resources {} 0 R", n3(w), n3(h), d.res),
                    body.as_bytes(),
                    true,
                );
                d.xobj.push((format!("Fm{id}"), id));
                let gs = d.gs(a);
                let _ = writeln!(bufs.last_mut().unwrap(), "q {gs} /Fm{id} Do Q");
            }
        };
        for op in ops {
            match op {
                VOp::Fill(p, c) => {
                    let s = segs(p);
                    if s.is_empty() || c >> 24 == 0 {
                        continue;
                    }
                    let a = (c >> 24) as u8;
                    let gs = if a != 255 { format!(" {}", self.gs(a)) } else { String::new() };
                    let o = bufs.last_mut().unwrap();
                    let _ = writeln!(o, "q{gs} {} rg", color(*c));
                    path_ops(o, &s);
                    o.push_str("f Q\n");
                }
                VOp::Stroke { path, color: c, width, round_cap, dash } => {
                    let s = segs(path);
                    if s.is_empty() || c >> 24 == 0 {
                        continue;
                    }
                    let a = (c >> 24) as u8;
                    let gs = if a != 255 { format!(" {}", self.gs(a)) } else { String::new() };
                    let o = bufs.last_mut().unwrap();
                    let d = match dash {
                        Some((x, y)) if *x > 0.0 || *y > 0.0 => format!("[{} {}] 0 d", n3(x.max(0.0)), n3(y.max(0.0))),
                        _ => "[] 0 d".into(),
                    };
                    let _ = writeln!(o, "q{gs} {} RG {} w {} J 1 j {d}", color(*c), n3(width.max(0.0)), if *round_cap { 1 } else { 0 });
                    path_ops(o, &s);
                    o.push_str("S Q\n");
                }
                VOp::Image(pm, m) => {
                    if ![m.sx, m.kx, m.ky, m.sy, m.tx, m.ty].iter().all(|v| v.is_finite()) {
                        continue;
                    }
                    let id = self.image(pm);
                    let (iw, ih) = (pm.width() as f32, pm.height() as f32);
                    // Einheitsquadrat (y nach oben) → Bildpixel → Ausgaberaum
                    let mm = Transform::from_row(m.sx * iw, m.ky * iw, -m.kx * ih, -m.sy * ih, m.kx * ih + m.tx, m.sy * ih + m.ty);
                    let n = |v: f32| num(v, 5);
                    let _ = writeln!(bufs.last_mut().unwrap(), "q {} {} {} {} {} {} cm /Im{id} Do Q", n(mm.sx), n(mm.ky), n(mm.kx), n(mm.sy), n(mm.tx), n(mm.ty));
                }
                VOp::Save => {
                    bufs.last_mut().unwrap().push_str("q\n");
                    frames.push(None);
                }
                VOp::Group(a) => {
                    frames.push(Some(*a));
                    bufs.push(String::new());
                }
                VOp::Restore => {
                    if let Some(f) = frames.pop() {
                        close(self, &mut bufs, f);
                    }
                }
                VOp::Clip(p) => {
                    let s = segs(p);
                    let o = bufs.last_mut().unwrap();
                    if s.is_empty() {
                        o.push_str("0 0 0 0 re\n");
                    } else {
                        path_ops(o, &s);
                    }
                    o.push_str("W n\n");
                }
            }
        }
        while let Some(f) = frames.pop() {
            close(self, &mut bufs, f);
        }
        bufs.pop().unwrap_or_default()
    }
}

/// PDF-Textstring: ASCII als Literal, sonst UTF-16BE mit BOM.
fn pdf_text(s: &str) -> String {
    if s.chars().all(|c| (' '..='~').contains(&c)) {
        let mut o = String::from("(");
        for c in s.chars() {
            if matches!(c, '(' | ')' | '\\') {
                o.push('\\');
            }
            o.push(c);
        }
        o.push(')');
        o
    } else {
        let mut o = String::from("<FEFF");
        for u in s.encode_utf16() {
            let _ = write!(o, "{u:04X}");
        }
        o.push('>');
        o
    }
}

fn xml_esc(s: &str) -> String {
    s.chars()
        .filter(|&c| c == '\t' || c == '\n' || c >= ' ')
        .map(|c| match c {
            '&' => "&amp;".into(),
            '<' => "&lt;".into(),
            '>' => "&gt;".into(),
            '"' => "&quot;".into(),
            c => c.to_string(),
        })
        .collect()
}

/// Minimales sRGB-ICC-Profil (v2, mntr, RGB → XYZ, Gamma 2,2).
pub(crate) fn srgb_icc() -> Vec<u8> {
    let s15 = |v: f64| ((v * 65536.0).round() as i32).to_be_bytes();
    let xyz = |x: f64, y: f64, z: f64| {
        let mut d = b"XYZ \0\0\0\0".to_vec();
        for v in [x, y, z] {
            d.extend_from_slice(&s15(v));
        }
        d
    };
    let mut desc = b"desc\0\0\0\0".to_vec();
    let name = b"sRGB (OwnDraw)\0";
    desc.extend_from_slice(&(name.len() as u32).to_be_bytes());
    desc.extend_from_slice(name);
    desc.extend_from_slice(&[0; 4 + 4 + 2 + 1 + 67]); // Unicode- und ScriptCode-Teil leer
    let mut cprt = b"text\0\0\0\0".to_vec();
    cprt.extend_from_slice(b"No copyright, use freely\0");
    let mut trc = b"curv\0\0\0\0".to_vec();
    trc.extend_from_slice(&1u32.to_be_bytes());
    trc.extend_from_slice(&0x0233u16.to_be_bytes()); // u8Fixed8: 2,2
    let tags: Vec<(&[u8; 4], Vec<u8>)> = vec![
        (b"desc", desc),
        (b"cprt", cprt),
        (b"wtpt", xyz(0.9642, 1.0, 0.8249)),
        (b"rXYZ", xyz(0.4361, 0.2225, 0.0139)),
        (b"gXYZ", xyz(0.3851, 0.7169, 0.0971)),
        (b"bXYZ", xyz(0.1431, 0.0606, 0.7141)),
        (b"rTRC", trc),
    ];
    // gTRC/bTRC teilen sich die Daten von rTRC
    let ntag = tags.len() + 2;
    let mut data: Vec<u8> = Vec::new();
    let mut table: Vec<u8> = (ntag as u32).to_be_bytes().to_vec();
    let base = 128 + 4 + 12 * ntag;
    let mut trc_at = (0, 0);
    for (sig, d) in &tags {
        let off = base + data.len();
        table.extend_from_slice(*sig);
        table.extend_from_slice(&(off as u32).to_be_bytes());
        table.extend_from_slice(&(d.len() as u32).to_be_bytes());
        if *sig == b"rTRC" {
            trc_at = (off, d.len());
        }
        data.extend_from_slice(d);
        while !data.len().is_multiple_of(4) {
            data.push(0);
        }
    }
    for sig in [b"gTRC", b"bTRC"] {
        table.extend_from_slice(sig);
        table.extend_from_slice(&(trc_at.0 as u32).to_be_bytes());
        table.extend_from_slice(&(trc_at.1 as u32).to_be_bytes());
    }
    let size = base + data.len();
    let mut p: Vec<u8> = Vec::with_capacity(size);
    p.extend_from_slice(&(size as u32).to_be_bytes());
    p.extend_from_slice(&[0; 4]); // CMM
    p.extend_from_slice(&0x0210_0000u32.to_be_bytes());
    p.extend_from_slice(b"mntrRGB XYZ ");
    for v in [2024u16, 1, 1, 0, 0, 0] {
        p.extend_from_slice(&v.to_be_bytes());
    }
    p.extend_from_slice(b"acsp");
    p.extend_from_slice(&[0; 4 + 4 + 4 + 4 + 8 + 4]); // Plattform, Flags, Hersteller, Modell, Attribute, Intent
    for v in [0.9642, 1.0, 0.8249] {
        p.extend_from_slice(&s15(v));
    }
    p.extend_from_slice(&[0; 4 + 16 + 28]); // Ersteller, ID, reserviert
    debug_assert_eq!(p.len(), 128);
    p.extend_from_slice(&table);
    p.extend_from_slice(&data);
    p
}

/// Mehrseitiges PDF; jede Seite (ops, w, h) mit Größe w × h Punkt. `pdfa` = PDF/A-2b (XMP, sRGB-OutputIntent).
pub fn pdf(pages: &[(Vec<VOp>, f32, f32)], pdfa: bool, title: &str) -> Vec<u8> {
    let mut w = W { out: Vec::new(), offs: Vec::new() };
    w.out.extend_from_slice(b"%PDF-1.7\n%\xE2\xE3\xCF\xD3\n");
    let (cat, tree, res) = (w.alloc(), w.alloc(), w.alloc());
    let mut d = Doc { w, res, alphas: BTreeSet::new(), xobj: Vec::new(), images: HashMap::new() };
    let leer: [(Vec<VOp>, f32, f32); 1] = [(Vec::new(), 595.0, 842.0)];
    let pages = if pages.is_empty() { &leer[..] } else { pages };
    let mut kids = Vec::new();
    for (ops, pw, ph) in pages {
        let (pw, ph) = (dim(*pw), dim(*ph));
        let body = d.content(ops, pw, ph);
        let c = format!("1 0 0 -1 0 {} cm\n{body}", n3(ph));
        let cid = d.w.alloc();
        d.w.stream(cid, "", c.as_bytes(), true);
        let pid = d.w.alloc();
        d.w.obj(
            pid,
            &format!(
                "<< /Type /Page /Parent {tree} 0 R /MediaBox [0 0 {} {}] /Resources {res} 0 R /Contents {cid} 0 R /Group << /S /Transparency /CS /DeviceRGB >> >>",
                n3(pw),
                n3(ph)
            ),
        );
        kids.push(format!("{pid} 0 R"));
    }
    // Ressourcen
    let mut r = String::from("<< /ProcSet [/PDF /ImageC /ImageB]");
    if !d.alphas.is_empty() {
        r.push_str(" /ExtGState <<");
        for a in &d.alphas {
            let v = num(*a as f32 / 255.0, 4);
            let _ = write!(r, " /GA{a} << /Type /ExtGState /ca {v} /CA {v} >>");
        }
        r.push_str(" >>");
    }
    if !d.xobj.is_empty() {
        r.push_str(" /XObject <<");
        for (n, id) in &d.xobj {
            let _ = write!(r, " /{n} {id} 0 R");
        }
        r.push_str(" >>");
    }
    r.push_str(" >>");
    let mut w = d.w;
    w.obj(res, &r);
    w.obj(tree, &format!("<< /Type /Pages /Kids [{}] /Count {} >>", kids.join(" "), kids.len()));
    let info = w.alloc();
    let mut inf = String::from("<< /Producer (OwnDraw)");
    if !title.is_empty() {
        let _ = write!(inf, " /Title {}", pdf_text(title));
    }
    inf.push_str(" >>");
    w.obj(info, &inf);
    let mut extra = String::new();
    if pdfa {
        let icc = w.alloc();
        w.stream(icc, "/N 3", &srgb_icc(), true);
        let meta = w.alloc();
        let xmp = format!(
            "<?xpacket begin=\"\u{FEFF}\" id=\"W5M0MpCehiHzreSzNTczkc9d\"?>\n\
<x:xmpmeta xmlns:x=\"adobe:ns:meta/\">\n\
<rdf:RDF xmlns:rdf=\"http://www.w3.org/1999/02/22-rdf-syntax-ns#\">\n\
<rdf:Description rdf:about=\"\" xmlns:pdfaid=\"http://www.aiim.org/pdfa/ns/id/\" xmlns:dc=\"http://purl.org/dc/elements/1.1/\" xmlns:pdf=\"http://ns.adobe.com/pdf/1.3/\">\n\
<pdfaid:part>2</pdfaid:part>\n<pdfaid:conformance>B</pdfaid:conformance>\n\
<pdf:Producer>OwnDraw</pdf:Producer>\n{}\
</rdf:Description>\n</rdf:RDF>\n</x:xmpmeta>\n<?xpacket end=\"w\"?>",
            if title.is_empty() { String::new() } else { format!("<dc:title><rdf:Alt><rdf:li xml:lang=\"x-default\">{}</rdf:li></rdf:Alt></dc:title>\n", xml_esc(title)) }
        );
        w.stream(meta, "/Type /Metadata /Subtype /XML", xmp.as_bytes(), false);
        let _ = write!(
            extra,
            " /Metadata {meta} 0 R /OutputIntents [<< /Type /OutputIntent /S /GTS_PDFA1 /OutputConditionIdentifier (sRGB) /Info (sRGB IEC61966-2.1) /DestOutputProfile {icc} 0 R >>]"
        );
    }
    w.obj(cat, &format!("<< /Type /Catalog /Pages {tree} 0 R{extra} >>"));
    // Datei-ID aus dem Inhalt
    let id: String = Sha256::digest(&w.out)[..16].iter().map(|b| format!("{b:02X}")).collect();
    let xref = w.out.len();
    let mut x = format!("xref\n0 {}\n0000000000 65535 f \n", w.offs.len() + 1);
    for o in &w.offs {
        let _ = writeln!(x, "{o:010} 00000 n ");
    }
    let _ = write!(x, "trailer\n<< /Size {} /Root {cat} 0 R /Info {info} 0 R /ID [<{id}> <{id}>] >>\nstartxref\n{xref}\n%%EOF\n", w.offs.len() + 1);
    w.out.extend_from_slice(x.as_bytes());
    w.out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::export::vector::tests::sample_ops;
    use tiny_skia::{PathBuilder, Rect};

    fn check_xref(b: &[u8]) -> usize {
        // Byte-Positionen: nur den ASCII-Schluss als Text lesen
        let pos = b.windows(10).rposition(|w| w == b"startxref\n").unwrap();
        let tail = std::str::from_utf8(&b[pos + 10..]).unwrap();
        let sx: usize = tail.lines().next().unwrap().parse().unwrap();
        let s = std::str::from_utf8(&b[sx..]).unwrap();
        assert!(s.starts_with("xref\n"));
        let mut lines = s.lines().skip(1);
        let n: usize = lines.next().unwrap().split(' ').nth(1).unwrap().parse().unwrap();
        lines.next();
        for i in 1..n {
            let off: usize = lines.next().unwrap()[..10].parse().unwrap();
            assert!(b[off..].starts_with(format!("{i} 0 obj").as_bytes()), "Objekt {i} nicht bei {off}");
        }
        n
    }

    #[test]
    fn pdf_struktur() {
        let ops = sample_ops();
        let b = pdf(&[(ops.clone(), 100.0, 80.0), (ops, 50.0, 50.0)], false, "Test (1)");
        assert!(b.starts_with(b"%PDF-1.7\n%"));
        check_xref(&b);
        let s = String::from_utf8_lossy(&b);
        assert_eq!(s.matches("/Type /Page /Parent").count(), 2);
        assert!(s.contains("/Count 2"));
        assert!(s.contains("/Title (Test \\(1\\))"));
        assert!(s.contains("/SMask"));
        assert!(s.contains("/Subtype /Form"));
        assert!(s.contains("/GA128 << /Type /ExtGState /ca 0.502"));
        // gleiches Bild nur einmal
        assert_eq!(s.matches("/ColorSpace /DeviceRGB").count(), 1);
        assert!(s.ends_with("%%EOF\n"));
        assert!(!s.contains("/OutputIntents"));
        // leer
        let e = pdf(&[], false, "");
        check_xref(&e);
        assert!(String::from_utf8_lossy(&e).contains("/Count 1"));
    }

    #[test]
    fn pdf_inhalt() {
        let mut d = Doc { w: W { out: Vec::new(), offs: vec![0] }, res: 1, alphas: BTreeSet::new(), xobj: Vec::new(), images: HashMap::new() };
        let c = d.content(&sample_ops(), 100.0, 80.0);
        assert!(c.contains("5 5 m\n25.5 5 l"));
        assert!(!c.contains("W n"), "Clip liegt im Form-XObject der Gruppe");
        let r = PathBuilder::from_rect(Rect::from_xywh(1.0, 2.0, 3.0, 4.0).unwrap());
        let c2 = d.content(&[VOp::Clip(r.clone()), VOp::Fill(r, 0xFF000000)], 10.0, 10.0);
        assert!(c2.starts_with("1 2 m\n4 2 l\n4 6 l\n1 6 l\nh\nW n\nq 0 0 0 rg"));
        assert!(c.contains("[4 2] 0 d"));
        assert!(c.contains("1 J 1 j"));
        // Bild 3×2 mit scale 10 an (50,50): Einheitsquadrat → 30 × −20, Ursprung unten links (50,70)
        assert!(c.contains("q 30 0 0 -20 50 70 cm /Im"));
        assert_eq!(c.matches("q").count(), c.matches("Q").count());
    }

    #[test]
    fn pdfa() {
        let b = pdf(&[(sample_ops(), 100.0, 80.0)], true, "Ärger & Co");
        check_xref(&b);
        let s = String::from_utf8_lossy(&b);
        assert!(s.contains("/OutputIntents"));
        assert!(s.contains("/S /GTS_PDFA1"));
        assert!(s.contains("<pdfaid:part>2</pdfaid:part>"));
        assert!(s.contains("Ärger &amp; Co"));
        assert!(s.contains("/ID [<"));
        let icc = srgb_icc();
        assert_eq!(u32::from_be_bytes(icc[0..4].try_into().unwrap()) as usize, icc.len());
        assert_eq!(&icc[36..40], b"acsp");
        assert_eq!(icc.len() % 4, 0);
    }
}
