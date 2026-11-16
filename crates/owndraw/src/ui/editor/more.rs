//! Popover „Weitere Werkzeuge“ (Knopf + am Ende der Werkzeugleiste): zeigt ausgeschaltete Werkzeuge mit
//! Beschreibung; Antippen schaltet ein Werkzeug ein und wählt es. Neue Werkzeuge sind anfangs aus – hier
//! werden sie gefunden. Ein Punkt am Knopf markiert ungesehene neue Werkzeuge (Einstellung `tools_seen`).

use super::{tools, Editor, Lay, Tool, M};
use crate::app::{Req, Shared};
use crate::layout::min_touch;
use crate::settings::Settings;
use crate::style::{self, c, fs, sp};
use egui::{Align2, Order, Pos2, Sense, Vec2};

/// Ausgeschaltete Werkzeuge (ohne solche, die über ein anderes erreichbar sind).
pub(super) fn hidden(s: &Settings) -> Vec<&'static tools::ToolDef> {
    tools::ordered(s).into_iter().filter(|d| !tools::enabled(s, d.id) && d.parent.is_none()).collect()
}

/// Gibt es ausgeschaltete neue Werkzeuge, die noch nie angesehen wurden?
pub(super) fn unseen(s: &Settings) -> bool {
    let seen = s.extra.get("tools_seen").and_then(|v| v.as_bool()).unwrap_or(false);
    !seen && hidden(s).iter().any(|d| d.new)
}

pub(super) fn mark_seen(s: &mut Settings) {
    s.extra.insert("tools_seen".into(), serde_json::json!(true));
}

pub(super) fn show(ed: &mut Editor, ctx: &egui::Context, sh: &mut Shared, lay: &Lay) {
    if !ed.st.more_open {
        return;
    }
    let Some(anchor) = ed.st.more_btn else { return };
    let canvas = lay.canvas;
    // neben (senkrechte Leiste) bzw. über/unter (waagerechte Leiste) dem Knopf
    let (pos, pivot) = if ed.st.tools_vert {
        if anchor.center().x < canvas.center().x {
            (Pos2::new(anchor.right() + M, anchor.top()), Align2::LEFT_TOP)
        } else {
            (Pos2::new(anchor.left() - M, anchor.top()), Align2::RIGHT_TOP)
        }
    } else if anchor.center().y > canvas.center().y {
        (Pos2::new(anchor.center().x, anchor.top() - M), Align2::CENTER_BOTTOM)
    } else {
        (Pos2::new(anchor.center().x, anchor.bottom() + M), Align2::CENTER_TOP)
    };
    let list_h = (canvas.height() - 2.0 * M - 190.0).clamp(140.0, 520.0);
    let w = (canvas.width() - 2.0 * M).min(340.0);
    let r = Editor::float(ctx, "more-tools", pos, pivot, canvas.shrink(M), Order::Foreground, |ui| {
        style::pill(18).inner_margin(egui::Margin::same(sp::M as i8)).show(ui, |ui| {
            ui.set_width(w);
            ui.horizontal(|ui| {
                ui.label(egui::RichText::new("Weitere Werkzeuge").size(fs::SECTION).strong());
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if style::icon_btn(ui, &mut sh.icons, owndraw_core::icons::CLOSE, style::pal().dim, false, false, 28.0, "Schließen").clicked() {
                        ed.st.more_open = false;
                    }
                });
            });
            ui.add(egui::Label::new(style::hint("Neue Werkzeuge sind anfangs ausgeblendet, damit die Leiste übersichtlich bleibt. Tippe auf ein Werkzeug, um es zur Leiste hinzuzufügen.")).wrap());
            ui.add_space(sp::S);
            egui::ScrollArea::vertical().max_height(list_h).auto_shrink([false, true]).show(ui, |ui| {
                list(ui, ed, sh);
            });
            ui.add_space(sp::S);
            style::separator(ui);
            let h = min_touch(sh.env.touch).max(32.0);
            ui.horizontal(|ui| {
                if style::outline_btn(ui, "Leiste anordnen …", h).clicked() {
                    ed.st.more_open = false;
                    ed.begin_edit_tools(sh);
                }
                if style::plain_btn(ui, "Einstellungen …", h).on_hover_text("Alle Werkzeuge in den Einstellungen ein- und ausschalten").clicked() {
                    ed.st.more_open = false;
                    crate::ui::settings::open_tools_page(ui.ctx());
                    sh.reqs.push(Req::OpenSettings);
                }
            });
        });
    });
    ed.st.popup_rects.push(r);
}

fn list(ui: &mut egui::Ui, ed: &mut Editor, sh: &mut Shared) {
    let p = style::pal();
    let items = hidden(&sh.settings);
    if items.is_empty() {
        ui.label(style::hint("Alle Werkzeuge sind bereits in der Leiste."));
        return;
    }
    let w = ui.available_width();
    let mut pick: Option<Tool> = None;
    for d in items {
        let desc_g = ui.painter().layout(d.desc.to_string(), egui::FontId::proportional(fs::SMALL), c(p.dim), w - 96.0);
        let hgt = (desc_g.size().y + 34.0).max(52.0);
        let (r, resp) = ui.allocate_exact_size(Vec2::new(w, hgt), Sense::click());
        if resp.hovered() || resp.is_pointer_button_down_on() {
            ui.painter().rect_filled(r, egui::CornerRadius::same(8), c(if resp.is_pointer_button_down_on() { p.press } else { p.hover }));
        }
        let ic_r = egui::Rect::from_center_size(egui::pos2(r.left() + 22.0, r.top() + 24.0), Vec2::splat(34.0));
        ui.painter().circle_filled(ic_r.center(), 17.0, c(p.accent_soft));
        sh.icons.paint(ui, ic_r, d.icon, p.accent, false, 19.0);
        let x = r.left() + 48.0;
        let name_g = ui.painter().layout_no_wrap(d.name.to_string(), egui::FontId::proportional(14.5), c(p.text));
        let nw = name_g.size().x;
        ui.painter().galley(egui::pos2(x, r.top() + 8.0), name_g, c(p.text));
        let mut bx = x + nw + 8.0;
        if d.new {
            let g = ui.painter().layout_no_wrap("Neu".into(), egui::FontId::proportional(fs::CAPTION), c(p.accent));
            let br = egui::Rect::from_min_size(egui::pos2(bx, r.top() + 9.0), g.size() + Vec2::new(12.0, 4.0));
            ui.painter().rect_filled(br, egui::CornerRadius::same(255), c(p.accent_soft));
            ui.painter().galley(br.min + Vec2::new(6.0, 2.0), g, c(p.accent));
            bx = br.right() + 8.0;
        }
        let k = super::keys::labels(&sh.settings, &super::keys::tool_action(d.id));
        if let Some(k) = k.first() {
            ui.painter().text(egui::pos2(bx, r.top() + 18.0), Align2::LEFT_CENTER, k, egui::FontId::monospace(11.5), c(p.dim));
        }
        ui.painter().galley(egui::pos2(x, r.top() + 28.0), desc_g, c(p.dim));
        // „+“ rechts
        let pr = egui::Rect::from_center_size(egui::pos2(r.right() - 22.0, r.top() + 24.0), Vec2::splat(30.0));
        ui.painter().circle_filled(pr.center(), 15.0, c(if resp.hovered() { p.accent } else { p.accent_soft }));
        sh.icons.paint(ui, pr, owndraw_core::icons::PLUS, if resp.hovered() { p.on_accent } else { p.accent }, false, 16.0);
        if resp.on_hover_text(format!("„{}“ zur Leiste hinzufügen", d.name)).clicked() {
            pick = Some(d.tool);
            tools::set_enabled(&mut sh.settings, d.id, true);
        }
    }
    if let Some(t) = pick {
        sh.toast(&format!("„{}“ ist jetzt in der Werkzeugleiste", t.label()));
        ed.select_tool(sh, t);
        if hidden(&sh.settings).is_empty() {
            ed.st.more_open = false;
        }
    }
}
