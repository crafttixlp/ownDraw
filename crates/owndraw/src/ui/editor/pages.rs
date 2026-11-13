//! Seitenmodus im Editor: Seitenleiste (Miniaturen bzw. nummerierte Liste), Navigation und die dünnen
//! Schnittstellenfunktionen zu den Seiten-Methoden des Boards.
//!
//! Die Seiten-Operationen (löschen, verschieben, einfügen mit Verschieben der Inhalte) baut WP2 im Kern; die
//! UI ruft sie ausschließlich über die Funktionen dieser Datei auf (`add_page`, `delete_page`, `move_page`,
//! `set_bg`) über die Seiten-Methoden des Boards.

use super::{Editor, Lay, DOCK_PAGES, M};
use crate::app::Shared;
use crate::layout::FormFactor;
use crate::settings::BarPos;
use crate::style::{self, c};
use crate::ui::dialogs::{ConfirmAction, Dialog};
use egui::{Align2, Order, Pos2, Sense, TextureHandle, Ui, Vec2};
use owndraw_core::board::Board;
use owndraw_core::geom::RectF;
use owndraw_core::pages::{BoardMode, Page, PageBg, PAGE_GAP};

/// Miniaturen der Seiten (Textur je Seite), nur bei sichtbarer Seitenleiste aktualisiert.
#[derive(Default)]
pub struct PageThumbs {
    pub dirty: bool,
    last: f64,
    tex: Vec<Option<TextureHandle>>,
}

pub fn is_pages(b: &Board) -> bool {
    matches!(b.board_mode, BoardMode::Pages(_))
}

pub fn page_count(b: &Board) -> usize {
    match &b.board_mode {
        BoardMode::Pages(s) => s.pages.len(),
        BoardMode::Infinite => 0,
    }
}

/// Weltrechteck der Seite i (Vertrag aus `owndraw_core::pages`: untereinander, links bündig, Abstand `PAGE_GAP`).
pub fn page_rect(b: &Board, i: usize) -> Option<RectF> {
    let BoardMode::Pages(s) = &b.board_mode else { return None };
    let p = s.pages.get(i)?;
    let y: f32 = s.pages[..i].iter().map(|q| q.h + PAGE_GAP).sum();
    Some(RectF::new(0.0, y, p.w, y + p.h))
}

/// Seite, die in der Bildschirmmitte liegt (bzw. am nächsten ist).
pub fn current_page(b: &Board) -> usize {
    let n = page_count(b);
    if n == 0 {
        return 0;
    }
    let wy = (b.height() / 2.0 - b.ty) / b.sc;
    let mut best = (0, f32::MAX);
    for i in 0..n {
        if let Some(r) = page_rect(b, i) {
            let d = if wy < r.top { r.top - wy } else if wy > r.bottom { wy - r.bottom } else { 0.0 };
            if d < best.1 {
                best = (i, d);
            }
        }
    }
    best.0
}

/// Springt zur Seite i: Seitenbreite einpassen, Seitenanfang unter der oberen Leiste.
pub fn goto_page(b: &mut Board, i: usize) {
    let Some(r) = page_rect(b, i) else { return };
    let w = b.width().max(1.0);
    let sc = ((w - 24.0) / r.width()).clamp(0.2, 1.5);
    b.sc = sc;
    b.tx = (w - r.width() * sc) / 2.0;
    b.ty = 72.0 - r.top * sc;
    b.refresh();
}

/// Beim Öffnen eines Seiten-Dokuments ohne gespeicherte Ansicht: erste Seite einpassen.
pub fn initial_view(b: &mut Board) {
    if is_pages(b) && b.tx == 0.0 && b.ty == 0.0 && b.sc == 1.0 {
        goto_page(b, 0);
    }
}

pub fn current_bg(b: &Board) -> Option<PageBg> {
    let BoardMode::Pages(s) = &b.board_mode else { return None };
    s.pages.get(current_page(b)).map(|p| p.bg)
}

/// Hintergrund der aktuellen Seite (und Vorgabe für neue Seiten) setzen (mit Undo).
pub fn set_bg(b: &mut Board, bg: PageBg) {
    let cur = current_page(b);
    b.set_page_bg(Some(cur), bg);
    if let BoardMode::Pages(s) = &mut b.board_mode {
        s.bg = bg;
    }
}

/// Neue Seite nach der aktuellen einfügen (mit Undo) und dorthin springen.
pub fn add_page(b: &mut Board) -> bool {
    if !is_pages(b) {
        return false;
    }
    let cur = current_page(b);
    match b.add_page(Some(cur)) {
        Some(n) => {
            goto_page(b, n);
            true
        }
        None => false,
    }
}

/// Seite löschen (mit Inhalt, Undo möglich). Die letzte Seite bleibt.
pub fn delete_page(b: &mut Board, i: usize) -> bool {
    b.remove_page(i)
}

/// Seite verschieben (Inhalte wandern mit, Undo möglich).
pub fn move_page(b: &mut Board, from: usize, to: usize) -> bool {
    b.move_page(from, to)
}

impl PageThumbs {
    fn refresh(&mut self, ctx: &egui::Context, b: &Board, now: f64, width_pt: f32) {
        let n = page_count(b);
        if self.tex.len() == n && !self.dirty {
            return;
        }
        if self.tex.len() == n && now - self.last < 1.2 {
            ctx.request_repaint_after(std::time::Duration::from_millis(400));
            return;
        }
        self.last = now;
        self.dirty = false;
        self.tex.resize(n, None);
        let ppp = ctx.pixels_per_point();
        for i in 0..n.min(80) {
            let Some(r) = page_rect(b, i) else { continue };
            let k = width_pt * ppp / r.width();
            if let Some(pm) = b.render_composite(&r, k, true) {
                let img = egui::ColorImage::from_rgba_premultiplied([pm.width() as usize, pm.height() as usize], pm.data());
                match self.tex[i].as_mut() {
                    Some(t) => t.set(img, egui::TextureOptions::LINEAR),
                    None => self.tex[i] = Some(ctx.load_texture(format!("page-{i}"), img, egui::TextureOptions::LINEAR)),
                }
            }
        }
    }
}

enum Act {
    Goto(usize),
    Add,
    Delete(usize),
    Move(usize, usize),
    Close,
}

pub(super) fn show(ed: &mut Editor, ctx: &egui::Context, sh: &mut Shared, lay: &Lay) {
    if !lay.pages_mode {
        return;
    }
    let canvas = lay.canvas;
    let mut act: Option<Act> = None;
    let left = canvas.left() + M + if lay.tb == BarPos::Left { lay.thick + M } else { 0.0 };
    match lay.env.ff {
        FormFactor::Expanded => {
            if let Some(dock) = lay.dock_pages {
                let r = Editor::float(ctx, "pages-dock", dock.min, Align2::LEFT_TOP, dock, Order::Middle, |ui| {
                    let p = style::pal();
                    egui::Frame::new().fill(c(p.card)).stroke(egui::Stroke::new(1.0, c(p.line))).inner_margin(egui::Margin::same(10)).show(ui, |ui| {
                        ui.set_width(DOCK_PAGES - 22.0);
                        ui.set_min_height(dock.height() - 22.0);
                        header(ui, sh, &mut act);
                        list(ui, ed, sh, ctx, true, dock.height() - 120.0, &mut act);
                    });
                });
                ed.st.popup_rects.push(r);
            }
        }
        FormFactor::Medium => {
            if ed.st.pages_open == Some(true) {
                let max_h = (canvas.bottom() - lay.y_below - M - 70.0).max(140.0);
                let r = Editor::float(ctx, "pages-float", Pos2::new(left, lay.y_below), Align2::LEFT_TOP, canvas, Order::Middle, |ui| {
                    style::pill(20).show(ui, |ui| {
                        egui::Frame::new().inner_margin(egui::Margin::same(8)).show(ui, |ui| {
                            ui.set_width(150.0);
                            header(ui, sh, &mut act);
                            list(ui, ed, sh, ctx, true, max_h, &mut act);
                        });
                    });
                });
                ed.st.popup_rects.push(r);
            }
        }
        FormFactor::Compact => {
            let n = page_count(&ed.board);
            let cur = current_page(&ed.board);
            let btn = sh.env.btn;
            let p = style::pal();
            let r = Editor::float(ctx, "pages-chip", Pos2::new(left, lay.y_below), Align2::LEFT_TOP, canvas, Order::Middle, |ui| {
                style::pill(26).show(ui, |ui| {
                    ui.horizontal(|ui| {
                        ui.spacing_mut().item_spacing.x = 0.0;
                        if cur > 0 && ui.add(egui::Button::new("‹").fill(egui::Color32::TRANSPARENT).min_size(Vec2::splat(btn - 6.0))).clicked() {
                            act = Some(Act::Goto(cur - 1));
                        }
                        let t = egui::RichText::new(format!("Seite {} / {}", cur + 1, n)).size(14.0).color(c(p.text));
                        if ui.add(egui::Button::new(t).fill(egui::Color32::TRANSPARENT).min_size(Vec2::new(0.0, btn - 6.0))).on_hover_text("Seitenliste").clicked() {
                            ed.st.pages_open = Some(true);
                        }
                        if cur + 1 < n && ui.add(egui::Button::new("›").fill(egui::Color32::TRANSPARENT).min_size(Vec2::splat(btn - 6.0))).clicked() {
                            act = Some(Act::Goto(cur + 1));
                        }
                    });
                });
            });
            ed.st.popup_rects.push(r);
            if ed.st.pages_open == Some(true) {
                let r = Editor::float(ctx, "pages-full", canvas.min, Align2::LEFT_TOP, canvas, Order::Foreground, |ui| {
                    egui::Frame::new().fill(c(p.bg)).inner_margin(egui::Margin::same(12)).show(ui, |ui| {
                        ui.set_width(canvas.width() - 24.0);
                        ui.set_min_height(canvas.height() - 24.0);
                        header(ui, sh, &mut act);
                        list(ui, ed, sh, ctx, false, canvas.height() - 150.0, &mut act);
                    });
                });
                ed.st.popup_rects.push(r);
            }
        }
    }
    match act {
        Some(Act::Goto(i)) => {
            goto_page(&mut ed.board, i);
            if lay.env.ff == FormFactor::Compact {
                ed.st.pages_open = Some(false);
            }
        }
        Some(Act::Add) => {
            if !add_page(&mut ed.board) {
                sh.toast("Seite hinzufügen nicht möglich");
            }
        }
        Some(Act::Delete(i)) => {
            if page_count(&ed.board) <= 1 {
                sh.toast("Mindestens eine Seite bleibt bestehen");
            } else {
                sh.dialog = Some(Dialog::Confirm { title: "Seite löschen", text: format!("Seite {} mit allem Inhalt löschen?", i + 1), ok: "Löschen", action: ConfirmAction::DeletePage(i) });
            }
        }
        Some(Act::Move(a, b)) => {
            if move_page(&mut ed.board, a, b) {
                goto_page(&mut ed.board, b);
            } else {
                sh.toast("Seiten verschieben: folgt mit dem Seitenmodus (WP2)");
            }
        }
        Some(Act::Close) => ed.st.pages_open = Some(false),
        None => {}
    }
}

fn header(ui: &mut Ui, sh: &Shared, act: &mut Option<Act>) {
    ui.horizontal(|ui| {
        ui.label(egui::RichText::new("Seiten").size(16.0).strong());
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            if sh.env.ff != FormFactor::Expanded && style::plain_btn(ui, "Schließen", sh.env.btn - 8.0).clicked() {
                *act = Some(Act::Close);
            }
        });
    });
    ui.add_space(2.0);
}

fn list(ui: &mut Ui, ed: &mut Editor, sh: &mut Shared, ctx: &egui::Context, thumbs: bool, max_h: f32, act: &mut Option<Act>) {
    let p = style::pal();
    let n = page_count(&ed.board);
    let cur = current_page(&ed.board);
    let btn = sh.env.btn;
    let tw = (ui.available_width() - 8.0).clamp(80.0, 160.0);
    if thumbs {
        ed.page_thumbs.refresh(ctx, &ed.board, sh.now, tw);
    }
    egui::ScrollArea::vertical().max_height(max_h.max(100.0)).auto_shrink([false, true]).show(ui, |ui| {
        for i in 0..n {
            let on = i == cur;
            let fr = egui::Frame::new().fill(if on { c(p.active) } else { egui::Color32::TRANSPARENT }).corner_radius(10).inner_margin(egui::Margin::same(4));
            fr.show(ui, |ui| {
                if thumbs {
                    let (w, h) = page_rect(&ed.board, i).map(|r| (r.width(), r.height())).unwrap_or((595.0, 842.0));
                    let (rect, resp) = ui.allocate_exact_size(Vec2::new(tw, tw * h / w), Sense::click());
                    ui.painter().rect_filled(rect, 3.0, c(0xFFFBFBF8));
                    if let Some(Some(t)) = ed.page_thumbs.tex.get(i) {
                        ui.painter().image(t.id(), rect, egui::Rect::from_min_max(Pos2::ZERO, Pos2::new(1.0, 1.0)), egui::Color32::WHITE);
                    }
                    ui.painter().rect_stroke(rect, 3.0, egui::Stroke::new(1.0, c(if on { p.accent } else { p.line })), egui::StrokeKind::Inside);
                    ui.label(egui::RichText::new(format!("{}", i + 1)).size(12.0).color(c(p.dim)));
                    if resp.clicked() {
                        *act = Some(Act::Goto(i));
                    }
                    if on {
                        row_actions(ui, sh, i, n, btn, act);
                    }
                } else {
                    ui.horizontal(|ui| {
                        ui.spacing_mut().item_spacing.x = 2.0;
                        let t = egui::RichText::new(format!("Seite {}", i + 1)).size(15.0).color(c(p.text));
                        let w = ui.available_width() - 3.0 * btn - 8.0;
                        if ui.add(egui::Button::new(t).fill(egui::Color32::TRANSPARENT).min_size(Vec2::new(w.max(60.0), btn))).clicked() {
                            *act = Some(Act::Goto(i));
                        }
                        row_actions(ui, sh, i, n, btn, act);
                    });
                }
            });
        }
    });
    ui.add_space(6.0);
    if style::accent_btn(ui, "+ Seite", btn - 6.0).clicked() {
        *act = Some(Act::Add);
    }
}

fn row_actions(ui: &mut Ui, sh: &mut Shared, i: usize, n: usize, btn: f32, act: &mut Option<Act>) {
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = 2.0;
        let small = Vec2::splat(btn - 6.0);
        if i > 0 && ui.add(egui::Button::new("↑").min_size(small)).on_hover_text("Nach oben").clicked() {
            *act = Some(Act::Move(i, i - 1));
        }
        if i + 1 < n && ui.add(egui::Button::new("↓").min_size(small)).on_hover_text("Nach unten").clicked() {
            *act = Some(Act::Move(i, i + 1));
        }
        if style::icon_btn(ui, &mut sh.icons, owndraw_core::icons::TRASH, style::pal().danger, false, false, small.x, "Seite löschen").clicked() {
            *act = Some(Act::Delete(i));
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn seiten_geometrie_und_navigation() {
        let mut b = Board::new(1.0);
        b.board_mode = crate::session::NewParams::default().mode();
        assert!(add_page(&mut b));
        assert_eq!(page_count(&b), 2);
        let r1 = page_rect(&b, 1).unwrap();
        assert_eq!(r1.top, 842.0 + PAGE_GAP);
        b.set_size(800.0, 600.0, 1.0);
        goto_page(&mut b, 1);
        assert_eq!(current_page(&b), 1);
        goto_page(&mut b, 0);
        assert_eq!(current_page(&b), 0);
        assert!(delete_page(&mut b, 0));
        assert_eq!(page_count(&b), 1);
        assert!(!delete_page(&mut b, 0));
    }
}
