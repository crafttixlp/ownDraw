//! Werkzeuge Verbinder, Tabelle und Kommentar (gemeinsamer Zustand, Tippen erkennen, Andock-Ziel hervorheben)
//! sowie Gruppieren, Ausrichten und Verteilen der Auswahl (Auswahlleiste, Kontextmenü, Tastenkürzel).
//!
//! Tabelle und Kommentar setzen das Board-Werkzeug [`XT`], bei dem der Kern nichts zeichnet; ein Tippen wird hier
//! erkannt (Maus und Finger), Ziehen verschiebt die Ansicht. Der Verbinder ist das Formwerkzeug mit Linie/Pfeil
//! und `Board::dock`.

use super::{crop, keys, pins, table, tools, Editor, Lay, Tool};
use crate::app::Shared;
use crate::style::{self, c, icon_btn};
use egui::{Pos2, Rect, Stroke, TouchPhase, Ui, Vec2};
use owndraw_core::board::{self, Align};
use owndraw_core::icons as ic;
use owndraw_core::item::{self, Item};

/// Board-Werkzeug, solange Tabelle oder Kommentar gewählt ist (der Kern reagiert darauf nicht).
pub(super) const XT: i32 = 101;
/// Kennung der Maus in der Tipp-Erkennung (Finger haben ihre Touch-id).
pub(super) const MOUSE: u64 = u64::MAX;
/// Höchster Weg (Punkte) für ein Tippen.
const TAP: f32 = 10.0;
/// Stile des Verbinders (Index = `Xs::conn_style`).
pub(super) const CONN_STYLES: [&str; 3] = ["Pfeil", "Linie", "Doppelpfeil"];

/// Laufender Druck (Maus oder ein Finger).
pub(super) struct Press {
    id: u64,
    start: Pos2,
    t0: f64,
    moved: bool,
    /// Nicht an das Board weitergeben (Tabelle/Kommentar setzen, Kommentar öffnen).
    swallow: bool,
    /// Auswahl beim Drücken (zweites Tippen in eine ausgewählte Tabelle bearbeitet die Zelle).
    pre_sel: Vec<u64>,
    /// Gezogener Endpunkt einer ausgewählten Linie (id, Ende).
    end_drag: Option<(u64, usize)>,
}

/// Zustand der Werkzeuge dieses Moduls.
#[derive(Default)]
pub(super) struct Xs {
    /// Gewähltes Werkzeug Verbinder, Tabelle oder Kommentar.
    pub tool: Option<Tool>,
    /// Form und Pfeilstil des Formwerkzeugs, solange der Verbinder sie überschreibt.
    pub saved: Option<(i32, i32)>,
    /// 0 Pfeil, 1 Linie, 2 Doppelpfeil.
    pub conn_style: i32,
    loaded: bool,
    pub(super) press: Option<Press>,
    last_tap: Option<(f64, Pos2)>,
    pub table: table::TableUi,
    pub crop: Option<crop::Crop>,
    pub pin: Option<pins::PinWin>,
}

impl Editor {
    /// Gewähltes Werkzeug dieses Moduls, solange das Board-Werkzeug dazu passt.
    pub(super) fn xtool(&self) -> Option<Tool> {
        let t = self.xs.tool?;
        let want = if t == Tool::Connector { board::SHAPE } else { XT };
        (self.board.tool == want).then_some(t)
    }

    pub(super) fn select_xtool(&mut self, t: Tool) {
        self.leave_xtool();
        if t == Tool::Connector {
            self.xs.saved = Some((self.board.shape_kind, self.board.arrow_style));
            self.apply_conn_style();
            self.board.set_tool(board::SHAPE);
            self.board.dock = true;
        } else {
            self.board.set_tool(XT);
        }
        self.xs.tool = Some(t);
        self.board.invalidate();
    }

    /// Verlässt Verbinder/Tabelle/Kommentar (Formwerkzeug bekommt Form und Pfeilstil zurück).
    pub(super) fn leave_xtool(&mut self) {
        if let Some((k, a)) = self.xs.saved.take() {
            self.board.shape_kind = k;
            self.board.arrow_style = a;
        }
        self.xs.tool = None;
        self.board.dock = false;
    }

    pub(super) fn apply_conn_style(&mut self) {
        let (k, a) = match self.xs.conn_style {
            1 => (2, self.board.arrow_style),
            2 => (3, 4),
            _ => (3, 1),
        };
        self.board.shape_kind = k;
        self.board.arrow_style = a;
    }

    /// Darf das Board geändert werden (Live: Rolle „Nur ansehen“)?
    pub(super) fn x_can_write(&self) -> bool {
        {
            if let Some(l) = self.live.as_ref().filter(|l| l.is_live()) {
                return l.can_write();
            }
            if let Some(id) = self.file().and_then(crate::net::srv_id) {
                if crate::net::board(id).is_some_and(|b| b.role == "view") {
                    return false;
                }
            }
        }
        true
    }

    /// Name für Kommentare: angemeldetes Konto, sonst „Ich“.
    pub(super) fn x_author(&self) -> String {
        if let Some(me) = crate::net::with(|n| n.me.clone()) {
            let n = if me.name.trim().is_empty() { me.email } else { me.name };
            if !n.trim().is_empty() {
                return n;
            }
        }
        "Ich".into()
    }

    pub(super) fn x_notice(&mut self, s: &str) {
        self.board.events.push(board::Event::Notice(s.to_string()));
    }

    /// Bildschirmpunkt (relativ zur Zeichenfläche) eines Weltpunkts, absolut.
    pub(super) fn w2s(&self, x: f32, y: f32) -> Pos2 {
        self.canvas.min + Vec2::new(x * self.board.sc + self.board.tx, y * self.board.sc + self.board.ty)
    }

    // ---------------------------------------------------------------- Tippen

    /// Drücken (Maus oder erster Finger) bei `p` (relativ). true = nicht an das Board weitergeben.
    pub(super) fn x_down(&mut self, id: u64, p: Pos2, now: f64) -> bool {
        let (wx, wy) = self.board.screen_to_world(p.x, p.y);
        let pin = self.board.pin_at(wx, wy);
        if self.xs.pin.as_ref().is_some_and(|w| Some(w.id) != pin) {
            self.close_pin();
        }
        let t = self.xtool();
        let swallow = self.xs.crop.is_some() || matches!(t, Some(Tool::Table | Tool::Pin)) || (pin.is_some() && self.board.tool != board::SELECT);
        let end_drag = if self.board.tool == board::SELECT { self.end_near(p) } else { None };
        self.xs.press = Some(Press { id, start: p, t0: now, moved: false, swallow, pre_sel: self.board.sel.clone(), end_drag });
        swallow
    }

    pub(super) fn x_move(&mut self, id: u64, p: Pos2) {
        if let Some(pr) = self.xs.press.as_mut().filter(|pr| pr.id == id) {
            if pr.start.distance(p) > TAP {
                pr.moved = true;
            }
        }
    }

    /// Loslassen; `end` = false bei Abbruch.
    pub(super) fn x_up(&mut self, id: u64, p: Pos2, end: bool, now: f64) {
        if !self.xs.press.as_ref().is_some_and(|pr| pr.id == id) {
            return;
        }
        let Some(pr) = self.xs.press.take() else { return };
        if !end || pr.moved || pr.start.distance(p) > TAP {
            return;
        }
        self.x_tap(p, &pr.pre_sel, now);
    }

    /// Finger: Tippen erkennen. true = Ereignis nicht an das Board weitergeben.
    pub(super) fn x_touch(&mut self, id: u64, phase: TouchPhase, p: Pos2, now: f64) -> bool {
        let mine = |s: &Self| s.xs.press.as_ref().is_some_and(|pr| pr.id == id && pr.swallow);
        match phase {
            TouchPhase::Start => {
                // weiterer Finger: kein Tippen mehr
                if let Some(pr) = self.xs.press.as_mut().filter(|pr| pr.swallow && pr.id != MOUSE && now - pr.t0 < 30.0) {
                    pr.moved = true;
                    return false;
                }
                if self.touches.is_empty() {
                    return self.x_down(id, p, now);
                }
                if let Some(pr) = self.xs.press.as_mut() {
                    pr.moved = true;
                }
                false
            }
            TouchPhase::Move => {
                let sw = mine(self);
                self.x_move(id, p);
                sw
            }
            TouchPhase::End | TouchPhase::Cancel => {
                let sw = mine(self);
                self.x_up(id, p, phase == TouchPhase::End, now);
                sw
            }
        }
    }

    fn x_tap(&mut self, p: Pos2, pre_sel: &[u64], now: f64) {
        let (wx, wy) = self.board.screen_to_world(p.x, p.y);
        let dbl = self.xs.last_tap.is_some_and(|(t, q)| now - t < 0.45 && q.distance(p) < 24.0);
        self.xs.last_tap = Some((now, p));
        // Kommentar öffnen (mit jedem Werkzeug)
        if let Some(id) = self.board.pin_at(wx, wy) {
            self.open_pin(id);
            return;
        }
        match self.xtool() {
            Some(Tool::Table) => return self.place_table(wx, wy),
            Some(Tool::Pin) => return self.new_pin(wx, wy),
            _ => {}
        }
        if self.board.tool != board::SELECT {
            return;
        }
        // Tabelle: Zelle merken; Tippen in die schon ausgewählte Tabelle bearbeitet die Zelle
        if let Some((id, r, cc)) = table::cell_at(&self.board, wx, wy) {
            self.xs.table.cell = Some((id, r, cc));
            if pre_sel == [id] && self.board.sel == [id] {
                self.edit_cell(id, r, cc);
            }
            return;
        }
        // Doppeltipp auf ein Bild: zuschneiden
        if dbl {
            let img = self.board.sel_items().first().filter(|it| self.board.sel.len() == 1 && it.is_img() && it.contains(wx, wy)).map(|it| it.id);
            if let Some(id) = img {
                self.start_crop(id);
            }
        }
    }

    /// Endpunkt (id, Ende) einer einzeln ausgewählten Linie nahe `p` (relativ).
    fn end_near(&self, p: Pos2) -> Option<(u64, usize)> {
        let it = single_line(&self.board)?;
        let q = &it.as_shape()?.q;
        (0..2).find(|e| {
            let (x, y) = (q[e * 2] * self.board.sc + self.board.tx, q[e * 2 + 1] * self.board.sc + self.board.ty);
            Pos2::new(x, y).distance(p) < 22.0
        })
        .map(|e| (it.id, e))
    }

    // ---------------------------------------------------------------- je Frame

    /// Werkzeugzustand abgleichen, Hervorhebungen zeichnen, Zuschnitt-Modus und Kommentarfenster.
    pub(super) fn x_frame(&mut self, ctx: &egui::Context, sh: &mut Shared, lay: &Lay, layer: egui::LayerId) {
        if !self.xs.loaded {
            self.xs.loaded = true;
            self.xs.conn_style = sh.settings.extra.get("conn").and_then(|v| v.as_i64()).unwrap_or(0).clamp(0, 2) as i32;
        }
        // Werkzeug anderswo gewechselt (z. B. Taste T): Verbinder/Tabelle/Kommentar verlassen
        if self.xs.tool.is_some() && self.xtool().is_none() && self.board.tool != board::SELECT {
            self.leave_xtool();
        }
        if self.xs.tool.is_none() && self.board.tool == XT {
            self.board.set_tool(board::SELECT);
        }
        // Andocken: Verbinder-Werkzeug; mit der Auswahl Endpunkte ziehen, solange der Verbinder eingeschaltet ist
        self.board.dock = self.xtool() == Some(Tool::Connector) || (self.board.tool == board::SELECT && tools::enabled(&sh.settings, "connector"));
        let pt = ctx.layer_painter(layer).with_clip_rect(lay.canvas);
        self.paint_dock_target(ctx, &pt);
        self.paint_cell(&pt);
        self.crop_ui(ctx, sh, lay);
        self.pin_ui(ctx, sh, lay);
    }

    /// Hebt beim Zeichnen eines Verbinders bzw. beim Ziehen eines Endpunkts das Ziel hervor, an dem angedockt wird.
    fn paint_dock_target(&self, ctx: &egui::Context, pt: &egui::Painter) {
        if !self.board.dock || !self.board.busy() {
            return;
        }
        let Some(pr) = &self.xs.press else { return };
        let cur = if let Some(t) = self.touches.first() {
            t.1
        } else if let Some(p) = ctx.input(|i| i.pointer.latest_pos()) {
            (p - self.canvas.min).to_pos2()
        } else {
            return;
        };
        let mut targets: Vec<(f32, f32, Option<u64>)> = Vec::new();
        if self.board.tool == board::SHAPE && is_line_kind(self.board.shape_kind) {
            let (sx, sy) = self.board.screen_to_world(pr.start.x, pr.start.y);
            targets.push((sx, sy, None));
            let (wx, wy) = self.board.screen_to_world(cur.x, cur.y);
            targets.push((wx, wy, None));
        } else if let (board::SELECT, Some((id, _))) = (self.board.tool, pr.end_drag) {
            if pr.moved {
                let (wx, wy) = self.board.screen_to_world(cur.x, cur.y);
                targets.push((wx, wy, Some(id)));
            }
        }
        let acc = c(style::pal().accent);
        let mut seen = Vec::new();
        for (x, y, ex) in targets {
            let Some((tid, rel)) = self.board.dock_target_at(x, y, ex, None) else { continue };
            if seen.contains(&tid) {
                continue;
            }
            seen.push(tid);
            let Some(it) = self.board.items.iter().find(|i| i.id == tid) else { continue };
            let b = it.bounds();
            let r = Rect::from_min_max(self.w2s(b.left, b.top), self.w2s(b.right, b.bottom)).expand(4.0);
            pt.rect_stroke(r, 6.0, Stroke::new(2.0, acc), egui::StrokeKind::Outside);
            pt.rect_filled(r, 6.0, acc.gamma_multiply(0.08));
            let (ax, ay) = match rel {
                Some(rl) => it.rel_point(rl),
                None => (b.center_x(), b.center_y()),
            };
            pt.circle_filled(self.w2s(ax, ay), 5.0, acc);
        }
    }

    /// Verbinder der Auswahl von beiden Zielen lösen.
    fn release_connector(&mut self, id: u64) {
        let an = self.board.items.iter().find(|i| i.id == id).map(|i| i.anchors()).unwrap_or([None, None]);
        for (e, a) in an.iter().enumerate() {
            if a.is_some() {
                self.board.connect(id, e, None, None);
            }
        }
    }
}

impl Editor {
    /// Abnahme-Hilfe (demo.rs, `xtable`, `xpin`, `xsel`, `xcrop`, `xconn`).
    pub fn demo_x(&mut self, what: &str) {
        let (cx, cy) = self.board.screen_to_world(self.board.width() * 0.5, self.board.height() * 0.4);
        match what {
            "xtable" => {
                if let Some(id) = self.board.add_table(cx - 150.0, cy - 60.0, 3, 3) {
                    self.board.set_cell_text(id, 0, 0, "Teil");
                    self.board.set_cell_text(id, 0, 1, "Menge");
                    self.board.set_tool(board::SELECT);
                    self.board.select_only(id);
                    self.xs.table.cell = Some((id, 1, 1));
                }
            }
            "xpin" => {
                if let Some(id) = self.board.add_pin(cx, cy, "Anna", 1_763_300_000_000, "Bitte Maß prüfen") {
                    self.board.reply_pin(id, "Ben", 1_763_300_600_000, "Erledigt");
                    self.open_pin(id);
                }
            }
            "xsel" => {
                self.board.set_tool(board::SELECT);
                self.board.sel = self.board.items.iter().take(3).map(|i| i.id).collect();
            }
            "xcrop" => {
                if !self.board.items.iter().any(|i| i.is_img()) {
                    let (w, h) = (160u32, 100u32);
                    let rgba: Vec<u8> = (0..w * h).flat_map(|i| [(i % w * 255 / w) as u8, (i / w * 255 / h) as u8, 180, 255]).collect();
                    let _ = super::clip::insert_clip(self, crate::platform::ClipData::Image { w, h, rgba }, Some((cx, cy)));
                }
                if let Some(id) = self.board.items.iter().find(|i| i.is_img()).map(|i| i.id) {
                    self.board.set_tool(board::SELECT);
                    self.start_crop(id);
                    if let Some(c) = self.xs.crop.as_mut() {
                        c.cr = [0.15, 0.1, 0.8, 0.85];
                    }
                }
            }
            "xconn" => self.select_xtool(Tool::Connector),
            _ => {}
        }
    }
}

/// Formart des Formwerkzeugs ist Linie oder Pfeil.
fn is_line_kind(k: i32) -> bool {
    k == 2 || k == 3 || k == 4
}

/// Einzeln ausgewählte Linie bzw. Pfeil.
fn single_line(b: &board::Board) -> Option<&Item> {
    if b.sel.len() != 1 {
        return None;
    }
    b.sel_items().into_iter().next().filter(|it| it.as_shape().is_some_and(|g| item::is_linear(g.kind)))
}

/// Ist das Objekt ein angedockter Verbinder?
fn is_connector(it: &Item) -> bool {
    it.anchors() != [None, None]
}

// ---------------------------------------------------------------- Gruppieren, Ausrichten, Verteilen

/// Ausrichtungen mit Namen (Menü).
const ALIGNS: [(Align, &str); 6] =
    [(Align::Left, "Links"), (Align::CenterH, "Mittig"), (Align::Right, "Rechts"), (Align::Top, "Oben"), (Align::CenterV, "Mitte"), (Align::Bottom, "Unten")];

/// Kann die Auswahl gruppiert werden (mindestens zwei Objekte, nicht schon eine Gruppe)?
fn can_group(b: &board::Board) -> bool {
    let it = b.sel_items();
    it.len() >= 2 && !(it[0].gid != 0 && it.iter().all(|x| x.gid == it[0].gid))
}

/// Anzahl der Einheiten (Gruppen zählen als eine).
fn units(b: &board::Board) -> usize {
    let mut g: Vec<u64> = Vec::new();
    let mut n = 0;
    for it in b.sel_items() {
        if it.gid == 0 {
            n += 1;
        } else if !g.contains(&it.gid) {
            g.push(it.gid);
            n += 1;
        }
    }
    n
}

/// Tastenkürzel group/ungroup.
pub(super) fn run(ed: &mut Editor, id: &str) {
    if ed.board.tool != board::SELECT || ed.board.sel.is_empty() {
        return;
    }
    match id {
        "group" => {
            if can_group(&ed.board) {
                ed.board.group_sel();
                ed.x_notice("Gruppiert");
            }
        }
        "ungroup" => {
            if ed.board.ungroup_sel() {
                ed.x_notice("Gruppierung gelöst");
            }
        }
        _ => {}
    }
}

/// Knöpfe der Auswahlleiste: Gruppieren, Ausrichten, Verteilen, Verbinder lösen, Tabelle, Zuschneiden.
pub(super) fn sel_buttons(ui: &mut Ui, ed: &mut Editor, sh: &mut Shared, btn: f32) {
    let p = style::pal();
    let env = sh.env;
    if can_group(&ed.board) && icon_btn(ui, &mut sh.icons, ic::GROUP, p.text, false, false, btn, &env.tip("Gruppieren", &keys::label(&sh.settings, "group"))).clicked() {
        ed.board.group_sel();
    }
    if ed.board.sel_grouped() && icon_btn(ui, &mut sh.icons, ic::UNGROUP, p.text, false, false, btn, &env.tip("Gruppierung lösen", &keys::label(&sh.settings, "ungroup"))).clicked() {
        ed.board.ungroup_sel();
    }
    let n = units(&ed.board);
    if n >= 2 {
        let r = icon_btn(ui, &mut sh.icons, ic::ALIGN, p.text, false, false, btn, if n >= 3 { "Ausrichten und verteilen" } else { "Ausrichten" });
        let h = (btn - 6.0).max(32.0);
        egui::Popup::menu(&r).show(|ui| align_menu(ui, ed, n, h));
    }
    if let Some(id) = single_line(&ed.board).filter(|it| is_connector(it)).map(|it| it.id) {
        if style::chip(ui, "Lösen", false, (btn - 8.0).max(30.0)).on_hover_text("Verbinder von den Objekten lösen").clicked() {
            ed.release_connector(id);
        }
    }
    table::sel_buttons(ui, ed, sh, btn);
    let img = ed.board.sel_items().first().filter(|it| it.is_img()).map(|it| it.id);
    if let (Some(id), 1) = (img, ed.board.sel.len()) {
        if icon_btn(ui, &mut sh.icons, ic::CROP, p.text, false, false, btn, "Zuschneiden").clicked() {
            ed.start_crop(id);
        }
    }
}

/// Menü Ausrichten (ab 2) und Verteilen (ab 3 Einheiten).
fn align_menu(ui: &mut Ui, ed: &mut Editor, n: usize, h: f32) {
    style::caption(ui, "Ausrichten");
    for (a, l) in ALIGNS {
        if style::menu_row(ui, l, false, h) {
            ed.board.align_sel(a);
            ui.close();
        }
    }
    if n >= 3 {
        style::caption(ui, "Verteilen");
        for (hz, l) in [(true, "Waagerecht verteilen"), (false, "Senkrecht verteilen")] {
            if style::menu_row(ui, l, false, h) {
                ed.board.distribute_sel(hz);
                ui.close();
            }
        }
    }
}

/// Einträge im Kontextmenü der Auswahl (Rechtsklick/Lang-Tippen).
pub(super) fn context_items(ui: &mut Ui, ed: &mut Editor) {
    if ed.board.tool != board::SELECT || ed.board.sel.is_empty() {
        return;
    }
    let (grp, ungrp, n) = (can_group(&ed.board), ed.board.sel_grouped(), units(&ed.board));
    if !(grp || ungrp || n >= 2) {
        return;
    }
    ui.separator();
    if grp && ui.button("Gruppieren").clicked() {
        ed.board.group_sel();
        ui.close();
    }
    if ungrp && ui.button("Gruppierung lösen").clicked() {
        ed.board.ungroup_sel();
        ui.close();
    }
    if n >= 2 {
        ui.menu_button("Ausrichten", |ui| {
            for (a, l) in ALIGNS {
                if ui.button(l).clicked() {
                    ed.board.align_sel(a);
                    ui.close();
                }
            }
            if n >= 3 {
                ui.separator();
                if ui.button("Waagerecht verteilen").clicked() {
                    ed.board.distribute_sel(true);
                    ui.close();
                }
                if ui.button("Senkrecht verteilen").clicked() {
                    ed.board.distribute_sel(false);
                    ui.close();
                }
            }
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ui::editor::clip::tests::editor;
    use owndraw_core::item::Kind;
    use std::rc::Rc;

    fn rect_item(x: f32, y: f32) -> Item {
        Item::shape(item::POLY, vec![x, y, x + 40.0, y, x + 40.0, y + 30.0, x, y + 30.0], item::INK, 2.0)
    }

    #[test]
    fn werkzeuge_wechseln() {
        let mut ed = editor(false);
        ed.board.shape_kind = 1;
        ed.select_xtool(Tool::Connector);
        assert_eq!(ed.active_tool(), Tool::Connector);
        assert!(ed.board.dock && ed.board.tool == board::SHAPE && ed.board.shape_kind == 3);
        ed.select_xtool(Tool::Table);
        assert_eq!(ed.active_tool(), Tool::Table);
        assert_eq!(ed.board.shape_kind, 1, "Form des Formwerkzeugs zurück");
        assert!(!ed.board.dock);
        ed.leave_xtool();
        ed.board.set_tool(board::PEN);
        assert_eq!(ed.active_tool(), Tool::Pen);
    }

    #[test]
    fn tippen_setzt_tabelle_und_kommentar() {
        let mut ed = editor(false);
        ed.select_xtool(Tool::Table);
        assert!(ed.x_down(MOUSE, Pos2::new(100.0, 100.0), 0.0));
        ed.x_up(MOUSE, Pos2::new(102.0, 101.0), true, 0.1);
        assert!(matches!(ed.board.items.last().map(|i| &i.kind), Some(Kind::Table(t)) if t.rows() == 3 && t.cols() == 3));
        assert_eq!(ed.board.tool, board::SELECT);
        ed.select_xtool(Tool::Pin);
        ed.x_down(MOUSE, Pos2::new(400.0, 300.0), 1.0);
        ed.x_up(MOUSE, Pos2::new(400.0, 300.0), true, 1.1);
        let pid = ed.board.items.last().unwrap().id;
        assert!(matches!(ed.board.items.last().map(|i| &i.kind), Some(Kind::Pin(p)) if p.author == "Ich"));
        assert_eq!(ed.xs.pin.as_ref().map(|w| w.id), Some(pid));
        // leerer neuer Kommentar wird beim Schließen verworfen
        ed.close_pin();
        assert!(!ed.board.items.iter().any(|i| i.id == pid));
        // Ziehen ist kein Tippen
        let n = ed.board.items.len();
        ed.x_down(MOUSE, Pos2::new(10.0, 10.0), 2.0);
        ed.x_move(MOUSE, Pos2::new(80.0, 10.0));
        ed.x_up(MOUSE, Pos2::new(80.0, 10.0), true, 2.1);
        assert_eq!(ed.board.items.len(), n);
    }

    #[test]
    fn gruppieren_und_ausrichten() {
        let mut ed = editor(false);
        for (x, y) in [(0.0, 0.0), (100.0, 50.0), (300.0, 20.0)] {
            ed.board.items.push(Rc::new(rect_item(x, y)));
        }
        ed.board.set_tool(board::SELECT);
        ed.board.sel = ed.board.items.iter().map(|i| i.id).collect();
        assert!(can_group(&ed.board));
        assert_eq!(units(&ed.board), 3);
        run(&mut ed, "group");
        assert!(ed.board.sel_grouped() && !can_group(&ed.board));
        assert_eq!(units(&ed.board), 1);
        run(&mut ed, "ungroup");
        assert!(!ed.board.sel_grouped());
        assert!(ed.board.align_sel(Align::Top));
        let t0 = ed.board.items[0].bounds().top;
        assert!(ed.board.items.iter().all(|i| (i.bounds().top - t0).abs() < 0.01));
    }
}
