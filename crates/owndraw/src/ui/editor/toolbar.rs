//! Leisten des Editors: oben (Zurück/Name, Rückgängig/Wiederholen, ⋯), Werkzeugleiste, Auswahl-Aktionen.

use super::{clip, Editor, Lay, Tool, M, PAD};
use crate::app::{Req, Shared};
use crate::layout::FormFactor;
use crate::settings::{self, BarPos};
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
    if ed.st.edit_tools.is_some() {
        edit_bar(ed, ctx, sh, lay);
        return;
    }
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

// ---------------------------------------------------------------- Bearbeiten-Modus (WP9)

/// Zustand des Bearbeiten-Modus: alle Werkzeuge mit Sichtbarkeit, laufender Zieh-Vorgang.
pub struct EditTools {
    pub list: Vec<settings::ToolEntry>,
    drag: Option<String>,
    /// Letzte Zeigerposition beim Ziehen (Touch liefert beim Loslassen keine mehr).
    last: Option<Pos2>,
}

impl Editor {
    /// Startet den Bearbeiten-Modus der Werkzeugleiste (aus den Einstellungen heraus).
    pub fn begin_edit_tools(&mut self, sh: &Shared) {
        self.st.opts_open = false;
        self.st.menu_open = false;
        self.st.edit_tools = Some(EditTools { list: settings::edit_list(&sh.settings.tools), drag: None, last: None });
    }
}

fn apply_edit(sh: &mut Shared, list: &[settings::ToolEntry]) {
    sh.settings.tools = settings::tools_from_edit(list);
}

/// Werkzeugleiste im Bearbeiten-Modus: alle Werkzeuge, Ziehen ordnet um, Tippen blendet ein/aus.
fn edit_bar(ed: &mut Editor, ctx: &egui::Context, sh: &mut Shared, lay: &Lay) {
    let Some(mut st) = ed.st.edit_tools.take() else { return };
    let canvas = lay.canvas;
    let btn = lay.env.btn;
    let p = style::pal();
    let (pos, pivot, horiz) = match lay.tb {
        BarPos::Top => {
            let y = canvas.top() + M + if lay.row2 { lay.thick + M } else { 0.0 };
            (Pos2::new(canvas.center().x, y), Align2::CENTER_TOP, true)
        }
        BarPos::Bottom => (Pos2::new(canvas.center().x, canvas.bottom() - M), Align2::CENTER_BOTTOM, true),
        BarPos::Left => (Pos2::new(canvas.left() + M, lay.y_below), Align2::LEFT_TOP, false),
        BarPos::Right => (Pos2::new(canvas.right() - M, lay.y_below), Align2::RIGHT_TOP, false),
    };
    let mut rects: Vec<(usize, Rect)> = Vec::new();
    let mut dragged: Option<(usize, bool)> = None; // (Index, losgelassen?)
    let mut clicked: Option<usize> = None;
    let area = egui::Area::new(egui::Id::unique("bar-tools-edit")).order(Order::Middle).fixed_pos(pos).pivot(pivot).constrain_to(canvas).show(ctx, |ui| {
        style::pill(26).show(ui, |ui| {
            let body = |ui: &mut Ui| {
                ui.spacing_mut().item_spacing = Vec2::ZERO;
                for (i, (id, on)) in st.list.iter().enumerate() {
                    let Some(t) = Tool::from_id(id) else { continue };
                    let (rect, resp) = ui.allocate_exact_size(Vec2::splat(btn), Sense::click_and_drag());
                    let resp = resp.on_hover_text(format!("{} – ziehen: verschieben, tippen: {}", t.label(), if *on { "ausblenden" } else { "einblenden" }));
                    rects.push((i, rect));
                    if resp.dragged() || resp.drag_started() {
                        dragged = Some((i, false));
                    }
                    if resp.drag_stopped() {
                        dragged = Some((i, true));
                    }
                    if resp.clicked() {
                        clicked = Some(i);
                    }
                    let is_drag = st.drag.as_deref() == Some(id.as_str());
                    style::btn_bg(ui, rect, &resp, is_drag);
                    let tint = if *on { p.text } else { p.disabled };
                    let isz = (btn * 0.5).round();
                    match tool_icon(ed, t) {
                        Some(ico) => sh.icons.paint(ui, rect, ico, tint, false, isz),
                        None => style::paint_arrow_eng(ui.painter(), Rect::from_center_size(rect.center(), Vec2::splat(isz + 4.0)), c(tint)),
                    }
                    // Auge oben rechts
                    let eye_r = Rect::from_center_size(rect.right_top() + Vec2::new(-btn * 0.24, btn * 0.3), Vec2::splat(13.0));
                    ui.painter().circle_filled(eye_r.center(), 8.0, c(p.card));
                    sh.icons.paint(ui, eye_r, if *on { ic::EYE } else { ic::EYE_OFF }, if *on { p.accent } else { p.dim }, false, 12.0);
                }
            };
            if horiz {
                ui.horizontal(body);
            } else {
                ui.vertical(body);
            }
        });
    });
    let bar_rect = area.response.rect;
    ed.st.popup_rects.push(bar_rect);

    // Tippen: ein-/ausblenden
    if let Some(i) = clicked {
        if settings::toggle_visible(&mut st.list, i) {
            apply_edit(sh, &st.list);
        } else {
            sh.toast("Mindestens ein Werkzeug bleibt sichtbar");
        }
    }
    // Ziehen: Ziel nach Mittelpunkten, Einfügemarke, Loslassen ordnet um
    if let Some((i, released)) = dragged {
        st.drag = Some(st.list[i].0.clone());
        let ptr = ctx.input(|inp| inp.pointer.interact_pos().or(inp.pointer.latest_pos())).or(st.last);
        st.last = ptr;
        if let Some(ptr) = ptr {
            let key = |r: &Rect| if horiz { (r.center().x - ptr.x).abs() } else { (r.center().y - ptr.y).abs() };
            let target = rects.iter().min_by(|a, b| key(&a.1).total_cmp(&key(&b.1))).map(|(k, _)| *k).unwrap_or(i);
            if released {
                if target != i {
                    settings::reorder(&mut st.list, i, target);
                    apply_edit(sh, &st.list);
                }
                st.drag = None;
            } else {
                // Einfügemarke an der Zielkante
                if let Some((_, tr)) = rects.iter().find(|(k, _)| *k == target) {
                    let after = target > i;
                    let painter = ctx.layer_painter(egui::LayerId::new(Order::Tooltip, egui::Id::unique("tools-edit-mark")));
                    let stroke = Stroke::new(3.0, c(p.accent));
                    if horiz {
                        let x = if after { tr.right() } else { tr.left() };
                        painter.line_segment([Pos2::new(x, tr.top() + 4.0), Pos2::new(x, tr.bottom() - 4.0)], stroke);
                    } else {
                        let y = if after { tr.bottom() } else { tr.top() };
                        painter.line_segment([Pos2::new(tr.left() + 4.0, y), Pos2::new(tr.right() - 4.0, y)], stroke);
                    }
                    // Geist unter dem Zeiger
                    if let Some(t) = Tool::from_id(&st.list[i].0) {
                        let ghost = Rect::from_center_size(ptr, Vec2::splat(btn));
                        painter.circle_filled(ghost.center(), btn * 0.5, c(p.active));
                        if let Some(ico) = tool_icon(ed, t) {
                            if let Some(tex) = sh.icons.get(ctx, ico, p.accent, false, (btn * 0.5).round()) {
                                painter.image(tex.id(), Rect::from_center_size(ghost.center(), Vec2::splat((btn * 0.5).round())), Rect::from_min_max(Pos2::ZERO, Pos2::new(1.0, 1.0)), Color32::WHITE);
                            }
                        }
                    }
                }
            }
        }
    } else {
        st.drag = None;
    }

    // Bedienfeld: Hinweis, Standard, Fertig (Mitte der Zeichenfläche)
    let mut done = false;
    let mut reset = false;
    let maxw = (canvas.width() - 2.0 * M - 2.0 * PAD).min(400.0);
    let panel = Editor::float(ctx, "tools-edit-panel", canvas.center(), Align2::CENTER_CENTER, canvas, Order::Middle, |ui| {
        style::card().show(ui, |ui| {
            ui.set_max_width(maxw - 40.0);
            ui.vertical(|ui| {
                ui.label(egui::RichText::new("Werkzeuge anpassen").strong().size(16.0));
                ui.label(egui::RichText::new("Ziehen ordnet um, Tippen blendet ein oder aus (Auge).").size(13.0).color(c(p.dim)));
                ui.add_space(6.0);
                ui.horizontal_wrapped(|ui| {
                    if style::plain_btn(ui, "Standard", lay.env.btn).clicked() {
                        reset = true;
                    }
                    if style::primary_btn(ui, "Fertig", lay.env.btn).clicked() {
                        done = true;
                    }
                });
            });
        });
    });
    ed.st.popup_rects.push(panel);
    if reset {
        st.list = settings::edit_list(&settings::DEFAULT_TOOLS.iter().map(|s| s.to_string()).collect::<Vec<_>>());
        apply_edit(sh, &st.list);
    }
    if done || ctx.input(|i| i.key_pressed(egui::Key::Escape)) {
        sh.reqs.push(Req::ToolsEdited);
    } else {
        ed.st.edit_tools = Some(st);
    }
}
