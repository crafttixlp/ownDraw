//! Einstellungsbereich „Updates“ (Platzhalter, WP11).
//!
//! Vertrag: `section` wird von der Einstellungsseite (`ui/settings.rs`) im Bereich „Updates“ aufgerufen.

use crate::platform::Platform;
use crate::settings::Settings;
use crate::style::{self, c};

pub fn section(ui: &mut egui::Ui, _settings: &mut Settings, _platform: &dyn Platform) {
    style::caption(ui, "Updates");
    ui.label(egui::RichText::new("Nach Updates suchen: folgt (WP11).").color(c(style::pal().dim)));
}
