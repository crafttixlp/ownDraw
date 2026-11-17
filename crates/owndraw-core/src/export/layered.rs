//! Formate mit Ebenen: OpenRaster (.ora, Krita/GIMP) und Photoshop (.psd).

use std::io::{Cursor, Write};
use tiny_skia::Pixmap;
use zip::write::SimpleFileOptions;

/// Eine Ebene (alle gleich groß wie `merged`), von hinten nach vorne.
pub struct LayerImg {
    pub name: String,
    pub pm: Pixmap,
    pub opacity: f32,
    pub visible: bool,
}

fn esc(s: &str) -> String {
    let mut o = String::new();
    for c in s.chars() {
        match c {
            '&' => o.push_str("&amp;"),
            '<' => o.push_str("&lt;"),
            '>' => o.push_str("&gt;"),
            '"' => o.push_str("&quot;"),
            c if (c as u32) < 0x20 => o.push(' '),
            c => o.push(c),
        }
    }
    o
}

fn png(pm: &Pixmap) -> Result<Vec<u8>, String> {
    pm.encode_png().map_err(|e| e.to_string())
}

/// Verkleinert auf höchstens 256 px an der langen Kante.
fn thumb(pm: &Pixmap) -> Option<Pixmap> {
    let (w, h) = (pm.width() as f32, pm.height() as f32);
    let k = (256.0 / w.max(h)).min(1.0);
    let (tw, th) = (((w * k).round() as u32).max(1), ((h * k).round() as u32).max(1));
    let mut t = Pixmap::new(tw, th)?;
    let paint = tiny_skia::PixmapPaint { quality: tiny_skia::FilterQuality::Bilinear, ..Default::default() };
    t.draw_pixmap(0, 0, pm.as_ref(), &paint, tiny_skia::Transform::from_scale(tw as f32 / w, th as f32 / h), None);
    Some(t)
}

pub fn ora(layers: &[LayerImg], merged: &Pixmap) -> Result<Vec<u8>, String> {
    let mut z = zip::ZipWriter::new(Cursor::new(Vec::new()));
    let stored = SimpleFileOptions::default().compression_method(zip::CompressionMethod::Stored);
    let e = |x: zip::result::ZipError| x.to_string();
    // mimetype muss erster, unkomprimierter Eintrag sein
    z.start_file("mimetype", stored).map_err(e)?;
    z.write_all(b"image/openraster").map_err(|x| x.to_string())?;
    let mut st = format!(
        "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n<image version=\"0.0.3\" w=\"{}\" h=\"{}\"><stack>\n",
        merged.width(),
        merged.height()
    );
    // stack.xml: oberste Ebene zuerst
    for (i, l) in layers.iter().enumerate().rev() {
        st.push_str(&format!(
            "<layer name=\"{}\" src=\"data/{:03}.png\" opacity=\"{:.3}\" visibility=\"{}\" x=\"0\" y=\"0\" composite-op=\"svg:src-over\"/>\n",
            esc(&l.name),
            i,
            l.opacity.clamp(0.0, 1.0),
            if l.visible { "visible" } else { "hidden" }
        ));
    }
    st.push_str("</stack></image>\n");
    z.start_file("stack.xml", stored).map_err(e)?;
    z.write_all(st.as_bytes()).map_err(|x| x.to_string())?;
    for (i, l) in layers.iter().enumerate() {
        z.start_file(format!("data/{i:03}.png"), stored).map_err(e)?;
        z.write_all(&png(&l.pm)?).map_err(|x| x.to_string())?;
    }
    z.start_file("mergedimage.png", stored).map_err(e)?;
    z.write_all(&png(merged)?).map_err(|x| x.to_string())?;
    let t = thumb(merged).ok_or("Vorschau nicht erstellbar")?;
    z.start_file("Thumbnails/thumbnail.png", stored).map_err(e)?;
    z.write_all(&png(&t)?).map_err(|x| x.to_string())?;
    Ok(z.finish().map_err(e)?.into_inner())
}

// ---------- PSD ----------

fn demul(pm: &Pixmap) -> [Vec<u8>; 4] {
    let n = (pm.width() * pm.height()) as usize;
    let mut c = [vec![0u8; n], vec![0u8; n], vec![0u8; n], vec![0u8; n]];
    for (i, p) in pm.data().chunks_exact(4).enumerate() {
        let a = p[3] as u32;
        for k in 0..3 {
            c[k][i] = if a == 0 { 0 } else if a == 255 { p[k] } else { ((p[k] as u32 * 255 + a / 2) / a).min(255) as u8 };
        }
        c[3][i] = p[3];
    }
    c
}

/// PackBits einer Zeile.
fn packbits(row: &[u8], out: &mut Vec<u8>) {
    let n = row.len();
    let mut i = 0;
    while i < n {
        let mut run = 1;
        while i + run < n && run < 128 && row[i + run] == row[i] {
            run += 1;
        }
        if run >= 2 {
            out.push((257 - run) as u8);
            out.push(row[i]);
            i += run;
        } else {
            let s = i;
            i += 1;
            while i < n && i - s < 128 && !(i + 1 < n && row[i] == row[i + 1]) {
                i += 1;
            }
            out.push((i - s - 1) as u8);
            out.extend_from_slice(&row[s..i]);
        }
    }
}

/// RLE-Kodierung eines Kanals: (Zeilenlängen, Daten).
fn rle(ch: &[u8], w: usize, h: usize) -> (Vec<u16>, Vec<u8>) {
    let (mut lens, mut data) = (Vec::with_capacity(h), Vec::new());
    for r in 0..h {
        let s = data.len();
        packbits(&ch[r * w..(r + 1) * w], &mut data);
        lens.push((data.len() - s) as u16);
    }
    (lens, data)
}

fn u16b(o: &mut Vec<u8>, v: u16) {
    o.extend_from_slice(&v.to_be_bytes());
}
fn u32b(o: &mut Vec<u8>, v: u32) {
    o.extend_from_slice(&v.to_be_bytes());
}

pub fn psd(layers: &[LayerImg], merged: &Pixmap) -> Result<Vec<u8>, String> {
    let (w, h) = (merged.width(), merged.height());
    if w > 30000 || h > 30000 {
        return Err("Bild zu groß für PSD (max. 30000 px)".into());
    }
    if layers.len() > 1000 {
        return Err("zu viele Ebenen".into());
    }
    let (wu, hu) = (w as usize, h as usize);
    let mut o = Vec::new();
    o.extend_from_slice(b"8BPS");
    u16b(&mut o, 1);
    o.extend_from_slice(&[0; 6]);
    u16b(&mut o, 4);
    u32b(&mut o, h);
    u32b(&mut o, w);
    u16b(&mut o, 8);
    u16b(&mut o, 3); // RGB
    u32b(&mut o, 0); // Farbmoduslänge
    u32b(&mut o, 0); // Bildressourcen

    // Ebeneninfo: Anzahl, Datensätze, Kanaldaten
    let mut recs = Vec::new();
    let mut chan = Vec::new();
    for l in layers {
        let c = demul(&l.pm);
        u32b(&mut recs, 0);
        u32b(&mut recs, 0);
        u32b(&mut recs, h);
        u32b(&mut recs, w);
        u16b(&mut recs, 4);
        let mut blobs: Vec<(i16, Vec<u8>)> = Vec::new();
        for (id, idx) in [(-1i16, 3usize), (0, 0), (1, 1), (2, 2)] {
            let (lens, data) = rle(&c[idx], wu, hu);
            let mut b = Vec::with_capacity(2 + lens.len() * 2 + data.len());
            u16b(&mut b, 1);
            for v in lens {
                u16b(&mut b, v);
            }
            b.extend_from_slice(&data);
            blobs.push((id, b));
        }
        for (id, b) in &blobs {
            recs.extend_from_slice(&id.to_be_bytes());
            u32b(&mut recs, b.len() as u32);
        }
        recs.extend_from_slice(b"8BIMnorm");
        recs.push((l.opacity.clamp(0.0, 1.0) * 255.0).round() as u8);
        recs.push(0); // Clipping
        recs.push(if l.visible { 0 } else { 2 });
        recs.push(0);
        // Name: Pascal-String (ASCII), auf 4 Byte gepolstert
        let nm: Vec<u8> = l.name.chars().map(|c| if c.is_ascii() && !c.is_ascii_control() { c as u8 } else { b'?' }).take(255).collect();
        let mut name = vec![nm.len() as u8];
        name.extend_from_slice(&nm);
        while name.len() % 4 != 0 {
            name.push(0);
        }
        // luni: Unicode-Name
        let u: Vec<u16> = l.name.encode_utf16().collect();
        let mut luni = Vec::new();
        u32b(&mut luni, u.len() as u32);
        for v in &u {
            u16b(&mut luni, *v);
        }
        if luni.len() % 4 != 0 {
            luni.extend_from_slice(&[0; 2]);
        }
        let extra = 4 + 4 + name.len() + 12 + luni.len();
        u32b(&mut recs, extra as u32);
        u32b(&mut recs, 0); // Ebenenmaske
        u32b(&mut recs, 0); // Mischbereiche
        recs.extend_from_slice(&name);
        recs.extend_from_slice(b"8BIMluni");
        u32b(&mut recs, luni.len() as u32);
        recs.extend_from_slice(&luni);
        for (_, b) in &blobs {
            chan.extend_from_slice(b);
        }
    }
    let mut info = Vec::new();
    // negative Anzahl: erster Alphakanal des Composites ist Transparenz
    info.extend_from_slice(&(-(layers.len() as i16)).to_be_bytes());
    info.extend_from_slice(&recs);
    info.extend_from_slice(&chan);
    if info.len() % 2 != 0 {
        info.push(0);
    }
    u32b(&mut o, (4 + info.len() + 4) as u32);
    u32b(&mut o, info.len() as u32);
    o.extend_from_slice(&info);
    u32b(&mut o, 0); // globale Maske

    // Composite: RLE, kanalweise R,G,B,A
    let c = demul(merged);
    let enc: Vec<_> = (0..4).map(|k| rle(&c[k], wu, hu)).collect();
    u16b(&mut o, 1);
    for (lens, _) in &enc {
        for v in lens {
            u16b(&mut o, *v);
        }
    }
    for (_, d) in &enc {
        o.extend_from_slice(d);
    }
    Ok(o)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Read;

    fn layer(name: &str, col: (u8, u8, u8, u8), vis: bool) -> LayerImg {
        let mut pm = Pixmap::new(40, 30).unwrap();
        pm.fill(tiny_skia::Color::from_rgba8(col.0, col.1, col.2, col.3));
        LayerImg { name: name.into(), pm, opacity: 0.5, visible: vis }
    }

    fn sample() -> (Vec<LayerImg>, Pixmap) {
        let ls = vec![layer("Hintergrund", (255, 255, 255, 255), true), layer("Ü <A>", (200, 10, 10, 128), false)];
        let mut m = Pixmap::new(40, 30).unwrap();
        m.fill(tiny_skia::Color::from_rgba8(255, 200, 200, 255));
        (ls, m)
    }

    #[test]
    fn ora_aufbau() {
        let (ls, m) = sample();
        let b = ora(&ls, &m).unwrap();
        let mut z = zip::ZipArchive::new(Cursor::new(b)).unwrap();
        {
            let mut f = z.by_index(0).unwrap();
            assert_eq!(f.name().unwrap(), "mimetype");
            assert_eq!(f.compression(), zip::CompressionMethod::Stored);
            let mut s = String::new();
            f.read_to_string(&mut s).unwrap();
            assert_eq!(s, "image/openraster");
        }
        for n in ["stack.xml", "data/000.png", "data/001.png", "mergedimage.png", "Thumbnails/thumbnail.png"] {
            assert!(z.by_name(n).is_ok(), "fehlt {n}");
        }
        let mut s = String::new();
        z.by_name("stack.xml").unwrap().read_to_string(&mut s).unwrap();
        let d = roxmltree::Document::parse(&s).unwrap();
        let ln: Vec<_> = d.descendants().filter(|n| n.has_tag_name("layer")).collect();
        assert_eq!(ln.len(), 2);
        assert_eq!(ln[0].attribute("name"), Some("Ü <A>")); // oben zuerst
        assert_eq!(ln[0].attribute("visibility"), Some("hidden"));
        assert_eq!(ln[1].attribute("src"), Some("data/000.png"));
    }

    #[test]
    fn psd_aufbau() {
        let (ls, m) = sample();
        let b = psd(&ls, &m).unwrap();
        assert_eq!(&b[..4], b"8BPS");
        let r16 = |o: usize| u16::from_be_bytes([b[o], b[o + 1]]) as usize;
        let r32 = |o: usize| u32::from_be_bytes([b[o], b[o + 1], b[o + 2], b[o + 3]]) as usize;
        assert_eq!(r16(4), 1);
        assert_eq!(r16(12), 4);
        assert_eq!((r32(14), r32(18)), (30, 40));
        assert_eq!((r16(22), r16(24)), (8, 3));
        assert_eq!(r32(26), 0);
        assert_eq!(r32(30), 0);
        let lm = r32(34);
        let li = r32(38);
        assert_eq!(li % 2, 0);
        assert_eq!(i16::from_be_bytes([b[42], b[43]]), -2);
        assert_eq!(lm, 4 + li + 4);
        // Composite beginnt nach der Ebenensektion: RLE (1) und Rest plausibel
        let comp = 38 + lm;
        assert_eq!(r16(comp), 1);
        assert!(b.len() > comp + 2 + 4 * 30 * 2);
        // erster Datensatz: Rechteck und 4 Kanäle
        assert_eq!((r32(44), r32(48), r32(52), r32(56)), (0, 0, 30, 40));
        assert_eq!(r16(60), 4);
        // Summe Kanallängen + Datensätze = Rest der Ebeneninfo
    }

    #[test]
    fn packbits_roundtrip() {
        let row: Vec<u8> = vec![1, 2, 3, 3, 3, 3, 4, 5, 5, 6, 7, 8, 9, 9, 9];
        let mut o = Vec::new();
        packbits(&row, &mut o);
        let (mut r, mut i) = (Vec::new(), 0);
        while i < o.len() {
            let n = o[i] as i8;
            i += 1;
            if n >= 0 {
                r.extend_from_slice(&o[i..i + n as usize + 1]);
                i += n as usize + 1;
            } else {
                r.extend(std::iter::repeat(o[i]).take((1 - n as i32) as usize));
                i += 1;
            }
        }
        assert_eq!(r, row);
    }
}
