//! Werkzeug Tabelle: Platzieren (Zeilen × Spalten aus dem Popover), Zellen auf dem Blatt beschriften
//! (Inline-Texteditor aus text.rs), Zeilen/Spalten an der aktuellen Zelle hinzufügen und entfernen.

use super::text::TextEdState;
use super::Editor;
use crate::app::Shared;
use crate::style::{self, c, icon_btn};
use egui::{Rect, Stroke, Ui};
use owndraw_core::board::{self, Board};
use owndraw_core::icons as ic;
use owndraw_core::item::Kind;

/// Vorgaben des Werkzeugs und aktuelle Zelle (id, Zeile, Spalte).
pub(super) struct TableUi {
    pub rows: usize,
    pub cols: usize,
    pub cell: Option<(u64, usize, usize)>,
}

impl Default for TableUi {
    fn default() -> Self {
        TableUi { rows: 3, cols: 3, cell: None }
    }
}

/// Oberste Tabelle am Weltpunkt und ihre Zelle.
pub(super) fn cell_at(b: &Board, x: f32, y: f32) -> Option<(u64, usize, usize)> {
    b.items.iter().rev().find_map(|it| match &it.kind {
        Kind::Table(t) => t.cell_at(x, y).map(|(r, cc)| (it.id, r, cc)),
        _ => None,
    })
}

/// Zeilen- und Spaltenzahl der Tabelle.
fn dims(b: &Board, id: u64) -> Option<(usize, usize)> {
    match &b.items.iter().find(|it| it.id == id)?.kind {
        Kind::Table(t) => Some((t.rows(), t.cols())),
        _ => None,
    }
}

impl Editor {
    /// Tippen mit dem Tabellen-Werkzeug: Tabelle setzen, auswählen (Zellen dann per Tippen beschriften).
    pub(super) fn place_table(&mut self, wx: f32, wy: f32) {
        let (r, cc) = (self.xs.table.rows.clamp(1, 50), self.xs.table.cols.clamp(1, 20));
        match self.board.add_table(wx, wy, r, cc) {
            Some(id) => {
                self.leave_xtool();
                self.board.set_tool(board::SELECT);
                self.board.select_only(id);
                self.xs.table.cell = Some((id, 0, 0));
            }
            None => self.x_notice("Die Ebene ist gesperrt. Entsperre sie, um eine Tabelle einzufügen."),
        }
    }

    /// Zelle direkt auf dem Blatt bearbeiten.
    pub(super) fn edit_cell(&mut self, id: u64, r: usize, cc: usize) {
        if self.text.is_some() {
            self.finish_text();
        }
        let Some(it) = self.board.items.iter().find(|it| it.id == id) else { return };
        let Kind::Table(t) = &it.kind else { return };
        if r >= t.rows() || cc >= t.cols() {
            return;
        }
        let (x, y) = t.cell_origin(r, cc);
        let pad = t.pad();
        let rr = (x + pad, y + pad, t.size, (t.cw[cc] - 2.0 * pad).max(t.size), (t.rh[r] - 2.0 * pad).max(t.size));
        self.text = Some(TextEdState { r: rr, note: false, edit: None, text: t.cell(r, cc).to_string(), focus: true, color: it.color, bg: 0, commit: false, cell: Some((id, r, cc)), math: None });
        self.xs.table.cell = Some((id, r, cc));
    }

    /// Aktuelle Zelle der einzeln ausgewählten Tabelle (ohne gemerkte Zelle: die letzte).
    fn sel_table(&self) -> Option<(u64, usize, usize)> {
        if self.board.sel.len() != 1 || self.board.tool != board::SELECT {
            return None;
        }
        let id = self.board.sel[0];
        let (rows, cols) = dims(&self.board, id)?;
        Some(match self.xs.table.cell {
            Some((cid, r, cc)) if cid == id => (id, r.min(rows - 1), cc.min(cols - 1)),
            _ => (id, rows - 1, cols - 1),
        })
    }

    /// Rahmen um die aktuelle Zelle der ausgewählten Tabelle.
    pub(super) fn paint_cell(&self, pt: &egui::Painter) {
        let Some((id, r, cc)) = self.sel_table() else { return };
        if self.xs.table.cell.map(|x| x.0) != Some(id) {
            return;
        }
        let Some(it) = self.board.items.iter().find(|it| it.id == id) else { return };
        let Kind::Table(t) = &it.kind else { return };
        let (x, y) = t.cell_origin(r, cc);
        let rect = Rect::from_min_max(self.w2s(x, y), self.w2s(x + t.cw[cc], y + t.rh[r]));
        pt.rect_stroke(rect, 0.0, Stroke::new(2.0, c(style::pal().accent)), egui::StrokeKind::Middle);
    }
}

/// Knöpfe der Auswahlleiste bei einer ausgewählten Tabelle.
pub(super) fn sel_buttons(ui: &mut Ui, ed: &mut Editor, sh: &mut Shared, btn: f32) {
    let Some((id, r, cc)) = ed.sel_table() else { return };
    let p = style::pal();
    if icon_btn(ui, &mut sh.icons, ic::TEXT, p.text, false, false, btn, "Zelle bearbeiten (oder in die Zelle tippen)").clicked() {
        ed.edit_cell(id, r, cc);
    }
    let h = (btn - 8.0).max(30.0);
    let (rows, cols) = dims(&ed.board, id).unwrap_or((1, 1));
    let mut cell = None;
    if style::chip(ui, "+ Zeile", false, h).on_hover_text("Zeile unter der aktuellen Zelle einfügen").clicked() && ed.board.add_row(id, r + 1) {
        cell = Some((id, r + 1, cc));
    }
    if rows > 1 && style::chip(ui, "− Zeile", false, h).on_hover_text("Zeile der aktuellen Zelle entfernen").clicked() && ed.board.remove_row(id, r) {
        cell = Some((id, r.min(rows - 2), cc));
    }
    if style::chip(ui, "+ Spalte", false, h).on_hover_text("Spalte rechts der aktuellen Zelle einfügen").clicked() && ed.board.add_col(id, cc + 1) {
        cell = Some((id, r, cc + 1));
    }
    if cols > 1 && style::chip(ui, "− Spalte", false, h).on_hover_text("Spalte der aktuellen Zelle entfernen").clicked() && ed.board.remove_col(id, cc) {
        cell = Some((id, r, cc.min(cols - 2)));
    }
    if cell.is_some() {
        ed.xs.table.cell = cell;
    }
}

/// Optionen des Werkzeugs: Zeilen und Spalten.
pub(super) fn options(ui: &mut Ui, ed: &mut Editor) {
    style::caption(ui, "Größe");
    ui.horizontal(|ui| {
        ui.label("Zeilen");
        ui.add(egui::DragValue::new(&mut ed.xs.table.rows).range(1..=50));
        ui.add_space(12.0);
        ui.label("Spalten");
        ui.add(egui::DragValue::new(&mut ed.xs.table.cols).range(1..=20));
    });
    ui.label(egui::RichText::new("Tippe auf das Blatt, um die Tabelle zu setzen. Tippe dann in eine Zelle, um sie zu beschriften.").size(12.0).color(c(style::pal().dim)));
}

#[cfg(test)]
mod tests {
    use crate::ui::editor::clip::tests::editor;

    #[test]
    fn zelle_bearbeiten_und_zeilen() {
        let mut ed = editor(false);
        let id = ed.board.add_table(10.0, 10.0, 2, 2).unwrap();
        ed.board.set_tool(owndraw_core::board::SELECT);
        ed.board.select_only(id);
        ed.edit_cell(id, 1, 0);
        ed.text.as_mut().unwrap().text = "Hallo".into();
        ed.finish_text();
        assert!(ed.text.is_none());
        assert_eq!(super::cell_at(&ed.board, 12.0, 12.0).map(|x| x.0), Some(id));
        match &ed.board.items.iter().find(|i| i.id == id).unwrap().kind {
            owndraw_core::item::Kind::Table(t) => assert_eq!(t.cell(1, 0), "Hallo"),
            _ => panic!(),
        }
        assert_eq!(ed.sel_table(), Some((id, 1, 0)));
    }
}
