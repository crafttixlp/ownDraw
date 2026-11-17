//! Leisten des Editors: oben (Zurück/Name, Rückgängig/Wiederholen, ⋯), Werkzeugleiste, Auswahl-Aktionen.

use super::{clip, keys, tools, Editor, Lay, Tool, M, PAD};
use crate::app::{Req, Shared};
use crate::layout::FormFactor;
use crate::settings::{self, BarPos, Dock};
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
    let right_w = 4.0 * btn + if compact { 0.0 } else { 64.0 } + 2.0 * PAD;
    let name_max = (canvas.width() - 2.0 * M - btn - 2.0 * PAD - right_w - 3.0 * M - 14.0).clamp(40.0, 260.0);

    // links: Zurück + Titel
    let r = tile(ed, ctx, sh, lay, "bar-tl", Pos2::new(canvas.left() + M, canvas.top() + M), Align2::LEFT_TOP, Some(BarPos::Top), false, |ed, sh, ui, tl| {
        let vert = tl.vert;
        tl.frame.show(ui, |ui| {
            orient(ui, vert, btn, |ui| {
                ui.spacing_mut().item_spacing = Vec2::splat(2.0);
                let g = grip(ui, btn, !vert);
                if icon_btn(ui, &mut sh.icons, ic::BACK, p.text, false, false, btn, "Zurück zum Start").clicked() {
                    sh.reqs.push(Req::Back);
                }
                // Speicherstatus als Punkt
                let (label, err) = ed.save_state();
                if !label.is_empty() {
                    let sz = if vert { Vec2::new(btn, 10.0) } else { Vec2::new(10.0, btn) };
                    let (rect, resp) = ui.allocate_exact_size(sz, Sense::hover());
                    let col = if err { p.danger } else if label.starts_with("Änderungen") { 0xFFFFC53D } else { 0xFF30A46C };
                    ui.painter().circle_filled(rect.center(), 3.5, c(col));
                    resp.on_hover_text(label);
                }
                // Senkrecht: kein Name (Umbenennen über das Menü ⋯)
                if !vert {
                    ui.scope(|ui| {
                        ui.set_max_width(name_max);
                        let t = egui::RichText::new(ed.name()).size(15.0).strong().color(c(p.text));
                        let r = ui.add(egui::Button::new(t).fill(Color32::TRANSPARENT).truncate().min_size(Vec2::new(0.0, btn - 6.0))).on_hover_text("Board umbenennen");
                        if r.clicked() {
                            sh.dialog = Some(crate::ui::dialogs::Dialog::Rename { what: crate::ui::dialogs::RenameWhat::Doc, title: "Board umbenennen", text: ed.name().to_string(), focus: true });
                        }
                    });
                    ui.add_space(6.0);
                }
                g
            })
        })
        .inner
    });
    ed.st.popup_rects.push(r);

    // rechts: Rückgängig, Wiederholen, Zoom, ⋯, Exportieren
    let r = tile(ed, ctx, sh, lay, "bar-tr", Pos2::new(canvas.right() - M, canvas.top() + M), Align2::RIGHT_TOP, Some(BarPos::Top), false, |ed, sh, ui, tl| {
        let vert = tl.vert;
        tl.frame.show(ui, |ui| {
            orient(ui, vert, btn, |ui| {
                ui.spacing_mut().item_spacing = Vec2::ZERO;
                let g = grip(ui, btn, !vert);
                let b = &mut ed.board;
                let u = if b.can_undo() { p.text } else { p.disabled };
                if icon_btn(ui, &mut sh.icons, ic::UNDO, u, false, false, btn, &env.tip("Rückgängig", &keys::label(&sh.settings, "undo"))).clicked() {
                    b.undo();
                }
                let rd = if b.can_redo() { p.text } else { p.disabled };
                if icon_btn(ui, &mut sh.icons, ic::REDO, rd, false, false, btn, &env.tip("Wiederholen", &keys::label(&sh.settings, "redo"))).clicked() {
                    b.redo();
                }
                if !compact {
                    let t = if vert { format!("{}%", (ed.board.sc * 100.0).round() as i32) } else { format!("{} %", (ed.board.sc * 100.0).round() as i32) };
                    let r = if vert {
                        ui.add(egui::Button::new(egui::RichText::new(t).size(11.0)).fill(Color32::TRANSPARENT).wrap_mode(egui::TextWrapMode::Extend).min_size(Vec2::new(btn, 28.0)))
                    } else {
                        ui.add(egui::Button::new(egui::RichText::new(t).size(13.0)).fill(Color32::TRANSPARENT).min_size(Vec2::new(60.0, btn - 6.0)))
                    };
                    if r.on_hover_text(env.tip("Alles zeigen", &keys::label(&sh.settings, "fit"))).clicked() {
                        ed.board.fit();
                    }
                }
                let more = icon_btn(ui, &mut sh.icons, ic::MORE, p.text, ed.st.menu_open, false, btn, "Menü");
                ed.st.menu_btn = Some(more.rect);
                if more.clicked() {
                    ed.st.menu_open = !ed.st.menu_open;
                    ed.st.opts_open = false;
                    ed.st.more_open = false;
                    ed.st.export.open = false;
                }
                let exp = icon_btn(ui, &mut sh.icons, ic::EXPORT, p.text, ed.st.export.open, false, btn, &env.tip("Exportieren", &keys::label(&sh.settings, "export")));
                ed.st.export.btn = Some(exp.rect);
                if exp.clicked() {
                    ed.st.export.open = !ed.st.export.open;
                    ed.st.menu_open = false;
                    ed.st.opts_open = false;
                    ed.st.more_open = false;
                }
                g
            })
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
pub(super) struct Grip(egui::Response);

/// Greif-Griff am Anfang einer Kachel (6 Punkte). `horiz`: Leiste läuft waagerecht (Griff links, sonst oben).
pub(super) fn grip(ui: &mut Ui, btn: f32, horiz: bool) -> Grip {
    let size = if horiz { Vec2::new(14.0, btn) } else { Vec2::new(btn, 14.0) };
    let (rect, resp) = ui.allocate_exact_size(size, Sense::click_and_drag());
    let resp = resp.on_hover_text("Ziehen zum Verschieben, Doppelklick zum Zurücksetzen");
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

/// Waagerechte oder senkrechte Anordnung des Inhalts einer Kachel.
/// Senkrecht mit fester Breite `w` (sonst würde die Spalte die ganze verfügbare Breite füllen),
/// Inhalt mittig.
pub(super) fn orient<R>(ui: &mut Ui, vert: bool, w: f32, f: impl FnOnce(&mut Ui) -> R) -> R {
    if vert {
        ui.allocate_ui_with_layout(Vec2::new(w, 0.0), egui::Layout::top_down(egui::Align::Center), |ui| {
            ui.set_width(w);
            f(ui)
        })
        .inner
    } else {
        ui.horizontal(f).inner
    }
}

/// Abstand (Punkte) des Zeigers zum Rand der Zeichenfläche, ab dem eine gezogene Kachel andockt.
pub(super) const SNAP: f32 = 48.0;

/// Rand, an dem eine beim Zeiger `p` losgelassene Kachel andockt (nächster Rand innerhalb `SNAP`).
pub(super) fn snap_edge(canvas: Rect, p: Pos2) -> Option<BarPos> {
    [(BarPos::Left, p.x - canvas.left()), (BarPos::Right, canvas.right() - p.x), (BarPos::Top, p.y - canvas.top()), (BarPos::Bottom, canvas.bottom() - p.y)]
        .into_iter()
        .filter(|(_, d)| *d < SNAP)
        .min_by(|a, b| a.1.total_cmp(&b.1))
        .map(|(e, _)| e)
}

/// Linke obere Ecke einer am Rand `e` angedockten Kachel (bündig am Rand); längs des Randes per Anteil `f`.
pub(super) fn dock_pos(canvas: Rect, size: Vec2, e: BarPos, f: [f32; 2]) -> Pos2 {
    let area = canvas;
    let p = frac_to_pos(area, size, f);
    match e {
        BarPos::Left => Pos2::new(area.left(), p.y),
        BarPos::Right => Pos2::new(area.right() - size.x, p.y),
        BarPos::Top => Pos2::new(p.x, area.top()),
        BarPos::Bottom => Pos2::new(p.x, area.bottom() - size.y),
    }
}

/// Schiebt `r` vom Rand `e` weg, bis es keine schon platzierte Kachel am selben Rand überlappt.
pub(super) fn stack(mut r: Rect, e: BarPos, placed: &[(BarPos, Rect)]) -> Rect {
    for _ in 0..placed.len() {
        let Some(o) = placed.iter().filter(|(e2, _)| *e2 == e).map(|(_, o)| *o).find(|o| o.shrink(1.0).intersects(r.shrink(1.0))) else { break };
        let d = match e {
            BarPos::Top => Vec2::new(0.0, o.bottom() + M - r.top()),
            BarPos::Bottom => Vec2::new(0.0, o.top() - M - r.bottom()),
            BarPos::Left => Vec2::new(o.right() + M - r.left(), 0.0),
            BarPos::Right => Vec2::new(o.left() - M - r.right(), 0.0),
        };
        r = r.translate(d);
    }
    r
}

/// Was der Inhalt einer Kachel braucht: Ausrichtung und Rahmen (bündig angedockt: Randseite flach).
pub(super) struct Tl {
    pub vert: bool,
    pub frame: egui::Frame,
}

/// Pillen-Rahmen einer Leiste; bündig an `flush` angedockt ohne Rundung auf dieser Seite.
pub(super) fn bar_frame(flush: Option<BarPos>) -> egui::Frame {
    let mut f = style::pill(26);
    let r = &mut f.corner_radius;
    match flush {
        Some(BarPos::Top) => (r.nw, r.ne) = (0, 0),
        Some(BarPos::Bottom) => (r.sw, r.se) = (0, 0),
        Some(BarPos::Left) => (r.nw, r.sw) = (0, 0),
        Some(BarPos::Right) => (r.ne, r.se) = (0, 0),
        None => {}
    }
    f
}

/// Verschiebbare, andockbare Kachel. Position: angedockt (Rand + Anteil längs), frei (Anteil) oder
/// Vorgabe `pos`/`pivot` am Rand `def_edge`. Ziehen am Griff verschiebt; Loslassen nahe einem Rand
/// dockt dort an (links/rechts senkrecht, oben/unten waagerecht), sonst bleibt sie frei liegen.
/// Doppelklick auf den Griff setzt zurück. `content` bekommt `vert` (senkrechte Anordnung).
#[allow(clippy::too_many_arguments)]
pub(super) fn tile(
    ed: &mut Editor,
    ctx: &egui::Context,
    sh: &mut Shared,
    lay: &Lay,
    id: &str,
    pos: Pos2,
    pivot: Align2,
    def_edge: Option<BarPos>,
    def_vert: bool,
    content: impl FnOnce(&mut Editor, &mut Shared, &mut Ui, Tl) -> Grip,
) -> Rect {
    let canvas = lay.canvas;
    let key = tile_key(id, lay.env.ff);
    let size = ed.st.tile_sizes.get(id).copied().unwrap_or(Vec2::ZERO);
    let dock = sh.settings.tile_dock.get(&key).copied();
    let vert = dock.map(|d| d.vert).unwrap_or(def_vert);
    let dragging = ed.st.tile_drag.as_ref().is_some_and(|(k, _)| *k == key);
    let saved = sh.settings.tile_pos.get(&key).copied();
    let (mut pos, mut pivot, mut edge) = (pos, pivot, None);
    if let (true, Some((_, p))) = (dragging, &ed.st.tile_drag) {
        (pos, pivot) = (*p, Align2::LEFT_TOP);
    } else if let Some(Dock { edge: Some(e), .. }) = dock {
        (pos, pivot, edge) = (dock_pos(canvas, size, e, saved.unwrap_or([0.5, 0.5])), Align2::LEFT_TOP, Some(e));
    } else if let Some(f) = saved {
        (pos, pivot) = (frac_to_pos(canvas, size, f), Align2::LEFT_TOP);
    } else {
        edge = def_edge;
    }
    // Angedockte Kacheln am selben Rand stapeln statt überlappen
    let mut flush = None;
    if let (Some(e), true) = (edge, size.x > 0.0) {
        let r = stack(pivot.anchor_size(pos, size), e, &ed.st.dock_rects);
        (pos, pivot) = (r.min, Align2::LEFT_TOP);
        // vom Nutzer angedockt und direkt am Rand: Randseite flach
        let touches = match e {
            BarPos::Top => r.top() <= canvas.top() + 0.5,
            BarPos::Bottom => r.bottom() >= canvas.bottom() - 0.5,
            BarPos::Left => r.left() <= canvas.left() + 0.5,
            BarPos::Right => r.right() >= canvas.right() - 0.5,
        };
        if dock.is_some_and(|d| d.edge.is_some()) && touches {
            flush = Some(e);
        }
    }
    let mut grip_resp = None;
    // eigene Area je Ausrichtung, damit keine Größe der anderen Ausrichtung hängen bleibt
    let area_id = if vert { format!("{id}-v") } else { id.to_string() };
    let r = Editor::float(ctx, &area_id, pos, pivot, canvas, Order::Middle, |ui| {
        grip_resp = Some(content(ed, sh, ui, Tl { vert, frame: bar_frame(flush) }).0);
    });
    ed.st.tile_sizes.insert(id.to_string(), r.size());
    if let Some(e) = edge {
        ed.st.dock_rects.push((e, r));
    }
    if let Some(g) = grip_resp {
        let ptr = ctx.input(|i| i.pointer.interact_pos().or(i.pointer.latest_pos()));
        if g.double_clicked() {
            ed.st.tile_drag = None;
            sh.settings.tile_pos.remove(&key);
            sh.settings.tile_dock.remove(&key);
        } else if g.dragged() {
            let cur = if dragging { ed.st.tile_drag.as_ref().map(|d| d.1).unwrap_or(r.min) } else { r.min };
            let np = cur + g.drag_delta();
            // in der Zeichenfläche halten
            let np = frac_to_pos(canvas, r.size(), pos_to_frac(canvas, r.size(), np));
            ed.st.tile_drag = Some((key, np));
            if ptr.is_some() {
                ed.st.tile_ptr = ptr;
            }
            // Andock-Zone anzeigen
            if let Some(e) = ed.st.tile_ptr.and_then(|p| snap_edge(canvas, p)) {
                let t = 6.0;
                let zone = match e {
                    BarPos::Left => Rect::from_min_max(canvas.min, Pos2::new(canvas.left() + t, canvas.bottom())),
                    BarPos::Right => Rect::from_min_max(Pos2::new(canvas.right() - t, canvas.top()), canvas.max),
                    BarPos::Top => Rect::from_min_max(canvas.min, Pos2::new(canvas.right(), canvas.top() + t)),
                    BarPos::Bottom => Rect::from_min_max(Pos2::new(canvas.left(), canvas.bottom() - t), canvas.max),
                };
                let painter = ctx.layer_painter(egui::LayerId::new(Order::Tooltip, egui::Id::unique("dock-zone")));
                painter.rect_filled(zone, 3.0, c(style::pal().accent).gamma_multiply(0.7));
            }
        } else if g.drag_stopped() && dragging {
            if let Some((_, p)) = ed.st.tile_drag.take() {
                let at = ptr.or(ed.st.tile_ptr.take());
                ed.st.tile_ptr = None;
                match at.and_then(|q| snap_edge(canvas, q)) {
                    Some(e) => {
                        let v = matches!(e, BarPos::Left | BarPos::Right);
                        // Größe nach dem Drehen schätzen (Länge und Dicke tauschen)
                        let sz = if v != vert { Vec2::new(r.height(), r.width()) } else { r.size() };
                        let sz = sz.min(canvas.size());
                        let mut f = pos_to_frac(canvas, sz, p);
                        if v {
                            f[0] = 0.0;
                        } else {
                            f[1] = 0.0;
                        }
                        sh.settings.tile_pos.insert(key.clone(), f);
                        sh.settings.tile_dock.insert(key, Dock { edge: Some(e), vert: v });
                    }
                    None => {
                        sh.settings.tile_pos.insert(key.clone(), pos_to_frac(canvas, r.size(), p));
                        sh.settings.tile_dock.insert(key, Dock { edge: None, vert });
                    }
                }
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
            _ => return None, // Maschinenbau-Pfeil und neue Formen: gezeichnetes Icon
        },
        Tool::Text => {
            if b.tool == board::NOTE {
                ic::NOTE
            } else {
                ic::TEXT
            }
        }
        Tool::Note => ic::NOTE,
        Tool::Image => ic::IMAGE,
        Tool::Ruler => ic::RULER,
        Tool::Laser => ic::LASER,
        Tool::Space => ic::SPACE,
        Tool::Fill | Tool::Picker | Tool::Dim | Tool::Stamp | Tool::Calc | Tool::Convert => return None, // gezeichnetes Icon (glyphs.rs)
        Tool::Connector => ic::CONNECTOR,
        Tool::Table => ic::TABLE,
        Tool::Pin => ic::PIN,
        Tool::Math => ic::FORMULA,
        Tool::PenMode => ic::FINGER,
    })
}

fn tool_btn(ui: &mut Ui, ed: &mut Editor, sh: &mut Shared, t: Tool, size: f32) {
    let p = style::pal();
    let active = match t {
        Tool::Image => false,
        Tool::PenMode => ed.pen_mode(),
        _ => ed.active_tool() == t || (t == Tool::Ruler && ed.ext.ruler.visible),
    };
    let (rect, resp) = ui.allocate_exact_size(Vec2::splat(size), Sense::click());
    style::btn_bg(ui, rect, &resp, active);
    let tint = if active { p.accent } else { p.text };
    let isz = (size * 0.55).round();
    match tool_icon(ed, t) {
        Some(id) => sh.icons.paint(ui, rect, id, tint, false, isz),
        None => super::glyphs::paint_tool(ui.painter(), rect, ed, t, isz, c(tint)),
    }
    if t == Tool::PenMode && active {
        // Finger durchgestrichen: Finger zeichnen nicht
        let h = isz * 0.5;
        ui.painter().line_segment([rect.center() + Vec2::new(-h, -h), rect.center() + Vec2::new(h, h)], Stroke::new(2.0, c(tint)));
    }
    // Farbpunkt als Hinweis auf die aktuelle Farbe – nur am aktiven Werkzeug (sonst unruhig)
    if active && matches!(t, Tool::Pen | Tool::Marker | Tool::Shape | Tool::Text | Tool::Note | Tool::Fill | Tool::Stamp | Tool::Dim | Tool::Connector | Tool::Pin | Tool::Math | Tool::Calc) {
        let pos = rect.right_bottom() - Vec2::new(size * 0.22, size * 0.22);
        ui.painter().circle_filled(pos, 4.5, c(ed.board.color));
        ui.painter().circle_stroke(pos, 4.5, Stroke::new(1.5, c(p.pill)));
    }
    ed.st.tool_rects.push((t, rect));
    let tip = ed.tool_tip(&sh.env, t, &sh.settings);
    if resp.on_hover_text(tip).clicked() {
        if t == Tool::Image {
            ed.pick_image(sh);
        } else if t == Tool::PenMode {
            ed.select_tool(sh, t);
        } else if active {
            ed.st.opts_open = !ed.st.opts_open;
            ed.st.menu_open = false;
            ed.ext.ruler_pop = t == Tool::Ruler && ed.st.opts_open;
        } else {
            ed.select_tool(sh, t);
            ed.st.opts_open = false;
        }
    }
}

/// Knopf „+“: Weitere (ausgeschaltete) Werkzeuge. Punkt = ungesehene neue Werkzeuge.
fn more_btn(ui: &mut Ui, ed: &mut Editor, sh: &mut Shared, size: f32, vert: bool) {
    ed.st.more_btn = None;
    if super::more::hidden(&sh.settings).is_empty() {
        ed.st.more_open = false;
        return;
    }
    let p = style::pal();
    // dünne Trennlinie vor dem Knopf
    let (sep, _) = ui.allocate_exact_size(if vert { Vec2::new(size, 7.0) } else { Vec2::new(7.0, size) }, Sense::hover());
    let (a, b) = if vert {
        (Pos2::new(sep.left() + size * 0.25, sep.center().y), Pos2::new(sep.right() - size * 0.25, sep.center().y))
    } else {
        (Pos2::new(sep.center().x, sep.top() + size * 0.25), Pos2::new(sep.center().x, sep.bottom() - size * 0.25))
    };
    ui.painter().line_segment([a, b], Stroke::new(1.0, c(p.line)));
    let open = ed.st.more_open;
    let r = icon_btn(ui, &mut sh.icons, ic::PLUS, if open { p.accent } else { p.dim }, open, false, size, "Weitere Werkzeuge hinzufügen");
    if super::more::unseen(&sh.settings) {
        let pos = r.rect.right_top() + Vec2::new(-size * 0.24, size * 0.24);
        ui.painter().circle_filled(pos, 4.0, c(p.accent));
        ui.painter().circle_stroke(pos, 4.0, Stroke::new(1.5, c(p.pill)));
    }
    ed.st.more_btn = Some(r.rect);
    if r.clicked() {
        ed.st.more_open = !open;
        ed.st.opts_open = false;
        ed.st.menu_open = false;
        super::more::mark_seen(&mut sh.settings);
    }
}

impl Editor {
    fn tool_tip(&self, env: &crate::layout::Env, t: Tool, set: &settings::Settings) -> String {
        let mut s = env.tip(t.label(), &keys::label(set, &t.action()));
        if t == Tool::PenMode {
            s.push_str(if self.pen_mode() { " · an: nur der Stift zeichnet" } else { " · aus: Finger zeichnen" });
        }
        if t.has_options() && self.active_tool() == t {
            s.push_str(" · erneut tippen für Optionen");
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
    let r = tile(ed, ctx, sh, lay, "bar-tools", pos, pivot, lay.tb_edge, !horiz, |ed, sh, ui, tl| {
        let vert = tl.vert;
        ed.st.tools_vert = vert;
        tl.frame
            .show(ui, |ui| {
                orient(ui, vert, btn, |ui| {
                    ui.spacing_mut().item_spacing = Vec2::ZERO;
                    let g = grip(ui, btn, !vert);
                    for &t in tools {
                        tool_btn(ui, ed, sh, t, btn);
                    }
                    more_btn(ui, ed, sh, btn, vert);
                    g
                })
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
    let bottom = canvas.bottom() - M - if lay.tb_edge == Some(BarPos::Bottom) { lay.thick + M } else { 0.0 };
    let maxw = canvas.width() - 2.0 * M;
    let maxh = canvas.height() - 2.0 * M;
    let r = tile(ed, ctx, sh, lay, "bar-sel", Pos2::new(canvas.center().x, bottom), Align2::CENTER_BOTTOM, Some(BarPos::Bottom), false, |ed, sh, ui, tl| {
        let vert = tl.vert;
        tl.frame
            .show(ui, |ui| {
                let body = |ui: &mut Ui| {
                    ui.spacing_mut().item_spacing = Vec2::ZERO;
                    let g = grip(ui, btn, !vert);
                    let env = sh.env;
                    if has_sel {
                        if icon_btn(ui, &mut sh.icons, ic::COPY, p.text, false, false, btn, &env.tip("Kopieren", &keys::label(&sh.settings, "copy"))).clicked() {
                            clip::copy(ed, &*sh.platform);
                        }
                        if icon_btn(ui, &mut sh.icons, ic::CUT, p.text, false, false, btn, &env.tip("Ausschneiden", &keys::label(&sh.settings, "cut"))).clicked() {
                            clip::cut(ed, &*sh.platform);
                        }
                    }
                    if icon_btn(ui, &mut sh.icons, ic::PASTE, p.text, false, false, btn, &env.tip("Einfügen", &keys::label(&sh.settings, "paste"))).clicked() {
                        clip::paste_center(ed, &*sh.platform);
                    }
                    if has_sel {
                        if icon_btn(ui, &mut sh.icons, ic::DUPLICATE, p.text, false, false, btn, &env.tip("Duplizieren", &keys::label(&sh.settings, "duplicate"))).clicked() {
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
                        super::arrange::sel_buttons(ui, ed, sh, btn);
                        if icon_btn(ui, &mut sh.icons, ic::TRASH, p.danger, false, false, btn, &env.tip("Löschen", &keys::label(&sh.settings, "delete"))).clicked() {
                            ed.board.delete_sel();
                        }
                        ui.spacing_mut().slider_width = if vert { 90.0 } else { 110.0 };
                        // Strichstärke der ausgewählten Striche/Formen
                        if let Some(w0) = ed.board.sel_width() {
                            let mut w = w0;
                            ui.add_space(6.0);
                            let r = ui.add(slider(egui::Slider::new(&mut w, 0.5..=40.0).logarithmic(true).max_decimals(1), vert, "Dicke")).on_hover_text(format!("Strichstärke der Auswahl: {w0:.1}"));
                            // Ein Rückgängig-Schritt je Ziehen (bei Beginn), sonst je Änderung (Tastatur/Klick)
                            if r.drag_started() {
                                ed.board.set_sel_width(w0, true);
                            }
                            if r.changed() {
                                ed.board.set_sel_width(w, !r.dragged() && !r.drag_started());
                            }
                            ui.add_space(6.0);
                        }
                        // Text/Notiz: Schriftgröße
                        if let Some(s0) = ed.board.sel_note_size() {
                            let sc = ed.board.sc;
                            let mut v = s0 * sc;
                            ui.add_space(6.0);
                            let r = ui.add(slider(egui::Slider::new(&mut v, 6.0..=160.0).logarithmic(true).max_decimals(0), vert, "Größe")).on_hover_text(format!("Schriftgröße bei aktuellem Zoom: {:.0}", s0 * sc));
                            if r.drag_started() {
                                ed.board.set_sel_note_size(s0, true);
                            }
                            if r.changed() {
                                ed.board.set_sel_note_size(v / sc, !r.dragged() && !r.drag_started());
                            }
                            ui.add_space(4.0);
                        }
                        // Farbe der Auswahl (Striche, Formen, Texte; Bilder bleiben unverändert)
                        if let Some(cur) = ed.board.sel_color() {
                            sel_colors(ui, ed, btn, vert, cur);
                            ui.add_space(6.0);
                        }
                        // Füllfarbe (geschlossene Formen)
                        if let Some(cur) = ed.board.sel_fill() {
                            sel_fills(ui, ed, btn, vert, cur);
                            ui.add_space(6.0);
                        }
                        // Stiftart der Striche
                        let ps: Vec<u8> = ed.board.sel_items().iter().filter_map(|it| it.as_stroke().filter(|s| !s.hl).map(|s| s.ps)).collect();
                        if !ps.is_empty() {
                            sel_pen_styles(ui, ed, vert, if ps.iter().all(|x| *x == ps[0]) { Some(ps[0]) } else { None });
                            ui.add_space(6.0);
                        }
                    }
                    g
                };
                if vert {
                    // min_scrolled_height: nicht auf den Platz unter der aktuellen Lage schrumpfen (sonst bleibt
                    // die am unteren Rand ausgerichtete Leiste abgeschnitten)
                    egui::ScrollArea::vertical().max_height(maxh - 2.0 * PAD).min_scrolled_height(maxh - 2.0 * PAD).auto_shrink([true, true]).show(ui, |ui| orient(ui, true, btn.max(48.0), body)).inner
                } else {
                    ui.set_max_width(maxw - 2.0 * PAD);
                    ui.horizontal_wrapped(body).inner
                }
            })
            .inner
    });
    ed.st.popup_rects.push(r);
}

/// Regler der Auswahlleiste: senkrecht ohne Text/Wert, sonst mit Beschriftung.
fn slider<'a>(s: egui::Slider<'a>, vert: bool, text: &str) -> egui::Slider<'a> {
    if vert {
        s.vertical().show_value(false)
    } else {
        s.text(text)
    }
}

/// Farbpunkte für die Auswahl; `cur` = gemeinsame Farbe (None bei gemischter Auswahl).
/// Senkrecht in zwei Spalten.
fn sel_colors(ui: &mut Ui, ed: &mut Editor, btn: f32, vert: bool, cur: Option<u32>) {
    let p = style::pal();
    let mut dot = |ui: &mut Ui, col: u32, name: &str, size: Vec2| {
        let (rr, resp) = ui.allocate_exact_size(size, Sense::click());
        let pt = ui.painter();
        pt.circle_filled(rr.center(), 8.0, c(col));
        if col == owndraw_core::item::INK {
            pt.circle_stroke(rr.center(), 8.0, Stroke::new(1.0, c(0x80808080)));
        }
        if cur == Some(col) {
            pt.circle_stroke(rr.center(), 11.5, Stroke::new(2.0, c(p.text)));
        } else if resp.hovered() {
            pt.circle_stroke(rr.center(), 11.5, Stroke::new(1.5, c(p.line)));
        }
        if resp.on_hover_text(format!("Auswahl: {name}")).clicked() {
            ed.board.set_color(col);
        }
    };
    if vert {
        let cell = (btn / 2.0).max(24.0);
        for pair in super::COLORS.chunks(2) {
            ui.horizontal(|ui| {
                ui.spacing_mut().item_spacing = Vec2::ZERO;
                for (col, name) in pair {
                    dot(ui, *col, name, Vec2::splat(cell));
                }
            });
        }
    } else {
        for (col, name) in super::COLORS {
            dot(ui, col, name, Vec2::new(26.0, btn));
        }
    }
}

/// Füllfarbe der Auswahl; `cur` = gemeinsame Füllung (None = keine bzw. gemischt). Erster Punkt: „keine“.
fn sel_fills(ui: &mut Ui, ed: &mut Editor, btn: f32, vert: bool, cur: Option<u32>) {
    let p = style::pal();
    let mut dot = |ui: &mut Ui, col: Option<u32>, size: Vec2| {
        let (rr, resp) = ui.allocate_exact_size(size, Sense::click());
        let pt = ui.painter();
        match col {
            Some(k) => {
                pt.rect_filled(Rect::from_center_size(rr.center(), Vec2::splat(15.0)), 3.0, c(k));
            }
            None => {
                let r = Rect::from_center_size(rr.center(), Vec2::splat(15.0));
                pt.rect_stroke(r, 3.0, Stroke::new(1.2, c(p.dim)), egui::StrokeKind::Inside);
                pt.line_segment([r.left_bottom(), r.right_top()], Stroke::new(1.2, c(p.danger)));
            }
        }
        if cur == col {
            pt.rect_stroke(Rect::from_center_size(rr.center(), Vec2::splat(21.0)), 5.0, Stroke::new(2.0, c(p.text)), egui::StrokeKind::Outside);
        } else if resp.hovered() {
            pt.rect_stroke(Rect::from_center_size(rr.center(), Vec2::splat(21.0)), 5.0, Stroke::new(1.5, c(p.line)), egui::StrokeKind::Outside);
        }
        let name = col.and_then(|k| super::COLORS.iter().find(|x| x.0 == k).map(|x| x.1)).unwrap_or("keine");
        if resp.on_hover_text(format!("Füllung: {name}")).clicked() {
            ed.board.set_sel_fill(col);
        }
    };
    let all: Vec<Option<u32>> = std::iter::once(None).chain(super::COLORS.iter().map(|x| Some(x.0))).collect();
    if vert {
        let cell = (btn / 2.0).max(24.0);
        for pair in all.chunks(2) {
            ui.horizontal(|ui| {
                ui.spacing_mut().item_spacing = Vec2::ZERO;
                for col in pair {
                    dot(ui, *col, Vec2::splat(cell));
                }
            });
        }
    } else {
        for col in all {
            dot(ui, col, Vec2::new(26.0, btn));
        }
    }
}

/// Stiftart der ausgewählten Striche (`cur` = gemeinsame Art, None bei gemischter Auswahl).
fn sel_pen_styles(ui: &mut Ui, ed: &mut Editor, vert: bool, cur: Option<u8>) {
    let size = if vert { Vec2::new(40.0, 26.0) } else { Vec2::new(44.0, 30.0) };
    for (ps, name) in super::glyphs::PEN_STYLES {
        if super::glyphs::pen_style_btn(ui, ps, cur == Some(ps), size).on_hover_text(format!("Stiftart: {name}")).clicked() {
            ed.board.set_sel_pen_style(ps);
        }
    }
}

// ---------------------------------------------------------------- Bearbeiten-Modus (WP9)

/// Zustand des Bearbeiten-Modus: alle Werkzeuge (Kennung, eingeschaltet) in Reihenfolge, laufender Zieh-Vorgang.
/// Die Liste ist eine Ansicht von `Settings::tool_on`/`tool_order` (dieselbe Einstellung wie die Seite „Werkzeuge“).
pub struct EditTools {
    pub list: Vec<(String, bool)>,
    drag: Option<String>,
    /// Letzte Zeigerposition beim Ziehen (Touch liefert beim Loslassen keine mehr).
    last: Option<Pos2>,
}

impl Editor {
    /// Startet den Bearbeiten-Modus der Werkzeugleiste (aus den Einstellungen heraus).
    pub fn begin_edit_tools(&mut self, sh: &Shared) {
        self.st.opts_open = false;
        self.st.menu_open = false;
        self.st.edit_tools = Some(EditTools { list: edit_list(&sh.settings), drag: None, last: None });
    }
}

/// Alle Werkzeuge in Leisten-Reihenfolge mit an/aus.
fn edit_list(s: &settings::Settings) -> Vec<(String, bool)> {
    tools::ordered(s).iter().map(|d| (d.id.to_string(), tools::enabled(s, d.id))).collect()
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
                    let resp = resp.on_hover_text(format!("{}: Ziehen zum Verschieben, Tippen zum {}", t.label(), if *on { "Ausblenden" } else { "Einblenden" }));
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
                        None => super::glyphs::paint_tool(ui.painter(), rect, ed, t, isz, c(tint)),
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
        let (id, on) = st.list[i].clone();
        if tools::set_enabled(&mut sh.settings, &id, !on) {
            st.list = edit_list(&sh.settings);
        } else {
            sh.toast("Mindestens ein Werkzeug muss eingeschaltet bleiben");
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
                    tools::reorder(&mut sh.settings, i, target);
                    st.list = edit_list(&sh.settings);
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
                ui.label(egui::RichText::new("Ziehen ändert die Reihenfolge. Tippen auf das Auge blendet ein Werkzeug ein oder aus.").size(13.0).color(c(p.dim)));
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
        tools::reset(&mut sh.settings);
        st.list = edit_list(&sh.settings);
    }
    if done || ctx.input(|i| keys::pressed(i, &sh.settings, "cancel")) {
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
    fn andocken() {
        let canvas = Rect::from_min_size(Pos2::new(0.0, 0.0), Vec2::new(1000.0, 800.0));
        assert_eq!(snap_edge(canvas, Pos2::new(10.0, 400.0)), Some(BarPos::Left));
        assert_eq!(snap_edge(canvas, Pos2::new(990.0, 400.0)), Some(BarPos::Right));
        assert_eq!(snap_edge(canvas, Pos2::new(500.0, 5.0)), Some(BarPos::Top));
        assert_eq!(snap_edge(canvas, Pos2::new(500.0, 400.0)), None);
        let sz = Vec2::new(50.0, 300.0);
        let p = dock_pos(canvas, sz, BarPos::Right, [0.0, 0.0]);
        assert_eq!(p, Pos2::new(1000.0 - 50.0, 0.0));
        let a = Rect::from_min_size(Pos2::new(M, M), Vec2::new(200.0, 50.0));
        let b = stack(Rect::from_min_size(Pos2::new(100.0, M), Vec2::new(300.0, 50.0)), BarPos::Top, &[(BarPos::Top, a)]);
        assert_eq!(b.top(), a.bottom() + M);
        let c = stack(Rect::from_min_size(Pos2::new(100.0, M), Vec2::new(300.0, 50.0)), BarPos::Left, &[(BarPos::Top, a)]);
        assert_eq!(c.top(), M);
    }

    #[test]
    fn schluessel() {
        assert_eq!(tile_key("bar-tools", FormFactor::Expanded), "bar-tools@Expanded");
    }
}
