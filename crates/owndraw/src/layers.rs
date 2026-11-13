//! Ebenen-Panel (rechts): Sichtbarkeit, Sperre, Reihenfolge, Animation, Export, Vorlage.

use crate::app::{AnimDlg, ConfirmAction, Dialog, FilePurpose, RenameWhat, Shared};
use crate::editor::Editor;
use crate::style::{self, c, icon_btn};
use egui::{Align2, Color32, CornerRadius, Sense, Vec2};
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
}

fn menu(ui: &mut egui::Ui, i: usize, n: usize, out: &mut Option<Act>) {
    if style::menu_item(ui, "Umbenennen", false) {
        *out = Some(Act::Rename(i));
    }
    if style::menu_item(ui, "Animation & Deckkraft…", false) {
        *out = Some(Act::Anim(i));
    }
    if style::menu_item(ui, "Als PNG exportieren", false) {
        *out = Some(Act::Export(i));
    }
    if i + 1 < n && style::menu_item(ui, "Nach vorne", false) {
        *out = Some(Act::Move(i, i + 1));
    }
    if i > 0 && style::menu_item(ui, "Nach hinten", false) {
        *out = Some(Act::Move(i, i - 1));
    }
    if style::menu_item(ui, "Löschen", true) {
        *out = Some(Act::Delete(i));
    }
}

pub fn panel(ctx: &egui::Context, e: &mut Editor, sh: &mut Shared) {
    let mut act: Option<Act> = None;
    let mut add = false;
    let mut tpl = false;
    let mut img = false;
    let avail = (ctx.content_rect().height() - 246.0 - 124.0).max(80.0);
    egui::Area::new(egui::Id::unique("layers")).anchor(Align2::RIGHT_TOP, Vec2::new(-12.0, 166.0)).show(ctx, |ui| {
        style::pill(22).show(ui, |ui| {
            ui.set_width(268.0);
            ui.add_space(4.0);
            ui.horizontal(|ui| {
                ui.add_space(10.0);
                ui.label(egui::RichText::new("Ebenen").size(13.0).strong().color(c(style::DIM)));
            });
            let b = &e.board;
            let n = b.layers.len();
            egui::ScrollArea::vertical().max_height(avail).auto_shrink([false, true]).show(ui, |ui| {
                for i in (0..n).rev() {
                    let l = &b.layers[i];
                    let active = i == b.active;
                    let row = egui::Frame::new()
                        .fill(if active { c(style::ACTIVE) } else { Color32::TRANSPARENT })
                        .corner_radius(CornerRadius::same(22))
                        .inner_margin(egui::Margin::symmetric(2, 0))
                        .show(ui, |ui| {
                            ui.horizontal(|ui| {
                                ui.spacing_mut().item_spacing.x = 0.0;
                                let (eye, et) = if l.visible { (ic::EYE, style::TEXT) } else { (ic::EYE_OFF, style::DIM) };
                                if icon_btn(ui, &mut sh.icons, eye, et, false, false, if l.visible { "Ebene ausblenden" } else { "Ebene einblenden" }).clicked() {
                                    act = Some(Act::Toggle(i, true));
                                }
                                let (lk, lt) = if l.locked { (ic::LOCK, style::ACCENT) } else { (ic::UNLOCK, style::DIM) };
                                if icon_btn(ui, &mut sh.icons, lk, lt, false, false, if l.locked { "Ebene entsperren" } else { "Ebene sperren" }).clicked() {
                                    act = Some(Act::Toggle(i, false));
                                }
                                let name_w = ui.available_width() - 42.0;
                                let (r, resp) = ui.allocate_exact_size(Vec2::new(name_w.max(40.0), 40.0), Sense::click());
                                let p = ui.painter_at(r);
                                let has_anim = l.anim > 0 && (l.anim as usize) < ANIM_NAMES.len();
                                let ny = if has_anim { r.center().y - 8.0 } else { r.center().y };
                                p.text(egui::pos2(r.left() + 6.0, ny), Align2::LEFT_CENTER, &l.name, egui::FontId::proportional(14.5), c(if l.visible { style::TEXT } else { style::DIM }));
                                if has_anim {
                                    p.text(egui::pos2(r.left() + 6.0, r.center().y + 9.0), Align2::LEFT_CENTER, ANIM_NAMES[l.anim as usize], egui::FontId::proportional(11.0), c(style::DIM));
                                }
                                if resp.double_clicked() {
                                    act = Some(Act::Rename(i));
                                } else if resp.clicked() {
                                    act = Some(Act::Active(i));
                                }
                                resp.context_menu(|ui| menu(ui, i, n, &mut act));
                                let more = icon_btn(ui, &mut sh.icons, ic::MORE, style::TEXT, false, false, "Optionen");
                                egui::Popup::menu(&more).align(egui::RectAlign::BOTTOM_END).show(|ui| menu(ui, i, n, &mut act));
                            });
                        });
                    let _ = row;
                }
            });
            ui.add_space(4.0);
            ui.horizontal(|ui| {
                ui.add_space(4.0);
                add = style::accent_btn(ui, "+ Ebene").clicked();
                tpl = style::accent_btn(ui, "Vorlage").clicked();
                img = style::accent_btn(ui, "Bild").clicked();
            });
            ui.add_space(4.0);
        });
    });

    let b = &mut e.board;
    if add {
        let name = format!("{:02}-ebene", b.layers.len() + 1);
        b.add_layer(Some(&name));
    }
    if tpl {
        b.apply_template();
        sh.toast_for("Vorlage: 7 Ebenen (Hintergrund bis Text) mit passenden Animationen angelegt", 3.5);
    }
    if img {
        e.pick_image(sh);
    }
    let b = &mut e.board;
    match act {
        Some(Act::Active(i)) => b.set_active_layer(i),
        Some(Act::Toggle(i, vis)) => b.edit_layer(i, |l| {
            if vis {
                l.visible = !l.visible
            } else {
                l.locked = !l.locked
            }
        }),
        Some(Act::Rename(i)) => {
            sh.dialog = Some(Dialog::Rename { what: RenameWhat::Layer(i), title: "Ebene umbenennen", text: b.layers[i].name.clone(), focus: true });
        }
        Some(Act::Anim(i)) => {
            let l = &b.layers[i];
            sh.dialog = Some(Dialog::Anim(AnimDlg { idx: i, anim: l.anim, speed: l.speed, hover: l.hover, opacity: l.opacity }));
        }
        Some(Act::Export(i)) => {
            let l = &b.layers[i];
            if !b.items.iter().any(|it| it.layer == l.id) {
                sh.toast(&format!("Die Ebene „{}“ ist leer", l.name));
            } else {
                let name = format!("{}.png", l.file_name());
                sh.open_file_dialog(FilePurpose::ExportLayer(i), "Ebene als PNG exportieren", ("PNG-Bild (*.png)", &["png"]), Some(name));
            }
        }
        Some(Act::Move(from, to)) => b.move_layer(from, to),
        Some(Act::Delete(i)) => {
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
        None => {}
    }
}
