//! Einstellungsbereich „Updates“ (WP11).
//!
//! Vertrag: `section` wird von der Einstellungsseite (`ui/settings.rs`) im Bereich „Updates“ aufgerufen,
//! `start_prompt` jedes Frame von der App-Hülle (Dialog „Update verfügbar“ nach dem Start-Check).
//! Installiert wird nie von selbst: Netzwerk nur auf Klick oder beim Start-Check (abschaltbar). Der Dienst läuft in
//! Hintergrund-Threads (`update::global()`), diese Oberfläche liest nur den Status.

use crate::platform::Platform;
use crate::settings::{Settings, UpdateMode};
use crate::style::{self, c};
use crate::update::github::Release;
use crate::update::{self, versions, Config, Error, Status, Updater};

/// Wartet auf Bestätigung (Format-Warnung beim Wechsel auf eine ältere Version).
#[derive(Clone)]
enum Pending {
    Install(Release),
    Restore(versions::Saved),
}

const RUSTUP_URL: &str = "https://rustup.rs";

fn pending_id() -> egui::Id {
    egui::Id::unique("owndraw_update_pending")
}

fn list_id() -> egui::Id {
    egui::Id::unique("owndraw_update_list_open")
}

fn mb(n: u64) -> String {
    format!("{:.1} MB", n as f64 / 1_048_576.0)
}

pub fn section(ui: &mut egui::Ui, settings: &mut Settings, platform: &dyn Platform) {
    let up = update::global().clone();
    up.set_ctx(ui.ctx());
    let st = up.status();
    let mut cfg = Config::new(settings, platform);
    let p = style::pal();
    let h = if platform.is_touch() { 44.0 } else { 30.0 };
    let busy = st.busy.is_some();
    let desktop = !cfg!(target_os = "android");

    style::caption(ui, "Updates");
    ui.label(egui::RichText::new(format!("Installiert: Version {}", cfg.current)).color(c(p.dim)));
    if !cfg.configured() {
        ui.add_space(4.0);
        ui.label(
            egui::RichText::new("Updates sind nicht eingerichtet. Diese Version enthält noch keinen gültigen Signaturschlüssel, deshalb wird nichts geladen oder installiert.")
                .color(c(p.danger_text)),
        );
    }
    ui.add_space(6.0);

    // Modus und Start-Hinweis
    if desktop {
        ui.horizontal_wrapped(|ui| {
            for (m, l) in [(UpdateMode::Package, "Fertiges Paket"), (UpdateMode::Source, "Aus Quellcode bauen")] {
                if style::chip(ui, l, settings.update_mode == m, h).clicked() {
                    settings.update_mode = m;
                }
            }
        });
        let hint = match settings.update_mode {
            UpdateMode::Package => "Lädt das fertige AppImage. Das geht schnell.",
            UpdateMode::Source => "Lädt den Quellcode und baut die App selbst. Das braucht Rust und dauert einige Minuten.",
        };
        ui.label(egui::RichText::new(hint).size(12.0).color(c(p.dim)));
    } else {
        ui.label(egui::RichText::new("Auf Android lädt OwnDraw immer die fertige APK und übergibt sie an den System-Installer. Dafür brauchst du die Berechtigung „Unbekannte Apps installieren“.").size(12.0).color(c(p.dim)));
    }
    cfg.mode = if desktop { settings.update_mode } else { UpdateMode::Package };
    ui.checkbox(&mut settings.update_ask_on_start, "Beim Start nach Updates suchen und fragen (installiert nie ohne deine Zustimmung)");
    ui.add_space(6.0);

    ui.horizontal(|ui| {
        let enabled = !busy && cfg.configured();
        if ui.add_enabled_ui(enabled, |ui| style::primary_btn(ui, "Nach Updates suchen", h)).inner.clicked() {
            up.check(cfg.clone(), false);
        }
        if busy && style::danger_btn(ui, "Abbrechen", h).clicked() {
            up.cancel();
        }
    });

    busy_view(ui, &st);
    messages(ui, &up, &st, settings, platform, h);

    // Neue Version
    if let Some(chk) = &st.check {
        match (&chk.latest, chk.newer) {
            (Some(rel), true) if !st.dismissed && st.done.is_none() => {
                ui.add_space(8.0);
                style::card().show(ui, |ui| {
                    if chk.skipped {
                        ui.label(egui::RichText::new("Du hast diese Version übersprungen.").color(c(p.dim)));
                    }
                    ui.label(egui::RichText::new(format!("Neue Version {} verfügbar", rel.version)).size(style::fs::SECTION + 1.0).strong());
                    if !rel.date.is_empty() {
                        ui.label(egui::RichText::new(&rel.date).size(12.0).color(c(p.dim)));
                    }
                    notes(ui, rel, "update_notes");
                    ui.add_space(6.0);
                    ui.horizontal_wrapped(|ui| {
                        if !busy && style::primary_btn(ui, "Jetzt aktualisieren", h).clicked() {
                            up.install(cfg.clone(), rel.clone());
                        }
                        if style::plain_btn(ui, "Später", h).clicked() {
                            up.dismiss_update();
                        }
                        if !chk.skipped && style::plain_btn(ui, "Version überspringen", h).clicked() {
                            settings.update_skip = Some(rel.version.to_string());
                            up.dismiss_update();
                        }
                    });
                });
            }
            (Some(_), true) => {}
            (Some(rel), false) => {
                ui.label(egui::RichText::new(format!("OwnDraw {} ist aktuell. Neueste veröffentlichte Version: {}.", cfg.current, rel.version)).color(c(p.dim)));
            }
            (None, _) => {
                ui.label(egui::RichText::new("Auf GitHub gibt es noch keine veröffentlichte Version.").color(c(p.dim)));
            }
        }
    }

    // Zurückspringen
    ui.add_space(10.0);
    style::caption(ui, "Frühere Versionen");
    saved_view(ui, &up, &cfg, &st, platform, h, desktop);
    release_list(ui, &up, &cfg, &st, h, desktop);
    confirm_view(ui, &up, &cfg, h);
}

/// Dialog „Update verfügbar“ nach dem Start-Check: fragt, ob jetzt aktualisiert werden soll, zeigt den
/// Fortschritt und danach „Neu starten“ (AppImage) bzw. den Hinweis auf den System-Installer (Android).
#[cfg_attr(target_arch = "wasm32", allow(dead_code))]
pub fn start_prompt(ctx: &egui::Context, settings: &mut Settings, platform: &dyn Platform) {
    let up = update::global().clone();
    let st = up.status();
    if !st.prompt {
        return;
    }
    let Some(rel) = st.check.as_ref().and_then(|c| c.latest.clone()) else {
        up.close_prompt();
        return;
    };
    let cfg = Config::new(settings, platform);
    let p = style::pal();
    let h = if platform.is_touch() { 44.0 } else { 30.0 };
    let busy = st.busy.is_some();
    let asking = !busy && st.done.is_none() && st.error.is_none();
    let r = egui::Modal::new(egui::Id::unique("owndraw-update-prompt")).frame(style::card().inner_margin(egui::Margin::same(style::sp::XL as i8))).show(ctx, |ui| {
        ui.set_max_width(440.0);
        ui.label(egui::RichText::new(format!("OwnDraw {} ist verfügbar", rel.version)).size(style::fs::SECTION + 2.0).strong());
        ui.label(egui::RichText::new(format!("Installiert ist Version {}.", cfg.current)).color(c(p.dim)));
        if asking {
            notes(ui, &rel, "prompt_notes");
            ui.add_space(6.0);
            let hint = if cfg!(target_os = "android") {
                "OwnDraw lädt die neue APK und öffnet den Android-Installer. Dort bestätigst du die Installation."
            } else {
                "OwnDraw lädt das neue AppImage, prüft die Signatur und ersetzt die Datei. Die bisherige Version bleibt gesichert."
            };
            ui.label(egui::RichText::new(hint).size(12.0).color(c(p.dim)));
            ui.add_space(8.0);
            ui.horizontal_wrapped(|ui| {
                if style::primary_btn(ui, "Jetzt aktualisieren", h).clicked() {
                    up.install(cfg.clone(), rel.clone());
                }
                if style::plain_btn(ui, "Später", h).clicked() {
                    up.close_prompt();
                }
                if style::plain_btn(ui, "Diese Version überspringen", h).clicked() {
                    settings.update_skip = Some(rel.version.to_string());
                    up.close_prompt();
                }
            });
        } else {
            busy_view(ui, &st);
            if busy && style::danger_btn(ui, "Abbrechen", h).clicked() {
                up.cancel();
            }
            messages(ui, &up, &st, settings, platform, h);
        }
    });
    if asking && r.should_close() {
        up.close_prompt();
    }
}

fn busy_view(ui: &mut egui::Ui, st: &Status) {
    let p = style::pal();
    let Some(what) = &st.busy else { return };
    ui.add_space(6.0);
    ui.label(egui::RichText::new(what).strong());
    if !st.step.is_empty() {
        ui.label(egui::RichText::new(&st.step).color(c(p.dim)));
    }
    match st.progress {
        Some((d, Some(t))) if t > 0 => {
            ui.add(egui::ProgressBar::new((d as f32 / t as f32).clamp(0.0, 1.0)).text(format!("{} von {}", mb(d), mb(t))));
        }
        Some((d, _)) => {
            ui.add(egui::ProgressBar::new(0.0).animate(true).text(mb(d)));
        }
        None => {
            ui.add(egui::Spinner::new());
        }
    }
    if !st.log.is_empty() {
        egui::ScrollArea::vertical().id_salt("update_log").max_height(180.0).stick_to_bottom(true).show(ui, |ui| {
            for l in &st.log {
                ui.label(egui::RichText::new(l).monospace().size(11.0));
            }
        });
    }
}

fn messages(ui: &mut egui::Ui, up: &Updater, st: &Status, settings: &mut Settings, platform: &dyn Platform, h: f32) {
    let p = style::pal();
    if let Some(e) = &st.error {
        ui.add_space(6.0);
        let col = if *e == Error::Cancelled { c(p.dim) } else { c(p.danger_text) };
        ui.label(egui::RichText::new(e.to_string()).color(col));
        ui.horizontal_wrapped(|ui| {
            if *e == Error::NoToolchain {
                ui.label("Rust installieren:");
                if style::accent_btn(ui, "rustup.rs öffnen", h).clicked() {
                    platform.open_url(RUSTUP_URL);
                }
                if style::primary_btn(ui, "Fertiges Paket verwenden", h).clicked() {
                    settings.update_mode = UpdateMode::Package;
                    up.dismiss_message();
                }
            }
            if style::plain_btn(ui, "OK", h).clicked() {
                up.dismiss_message();
            }
        });
    }
    if let Some(d) = &st.done {
        ui.add_space(6.0);
        if d.restart {
            ui.label(format!("Version {} ist installiert. Die bisherige Version ist als „{}“ gesichert und unter „Frühere Versionen“ verfügbar.", d.version, d.backup.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default()));
            ui.horizontal_wrapped(|ui| {
                if style::primary_btn(ui, "Neu starten", h).clicked() {
                    match update::install::spawn_new(&d.path) {
                        Ok(()) => ui.ctx().send_viewport_cmd(egui::ViewportCommand::Close),
                        Err(e) => {
                            ui.ctx().memory_mut(|m| m.data.insert_temp(egui::Id::unique("owndraw_update_restart_err"), e.to_string()));
                        }
                    }
                }
                if style::plain_btn(ui, "Später", h).clicked() {
                    up.dismiss_message();
                }
            });
            if let Some(e) = ui.ctx().memory(|m| m.data.get_temp::<String>(egui::Id::unique("owndraw_update_restart_err"))) {
                ui.label(egui::RichText::new(format!("Neustart nicht möglich: {e}")).color(c(p.danger_text)));
            }
        } else {
            ui.label("Der System-Installer wurde geöffnet. Bestätige dort die Installation. Erlaube OwnDraw dafür gegebenenfalls zuerst „Unbekannte Apps installieren“.");
            if style::plain_btn(ui, "OK", h).clicked() {
                up.dismiss_message();
            }
        }
    }
}

fn notes(ui: &mut egui::Ui, rel: &Release, salt: &str) {
    if rel.body.trim().is_empty() {
        return;
    }
    ui.add_space(4.0);
    egui::ScrollArea::vertical().id_salt(format!("{salt}{}", rel.tag)).max_height(160.0).show(ui, |ui| {
        ui.label(rel.body.trim());
    });
}

fn saved_view(ui: &mut egui::Ui, up: &Updater, cfg: &Config, st: &Status, platform: &dyn Platform, h: f32, desktop: bool) {
    if !desktop {
        return;
    }
    let p = style::pal();
    let saved = versions::list_saved(&cfg.versions_dir);
    if saved.is_empty() {
        ui.label(egui::RichText::new("Noch keine frühere Version gesichert. Sie entsteht beim nächsten Update.").size(12.0).color(c(p.dim)));
        return;
    }
    let busy = st.busy.is_some();
    if let Some(n) = versions::newest_saved(&cfg.versions_dir) {
        if !busy && style::accent_btn(ui, &format!("Vorherige Version wiederherstellen ({})", n.version), h).clicked() {
            request_restore(ui, up, cfg, n, platform);
        }
    }
    ui.label(egui::RichText::new("Gesicherte Versionen, auch offline verfügbar:").size(12.0).color(c(p.dim)));
    for s in saved {
        ui.horizontal(|ui| {
            ui.label(format!("Version {}", s.version));
            if !busy && style::plain_btn(ui, "Wiederherstellen", h).clicked() {
                request_restore(ui, up, cfg, s.clone(), platform);
            }
        });
    }
}

fn request_restore(ui: &mut egui::Ui, up: &Updater, cfg: &Config, s: versions::Saved, _platform: &dyn Platform) {
    if s.version < cfg.current && versions::format_warning(&s.version, cfg.recent_boards).is_some() {
        ui.ctx().memory_mut(|m| m.data.insert_temp(pending_id(), Some(Pending::Restore(s))));
    } else {
        up.restore(cfg.clone(), s);
    }
}

fn release_list(ui: &mut egui::Ui, up: &Updater, cfg: &Config, st: &Status, h: f32, desktop: bool) {
    let p = style::pal();
    let busy = st.busy.is_some();
    let open = ui.ctx().memory(|m| m.data.get_temp::<bool>(list_id())).unwrap_or(false);
    ui.horizontal(|ui| {
        if !busy && cfg.configured() && style::plain_btn(ui, if open { "Alle Versionen ausblenden" } else { "Alle Versionen anzeigen" }, h).clicked() {
            ui.ctx().memory_mut(|m| m.data.insert_temp(list_id(), !open));
            if !open && st.releases.is_none() {
                up.load_list(cfg.clone());
            }
        }
    });
    if !open {
        return;
    }
    let Some(list) = &st.releases else {
        if !busy {
            ui.label(egui::RichText::new("Keine Liste geladen.").color(c(p.dim)));
        }
        return;
    };
    for rel in list {
        let current = rel.version == cfg.current;
        let header = format!("{}{}   {}", rel.version, if current { "  (installiert)" } else { "" }, rel.date);
        egui::CollapsingHeader::new(egui::RichText::new(header).strong()).id_salt(("rel", &rel.tag)).show(ui, |ui| {
            if !rel.title.is_empty() && rel.title != rel.tag {
                ui.label(&rel.title);
            }
            if rel.body.trim().is_empty() {
                ui.label(egui::RichText::new("Keine Versionshinweise.").color(c(p.dim)));
            }
            notes(ui, rel, "list_notes");
            let allowed = desktop || rel.version > cfg.current;
            if !current && allowed && !busy {
                let label = if rel.version < cfg.current { "Zu dieser Version zurückkehren" } else { "Diese Version installieren" };
                if style::accent_btn(ui, label, h).clicked() {
                    if rel.version < cfg.current && versions::format_warning(&rel.version, cfg.recent_boards).is_some() {
                        ui.ctx().memory_mut(|m| m.data.insert_temp(pending_id(), Some(Pending::Install(rel.clone()))));
                    } else {
                        up.install(cfg.clone(), rel.clone());
                    }
                }
            }
        });
    }
}

fn confirm_view(ui: &mut egui::Ui, up: &Updater, cfg: &Config, h: f32) {
    let Some(Some(pend)) = ui.ctx().memory(|m| m.data.get_temp::<Option<Pending>>(pending_id())) else { return };
    let version = match &pend {
        Pending::Install(r) => r.version.clone(),
        Pending::Restore(s) => s.version.clone(),
    };
    let warn = versions::format_warning(&version, cfg.recent_boards).unwrap_or_default();
    let clear = |ui: &egui::Ui| ui.ctx().memory_mut(|m| m.data.insert_temp::<Option<Pending>>(pending_id(), None));
    ui.add_space(8.0);
    style::card().show(ui, |ui| {
        ui.label(egui::RichText::new(format!("Auf Version {version} wechseln?")).strong());
        ui.label(egui::RichText::new(warn).color(c(style::pal().danger_text)));
        ui.horizontal_wrapped(|ui| {
            if style::danger_btn(ui, "Trotzdem wechseln", h).clicked() {
                match &pend {
                    Pending::Install(r) => up.install(cfg.clone(), r.clone()),
                    Pending::Restore(s) => up.restore(cfg.clone(), s.clone()),
                };
                clear(ui);
            }
            if style::plain_btn(ui, "Abbrechen", h).clicked() {
                clear(ui);
            }
        });
    });
}
