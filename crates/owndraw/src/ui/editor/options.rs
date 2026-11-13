//! Werkzeug-Optionen: Kontext-Popover (Medium/Expanded) bzw. Bottom-Sheet (Compact).

use super::{Editor, Lay, Tool, COLORS, M, PAD, WIDTHS};
use crate::app::Shared;
use crate::layout::{min_touch, FormFactor};
use crate::settings::BarPos;
use crate::style::{self, c};
use egui::{Align2, CornerRadius, Order, Pos2, Sense, Stroke, Ui, Vec2};
use owndraw_core::board;
use owndraw_core::icons as ic;

pub(super) fn show(ed: &mut Editor, ctx: &egui::Context, sh: &mut Shared, lay: &Lay) {
    if !ed.st.opts_open {
        return;
    }
    let t = ed.active_tool();
    if !t.has_options() {
        ed.st.opts_open = false;
        return;
    }
    let canvas = lay.canvas;
    if lay.env.ff == FormFactor::Compact {
        // Bottom-Sheet über der Werkzeugleiste
        let bottom = canvas.bottom() - if lay.tb == BarPos::Bottom { lay.thick + 2.0 * M } else { 0.0 };
        let w = canvas.width();
        let max_h = (canvas.height() * 0.55).max(160.0);
        let r = Editor::float(ctx, "opts-sheet", Pos2::new(canvas.left(), bottom), Align2::LEFT_BOTTOM, canvas, Order::Foreground, |ui| {
            style::sheet(20).show(ui, |ui| {
                ui.set_width(w - 34.0);
                // Griff
                let (hr, resp) = ui.allocate_exact_size(Vec2::new(ui.available_width(), 14.0), Sense::click());
                ui.painter().rect_filled(egui::Rect::from_center_size(hr.center(), Vec2::new(40.0, 4.0)), CornerRadius::same(2), c(style::pal().line));
                if resp.clicked() {
                    ed.st.opts_open = false;
                }
                ui.horizontal(|ui| {
                    ui.label(egui::RichText::new(t.label()).size(16.0).strong());
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        if style::plain_btn(ui, "Fertig", min_touch(sh.env.touch).max(36.0)).clicked() {
                            ed.st.opts_open = false;
                        }
                    });
                });
                egui::ScrollArea::vertical().max_height(max_h).auto_shrink([false, true]).show(ui, |ui| content(ui, ed, sh, t));
            });
        });
        ed.st.popup_rects.push(r);
        return;
    }
    // Popover neben dem Werkzeugknopf
    let Some(anchor) = ed.st.tool_rects.iter().find(|(x, _)| *x == t).map(|(_, r)| *r) else { return };
    let (pos, pivot) = match lay.tb {
        BarPos::Left => (Pos2::new(anchor.right() + M + PAD, anchor.top()), Align2::LEFT_TOP),
        BarPos::Right => (Pos2::new(anchor.left() - M - PAD, anchor.top()), Align2::RIGHT_TOP),
        BarPos::Top => (Pos2::new(anchor.center().x, anchor.bottom() + M + PAD), Align2::CENTER_TOP),
        BarPos::Bottom => (Pos2::new(anchor.center().x, anchor.top() - M - PAD), Align2::CENTER_BOTTOM),
    };
    let r = Editor::float(ctx, "opts-pop", pos, pivot, canvas.shrink(M), Order::Foreground, |ui| {
        style::pill(18).show(ui, |ui| {
            egui::Frame::new().inner_margin(egui::Margin::same(8)).show(ui, |ui| {
                ui.set_width(268.0);
                ui.label(egui::RichText::new(t.label()).size(15.0).strong());
                ui.add_space(2.0);
                content(ui, ed, sh, t);
            });
        });
    });
    ed.st.popup_rects.push(r);
}

fn content(ui: &mut Ui, ed: &mut Editor, sh: &mut Shared, t: Tool) {
    let h = min_touch(sh.env.touch).max(34.0);
    match t {
        Tool::Pen | Tool::Marker => {
            colors(ui, ed, sh);
            widths(ui, ed, sh, "Breite");
            recog(ui, ed, h);
            if sh.env.touch || ed.board.finger_draw {
                ui.add_space(4.0);
                let mut f = ed.board.finger_draw;
                if ui.checkbox(&mut f, "Mit dem Finger zeichnen").changed() {
                    ed.board.finger_draw = f;
                }
            }
        }
        Tool::Shape => {
            style::caption(ui, "Form");
            let kinds = shape_kinds();
            ui.horizontal_wrapped(|ui| {
                ui.spacing_mut().item_spacing = Vec2::ZERO;
                for (i, (icon, name)) in kinds.iter().enumerate() {
                    let on = ed.board.shape_kind == i as i32;
                    let size = sh.env.btn;
                    let (rect, resp) = ui.allocate_exact_size(Vec2::splat(size), Sense::click());
                    style::btn_bg(ui, rect, &resp, on);
                    let tint = if on { style::pal().accent } else { style::pal().text };
                    match icon {
                        Some(id) => sh.icons.paint(ui, rect, *id, tint, false, (size * 0.55).round()),
                        None => style::paint_arrow_eng(ui.painter(), egui::Rect::from_center_size(rect.center(), Vec2::splat(size * 0.55 + 4.0)), c(tint)),
                    }
                    if resp.on_hover_text(*name).clicked() {
                        ed.board.shape_kind = i as i32;
                    }
                }
            });
            let k = ed.board.shape_kind.clamp(0, 4) as usize;
            let mut lbl = kinds[k].1.to_string();
            if k == 4 {
                lbl.push_str(" (15°, gefüllt)");
            }
            ui.label(egui::RichText::new(lbl).size(12.0).color(c(style::pal().dim)));
            colors(ui, ed, sh);
            widths(ui, ed, sh, "Linienbreite");
        }
        Tool::Eraser => {
            style::caption(ui, "Modus");
            ui.horizontal_wrapped(|ui| {
                for (m, l) in [(0, "Ganze Objekte"), (1, "Nur überstrichene Stellen")] {
                    if style::chip(ui, l, ed.board.erase_mode == m, h).clicked() {
                        ed.board.erase_mode = m;
                        ed.board.invalidate();
                    }
                }
            });
            if ed.board.erase_mode == 1 {
                widths(ui, ed, sh, "Größe");
            }
        }
        Tool::Select => {
            style::caption(ui, "Modus");
            ui.horizontal_wrapped(|ui| {
                for (m, l) in [(0, "Lasso"), (1, "Rechteck")] {
                    if style::chip(ui, l, ed.board.sel_mode == m, h).clicked() {
                        ed.board.sel_mode = m;
                        ed.board.invalidate();
                    }
                }
            });
            let hint = if ed.board.sel_mode == 0 { "Objekte freihand umkreisen." } else { "Rechteck über die Objekte aufziehen." };
            ui.label(egui::RichText::new(hint).size(12.0).color(c(style::pal().dim)));
        }
        Tool::Text => {
            style::caption(ui, "Art");
            ui.horizontal_wrapped(|ui| {
                for (bt, l) in [(board::TEXT, "Text"), (board::NOTE, "Haftnotiz")] {
                    if style::chip(ui, l, ed.board.tool == bt, h).clicked() {
                        ed.board.set_tool(bt);
                    }
                }
            });
            colors(ui, ed, sh);
        }
        Tool::Image => {}
    }
}

/// Symbol (None = gezeichnet) und Name der Formen; Index = `Board::shape_kind`.
fn shape_kinds() -> [(Option<i32>, &'static str); 5] {
    [(Some(ic::RECT), "Rechteck"), (Some(ic::ELLIPSE), "Ellipse"), (Some(ic::LINE), "Linie"), (Some(ic::ARROW), "Pfeil"), (None, "Maschinenbau-Pfeil")]
}

fn colors(ui: &mut Ui, ed: &mut Editor, sh: &Shared) {
    style::caption(ui, "Farbe");
    let p = style::pal();
    let avail = ui.available_width();
    let size = sh.env.btn.min(avail / COLORS.len() as f32).max(28.0);
    ui.horizontal_wrapped(|ui| {
        ui.spacing_mut().item_spacing = Vec2::ZERO;
        for (col, name) in COLORS {
            let (r, resp) = ui.allocate_exact_size(Vec2::splat(size), Sense::click());
            let pt = ui.painter();
            pt.circle_filled(r.center(), 9.0, c(col));
            if col == owndraw_core::item::INK {
                pt.circle_stroke(r.center(), 9.0, Stroke::new(1.0, c(0x80808080)));
            }
            if ed.board.color == col {
                pt.circle_stroke(r.center(), 13.0, Stroke::new(2.0, c(p.text)));
            } else if resp.hovered() {
                pt.circle_stroke(r.center(), 13.0, Stroke::new(1.5, c(p.line)));
            }
            if resp.on_hover_text(name).clicked() {
                ed.board.set_color(col);
                ed.board.invalidate();
            }
        }
    });
}

fn widths(ui: &mut Ui, ed: &mut Editor, sh: &Shared, title: &str) {
    style::caption(ui, title);
    let p = style::pal();
    let avail = ui.available_width();
    let w = (avail / WIDTHS.len() as f32).min(72.0);
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing = Vec2::ZERO;
        for (i, wd) in WIDTHS.iter().enumerate() {
            let (r, resp) = ui.allocate_exact_size(Vec2::new(w, sh.env.btn), Sense::click());
            let d = [4.0, 7.0, 11.0, 16.0][i];
            let on = ed.board.pen_w == *wd;
            let bg = r.shrink2(Vec2::new(2.0, 4.0));
            if on {
                ui.painter().rect_filled(bg, CornerRadius::same(255), c(p.active));
            } else if resp.hovered() {
                ui.painter().rect_filled(bg, CornerRadius::same(255), c(p.hover));
            }
            ui.painter().circle_filled(r.center(), d / 2.0, c(p.text));
            if resp.on_hover_text(format!("Strichstärke {}", i + 1)).clicked() {
                ed.board.pen_w = *wd;
            }
        }
    });
}

fn recog(ui: &mut Ui, ed: &mut Editor, h: f32) {
    style::caption(ui, "Formerkennung");
    ui.horizontal_wrapped(|ui| {
        for (m, l) in [(0, "Halten"), (1, "Immer"), (2, "Aus")] {
            if style::chip(ui, l, ed.board.recog == m, h).clicked() {
                ed.board.recog = m;
            }
        }
    });
    let hint = ["Am Ende des Strichs kurz halten, dann wird die Form geglättet.", "Jeder Strich wird, wenn möglich, zur Form.", "Striche bleiben, wie gezeichnet."][ed.board.recog.clamp(0, 2) as usize];
    ui.label(egui::RichText::new(hint).size(12.0).color(c(style::pal().dim)));
}
