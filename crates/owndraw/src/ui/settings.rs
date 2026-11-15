//! Einstellungsseite (WP9).
//!
//! Vertrag: `show` zeichnet die Seite in den übergebenen `ui` und meldet, ob zurück gewechselt werden soll
//! bzw. ob sich Einstellungen geändert haben (die Shell wendet sie dann an und speichert `settings.json`).
//! Auf dem Handy eine Spalte (Abschnitte untereinander, Beschriftung über dem Bedienelement), sonst zentrierte
//! Spalte (max. 720 pt) mit Beschriftung links und Bedienelement rechts.

use crate::layout::{Env, FormFactor};
use crate::platform::{FileRef, Platform};
use crate::settings::{BarPos, SaveChoice, Settings, Theme, ACCENTS};
use crate::style::{self, c};
use egui::{Color32, Id, Sense, Stroke, Ui, Vec2};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SettingsAction {
    None,
    /// Zurück zum vorherigen Bildschirm (Start oder Editor).
    Back,
    /// Einstellungen wurden verändert.
    Changed,
    /// Standard-Ort wählen (die Shell öffnet den Ordner-Dialog; Ergebnis kommt als `Picked`).
    PickFolder,
    /// Editor im Bearbeiten-Modus der Werkzeugleiste öffnen.
    EditTools,
}

const MAX_W: f32 = 720.0;

/// Zeile: Beschriftung (mit optionaler Erklärung) und Bedienelement.
fn row(ui: &mut Ui, env: &Env, label: &str, hint: Option<&str>, controls: impl FnOnce(&mut Ui)) {
    let p = style::pal();
    if env.ff == FormFactor::Compact {
        ui.label(egui::RichText::new(label).strong());
        if let Some(h) = hint {
            ui.label(egui::RichText::new(h).size(12.5).color(c(p.dim)));
        }
        ui.horizontal_wrapped(controls);
    } else {
        ui.horizontal(|ui| {
            ui.allocate_ui_with_layout(Vec2::new(200.0, env.btn), egui::Layout::top_down(egui::Align::Min), |ui| {
                ui.set_width(200.0);
                ui.label(egui::RichText::new(label).strong());
                if let Some(h) = hint {
                    ui.label(egui::RichText::new(h).size(12.5).color(c(p.dim)));
                }
            });
            ui.horizontal_wrapped(controls);
        });
    }
    ui.add_space(6.0);
}

fn section(ui: &mut Ui, title: &str, body: impl FnOnce(&mut Ui)) {
    ui.add_space(14.0);
    ui.label(egui::RichText::new(title).size(13.0).strong().color(c(style::pal().dim)));
    ui.add_space(4.0);
    style::card().show(ui, |ui| {
        ui.set_width(ui.available_width());
        body(ui);
    });
}

fn bar_chips(ui: &mut Ui, env: &Env, cur: &mut Option<BarPos>) {
    for (v, l) in [(None, "Automatisch"), (Some(BarPos::Top), "Oben"), (Some(BarPos::Bottom), "Unten"), (Some(BarPos::Left), "Links"), (Some(BarPos::Right), "Rechts")] {
        if style::chip(ui, l, *cur == v, env.btn).clicked() {
            *cur = v;
        }
    }
}

fn argb_to_color(a: u32) -> Color32 {
    Color32::from_rgb((a >> 16) as u8, (a >> 8) as u8, a as u8)
}

fn color_to_argb(col: Color32) -> u32 {
    0xFF00_0000 | ((col.r() as u32) << 16) | ((col.g() as u32) << 8) | col.b() as u32
}

/// Farbfeld (rund) zum Anwählen; liefert true bei Klick.
fn swatch(ui: &mut Ui, col: u32, selected: bool, size: f32, tip: &str) -> bool {
    let p = style::pal();
    let (rect, resp) = ui.allocate_exact_size(Vec2::splat(size), Sense::click());
    let r = size * 0.5 - 3.0;
    ui.painter().circle_filled(rect.center(), r, argb_to_color(col));
    if selected {
        ui.painter().circle_stroke(rect.center(), r + 3.0, Stroke::new(2.5, c(p.text)));
    } else if resp.hovered() {
        ui.painter().circle_stroke(rect.center(), r + 2.0, Stroke::new(1.5, c(p.dim)));
    }
    resp.on_hover_text(tip).clicked()
}

pub fn show(ui: &mut Ui, settings: &mut Settings, platform: &dyn Platform) -> SettingsAction {
    let env = Env::new(ui.ctx().content_rect().width(), platform.is_touch());
    let compact = env.ff == FormFactor::Compact;
    let before = settings.clone();
    let mut act = SettingsAction::None;
    let side = if compact { 16 } else { 32 };

    egui::Frame::new().inner_margin(egui::Margin { left: side, right: side, top: 12, bottom: 0 }).show(ui, |ui| {
        ui.horizontal(|ui| {
            if style::plain_btn(ui, "‹ Zurück", env.btn).clicked() {
                act = SettingsAction::Back;
            }
            ui.label(egui::RichText::new("Einstellungen").size(22.0).strong());
        });
    });

    egui::ScrollArea::vertical().auto_shrink(false).show(ui, |ui| {
        let full = ui.available_width();
        let w = (full - 2.0 * side as f32).clamp(100.0, MAX_W);
        let pad = ((full - w) / 2.0).max(0.0);
        ui.horizontal_top(|ui| {
            ui.add_space(pad);
            ui.vertical(|ui| {
                ui.set_width(w);
                content(ui, &env, settings, platform, &mut act);
                ui.add_space(32.0);
            });
        });
    });

    if act == SettingsAction::None && *settings != before {
        act = SettingsAction::Changed;
    }
    act
}

fn content(ui: &mut Ui, env: &Env, settings: &mut Settings, platform: &dyn Platform, act: &mut SettingsAction) {
    let p = style::pal();
    let btn = env.btn;

    // ---- Darstellung
    section(ui, "Darstellung", |ui| {
        row(ui, env, "Design", None, |ui| {
            for (t, l) in [(Theme::System, "System"), (Theme::Light, "Hell"), (Theme::Dark, "Dunkel")] {
                if style::chip(ui, l, settings.theme == t, btn).clicked() {
                    settings.theme = t;
                }
            }
        });
        row(ui, env, "Akzentfarbe", None, |ui| {
            let sz = if env.touch { 40.0 } else { 34.0 };
            ui.spacing_mut().item_spacing = Vec2::splat(2.0);
            for (name, col) in ACCENTS {
                if swatch(ui, col, settings.accent == col, sz, name) {
                    settings.accent = col;
                }
            }
            ui.add_space(6.0);
            let custom = !ACCENTS.iter().any(|(_, col)| *col == settings.accent);
            let mut col = argb_to_color(settings.accent);
            if egui::color_picker::color_edit_button_srgba(ui, &mut col, egui::color_picker::Alpha::Opaque).changed() {
                settings.accent = color_to_argb(col);
            }
            ui.label(egui::RichText::new(if custom { "eigene" } else { "eigene wählen" }).size(12.5).color(c(p.dim)));
        });
    });

    // ---- UI-Größe
    section(ui, "UI-Größe", |ui| {
        row(ui, env, "Größe der Oberfläche", Some("75 bis 200 %, zusätzlich zur Bildschirm-Skalierung des Systems."), |ui| {
            let id = Id::unique("ui-scale-tmp");
            let cur = settings.ui_scale;
            let mut v = ui.data(|d| d.get_temp::<f32>(id)).unwrap_or(cur as f32);
            if icon_step(ui, "−", btn).clicked() {
                settings.ui_scale = (cur.saturating_sub(5)).clamp(75, 200);
                v = settings.ui_scale as f32;
            }
            ui.spacing_mut().slider_width = if env.ff == FormFactor::Compact { (ui.available_width() - 2.0 * btn - 90.0).clamp(80.0, 300.0) } else { 220.0 };
            let r = ui.add(egui::Slider::new(&mut v, 75.0..=200.0).step_by(5.0).fixed_decimals(0).suffix(" %"));
            if icon_step(ui, "+", btn).clicked() {
                settings.ui_scale = (cur + 5).clamp(75, 200);
                v = settings.ui_scale as f32;
            }
            // Beim Ziehen ändert sich der Maßstab unter dem Zeiger: Vorschau-Wert merken, beim Loslassen anwenden.
            if r.dragged() {
                ui.data_mut(|d| d.insert_temp(id, v));
            } else {
                ui.data_mut(|d| d.remove_temp::<f32>(id));
                if r.drag_stopped() || r.changed() {
                    settings.ui_scale = (v.round() as u32).clamp(75, 200);
                }
            }
            if style::chip(ui, "100 %", cur == 100, btn).clicked() {
                settings.ui_scale = 100;
                ui.data_mut(|d| d.remove_temp::<f32>(id));
            }
        });
    });

    // ---- Leisten
    section(ui, "Leisten anpassen", |ui| {
        row(ui, env, "Werkzeugleiste", None, |ui| bar_chips(ui, env, &mut settings.toolbar_pos));
        row(ui, env, "Optionsleiste", None, |ui| bar_chips(ui, env, &mut settings.options_pos));
        row(ui, env, "Leisten-Positionen", Some("Mit dem Griff verschobene Leisten zurück an den Standardplatz."), |ui| {
            if ui.add_enabled(!settings.tile_pos.is_empty(), egui::Button::new("Leisten-Positionen zurücksetzen").min_size(egui::vec2(0.0, btn))).clicked() {
                settings.tile_pos.clear();
            }
        });
        row(ui, env, "Werkzeuge", Some("Reihenfolge ändern und Werkzeuge aus- oder einblenden."), |ui| {
            if style::accent_btn(ui, "Werkzeuge bearbeiten…", btn).clicked() {
                *act = SettingsAction::EditTools;
            }
        });
    });

    // ---- Speichern
    section(ui, "Speichern", |ui| {
        let (dir_name, is_default) = match settings.default_dir.as_deref().and_then(FileRef::from_key) {
            Some(f) => (f.display_name(), false),
            None => (platform.default_dir().display_name(), true),
        };
        row(ui, env, "Standard-Ort", Some(if is_default { "Vorgabe des Systems" } else { "selbst gewählt" }), |ui| {
            ui.label(egui::RichText::new(dir_name).color(c(p.dim)));
        });
        row(ui, env, "", None, |ui| {
            if style::accent_btn(ui, "Ändern…", btn).clicked() {
                *act = SettingsAction::PickFolder;
            }
            ui.add_enabled_ui(!is_default, |ui| {
                if style::plain_btn(ui, "Zurücksetzen", btn).clicked() {
                    settings.default_dir = None;
                }
            });
        });
        row(ui, env, "Neue Boards", None, |ui| {
            for (v, l) in [(SaveChoice::Ask, "Immer fragen"), (SaveChoice::AlwaysDefault, "Immer Standard-Ort")] {
                if style::chip(ui, l, settings.save_choice == v, btn).clicked() {
                    settings.save_choice = v;
                }
            }
        });
        row(ui, env, "Xournal++", Some("Autosave direkt in die .xopp-Datei statt in eine .owndraw daneben."), |ui| {
            ui.checkbox(&mut settings.xopp_autosave_direct, "Direkt ins .xopp speichern");
        });
    });

    // ---- Updates
    ui.add_space(14.0);
    style::card().show(ui, |ui| {
        ui.set_width(ui.available_width());
        super::update::section(ui, settings, platform);
    });

    // ---- Über
    section(ui, "Über", |ui| {
        row(ui, env, "OwnDraw", Some(concat!("Version ", env!("CARGO_PKG_VERSION"))), |ui| {
            if style::accent_btn(ui, "Projekt auf GitHub", btn).clicked() {
                platform.open_url(&format!("https://github.com/{}", settings.update_repo.trim()));
            }
        });
    });

    // ---- Alles zurücksetzen
    ui.add_space(14.0);
    let cid = Id::unique("settings-reset-confirm");
    let asking = ui.data(|d| d.get_temp::<bool>(cid)).unwrap_or(false);
    style::card().show(ui, |ui| {
        ui.set_width(ui.available_width());
        if asking {
            ui.label(egui::RichText::new("Alle Einstellungen auf die Vorgaben zurücksetzen? Deine Boards bleiben unberührt.").strong());
            ui.add_space(6.0);
            ui.horizontal_wrapped(|ui| {
                if style::plain_btn(ui, "Abbrechen", btn).clicked() {
                    ui.data_mut(|d| d.insert_temp(cid, false));
                }
                if style::danger_btn(ui, "Ja, alles zurücksetzen", btn).clicked() {
                    settings.reset();
                    ui.data_mut(|d| d.insert_temp(cid, false));
                }
            });
        } else if style::danger_btn(ui, "Alles zurücksetzen…", btn).clicked() {
            ui.data_mut(|d| d.insert_temp(cid, true));
        }
    });
}

fn icon_step(ui: &mut Ui, label: &str, h: f32) -> egui::Response {
    ui.add(egui::Button::new(egui::RichText::new(label).size(16.0)).min_size(Vec2::new(h, h)).corner_radius(egui::CornerRadius::same(255)))
}
