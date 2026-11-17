//! Einstellungsseite (WP9).
//!
//! Vertrag: `show` zeichnet die Seite in den übergebenen `ui` und meldet, ob zurück gewechselt werden soll
//! bzw. ob sich Einstellungen geändert haben (die Shell wendet sie dann an und speichert `settings.json`).
//! Auf dem Handy eine Spalte (Abschnitte untereinander, Beschriftung über dem Bedienelement), sonst zentrierte
//! Spalte (max. 720 pt) mit Beschriftung links und Bedienelement rechts.

use crate::layout::{Env, FormFactor};
use crate::platform::{FileRef, Platform};
use crate::settings::{BarPos, SaveChoice, Settings, Theme, ACCENTS};
use crate::style::{self, c, sp, Icons};
use crate::ui::editor::keys::{self, Chord, Kind};
use crate::ui::editor::tools::{self, Group};
use egui::{Color32, CornerRadius, Event, Id, Key, Sense, Stroke, Ui, Vec2};

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

/// Unterseite der Einstellungen.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
enum Page {
    #[default]
    Main,
    /// Werkzeuge ein-/ausschalten.
    Tools,
    /// Tastenkürzel belegen.
    Keys,
}

fn page_id() -> Id {
    Id::unique("settings-page")
}

/// Öffnet die Einstellungen beim nächsten Anzeigen auf der Seite „Tastenkürzel“.
pub fn open_keys_page(ctx: &egui::Context) {
    ctx.data_mut(|d| d.insert_temp(page_id(), Page::Keys));
}

/// Öffnet die Einstellungen beim nächsten Anzeigen auf der Seite „Werkzeuge“.
pub fn open_tools_page(ctx: &egui::Context) {
    ctx.data_mut(|d| d.insert_temp(page_id(), Page::Tools));
}

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
    if style::modern() {
        ui.add_space(22.0);
        ui.label(egui::RichText::new(title).size(15.0).strong().color(c(style::pal().text)));
        ui.add_space(6.0);
    } else {
        ui.add_space(14.0);
        ui.label(egui::RichText::new(title).size(13.0).strong().color(c(style::pal().dim)));
        ui.add_space(4.0);
    }
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

pub fn show(ui: &mut Ui, settings: &mut Settings, platform: &dyn Platform, icons: &mut Icons) -> SettingsAction {
    let env = Env::new(ui.ctx().content_rect().width(), platform.is_touch());
    let compact = env.ff == FormFactor::Compact;
    let before = settings.clone();
    let mut act = SettingsAction::None;
    let side = if compact { 16 } else { 32 };
    let mut page: Page = ui.data(|d| d.get_temp(page_id())).unwrap_or_default();

    // Kopfzeile bündig mit der Inhaltsspalte: Zurück-Knopf + Titel (+ Unterseite als Pfad)
    let full = ui.available_width();
    let col_w = (full - 2.0 * side as f32).clamp(100.0, MAX_W);
    let pad = ((full - col_w) / 2.0).max(0.0);
    egui::Frame::new().inner_margin(egui::Margin { left: 0, right: 0, top: 12, bottom: 4 }).show(ui, |ui| {
        ui.horizontal(|ui| {
            ui.add_space((pad - env.btn - sp::S).max(side as f32 * 0.5));
            let p = style::pal();
            let tip = if page == Page::Main { "Zurück" } else { "Zurück zu den Einstellungen" };
            if style::icon_btn(ui, icons, owndraw_core::icons::BACK, p.text, false, false, env.btn, tip).clicked() {
                if page == Page::Main {
                    act = SettingsAction::Back;
                } else {
                    page = Page::Main;
                    ui.data_mut(|d| d.remove::<KeyEdit>(key_edit_id()));
                }
            }
            ui.add_space(sp::XS);
            if page != Page::Main && !compact {
                if ui.add(egui::Label::new(egui::RichText::new("Einstellungen").size(style::fs::TITLE).color(c(p.dim))).sense(Sense::click())).on_hover_text("Zurück zu den Einstellungen").clicked() {
                    page = Page::Main;
                    ui.data_mut(|d| d.remove::<KeyEdit>(key_edit_id()));
                }
                ui.label(egui::RichText::new("›").size(style::fs::TITLE).color(c(p.dim)));
            }
            let title = match page {
                Page::Main => "Einstellungen",
                Page::Tools => "Werkzeuge",
                Page::Keys => "Tastenkürzel",
            };
            ui.label(style::title(title));
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
                match page {
                    Page::Main => content(ui, &env, settings, platform, icons, &mut act, &mut page),
                    Page::Tools => tools_page(ui, &env, settings, icons, &mut act),
                    Page::Keys => keys_page(ui, &env, settings),
                }
                ui.add_space(32.0);
            });
        });
    });

    ui.data_mut(|d| d.insert_temp(page_id(), page));
    if act == SettingsAction::None && *settings != before {
        act = SettingsAction::Changed;
    }
    act
}

fn content(ui: &mut Ui, env: &Env, settings: &mut Settings, platform: &dyn Platform, icons: &mut Icons, act: &mut SettingsAction, page: &mut Page) {
    let p = style::pal();
    let btn = env.btn;

    // ---- Schnellzugriff auf die Unterseiten (gut auffindbar statt tief in den Abschnitten)
    ui.add_space(sp::S);
    style::card().show(ui, |ui| {
        ui.set_width(ui.available_width());
        ui.spacing_mut().item_spacing.y = 0.0;
        let n_on = tools::TOOLS.iter().filter(|d| tools::enabled(settings, d.id)).count();
        let n_new = tools::TOOLS.iter().filter(|d| d.new && !tools::enabled(settings, d.id)).count();
        let sub = if n_new > 0 { format!("{n_on} von {} eingeschaltet · {n_new} neue verfügbar", tools::TOOLS.len()) } else { format!("{n_on} von {} eingeschaltet", tools::TOOLS.len()) };
        if nav_row(ui, icons, owndraw_core::icons::SLIDERS, "Werkzeuge", &sub, n_new > 0, btn) {
            *page = Page::Tools;
        }
        style::separator(ui);
        if nav_row(ui, icons, owndraw_core::icons::GEAR, "Tastenkürzel", "Jede Aktion frei belegen, auch mehrere Kürzel", false, btn) {
            *page = Page::Keys;
        }
    });

    // ---- Darstellung
    section(ui, "Darstellung", |ui| {
        row(ui, env, "Design", Some("Modern ist das neue Erscheinungsbild ab 4.9. Klassisch entspricht den früheren Versionen."), |ui| {
            let m = style::is_modern(settings);
            if style::chip(ui, "Modern", m, btn).clicked() {
                style::set_modern(settings, true);
            }
            if style::chip(ui, "Klassisch", !m, btn).clicked() {
                style::set_modern(settings, false);
            }
        });
        row(ui, env, "Farbschema", None, |ui| {
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
            ui.label(egui::RichText::new(if custom { "Eigene Farbe" } else { "Eigene Farbe wählen" }).size(12.5).color(c(p.dim)));
        });
    });

    // ---- UI-Größe
    section(ui, "Größe", |ui| {
        row(ui, env, "Größe der Oberfläche", Some("75 bis 200 %. Gilt zusätzlich zur Skalierung des Systems."), |ui| {
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
    section(ui, "Werkzeugleisten", |ui| {
        row(ui, env, "Ecken der Leisten", Some("0 px ist eckig, 40 px ganz rund."), |ui| {
            let cur = style::tile_radius_setting(settings);
            let mut v = cur;
            ui.spacing_mut().slider_width = if env.ff == FormFactor::Compact { (ui.available_width() - 100.0).clamp(80.0, 300.0) } else { 220.0 };
            ui.add(egui::Slider::new(&mut v, 0.0..=40.0).step_by(1.0).fixed_decimals(0).suffix(" px"));
            let d = style::tile_radius_default(settings);
            if style::chip(ui, "Standard", cur == d, btn).clicked() {
                v = d;
            }
            if v != cur {
                style::set_tile_radius(settings, v);
            }
        });
        let before = settings.toolbar_pos;
        row(ui, env, "Werkzeugleiste", None, |ui| bar_chips(ui, env, &mut settings.toolbar_pos));
        if settings.toolbar_pos != before {
            // feste Wahl ersetzt eine per Griff verschobene/angedockte Werkzeugleiste
            settings.tile_pos.retain(|k, _| !k.starts_with("bar-tools@"));
            settings.tile_dock.retain(|k, _| !k.starts_with("bar-tools@"));
        }
        row(ui, env, "Optionsleiste", None, |ui| bar_chips(ui, env, &mut settings.options_pos));
        row(ui, env, "Positionen", Some("Leisten lassen sich an ihrem Griff verschieben. Am Bildschirmrand losgelassen, docken sie an."), |ui| {
            if ui.add_enabled(!settings.tile_pos.is_empty() || !settings.tile_dock.is_empty(), egui::Button::new("Positionen zurücksetzen").min_size(egui::vec2(0.0, btn))).clicked() {
                settings.tile_pos.clear();
                settings.tile_dock.clear();
            }
        });
        row(ui, env, "Werkzeuge", Some("Werkzeuge ein- oder ausschalten und die Reihenfolge der Leiste ändern."), |ui| {
            if style::accent_btn(ui, "Werkzeuge …", btn).clicked() {
                *page = Page::Tools;
            }
            if style::plain_btn(ui, "Leiste anordnen …", btn).clicked() {
                *act = SettingsAction::EditTools;
            }
        });
        row(ui, env, "Rechnen", Some("Aus = keine Ergebnisse, z. B. für Prüfungen."), |ui| {
            ui.checkbox(&mut settings.calc.on, "Rechnen erlauben");
        });
        row(ui, env, "Tastenkürzel", Some("Jede Aktion und jedes Werkzeug frei belegen."), |ui| {
            if style::accent_btn(ui, "Tastenkürzel …", btn).clicked() {
                *page = Page::Keys;
            }
        });
    });

    // ---- Einrasten
    section(ui, "Einrasten", |ui| {
        let sn = &mut settings.snap;
        let slider_w = if env.ff == FormFactor::Compact { (ui.available_width() - 100.0).clamp(80.0, 300.0) } else { 220.0 };
        row(ui, env, "Formerkennung", Some("Erkannte Linien und Pfeile werden auf das Winkelraster gedreht."), |ui| {
            ui.checkbox(&mut sn.lines, "Linien und Pfeile einrasten");
        });
        ui.add_enabled_ui(sn.lines, |ui| {
            row(ui, env, "Winkelraster", None, |ui| snap_steps(ui, btn, &mut sn.line_step));
            row(ui, env, "Fangbereich", Some("Bis zu dieser Abweichung rastet die Linie ein. Bei 0° ist es aus."), |ui| {
                ui.spacing_mut().slider_width = slider_w;
                ui.add(egui::Slider::new(&mut sn.line_tol, 0..=15).suffix("°"));
            });
        });
        row(ui, env, "Formen ausrichten", Some("Fast gerade Rechtecke und Vielecke werden waagerecht ausgerichtet."), |ui| {
            ui.checkbox(&mut sn.shapes, "Gerade ausrichten");
        });
        row(ui, env, "Griffe", Some("Beim Ziehen von Pfeilenden und beim Drehen einer Auswahl rastet der Winkel ein."), |ui| {
            ui.checkbox(&mut sn.handles, "Griffe einrasten");
        });
        ui.add_enabled_ui(sn.handles, |ui| {
            row(ui, env, "Winkelraster", None, |ui| snap_steps(ui, btn, &mut sn.handle_step));
            row(ui, env, "Fangbereich", Some("Bei 0° ist es aus."), |ui| {
                ui.spacing_mut().slider_width = slider_w;
                ui.add(egui::Slider::new(&mut sn.handle_tol, 0..=15).suffix("°"));
            });
        });
        if !platform.is_touch() {
            row(ui, env, "Taste", Some("Solange die Taste gedrückt ist, gilt das Gegenteil der Einstellung oben."), |ui| {
                for k in crate::settings::SnapKey::ALL {
                    if style::chip(ui, k.label(), sn.key == k, btn).clicked() {
                        sn.key = k;
                    }
                }
            });
        }
        row(ui, env, "", None, |ui| {
            let d = crate::settings::Snapping::default();
            if ui.add_enabled(*sn != d, egui::Button::new("Einrasten zurücksetzen").min_size(egui::vec2(0.0, btn))).clicked() {
                *sn = d;
            }
        });
    });

    // ---- Handschrift-Erkennung (Werkzeug „Umwandeln“)
    section(ui, "Handschrift-Erkennung", |ui| {
        use owndraw_core::hw::{self as chw, Stufe};
        row(ui, env, "Handschrift-Erkennung", Some("Höher = größere Modelle, bessere Erkennung, längeres Laden. Erkannt wird nur im Browser, nichts geht an den Server."), |ui| {
            let cur = settings.hw.stufe();
            for s in Stufe::ALL {
                if style::chip(ui, s.label(), cur == s, btn).clicked() {
                    settings.hw.set(s);
                }
            }
        });
        let stufe = settings.hw.stufe();
        let info = crate::hw::info();
        row(ui, env, "Modelle", Some("Werden nur auf Klick vom OwnDraw-Server geladen und im Browser gespeichert."), |ui| {
            if !crate::hw::SUPPORTED {
                ui.label(egui::RichText::new("Nur im Browser verfügbar (Web-App).").color(c(p.dim)));
                return;
            }
            let Some(info) = info.as_ref() else {
                ui.label(egui::RichText::new("Die Erkennung wird noch geladen …").color(c(p.dim)));
                ui.ctx().request_repaint_after(std::time::Duration::from_millis(300));
                return;
            };
            match info.liste.as_str() {
                "leer" => {
                    crate::hw::refresh_list();
                    ui.spinner();
                }
                "laedt" => {
                    ui.spinner();
                    ui.ctx().request_repaint_after(std::time::Duration::from_millis(200));
                }
                "fehler" => {
                    ui.label(egui::RichText::new(&info.fehler).color(c(p.danger)));
                    if style::plain_btn(ui, "Erneut versuchen", btn).clicked() {
                        crate::hw::refresh_list();
                    }
                }
                _ => {
                    let ms = info.manifests();
                    let need = chw::pakete_fuer_stufe(&ms, stufe);
                    ui.vertical(|ui| {
                        if stufe == Stufe::Aus {
                            ui.label(egui::RichText::new("Aus: das Werkzeug „Umwandeln“ erkennt nichts.").color(c(p.dim)));
                        } else if need.is_empty() {
                            ui.label(egui::RichText::new("Der Server bietet keine Modelle an (Admin → Erkennungsmodelle).").color(c(p.dim)));
                        }
                        let mut busy = false;
                        for m in &need {
                            let pk = info.paket(&m.id);
                            let (st, col) = match pk.map(|x| x.zustand.as_str()) {
                                Some("da") => ("gespeichert".to_string(), p.text),
                                Some("laedt") => {
                                    busy = true;
                                    let x = pk.unwrap();
                                    (format!("lädt {} %", x.geladen * 100 / x.gesamt.max(1)), p.accent)
                                }
                                Some("fehler") => (format!("Fehler: {}", pk.unwrap().fehler), p.danger),
                                _ => ("nicht geladen".to_string(), p.dim),
                            };
                            let art = m.art().map(|a| a.label()).unwrap_or("Laufzeit");
                            ui.label(egui::RichText::new(format!("{art}: {} · {} · {st}", m.name, chw::fmt_mb(m.groesse()))).color(c(col)));
                        }
                        if busy {
                            ui.ctx().request_repaint_after(std::time::Duration::from_millis(200));
                        }
                        let missing: Vec<String> = need.iter().filter(|m| !info.paket(&m.id).is_some_and(|x| x.da() || x.zustand == "laedt")).map(|m| m.id.clone()).collect();
                        let sum: u64 = need.iter().filter(|m| missing.contains(&m.id)).map(|m| m.groesse()).sum();
                        ui.horizontal_wrapped(|ui| {
                            if ui.add_enabled_ui(!missing.is_empty(), |ui| style::accent_btn(ui, &if missing.is_empty() { "Alle Modelle geladen".to_string() } else { format!("Modelle laden ({})", chw::fmt_mb(sum)) }, btn)).inner.clicked() {
                                crate::hw::download(&missing);
                            }
                            let any = info.pakete.iter().any(|x| x.da());
                            if ui.add_enabled(any && !busy, egui::Button::new("Gespeicherte Modelle löschen").min_size(egui::vec2(0.0, btn))).clicked() {
                                crate::hw::clear();
                            }
                        });
                        ui.label(egui::RichText::new(if info.webgpu { "Beschleunigung: WebGPU verfügbar (große Modelle), sonst WebAssembly." } else { "Rechnet mit WebAssembly auf dem Prozessor (kein WebGPU)." }).size(12.0).color(c(p.dim)));
                        if !info.cache {
                            ui.label(egui::RichText::new("Dieser Browser speichert Modelle nur bis zum Neuladen (kein Cache Storage, z. B. ohne HTTPS).").size(12.0).color(c(p.dim)));
                        }
                    });
                }
            }
        });
        if let Some(info) = info.as_ref().filter(|i| !i.pakete.is_empty()) {
            row(ui, env, "Lizenzen", Some("Herkunft und Lizenz der Modelle auf dem Server."), |ui| {
                ui.vertical(|ui| {
                    for pk in &info.pakete {
                        let m = &pk.m;
                        ui.label(egui::RichText::new(format!("{} ({}, Version {})", m.name, m.lizenz, m.version)).strong());
                        if !m.lizenz_hinweis.is_empty() {
                            ui.label(egui::RichText::new(&m.lizenz_hinweis).size(12.0).color(c(p.dim)));
                        }
                        ui.horizontal_wrapped(|ui| {
                            if !m.quelle.is_empty() && ui.link(egui::RichText::new("Quelle").size(12.0)).on_hover_text(&m.quelle).clicked() {
                                platform.open_url(&m.quelle);
                            }
                            if !m.lizenz_datei.is_empty() && ui.link(egui::RichText::new("Lizenztext").size(12.0)).clicked() {
                                platform.open_url(&format!("api/models/{}/{}", m.id, m.lizenz_datei));
                            }
                        });
                        ui.add_space(4.0);
                    }
                });
            });
        }
    });

    // ---- Speichern
    if crate::platform::WEB {
        section(ui, "Speichern", |ui| {
            if crate::ui::server_store_available() {
                row(ui, env, "Neue Boards", Some("Vorgabe im Dialog „Neu“. Den Speicherort wählst du dort für jedes Board, später auch im Kartenmenü."), |ui| {
                    for (v, l) in [(true, "Auf dem Server"), (false, "In diesem Browser")] {
                        if style::chip(ui, l, settings.new_on_server == v, btn).clicked() {
                            settings.new_on_server = v;
                        }
                    }
                });
                row(ui, env, "", None, |ui| {
                    ui.add(egui::Label::new(egui::RichText::new("Boards im Browser liegen nur auf diesem Gerät. Teilen und Live-Zusammenarbeit gehen nur mit Boards auf dem Server.").color(c(p.dim))).wrap());
                });
            } else {
                row(ui, env, "Gastmodus", None, |ui| {
                    ui.add(egui::Label::new(egui::RichText::new("Boards werden nur in diesem Browser gespeichert. Lade wichtige Boards mit Strg+S herunter.").color(c(p.dim))).wrap());
                });
            }
        });
    } else {
    section(ui, "Speichern", |ui| {
        let (dir_name, is_default) = match settings.default_dir.as_deref().and_then(FileRef::from_key) {
            Some(f) => (f.display_name(), false),
            None => (platform.default_dir().display_name(), true),
        };
        row(ui, env, "Standard-Ort", Some(if is_default { "Vorgabe des Systems" } else { "Eigener Ordner" }), |ui| {
            ui.label(egui::RichText::new(dir_name).color(c(p.dim)));
        });
        row(ui, env, "", None, |ui| {
            if style::accent_btn(ui, "Ändern …", btn).clicked() {
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
        row(ui, env, "Xournal++", Some("Änderungen direkt in der .xopp-Datei speichern statt in einer .owndraw-Datei daneben."), |ui| {
            ui.checkbox(&mut settings.xopp_autosave_direct, "Direkt in .xopp speichern");
        });
    });
    }

    // ---- Konto & Zusammenarbeit (Desktop: Server-Adresse, Speicherort neuer Boards)
    #[cfg(not(any(target_os = "android", target_arch = "wasm32")))]
    section(ui, "Konto & Zusammenarbeit", |ui| {
        let sid = Id::unique("settings-server-url");
        let mut url = ui.data(|d| d.get_temp::<String>(sid)).unwrap_or_else(|| settings.server_url.clone());
        row(ui, env, "Server", Some("Adresse des OwnDraw-Servers für Anmelden, Boards auf dem Server und Live-Zusammenarbeit. Ohne Verbindung arbeitest du wie gewohnt mit Boards auf diesem Computer."), |ui| {
            ui.add(egui::TextEdit::singleline(&mut url).desired_width(260.0).hint_text(crate::settings::DEFAULT_SERVER).margin(egui::vec2(8.0, 6.0)));
        });
        let changed = url.trim().trim_end_matches('/') != settings.server_url.trim_end_matches('/');
        row(ui, env, "", None, |ui| {
            ui.add_enabled_ui(changed && !url.trim().is_empty(), |ui| {
                if style::accent_btn(ui, "Übernehmen", btn).clicked() {
                    crate::net::change_server(&url);
                    settings.server_url = crate::net::base_url();
                    url = settings.server_url.clone();
                }
            });
            ui.add_enabled_ui(settings.server_url != crate::settings::DEFAULT_SERVER, |ui| {
                if style::plain_btn(ui, "Standard", btn).clicked() {
                    crate::net::change_server(crate::settings::DEFAULT_SERVER);
                    settings.server_url = crate::settings::DEFAULT_SERVER.into();
                    url = settings.server_url.clone();
                }
            });
        });
        if changed {
            row(ui, env, "", None, |ui| {
                ui.add(egui::Label::new(egui::RichText::new("Beim Wechsel des Servers wirst du abgemeldet.").color(c(p.dim))).wrap());
            });
        }
        ui.data_mut(|d| d.insert_temp(sid, url));
        if crate::ui::server_store_available() {
            row(ui, env, "Neue Boards", Some("Vorgabe im Dialog „Neu“. Den Speicherort wählst du dort für jedes Board, später auch im Kartenmenü."), |ui| {
                for (v, l) in [(true, "Auf dem Server"), (false, "Auf diesem Computer")] {
                    if style::chip(ui, l, settings.new_on_server == v, btn).clicked() {
                        settings.new_on_server = v;
                    }
                }
            });
        }
    });

    // ---- Updates (nicht im Browser: dort liefert der Server die neue Version aus)
    #[cfg(not(target_arch = "wasm32"))]
    {
        ui.add_space(14.0);
        style::card().show(ui, |ui| {
            ui.set_width(ui.available_width());
            super::update::section(ui, settings, platform);
        });
    }

    // ---- Über
    section(ui, "Über OwnDraw", |ui| {
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
            ui.label(egui::RichText::new("Alle Einstellungen zurücksetzen? Deine Boards bleiben erhalten.").strong());
            ui.add_space(6.0);
            ui.horizontal_wrapped(|ui| {
                if style::plain_btn(ui, "Abbrechen", btn).clicked() {
                    ui.data_mut(|d| d.insert_temp(cid, false));
                }
                if style::danger_btn(ui, "Zurücksetzen", btn).clicked() {
                    settings.reset();
                    ui.data_mut(|d| d.insert_temp(cid, false));
                }
            });
        } else if style::danger_btn(ui, "Alle Einstellungen zurücksetzen …", btn).clicked() {
            ui.data_mut(|d| d.insert_temp(cid, true));
        }
    });
}

fn snap_steps(ui: &mut Ui, btn: f32, cur: &mut u32) {
    for v in crate::settings::SNAP_STEPS {
        if style::chip(ui, &format!("{v}°"), *cur == v, btn).clicked() {
            *cur = v;
        }
    }
}

fn icon_step(ui: &mut Ui, label: &str, h: f32) -> egui::Response {
    ui.add(egui::Button::new(egui::RichText::new(label).size(16.0)).min_size(Vec2::new(h, h)).corner_radius(egui::CornerRadius::same(255)))
}

// ---------------------------------------------------------------- Seite „Werkzeuge“

/// Hinweis-Plakette (z. B. „Neu“).
/// Navigationszeile zu einer Unterseite: Symbol, Titel, Untertitel, „Neu“-Punkt, Pfeil.
fn nav_row(ui: &mut Ui, icons: &mut Icons, icon: i32, title: &str, sub: &str, dot: bool, btn: f32) -> bool {
    let p = style::pal();
    let w = ui.available_width();
    let (r, resp) = ui.allocate_exact_size(Vec2::new(w, (btn + 18.0).max(56.0)), Sense::click());
    if resp.hovered() {
        ui.painter().rect_filled(r.expand2(Vec2::new(6.0, 0.0)), CornerRadius::same(8), c(p.hover));
    }
    let ic_c = egui::pos2(r.left() + 18.0, r.center().y);
    ui.painter().circle_filled(ic_c, 18.0, c(p.accent_soft));
    icons.paint(ui, egui::Rect::from_center_size(ic_c, Vec2::splat(36.0)), icon, p.accent, false, 20.0);
    ui.painter().text(egui::pos2(r.left() + 50.0, r.center().y - 9.0), egui::Align2::LEFT_CENTER, title, egui::FontId::proportional(style::fs::BODY + 0.5), c(p.text));
    ui.painter().text(egui::pos2(r.left() + 50.0, r.center().y + 10.0), egui::Align2::LEFT_CENTER, sub, egui::FontId::proportional(style::fs::SMALL), c(p.dim));
    if dot {
        let tw = ui.painter().layout_no_wrap(title.to_string(), egui::FontId::proportional(style::fs::BODY + 0.5), c(p.text)).size().x;
        ui.painter().circle_filled(egui::pos2(r.left() + 50.0 + tw + 8.0, r.center().y - 9.0), 4.0, c(p.accent));
    }
    ui.painter().text(egui::pos2(r.right() - 8.0, r.center().y), egui::Align2::RIGHT_CENTER, "›", egui::FontId::proportional(22.0), c(p.dim));
    resp.on_hover_cursor(egui::CursorIcon::PointingHand).clicked()
}

/// Ein/Aus-Schalter; liefert true bei Klick.
fn switch(ui: &mut Ui, on: bool, enabled: bool, tip: &str) -> bool {
    let p = style::pal();
    let size = Vec2::new(44.0, 26.0);
    let (rect, resp) = ui.allocate_exact_size(size, if enabled { Sense::click() } else { Sense::hover() });
    let t = ui.ctx().animate_bool_responsive(resp.id, on);
    let bg = if on { p.accent } else { p.line };
    let alpha = if enabled { 1.0 } else { 0.45 };
    ui.painter().rect_filled(rect, CornerRadius::same(13), c(bg).gamma_multiply(alpha));
    let x = egui::lerp((rect.left() + 13.0)..=(rect.right() - 13.0), t);
    ui.painter().circle_filled(egui::pos2(x, rect.center().y), 10.0, Color32::WHITE.gamma_multiply(alpha.max(0.8)));
    resp.on_hover_text(tip).clicked() && enabled
}

fn tools_page(ui: &mut Ui, env: &Env, settings: &mut Settings, icons: &mut Icons, act: &mut SettingsAction) {
    let p = style::pal();
    let btn = env.btn;
    ui.add_space(sp::S);
    let n_new_off = tools::TOOLS.iter().filter(|d| d.new && !tools::enabled(settings, d.id)).count();
    style::info_card(ui, |ui| {
        ui.label(egui::RichText::new("So passt du die Werkzeugleiste an").strong());
        ui.add(egui::Label::new(style::hint("Neue Werkzeuge sind anfangs ausgeschaltet, damit die Leiste übersichtlich bleibt. Schalte hier ein, was du brauchst – oder im Editor über den Knopf „+“ am Ende der Werkzeugleiste. Ausgeschaltete Werkzeuge verschwinden aus Leiste und Tastenkürzeln; schon gezeichnete Objekte bleiben bearbeitbar.")).wrap());
        if n_new_off > 0 {
            ui.add_space(sp::XS);
            if style::accent_btn(ui, &format!("Alle {n_new_off} neuen Werkzeuge einschalten"), btn.min(36.0)).clicked() {
                for d in tools::TOOLS.iter().filter(|d| d.new) {
                    tools::set_enabled(settings, d.id, true);
                }
            }
        }
    });
    let n_on = tools::TOOLS.iter().filter(|d| tools::enabled(settings, d.id)).count();
    for g in Group::ALL {
        let list: Vec<&tools::ToolDef> = tools::TOOLS.iter().filter(|d| d.group == g).collect();
        if list.is_empty() {
            continue;
        }
        section(ui, g.label(), |ui| {
            ui.spacing_mut().item_spacing.y = 0.0;
            for (i, d) in list.iter().enumerate() {
                if i > 0 {
                    style::separator(ui);
                }
                let on = tools::enabled(settings, d.id);
                let last = on && n_on <= 1;
                let w = ui.available_width();
                let mut desc = d.desc.to_string();
                if let Some(par) = d.parent.and_then(tools::def) {
                    if !tools::enabled(settings, par.id) {
                        desc.push_str(&format!(" Da „{}“ aus ist, bekommt sie einen eigenen Knopf.", par.name));
                    }
                }
                let desc_g = ui.painter().layout(desc, egui::FontId::proportional(style::fs::SMALL), c(p.dim), (w - 120.0).max(80.0));
                let h = (desc_g.size().y + 36.0).max(env.btn + 12.0);
                let (r, resp) = ui.allocate_exact_size(Vec2::new(w, h), if last { Sense::hover() } else { Sense::click() });
                if resp.hovered() && !last {
                    ui.painter().rect_filled(r.expand2(Vec2::new(6.0, 0.0)), CornerRadius::same(8), c(p.hover));
                }
                // Symbol mittig zur Textspalte
                let ic_c = egui::pos2(r.left() + 18.0, r.center().y);
                ui.painter().circle_filled(ic_c, 18.0, c(if on { p.accent_soft } else { p.hover }));
                icons.paint(ui, egui::Rect::from_center_size(ic_c, Vec2::splat(36.0)), d.icon, if on { p.accent } else { p.dim }, false, 20.0);
                let x = r.left() + 50.0;
                let top = r.center().y - (desc_g.size().y + 22.0) / 2.0;
                let name_g = ui.painter().layout_no_wrap(d.name.to_string(), egui::FontId::proportional(style::fs::BODY), c(p.text));
                let mut bx = x + name_g.size().x + 8.0;
                ui.painter().galley(egui::pos2(x, top), name_g, c(p.text));
                if d.new {
                    let g = ui.painter().layout_no_wrap("Neu".into(), egui::FontId::proportional(style::fs::CAPTION), c(p.accent));
                    let br = egui::Rect::from_min_size(egui::pos2(bx, top + 1.0), g.size() + Vec2::new(12.0, 4.0));
                    ui.painter().rect_filled(br, CornerRadius::same(255), c(p.accent_soft));
                    ui.painter().galley(br.min + Vec2::new(6.0, 2.0), g, c(p.accent));
                    bx = br.right() + 8.0;
                }
                let k = keys::labels(settings, &keys::tool_action(d.id));
                if !k.is_empty() {
                    ui.painter().text(egui::pos2(bx, top + 9.0), egui::Align2::LEFT_CENTER, k.join(" · "), egui::FontId::monospace(11.5), c(p.dim));
                }
                ui.painter().galley(egui::pos2(x, top + 22.0), desc_g, c(p.dim));
                // Schalter rechts, mittig
                let sw = egui::Rect::from_center_size(egui::pos2(r.right() - 22.0, r.center().y), Vec2::new(44.0, 26.0));
                let mut child = ui.new_child(egui::UiBuilder::new().max_rect(sw));
                let tip = if last { "Mindestens ein Werkzeug bleibt eingeschaltet" } else if on { "Ausschalten" } else { "Einschalten" };
                let sw_click = switch(&mut child, on, !last, tip);
                if (sw_click || resp.clicked()) && !last {
                    tools::set_enabled(settings, d.id, !on);
                }
            }
        });
    }
    ui.add_space(sp::L);
    ui.horizontal_wrapped(|ui| {
        if style::accent_btn(ui, "Leiste anordnen …", btn).clicked() {
            *act = SettingsAction::EditTools;
        }
        let default = settings.tool_on.is_empty() && settings.tool_order.is_empty();
        if ui.add_enabled(!default, egui::Button::new("Auf Standard zurücksetzen").min_size(egui::vec2(0.0, btn))).clicked() {
            tools::reset(settings);
        }
    });
}

// ---------------------------------------------------------------- Seite „Tastenkürzel“

/// Bearbeitungszustand der Seite „Tastenkürzel“.
#[derive(Clone, Debug, Default)]
enum KeyEdit {
    #[default]
    Idle,
    /// Wartet auf die nächste Taste für Aktion `id` (`slot` None = neues Kürzel).
    Rec { id: String, slot: Option<usize> },
    /// Kombination gehört schon `other`: dort entfernen?
    Conflict { id: String, slot: Option<usize>, chord: Chord, other: String },
    /// Hinweis (z. B. im Browser gesperrt).
    Msg(String),
}

fn key_edit_id() -> Id {
    Id::unique("settings-key-edit")
}

/// Ergebnis der Aufnahme: nächste Taste bzw. Kombination dieses Frames (Ereignisse werden verbraucht).
enum Captured {
    Cancel,
    Remove,
    Chord(Chord),
}

fn capture(ui: &mut Ui) -> Option<Captured> {
    ui.input_mut(|i| {
        let mut out = None;
        let mut idx = None;
        for (n, e) in i.events.iter().enumerate() {
            let got = match e {
                Event::Key { key, pressed: true, repeat: false, modifiers, .. } => {
                    let ch = Chord::from_event(*key, modifiers);
                    Some(match ch {
                        Chord { key: Key::Escape, ctrl: false, alt: false, shift: false } => Captured::Cancel,
                        Chord { key: Key::Delete, ctrl: false, alt: false, shift: false } => Captured::Remove,
                        _ => Captured::Chord(ch),
                    })
                }
                Event::Copy => Some(Captured::Chord(Chord { key: Key::C, ctrl: true, alt: false, shift: false })),
                Event::Cut => Some(Captured::Chord(Chord { key: Key::X, ctrl: true, alt: false, shift: false })),
                Event::Paste(_) => Some(Captured::Chord(Chord { key: Key::V, ctrl: true, alt: false, shift: false })),
                _ => None,
            };
            if got.is_some() {
                out = got;
                idx = Some(n);
                break;
            }
        }
        if idx.is_some() {
            // alle Tasten dieses Frames gehören der Aufnahme (kein Tab-Fokuswechsel o. Ä.)
            i.events.retain(|e| !matches!(e, Event::Key { .. } | Event::Copy | Event::Cut | Event::Paste(_) | Event::Text(_)));
        }
        out
    })
}

/// Übernimmt eine aufgenommene Kombination (prüft Browser-Sperre und Konflikte).
fn take_chord(settings: &mut Settings, id: &str, slot: Option<usize>, ch: Chord) -> KeyEdit {
    if let Some(m) = keys::refuse(ch) {
        return KeyEdit::Msg(m);
    }
    let Some(a) = keys::find(id) else { return KeyEdit::Idle };
    if let Some(o) = keys::conflict(settings, ch, a) {
        return KeyEdit::Conflict { id: id.to_string(), slot, chord: ch, other: o.id.clone() };
    }
    keys::assign(settings, id, slot, ch);
    KeyEdit::Idle
}

fn norm(s: &str) -> String {
    s.to_lowercase().replace('ä', "a").replace('ö', "o").replace('ü', "u").replace('ß', "ss")
}

fn keys_page(ui: &mut Ui, env: &Env, settings: &mut Settings) {
    let p = style::pal();
    let btn = env.btn;
    let chip_h = if env.touch { btn } else { 30.0 };
    let mut st: KeyEdit = ui.data(|d| d.get_temp(key_edit_id())).unwrap_or_default();

    // Aufnahme läuft: nächste Taste
    if let KeyEdit::Rec { id, slot } = st.clone() {
        if let Some(cap) = capture(ui) {
            st = match cap {
                Captured::Cancel => KeyEdit::Idle,
                Captured::Remove => {
                    if let Some(i) = slot {
                        keys::remove(settings, &id, i);
                    }
                    KeyEdit::Idle
                }
                Captured::Chord(ch) => take_chord(settings, &id, slot, ch),
            };
        }
    }

    ui.add_space(8.0);
    ui.add(egui::Label::new(egui::RichText::new("Tippe auf ein Kürzel und drücke die neue Taste oder Kombination (mit Strg, Alt oder Umschalt). Esc bricht ab, Entf entfernt das Kürzel. „+“ fügt ein weiteres hinzu.").color(c(p.dim))).wrap());
    ui.add_space(8.0);
    let qid = Id::unique("settings-keys-query");
    let mut query: String = ui.data(|d| d.get_temp(qid)).unwrap_or_default();
    ui.add(egui::TextEdit::singleline(&mut query).hint_text("Aktion suchen … (z. B. Stift, Zoom, Strg+Z)").desired_width(ui.available_width()).margin(egui::Margin::symmetric(12, 9)));
    ui.data_mut(|d| d.insert_temp(qid, query.clone()));

    // Konflikt bzw. Hinweis
    match st.clone() {
        KeyEdit::Conflict { id, slot, chord, other } => {
            let on = keys::find(&other).map(|a| a.name.clone()).unwrap_or(other.clone());
            let me = keys::find(&id).map(|a| a.name.clone()).unwrap_or(id.clone());
            ui.add_space(10.0);
            style::card().show(ui, |ui| {
                ui.set_width(ui.available_width());
                ui.add(egui::Label::new(egui::RichText::new(format!("{} ist schon „{on}“ zugewiesen. Dort entfernen und „{me}“ zuweisen?", chord.label())).strong()).wrap());
                ui.add_space(6.0);
                ui.horizontal_wrapped(|ui| {
                    if style::plain_btn(ui, "Abbrechen", btn).clicked() {
                        st = KeyEdit::Idle;
                    }
                    if style::primary_btn(ui, "Dort entfernen und zuweisen", btn).clicked() {
                        keys::unbind(settings, &other, chord);
                        keys::assign(settings, &id, slot, chord);
                        st = KeyEdit::Idle;
                    }
                });
            });
        }
        KeyEdit::Msg(m) => {
            ui.add_space(10.0);
            style::card().show(ui, |ui| {
                ui.set_width(ui.available_width());
                ui.add(egui::Label::new(egui::RichText::new(m).color(c(p.danger_text))).wrap());
                if style::plain_btn(ui, "OK", btn).clicked() {
                    st = KeyEdit::Idle;
                }
            });
        }
        _ => {}
    }

    let q = norm(&query);
    let mut groups: Vec<&'static str> = Vec::new();
    for a in keys::all() {
        if !groups.contains(&a.group) {
            groups.push(a.group);
        }
    }
    let mut shown = 0;
    for g in groups {
        let list: Vec<&keys::Action> = keys::all()
            .iter()
            .filter(|a| a.group == g)
            .filter(|a| {
                let hay = norm(&format!("{} {} {}", a.group, a.name, keys::labels(settings, &a.id).join(" ")));
                q.split_whitespace().all(|w| hay.contains(w))
            })
            .collect();
        if list.is_empty() {
            continue;
        }
        shown += list.len();
        section(ui, g, |ui| {
            for (n, a) in list.iter().enumerate() {
                if n > 0 {
                    ui.separator();
                }
                key_row(ui, env, settings, a, &mut st, chip_h);
            }
        });
    }
    if shown == 0 {
        ui.add_space(16.0);
        ui.label(egui::RichText::new("Keine Aktion gefunden.").color(c(p.dim)));
    }

    // Alle zurücksetzen (mit Rückfrage)
    ui.add_space(14.0);
    let cid = Id::unique("settings-keys-reset");
    let asking = ui.data(|d| d.get_temp::<bool>(cid)).unwrap_or(false);
    let custom = !settings.keys.is_empty() || settings.snap.key != crate::settings::Snapping::default().key;
    ui.horizontal_wrapped(|ui| {
        if asking {
            ui.label(egui::RichText::new("Alle Tastenkürzel auf Standard setzen?").strong());
            if style::plain_btn(ui, "Abbrechen", btn).clicked() {
                ui.data_mut(|d| d.insert_temp(cid, false));
            }
            if style::danger_btn(ui, "Zurücksetzen", btn).clicked() {
                keys::reset_all(settings);
                st = KeyEdit::Idle;
                ui.data_mut(|d| d.insert_temp(cid, false));
            }
        } else if ui.add_enabled(custom, egui::Button::new("Alle zurücksetzen …").min_size(egui::vec2(0.0, btn))).clicked() {
            ui.data_mut(|d| d.insert_temp(cid, true));
        }
    });
    if matches!(st, KeyEdit::Rec { .. }) {
        ui.ctx().request_repaint();
    }
    ui.data_mut(|d| d.insert_temp(key_edit_id(), st));
}

/// Zeile einer Aktion: Name, Kürzel als Felder, „+“, „Standard“.
fn key_row(ui: &mut Ui, env: &Env, settings: &mut Settings, a: &keys::Action, st: &mut KeyEdit, h: f32) {
    let p = style::pal();
    let avail = keys::available(settings, a);
    let compact = env.ff == FormFactor::Compact;
    let name = |ui: &mut Ui| {
        ui.horizontal_wrapped(|ui| {
            ui.label(egui::RichText::new(&a.name).strong().color(c(if avail { p.text } else { p.dim })));
            if !avail {
                ui.label(egui::RichText::new("(Werkzeug aus)").size(12.0).color(c(p.dim)));
            }
        });
    };
    let controls = |ui: &mut Ui, settings: &mut Settings, st: &mut KeyEdit| {
        if a.kind == Kind::Modifier {
            for k in crate::settings::SnapKey::ALL {
                if style::chip(ui, k.label(), settings.snap.key == k, h).clicked() {
                    settings.snap.key = k;
                }
            }
        } else {
            let list = keys::bindings(settings, a);
            for (i, ch) in list.iter().enumerate() {
                let rec = matches!(st, KeyEdit::Rec { id, slot: Some(s) } if *id == a.id && *s == i);
                let t = if rec { "Taste drücken …".to_string() } else { ch.label() };
                if style::chip(ui, &t, rec, h).on_hover_text("Antippen und neue Taste drücken (Entf entfernt)").clicked() {
                    *st = if rec { KeyEdit::Idle } else { KeyEdit::Rec { id: a.id.clone(), slot: Some(i) } };
                }
            }
            let rec_new = matches!(st, KeyEdit::Rec { id, slot: None } if *id == a.id);
            let t = if rec_new { "Taste drücken …" } else if list.is_empty() { "Kein Kürzel · +" } else { "+" };
            if style::chip(ui, t, rec_new, h).on_hover_text("Weiteres Kürzel hinzufügen").clicked() {
                *st = if rec_new { KeyEdit::Idle } else { KeyEdit::Rec { id: a.id.clone(), slot: None } };
            }
        }
        if keys::is_custom(settings, a) && style::plain_btn(ui, "Standard", h).on_hover_text(format!("Standard: {}", default_label(a))).clicked() {
            keys::reset(settings, &a.id);
            *st = KeyEdit::Idle;
        }
    };
    if compact {
        name(ui);
        ui.horizontal_wrapped(|ui| controls(ui, settings, st));
    } else {
        ui.horizontal(|ui| {
            ui.allocate_ui_with_layout(Vec2::new(230.0, h), egui::Layout::top_down(egui::Align::Min), |ui| {
                ui.set_width(230.0);
                name(ui);
            });
            ui.horizontal_wrapped(|ui| controls(ui, settings, st));
        });
    }
}

fn default_label(a: &keys::Action) -> String {
    if a.kind == Kind::Modifier {
        return crate::settings::Snapping::default().key.label().to_string();
    }
    if a.defaults.is_empty() {
        return "keins".into();
    }
    a.defaults.iter().map(|c| c.label()).collect::<Vec<_>>().join(", ")
}
