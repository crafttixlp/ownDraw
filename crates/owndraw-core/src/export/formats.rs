//! Export-Formate: Liste, Optionen und Erzeugung der Datei (gemeinsam für Desktop, Web und Android).

use super::{interchange, layered, office, pdfv, raster, vector};
use crate::board::{Board, ExportFrame, Scope};
use crate::canvas::{Canvas, VOp};
use crate::geom::RectF;
use std::collections::HashSet;
use tiny_skia::{Pixmap, Transform};

/// Ein Export-Format, das der Kern erzeugt (.owndraw, .xopp und Website-ZIP laufen über die App).
#[derive(Clone, Copy, PartialEq, Eq, Debug, Hash)]
pub enum Fmt {
    Png,
    Jpeg,
    Webp,
    Avif,
    Tiff,
    Bmp,
    Svg,
    /// PDF mit Vektorinhalt.
    Pdf,
    /// PDF mit dem Inhalt als Bild (wie bis 4.30).
    PdfImage,
    PdfA,
    Eps,
    Emf,
    Pptx,
    Docx,
    Excalidraw,
    Drawio,
    Ora,
    Psd,
    Inkml,
    Json,
    /// Website als eine HTML-Datei.
    Html,
    /// Zeichenverlauf als animiertes GIF.
    Gif,
    /// Zeichenverlauf als animiertes PNG.
    Apng,
}

/// Gruppen im Export-Fenster.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Group {
    Bild,
    Vektor,
    Dokument,
    Bearbeitbar,
    Web,
}

pub const ALL: [Fmt; 23] = [
    Fmt::Png,
    Fmt::Jpeg,
    Fmt::Webp,
    Fmt::Avif,
    Fmt::Tiff,
    Fmt::Bmp,
    Fmt::Svg,
    Fmt::Pdf,
    Fmt::PdfImage,
    Fmt::PdfA,
    Fmt::Eps,
    Fmt::Emf,
    Fmt::Pptx,
    Fmt::Docx,
    Fmt::Excalidraw,
    Fmt::Drawio,
    Fmt::Ora,
    Fmt::Psd,
    Fmt::Inkml,
    Fmt::Json,
    Fmt::Html,
    Fmt::Gif,
    Fmt::Apng,
];

impl Fmt {
    /// Dateiendung ohne Punkt.
    pub fn ext(self) -> &'static str {
        match self {
            Fmt::Png => "png",
            Fmt::Jpeg => "jpg",
            Fmt::Webp => "webp",
            Fmt::Avif => "avif",
            Fmt::Tiff => "tiff",
            Fmt::Bmp => "bmp",
            Fmt::Svg => "svg",
            Fmt::Pdf | Fmt::PdfImage | Fmt::PdfA => "pdf",
            Fmt::Eps => "eps",
            Fmt::Emf => "emf",
            Fmt::Pptx => "pptx",
            Fmt::Docx => "docx",
            Fmt::Excalidraw => "excalidraw",
            Fmt::Drawio => "drawio",
            Fmt::Ora => "ora",
            Fmt::Psd => "psd",
            Fmt::Inkml => "inkml",
            Fmt::Json => "json",
            Fmt::Html => "html",
            Fmt::Gif => "gif",
            Fmt::Apng => "png",
        }
    }

    pub fn mime(self) -> &'static str {
        match self {
            Fmt::Png | Fmt::Apng => "image/png",
            Fmt::Jpeg => "image/jpeg",
            Fmt::Webp => "image/webp",
            Fmt::Avif => "image/avif",
            Fmt::Tiff => "image/tiff",
            Fmt::Bmp => "image/bmp",
            Fmt::Gif => "image/gif",
            Fmt::Svg => "image/svg+xml",
            Fmt::Pdf | Fmt::PdfImage | Fmt::PdfA => "application/pdf",
            Fmt::Eps => "application/postscript",
            Fmt::Emf => "image/emf",
            Fmt::Pptx => "application/vnd.openxmlformats-officedocument.presentationml.presentation",
            Fmt::Docx => "application/vnd.openxmlformats-officedocument.wordprocessingml.document",
            Fmt::Excalidraw | Fmt::Json => "application/json",
            Fmt::Drawio => "application/vnd.jgraph.mxfile",
            Fmt::Ora => "image/openraster",
            Fmt::Psd => "image/vnd.adobe.photoshop",
            Fmt::Inkml => "application/inkml+xml",
            Fmt::Html => "text/html",
        }
    }

    /// Name im Export-Fenster.
    pub fn label(self) -> &'static str {
        match self {
            Fmt::Png => "PNG",
            Fmt::Jpeg => "JPEG",
            Fmt::Webp => "WebP",
            Fmt::Avif => "AVIF",
            Fmt::Tiff => "TIFF",
            Fmt::Bmp => "BMP",
            Fmt::Svg => "SVG",
            Fmt::Pdf => "PDF",
            Fmt::PdfImage => "PDF (als Bild)",
            Fmt::PdfA => "PDF/A (Archiv)",
            Fmt::Eps => "EPS",
            Fmt::Emf => "EMF (Office)",
            Fmt::Pptx => "PowerPoint (.pptx)",
            Fmt::Docx => "Word (.docx)",
            Fmt::Excalidraw => "Excalidraw",
            Fmt::Drawio => "draw.io",
            Fmt::Ora => "OpenRaster (.ora)",
            Fmt::Psd => "Photoshop (.psd)",
            Fmt::Inkml => "InkML",
            Fmt::Json => "JSON (Rohdaten)",
            Fmt::Html => "HTML-Datei (Website)",
            Fmt::Gif => "GIF (Zeichenverlauf)",
            Fmt::Apng => "APNG (Zeichenverlauf)",
        }
    }

    /// Kurze Erklärung (Tooltip, Anleitung).
    pub fn hint(self) -> &'static str {
        match self {
            Fmt::Png => "Verlustfrei, mit Transparenz – der Standard",
            Fmt::Jpeg => "Kleine Datei für Mail und Messenger",
            Fmt::Webp => "Modern und klein, gut fürs Web",
            Fmt::Avif => "Sehr klein, neuere Programme",
            Fmt::Tiff => "Für Druck und Archiv",
            Fmt::Bmp => "Unkomprimiert, für alte Programme",
            Fmt::Svg => "Beliebig skalierbar, in Inkscape/Figma bearbeitbar",
            Fmt::Pdf => "Scharf in jeder Größe, eine Seite je Board-Seite",
            Fmt::PdfImage => "Inhalt als Bild, sieht überall gleich aus",
            Fmt::PdfA => "Langzeitarchiv (PDF/A-2b)",
            Fmt::Eps => "Druckvorstufe",
            Fmt::Emf => "Zum Einfügen in Word und PowerPoint (Windows)",
            Fmt::Pptx => "Eine Folie je Seite",
            Fmt::Docx => "Ein Bild je Seite",
            Fmt::Excalidraw => "In excalidraw.com weiterbearbeiten",
            Fmt::Drawio => "In draw.io / diagrams.net weiterbearbeiten",
            Fmt::Ora => "Ebenen für Krita und GIMP",
            Fmt::Psd => "Ebenen für Photoshop",
            Fmt::Inkml => "Handschrift-Standard (Striche mit Punkten)",
            Fmt::Json => "Objekte als Daten für Entwickler",
            Fmt::Html => "Website mit Ebenen und Animationen, alles in einer Datei",
            Fmt::Gif => "Animation: das Board entsteht Strich für Strich",
            Fmt::Apng => "Wie GIF, aber in voller Farbqualität",
        }
    }

    pub fn group(self) -> Group {
        match self {
            Fmt::Png | Fmt::Jpeg | Fmt::Webp | Fmt::Avif | Fmt::Tiff | Fmt::Bmp => Group::Bild,
            Fmt::Svg | Fmt::Eps | Fmt::Emf => Group::Vektor,
            Fmt::Pdf | Fmt::PdfImage | Fmt::PdfA | Fmt::Pptx | Fmt::Docx => Group::Dokument,
            Fmt::Excalidraw | Fmt::Drawio | Fmt::Ora | Fmt::Psd | Fmt::Inkml | Fmt::Json => Group::Bearbeitbar,
            Fmt::Html | Fmt::Gif | Fmt::Apng => Group::Web,
        }
    }

    /// Wirkt die Option „Hintergrund“?
    pub fn uses_background(self) -> bool {
        matches!(self, Fmt::Png | Fmt::Webp | Fmt::Avif | Fmt::Tiff | Fmt::Svg | Fmt::Eps | Fmt::Emf | Fmt::Gif | Fmt::Apng | Fmt::Ora | Fmt::Psd)
    }

    /// Wirkt die Option „Auflösung“?
    pub fn uses_scale(self) -> bool {
        matches!(self, Fmt::Png | Fmt::Jpeg | Fmt::Webp | Fmt::Avif | Fmt::Tiff | Fmt::Bmp | Fmt::PdfImage | Fmt::Pptx | Fmt::Docx | Fmt::Ora | Fmt::Psd)
    }

    /// Wirkt der Bereich (Board/Seite/Auswahl)? Die Website enthält immer das ganze Board.
    pub fn uses_scope(self) -> bool {
        self != Fmt::Html
    }
}

/// Einstellungen eines Exports.
#[derive(Clone, Debug)]
pub struct Opts {
    pub scope: Scope,
    /// Pixel je Welteinheit bei Bildern (1 = 100 %).
    pub scale: f32,
    /// Hintergrund mitexportieren (sonst durchsichtig, wo das Format es kann).
    pub background: bool,
    /// JPEG-Qualität 1..100.
    pub quality: u8,
    /// Titel (Board-Name) für Dokument-Metadaten.
    pub title: String,
}

impl Default for Opts {
    fn default() -> Self {
        Opts { scope: Scope::Board, scale: 2.0, background: true, quality: 90, title: String::new() }
    }
}

/// Abstand zwischen untereinander gesetzten Seiten (Welteinheiten).
const GAP: f32 = 24.0;

/// Erzeugt die Datei. Fehlertexte sind für die Anzeige gedacht.
pub fn build(b: &Board, fmt: Fmt, o: &Opts) -> Result<Vec<u8>, String> {
    if fmt == Fmt::Html {
        return interchange::html_single(b, &o.title).map(String::into_bytes).ok_or_else(|| "Das Board ist noch leer".to_string());
    }
    let frames = b.export_frames(o.scope);
    if frames.is_empty() {
        return Err(match o.scope {
            Scope::Selection => "Es ist nichts ausgewählt",
            _ => "Das Board ist noch leer",
        }
        .into());
    }
    let bg_opaque = b.export_bg();
    match fmt {
        Fmt::Png => stacked_image(b, &frames, o, o.background)?.encode_png().map_err(|e| e.to_string()),
        Fmt::Jpeg => raster::encode(&stacked_image(b, &frames, o, true)?, raster::Raster::Jpeg { quality: o.quality.clamp(1, 100) }, bg_opaque),
        Fmt::Webp => raster::encode(&stacked_image(b, &frames, o, o.background)?, raster::Raster::Webp, bg_opaque),
        Fmt::Avif => raster::encode(&stacked_image(b, &frames, o, o.background)?, raster::Raster::Avif, bg_opaque),
        Fmt::Tiff => raster::encode(&stacked_image(b, &frames, o, o.background)?, raster::Raster::Tiff, bg_opaque),
        Fmt::Bmp => raster::encode(&stacked_image(b, &frames, o, true)?, raster::Raster::Bmp, bg_opaque),
        Fmt::Svg | Fmt::Eps | Fmt::Emf => {
            let (ops, w, h) = stacked_ops(b, &frames, o);
            Ok(match fmt {
                Fmt::Svg => vector::svg(&ops, w, h).into_bytes(),
                Fmt::Eps => vector::eps(&ops, w, h).into_bytes(),
                _ => vector::emf(&ops, w, h),
            })
        }
        Fmt::Pdf | Fmt::PdfA => {
            let pages: Vec<(Vec<VOp>, f32, f32)> = frames.iter().map(|f| (b.record_frame(o.scope, f, true), f.width(), f.height())).collect();
            Ok(pdfv::pdf(&pages, fmt == Fmt::PdfA, &o.title))
        }
        Fmt::PdfImage => {
            let k = o.scale.clamp(1.0, 4.0);
            super::pdf_pages(frames.len(), |i| {
                let f = frames.get(i)?;
                let kk = Board::frame_scale(f, k);
                Some((b.render_frame(o.scope, f, kk, true)?, f.width(), f.height()))
            })
            .ok_or_else(|| "PDF konnte nicht erstellt werden".to_string())
        }
        Fmt::Pptx | Fmt::Docx => {
            let mut pages = Vec::new();
            for f in &frames {
                let k = Board::frame_scale(f, o.scale.clamp(1.0, 4.0));
                let pm = b.render_frame(o.scope, f, k, true).ok_or("Bild konnte nicht erstellt werden")?;
                pages.push(office::PageImg { png: pm.encode_png().map_err(|e| e.to_string())?, w: f.width(), h: f.height() });
            }
            if fmt == Fmt::Pptx {
                office::pptx(&pages, &o.title)
            } else {
                office::docx(&pages, &o.title)
            }
        }
        Fmt::Ora | Fmt::Psd => {
            let f = union(&frames);
            let k = Board::frame_scale(&f, o.scale);
            let merged = b.render_frame(o.scope, &f, k, o.background).ok_or("Bild konnte nicht erstellt werden")?;
            let mut layers = Vec::new();
            if o.background {
                // Hintergrund als eigene unterste Ebene
                let bgm = b.render_frame_ids(&f, k, true, Some(HashSet::new())).ok_or("Bild konnte nicht erstellt werden")?;
                layers.push(layered::LayerImg { name: "Hintergrund".into(), pm: bgm, opacity: 1.0, visible: true });
            }
            for (i, l) in b.layers.iter().enumerate() {
                let pm = b.render_frame_layer(o.scope, &f, i, k).ok_or("Ebene konnte nicht erstellt werden")?;
                layers.push(layered::LayerImg { name: l.name.clone(), pm, opacity: l.opacity.clamp(0.0, 1.0), visible: l.visible });
            }
            if fmt == Fmt::Ora {
                layered::ora(&layers, &merged)
            } else {
                layered::psd(&layers, &merged)
            }
        }
        Fmt::Excalidraw | Fmt::Drawio | Fmt::Inkml | Fmt::Json => {
            let f = union(&frames);
            let items = b.export_items(o.scope, &f);
            Ok(match fmt {
                Fmt::Excalidraw => interchange::excalidraw(b, &items, &f),
                Fmt::Drawio => interchange::drawio(b, &items, &f, &o.title),
                Fmt::Inkml => interchange::inkml(b, &items, &f),
                _ => interchange::json(b, &items, &f, &o.title),
            }
            .into_bytes())
        }
        Fmt::Gif | Fmt::Apng => {
            let frames_px = progress_frames(b, &frames, o, if fmt == Fmt::Gif { 960.0 } else { 1600.0 })?;
            if fmt == Fmt::Gif {
                raster::gif_anim(&frames_px, 120)
            } else {
                raster::apng(&frames_px, 120)
            }
        }
        Fmt::Html => unreachable!(),
    }
}

/// Hülle mehrerer Ausschnitte (ein Ausschnitt bleibt unverändert, mit Seite).
fn union(frames: &[ExportFrame]) -> ExportFrame {
    if frames.len() == 1 {
        return frames[0];
    }
    let mut r = frames[0].rect;
    for f in &frames[1..] {
        r.union(&f.rect);
    }
    ExportFrame { rect: r, page: None }
}

/// Lage der Ausschnitte untereinander (links bündig, Abstand GAP): (x, y) je Ausschnitt und Gesamtgröße.
fn stack_layout(frames: &[ExportFrame]) -> (Vec<(f32, f32)>, f32, f32) {
    let (mut y, mut w) = (0.0f32, 0.0f32);
    let mut pos = Vec::new();
    for (i, f) in frames.iter().enumerate() {
        if i > 0 {
            y += GAP;
        }
        pos.push((0.0, y));
        y += f.height();
        w = w.max(f.width());
    }
    (pos, w, y)
}

/// Alle Ausschnitte als ein Bild (mehrere Seiten untereinander).
fn stacked_image(b: &Board, frames: &[ExportFrame], o: &Opts, with_bg: bool) -> Result<Pixmap, String> {
    let fail = || "Bild konnte nicht erstellt werden (zu groß?)".to_string();
    if frames.len() == 1 {
        let k = Board::frame_scale(&frames[0], o.scale);
        return b.render_frame(o.scope, &frames[0], k, with_bg).ok_or_else(fail);
    }
    let (pos, w, h) = stack_layout(frames);
    let all = ExportFrame { rect: RectF::new(0.0, 0.0, w, h), page: None };
    let k = Board::frame_scale(&all, o.scale);
    let mut out = Pixmap::new(((w * k).round() as u32).max(1), ((h * k).round() as u32).max(1)).ok_or_else(fail)?;
    if with_bg {
        out.fill(crate::canvas::sk_color(b.export_desk()));
    }
    {
        let mut c = Canvas::new(&mut out);
        for (f, (x, y)) in frames.iter().zip(pos) {
            let pm = b.render_frame(o.scope, f, k, with_bg).ok_or_else(fail)?;
            c.draw_pixmap(pm.as_ref(), (x * k).round(), (y * k).round(), (x * k).round() + pm.width() as f32, (y * k).round() + pm.height() as f32);
        }
    }
    Ok(out)
}

/// Vektor-Befehle aller Ausschnitte untereinander.
fn stacked_ops(b: &Board, frames: &[ExportFrame], o: &Opts) -> (Vec<VOp>, f32, f32) {
    if frames.len() == 1 {
        let f = &frames[0];
        return (b.record_frame(o.scope, f, o.background), f.width(), f.height());
    }
    let (pos, w, h) = stack_layout(frames);
    let mut ops = Vec::new();
    for (f, (x, y)) in frames.iter().zip(pos) {
        ops.push(VOp::Save);
        for op in b.record_frame(o.scope, f, o.background) {
            ops.push(translate(op, x, y));
        }
        ops.push(VOp::Restore);
    }
    (ops, w, h)
}

fn translate(op: VOp, dx: f32, dy: f32) -> VOp {
    let t = Transform::from_translate(dx, dy);
    let tp = |p: tiny_skia::Path| p.clone().transform(t).unwrap_or(p);
    match op {
        VOp::Fill(p, c) => VOp::Fill(tp(p), c),
        VOp::Stroke { path, color, width, round_cap, dash } => VOp::Stroke { path: tp(path), color, width, round_cap, dash },
        VOp::Image(pm, m) => VOp::Image(pm, m.post_concat(t)),
        VOp::Clip(p) => VOp::Clip(tp(p)),
        other => other,
    }
}

/// Bilder des Zeichenverlaufs: das Board entsteht Objekt für Objekt (höchstens 48 Schritte, Ende länger stehend).
fn progress_frames(b: &Board, frames: &[ExportFrame], o: &Opts, max_edge: f32) -> Result<Vec<Pixmap>, String> {
    let f = union(frames);
    let items = b.export_items(o.scope, &f);
    if items.is_empty() {
        return Err(if matches!(o.scope, Scope::Page(_)) { "Auf dieser Seite ist nichts gezeichnet" } else { "Das Board ist noch leer" }.into());
    }
    let k = Board::frame_scale(&f, (max_edge / f.width().max(f.height())).min(o.scale.max(1.0)));
    let steps = items.len().min(48);
    let mut out = Vec::new();
    for s in 0..=steps {
        let n = (items.len() * s).div_ceil(steps);
        let ids: HashSet<u64> = items[..n].iter().map(|it| it.id).collect();
        out.push(b.render_frame_ids(&f, k, o.background, Some(ids)).ok_or("Bild konnte nicht erstellt werden")?);
    }
    // Endbild länger zeigen
    if let Some(last) = out.last().cloned() {
        for _ in 0..12 {
            out.push(last.clone());
        }
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::item::{Item, Kind, ARROW, POLY};
    use crate::pages::{BoardMode, PageBg, PageSetup};
    use std::rc::Rc;

    fn board() -> Board {
        let mut b = Board::new(1.0);
        let mut s = Item::stroke(0x80CC0000, 6.0, true, false);
        if let Kind::Stroke(st) = &mut s.kind {
            st.p = vec![10.0, 10.0, 0.5, 60.0, 40.0, 0.6, 120.0, 12.0, 0.7];
        }
        b.items.push(Rc::new(s));
        let mut r = Item::shape(POLY, vec![50.0, 50.0, 150.0, 50.0, 150.0, 120.0, 50.0, 120.0], 0xFF000000, 2.0);
        r.fill = Some(0xFFFFFF00);
        b.items.push(Rc::new(r));
        b.items.push(Rc::new(Item::shape(ARROW, vec![0.0, 200.0, 100.0, 220.0], 0xFF0000FF, 2.0)));
        b
    }

    fn check(b: &Board, scope: Scope) {
        let o = Opts { scope, scale: 1.0, title: "Test".into(), ..Opts::default() };
        for f in ALL {
            let d = build(b, f, &o).unwrap_or_else(|e| panic!("{f:?} {scope:?}: {e}"));
            assert!(d.len() > 20, "{f:?} {scope:?}: nur {} Bytes", d.len());
            match f {
                Fmt::Png | Fmt::Apng => assert!(d.starts_with(b"\x89PNG")),
                Fmt::Jpeg => assert!(d.starts_with(&[0xFF, 0xD8])),
                Fmt::Pdf | Fmt::PdfA | Fmt::PdfImage => assert!(d.starts_with(b"%PDF")),
                Fmt::Svg => assert!(String::from_utf8_lossy(&d).contains("<svg")),
                Fmt::Pptx | Fmt::Docx | Fmt::Ora => assert!(d.starts_with(b"PK")),
                Fmt::Psd => assert!(d.starts_with(b"8BPS")),
                Fmt::Gif => assert!(d.starts_with(b"GIF8")),
                _ => {}
            }
        }
    }

    #[test]
    fn alle_formate_board_und_auswahl() {
        let mut b = board();
        check(&b, Scope::Board);
        b.sel = vec![b.items[1].id];
        check(&b, Scope::Selection);
        // Auswahl: Ausschnitt nur um das Rechteck
        let f = b.export_frames(Scope::Selection);
        assert_eq!(f.len(), 1);
        assert!(f[0].rect.left > 20.0 && f[0].rect.right < 200.0);
        assert_eq!(b.export_items(Scope::Selection, &f[0]).len(), 1);
    }

    #[test]
    fn alle_formate_seiten() {
        let mut b = board();
        b.board_mode = BoardMode::Pages(PageSetup::new(595.0, 842.0, PageBg::Lined, 2));
        assert_eq!(b.export_frames(Scope::Board).len(), 2);
        check(&b, Scope::Board);
        check(&b, Scope::Page(0));
        let o = Opts { scope: Scope::Page(1), ..Opts::default() };
        assert!(build(&b, Fmt::Gif, &o).is_err() && build(&b, Fmt::Png, &o).is_ok());
        // PDF: eine Seite je Board-Seite
        let pdf = build(&b, Fmt::Pdf, &Opts::default()).unwrap();
        let s = String::from_utf8_lossy(&pdf);
        assert_eq!(s.matches("/Type /Page\n").count() + s.matches("/Type /Page ").count() + s.matches("/Type/Page/").count() + s.matches("/Type /Page>").count() + s.matches("/Type/Page>").count(), 2, "Seitenzahl");
    }

    #[test]
    fn leer_und_ohne_auswahl() {
        let b = Board::new(1.0);
        assert!(build(&b, Fmt::Png, &Opts::default()).is_err());
        let b = board();
        let o = Opts { scope: Scope::Selection, ..Opts::default() };
        assert_eq!(build(&b, Fmt::Svg, &o).unwrap_err(), "Es ist nichts ausgewählt");
    }

    #[test]
    fn eindeutige_namen() {
        let mut seen = HashSet::new();
        for f in ALL {
            assert!(seen.insert(f.label()), "doppelt: {}", f.label());
        }
    }
}

/// Beispieldateien aller Formate zum Ansehen: `OWNDRAW_EXPORT_DIR=/tmp/x cargo test -p owndraw-core --lib export_beispiele -- --ignored`
#[cfg(test)]
#[test]
#[ignore]
fn export_beispiele() {
    let dir = std::path::PathBuf::from(std::env::var("OWNDRAW_EXPORT_DIR").expect("OWNDRAW_EXPORT_DIR"));
    std::fs::create_dir_all(&dir).unwrap();
    let mut b = Board::new(1.0);
    for (i, c) in [0xFF3355DD_u32, 0x80EE6622, 0xFF22AA55].iter().enumerate() {
        let mut s = crate::item::Item::stroke(*c, 4.0 + 6.0 * i as f32, i == 1, false);
        if let crate::item::Kind::Stroke(st) = &mut s.kind {
            st.p = (0..30).flat_map(|k| [20.0 + k as f32 * 12.0, 60.0 + 50.0 * i as f32 + 15.0 * (k as f32 * 0.5).sin(), 0.6]).collect();
        }
        b.items.push(std::rc::Rc::new(s));
    }
    let mut r = crate::item::Item::shape(crate::item::POLY, vec![60.0, 220.0, 260.0, 220.0, 260.0, 320.0, 60.0, 320.0], 0xFF000000, 3.0);
    r.fill = Some(0xFFFFE066);
    b.items.push(std::rc::Rc::new(r));
    b.items.push(std::rc::Rc::new(crate::item::Item::shape(crate::item::ARROW, vec![300.0, 270.0, 420.0, 230.0], 0xFFCC0000, 3.0)));
    b.items.push(std::rc::Rc::new(crate::item::Item::note("Hallo Export!", 0xFF111111, 80.0, 360.0, 24.0, 300.0, 0.0, 0)));
    let o = Opts { title: "Beispiel".into(), ..Opts::default() };
    for f in ALL {
        let d = build(&b, f, &o).unwrap();
        std::fs::write(dir.join(format!("beispiel-{:?}.{}", f, f.ext()).to_lowercase()), d).unwrap();
    }
}
