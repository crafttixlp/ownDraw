//! Export-Fenster (Knopf „Exportieren“ in der Kachel oben rechts): Bereich, Auflösung, Hintergrund und alle Formate,
//! dazu Teilen, Drucken und Kopieren.

use super::{Editor, Lay, M};
use crate::app::{PickPurpose, Req, Shared};
use crate::layout::min_touch;
use crate::platform::{ClipData, FileKind};
use crate::style::{self, c};
use egui::{Align2, Order, Pos2, Rect, Response, Sense, Ui, Vec2};
use owndraw_core::board::Scope;
use owndraw_core::export::formats::{self, Fmt, Opts};
use owndraw_core::icons as ic;

/// Zustand des Export-Fensters (bleibt zwischen zwei Exporten erhalten).
pub struct ExportUi {
    pub open: bool,
    pub btn: Option<Rect>,
    /// 0 = ganzes Board, 1 = aktuelle Seite, 2 = Auswahl.
    pub scope: u8,
    /// Pixel je Welteinheit bei Bildern.
    pub scale: f32,
    pub background: bool,
}

impl Default for ExportUi {
    fn default() -> Self {
        ExportUi { open: false, btn: None, scope: 0, scale: 2.0, background: true }
    }
}

/// Ein Eintrag im Export-Fenster.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Entry {
    /// Vom Kern erzeugtes Format.
    Core(Fmt),
    Owndraw,
    Xopp,
    WebsiteZip,
    Preview,
    LayerPng,
    Share,
    Print,
    CopyImage,
    CopySvg,
}

use Entry::*;

/// Abschnitte des Fensters (Reihenfolge = Anzeige; die häufigsten Formate zuerst).
pub const SECTIONS: &[(&str, &[Entry])] = &[
    ("Häufig", &[Core(Fmt::Png), Core(Fmt::Jpeg), Core(Fmt::Pdf)]),
    ("Bild", &[Core(Fmt::Webp), Core(Fmt::Avif), Core(Fmt::Tiff), Core(Fmt::Bmp)]),
    ("Dokument", &[Core(Fmt::PdfImage), Core(Fmt::PdfA), Core(Fmt::Pptx), Core(Fmt::Docx)]),
    ("Vektor", &[Core(Fmt::Svg), Core(Fmt::Eps), Core(Fmt::Emf)]),
    ("Ebenen und Web", &[WebsiteZip, Core(Fmt::Html), Preview, LayerPng, Core(Fmt::Gif), Core(Fmt::Apng)]),
    ("Bearbeitbar", &[Owndraw, Xopp, Core(Fmt::Excalidraw), Core(Fmt::Drawio), Core(Fmt::Ora), Core(Fmt::Psd), Core(Fmt::Inkml), Core(Fmt::Json)]),
    ("Weitergeben", &[Share, Print, CopyImage, CopySvg]),
];

impl Entry {
    pub fn label(self) -> &'static str {
        match self {
            Core(f) => f.label(),
            Owndraw => "OwnDraw-Datei",
            Xopp => "Xournal++",
            WebsiteZip => "Ebenen und Website (ZIP)",
            Preview => "Website-Vorschau",
            LayerPng => "Aktive Ebene (PNG)",
            Share => "Teilen …",
            Print => "Drucken …",
            CopyImage => "Als Bild kopieren",
            CopySvg => "Als SVG kopieren",
        }
    }

    /// Text rechts in der Zeile (Dateiendung).
    fn right(self) -> String {
        match self {
            Core(f) => format!(".{}", f.ext()),
            Owndraw => ".owndraw".into(),
            Xopp => ".xopp".into(),
            WebsiteZip => ".zip".into(),
            LayerPng => ".png".into(),
            _ => String::new(),
        }
    }

    pub fn hint(self) -> &'static str {
        match self {
            Core(f) => f.hint(),
            Owndraw => "Das ganze Board zum Weiterbearbeiten in OwnDraw",
            Xopp => "Für Xournal++ (Striche, Text, Bilder)",
            WebsiteZip => "Alle Ebenen als PNG plus eine animierte Webseite",
            Preview => "Die Website gleich im Browser ansehen",
            LayerPng => "Nur die aktive Ebene als Bild",
            Share => "Als PNG an andere Apps senden",
            Print => "Als PDF drucken",
            CopyImage => "PNG in die Zwischenablage",
            CopySvg => "SVG-Text in die Zwischenablage (z. B. für Inkscape oder Office)",
        }
    }

    fn icon(self) -> i32 {
        match self {
            Core(f) => match f.group() {
                formats::Group::Bild => ic::IMAGE,
                formats::Group::Vektor => ic::PEN,
                formats::Group::Dokument => ic::EXPORT,
                formats::Group::Bearbeitbar => ic::COPY,
                formats::Group::Web => ic::PLAY,
            },
            Owndraw | Xopp => ic::COPY,
            WebsiteZip | LayerPng => ic::LAYERS,
            Preview => ic::PLAY,
            Share => ic::SHARE,
            Print => ic::EXPORT,
            CopyImage => ic::IMAGE,
            CopySvg => ic::COPY,
        }
    }

    /// Auf dieser Plattform verfügbar?
    fn available(self, sh: &Shared) -> bool {
        match self {
            Preview => !crate::platform::WEB,
            Share => sh.platform.can_share(),
            _ => true,
        }
    }
}

/// Alle Einträge (für die Anleitung: jeder braucht dort einen Eintrag).
#[cfg_attr(not(test), allow(dead_code))]
pub fn all_entries() -> impl Iterator<Item = Entry> {
    SECTIONS.iter().flat_map(|(_, es)| es.iter().copied())
}

impl Editor {
    /// Optionen für den Kern aus dem Fenster-Zustand.
    pub(super) fn export_opts(&self) -> Opts {
        let x = &self.st.export;
        let scope = match x.scope {
            1 if self.board.pages().is_some() => Scope::Page(super::pages::current_page(&self.board)),
            2 if !self.board.sel.is_empty() => Scope::Selection,
            _ => Scope::Board,
        };
        Opts { scope, scale: x.scale, background: x.background, quality: 90, title: self.doc.meta.name.clone() }
    }

    /// Dateiname ohne Endung, mit Bereich (Auswahl/Seite).
    fn export_base(&self, o: &Opts) -> String {
        let base = crate::app::safe_file_name(&self.doc.meta.name);
        match o.scope {
            Scope::Selection => format!("{base}-auswahl"),
            Scope::Page(i) => format!("{base}-seite-{}", i + 1),
            Scope::Board => base,
        }
    }

    /// Format wählen: Speicherziel erfragen (Web: Download).
    pub(super) fn export_fmt(&mut self, sh: &mut Shared, fmt: Fmt) {
        let o = self.export_opts();
        if fmt.uses_scope() && self.board.export_frames(o.scope).is_empty() {
            sh.toast(if o.scope == Scope::Selection { "Es ist nichts ausgewählt" } else { "Das Board ist noch leer" });
            return;
        }
        let name = format!("{}.{}", self.export_base(&o), fmt.ext());
        sh.reqs.push(Req::PickSave { purpose: PickPurpose::Export(fmt), name, kind: FileKind::Export(fmt) });
    }

    /// Datei für das gewählte Format erzeugen (nach dem Speicherdialog).
    pub(super) fn export_bytes(&self, fmt: Fmt) -> Result<Vec<u8>, String> {
        formats::build(&self.board, fmt, &self.export_opts())
    }

    fn run_entry(&mut self, sh: &mut Shared, e: Entry) {
        match e {
            Core(f) => self.export_fmt(sh, f),
            Owndraw => {
                let name = format!("{}.owndraw", crate::app::safe_file_name(self.name()));
                sh.reqs.push(Req::PickSave { purpose: PickPurpose::SaveCopy, name, kind: FileKind::Owndraw });
            }
            Xopp => {
                let name = format!("{}.xopp", crate::app::safe_file_name(self.name()));
                sh.reqs.push(Req::PickSave { purpose: PickPurpose::ExportXopp, name, kind: FileKind::Xopp });
            }
            WebsiteZip => self.export(sh, 2),
            Preview => self.preview(sh),
            LayerPng => {
                let act = self.board.active.min(self.board.layers.len().saturating_sub(1));
                self.export_layer(sh, act);
            }
            Share => {
                let o = self.export_opts();
                match formats::build(&self.board, Fmt::Png, &o) {
                    Ok(png) => {
                        let name = format!("{}.png", self.export_base(&o));
                        if let Err(e) = sh.platform.share_file(&name, "image/png", &png) {
                            sh.toast(&e);
                        }
                    }
                    Err(e) => sh.toast(&e),
                }
            }
            Print => {
                let o = self.export_opts();
                match formats::build(&self.board, Fmt::Pdf, &o) {
                    Ok(pdf) => {
                        let name = format!("{}.pdf", self.export_base(&o));
                        match sh.platform.print_pdf(&name, &pdf) {
                            Ok(msg) => {
                                if !msg.is_empty() {
                                    sh.toast(&msg);
                                }
                            }
                            Err(e) => sh.toast(&e),
                        }
                    }
                    Err(e) => sh.toast(&e),
                }
            }
            CopyImage => {
                let mut o = self.export_opts();
                o.scale = o.scale.min(2.0);
                let frames = self.board.export_frames(o.scope);
                let pm = match frames.first() {
                    Some(f) => self.board.render_frame(o.scope, f, owndraw_core::board::Board::frame_scale(f, o.scale), o.background),
                    None => None,
                };
                match pm {
                    Some(pm) => {
                        let rgba: Vec<u8> = pm.pixels().iter().flat_map(|p| {
                            let d = p.demultiply();
                            [d.red(), d.green(), d.blue(), d.alpha()]
                        }).collect();
                        sh.platform.clipboard_set(ClipData::Image { w: pm.width(), h: pm.height(), rgba });
                        sh.toast("Als Bild kopiert");
                    }
                    None => sh.toast(if o.scope == Scope::Selection { "Es ist nichts ausgewählt" } else { "Das Board ist noch leer" }),
                }
            }
            CopySvg => match formats::build(&self.board, Fmt::Svg, &self.export_opts()) {
                Ok(svg) => {
                    sh.platform.clipboard_set(ClipData::Text(String::from_utf8_lossy(&svg).into_owned()));
                    sh.toast("Als SVG kopiert");
                }
                Err(e) => sh.toast(&e),
            },
        }
    }
}

/// Zeigt das Fenster unter dem Export-Knopf.
pub(super) fn show(ed: &mut Editor, ctx: &egui::Context, sh: &mut Shared, lay: &Lay) {
    if !ed.st.export.open {
        return;
    }
    let Some(btn) = ed.st.export.btn else { return };
    let canvas = lay.canvas;
    let max_h = (canvas.bottom() - btn.bottom() - 2.0 * M - 4.0).max(160.0);
    let r = Editor::float(ctx, "menu-export", Pos2::new(btn.right(), btn.bottom() + M), Align2::RIGHT_TOP, canvas.shrink(M), Order::Foreground, |ui| {
        style::pill(18).show(ui, |ui| {
            ui.set_width(300.0_f32.min(canvas.width() - 4.0 * M));
            options(ui, ed, lay, sh);
            style::separator(ui);
            let lh = (max_h - 190.0).max(120.0);
            egui::ScrollArea::vertical().max_height(lh).min_scrolled_height(lh).auto_shrink([false, true]).show(ui, |ui| {
                list(ui, ed, sh);
            });
        });
    });
    ed.st.popup_rects.push(r);
}

fn options(ui: &mut Ui, ed: &mut Editor, lay: &Lay, sh: &Shared) {
    let h = if sh.env.touch { min_touch(true).max(36.0) } else { 28.0 };
    ui.label(style::title("Exportieren"));
    let x = &mut ed.st.export;
    let has_sel = !ed.board.sel.is_empty();
    if x.scope == 2 && !has_sel {
        x.scope = 0;
    }
    if x.scope == 1 && !lay.pages_mode {
        x.scope = 0;
    }
    style::caption(ui, "Bereich");
    ui.horizontal_wrapped(|ui| {
        ui.spacing_mut().item_spacing = egui::vec2(4.0, 4.0);
        if style::chip(ui, if lay.pages_mode { "Alle Seiten" } else { "Ganzes Board" }, x.scope == 0, h).clicked() {
            x.scope = 0;
        }
        if lay.pages_mode && style::chip(ui, "Diese Seite", x.scope == 1, h).clicked() {
            x.scope = 1;
        }
        if has_sel && style::chip(ui, "Auswahl", x.scope == 2, h).clicked() {
            x.scope = 2;
        }
    });
    style::caption(ui, "Bilder");
    ui.horizontal_wrapped(|ui| {
        ui.spacing_mut().item_spacing = egui::vec2(4.0, 4.0);
        for (k, l) in [(1.0, "1×"), (2.0, "2×"), (4.0, "4×")] {
            if style::chip(ui, l, x.scale == k, h).on_hover_text("Auflösung (Pixel je Board-Punkt)").clicked() {
                x.scale = k;
            }
        }
        ui.add_space(6.0);
        if style::chip(ui, "Hintergrund", x.background, h).on_hover_text("Aus: durchsichtig (PNG, WebP, SVG …)").clicked() {
            x.background = !x.background;
        }
    });
}

fn list(ui: &mut Ui, ed: &mut Editor, sh: &mut Shared) {
    let h = if sh.env.touch { min_touch(true).max(40.0) } else { 30.0 };
    ui.spacing_mut().item_spacing.y = 1.0;
    let mut run = None;
    for (title, entries) in SECTIONS {
        let vis: Vec<Entry> = entries.iter().copied().filter(|e| e.available(sh)).collect();
        if vis.is_empty() {
            continue;
        }
        style::caption(ui, title);
        for e in vis {
            if row(ui, sh, e, h).on_hover_text(e.hint()).clicked() {
                run = Some(e);
            }
        }
    }
    if let Some(e) = run {
        ed.st.export.open = false;
        ed.run_entry(sh, e);
    }
}

/// Zeile mit Icon, Name und Dateiendung (wie die Menüzeilen, aber mit Tooltip).
fn row(ui: &mut Ui, sh: &mut Shared, e: Entry, min_h: f32) -> Response {
    let p = style::pal();
    let w = ui.available_width();
    let (r, resp) = ui.allocate_exact_size(Vec2::new(w, min_h), Sense::click());
    if resp.hovered() || resp.has_focus() || resp.is_pointer_button_down_on() {
        let bg = if resp.is_pointer_button_down_on() { p.press } else { p.hover };
        ui.painter().rect_filled(r, egui::CornerRadius::same(if style::modern() { 6 } else { 10 }), c(bg));
    }
    let ir = Rect::from_center_size(egui::pos2(r.left() + 18.0, r.center().y), Vec2::splat(20.0));
    sh.icons.paint(ui, ir, e.icon(), p.dim, false, 17.0);
    ui.painter().text(egui::pos2(r.left() + 38.0, r.center().y), Align2::LEFT_CENTER, e.label(), egui::FontId::proportional(14.0), c(p.text));
    let right = e.right();
    if !right.is_empty() {
        ui.painter().text(egui::pos2(r.right() - 10.0, r.center().y), Align2::RIGHT_CENTER, right, egui::FontId::proportional(12.0), c(p.dim));
    }
    resp
}
