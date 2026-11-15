//! „⋯“-Menü: Ebenen, Seiten, Hintergrund, Export, Kopie/Xournal++, Einstellungen, Schließen.

use super::{pages, Editor, Lay, M};
use crate::app::{PickPurpose, Req, Shared};
use crate::layout::min_touch;
use crate::platform::FileKind;
use crate::style::{self, c};
use egui::{Align2, Order, Pos2, Ui};
use owndraw_core::pages::PageBg;

pub(super) fn show(ed: &mut Editor, ctx: &egui::Context, sh: &mut Shared, lay: &Lay) {
    if !ed.st.menu_open {
        return;
    }
    let Some(btn) = ed.st.menu_btn else { return };
    let canvas = lay.canvas;
    let max_h = (canvas.bottom() - btn.bottom() - 2.0 * M - 4.0).max(160.0);
    let r = Editor::float(
        ctx,
        "menu-more",
        Pos2::new(btn.right(), btn.bottom() + M),
        Align2::RIGHT_TOP,
        canvas.shrink(M),
        Order::Foreground,
        |ui| {
            style::pill(18).show(ui, |ui| {
                egui::ScrollArea::vertical()
                    .max_height(max_h)
                    .min_scrolled_height(
                        (15.0 * min_touch(sh.env.touch).max(34.0) + 120.0).min(max_h),
                    )
                    .auto_shrink([true, false])
                    .show(ui, |ui| {
                        ui.set_width(250.0);
                        content(ui, ed, sh, lay);
                    });
            });
        },
    );
    ed.st.popup_rects.push(r);
}

fn content(ui: &mut Ui, ed: &mut Editor, sh: &mut Shared, lay: &Lay) {
    let h = min_touch(sh.env.touch).max(34.0);
    let mut close = false;
    let pages_mode = lay.pages_mode;

    style::caption(ui, "Ansicht");
    if style::menu_row(
        ui,
        if ed.st.layers_open {
            "Ebenen ausblenden"
        } else {
            "Ebenen"
        },
        false,
        h,
    ) {
        ed.st.layers_open = !ed.st.layers_open;
        close = true;
    }
    if pages_mode {
        let open = ed.st.pages_open.unwrap_or(lay.dock_pages.is_some());
        if style::menu_row(
            ui,
            if open { "Seiten ausblenden" } else { "Seiten" },
            false,
            h,
        ) {
            ed.st.pages_open = Some(!open);
            close = true;
        }
    }
    if style::menu_row(ui, "Alles zeigen", false, h) {
        ed.board.fit();
        close = true;
    }
    // Hintergrund
    ui.add_space(2.0);
    style::caption(
        ui,
        if pages_mode {
            "Hintergrund dieser Seite"
        } else {
            "Hintergrund"
        },
    );
    ui.horizontal_wrapped(|ui| {
        if pages_mode {
            let cur = pages::current_bg(&ed.board);
            for (b, l) in [
                (PageBg::Plain, "Blanko"),
                (PageBg::Lined, "Liniert"),
                (PageBg::Grid, "Kariert"),
                (PageBg::Dotted, "Punktiert"),
            ] {
                if style::chip(ui, l, cur == Some(b), h).clicked() {
                    pages::set_bg(&mut ed.board, b);
                }
            }
        } else {
            for (i, l) in ["Punkte", "Raster", "Linien", "Leer"].iter().enumerate() {
                if style::chip(ui, l, ed.board.bg == i as i32, h).clicked() {
                    ed.board.bg = i as i32;
                    ed.board.modified = true;
                    ed.board.refresh();
                }
            }
        }
    });
    let dark = ed.board.dark;
    if style::menu_row(
        ui,
        if dark { "Board: hell" } else { "Board: dunkel" },
        false,
        h,
    ) {
        ed.board.dark = !dark;
        ed.board.modified = true;
        ed.board.refresh();
        close = true;
    }

    ui.add_space(4.0);
    style::caption(ui, "Export");
    if style::menu_row(ui, "Als PNG-Bild …", false, h) {
        ed.export(sh, 0);
        close = true;
    }
    if style::menu_row(ui, "Als PDF …", false, h) {
        ed.export(sh, 1);
        close = true;
    }
    if style::menu_row(ui, "Ebenen + Website (ZIP) …", false, h) {
        ed.export(sh, 2);
        close = true;
    }
    if style::menu_row(ui, "Website-Vorschau", false, h) {
        ed.preview(sh);
        close = true;
    }
    let act = ed.board.active.min(ed.board.layers.len().saturating_sub(1));
    if style::menu_row(ui, "Aktive Ebene als PNG …", false, h) {
        ed.export_layer(sh, act);
        close = true;
    }

    ui.add_space(4.0);
    style::caption(ui, "Datei");
    if style::menu_row(ui, "Umbenennen …", false, h) {
        sh.dialog = Some(crate::ui::dialogs::Dialog::Rename {
            what: crate::ui::dialogs::RenameWhat::Doc,
            title: "Board umbenennen",
            text: ed.name().to_string(),
            focus: true,
        });
        close = true;
    }
    if style::menu_row(ui, "Kopie speichern unter …", false, h) {
        let name = format!("{}.owndraw", crate::app::safe_file_name(ed.name()));
        sh.reqs.push(Req::PickSave {
            purpose: PickPurpose::SaveCopy,
            name,
            kind: FileKind::Owndraw,
        });
        close = true;
    }
    if style::menu_row(ui, "Xournal++ exportieren …", false, h) {
        let name = format!("{}.xopp", crate::app::safe_file_name(ed.name()));
        sh.reqs.push(Req::PickSave {
            purpose: PickPurpose::ExportXopp,
            name,
            kind: FileKind::Xopp,
        });
        close = true;
    }
    if style::menu_row(ui, "Einstellungen", false, h) {
        sh.reqs.push(Req::OpenSettings);
        close = true;
    }
    ui.add_space(2.0);
    ui.painter().hline(
        ui.max_rect().x_range(),
        ui.cursor().top(),
        egui::Stroke::new(1.0, c(style::pal().line)),
    );
    ui.add_space(4.0);
    if style::menu_row(ui, "Schließen", false, h) {
        sh.reqs.push(Req::Back);
        close = true;
    }
    if close {
        ed.st.menu_open = false;
    }
}
