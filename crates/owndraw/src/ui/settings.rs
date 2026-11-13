//! Einstellungsseite (Platzhalter, WP9 ersetzt den Inhalt).
//!
//! Vertrag: `show` zeichnet die Seite in den übergebenen `ui` und meldet, ob zurück gewechselt werden soll
//! bzw. ob sich Einstellungen geändert haben (die Shell wendet sie dann an und speichert `settings.json`).

use crate::layout::Env;
use crate::platform::Platform;
use crate::settings::{Settings, Theme};
use crate::style::{self, c};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SettingsAction {
    None,
    /// Zurück zum vorherigen Bildschirm (Start oder Editor).
    Back,
    /// Einstellungen wurden verändert.
    Changed,
}

pub fn show(ui: &mut egui::Ui, settings: &mut Settings, platform: &dyn Platform) -> SettingsAction {
    let env = Env::new(ui.ctx().content_rect().width(), platform.is_touch());
    let p = style::pal();
    let mut act = SettingsAction::None;
    let side = if env.ff == crate::layout::FormFactor::Compact { 16 } else { 32 };
    egui::Frame::new().inner_margin(egui::Margin { left: side, right: side, top: 16, bottom: 16 }).show(ui, |ui| {
        ui.horizontal(|ui| {
            if style::plain_btn(ui, "‹ Zurück", env.btn).clicked() {
                act = SettingsAction::Back;
            }
            ui.label(egui::RichText::new("Einstellungen").size(22.0).strong());
        });
        ui.add_space(12.0);
        ui.label(egui::RichText::new("Die vollständige Einstellungsseite folgt (WP9). Vorläufig:").color(c(p.dim)));
        ui.add_space(8.0);
        style::caption(ui, "Darstellung");
        ui.horizontal_wrapped(|ui| {
            for (t, l) in [(Theme::System, "System"), (Theme::Light, "Hell"), (Theme::Dark, "Dunkel")] {
                if style::chip(ui, l, settings.theme == t, env.btn).clicked() {
                    settings.theme = t;
                    act = SettingsAction::Changed;
                }
            }
        });
        ui.add_space(8.0);
        style::caption(ui, "UI-Größe");
        let mut s = settings.ui_scale as f32;
        if ui.add(egui::Slider::new(&mut s, 75.0..=200.0).suffix(" %")).changed() {
            settings.ui_scale = s.round() as u32;
            act = SettingsAction::Changed;
        }
        ui.add_space(12.0);
        super::update::section(ui, settings, platform);
    });
    act
}
