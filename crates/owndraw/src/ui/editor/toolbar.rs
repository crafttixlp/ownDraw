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
    let r = tile(ed, ctx, sh, lay, "bar-tl", Pos2::new(canvas.left() + M, canvas.top() + M), Align2::LEFT_TOP, |ed, sh, ui| {
        style::pill(26).show(ui, |ui| {
            ui.horizontal(|ui| {
                ui.spacing_mut().item_spacing.x = 2.0;
                let g = grip(ui, btn, true);
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
                g
            })
            .inner
        })
        .inner
    });
    ed.st.popup_rects.push(r);

    // rechts: Rückgängig, Wiederholen, Zoom, ⋯
    let r = tile(ed, ctx, sh, lay, "bar-tr", Pos2::new(canvas.right() - M, canvas.top() + M), Align2::RIGHT_TOP, |ed, sh, ui| {
        style::pill(26).show(ui, |ui| {
            ui.horizontal(|ui| {
                ui.spacing_mut().item_spacing.x = 0.0;
                let g = grip(ui, btn, true);
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
                g
            })
            .inner
        })
        .inner
    });
    ed.st.popup_rects.push(r);
}

/// Schlüssel einer Kachel in den Einstellungen: Kennung + Formfaktor.
pub(super) fn tile_key(id: &str, ff: FormFactor) -> String {
    format!("{id}@{ff:?}")
}

/// Anteil (0..1) der linken oberen Ecke bezogen auf den freien Platz (Zeichenfläche minus Kachel) -> Position.
pub(super) fn frac_to_pos(canvas: Rect, size: Vec2, f: [f32; 2]) -> Pos2 {
    let free = (canvas.size() - size).max(Vec2::ZERO);
    Pos2::new(canvas.min.x + f[0].clamp(0.0, 1.0) * free.x, canvas.min.y + f[1].clamp(0.0, 1.0) * free.y)
}

/// Position der linken oberen Ecke -> Anteil (0..1, geklammert; bei keinem freien Platz 0).
pub(super) fn pos_to_frac(canvas: Rect, size: Vec2, pos: Pos2) -> [f32; 2] {
    let free = (canvas.size() - size).max(Vec2::ZERO);
    let f = |p: f32, o: f32, fr: f32| if fr > 0.5 { ((p - o) / fr).clamp(0.0, 1.0) } else { 0.0 };
    [f(pos.x, canvas.min.x, free.x), f(pos.y, canvas.min.y, free.y)]
}

/// Antwort des Griffs einer Kachel in diesem Frame.
struct Grip(egui::Response);

/// Greif-Griff am Anfang einer Kachel (6 Punkte). `horiz`: Leiste läuft waagerecht (Griff links, sonst oben).
fn grip(ui: &mut Ui, btn: f32, horiz: bool) -> Grip {
    let size = if horiz { Vec2::new(14.0, btn) } else { Vec2::new(btn, 14.0) };
    let (rect, resp) = ui.allocate_exact_size(size, Sense::click_and_drag());
    let resp = resp.on_hover_text("Ziehen zum Verschieben, Doppelklick: zurücksetzen");
    if resp.dragged() {
        ui.ctx().set_cursor_icon(egui::CursorIcon::Grabbing);
    } else if resp.hovered() {
        ui.ctx().set_cursor_icon(egui::CursorIcon::Grab);
    }
    let p = style::pal();
    let col = if resp.hovered() || resp.dragged() { c(p.text) } else { c(p.disabled) };
    let (cols, rows) = if horiz { (2, 3) } else { (3, 2) };
    let step = 5.0;
    for i in 0..cols {
        for j in 0..rows {
            let o = Vec2::new((i as f32 - (cols - 1) as f32 / 2.0) * step, (j as f32 - (rows - 1) as f32 / 2.0) * step);
            ui.painter().circle_filled(rect.center() + o, 1.3, col);
        }
    }
    Grip(resp)
}

/// Verschiebbare Kachel: Position aus den Einstellungen (falls vorhanden) sonst `pos`/`pivot`;
/// Ziehen am Griff verschiebt, Loslassen speichert, Doppelklick setzt zurück.
#[allow(clippy::too_many_arguments)]
fn tile(ed: &mut Editor, ctx: &egui::Context, sh: &mut Shared, lay: &Lay, id: &str, pos: Pos2, pivot: Align2, content: impl FnOnce(&mut Editor, &mut Shared, &mut Ui) -> Grip) -> Rect {
    let canvas = lay.canvas;
    let key = tile_key(id, lay.env.ff);
    let size = ed.st.tile_sizes.get(id).copied().unwrap_or(Vec2::ZERO);
    let dragging = ed.st.tile_drag.as_ref().is_some_and(|(k, _)| *k == key);
    let (pos, pivot) = if let (true, Some((_, p))) = (dragging, &ed.st.tile_drag) {
        (*p, Align2::LEFT_TOP)
    } else if let Some(f) = sh.settings.tile_pos.get(&key) {
        (frac_to_pos(canvas, size, *f), Align2::LEFT_TOP)
    } else {
        (pos, pivot)
    };
    let mut grip_resp = None;
    let r = Editor::float(ctx, id, pos, pivot, canvas, Order::Middle, |ui| {
        grip_resp = Some(content(ed, sh, ui).0);
    });
    ed.st.tile_sizes.insert(id.to_string(), r.size());
    if let Some(g) = grip_resp {
        if g.double_clicked() {
            ed.st.tile_drag = None;
            sh.settings.tile_pos.remove(&key);
        } else if g.dragged() {
            let cur = if dragging { ed.st.tile_drag.as_ref().map(|d| d.1).unwrap_or(r.min) } else { r.min };
            let np = cur + g.drag_delta();
            // in der Zeichenfläche halten
            let np = frac_to_pos(canvas, r.size(), pos_to_frac(canvas, r.size(), np));
            ed.st.tile_drag = Some((key, np));
        } else if g.drag_stopped() && dragging {
            if let Some((_, p)) = ed.st.tile_drag.take() {
                sh.settings.tile_pos.insert(key, pos_to_frac(canvas, r.size(), p));
            }
        }
    }
    r
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
    let r = tile(ed, ctx, sh, lay, "bar-tools", pos, pivot, |ed, sh, ui| {
        style::pill(26)
            .show(ui, |ui| {
                let body = |ui: &mut Ui| {
                    ui.spacing_mut().item_spacing = Vec2::ZERO;
                    let g = grip(ui, btn, horiz);
                    for &t in tools {
                        tool_btn(ui, ed, sh, t, btn);
                    }
                    g
                };
                if horiz {
                    ui.horizontal(body).inner
                } else {
                    ui.vertical(body).inner
                }
            })
            .inner
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
    let r = tile(ed, ctx, sh, lay, "bar-sel", Pos2::new(canvas.center().x, bottom), Align2::CENTER_BOTTOM, |ed, sh, ui| {
        style::pill(26).show(ui, |ui| {
            ui.set_max_width(maxw - 2.0 * PAD);
            ui.horizontal_wrapped(|ui| {
                ui.spacing_mut().item_spacing = Vec2::ZERO;
                let g = grip(ui, btn, true);
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
                    // Strichstärke der ausgewählten Striche/Formen
                    if let Some(w0) = ed.board.sel_width() {
                        let mut w = w0;
                        ui.add_space(6.0);
                        ui.spacing_mut().slider_width = 110.0;
                        let r = ui
                            .add(egui::Slider::new(&mut w, 0.5..=40.0).logarithmic(true).max_decimals(1).text("Dicke"))
                            .on_hover_text("Strichstärke der Auswahl");
                        // Ein Rückgängig-Schritt je Ziehen (bei Beginn), sonst je Änderung (Tastatur/Klick)
                        if r.drag_started() {
                            ed.board.set_sel_width(w0, true);
                        }
                        if r.changed() {
                            ed.board.set_sel_width(w, !r.dragged() && !r.drag_started());
                        }
                        ui.add_space(6.0);
                    }
                    // Text/Notiz: Schriftgröße und Farbe (erweitertes Menü)
                    if let Some(s0) = ed.board.sel_note_size() {
                        let sc = ed.board.sc;
                        let mut v = s0 * sc;
                        ui.add_space(6.0);
                        ui.spacing_mut().slider_width = 110.0;
                        let r = ui.add(egui::Slider::new(&mut v, 6.0..=160.0).logarithmic(true).max_decimals(0).text("Größe")).on_hover_text("Schriftgröße (bei aktuellem Zoom)");
                        if r.drag_started() {
                            ed.board.set_sel_note_size(s0, true);
                        }
                        if r.changed() {
                            ed.board.set_sel_note_size(v / sc, !r.dragged() && !r.drag_started());
                        }
                        ui.add_space(4.0);
                        let cur_col = ed.board.single_note().map(|n| n.color).unwrap_or(ed.board.color);
                        for (col, name) in super::COLORS {
                            let (rr, resp) = ui.allocate_exact_size(Vec2::new(26.0, btn), egui::Sense::click());
                            let pt = ui.painter();
                            pt.circle_filled(rr.center(), 8.0, style::c(col));
                            if col == owndraw_core::item::INK {
                                pt.circle_stroke(rr.center(), 8.0, egui::Stroke::new(1.0, style::c(0x80808080)));
                            }
                            if cur_col == col {
                                pt.circle_stroke(rr.center(), 11.5, egui::Stroke::new(2.0, style::c(p.text)));
                            }
                            if resp.on_hover_text(name).clicked() {
                                ed.board.set_color(col);
                            }
                        }
                        ui.add_space(6.0);
                    }
                }
                g
            })
            .inner
        })
        .inner
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

#[cfg(test)]
mod tile_tests {
    use super::*;

    #[test]
    fn anteil_und_position() {
        let canvas = Rect::from_min_size(Pos2::new(10.0, 20.0), Vec2::new(400.0, 300.0));
        let size = Vec2::new(100.0, 50.0);
        assert_eq!(frac_to_pos(canvas, size, [0.0, 0.0]), Pos2::new(10.0, 20.0));
        assert_eq!(frac_to_pos(canvas, size, [1.0, 1.0]), Pos2::new(310.0, 270.0));
        assert_eq!(frac_to_pos(canvas, size, [5.0, -2.0]), Pos2::new(310.0, 20.0));
        let f = pos_to_frac(canvas, size, Pos2::new(160.0, 145.0));
        assert!((f[0] - 0.5).abs() < 1e-5 && (f[1] - 0.5).abs() < 1e-5);
        assert_eq!(pos_to_frac(canvas, size, Pos2::new(-100.0, 999.0)), [0.0, 1.0]);
        assert_eq!(pos_to_frac(canvas, Vec2::new(400.0, 50.0), Pos2::new(50.0, 20.0)), [0.0, 0.0]);
    }

    #[test]
    fn schluessel() {
        assert_eq!(tile_key("bar-tools", FormFactor::Expanded), "bar-tools@Expanded");
    }
}
