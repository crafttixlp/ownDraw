//! Drag & Drop in den Editor/die App (Platzhalter, WP10).
//!
//! Vertrag: `handle_drops` wird pro Frame aufgerufen (auch auf dem Startbildschirm, dann mit `editor = None`),
//! verarbeitet `platform.take_drops` und zeigt den Ablage-Hinweis, solange `platform.drag_hover` gilt.
//! Rückgabe: Aufträge an die Shell (Dokument öffnen, Hinweis anzeigen).

use super::Editor;
use crate::platform::{FileKind, FileRef, Platform};
use crate::style::{self, c};

pub enum DropOutcome {
    /// `.owndraw`/`.xopp` öffnen.
    Open(FileRef),
    Toast(String),
}

pub fn handle_drops(ctx: &egui::Context, platform: &dyn Platform, mut editor: Option<&mut Editor>) -> Vec<DropOutcome> {
    let mut out = Vec::new();
    for d in platform.take_drops(ctx) {
        let kind = FileKind::of_name(&d.name).max_by_file(&d.file);
        match (kind, &d.file) {
            (FileKind::Owndraw | FileKind::Xopp, Some(f)) => out.push(DropOutcome::Open(f.clone())),
            (FileKind::Image, _) => {
                let bytes = d.bytes.clone().or_else(|| d.file.as_ref().and_then(|f| platform.read(f).ok()));
                match (editor.as_deref_mut(), bytes) {
                    (Some(e), Some(b)) => {
                        if let Err(m) = e.import_image_bytes(&b) {
                            out.push(DropOutcome::Toast(m));
                        }
                    }
                    (None, _) => out.push(DropOutcome::Toast("Zum Einfügen zuerst ein Board öffnen".into())),
                    _ => out.push(DropOutcome::Toast("Bild konnte nicht gelesen werden".into())),
                }
            }
            _ => out.push(DropOutcome::Toast("Dieser Dateityp wird nicht unterstützt".into())),
        }
    }
    if platform.drag_hover(ctx) {
        let p = style::pal();
        egui::Area::new(egui::Id::unique("drop-hint")).order(egui::Order::Tooltip).anchor(egui::Align2::CENTER_CENTER, egui::Vec2::ZERO).interactable(false).show(ctx, |ui| {
            egui::Frame::new()
                .fill(c(p.card))
                .stroke(egui::Stroke::new(2.0, c(p.accent)))
                .corner_radius(egui::CornerRadius::same(18))
                .inner_margin(egui::Margin::symmetric(28, 20))
                .show(ui, |ui| {
                    ui.label(egui::RichText::new("Hier ablegen zum Öffnen oder Einfügen").size(16.0).strong());
                });
        });
    }
    out
}

trait ByFile {
    fn max_by_file(self, f: &Option<FileRef>) -> FileKind;
}

impl ByFile for FileKind {
    /// Ohne Namen den Dateinamen aus dem Verweis verwenden.
    fn max_by_file(self, f: &Option<FileRef>) -> FileKind {
        if self != FileKind::Any {
            return self;
        }
        f.as_ref().map(|f| FileKind::of_name(&f.display_name())).unwrap_or(FileKind::Any)
    }
}
