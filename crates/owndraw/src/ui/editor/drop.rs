//! Drag & Drop in den Editor/die App (WP10).
//!
//! `handle_drops` wird pro Frame aufgerufen (auch auf dem Startbildschirm, dann mit `editor = None`),
//! verarbeitet `platform.take_drops` und zeigt den Ablage-Hinweis, solange `platform.drag_hover` gilt.
//! Die Entscheidung je Datei trifft die reine Funktion [`decide`]. Rückgabe: Aufträge an die Shell.

use super::{clip, Editor};
use crate::platform::{FileKind, FileRef, Platform};
use crate::style::{self, c};
use egui::{Align2, Color32, Pos2, Rect, Stroke};

pub enum DropOutcome {
    /// `.owndraw`/`.xopp` öffnen.
    Open(FileRef),
    /// Startbildschirm: neues Board mit diesem Bild (Dateiinhalt).
    NewWithImage(Vec<u8>),
    Toast(String),
}

/// Was mit einer abgelegten Datei geschieht.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Decision {
    Open,
    InsertImage,
    NewWithImage,
    Toast(&'static str),
}

const MSG_PDF: &str = "PDF wird noch nicht unterstützt";
const MSG_TYPE: &str = "Dieser Dateityp wird nicht unterstützt";
const MSG_READ: &str = "Datei konnte nicht gelesen werden";

/// Dateiart nach Name, bei unbekannter Endung nach den ersten Bytes (z. B. Bild aus dem Browser).
pub fn kind_of(name: &str, head: Option<&[u8]>) -> FileKind {
    let k = FileKind::of_name(name);
    if k != FileKind::Any {
        return k;
    }
    match head {
        Some(h) if h.starts_with(b"%PDF") => FileKind::Pdf,
        Some(h) if image::guess_format(h).is_ok() => FileKind::Image,
        _ => FileKind::Any,
    }
}

/// Reine Entscheidung je Datei. `has_file`: Verweis vorhanden (zum Öffnen nötig), `has_editor`: Board offen.
pub fn decide(kind: FileKind, has_file: bool, has_editor: bool) -> Decision {
    match kind {
        FileKind::Owndraw | FileKind::Xopp if has_file => Decision::Open,
        FileKind::Owndraw | FileKind::Xopp => Decision::Toast(MSG_READ),
        FileKind::Image | FileKind::Png if has_editor => Decision::InsertImage,
        FileKind::Image | FileKind::Png => Decision::NewWithImage,
        FileKind::Pdf => Decision::Toast(MSG_PDF),
        // Startbildschirm: alles übrige zum Öffnen versuchen (meldet selbst, wenn es kein Board ist)
        _ if !has_editor && has_file => Decision::Open,
        _ => Decision::Toast(MSG_TYPE),
    }
}

/// Abgelegtes Bild: Inhalt und Drop-Position.
type PendingImage = (Vec<u8>, Option<(f32, f32)>);

/// Setzt abgelegte Bilder ins Board; mehrere werden versetzt. Liefert Fehlertexte.
fn insert_images(e: &mut Editor, images: &[PendingImage]) -> Vec<String> {
    let mut errs = Vec::new();
    for (k, (bytes, pos)) in images.iter().enumerate() {
        let base = pos.and_then(|(x, y)| clip::world_at(e, Pos2::new(x, y)));
        let off = 24.0 * k as f32 / e.board.sc.max(0.01);
        let at = base.map(|(x, y)| (x + off, y + off));
        if let Err(m) = clip::insert_image_bytes(e, bytes, at) {
            errs.push(m);
        }
    }
    errs
}

/// Verarbeitet bereitgestellte Drops (ohne Oberfläche, testbar).
pub fn route_drops(platform: &dyn Platform, drops: Vec<crate::platform::Drop>, editor: Option<&mut Editor>) -> Vec<DropOutcome> {
    let mut out = Vec::new();
    let mut images: Vec<PendingImage> = Vec::new();
    for d in drops {
        let name = if d.name.is_empty() { d.file.as_ref().map(|f| f.display_name()).unwrap_or_default() } else { d.name.clone() };
        let kind = kind_of(&name, d.bytes.as_deref());
        match decide(kind, d.file.is_some(), editor.is_some()) {
            Decision::Open => out.extend(d.file.map(DropOutcome::Open)),
            Decision::InsertImage | Decision::NewWithImage => {
                let bytes = d.bytes.clone().or_else(|| d.file.as_ref().and_then(|f| platform.read(f).ok()));
                let Some(b) = bytes else {
                    out.push(DropOutcome::Toast(MSG_READ.into()));
                    continue;
                };
                if editor.is_some() {
                    images.push((b, d.pos));
                } else {
                    out.push(DropOutcome::NewWithImage(b));
                    // weitere Dateien beim Start: nur das erste Bild öffnet ein Board
                    break;
                }
            }
            Decision::Toast(m) => out.push(DropOutcome::Toast(m.into())),
        }
    }
    if let (Some(e), false) = (editor, images.is_empty()) {
        out.extend(insert_images(e, &images).into_iter().map(DropOutcome::Toast));
    }
    out
}

pub fn handle_drops(ctx: &egui::Context, platform: &dyn Platform, editor: Option<&mut Editor>) -> Vec<DropOutcome> {
    let out = route_drops(platform, platform.take_drops(ctx), editor);
    if platform.drag_hover(ctx) || std::env::var("OWNDRAW_DROP_HINT").map(|v| v == "1").unwrap_or(false) {
        overlay(ctx);
    }
    out
}

/// Ablage-Hinweis: abgedunkelte Fläche, gestrichelter Rahmen in Akzentfarbe, Karte mit Text.
fn overlay(ctx: &egui::Context) {
    let p = style::pal();
    let screen = ctx.content_rect();
    let accent = c(p.accent);
    #[allow(deprecated)]
    let painter = ctx.layer_painter(egui::LayerId::new(egui::Order::Tooltip, egui::Id::new("drop-hint")));
    let scrim = if p.dark { Color32::from_black_alpha(140) } else { Color32::from_black_alpha(70) };
    painter.rect_filled(screen, 0.0, scrim);
    let r = screen.shrink(14.0);
    painter.rect_filled(r, 18.0, Color32::from_rgba_unmultiplied(accent.r(), accent.g(), accent.b(), 28));
    painter.extend(egui::Shape::dashed_line(&rounded_outline(r, 18.0), Stroke::new(3.0, accent), 14.0, 9.0));
    let card = Rect::from_center_size(screen.center(), egui::vec2(300.0_f32.min(screen.width() - 40.0), 96.0));
    painter.rect(card, 18.0, c(p.card), Stroke::new(1.0, c(p.line)), egui::StrokeKind::Inside);
    painter.text(card.center() - egui::vec2(0.0, 12.0), Align2::CENTER_CENTER, "Zum Einfügen loslassen", egui::FontId::proportional(18.0), c(p.text));
    painter.text(card.center() + egui::vec2(0.0, 16.0), Align2::CENTER_CENTER, "Bild, .owndraw oder .xopp", egui::FontId::proportional(13.0), c(p.dim));
}

/// Geschlossener Linienzug eines abgerundeten Rechtecks (für gestrichelte Linie).
fn rounded_outline(r: Rect, rad: f32) -> Vec<Pos2> {
    let mut v = Vec::new();
    let corners = [
        (Pos2::new(r.right() - rad, r.top() + rad), -90.0f32),
        (Pos2::new(r.right() - rad, r.bottom() - rad), 0.0),
        (Pos2::new(r.left() + rad, r.bottom() - rad), 90.0),
        (Pos2::new(r.left() + rad, r.top() + rad), 180.0),
    ];
    for (ctr, a0) in corners {
        for i in 0..=8 {
            let a = (a0 + i as f32 * 90.0 / 8.0).to_radians();
            v.push(ctr + egui::vec2(a.cos(), a.sin()) * rad);
        }
    }
    v.push(v[0]);
    v
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::platform::stub::StubPlatform;
    use crate::platform::Drop;
    use std::path::PathBuf;

    #[test]
    fn router_je_dateiname() {
        let k = |n: &str| kind_of(n, None);
        for n in ["a.png", "A.JPG", "x.jpeg", "y.webp", "z.gif", "w.bmp"] {
            assert_eq!(decide(k(n), true, true), Decision::InsertImage, "{n}");
            assert_eq!(decide(k(n), true, false), Decision::NewWithImage, "{n}");
        }
        assert_eq!(decide(k("b.owndraw"), true, true), Decision::Open);
        assert_eq!(decide(k("b.xopp"), true, false), Decision::Open);
        assert_eq!(decide(k("b.xopp"), false, true), Decision::Toast(MSG_READ));
        assert_eq!(decide(k("s.pdf"), true, true), Decision::Toast(MSG_PDF));
        assert_eq!(decide(k("t.txt"), true, true), Decision::Toast(MSG_TYPE));
        assert_eq!(decide(k("t.txt"), true, false), Decision::Open);
        assert_eq!(decide(k("t.txt"), false, false), Decision::Toast(MSG_TYPE));
    }

    #[test]
    fn router_nach_bytes() {
        let png = [0x89, b'P', b'N', b'G', 0x0D, 0x0A, 0x1A, 0x0A, 0, 0, 0, 0];
        assert_eq!(kind_of("image", Some(&png)), FileKind::Image);
        assert_eq!(kind_of("doc", Some(b"%PDF-1.7")), FileKind::Pdf);
        assert_eq!(kind_of("doc", Some(b"hallo")), FileKind::Any);
        assert_eq!(kind_of("a.png", Some(b"hallo")), FileKind::Image);
    }

    fn png_bytes(w: u32) -> Vec<u8> {
        let mut o = Vec::new();
        image::DynamicImage::ImageRgba8(image::RgbaImage::from_pixel(w, w, image::Rgba([200, 30, 30, 255])))
            .write_to(&mut std::io::Cursor::new(&mut o), image::ImageFormat::Png)
            .unwrap();
        o
    }

    #[test]
    fn simulierte_drops_xopp_und_png() {
        let pf = StubPlatform::new();
        let xopp = PathBuf::from(concat!(env!("CARGO_MANIFEST_DIR"), "/../owndraw-core/tests/xopp/strokes.xopp"));
        assert!(xopp.is_file());
        let drops = vec![
            Drop { file: Some(FileRef::Path(xopp.clone())), bytes: None, name: "strokes.xopp".into(), pos: None },
            Drop { file: None, bytes: Some(png_bytes(20)), name: "bild.png".into(), pos: Some((300.0, 300.0)) },
            Drop { file: None, bytes: Some(png_bytes(30)), name: "zwei.png".into(), pos: Some((300.0, 300.0)) },
            Drop { file: None, bytes: None, name: "x.pdf".into(), pos: None },
        ];
        // mit Editor: .xopp öffnen, zwei Bilder (versetzt), PDF-Hinweis
        let mut ed = super::super::clip::tests::editor(false);
        ed.canvas = Rect::from_min_size(Pos2::ZERO, egui::vec2(800.0, 600.0));
        let out = route_drops(&pf, drops.clone(), Some(&mut ed));
        assert!(matches!(&out[0], DropOutcome::Open(f) if *f == FileRef::Path(xopp.clone())));
        assert!(matches!(&out[1], DropOutcome::Toast(m) if m.contains("PDF")));
        assert_eq!(out.len(), 2);
        assert_eq!(ed.board.items.len(), 2);
        let (a, b) = (ed.board.items[0].bounds(), ed.board.items[1].bounds());
        assert!((a.center_x() - b.center_x()).abs() > 1.0, "Bilder müssen versetzt liegen");
        // Startbildschirm: Bild -> neues Board, .xopp -> öffnen
        let out = route_drops(&pf, drops[..2].to_vec(), None);
        assert!(matches!(&out[0], DropOutcome::Open(_)));
        assert!(matches!(&out[1], DropOutcome::NewWithImage(b) if !b.is_empty()));
    }
}
