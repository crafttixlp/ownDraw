//! Leisten des Editors: oben (Zurück/Name, Rückgängig/Wiederholen, ⋯), Werkzeugleiste, Auswahl-Aktionen.

use super::{clip, Editor, Lay, Tool, M, PAD};
use crate::app::{Req, Shared};
use crate::layout::FormFactor;
use crate::settings::BarPos;
use crate::style::{self, c, icon_btn};
use egui::{Align2, Color32, Order, Pos2, Rect, Sense, Stroke, Ui, Vec2};
use owndraw_core::board;
use owndraw_core::icons as ic;

pub(super) fn show(ed: &mut Editor, ctx: &egui::Context, sh: &mut Shared, lay: &Lay, tools: &[Tool]) {
    top_bar(ed, ctx, sh, lay);
    tool_bar(ed, ctx, sh, lay, tools);
    selection_bar(ed, ctx, sh, lay);
}

fn top_bar(ed: &mut Editor, ctx: &egui::Context, sh: &mut Shared, lay: &Lay) {
    let env = lay.env;
    let p = style::pal();
    let canvas = lay.canvas;
    let btn = env.btn;
    let compact = env.ff == FormFactor::Compact;
    let right_w = 3.0 * btn + if compact { 0.0 } else { 64.0 } + 2.0 * PAD;
    let name_max = (canvas.width() - 2.0 * M - btn - 2.0 * PAD - right_w - 3.0 * M - 14.0).clamp(40.0, 260.0);

    // links: Zurück + Titel
    let r = Editor::float(ctx, "bar-tl", Pos2::new(canvas.left() + M, canvas.top() + M), Align2::LEFT_TOP, canvas, Order::Middle, |ui| {
        style::pill(26).show(ui, |ui| {
            ui.horizontal(|ui| {
                ui.spacing_mut().item_spacing.x = 2.0;
                if icon_btn(ui, &mut sh.icons, ic::BACK, p.text, false, false, btn, "Zurück zum Start").clicked() {
                    sh.reqs.push(Req::Back);
                }
                // Speicherstatus als Punkt
                let (label, err) = ed.save_state();
                if !label.is_empty() {
                    let (rect, resp) = ui.allocate_exact_size(Vec2::new(10.0, btn), Sense::hover());
                    let col = if err { p.danger } else if label.starts_with("Änderungen") { 0xFFFFC53D } else { 0xFF30A46C };
                    ui.painter().circle_filled(rect.center(), 3.5, c(col));
                    resp.on_hover_text(label);
                }
                ui.scope(|ui| {
                    ui.set_max_width(name_max);
                    let t = egui::RichText::new(ed.name()).size(15.0).strong().color(c(p.text));
                    let r = ui.add(egui::Button::new(t).fill(Color32::TRANSPARENT).truncate().min_size(Vec2::new(0.0, btn - 6.0))).on_hover_text("Board umbenennen");
                    if r.clicked() {
                        sh.dialog = Some(crate::ui::dialogs::Dialog::Rename { what: crate::ui::dialogs::RenameWhat::Doc, title: "Board umbenennen", text: ed.name().to_string(), focus: true });
                    }
                });
                ui.add_space(6.0);
            });
        });
    });
    ed.st.popup_rects.push(r);

    // rechts: Rückgängig, Wiederholen, Zoom, ⋯
    let r = Editor::float(ctx, "bar-tr", Pos2::new(canvas.right() - M, canvas.top() + M), Align2::RIGHT_TOP, canvas, Order::Middle, |ui| {
        style::pill(26).show(ui, |ui| {
            ui.horizontal(|ui| {
                ui.spacing_mut().item_spacing.x = 0.0;
                let b = &mut ed.board;
                let u = if b.can_undo() { p.text } else { p.disabled };
                if icon_btn(ui, &mut sh.icons, ic::UNDO, u, false, false, btn, &env.tip("Rückgängig", "Strg+Z")).clicked() {
                    b.undo();
                }
                let rd = if b.can_redo() { p.text } else { p.disabled };
                if icon_btn(ui, &mut sh.icons, ic::REDO, rd, false, false, btn, &env.tip("Wiederholen", "Strg+Y")).clicked() {
                    b.redo();
                }
                if !compact {
                    let t = format!("{} %", (ed.board.sc * 100.0).round() as i32);
                    let r = ui.add(egui::Button::new(egui::RichText::new(t).size(13.0)).fill(Color32::TRANSPARENT).min_size(Vec2::new(60.0, btn - 6.0)));
                    if r.on_hover_text(env.tip("Alles zeigen", "0")).clicked() {
                        ed.board.fit();
                    }
                }
                let more = icon_btn(ui, &mut sh.icons, ic::MORE, p.text, ed.st.menu_open, false, btn, "Menü");
                ed.st.menu_btn = Some(more.rect);
                if more.clicked() {
                    ed.st.menu_open = !ed.st.menu_open;
                    ed.st.opts_open = false;
                }
            });
        });
    });
    ed.st.popup_rects.push(r);
}

fn tool_icon(ed: &Editor, t: Tool) -> Option<i32> {
    let b = &ed.board;
    Some(match t {
        Tool::Pen => ic::PEN,
        Tool::Marker => ic::MARKER,
        Tool::Eraser => {
            if b.erase_mode == 1 {
                ic::ERASER_PART
            } else {
                ic::ERASER
            }
        }
        Tool::Select => {
            if b.sel_mode == 0 {
                ic::LASSO
            } else {
                ic::SELECT
            }
        }
        Tool::Shape => match b.shape_kind {
            0 => ic::RECT,
            1 => ic::ELLIPSE,
            2 => ic::LINE,
            3 => ic::ARROW,
            _ => return None, // Maschinenbau-Pfeil: gezeichnetes Icon
        },
        Tool::Text => {
            if b.tool == board::NOTE {
                ic::NOTE
            } else {
                ic::TEXT
            }
        }
        Tool::Image => ic::IMAGE,
    })
}

fn tool_btn(ui: &mut Ui, ed: &mut Editor, sh: &mut Shared, t: Tool, size: f32) {
    let p = style::pal();
    let active = t != Tool::Image && ed.active_tool() == t;
    let (rect, resp) = ui.allocate_exact_size(Vec2::splat(size), Sense::click());
    style::btn_bg(ui, rect, &resp, active);
    let tint = if active { p.accent } else { p.text };
    let isz = (size * 0.55).round();
    match tool_icon(ed, t) {
        Some(id) => sh.icons.paint(ui, rect, id, tint, false, isz),
        None => style::paint_arrow_eng(ui.painter(), Rect::from_center_size(rect.center(), Vec2::splat(isz + 4.0)), c(tint)),
    }
    // Farbpunkt als Hinweis auf die aktuelle Farbe
    if matches!(t, Tool::Pen | Tool::Marker | Tool::Shape | Tool::Text) {
        let pos = rect.right_bottom() - Vec2::new(size * 0.22, size * 0.22);
        ui.painter().circle_filled(pos, 4.5, c(ed.board.color));
        ui.painter().circle_stroke(pos, 4.5, Stroke::new(1.0, c(p.line)));
    }
    ed.st.tool_rects.push((t, rect));
    let tip = ed.tool_tip(&sh.env, t);
    if resp.on_hover_text(tip).clicked() {
        if t == Tool::Image {
            ed.pick_image(sh);
        } else if active {
            ed.st.opts_open = !ed.st.opts_open;
            ed.st.menu_open = false;
        } else {
            ed.select_tool(sh, t);
            ed.st.opts_open = false;
        }
    }
}

impl Editor {
    fn tool_tip(&self, env: &crate::layout::Env, t: Tool) -> String {
        let mut s = env.tip(t.label(), t.key());
        if t.has_options() && self.active_tool() == t {
            s.push_str(" · erneut tippen: Optionen");
        }
        s
    }
}

fn tool_bar(ed: &mut Editor, ctx: &egui::Context, sh: &mut Shared, lay: &Lay, tools: &[Tool]) {
    let canvas = lay.canvas;
    let btn = lay.env.btn;
    let (pos, pivot, horiz) = match lay.tb {
        BarPos::Top => {
            let y = canvas.top() + M + if lay.row2 { lay.thick + M } else { 0.0 };
            (Pos2::new(canvas.center().x, y), Align2::CENTER_TOP, true)
        }
        BarPos::Bottom => (Pos2::new(canvas.center().x, canvas.bottom() - M), Align2::CENTER_BOTTOM, true),
        BarPos::Left => (Pos2::new(canvas.left() + M, lay.y_below), Align2::LEFT_TOP, false),
        BarPos::Right => (Pos2::new(canvas.right() - M, lay.y_below), Align2::RIGHT_TOP, false),
    };
    let r = Editor::float(ctx, "bar-tools", pos, pivot, canvas, Order::Middle, |ui| {
        style::pill(26).show(ui, |ui| {
            let body = |ui: &mut Ui| {
                ui.spacing_mut().item_spacing = Vec2::ZERO;
                for &t in tools {
                    tool_btn(ui, ed, sh, t, btn);
                }
            };
            if horiz {
                ui.horizontal(body);
            } else {
                ui.vertical(body);
            }
        });
    });
    ed.st.popup_rects.push(r);
}

fn selection_bar(ed: &mut Editor, ctx: &egui::Context, sh: &mut Shared, lay: &Lay) {
    let b = &ed.board;
    let has_sel = !b.sel.is_empty();
    if b.tool != board::SELECT || !(has_sel || b.has_clip()) {
        return;
    }
    // Auf dem Handy verdeckt das Optionen-Sheet diese Leiste.
    if lay.env.ff == FormFactor::Compact && ed.st.opts_open {
        return;
    }
    let p = style::pal();
    let btn = lay.env.btn;
    let canvas = lay.canvas;
    let single_note = b.single_note().map(|n| n.id);
    let bottom = canvas.bottom() - M - if lay.tb == BarPos::Bottom { lay.thick + M } else { 0.0 };
    let maxw = canvas.width() - 2.0 * M;
    let r = Editor::float(ctx, "bar-sel", Pos2::new(canvas.center().x, bottom), Align2::CENTER_BOTTOM, canvas, Order::Middle, |ui| {
        style::pill(26).show(ui, |ui| {
            ui.set_max_width(maxw - 2.0 * PAD);
            ui.horizontal_wrapped(|ui| {
                ui.spacing_mut().item_spacing = Vec2::ZERO;
                let env = sh.env;
                if has_sel {
                    if icon_btn(ui, &mut sh.icons, ic::COPY, p.text, false, false, btn, &env.tip("Kopieren", "Strg+C")).clicked() {
                        clip::copy(ed, &*sh.platform);
                    }
                    if icon_btn(ui, &mut sh.icons, ic::CUT, p.text, false, false, btn, &env.tip("Ausschneiden", "Strg+X")).clicked() {
                        clip::cut(ed, &*sh.platform);
                    }
                }
                if icon_btn(ui, &mut sh.icons, ic::PASTE, p.text, false, false, btn, &env.tip("Einfügen", "Strg+V")).clicked() {
                    clip::paste_center(ed, &*sh.platform);
                }
                if has_sel {
                    if icon_btn(ui, &mut sh.icons, ic::DUPLICATE, p.text, false, false, btn, &env.tip("Duplizieren", "Strg+D")).clicked() {
                        ed.board.duplicate_sel();
                    }
                    if icon_btn(ui, &mut sh.icons, ic::ROTATE, p.text, false, false, btn, "Um 90° drehen").clicked() {
                        ed.board.rotate_sel(std::f32::consts::FRAC_PI_2);
                    }
                    if icon_btn(ui, &mut sh.icons, ic::FRONT, p.text, false, false, btn, "Ganz nach vorne").clicked() {
                        ed.board.bring_forward();
                    }
                    if icon_btn(ui, &mut sh.icons, ic::BACKWARD, p.text, false, false, btn, "Ganz nach hinten").clicked() {
                        ed.board.send_backward();
                    }
                    if let Some(nid) = single_note {
                        if icon_btn(ui, &mut sh.icons, ic::PEN, p.text, false, false, btn, "Text bearbeiten").clicked() {
                            ed.edit_note(nid);
                        }
                    }
                    if icon_btn(ui, &mut sh.icons, ic::TRASH, p.danger, false, false, btn, &env.tip("Löschen", "Entf")).clicked() {
                        ed.board.delete_sel();
                    }
                }
            });
        });
    });
    ed.st.popup_rects.push(r);
}
