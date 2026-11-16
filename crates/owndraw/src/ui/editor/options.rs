//! Werkzeug-Optionen: Kontext-Popover (Medium/Expanded) bzw. Bottom-Sheet (Compact).

use super::{Editor, Lay, Tool, COLORS, M, PAD, WIDTHS};
use crate::app::Shared;
use crate::layout::{min_touch, FormFactor};
use crate::settings::BarPos;
use crate::style::{self, c};
use egui::{Align2, CornerRadius, Order, Pos2, Sense, Stroke, Ui, Vec2};
use owndraw_core::board;
use owndraw_core::icons as ic;
use owndraw_core::item;

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
        let bottom = canvas.bottom() - if lay.tb_edge == Some(BarPos::Bottom) { lay.thick + 2.0 * M } else { 0.0 };
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
    // Seite nach Lage der Werkzeugleiste: senkrecht -> rechts/links daneben, waagerecht -> darunter/darüber
    let side = if ed.st.tools_vert {
        if anchor.center().x < canvas.center().x {
            BarPos::Left
        } else {
            BarPos::Right
        }
    } else if anchor.center().y < canvas.center().y {
        BarPos::Top
    } else {
        BarPos::Bottom
    };
    let (pos, pivot) = match side {
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
            if ed.board.recog != 2 {
                arrow_styles(ui, ed, sh, "Erkannte Pfeile werden zu");
            }
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
            if k == 3 {
                arrow_styles(ui, ed, sh, "Pfeilstil");
            }
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
    let hint = ["Halte am Strichende kurz an, dann wird die Form geglättet.", "Jeder Strich wird, wenn möglich, in eine Form umgewandelt.", "Striche bleiben, wie gezeichnet."][ed.board.recog.clamp(0, 2) as usize];
    ui.label(egui::RichText::new(hint).size(12.0).color(c(style::pal().dim)));
}

/// Auswahl des Pfeilstils mit kleiner Vorschau je Stil (Index = `Board::arrow_style`).
fn arrow_styles(ui: &mut Ui, ed: &mut Editor, sh: &Shared, title: &str) {
    style::caption(ui, title);
    let p = style::pal();
    let size = Vec2::new(sh.env.btn * 1.6, sh.env.btn);
    ui.horizontal_wrapped(|ui| {
        ui.spacing_mut().item_spacing = Vec2::ZERO;
        for (i, (name, single, _)) in item::ARROW_STYLES.iter().enumerate() {
            let on = ed.board.arrow_style == i as i32;
            let (rect, resp) = ui.allocate_exact_size(size, Sense::click());
            style::btn_bg(ui, rect, &resp, on);
            let tint = c(if on { p.accent } else { p.text });
            if i == 0 {
                ui.painter().text(rect.center(), Align2::CENTER_CENTER, "Auto", egui::FontId::proportional(12.0), tint);
            } else {
                paint_arrow_preview(ui.painter(), rect.shrink2(Vec2::new(10.0, rect.height() * 0.3)), *single, i == 4 || i == 5, tint);
            }
            if resp.on_hover_text(*name).clicked() {
                ed.board.arrow_style = i as i32;
            }
        }
    });
    let n = item::ARROW_STYLES.get(ed.board.arrow_style.max(0) as usize).map(|s| s.0).unwrap_or("");
    ui.label(egui::RichText::new(n).size(12.0).color(c(p.dim)));
}

/// Kleine Pfeil-Vorschau von links nach rechts im Rechteck r.
fn paint_arrow_preview(pt: &egui::Painter, r: egui::Rect, kind: i32, double: bool, col: egui::Color32) {
    let (a, b) = (Pos2::new(r.left(), r.center().y), Pos2::new(r.right(), r.center().y));
    let st = Stroke::new(1.8, col);
    let (filled, dashed, eng) = (matches!(kind, item::ARROW_FILLED | item::ARROW2_FILLED), kind == item::ARROW_DASHED, kind == item::ARROW_ENG);
    let head = |tip: Pos2, dir: f32| {
        let (len, half) = if eng { (11.0, 11.0 * 7.5f32.to_radians().tan()) } else { (8.0, 5.0) };
        let base = Pos2::new(tip.x - dir * len, tip.y);
        let (p1, p2) = (Pos2::new(base.x, base.y - half), Pos2::new(base.x, base.y + half));
        if filled || eng {
            pt.add(egui::Shape::convex_polygon(vec![tip, p1, p2], col, Stroke::NONE));
        } else {
            pt.line_segment([tip, p1], st);
            pt.line_segment([tip, p2], st);
        }
    };
    if dashed {
        pt.add(egui::Shape::dashed_line(&[a, b], st, 4.0, 3.0));
    } else {
        pt.line_segment([a, b], st);
    }
    head(b, 1.0);
    if double {
        head(a, -1.0);
    }
}
