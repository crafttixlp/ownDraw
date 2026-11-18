//! „⋯“-Menü: Ebenen, Seiten, Hintergrund, Exportieren (öffnet das Export-Fenster), Kopie, Einstellungen, Schließen.

use super::{pages, Editor, Lay, M};
use crate::app::{PickPurpose, Req, Shared};
use crate::layout::min_touch;
use crate::platform::FileKind;
use super::keys;
use crate::style;
use owndraw_core::icons as ic;
use egui::{Align2, Order, Pos2, Ui};
use owndraw_core::pages::PageBg;
use owndraw_core::features as F;
use crate::locks::locked;

pub(super) fn show(ed: &mut Editor, ctx: &egui::Context, sh: &mut Shared, lay: &Lay) {
    if !ed.st.menu_open {
        return;
    }
    let Some(btn) = ed.st.menu_btn else { return };
    let canvas = lay.canvas;
    let max_h = (canvas.bottom() - btn.bottom() - 2.0 * M - 4.0).max(160.0);
    let r = Editor::float(ctx, "menu-more", Pos2::new(btn.right(), btn.bottom() + M), Align2::RIGHT_TOP, canvas.shrink(M), Order::Foreground, |ui| {
        style::pill(18).show(ui, |ui| {
            egui::ScrollArea::vertical().max_height(max_h).auto_shrink([true, true]).show(ui, |ui| {
                ui.set_width(264.0);
                content(ui, ed, sh, lay);
            });
        });
    });
    ed.st.popup_rects.push(r);
}

fn content(ui: &mut Ui, ed: &mut Editor, sh: &mut Shared, lay: &Lay) {
    // Zeilenhöhe: mit Touch groß genug für den Finger, sonst kompakt
    let h = if sh.env.touch { min_touch(true).max(40.0) } else { 30.0 };
    ui.spacing_mut().item_spacing.y = 1.0;
    let mut close = false;
    let pages_mode = lay.pages_mode;

    style::caption(ui, "Ansicht");
    if !locked(F::LAYERS) && style::menu_row_ic(ui, &mut sh.icons, ic::LAYERS, if ed.st.layers_open { "Ebenen ausblenden" } else { "Ebenen" }, "", false, h) {
        ed.st.layers_open = !ed.st.layers_open;
        close = true;
    }
    if pages_mode {
        let open = ed.st.pages_open.unwrap_or(lay.dock_pages.is_some());
        if style::menu_row_ic(ui, &mut sh.icons, ic::COPY, if open { "Seiten ausblenden" } else { "Seiten" }, "", false, h) {
            ed.st.pages_open = Some(!open);
            close = true;
        }
    }
    if style::menu_row_ic(ui, &mut sh.icons, ic::EYE, "Alles zeigen", &keys::label(&sh.settings, "fit"), false, h) {
        ed.board.fit();
        close = true;
    }
    // Hintergrund
    style::caption(ui, if pages_mode { "Hintergrund dieser Seite" } else { "Hintergrund" });
    ui.horizontal_wrapped(|ui| {
        ui.spacing_mut().item_spacing = egui::vec2(4.0, 4.0);
        if pages_mode {
            let cur = pages::current_bg(&ed.board);
            for (b, l) in [(PageBg::Plain, "Blanko"), (PageBg::Lined, "Liniert"), (PageBg::Grid, "Kariert"), (PageBg::Dotted, "Punktiert")] {
                if style::chip(ui, l, cur == Some(b), h - 2.0).clicked() {
                    pages::set_bg(&mut ed.board, b);
                }
            }
        } else {
            for (i, l) in ["Punkte", "Raster", "Linien", "Leer"].iter().enumerate() {
                if style::chip(ui, l, ed.board.bg == i as i32, h - 2.0).clicked() {
                    ed.board.bg = i as i32;
                    ed.board.modified = true;
                    ed.board.refresh();
                }
            }
        }
    });
    let dark = ed.board.dark;
    if style::menu_row_ic(ui, &mut sh.icons, ic::DARK, if dark { "Board: hell" } else { "Board: dunkel" }, "", false, h) {
        ed.board.dark = !dark;
        ed.board.modified = true;
        ed.board.refresh();
        close = true;
    }

    if !locked(F::EXPORT) || !locked(F::FORMULAS) {
        style::caption(ui, "Export");
    }
    if !locked(F::EXPORT) && style::menu_row_ic(ui, &mut sh.icons, ic::EXPORT, "Exportieren …", &keys::label(&sh.settings, "export"), false, h) {
        // gleiches Fenster wie der Knopf in der Kachel
        ed.st.export.open = true;
        close = true;
    }
    if !locked(F::FORMULAS) && style::menu_row_ic(ui, &mut sh.icons, ic::FORMULA, "Formelsammlung …", "", false, h) {
        ed.open_fbook(None);
        close = true;
    }

    style::caption(ui, "Datei");
    #[cfg(not(target_os = "android"))]
    server_rows(ui, ed, sh, h, &mut close);
    if crate::platform::WEB && style::menu_row_ic(ui, &mut sh.icons, ic::DOWNLOAD, "Herunterladen (.owndraw)", "", false, h) {
        let name = format!("{}.owndraw", crate::app::safe_file_name(ed.name()));
        sh.reqs.push(Req::PickSave { purpose: PickPurpose::SaveCopy, name, kind: FileKind::Owndraw });
        close = true;
    }
    if style::menu_row_ic(ui, &mut sh.icons, ic::PEN, "Umbenennen …", "", false, h) {
        sh.dialog = Some(crate::ui::dialogs::Dialog::Rename { what: crate::ui::dialogs::RenameWhat::Doc, title: "Board umbenennen", text: ed.name().to_string(), focus: true });
        close = true;
    }
    if !crate::platform::WEB && style::menu_row_ic(ui, &mut sh.icons, ic::COPY, "Kopie speichern unter …", "", false, h) {
        let name = format!("{}.owndraw", crate::app::safe_file_name(ed.name()));
        sh.reqs.push(Req::PickSave { purpose: PickPurpose::SaveCopy, name, kind: FileKind::Owndraw });
        close = true;
    }
    style::caption(ui, "Einstellungen & Hilfe");
    if style::menu_row_ic(ui, &mut sh.icons, ic::SLIDERS, "Werkzeuge anpassen …", "", false, h) {
        crate::ui::settings::open_tools_page(ui.ctx());
        sh.reqs.push(Req::OpenSettings);
        close = true;
    }
    if style::menu_row_ic(ui, &mut sh.icons, ic::GEAR, "Einstellungen", "", false, h) {
        sh.reqs.push(Req::OpenSettings);
        close = true;
    }
    if style::menu_row_ic(ui, &mut sh.icons, ic::HELP, "Anleitung & Tipps", &keys::label(&sh.settings, "help"), false, h) {
        sh.reqs.push(Req::OpenTutorial);
        close = true;
    }
    if crate::ui::bug_report_available() && !locked(F::BUGREPORT) && style::menu_row_ic(ui, &mut sh.icons, ic::BUG, "Fehler melden …", "", false, h) {
        crate::ui::open_bug_report(sh);
        close = true;
    }
    if crate::ui::bug_report_available() && style::menu_row_ic(ui, &mut sh.icons, ic::DOWNLOAD, "Apps herunterladen", "", false, h) {
        crate::ui::open_downloads(sh);
        close = true;
    }
    style::separator(ui);
    if style::menu_row_ic(ui, &mut sh.icons, ic::BACK, "Schließen", "", false, h) {
        sh.reqs.push(Req::Back);
        close = true;
    }
    if close {
        ed.st.menu_open = false;
    }
}

/// Web-App/Desktop: „Teilen …“ (Server-Board, Besitzer mit Lizenz) bzw. „Am Server speichern“ (lokales Board, Lizenz).
#[cfg(not(target_os = "android"))]
fn server_rows(ui: &mut Ui, ed: &mut Editor, sh: &mut Shared, h: f32, close: &mut bool) {
    use crate::net as api;
    if !api::licensed() {
        return;
    }
    match ed.file().and_then(api::srv_id).map(String::from) {
        Some(id) => {
            let owner = api::board(&id).is_some_and(|b| b.role == "owner");
            if owner && !locked(F::SHARE) && style::menu_row_ic(ui, &mut sh.icons, ic::SHARE, "Teilen …", "", false, h) {
                if let Some(l) = ed.live.as_mut() {
                    l.open_share();
                }
                *close = true;
            }
        }
        None if ed.file().is_some() && style::menu_row_ic(ui, &mut sh.icons, ic::CLOUD, "Auf dem Server speichern", "", false, h) => {
            let mut d = ed.snapshot_doc();
            d.thumb = ed.board.thumb().encode_png().ok();
            match crate::session::serialize(&d) {
                Ok(b) => {
                    api::create_board(&d.meta.name, b);
                    sh.toast("Board wird auf dem Server erstellt …");
                }
                Err(e) => sh.toast(&e),
            }
            *close = true;
        }
        None => {}
    }
}
