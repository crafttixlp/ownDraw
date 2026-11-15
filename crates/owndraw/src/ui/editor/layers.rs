//! Ebenen-Panel: Compact = Vollbild-Sheet, Medium = schwebendes Panel, Expanded = angedocktes Panel rechts.

use super::{Editor, Lay, DOCK_LAYERS, M};
use crate::app::Shared;
use crate::layout::FormFactor;
use crate::settings::BarPos;
use crate::style::{self, c, icon_btn};
use crate::ui::dialogs::{AnimDlg, ConfirmAction, Dialog, RenameWhat};
use egui::{Align2, Color32, CornerRadius, Order, Pos2, Sense, Ui, Vec2};
use owndraw_core::icons as ic;
use owndraw_core::layer::ANIM_NAMES;

enum Act {
    Active(usize),
    Toggle(usize, bool),
    Rename(usize),
    Anim(usize),
    Export(usize),
    Move(usize, usize),
    Delete(usize),
    Add,
    Template,
    Image,
    Close,
}

pub(super) fn show(ed: &mut Editor, ctx: &egui::Context, sh: &mut Shared, lay: &Lay) {
    if !ed.st.layers_open {
        return;
    }
    let canvas = lay.canvas;
    let mut act: Option<Act> = None;
    match lay.env.ff {
        FormFactor::Expanded => {
            let dock = lay.dock_layers.unwrap_or(canvas);
            let r = Editor::float(ctx, "layers-dock", dock.min, Align2::LEFT_TOP, dock, Order::Middle, |ui| {
                let p = style::pal();
                egui::Frame::new().fill(c(p.card)).stroke(egui::Stroke::new(1.0, c(p.line))).inner_margin(egui::Margin::same(12)).show(ui, |ui| {
                    ui.set_width(DOCK_LAYERS - 26.0);
                    ui.set_min_height(dock.height() - 26.0);
                    header(ui, sh, &mut act, true);
                    content(ui, ed, sh, &mut act, dock.height() - 150.0);
                });
            });
            ed.st.popup_rects.push(r);
        }
        FormFactor::Medium => {
            let right = canvas.right() - M - if lay.tb_edge == Some(BarPos::Right) { lay.thick + M } else { 0.0 };
            let max_h = (canvas.bottom() - lay.y_below - M - if lay.tb_edge == Some(BarPos::Bottom) { lay.thick + M } else { 0.0 } - 110.0).max(120.0);
            let r = Editor::float(ctx, "layers-float", Pos2::new(right, lay.y_below), Align2::RIGHT_TOP, canvas, Order::Middle, |ui| {
                style::pill(22).show(ui, |ui| {
                    egui::Frame::new().inner_margin(egui::Margin::same(8)).show(ui, |ui| {
                        ui.set_width(280.0);
                        header(ui, sh, &mut act, true);
                        content(ui, ed, sh, &mut act, max_h);
                    });
                });
            });
            ed.st.popup_rects.push(r);
        }
        FormFactor::Compact => {
            let r = Editor::float(ctx, "layers-full", canvas.min, Align2::LEFT_TOP, canvas, Order::Foreground, |ui| {
                let p = style::pal();
                egui::Frame::new().fill(c(p.bg)).inner_margin(egui::Margin::same(12)).show(ui, |ui| {
                    ui.set_width(canvas.width() - 24.0);
                    ui.set_min_height(canvas.height() - 24.0);
                    header(ui, sh, &mut act, true);
                    content(ui, ed, sh, &mut act, canvas.height() - 150.0);
                });
            });
            ed.st.popup_rects.push(r);
        }
    }
    apply(ed, sh, act);
}

fn header(ui: &mut Ui, sh: &Shared, act: &mut Option<Act>, closable: bool) {
    ui.horizontal(|ui| {
        ui.label(egui::RichText::new("Ebenen").size(16.0).strong());
        if closable {
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if style::plain_btn(ui, "Schließen", sh.env.btn - 8.0).clicked() {
                    *act = Some(Act::Close);
                }
            });
        }
    });
    ui.add_space(2.0);
}

fn row_menu(ui: &mut Ui, i: usize, n: usize, h: f32, out: &mut Option<Act>) {
    ui.set_min_width(190.0);
    let mut item = |ui: &mut Ui, label: &str, danger: bool, a: Act| {
        if style::menu_row(ui, label, danger, h) {
            *out = Some(a);
            ui.close();
        }
    };
    item(ui, "Umbenennen", false, Act::Rename(i));
    item(ui, "Animation & Deckkraft …", false, Act::Anim(i));
    item(ui, "Als PNG exportieren …", false, Act::Export(i));
    if i + 1 < n {
        item(ui, "Nach vorne", false, Act::Move(i, i + 1));
    }
    if i > 0 {
        item(ui, "Nach hinten", false, Act::Move(i, i - 1));
    }
    item(ui, "Löschen", true, Act::Delete(i));
}

fn content(ui: &mut Ui, ed: &mut Editor, sh: &mut Shared, act: &mut Option<Act>, max_h: f32) {
    let p = style::pal();
    let btn = sh.env.btn;
    let n = ed.board.layers.len();
    egui::ScrollArea::vertical().max_height(max_h.max(100.0)).auto_shrink([false, true]).show(ui, |ui| {
        for i in (0..n).rev() {
            let b = &ed.board;
            let l = &b.layers[i];
            let active = i == b.active;
            egui::Frame::new().fill(if active { c(p.active) } else { Color32::TRANSPARENT }).corner_radius(CornerRadius::same(24)).inner_margin(egui::Margin::symmetric(2, 0)).show(ui, |ui| {
                ui.horizontal(|ui| {
                    ui.spacing_mut().item_spacing.x = 0.0;
                    let (eye, et) = if l.visible { (ic::EYE, p.text) } else { (ic::EYE_OFF, p.dim) };
                    if icon_btn(ui, &mut sh.icons, eye, et, false, false, btn, if l.visible { "Ebene ausblenden" } else { "Ebene einblenden" }).clicked() {
                        *act = Some(Act::Toggle(i, true));
                    }
                    let (lk, lt) = if l.locked { (ic::LOCK, p.accent) } else { (ic::UNLOCK, p.dim) };
                    if icon_btn(ui, &mut sh.icons, lk, lt, false, false, btn, if l.locked { "Ebene entsperren" } else { "Ebene sperren" }).clicked() {
                        *act = Some(Act::Toggle(i, false));
                    }
                    let name_w = ui.available_width() - btn - 2.0;
                    let (r, resp) = ui.allocate_exact_size(Vec2::new(name_w.max(40.0), btn), Sense::click());
                    let pt = ui.painter_at(r);
                    let has_anim = l.anim > 0 && (l.anim as usize) < ANIM_NAMES.len();
                    let ny = if has_anim { r.center().y - 8.0 } else { r.center().y };
                    pt.text(egui::pos2(r.left() + 6.0, ny), Align2::LEFT_CENTER, &l.name, egui::FontId::proportional(14.5), c(if l.visible { p.text } else { p.dim }));
                    if has_anim {
                        pt.text(egui::pos2(r.left() + 6.0, r.center().y + 9.0), Align2::LEFT_CENTER, ANIM_NAMES[l.anim as usize], egui::FontId::proportional(11.0), c(p.dim));
                    }
                    if resp.double_clicked() {
                        *act = Some(Act::Rename(i));
                    } else if resp.clicked() {
                        *act = Some(Act::Active(i));
                    }
                    let h = (btn - 6.0).max(32.0);
                    resp.context_menu(|ui| row_menu(ui, i, n, h, act));
                    let more = icon_btn(ui, &mut sh.icons, ic::MORE, p.text, false, false, btn, "Optionen");
                    egui::Popup::menu(&more).align(egui::RectAlign::BOTTOM_END).show(|ui| row_menu(ui, i, n, h, act));
                });
            });
        }
    });
    ui.add_space(6.0);
    ui.horizontal_wrapped(|ui| {
        if style::accent_btn(ui, "+ Ebene", btn - 6.0).clicked() {
            *act = Some(Act::Add);
        }
        if style::accent_btn(ui, "Vorlage", btn - 6.0).clicked() {
            *act = Some(Act::Template);
        }
        if style::accent_btn(ui, "Bild", btn - 6.0).clicked() {
            *act = Some(Act::Image);
        }
    });
}

fn apply(ed: &mut Editor, sh: &mut Shared, act: Option<Act>) {
    let Some(act) = act else { return };
    let b = &mut ed.board;
    match act {
        Act::Close => ed.st.layers_open = false,
        Act::Add => {
            let name = format!("{:02}-ebene", b.layers.len() + 1);
            b.add_layer(Some(&name));
        }
        Act::Template => {
            b.apply_template();
            sh.toast_for("Vorlage: 7 Ebenen (Hintergrund bis Text) mit passenden Animationen angelegt", 3.5);
        }
        Act::Image => ed.pick_image(sh),
        Act::Active(i) => b.set_active_layer(i),
        Act::Toggle(i, vis) => b.edit_layer(i, |l| {
            if vis {
                l.visible = !l.visible
            } else {
                l.locked = !l.locked
            }
        }),
        Act::Rename(i) => {
            sh.dialog = Some(Dialog::Rename { what: RenameWhat::Layer(i), title: "Ebene umbenennen", text: b.layers[i].name.clone(), focus: true });
        }
        Act::Anim(i) => {
            let l = &b.layers[i];
            sh.dialog = Some(Dialog::Anim(AnimDlg { idx: i, anim: l.anim, speed: l.speed, hover: l.hover, opacity: l.opacity }));
        }
        Act::Export(i) => ed.export_layer(sh, i),
        Act::Move(from, to) => b.move_layer(from, to),
        Act::Delete(i) => {
            if b.layers.len() <= 1 {
                sh.toast("Mindestens eine Ebene bleibt bestehen");
            } else {
                sh.dialog = Some(Dialog::Confirm {
                    title: "Ebene löschen",
                    text: format!("Ebene „{}“ mit allem Inhalt löschen?", b.layers[i].name),
                    ok: "Löschen",
                    action: ConfirmAction::DeleteLayer(i),
                });
            }
        }
    }
}
