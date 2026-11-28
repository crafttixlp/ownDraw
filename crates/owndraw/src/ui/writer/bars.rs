//! Leisten und Fenster des Textdokuments: Kopfzeile mit Menüs, Formatleiste (am Handy unten), Suche,
//! Gliederung, Statuszeile, Befehlssuche, Rechtsklick-Menü und kleine Fenster (Link, Seite, Zeichen …).

use super::{view, Cmd, Dlg, Writer};
use crate::app::Shared;
use crate::autosave::AutosaveStatus;
use crate::style::{self, c, fs, pal, sp};
use egui::{Align2, Color32, CornerRadius, FontId, Frame, Margin, Painter, Pos2, Rect, Response, Sense, Stroke, Vec2};
use owndraw_core::icons as ic;
use owndraw_core::office::fonts::{Family, FontKey};
use owndraw_core::office::{Align, Kind, PageSpec};

/// Textfarben und Markerfarben der Leiste (0 = automatisch/keine).
const COLORS: [u32; 10] = [0, 0xFF5F6368, 0xFFD93025, 0xFFE8710A, 0xFFF9AB00, 0xFF188038, 0xFF1A73E8, 0xFF9334E6, 0xFFE52592, 0xFF795548];
const HIGHLIGHTS: [u32; 7] = [0, 0xFFFDE68A, 0xFFBBF7D0, 0xFFBFDBFE, 0xFFFBCFE8, 0xFFFED7AA, 0xFFE5E7EB];
const SPECIAL: &str = "€£¥©®™§¶°±×÷≈≠≤≥∞√∑πΩαβγδµ←→↑↓⇒•–—…„“‚‘«»¼½¾¹²³✓✗★☆♥";

fn narrow(ui: &egui::Ui) -> bool {
    ui.ctx().content_rect().width() < 700.0
}

pub fn frame(w: &mut Writer, ui: &mut egui::Ui, sh: &mut Shared) {
    let p = pal();
    let nar = narrow(ui);
    let bar = Frame::new().fill(c(p.card)).inner_margin(Margin::symmetric(8, 4));
    egui::Panel::top("w-top").exact_size(if nar { 52.0 } else { 62.0 }).frame(bar).show(ui, |ui| top_bar(w, ui, sh, nar));
    let tools = Frame::new().fill(c(p.card)).inner_margin(Margin::symmetric(8, 3)).stroke(Stroke::new(1.0, c(p.line)));
    if nar {
        egui::Panel::bottom("w-tools").exact_size(52.0).frame(tools).show(ui, |ui| toolbar(w, ui, sh, true));
    } else {
        egui::Panel::top("w-tools").exact_size(44.0).frame(tools).show(ui, |ui| toolbar(w, ui, sh, false));
    }
    if !nar {
        egui::Panel::bottom("w-status").exact_size(28.0).frame(Frame::new().fill(c(p.card)).inner_margin(Margin::symmetric(10, 2))).show(ui, |ui| status(w, ui, sh));
    }
    if w.find.is_some() {
        egui::Panel::top("w-find").frame(Frame::new().fill(c(p.card)).inner_margin(Margin::symmetric(10, 6)).stroke(Stroke::new(1.0, c(p.line)))).show(ui, |ui| find_bar(w, ui, sh));
    }
    if w.outline {
        egui::Panel::left("w-outline").exact_size(if nar { 180.0 } else { 240.0 }).frame(Frame::new().fill(c(p.card)).inner_margin(Margin::same(10))).show(ui, |ui| outline(w, ui, sh));
    }
    let desk = if p.dark { 0xFF1B1C20 } else { 0xFFEEF0F4 };
    egui::CentralPanel::default().frame(Frame::new().fill(c(desk))).show(ui, |ui| w.canvas(ui, sh));
    palette(w, ui.ctx(), sh);
    dialogs(w, ui.ctx(), sh);
}

// ---------------------------------------------------------------- Kopfzeile

fn save_text(w: &Writer) -> &'static str {
    if w.doc.meta.read_only {
        return "Schreibgeschützt";
    }
    match w.autosave.status() {
        AutosaveStatus::Error(_) => "Speichern fehlgeschlagen",
        AutosaveStatus::Dirty | AutosaveStatus::Saving => "Wird gespeichert …",
        AutosaveStatus::Saved(_) => "Gespeichert",
        AutosaveStatus::Idle => "",
    }
}

fn top_bar(w: &mut Writer, ui: &mut egui::Ui, sh: &mut Shared, nar: bool) {
    let p = pal();
    let ctx = ui.ctx().clone();
    let b = 38.0;
    ui.horizontal_centered(|ui| {
        if style::icon_btn(ui, &mut sh.icons, ic::BACK, p.text, false, false, b, "Zurück zur Startseite").clicked() {
            w.run(Cmd::Back, sh, &ctx);
        }
        // Dokument-Symbol
        let (r, _) = ui.allocate_exact_size(Vec2::new(26.0, 32.0), Sense::hover());
        doc_icon(ui.painter(), r, c(p.accent));
        ui.vertical(|ui| {
            ui.spacing_mut().item_spacing.y = 0.0;
            ui.horizontal(|ui| {
                if let Some(name) = w.rename.as_mut() {
                    let te = ui.add(egui::TextEdit::singleline(name).desired_width(260.0).font(FontId::proportional(16.0)));
                    if !te.has_focus() && !te.lost_focus() {
                        te.request_focus();
                    }
                    if te.lost_focus() {
                        let n = w.rename.take().unwrap_or_default();
                        w.rename_to(n, sh.now);
                        w.focus_req = true;
                    }
                } else {
                    let maxw = if nar { ui.ctx().content_rect().width() - 250.0 } else { 360.0 };
                    let t = crate::ui::elide(w.name(), 16.0, maxw.max(60.0), ui);
                    let r = ui.add(egui::Label::new(egui::RichText::new(t).size(16.0).color(c(p.text))).sense(Sense::click())).on_hover_text("Umbenennen");
                    if r.clicked() {
                        w.rename = Some(w.name().to_string());
                    }
                    if !nar {
                        let st = save_text(w);
                        let col = if st.starts_with("Speichern f") { p.danger_text } else { p.dim };
                        ui.label(egui::RichText::new(st).size(fs::CAPTION).color(c(col)));
                    }
                }
            });
            if !nar {
                ui.horizontal(|ui| {
                    ui.spacing_mut().item_spacing.x = 0.0;
                    menus(w, ui, sh);
                });
            }
        });
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            if style::icon_btn(ui, &mut sh.icons, ic::HELP, p.dim, false, false, b, "Anleitung & Tipps (F1)").clicked() {
                w.run(Cmd::Help, sh, &ctx);
            }
            let ex = style::icon_btn(ui, &mut sh.icons, ic::DOWNLOAD, p.dim, false, false, b, "Herunterladen, Drucken");
            egui::Popup::menu(&ex).align(egui::RectAlign::BOTTOM_END).show(|ui| {
                let mut list = vec![Some(Cmd::ExportPdf)];
                if !crate::locks::locked(owndraw_core::features::OFFICE_WORD) {
                    list.push(Some(Cmd::ExportDocx));
                }
                list.extend([Some(Cmd::Print), None, Some(Cmd::SaveCopy)]);
                menu_list(w, ui, sh, &list);
            });
            if nar {
                let m = style::icon_btn(ui, &mut sh.icons, ic::MORE, p.dim, false, false, b, "Menü");
                egui::Popup::menu(&m).align(egui::RectAlign::BOTTOM_END).show(|ui| {
                    ui.set_min_width(230.0);
                    let list = [Some(Cmd::Find), Some(Cmd::Replace), Some(Cmd::Outline), None, Some(Cmd::PageSetup), Some(Cmd::HeaderFooter), Some(Cmd::PageNumbers), Some(Cmd::Toc), None, Some(Cmd::WordCount), Some(Cmd::Rename), Some(Cmd::ImportDocx)];
                    menu_list(w, ui, sh, &list);
                });
            }
            // Befehlssuche
            let r = if nar {
                style::icon_btn(ui, &mut sh.icons, ic::SEARCH, p.dim, false, false, b, "Funktion suchen (Strg+Umschalt+P)")
            } else {
                let (r, resp) = ui.allocate_exact_size(Vec2::new(190.0, 32.0), Sense::click());
                ui.painter().rect_filled(r, CornerRadius::same(16), c(if resp.hovered() { p.hover } else { p.pill }));
                sh.icons.paint(ui, Rect::from_center_size(Pos2::new(r.left() + 18.0, r.center().y), Vec2::splat(18.0)), ic::SEARCH, p.dim, false, 16.0);
                ui.painter().text(Pos2::new(r.left() + 34.0, r.center().y), Align2::LEFT_CENTER, "Funktion suchen …", FontId::proportional(13.5), c(p.dim));
                resp.on_hover_text("Alle Befehle durchsuchen (Strg+Umschalt+P)")
            };
            if r.clicked() {
                w.palette = Some((String::new(), 0));
            }
        });
    });
}

fn doc_icon(pt: &Painter, r: Rect, col: Color32) {
    let d = Rect::from_center_size(r.center(), Vec2::new(18.0, 24.0));
    pt.rect_filled(d, CornerRadius::same(3), col);
    for i in 0..4 {
        let y = d.top() + 7.0 + i as f32 * 4.0;
        let x1 = if i == 3 { d.right() - 8.0 } else { d.right() - 4.0 };
        pt.line_segment([Pos2::new(d.left() + 4.0, y), Pos2::new(x1, y)], Stroke::new(1.5, Color32::WHITE));
    }
}

/// Menüleiste (breite Fenster).
fn menus(w: &mut Writer, ui: &mut egui::Ui, sh: &mut Shared) {
    let word = !crate::locks::locked(owndraw_core::features::OFFICE_WORD);
    let datei: Vec<Option<Cmd>> = [Some(Cmd::Rename), Some(Cmd::SaveCopy), None, Some(Cmd::ExportPdf)]
        .into_iter()
        .chain(word.then_some([Some(Cmd::ExportDocx), Some(Cmd::ImportDocx)]).into_iter().flatten())
        .chain([Some(Cmd::Print), None, Some(Cmd::PageSetup), None, Some(Cmd::Back)])
        .collect();
    let groups: [(&str, Vec<Option<Cmd>>); 6] = [
        ("Datei", datei),
        ("Bearbeiten", vec![Some(Cmd::Undo), Some(Cmd::Redo), None, Some(Cmd::Cut), Some(Cmd::Copy), Some(Cmd::Paste), Some(Cmd::SelectAll), None, Some(Cmd::Find), Some(Cmd::Replace)]),
        ("Ansicht", vec![Some(Cmd::Outline), None, Some(Cmd::ZoomIn), Some(Cmd::ZoomOut), Some(Cmd::ZoomReset)]),
        ("Einfügen", vec![Some(Cmd::Image), Some(Cmd::Link), Some(Cmd::PageBreak), Some(Cmd::Date), Some(Cmd::Special), None, Some(Cmd::HeaderFooter), Some(Cmd::PageNumbers), Some(Cmd::Toc)]),
        (
            "Format",
            vec![
                Some(Cmd::Bold),
                Some(Cmd::Italic),
                Some(Cmd::Underline),
                Some(Cmd::Strike),
                Some(Cmd::Sup),
                Some(Cmd::Sub),
                None,
                Some(Cmd::Kind(Kind::Body)),
                Some(Cmd::Kind(Kind::Title)),
                Some(Cmd::Kind(Kind::H1)),
                Some(Cmd::Kind(Kind::H2)),
                Some(Cmd::Kind(Kind::H3)),
                Some(Cmd::Kind(Kind::Quote)),
                None,
                Some(Cmd::Bullets),
                Some(Cmd::Numbers),
                Some(Cmd::Indent),
                Some(Cmd::Outdent),
                None,
                Some(Cmd::Spacing(1.0)),
                Some(Cmd::Spacing(1.15)),
                Some(Cmd::Spacing(1.5)),
                Some(Cmd::Spacing(2.0)),
                None,
                Some(Cmd::ClearFmt),
            ],
        ),
        ("Extras", vec![Some(Cmd::WordCount), Some(Cmd::Shortcuts), Some(Cmd::Help)]),
    ];
    let p = pal();
    for (name, list) in groups {
        let g = ui.painter().layout_no_wrap(name.into(), FontId::proportional(13.0), c(p.text));
        let (r, resp) = ui.allocate_exact_size(Vec2::new(g.size().x + 14.0, 22.0), Sense::click());
        if resp.hovered() || egui::Popup::is_id_open(ui.ctx(), egui::Popup::default_response_id(&resp)) {
            ui.painter().rect_filled(r, CornerRadius::same(4), c(p.hover));
        }
        ui.painter().galley(Pos2::new(r.left() + 7.0, r.center().y - g.size().y / 2.0), g, c(p.text));
        egui::Popup::menu(&resp).show(|ui| {
            ui.set_min_width(250.0);
            menu_list(w, ui, sh, &list);
        });
    }
}

/// Menüeinträge mit Tastenkürzel; `None` = Trennlinie.
fn menu_list(w: &mut Writer, ui: &mut egui::Ui, sh: &mut Shared, list: &[Option<Cmd>]) {
    let ctx = ui.ctx().clone();
    for e in list {
        match e {
            None => style::separator(ui),
            Some(cmd) => {
                let (label, key) = super::cmds().find(|x| x.0 == *cmd).map(|x| (x.1, x.2)).unwrap_or(("", ""));
                let enabled = match cmd {
                    Cmd::Undo => w.can_undo(),
                    Cmd::Redo => w.can_redo(),
                    Cmd::Cut | Cmd::Copy => w.has_sel(),
                    _ => true,
                };
                let checked = match cmd {
                    Cmd::Outline => w.outline,
                    Cmd::Spacing(v) => {
                        let l = w.cur_para().line;
                        ((if l <= 0.0 { owndraw_core::office::layout::LINE } else { l }) - v).abs() < 0.01
                    }
                    Cmd::Kind(k) => w.cur_para().kind == *k,
                    _ => false,
                };
                if row(ui, label, key, enabled, checked) {
                    w.run(*cmd, sh, &ctx);
                    ui.close();
                }
            }
        }
    }
}

fn row(ui: &mut egui::Ui, label: &str, right: &str, enabled: bool, checked: bool) -> bool {
    let p = pal();
    let wdt = ui.available_width().max(200.0);
    let (r, resp) = ui.allocate_exact_size(Vec2::new(wdt, 30.0), if enabled { Sense::click() } else { Sense::hover() });
    if enabled && (resp.hovered() || resp.has_focus()) {
        ui.painter().rect_filled(r, CornerRadius::same(6), c(p.hover));
    }
    let col = if enabled { p.text } else { p.disabled };
    if checked {
        ui.painter().text(Pos2::new(r.left() + 12.0, r.center().y), Align2::CENTER_CENTER, "✓", FontId::proportional(13.0), c(p.accent));
    }
    ui.painter().text(Pos2::new(r.left() + 26.0, r.center().y), Align2::LEFT_CENTER, label, FontId::proportional(14.0), c(col));
    if !right.is_empty() {
        ui.painter().text(Pos2::new(r.right() - 10.0, r.center().y), Align2::RIGHT_CENTER, right, FontId::proportional(fs::CAPTION), c(p.dim));
    }
    enabled && resp.clicked()
}

// ---------------------------------------------------------------- Formatleiste

/// Knopf mit selbst gezeichnetem Inhalt.
fn tb(ui: &mut egui::Ui, wdt: f32, active: bool, enabled: bool, tip: &str, paint: impl FnOnce(&Painter, Rect, Color32)) -> Response {
    let p = pal();
    let h = if narrow(ui) { 42.0 } else { 34.0 };
    let (r, resp) = ui.allocate_exact_size(Vec2::new(wdt, h), if enabled { Sense::click() } else { Sense::hover() });
    style::btn_bg(ui, r, &resp, active);
    paint(ui.painter(), r, c(if enabled { p.text } else { p.disabled }));
    resp.on_hover_text(tip)
}

fn sep(ui: &mut egui::Ui) {
    let (r, _) = ui.allocate_exact_size(Vec2::new(9.0, 24.0), Sense::hover());
    ui.painter().line_segment([Pos2::new(r.center().x, r.top() + 2.0), Pos2::new(r.center().x, r.bottom() - 2.0)], Stroke::new(1.0, c(pal().line)));
}

fn letter(pt: &Painter, r: Rect, s: &str, k: FontKey, col: Color32, ready: bool) {
    let f = if ready { view::font_id(k, 17.0) } else { FontId::proportional(16.0) };
    pt.text(r.center(), Align2::CENTER_CENTER, s, f, col);
}

fn lines_icon(pt: &Painter, r: Rect, col: Color32, a: Align) {
    let ws = [16.0, 11.0, 16.0, 9.0];
    for (i, lw) in ws.iter().enumerate() {
        let lw = if a == Align::Justify { 16.0 } else { *lw };
        let y = r.center().y - 6.0 + i as f32 * 4.0;
        let (x0, x1) = match a {
            Align::Left | Align::Justify => (r.center().x - 8.0, r.center().x - 8.0 + lw),
            Align::Center => (r.center().x - lw / 2.0, r.center().x + lw / 2.0),
            Align::Right => (r.center().x + 8.0 - lw, r.center().x + 8.0),
        };
        pt.line_segment([Pos2::new(x0, y), Pos2::new(x1, y)], Stroke::new(1.6, col));
    }
}

fn list_icon(pt: &Painter, r: Rect, col: Color32, numbers: bool) {
    for i in 0..3 {
        let y = r.center().y - 6.0 + i as f32 * 6.0;
        let x = r.center().x - 8.0;
        if numbers {
            pt.text(Pos2::new(x, y), Align2::CENTER_CENTER, ["1", "2", "3"][i], FontId::proportional(8.5), col);
        } else {
            pt.circle_filled(Pos2::new(x, y), 1.7, col);
        }
        pt.line_segment([Pos2::new(x + 5.0, y), Pos2::new(x + 16.0, y)], Stroke::new(1.6, col));
    }
}

fn indent_icon(pt: &Painter, r: Rect, col: Color32, more: bool) {
    let cx = r.center().x;
    let cy = r.center().y;
    for i in 0..4 {
        let y = cy - 6.0 + i as f32 * 4.0;
        let x0 = if i == 1 || i == 2 { cx - 1.0 } else { cx - 8.0 };
        pt.line_segment([Pos2::new(x0, y), Pos2::new(cx + 8.0, y)], Stroke::new(1.5, col));
    }
    let (a, b) = if more { (cx - 8.0, cx - 4.0) } else { (cx - 4.0, cx - 8.0) };
    pt.add(egui::Shape::convex_polygon(vec![Pos2::new(a, cy - 3.0), Pos2::new(b, cy), Pos2::new(a, cy + 3.0)], col, Stroke::NONE));
}

fn link_icon(pt: &Painter, r: Rect, col: Color32) {
    let cy = r.center().y;
    for dx in [-4.0, 4.0] {
        let rr = Rect::from_center_size(Pos2::new(r.center().x + dx, cy), Vec2::new(11.0, 7.0));
        pt.rect_stroke(rr, CornerRadius::same(4), Stroke::new(1.6, col), egui::StrokeKind::Middle);
    }
}

fn caret_down(pt: &Painter, at: Pos2, col: Color32) {
    pt.add(egui::Shape::convex_polygon(vec![Pos2::new(at.x - 3.5, at.y - 2.0), Pos2::new(at.x + 3.5, at.y - 2.0), Pos2::new(at.x, at.y + 2.0)], col, Stroke::NONE));
}

/// Aufklapp-Knopf mit Text.
fn drop_btn(ui: &mut egui::Ui, label: &str, wdt: f32, tip: &str) -> Response {
    let p = pal();
    let h = if narrow(ui) { 42.0 } else { 34.0 };
    let (r, resp) = ui.allocate_exact_size(Vec2::new(wdt, h), Sense::click());
    style::btn_bg(ui, r, &resp, false);
    let t = crate::ui::elide(label, 13.5, wdt - 24.0, ui);
    ui.painter().text(Pos2::new(r.left() + 8.0, r.center().y), Align2::LEFT_CENTER, t, FontId::proportional(13.5), c(p.text));
    caret_down(ui.painter(), Pos2::new(r.right() - 9.0, r.center().y), c(p.dim));
    resp.on_hover_text(tip)
}

fn toolbar(w: &mut Writer, ui: &mut egui::Ui, sh: &mut Shared, nar: bool) {
    let ctx = ui.ctx().clone();
    let p = pal();
    let ready = ctx.fonts(|f| f.definitions().families.contains_key(&egui::FontFamily::Name(FontKey::default().id().into())));
    let st = w.cur_style();
    let mut para = w.cur_para().clone();
    if para.line <= 0.0 {
        para.line = owndraw_core::office::layout::LINE;
    }
    let mut cmd: Option<Cmd> = None;
    let b = if nar { 42.0 } else { 34.0 };
    egui::ScrollArea::horizontal().id_salt("w-tools-scroll").scroll_bar_visibility(egui::scroll_area::ScrollBarVisibility::AlwaysHidden).show(ui, |ui| {
        ui.horizontal_centered(|ui| {
            ui.spacing_mut().item_spacing.x = 1.0;
            let (u, r) = (w.can_undo(), w.can_redo());
            let ur = ui.add_enabled_ui(u, |ui| style::icon_btn(ui, &mut sh.icons, ic::UNDO, p.text, false, false, b, "Rückgängig (Strg+Z)")).inner;
            if ur.clicked() {
                cmd = Some(Cmd::Undo);
            }
            let rr = ui.add_enabled_ui(r, |ui| style::icon_btn(ui, &mut sh.icons, ic::REDO, p.text, false, false, b, "Wiederholen (Strg+Y)")).inner;
            if rr.clicked() {
                cmd = Some(Cmd::Redo);
            }
            sep(ui);
            // Absatzart
            let sr = drop_btn(ui, para.kind.label(), if nar { 118.0 } else { 136.0 }, "Absatzformat: Text, Überschriften, Listen");
            egui::Popup::menu(&sr).show(|ui| {
                ui.set_min_width(240.0);
                for k in [Kind::Body, Kind::Title, Kind::H1, Kind::H2, Kind::H3, Kind::Quote] {
                    let ps = owndraw_core::office::layout::para_style(&w.td, k);
                    let (r, resp) = ui.allocate_exact_size(Vec2::new(ui.available_width(), (ps.size * 1.25).clamp(28.0, 44.0)), Sense::click());
                    if resp.hovered() || para.kind == k {
                        ui.painter().rect_filled(r, CornerRadius::same(6), c(if para.kind == k { p.active } else { p.hover }));
                    }
                    let fk = FontKey { fam: ps.fam, bold: ps.bold, italic: ps.italic };
                    let f = if ready { view::font_id(fk, ps.size.min(24.0)) } else { FontId::proportional(14.0) };
                    ui.painter().text(Pos2::new(r.left() + 10.0, r.center().y), Align2::LEFT_CENTER, k.label(), f, c(p.text));
                    if resp.clicked() {
                        cmd = Some(Cmd::Kind(k));
                        ui.close();
                    }
                }
            });
            // Schriftart
            if !nar {
                let fam = w.cur_family();
                let fr = drop_btn(ui, fam.label(), 92.0, "Schriftart");
                egui::Popup::menu(&fr).show(|ui| {
                    ui.set_min_width(170.0);
                    for f in [Family::Serif, Family::Sans] {
                        let fk = FontKey { fam: f, bold: false, italic: false };
                        let (r, resp) = ui.allocate_exact_size(Vec2::new(ui.available_width(), 30.0), Sense::click());
                        if resp.hovered() || fam == f {
                            ui.painter().rect_filled(r, CornerRadius::same(6), c(if fam == f { p.active } else { p.hover }));
                        }
                        let font = if ready { view::font_id(fk, 15.0) } else { FontId::proportional(14.0) };
                        ui.painter().text(Pos2::new(r.left() + 10.0, r.center().y), Align2::LEFT_CENTER, format!("{} ({})", f.label(), f.word_name()), font, c(p.text));
                        if resp.clicked() {
                            cmd = Some(Cmd::Family(f));
                            ui.close();
                        }
                    }
                });
            }
            sep(ui);
            // Größe − n +
            if tb(ui, 26.0, false, true, "Schrift kleiner (Strg+Umschalt+,)", |pt, r, col| {
                pt.line_segment([Pos2::new(r.center().x - 5.0, r.center().y), Pos2::new(r.center().x + 5.0, r.center().y)], Stroke::new(1.6, col));
            })
            .clicked()
            {
                cmd = Some(Cmd::FontDown);
            }
            let size = w.cur_size();
            let sz = drop_btn(ui, &fmt_size(size), 46.0, "Schriftgröße");
            egui::Popup::menu(&sz).show(|ui| {
                ui.set_min_width(80.0);
                for v in [8.0, 9.0, 10.0, 11.0, 12.0, 14.0, 16.0, 18.0, 24.0, 30.0, 36.0, 48.0, 60.0, 72.0] {
                    if row(ui, &fmt_size(v), "", true, (v - size).abs() < 0.01) {
                        w.set_size(v, sh.now);
                        w.focus_req = true;
                        ui.close();
                    }
                }
            });
            if tb(ui, 26.0, false, true, "Schrift größer (Strg+Umschalt+.)", |pt, r, col| {
                let m = r.center();
                pt.line_segment([Pos2::new(m.x - 5.0, m.y), Pos2::new(m.x + 5.0, m.y)], Stroke::new(1.6, col));
                pt.line_segment([Pos2::new(m.x, m.y - 5.0), Pos2::new(m.x, m.y + 5.0)], Stroke::new(1.6, col));
            })
            .clicked()
            {
                cmd = Some(Cmd::FontUp);
            }
            sep(ui);
            let sans_b = FontKey { fam: Family::Sans, bold: true, italic: false };
            if tb(ui, b, st.b, true, "Fett (Strg+B)", |pt, r, col| letter(pt, r, "B", sans_b, col, ready)).clicked() {
                cmd = Some(Cmd::Bold);
            }
            if tb(ui, b, st.i, true, "Kursiv (Strg+I)", |pt, r, col| letter(pt, r, "I", FontKey { fam: Family::Serif, bold: false, italic: true }, col, ready)).clicked() {
                cmd = Some(Cmd::Italic);
            }
            if tb(ui, b, st.u, true, "Unterstrichen (Strg+U)", |pt, r, col| {
                letter(pt, r.translate(Vec2::new(0.0, -1.5)), "U", FontKey { fam: Family::Sans, ..Default::default() }, col, ready);
                pt.line_segment([Pos2::new(r.center().x - 6.0, r.center().y + 9.0), Pos2::new(r.center().x + 6.0, r.center().y + 9.0)], Stroke::new(1.4, col));
            })
            .clicked()
            {
                cmd = Some(Cmd::Underline);
            }
            if !nar
                && tb(ui, b, st.s, true, "Durchgestrichen (Alt+Umschalt+5)", |pt, r, col| {
                    letter(pt, r, "S", FontKey { fam: Family::Sans, ..Default::default() }, col, ready);
                    pt.line_segment([Pos2::new(r.center().x - 7.0, r.center().y + 0.5), Pos2::new(r.center().x + 7.0, r.center().y + 0.5)], Stroke::new(1.4, col));
                })
                .clicked()
            {
                cmd = Some(Cmd::Strike);
            }
            // Textfarbe
            let tc = if st.color == 0 { 0xFF1D1D24 } else { st.color };
            let cr = tb(ui, b, false, true, "Textfarbe", |pt, r, col| {
                letter(pt, r.translate(Vec2::new(0.0, -2.5)), "A", FontKey { fam: Family::Sans, ..Default::default() }, col, ready);
                pt.rect_filled(Rect::from_center_size(Pos2::new(r.center().x, r.center().y + 9.0), Vec2::new(14.0, 3.5)), CornerRadius::same(1), c(tc));
            });
            egui::Popup::menu(&cr).show(|ui| {
                if let Some(v) = swatches(ui, &COLORS, st.color, "Automatisch") {
                    cmd = Some(Cmd::Color(v));
                }
            });
            let hr = tb(ui, b, false, true, "Markieren (Hintergrundfarbe)", |pt, r, col| {
                let m = r.center() - Vec2::new(0.0, 2.0);
                pt.add(egui::Shape::convex_polygon(vec![m + Vec2::new(-2.0, 5.0), m + Vec2::new(4.0, -5.0), m + Vec2::new(7.0, -3.0), m + Vec2::new(1.0, 7.0)], Color32::TRANSPARENT, Stroke::new(1.5, col)));
                let hc = if st.hl == 0 { 0x00000000 } else { st.hl };
                pt.rect_filled(Rect::from_center_size(Pos2::new(r.center().x, r.center().y + 9.0), Vec2::new(14.0, 3.5)), CornerRadius::same(1), c(if hc == 0 { 0xFFFDE68A } else { hc }));
            });
            egui::Popup::menu(&hr).show(|ui| {
                if let Some(v) = swatches(ui, &HIGHLIGHTS, st.hl, "Keine") {
                    cmd = Some(Cmd::Highlight(v));
                }
            });
            if tb(ui, b, st.link.is_some(), true, "Link (Strg+K)", link_icon).clicked() {
                cmd = Some(Cmd::Link);
            }
            sep(ui);
            // Ausrichtung
            let ar = tb(ui, b + 8.0, false, true, "Ausrichtung", |pt, r, col| {
                lines_icon(pt, r.translate(Vec2::new(-4.0, 0.0)), col, para.align);
                caret_down(pt, Pos2::new(r.right() - 7.0, r.center().y), col);
            });
            egui::Popup::menu(&ar).show(|ui| {
                ui.horizontal(|ui| {
                    for (a, tip) in [(Align::Left, "Linksbündig"), (Align::Center, "Zentriert"), (Align::Right, "Rechtsbündig"), (Align::Justify, "Blocksatz")] {
                        if tb(ui, 36.0, para.align == a, true, tip, |pt, r, col| lines_icon(pt, r, col, a)).clicked() {
                            cmd = Some(Cmd::Align(a));
                            ui.close();
                        }
                    }
                });
            });
            if !nar {
                let lr = drop_btn(ui, &fmt_line(para.line), 52.0, "Zeilenabstand");
                egui::Popup::menu(&lr).show(|ui| {
                    ui.set_min_width(150.0);
                    for v in [1.0, 1.15, 1.5, 2.0] {
                        if row(ui, &fmt_line(v), "", true, (para.line - v).abs() < 0.01) {
                            cmd = Some(Cmd::Spacing(v));
                            ui.close();
                        }
                    }
                });
            }
            if tb(ui, b, para.kind == Kind::Bullet, true, "Aufzählung (Strg+Umschalt+8)", |pt, r, col| list_icon(pt, r, col, false)).clicked() {
                cmd = Some(Cmd::Bullets);
            }
            if tb(ui, b, para.kind == Kind::Number, true, "Nummerierung (Strg+Umschalt+7)", |pt, r, col| list_icon(pt, r, col, true)).clicked() {
                cmd = Some(Cmd::Numbers);
            }
            if tb(ui, b, false, para.indent > 0, "Einzug verkleinern (Umschalt+Tab)", |pt, r, col| indent_icon(pt, r, col, false)).clicked() {
                cmd = Some(Cmd::Outdent);
            }
            if tb(ui, b, false, true, "Einzug vergrößern (Tab)", |pt, r, col| indent_icon(pt, r, col, true)).clicked() {
                cmd = Some(Cmd::Indent);
            }
            sep(ui);
            // Einfügen
            if style::icon_btn(ui, &mut sh.icons, ic::IMAGE, p.text, false, false, b, "Bild einfügen (auch Strg+V oder auf die Seite ziehen)").clicked() {
                cmd = Some(Cmd::Image);
            }
            let ins = style::icon_btn(ui, &mut sh.icons, ic::PLUS, p.text, false, false, b, "Einfügen: Bild, Seitenumbruch, Link, Datum, Zeichen …");
            egui::Popup::menu(&ins).show(|ui| {
                ui.set_min_width(250.0);
                let mut list = vec![Some(Cmd::Image), Some(Cmd::Link), Some(Cmd::PageBreak), Some(Cmd::Date), Some(Cmd::Special), None, Some(Cmd::HeaderFooter), Some(Cmd::PageNumbers), Some(Cmd::Toc)];
                if !crate::locks::locked(owndraw_core::features::OFFICE_WORD) {
                    list.extend([None, Some(Cmd::ImportDocx)]);
                }
                menu_list(w, ui, sh, &list);
            });
            if tb(ui, b, false, true, "Formatierung entfernen (Strg+Leertaste)", |pt, r, col| {
                letter(pt, r.translate(Vec2::new(-2.0, 0.0)), "T", FontKey { fam: Family::Sans, ..Default::default() }, col, ready);
                let m = r.center() + Vec2::new(5.0, 5.0);
                pt.line_segment([m - Vec2::splat(3.0), m + Vec2::splat(3.0)], Stroke::new(1.5, col));
                pt.line_segment([m + Vec2::new(-3.0, 3.0), m + Vec2::new(3.0, -3.0)], Stroke::new(1.5, col));
            })
            .clicked()
            {
                cmd = Some(Cmd::ClearFmt);
            }
            if nar {
                let cr = style::icon_btn(ui, &mut sh.icons, ic::COPY, p.text, false, false, b, "Kopieren, Ausschneiden, Einfügen");
                egui::Popup::menu(&cr).show(|ui| {
                    ui.set_min_width(200.0);
                    menu_list(w, ui, sh, &[Some(Cmd::SelectAll), Some(Cmd::Cut), Some(Cmd::Copy), Some(Cmd::Paste)]);
                });
            }
        });
    });
    if let Some(c) = cmd {
        w.run(c, sh, &ctx);
    }
}

fn fmt_size(v: f32) -> String {
    if (v - v.round()).abs() < 0.05 {
        format!("{}", v.round() as i32)
    } else {
        format!("{v:.1}").replace('.', ",")
    }
}

fn fmt_line(v: f32) -> String {
    match v {
        x if (x - 1.0).abs() < 0.01 => "Einfach".into(),
        x if (x - 2.0).abs() < 0.01 => "Doppelt".into(),
        x => format!("{x:.2}").trim_end_matches('0').replace('.', ","),
    }
}

/// Farbfelder; liefert die gewählte Farbe.
fn swatches(ui: &mut egui::Ui, cols: &[u32], cur: u32, none: &str) -> Option<u32> {
    let p = pal();
    let mut out = None;
    ui.set_min_width(200.0);
    if row(ui, none, "", true, cur == 0) {
        out = Some(0);
    }
    ui.horizontal_wrapped(|ui| {
        ui.set_max_width(200.0);
        for &col in cols.iter().filter(|c| **c != 0) {
            let (r, resp) = ui.allocate_exact_size(Vec2::splat(30.0), Sense::click());
            ui.painter().circle_filled(r.center(), 11.0, c(col));
            if col == cur {
                ui.painter().circle_stroke(r.center(), 13.5, Stroke::new(2.0, c(p.accent)));
            } else if resp.hovered() {
                ui.painter().circle_stroke(r.center(), 13.0, Stroke::new(1.0, c(p.line)));
            }
            if resp.clicked() {
                out = Some(col);
            }
        }
    });
    if out.is_some() {
        ui.close();
    }
    out
}

// ---------------------------------------------------------------- Status, Suche, Gliederung

fn status(w: &mut Writer, ui: &mut egui::Ui, sh: &mut Shared) {
    let p = pal();
    let ctx = ui.ctx().clone();
    ui.horizontal_centered(|ui| {
        let ol = if w.outline { "Gliederung ausblenden" } else { "Gliederung" };
        if ui.add(egui::Button::new(egui::RichText::new(ol).size(fs::CAPTION)).frame(false)).clicked() {
            w.run(Cmd::Outline, sh, &ctx);
        }
        ui.add_space(sp::M);
        let cur = w.cur;
        let (pg, ..) = w.layout().caret(cur);
        let pages = w.layout().pages;
        ui.label(egui::RichText::new(format!("Seite {} von {}", pg + 1, pages)).size(fs::CAPTION).color(c(p.dim)));
        ui.add_space(sp::M);
        let words = w.td.words();
        let t = if w.has_sel() {
            let (a, b) = w.sel();
            let n = w.td.text_range(a, b).split_whitespace().count();
            format!("{n} von {words} Wörtern")
        } else {
            format!("{words} Wörter")
        };
        if ui.add(egui::Button::new(egui::RichText::new(t).size(fs::CAPTION).color(c(p.dim))).frame(false)).on_hover_text("Wörter zählen").clicked() {
            w.run(Cmd::WordCount, sh, &ctx);
        }
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            if ui.add(egui::Button::new(egui::RichText::new("+").size(14.0)).frame(false)).on_hover_text("Vergrößern (Strg++)").clicked() {
                w.run(Cmd::ZoomIn, sh, &ctx);
            }
            if ui.add(egui::Button::new(egui::RichText::new(format!("{} %", (w.zoom * 100.0).round() as i32)).size(fs::CAPTION)).frame(false)).on_hover_text("Zoom 100 % (Strg+0)").clicked() {
                w.run(Cmd::ZoomReset, sh, &ctx);
            }
            if ui.add(egui::Button::new(egui::RichText::new("−").size(14.0)).frame(false)).on_hover_text("Verkleinern (Strg+-)").clicked() {
                w.run(Cmd::ZoomOut, sh, &ctx);
            }
        });
    });
}

fn find_bar(w: &mut Writer, ui: &mut egui::Ui, sh: &mut Shared) {
    let now = sh.now;
    let mut close = false;
    let mut step = 0;
    let mut rep_one = false;
    let mut rep_all = false;
    w.update_find();
    let Some(f) = w.find.as_mut() else { return };
    ui.horizontal_wrapped(|ui| {
        let te = ui.add(egui::TextEdit::singleline(&mut f.q).hint_text("Im Dokument suchen").desired_width(220.0));
        if std::mem::take(&mut f.focus) {
            te.request_focus();
        }
        if te.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter)) {
            step = if ui.input(|i| i.modifiers.shift) { -1 } else { 1 };
            te.request_focus();
        }
        let n = f.hits.len();
        ui.label(style::hint(&if f.q.is_empty() { String::new() } else if n == 0 { "Keine Treffer".into() } else { format!("{} von {n}", f.idx + 1) }));
        for (d, tip) in [(-1, "Vorheriger Treffer (Umschalt+Enter)"), (1, "Nächster Treffer (Enter)")] {
            let r = tb(ui, 30.0, false, true, tip, |pt, r, col| {
                let m = r.center();
                let s = d as f32 * 3.5;
                pt.add(egui::Shape::convex_polygon(vec![Pos2::new(m.x - 5.0, m.y - s), Pos2::new(m.x + 5.0, m.y - s), Pos2::new(m.x, m.y + s)], col, Stroke::NONE));
            });
            if r.clicked() {
                step = d;
            }
        }
        if !f.replace && ui.button("Ersetzen …").clicked() {
            f.replace = true;
        }
        if f.replace {
            ui.add(egui::TextEdit::singleline(&mut f.rep).hint_text("Ersetzen durch").desired_width(180.0));
            if ui.button("Ersetzen").clicked() {
                rep_one = true;
            }
            if ui.button("Alle ersetzen").clicked() {
                rep_all = true;
            }
        }
        if style::icon_btn(ui, &mut sh.icons, ic::CLOSE, pal().dim, false, false, 30.0, "Schließen (Esc)").clicked() || ui.input(|i| i.key_pressed(egui::Key::Escape)) {
            close = true;
        }
    });
    if step != 0 {
        w.find_step(step, now);
    }
    if rep_one {
        w.replace_one(now);
    }
    if rep_all {
        let n = w.replace_all(now);
        sh.toast(&format!("{n} ersetzt"));
    }
    if close {
        w.find = None;
        w.focus_req = true;
    }
}

fn outline(w: &mut Writer, ui: &mut egui::Ui, sh: &mut Shared) {
    let p = pal();
    ui.label(egui::RichText::new("Gliederung").size(fs::SECTION).strong());
    ui.add_space(sp::S);
    let hs = w.headings();
    if hs.is_empty() {
        ui.label(style::hint("Überschriften erscheinen hier. Wähle oben „Überschrift 1“ oder drücke Strg+Alt+1."));
        return;
    }
    let cur_p = w.cur.p;
    let active = hs.iter().rposition(|h| h.0 <= cur_p);
    egui::ScrollArea::vertical().auto_shrink([false, false]).show(ui, |ui| {
        for (n, (pi, lvl, text)) in hs.iter().enumerate() {
            let ind = (*lvl as f32).max(1.0) * 12.0 - 12.0;
            let (r, resp) = ui.allocate_exact_size(Vec2::new(ui.available_width(), 28.0), Sense::click());
            if active == Some(n) {
                ui.painter().rect_filled(r, CornerRadius::same(6), c(p.active));
            } else if resp.hovered() {
                ui.painter().rect_filled(r, CornerRadius::same(6), c(p.hover));
            }
            let t = crate::ui::elide(text, 13.0, r.width() - ind - 16.0, ui);
            let font = if *lvl <= 1 { FontId::proportional(13.5) } else { FontId::proportional(13.0) };
            ui.painter().text(Pos2::new(r.left() + 8.0 + ind, r.center().y), Align2::LEFT_CENTER, t, font, c(if *lvl <= 1 { p.text } else { p.dim }));
            if resp.clicked() {
                w.jump_to_para(*pi, sh.now);
                w.focus_req = true;
            }
        }
    });
}

// ---------------------------------------------------------------- Rechtsklick, Befehlssuche, Fenster

pub fn context_menu(w: &mut Writer, ui: &mut egui::Ui, sh: &mut Shared) {
    ui.set_min_width(230.0);
    if let Some(p) = w.sel_image() {
        menu_list(w, ui, sh, &[Some(Cmd::Align(Align::Left)), Some(Cmd::Align(Align::Center)), Some(Cmd::Align(Align::Right)), None, Some(Cmd::Paste)]);
        style::separator(ui);
        if row(ui, "Bild löschen", "Entf", true, false) {
            w.remove_para(p, sh.now);
            ui.close();
        }
        return;
    }
    let mut list = vec![Some(Cmd::Cut), Some(Cmd::Copy), Some(Cmd::Paste), None, Some(Cmd::Bold), Some(Cmd::Italic), Some(Cmd::Underline), Some(Cmd::Link), None, Some(Cmd::ClearFmt)];
    if !w.has_sel() {
        list.extend([None, Some(Cmd::SelectAll), Some(Cmd::PageBreak)]);
    }
    menu_list(w, ui, sh, &list);
}

/// Strg+Umschalt+P: alle Befehle mit Suche.
fn palette(w: &mut Writer, ctx: &egui::Context, sh: &mut Shared) {
    let Some((mut q, mut sel)) = w.palette.clone() else { return };
    let p = pal();
    let ql = q.to_lowercase();
    let word = !crate::locks::locked(owndraw_core::features::OFFICE_WORD);
    let hits: Vec<_> = super::cmds().filter(|e| word || !matches!(e.0, Cmd::ExportDocx | Cmd::ImportDocx)).filter(|e| ql.split_whitespace().all(|t| e.1.to_lowercase().contains(t) || e.3.contains(t))).collect();
    let mut run = None;
    let mut close = false;
    let m = egui::Modal::new(egui::Id::unique("w-palette")).frame(style::card().inner_margin(Margin::same(12))).show(ctx, |ui| {
        ui.set_width(380.0_f32.min(ctx.content_rect().width() - 40.0));
        let te = ui.add(egui::TextEdit::singleline(&mut q).hint_text("Was möchtest du tun? z. B. „Seitenzahlen“").desired_width(f32::INFINITY).font(FontId::proportional(15.0)));
        te.request_focus();
        if ui.input(|i| i.key_pressed(egui::Key::ArrowDown)) {
            sel = (sel + 1).min(hits.len().saturating_sub(1));
        }
        if ui.input(|i| i.key_pressed(egui::Key::ArrowUp)) {
            sel = sel.saturating_sub(1);
        }
        if te.changed() {
            sel = 0;
        }
        if ui.input(|i| i.key_pressed(egui::Key::Enter)) {
            run = hits.get(sel).map(|e| e.0);
        }
        ui.add_space(sp::S);
        egui::ScrollArea::vertical().max_height(340.0).show(ui, |ui| {
            for (i, e) in hits.iter().enumerate() {
                let (r, resp) = ui.allocate_exact_size(Vec2::new(ui.available_width(), 30.0), Sense::click());
                if i == sel || resp.hovered() {
                    ui.painter().rect_filled(r, CornerRadius::same(6), c(if i == sel { p.active } else { p.hover }));
                }
                ui.painter().text(Pos2::new(r.left() + 10.0, r.center().y), Align2::LEFT_CENTER, e.1, FontId::proportional(14.0), c(p.text));
                ui.painter().text(Pos2::new(r.right() - 10.0, r.center().y), Align2::RIGHT_CENTER, e.2, FontId::proportional(fs::CAPTION), c(p.dim));
                if resp.clicked() {
                    run = Some(e.0);
                }
                if i == sel && (ui.input(|i| i.key_pressed(egui::Key::ArrowDown) || i.key_pressed(egui::Key::ArrowUp))) {
                    resp.scroll_to_me(None);
                }
            }
            if hits.is_empty() {
                ui.label(style::hint("Nichts gefunden"));
            }
        });
    });
    if m.should_close() {
        close = true;
    }
    if let Some(c) = run {
        w.palette = None;
        w.run(c, sh, ctx);
    } else if close {
        w.palette = None;
        w.focus_req = true;
    } else {
        w.palette = Some((q, sel));
    }
}

fn dialogs(w: &mut Writer, ctx: &egui::Context, sh: &mut Shared) {
    let Some(mut d) = w.dlg.clone() else { return };
    let mut close = false;
    let now = sh.now;
    let maxw = (ctx.content_rect().width() - 40.0).min(420.0);
    let title = match &d {
        Dlg::Link(..) => "Link",
        Dlg::Special => "Sonderzeichen",
        Dlg::HeaderFooter(..) => "Kopf- und Fußzeile",
        Dlg::PageSetup => "Seite einrichten",
        Dlg::WordCount => "Wörter zählen",
        Dlg::Shortcuts => "Tastenkürzel",
    };
    let m = egui::Modal::new(egui::Id::unique("w-dlg")).frame(style::card().inner_margin(Margin::same(sp::XL as i8))).show(ctx, |ui| {
        ui.set_width(maxw);
        ui.label(style::title(title));
        ui.add_space(sp::M);
        match &mut d {
            Dlg::Link(url, text) => {
                ui.label("Adresse");
                let te = ui.add(egui::TextEdit::singleline(url).hint_text("https://…").desired_width(f32::INFINITY));
                if !te.has_focus() && url.is_empty() {
                    te.request_focus();
                }
                if !w.has_sel() {
                    ui.add_space(sp::S);
                    ui.label("Angezeigter Text");
                    ui.add(egui::TextEdit::singleline(text).hint_text("leer = Adresse").desired_width(f32::INFINITY));
                }
                ui.add_space(sp::M);
                ui.horizontal(|ui| {
                    if style::primary_btn(ui, "Übernehmen", 36.0).clicked() || ui.input(|i| i.key_pressed(egui::Key::Enter)) {
                        w.set_link(url.clone(), text.clone(), now);
                        close = true;
                    }
                    if w.cur_style().link.is_some() && style::plain_btn(ui, "Link entfernen", 36.0).clicked() {
                        w.set_link(String::new(), String::new(), now);
                        close = true;
                    }
                    if style::plain_btn(ui, "Abbrechen", 36.0).clicked() {
                        close = true;
                    }
                });
                ui.label(style::hint("Tipp: Strg+Klick auf einen Link öffnet ihn."));
            }
            Dlg::Special => {
                ui.horizontal_wrapped(|ui| {
                    for ch in SPECIAL.chars() {
                        if ui.add(egui::Button::new(egui::RichText::new(ch.to_string()).size(18.0)).min_size(Vec2::splat(36.0))).clicked() {
                            w.insert(&ch.to_string(), now);
                            w.last = (super::Edit::Other, now);
                        }
                    }
                });
                ui.add_space(sp::M);
                if style::plain_btn(ui, "Fertig", 36.0).clicked() {
                    close = true;
                }
            }
            Dlg::HeaderFooter(h, f) => {
                ui.label("Kopfzeile (oben auf jeder Seite)");
                ui.add(egui::TextEdit::singleline(h).desired_width(f32::INFINITY));
                ui.add_space(sp::S);
                ui.label("Fußzeile (unten, mittig)");
                ui.add(egui::TextEdit::singleline(f).desired_width(f32::INFINITY));
                ui.label(style::hint("Platzhalter: {seite} = Seitenzahl, {seiten} = Anzahl Seiten"));
                ui.add_space(sp::M);
                ui.horizontal(|ui| {
                    if style::primary_btn(ui, "Übernehmen", 36.0).clicked() {
                        w.begin(super::Edit::Other, now);
                        w.td.header = h.clone();
                        w.td.footer = f.clone();
                        w.changed(now);
                        close = true;
                    }
                    if style::plain_btn(ui, "Abbrechen", 36.0).clicked() {
                        close = true;
                    }
                });
            }
            Dlg::PageSetup => page_setup(w, ui, now, &mut close),
            Dlg::WordCount => {
                let (a, b) = w.sel();
                let sel = w.has_sel().then(|| w.td.text_range(a, b));
                let all = w.td.plain_text();
                let pages = w.layout().pages;
                let line = |ui: &mut egui::Ui, k: &str, v: String| {
                    ui.horizontal(|ui| {
                        ui.label(k);
                        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| ui.label(egui::RichText::new(v).strong()));
                    });
                };
                if let Some(s) = &sel {
                    ui.label(style::hint("Auswahl"));
                    line(ui, "Wörter", s.split_whitespace().count().to_string());
                    line(ui, "Zeichen", s.chars().filter(|c| *c != '\n').count().to_string());
                    ui.add_space(sp::S);
                    ui.label(style::hint("Ganzes Dokument"));
                }
                line(ui, "Seiten", pages.to_string());
                line(ui, "Wörter", w.td.words().to_string());
                line(ui, "Zeichen", w.td.char_count().to_string());
                line(ui, "Zeichen ohne Leerzeichen", all.chars().filter(|c| !c.is_whitespace()).count().to_string());
                line(ui, "Absätze", w.td.paras.iter().filter(|p| !p.is_empty()).count().to_string());
                ui.add_space(sp::M);
                if style::plain_btn(ui, "Schließen", 36.0).clicked() {
                    close = true;
                }
            }
            Dlg::Shortcuts => {
                egui::ScrollArea::vertical().max_height(380.0).show(ui, |ui| {
                    for e in super::cmds().filter(|e| !e.2.is_empty()) {
                        ui.horizontal(|ui| {
                            ui.label(e.1);
                            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| ui.label(style::hint(e.2)));
                        });
                    }
                    for (k, v) in [("Wort links/rechts", "Strg+←/→"), ("Anfang/Ende", "Strg+Pos1/Ende"), ("Funktion suchen", "Strg+Umschalt+P"), ("Wort markieren", "Doppelklick"), ("Absatz markieren", "Dreifachklick")] {
                        ui.horizontal(|ui| {
                            ui.label(k);
                            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| ui.label(style::hint(v)));
                        });
                    }
                });
                ui.add_space(sp::M);
                if style::plain_btn(ui, "Schließen", 36.0).clicked() {
                    close = true;
                }
            }
        }
    });
    if m.should_close() {
        close = true;
    }
    if close {
        w.dlg = None;
        w.focus_req = true;
    } else if w.dlg.is_some() {
        w.dlg = Some(d);
    }
}

const FORMATS: [(&str, f32, f32); 4] = [("A4", 595.28, 841.89), ("A5", 419.53, 595.28), ("Letter", 612.0, 792.0), ("Legal", 612.0, 1008.0)];

fn page_setup(w: &mut Writer, ui: &mut egui::Ui, now: f64, close: &mut bool) {
    let pg = w.td.page;
    let land = pg.w > pg.h;
    let (sw, sh_) = if land { (pg.h, pg.w) } else { (pg.w, pg.h) };
    let cur = FORMATS.iter().position(|f| (f.1 - sw).abs() < 1.0 && (f.2 - sh_).abs() < 1.0);
    let mut next: Option<PageSpec> = None;
    ui.label("Format");
    ui.horizontal_wrapped(|ui| {
        for (i, f) in FORMATS.iter().enumerate() {
            if style::chip(ui, f.0, cur == Some(i), 32.0).clicked() {
                let (a, b) = if land { (f.2, f.1) } else { (f.1, f.2) };
                next = Some(PageSpec { w: a, h: b, ..pg });
            }
        }
    });
    ui.add_space(sp::S);
    ui.label("Ausrichtung");
    ui.horizontal(|ui| {
        if style::chip(ui, "Hochformat", !land, 32.0).clicked() && land {
            next = Some(PageSpec { w: pg.h, h: pg.w, ..pg });
        }
        if style::chip(ui, "Querformat", land, 32.0).clicked() && !land {
            next = Some(PageSpec { w: pg.h, h: pg.w, ..pg });
        }
    });
    ui.add_space(sp::S);
    ui.label("Ränder");
    let mm = |pt: f32| (pt / 72.0 * 25.4).round() as i32;
    let cur_m = mm(pg.ml);
    ui.horizontal_wrapped(|ui| {
        for (name, v) in [("Schmal", 12.7_f32), ("Normal", 25.0), ("Breit", 38.0)] {
            let pt = v / 25.4 * 72.0;
            if style::chip(ui, &format!("{name} ({} mm)", v.round() as i32), cur_m == mm(pt), 32.0).clicked() {
                next = Some(PageSpec { ml: pt, mr: pt, mt: pt, mb: pt, ..next.unwrap_or(pg) });
            }
        }
    });
    if let Some(n) = next {
        w.begin(super::Edit::Other, now);
        w.td.page = n;
        w.changed(now);
    }
    ui.add_space(sp::M);
    if style::plain_btn(ui, "Fertig", 36.0).clicked() {
        *close = true;
    }
}
