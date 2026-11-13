//! Board-Editor: Zeichenfläche, Leisten, Texteingabe, Tastenkürzel, Autosave.

use crate::app::{FilePurpose, Shared};
use crate::style::{self, c, icon_btn, sep};
use egui::{Align2, Color32, CornerRadius, CursorIcon, Event, Key, PointerButton, Pos2, Rect, Sense, Stroke, TextureHandle, TouchPhase, Vec2};
use owndraw_core::board::{self, Action, Board, Motion, Pointer, ToolType};
use owndraw_core::icons as ic;
use owndraw_core::item::{self, INK};
use owndraw_core::tiny_skia::Pixmap;
use std::path::Path;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::thread::JoinHandle;

pub const COLORS: [(u32, &str); 8] = [
    (INK, "Tinte"),
    (0xFFE5484D, "Rot"),
    (0xFFF76B15, "Orange"),
    (0xFFFFC53D, "Gelb"),
    (0xFF30A46C, "Grün"),
    (0xFF3E63DD, "Blau"),
    (0xFF8E4EC6, "Violett"),
    (0xFF8B8D98, "Grau"),
];
pub const WIDTHS: [f32; 4] = [1.6, 3.0, 5.5, 10.0];
const AUTOSAVE_SECS: f64 = 30.0;

struct TextEd {
    wx: f32,
    wy: f32,
    note: bool,
    edit: Option<u64>,
    text: String,
    focus: bool,
}

pub struct Editor {
    pub id: String,
    pub name: String,
    pub board: Board,
    tex: Option<TextureHandle>,
    pix: Option<Pixmap>,
    drawing: bool,
    panning: Option<(Pos2, PointerButton)>,
    touches: Vec<(u64, Pos2)>,
    pub layers_open: bool,
    text: Option<TextEd>,
    last_save: f64,
    saving: Option<JoinHandle<()>>,
    save_failed: Arc<AtomicBool>,
    first_frame: bool,
}

impl Editor {
    pub fn open(sh: &mut Shared, id: &str, name: &str) -> Editor {
        item::set_img_dir(sh.store.img_dir());
        let mut board = Board::new(1.0);
        if let Some(j) = sh.store.load(id) {
            board.from_json(&j);
        }
        board.recog = sh.prefs.int("recog", 0).clamp(0, 2);
        board.sel_mode = sh.prefs.int("selmode", 0).clamp(0, 1);
        board.erase_mode = sh.prefs.int("erasemode", 0).clamp(0, 1);
        board.pen_w = sh.prefs.float("w", 3.0);
        if !WIDTHS.contains(&board.pen_w) {
            board.pen_w = 3.0;
        }
        Editor {
            id: id.to_string(),
            name: name.to_string(),
            board,
            tex: None,
            pix: None,
            drawing: false,
            panning: None,
            touches: Vec::new(),
            layers_open: false,
            text: None,
            last_save: sh.now,
            saving: None,
            save_failed: Arc::new(AtomicBool::new(false)),
            first_frame: true,
        }
    }

    fn wait_saving(&mut self, sh: &mut Shared) {
        if let Some(h) = self.saving.take() {
            let _ = h.join();
        }
        if self.save_failed.swap(false, Ordering::Relaxed) {
            sh.toast("Speichern fehlgeschlagen");
        }
    }

    /// Speichert sofort (mit Vorschaubild, wenn thumb), wartet auf laufende Hintergrund-Speicherungen.
    pub fn save(&mut self, sh: &mut Shared, thumb: bool) {
        self.wait_saving(sh);
        let json = self.board.to_json().to_string();
        let th = if thumb { self.board.thumb().encode_png().ok() } else { None };
        if !sh.store.save(&self.id, &json, th.as_deref(), self.board.modified || thumb) {
            sh.toast("Speichern fehlgeschlagen");
        }
        self.board.modified = false;
        self.last_save = sh.now;
        sh.prefs.set("recog", self.board.recog);
        sh.prefs.set("selmode", self.board.sel_mode);
        sh.prefs.set("erasemode", self.board.erase_mode);
        sh.prefs.set("w", self.board.pen_w);
        sh.prefs.save();
    }

    fn autosave(&mut self, sh: &mut Shared) {
        if sh.now - self.last_save < AUTOSAVE_SECS {
            return;
        }
        self.last_save = sh.now;
        if !self.board.modified || self.board.busy() {
            return;
        }
        self.wait_saving(sh);
        let json = self.board.to_json().to_string();
        self.board.modified = false;
        let (store, id, fail) = (sh.store.clone(), self.id.clone(), self.save_failed.clone());
        self.saving = Some(std::thread::Builder::new().name("owndraw-save".into()).spawn(move || {
            if !store.save(&id, &json, None, true) {
                fail.store(true, Ordering::Relaxed);
            }
        }).ok().unwrap_or_else(|| std::thread::spawn(|| {})));
    }

    pub fn import_image(&mut self, sh: &mut Shared, p: &Path) {
        match sh.store.import_image_file(p) {
            Some((name, w, h)) => {
                self.board.add_image(&name, w, h);
            }
            None => sh.toast("Bild konnte nicht geladen werden"),
        }
    }

    fn empty_hint(&self, sh: &mut Shared) -> bool {
        if self.board.export_bounds().is_none() {
            sh.toast("Das Board ist noch leer");
            return true;
        }
        false
    }

    pub fn export(&mut self, sh: &mut Shared, kind: u8) {
        if self.empty_hint(sh) {
            return;
        }
        let base = crate::app::safe_file_name(&self.name);
        match kind {
            0 => sh.open_file_dialog(FilePurpose::ExportPng, "Exportieren", ("PNG-Bild (*.png)", &["png"]), Some(format!("{base}.png"))),
            1 => sh.open_file_dialog(FilePurpose::ExportPdf, "Exportieren", ("PDF (*.pdf)", &["pdf"]), Some(format!("{base}.pdf"))),
            _ => sh.open_file_dialog(FilePurpose::ExportZip, "Exportieren", ("ZIP-Archiv (*.zip)", &["zip"]), Some(format!("{base}-website.zip"))),
        }
    }

    pub fn pick_image(&self, sh: &mut Shared) {
        sh.open_file_dialog(FilePurpose::ImportImage, "Bild importieren", ("Bilder (png, jpg, gif, bmp, webp)", &["png", "jpg", "jpeg", "gif", "bmp", "webp"]), None);
    }

    fn preview(&mut self, sh: &mut Shared) {
        if self.empty_hint(sh) {
            return;
        }
        match owndraw_core::export::build_preview(&sh.cache_dir(), &self.board) {
            Some(index) => {
                if open::that_detached(&index).is_err() {
                    sh.toast_for(&format!("Vorschau konnte nicht geöffnet werden: {}", index.display()), 4.0);
                }
            }
            None => sh.toast("Vorschau konnte nicht erstellt werden"),
        }
    }

    /// Werkzeug wählen; erneutes Wählen wechselt den Modus.
    pub fn tool(&mut self, sh: &mut Shared, t: i32) {
        let b = &mut self.board;
        if b.tool == t {
            match t {
                board::SHAPE => b.shape_kind = (b.shape_kind + 1) % 4,
                board::SELECT => {
                    b.sel_mode = 1 - b.sel_mode;
                    sh.toast(if b.sel_mode == 0 { "Auswahl: Lasso (Freihand umkreisen)" } else { "Auswahl: Rechteck aufziehen" });
                }
                board::ERASER => {
                    b.erase_mode = 1 - b.erase_mode;
                    sh.toast(if b.erase_mode == 0 { "Radierer: ganze Objekte" } else { "Radierer: nur überstrichene Stellen" });
                }
                _ => {}
            }
        } else {
            b.set_tool(t);
        }
        b.invalidate();
    }

    // ---------------------------------------------------------------- Haupt-UI

    /// Zeichnet den Editor; true = zurück zur Übersicht.
    pub fn ui(&mut self, ui: &mut egui::Ui, sh: &mut Shared, modal: bool) -> bool {
        let ctx = ui.ctx().clone();
        let rect = ui.max_rect();
        let layer = ui.layer_id();
        let resp = ui.allocate_rect(rect, Sense::click_and_drag());
        let ppp = ctx.pixels_per_point();
        self.board.set_size(rect.width(), rect.height(), ppp);
        self.board.tick(sh.now);
        if self.first_frame {
            self.first_frame = false;
        }

        if !modal {
            self.handle_pointer(&ctx, rect, layer);
        }
        let mut back = false;
        if !modal && self.text.is_none() && !ctx.egui_wants_keyboard_input() {
            back |= self.shortcuts(&ctx, sh);
        }
        self.drain_events(sh);

        // Fläche zeichnen
        if self.board.needs_redraw || self.tex.is_none() {
            let (w, h) = (((rect.width() * ppp).round() as u32).max(1), ((rect.height() * ppp).round() as u32).max(1));
            if self.pix.as_ref().map(|p| p.width() != w || p.height() != h).unwrap_or(true) {
                self.pix = Pixmap::new(w, h);
            }
            if let Some(pm) = self.pix.as_mut() {
                self.board.render(pm);
                let img = egui::ColorImage::from_rgba_premultiplied([w as usize, h as usize], pm.data());
                match self.tex.as_mut() {
                    Some(t) => t.set(img, egui::TextureOptions::NEAREST),
                    None => self.tex = Some(ctx.load_texture("board", img, egui::TextureOptions::NEAREST)),
                }
            }
        }
        if let Some(t) = &self.tex {
            ui.painter().image(t.id(), rect, Rect::from_min_max(Pos2::ZERO, Pos2::new(1.0, 1.0)), Color32::WHITE);
        }
        if resp.hovered() || self.drawing {
            let space = ctx.input(|i| i.key_down(Key::Space)) && !ctx.egui_wants_keyboard_input();
            let cur = if self.panning.is_some() {
                CursorIcon::Grabbing
            } else if space {
                CursorIcon::Grab
            } else {
                match self.board.tool {
                    board::PEN | board::MARKER | board::SHAPE | board::ERASER => CursorIcon::Crosshair,
                    board::TEXT => CursorIcon::Text,
                    _ => CursorIcon::Default,
                }
            };
            ctx.set_cursor_icon(cur);
        }

        back |= self.bars(&ctx, sh);
        if self.layers_open {
            crate::layers::panel(&ctx, self, sh);
        }
        self.text_editor(&ctx, rect);
        self.drain_events(sh);
        self.autosave(sh);

        // Zeitgesteuertes Neuzeichnen (Halte-Timer, Ende von Zoom-Gesten, Autosave)
        let mut wait = (AUTOSAVE_SECS - (sh.now - self.last_save)).max(0.5);
        if let Some(d) = self.board.next_deadline() {
            wait = wait.min((d - sh.now).max(0.0) + 0.005);
        }
        if self.board.needs_redraw {
            ctx.request_repaint();
        } else {
            ctx.request_repaint_after(std::time::Duration::from_secs_f64(wait));
        }
        back
    }

    fn drain_events(&mut self, sh: &mut Shared) {
        for e in std::mem::take(&mut self.board.events) {
            match e {
                board::Event::Notice(s) => sh.toast(&s),
                board::Event::AskText { wx, wy, note } => {
                    self.text = Some(TextEd { wx, wy, note, edit: None, text: String::new(), focus: true });
                }
                board::Event::Changed | board::Event::Haptic => {}
            }
        }
    }

    // ---------------------------------------------------------------- Eingabe

    fn handle_pointer(&mut self, ctx: &egui::Context, rect: Rect, layer: egui::LayerId) {
        let events = ctx.input(|i| i.events.clone());
        let hover = ctx.input(|i| i.pointer.hover_pos());
        let space = ctx.input(|i| i.key_down(Key::Space)) && !ctx.egui_wants_keyboard_input() && self.text.is_none();
        let on_board = |p: Pos2| rect.contains(p) && ctx.layer_id_at(p).map(|l| l == layer).unwrap_or(true);
        let has_touch = !self.touches.is_empty() || events.iter().any(|e| matches!(e, Event::Touch { .. }));
        let rel = |p: Pos2| p - rect.min;
        for ev in events {
            match ev {
                Event::Touch { id, phase, pos, .. } => self.touch(id.0, phase, rel(pos).to_pos2()),
                _ if has_touch && matches!(ev, Event::PointerMoved(_) | Event::PointerButton { .. }) => {}
                Event::PointerMoved(p) => {
                    let r = rel(p);
                    if self.drawing {
                        self.board.on_touch(&Motion::single(Action::Move, r.x, r.y, 0.5, ToolType::Stylus));
                    } else if let Some((last, b)) = self.panning {
                        self.board.pan_by(p.x - last.x, p.y - last.y);
                        self.panning = Some((p, b));
                    }
                }
                Event::PointerButton { pos, button, pressed, .. } => {
                    let r = rel(pos);
                    if pressed {
                        if self.drawing || self.panning.is_some() || !on_board(pos) || self.text.is_some() {
                            continue;
                        }
                        if button == PointerButton::Primary && !space {
                            self.drawing = true;
                            self.board.on_touch(&Motion::single(Action::Down, r.x, r.y, 0.5, ToolType::Stylus));
                        } else if matches!(button, PointerButton::Primary | PointerButton::Secondary | PointerButton::Middle) {
                            self.panning = Some((pos, button));
                        }
                    } else if self.drawing && button == PointerButton::Primary {
                        self.drawing = false;
                        self.board.on_touch(&Motion::single(Action::Up, r.x, r.y, 0.5, ToolType::Stylus));
                    } else if self.panning.map(|(_, b)| b == button).unwrap_or(false) {
                        self.panning = None;
                    }
                }
                Event::MouseWheel { unit, delta, modifiers, .. } => {
                    let Some(h) = hover.filter(|h| on_board(*h)) else { continue };
                    let r = rel(h);
                    match unit {
                        egui::MouseWheelUnit::Point if !modifiers.ctrl && !modifiers.command => self.board.pan_by(delta.x, delta.y),
                        _ => {
                            let lines = match unit {
                                egui::MouseWheelUnit::Line => delta.y,
                                egui::MouseWheelUnit::Point => delta.y / 50.0,
                                egui::MouseWheelUnit::Page => delta.y * 10.0,
                            };
                            if lines != 0.0 {
                                self.board.zoom_at(r.x, r.y, 1.1f32.powf(lines));
                            }
                        }
                    }
                }
                Event::Zoom(f) => {
                    if let Some(h) = hover.filter(|h| on_board(*h)) {
                        let r = rel(h);
                        self.board.zoom_at(r.x, r.y, f);
                    }
                }
                _ => {}
            }
        }
    }

    /// Touchscreen: Finger verschieben/zoomen wie auf dem Tablet (Zeichnen mit der Maus bzw. dem Stift als Maus).
    fn touch(&mut self, id: u64, phase: TouchPhase, p: Pos2) {
        let ptrs = |t: &Vec<(u64, Pos2)>| -> Vec<Pointer> {
            t.iter().map(|(i, q)| Pointer { id: *i as i64, x: q.x, y: q.y, pressure: 0.5, tool: ToolType::Finger }).collect()
        };
        match phase {
            TouchPhase::Start => {
                self.touches.retain(|t| t.0 != id);
                self.touches.push((id, p));
                let action = if self.touches.len() == 1 { Action::Down } else { Action::PointerDown(self.touches.len() - 1) };
                self.board.on_touch(&Motion { action, pointers: ptrs(&self.touches), history: Vec::new(), stylus_button: false });
            }
            TouchPhase::Move => {
                if let Some(t) = self.touches.iter_mut().find(|t| t.0 == id) {
                    t.1 = p;
                    self.board.on_touch(&Motion { action: Action::Move, pointers: ptrs(&self.touches), history: Vec::new(), stylus_button: false });
                }
            }
            TouchPhase::End | TouchPhase::Cancel => {
                let Some(i) = self.touches.iter().position(|t| t.0 == id) else { return };
                self.touches[i].1 = p;
                let action = if self.touches.len() == 1 {
                    if phase == TouchPhase::End {
                        Action::Up
                    } else {
                        Action::Cancel
                    }
                } else {
                    Action::PointerUp(i)
                };
                self.board.on_touch(&Motion { action, pointers: ptrs(&self.touches), history: Vec::new(), stylus_button: false });
                self.touches.remove(i);
            }
        }
    }

    fn zoom_center(&mut self, f: f32) {
        let (w, h) = (self.board.width(), self.board.height());
        self.board.zoom_at(w / 2.0, h / 2.0, f);
    }

    /// Tastenkürzel; true = zurück zur Übersicht.
    fn shortcuts(&mut self, ctx: &egui::Context, sh: &mut Shared) -> bool {
        let events = ctx.input(|i| i.events.clone());
        let (mut copy, mut cut, mut paste) = (false, false, false);
        for ev in events {
            match ev {
                Event::Copy => copy = true,
                Event::Cut => cut = true,
                Event::Paste(_) => paste = true,
                Event::Key { key, pressed: true, modifiers, repeat, .. } => {
                    let cmd = modifiers.command || modifiers.ctrl;
                    let sel = self.board.tool == board::SELECT && !self.board.sel.is_empty();
                    match key {
                        Key::C if cmd => copy = true,
                        Key::X if cmd => cut = true,
                        Key::V if cmd => paste = true,
                        Key::D if cmd => {
                            if sel {
                                self.board.duplicate_sel();
                            }
                        }
                        Key::Z if cmd && modifiers.shift => self.board.redo(),
                        Key::Z if cmd => self.board.undo(),
                        Key::Y if cmd => self.board.redo(),
                        Key::S if cmd => {
                            self.save(sh, true);
                            sh.toast("Gespeichert");
                        }
                        Key::E if cmd => {
                            ctx.memory_mut(|m| m.data.insert_temp(egui::Id::unique("export-open"), true));
                        }
                        Key::Delete | Key::Backspace if !repeat => {
                            if sel {
                                self.board.delete_sel();
                            }
                        }
                        _ if cmd || modifiers.alt || repeat => {}
                        Key::P => self.tool(sh, board::PEN),
                        Key::M => self.tool(sh, board::MARKER),
                        Key::E => self.tool(sh, board::ERASER),
                        Key::S => self.tool(sh, board::SELECT),
                        Key::R => self.tool(sh, board::SHAPE),
                        Key::N => self.tool(sh, board::NOTE),
                        Key::T => self.tool(sh, board::TEXT),
                        Key::Num0 => self.board.fit(),
                        Key::Plus | Key::Equals => self.zoom_center(1.25),
                        Key::Minus => self.zoom_center(0.8),
                        _ => {}
                    }
                }
                _ => {}
            }
        }
        let sel = self.board.tool == board::SELECT && !self.board.sel.is_empty();
        if copy && sel {
            self.board.copy_sel();
        }
        if cut && sel {
            self.board.cut_sel();
        }
        if paste && self.board.has_clip() {
            self.board.paste();
        }
        false
    }

    // ---------------------------------------------------------------- Leisten

    fn bars(&mut self, ctx: &egui::Context, sh: &mut Shared) -> bool {
        let mut back = false;
        let tint = style::TEXT;
        // oben links: Zurück + Titel
        egui::Area::new(egui::Id::unique("bar-tl")).anchor(Align2::LEFT_TOP, Vec2::new(12.0, 12.0)).show(ctx, |ui| {
            style::pill(26).show(ui, |ui| {
                ui.horizontal(|ui| {
                    if icon_btn(ui, &mut sh.icons, ic::BACK, tint, false, false, "Zurück zur Übersicht").clicked() {
                        back = true;
                    }
                    let t = egui::RichText::new(&self.name).size(15.0).strong().color(c(style::TEXT));
                    let r = ui.add(egui::Button::new(t).fill(Color32::TRANSPARENT).truncate().min_size(Vec2::new(80.0, 36.0))).on_hover_text("Board umbenennen");
                    ui.add_space(6.0);
                    if r.clicked() {
                        sh.dialog = Some(crate::app::Dialog::Rename {
                            what: crate::app::RenameWhat::Board(self.id.clone()),
                            title: "Board umbenennen",
                            text: self.name.clone(),
                            focus: true,
                        });
                    }
                });
            });
        });

        // oben rechts: Aktionen
        let export_id = egui::Id::unique("export-open");
        let mut export_open = ctx.memory(|m| m.data.get_temp::<bool>(export_id).unwrap_or(false));
        egui::Area::new(egui::Id::unique("bar-tr")).anchor(Align2::RIGHT_TOP, Vec2::new(-12.0, 12.0)).show(ctx, |ui| {
            style::pill(26).show(ui, |ui| {
                ui.horizontal(|ui| {
                    ui.spacing_mut().item_spacing.x = 0.0;
                    let b = &mut self.board;
                    let u = if b.can_undo() { tint } else { style::DISABLED };
                    if icon_btn(ui, &mut sh.icons, ic::UNDO, u, false, false, "Rückgängig (Strg+Z)").clicked() {
                        b.undo();
                    }
                    let r = if b.can_redo() { tint } else { style::DISABLED };
                    if icon_btn(ui, &mut sh.icons, ic::REDO, r, false, false, "Wiederholen (Strg+Y)").clicked() {
                        b.redo();
                    }
                    sep(ui);
                    let mt = if b.recog == 2 { style::DIM } else { tint };
                    if icon_btn(ui, &mut sh.icons, ic::MAGIC, mt, false, b.recog == 1, "Formerkennung (3 Modi)").clicked() {
                        b.recog = (b.recog + 1) % 3;
                        sh.toast(["Formerkennung: Stift am Ende kurz halten", "Formerkennung: immer", "Formerkennung: aus"][b.recog as usize]);
                    }
                    if icon_btn(ui, &mut sh.icons, ic::GRID, tint, false, false, "Hintergrund wechseln").clicked() {
                        b.bg = (b.bg + 1) % 4;
                        b.modified = true;
                        b.refresh();
                        sh.toast(&format!("Hintergrund: {}", ["Punkte", "Raster", "Linien", "Leer"][b.bg as usize]));
                    }
                    if icon_btn(ui, &mut sh.icons, ic::DARK, tint, b.dark, false, "Hell/Dunkel").clicked() {
                        b.dark = !b.dark;
                        b.modified = true;
                        b.refresh();
                    }
                    sep(ui);
                    if icon_btn(ui, &mut sh.icons, ic::LAYERS, tint, self.layers_open, false, "Ebenen").clicked() {
                        self.layers_open = !self.layers_open;
                    }
                    if icon_btn(ui, &mut sh.icons, ic::IMAGE, tint, false, false, "Bild importieren").clicked() {
                        self.pick_image(sh);
                    }
                    if icon_btn(ui, &mut sh.icons, ic::PLAY, tint, false, false, "Website-Vorschau").clicked() {
                        self.preview(sh);
                    }
                    sep(ui);
                    let er = icon_btn(ui, &mut sh.icons, ic::EXPORT, tint, export_open, false, "Exportieren (Strg+E)");
                    if er.clicked() {
                        export_open = !export_open;
                    }
                    if export_open && self.board.export_bounds().is_none() {
                        export_open = false;
                        sh.toast("Das Board ist noch leer");
                    }
                    let mut pick: Option<u8> = None;
                    let pop = egui::Popup::menu(&er).open(export_open).align(egui::RectAlign::BOTTOM_END).close_behavior(egui::PopupCloseBehavior::CloseOnClickOutside).show(|ui| {
                        if style::menu_item(ui, "Als PNG-Bild", false) {
                            pick = Some(0);
                        }
                        if style::menu_item(ui, "Als PDF", false) {
                            pick = Some(1);
                        }
                        if style::menu_item(ui, "Ebenen + Website (ZIP)", false) {
                            pick = Some(2);
                        }
                    });
                    if let Some(k) = pick {
                        export_open = false;
                        self.export(sh, k);
                    } else if export_open && pop.is_none() {
                        export_open = false;
                    } else if export_open && ui.input(|i| i.pointer.any_click()) && !er.clicked() {
                        if let Some(p) = &pop {
                            if !p.response.contains_pointer() {
                                export_open = false;
                            }
                        }
                    }
                });
            });
            // Zoom-Anzeige
            ui.add_space(8.0);
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Min), |ui| {
                style::pill(18).show(ui, |ui| {
                    let t = format!("{} %", (self.board.sc * 100.0).round() as i32);
                    if ui.add(egui::Button::new(egui::RichText::new(t).size(13.0)).fill(Color32::TRANSPARENT)).on_hover_text("Alles zeigen (0)").clicked() {
                        self.board.fit();
                    }
                });
            });
        });
        ctx.memory_mut(|m| m.data.insert_temp(export_id, export_open));

        // Auswahl-Aktionsleiste
        let b = &self.board;
        let has_sel = !b.sel.is_empty();
        if b.tool == board::SELECT && (has_sel || b.has_clip()) {
            let single_note = b.single_note().map(|n| n.id);
            egui::Area::new(egui::Id::unique("bar-sel")).anchor(Align2::CENTER_BOTTOM, Vec2::new(0.0, -78.0)).show(ctx, |ui| {
                style::pill(26).show(ui, |ui| {
                    ui.horizontal(|ui| {
                        ui.spacing_mut().item_spacing.x = 0.0;
                        let ics = &mut sh.icons;
                        if has_sel {
                            if icon_btn(ui, ics, ic::COPY, tint, false, false, "Kopieren (Strg+C)").clicked() {
                                self.board.copy_sel();
                            }
                            if icon_btn(ui, ics, ic::CUT, tint, false, false, "Ausschneiden (Strg+X)").clicked() {
                                self.board.cut_sel();
                            }
                        }
                        if self.board.has_clip() && icon_btn(ui, ics, ic::PASTE, tint, false, false, "Einfügen (Strg+V)").clicked() {
                            self.board.paste();
                        }
                        if has_sel {
                            if icon_btn(ui, ics, ic::DUPLICATE, tint, false, false, "Duplizieren (Strg+D)").clicked() {
                                self.board.duplicate_sel();
                            }
                            sep(ui);
                            if icon_btn(ui, ics, ic::ROTATE, tint, false, false, "Um 90° drehen").clicked() {
                                self.board.rotate_sel(std::f32::consts::FRAC_PI_2);
                            }
                            if icon_btn(ui, ics, ic::FRONT, tint, false, false, "Ganz nach vorne").clicked() {
                                self.board.bring_forward();
                            }
                            if icon_btn(ui, ics, ic::BACKWARD, tint, false, false, "Ganz nach hinten").clicked() {
                                self.board.send_backward();
                            }
                            sep(ui);
                            if let Some(nid) = single_note {
                                if icon_btn(ui, ics, ic::PEN, tint, false, false, "Text bearbeiten").clicked() {
                                    self.edit_note(nid);
                                }
                            }
                            if icon_btn(ui, ics, ic::TRASH, style::DANGER, false, false, "Löschen (Entf)").clicked() {
                                self.board.delete_sel();
                            }
                        }
                    });
                });
            });
        }

        // unten: Werkzeuge, Farben, Stärken
        egui::Area::new(egui::Id::unique("bar-bottom")).anchor(Align2::CENTER_BOTTOM, Vec2::new(0.0, -16.0)).show(ctx, |ui| {
            style::pill(26).show(ui, |ui| {
                ui.horizontal(|ui| {
                    ui.spacing_mut().item_spacing.x = 0.0;
                    let b = &self.board;
                    let tools: [(i32, &str); 7] = [
                        (ic::PEN, "Stift (P)"),
                        (ic::MARKER, "Marker (M)"),
                        (if b.erase_mode == 1 { ic::ERASER_PART } else { ic::ERASER }, "Radierer (E) · erneut klicken: Modus wechseln"),
                        (if b.sel_mode == 0 { ic::LASSO } else { ic::SELECT }, "Auswahl (S) · erneut klicken: Lasso/Rechteck"),
                        ([ic::RECT, ic::ELLIPSE, ic::LINE, ic::ARROW][b.shape_kind.clamp(0, 3) as usize], "Form (R) · erneut klicken: Rechteck/Ellipse/Linie/Pfeil"),
                        (ic::NOTE, "Notiz (N)"),
                        (ic::TEXT, "Text (T)"),
                    ];
                    let cur = b.tool;
                    for (i, (icn, tip)) in tools.iter().enumerate() {
                        if icon_btn(ui, &mut sh.icons, *icn, tint, cur == i as i32, false, tip).clicked() {
                            self.tool(sh, i as i32);
                        }
                    }
                    sep(ui);
                    for (col, name) in COLORS {
                        let (r, resp) = ui.allocate_exact_size(Vec2::new(34.0, 40.0), Sense::click());
                        let p = ui.painter();
                        p.circle_filled(r.center(), 9.0, c(col));
                        if col == INK {
                            p.circle_stroke(r.center(), 9.0, Stroke::new(1.0, c(0x80FFFFFF)));
                        }
                        if self.board.color == col {
                            p.circle_stroke(r.center(), 13.0, Stroke::new(2.0, Color32::WHITE));
                        } else if resp.hovered() {
                            p.circle_stroke(r.center(), 13.0, Stroke::new(1.5, c(0x55FFFFFF)));
                        }
                        if resp.on_hover_text(name).clicked() {
                            self.board.set_color(col);
                            self.board.invalidate();
                        }
                    }
                    sep(ui);
                    for (i, w) in WIDTHS.iter().enumerate() {
                        let (r, resp) = ui.allocate_exact_size(Vec2::new(34.0, 40.0), Sense::click());
                        let d = [4.0, 7.0, 11.0, 16.0][i];
                        let on = self.board.pen_w == *w;
                        if on {
                            ui.painter().rect_filled(r.shrink2(Vec2::new(1.0, 4.0)), CornerRadius::same(16), c(style::ACTIVE));
                        } else if resp.hovered() {
                            ui.painter().rect_filled(r.shrink2(Vec2::new(1.0, 4.0)), CornerRadius::same(16), c(0x17FFFFFF));
                        }
                        ui.painter().circle_filled(r.center(), d / 2.0, c(style::TEXT));
                        if resp.on_hover_text(format!("Strichstärke {}", i + 1)).clicked() {
                            self.board.pen_w = *w;
                        }
                    }
                });
            });
        });
        back
    }

    pub fn edit_note(&mut self, id: u64) {
        let Some(it) = self.board.items.iter().find(|it| it.id == id) else { return };
        let Some(n) = it.as_note() else { return };
        self.text = Some(TextEd { wx: n.x, wy: n.y, note: n.bg != 0, edit: Some(id), text: n.text.clone(), focus: true });
    }

    // ---------------------------------------------------------------- Texteingabe (Notiz/Text)

    fn text_editor(&mut self, ctx: &egui::Context, rect: Rect) {
        let Some(te) = self.text.as_mut() else { return };
        let b = &self.board;
        let w = (300.0f32).min(rect.width() - 20.0);
        let sx = te.wx * b.sc + b.tx;
        let sy = te.wy * b.sc + b.ty;
        let x = sx.clamp(12.0, (rect.width() - w - 12.0).max(12.0));
        let y = sy.clamp(70.0, (rect.height() / 2.0 - 200.0).max(70.0));
        let note_col = board::note_bg(b.color);
        let (bg, fg, hint) = if te.note { (note_col, 0xFF1F2025u32, "Notiz schreiben …") } else { (0xEE1C1D22u32, style::TEXT, "Text schreiben …") };
        let mut done = ctx.input_mut(|i| i.consume_key(egui::Modifiers::COMMAND, Key::Enter) || i.consume_key(egui::Modifiers::CTRL, Key::Enter));
        let mut cancel = ctx.input_mut(|i| i.consume_key(egui::Modifiers::NONE, Key::Escape));
        egui::Area::new(egui::Id::unique("text-ed")).order(egui::Order::Foreground).fixed_pos(rect.min + Vec2::new(x, y)).show(ctx, |ui| {
            egui::Frame::new()
                .fill(c(bg))
                .stroke(Stroke::new(1.0, c(if te.note { 0x22000000 } else { style::LINE })))
                .corner_radius(CornerRadius::same(18))
                .inner_margin(egui::Margin::same(12))
                .shadow(egui::Shadow { offset: [0, 4], blur: 18, spread: 0, color: Color32::from_black_alpha(110) })
                .show(ui, |ui| {
                    ui.set_width(w - 24.0);
                    let hint_col = if te.note { 0x801F2025 } else { 0x808B8D98 };
                    let ed = egui::TextEdit::multiline(&mut te.text)
                        .hint_text(egui::RichText::new(hint).color(c(hint_col)))
                        .font(egui::FontId::proportional(17.0))
                        .text_color(c(fg))
                        .desired_rows(4)
                        .desired_width(w - 24.0)
                        .frame(egui::Frame::NONE)
                        .background_color(Color32::TRANSPARENT);
                    let r = egui::ScrollArea::vertical().max_height(17.0 * 1.35 * 4.0 + 8.0).show(ui, |ui| ui.add(ed)).inner;
                    if te.focus {
                        r.request_focus();
                        te.focus = false;
                    }
                    ui.add_space(8.0);
                    ui.horizontal(|ui| {
                        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                            let (okc, cc) = if te.note { (0xFF3B4BD6u32, 0xFF4A4D5Cu32) } else { (style::ACCENT, style::DIM) };
                            if ui.add(egui::Button::new(egui::RichText::new("Fertig (Strg+↵)").strong().color(c(okc))).fill(Color32::TRANSPARENT)).clicked() {
                                done = true;
                            }
                            if ui.add(egui::Button::new(egui::RichText::new("Abbrechen (Esc)").color(c(cc))).fill(Color32::TRANSPARENT)).clicked() {
                                cancel = true;
                            }
                        });
                    });
                });
        });
        if done {
            let te = self.text.take().unwrap();
            let t = te.text.trim();
            if !t.is_empty() {
                match te.edit {
                    Some(id) => self.board.edit_note(id, t),
                    None => self.board.add_note(te.wx, te.wy, t, te.note),
                }
            }
        } else if cancel {
            self.text = None;
        }
    }
}
