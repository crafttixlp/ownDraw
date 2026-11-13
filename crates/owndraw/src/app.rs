//! Hauptfenster: Board-Übersicht, Dialoge, Dateiauswahl, Hinweise (Toast) und Wechsel in den Editor.

use crate::editor::Editor;
use crate::prefs::Prefs;
use crate::style::{self, c, Icons};
use egui::{Align2, Color32, CornerRadius, Key, Pos2, Rect, Sense, Stroke, TextureHandle, Vec2};
use egui_file_dialog::{FileDialog, FileDialogLabels};
use owndraw_core::layer::{A_PARALLAX, ANIM_NAMES};
use owndraw_core::store::Store;
use std::collections::HashMap;
use std::path::{Path, PathBuf};

pub enum RenameWhat {
    Board(String),
    Layer(usize),
}

pub enum ConfirmAction {
    DeleteBoard(String),
    DeleteLayer(usize),
}

pub struct AnimDlg {
    pub idx: usize,
    pub anim: i32,
    pub speed: f32,
    pub hover: bool,
    pub opacity: f32,
}

pub enum Dialog {
    Rename { what: RenameWhat, title: &'static str, text: String, focus: bool },
    Confirm { title: &'static str, text: String, ok: &'static str, action: ConfirmAction },
    Anim(AnimDlg),
}

#[derive(Clone)]
pub enum FilePurpose {
    ImportBoard,
    ExportBoard(String),
    ImportImage,
    ExportPng,
    ExportPdf,
    ExportZip,
    ExportLayer(usize),
}

/// Von Übersicht und Editor gemeinsam genutzter Zustand.
pub struct Shared {
    pub data_dir: PathBuf,
    pub store: Store,
    pub prefs: Prefs,
    pub icons: Icons,
    toast: Option<(String, f64)>,
    pub dialog: Option<Dialog>,
    fdialog: Option<(FileDialog, FilePurpose)>,
    pub now: f64,
}

impl Shared {
    pub fn toast(&mut self, s: &str) {
        self.toast_for(s, 1.8);
    }
    pub fn toast_for(&mut self, s: &str, secs: f64) {
        self.toast = Some((s.to_string(), self.now + secs));
    }

    pub fn cache_dir(&self) -> PathBuf {
        self.data_dir.join("cache")
    }

    fn last_dir(&self) -> PathBuf {
        self.prefs
            .str("lastdir")
            .map(PathBuf::from)
            .filter(|p| p.is_dir())
            .unwrap_or_else(|| std::env::var("HOME").map(PathBuf::from).unwrap_or_else(|_| PathBuf::from("/")))
    }

    /// Öffnet die Dateiauswahl. save_name = Some(...) für Speichern.
    pub fn open_file_dialog(&mut self, purpose: FilePurpose, title: &str, filter: (&str, &[&'static str]), save_name: Option<String>) {
        let mut d = FileDialog::new()
            .title(title)
            .labels(german_labels())
            .initial_directory(self.last_dir())
            .add_file_filter_extensions(filter.0, filter.1.to_vec())
            .default_file_filter(filter.0)
            .default_size([760.0, 500.0]);
        match save_name {
            Some(n) => {
                d = d.add_save_extension(filter.0, filter.1[0]).default_save_extension(filter.0).default_file_name(&n);
                d.save_file();
            }
            None => d.pick_file(),
        }
        self.fdialog = Some((d, purpose));
    }

    fn modal_open(&self) -> bool {
        self.dialog.is_some() || self.fdialog.is_some()
    }
}

fn german_labels() -> FileDialogLabels {
    FileDialogLabels {
        title_select_directory: "Ordner wählen".into(),
        title_select_file: "Datei öffnen".into(),
        title_select_multiple: "Dateien wählen".into(),
        title_save_file: "Datei speichern".into(),
        cancel: "Abbrechen".into(),
        overwrite: "Überschreiben".into(),
        reload: "Neu laden".into(),
        working_directory: "Arbeitsordner".into(),
        show_hidden: "Versteckte Dateien zeigen".into(),
        heading_pinned: "Angeheftet".into(),
        heading_places: "Orte".into(),
        heading_devices: "Geräte".into(),
        heading_removable_devices: "Wechseldatenträger".into(),
        home_dir: "Persönlicher Ordner".into(),
        desktop_dir: "Schreibtisch".into(),
        documents_dir: "Dokumente".into(),
        downloads_dir: "Downloads".into(),
        pictures_dir: "Bilder".into(),
        selected_file: "Datei:".into(),
        file_name: "Dateiname:".into(),
        file_filter_all_files: "Alle Dateien".into(),
        open_button: "Öffnen".into(),
        save_button: "Speichern".into(),
        cancel_button: "Abbrechen".into(),
        overwrite_file_modal_text: "existiert bereits. Überschreiben?".into(),
        err_empty_file_name: "Dateiname fehlt".into(),
        err_file_exists: "Datei existiert bereits".into(),
        err_directory_exists: "Ordner existiert bereits".into(),
        ..Default::default()
    }
}

pub struct App {
    pub sh: Shared,
    editor: Option<Editor>,
    thumbs: HashMap<String, (TextureHandle, std::time::SystemTime)>,
    pending_open: Option<PathBuf>,
}

impl App {
    pub fn new(data_dir: PathBuf, open_file: Option<PathBuf>) -> App {
        let store = Store::new(&data_dir.join("files"));
        let prefs = Prefs::load(data_dir.join("prefs.txt"));
        App {
            sh: Shared { data_dir, store, prefs, icons: Icons::default(), toast: None, dialog: None, fdialog: None, now: 0.0 },
            editor: None,
            thumbs: HashMap::new(),
            pending_open: open_file,
        }
    }

    fn open_board(&mut self, id: &str, ctx: &egui::Context) {
        if let Some(mut e) = self.editor.take() {
            e.save(&mut self.sh, true);
        }
        let name = self.sh.store.name(id);
        let e = Editor::open(&mut self.sh, id, &name);
        ctx.send_viewport_cmd(egui::ViewportCommand::Title(format!("OwnDraw – {name}")));
        self.editor = Some(e);
    }

    fn close_board(&mut self, ctx: &egui::Context) {
        if let Some(mut e) = self.editor.take() {
            e.save(&mut self.sh, true);
            self.thumbs.remove(&e.id);
        }
        ctx.send_viewport_cmd(egui::ViewportCommand::Title("OwnDraw".into()));
    }

    fn import_board_file(&mut self, p: &Path) {
        let ok = std::fs::File::open(p).ok().and_then(|f| self.sh.store.import_board(std::io::BufReader::new(f)));
        self.sh.toast(if ok.is_some() { "Board importiert" } else { "Import fehlgeschlagen" });
    }

    // ---------------------------------------------------------------- Übersicht

    fn thumb(&mut self, ctx: &egui::Context, id: &str) -> Option<TextureHandle> {
        let p = self.sh.store.thumb(id);
        let mt = std::fs::metadata(&p).and_then(|m| m.modified()).ok()?;
        if let Some((t, m)) = self.thumbs.get(id) {
            if *m == mt {
                return Some(t.clone());
            }
        }
        let pm = owndraw_core::tiny_skia::Pixmap::load_png(&p).ok()?;
        let img = egui::ColorImage::from_rgba_premultiplied([pm.width() as usize, pm.height() as usize], pm.data());
        let t = ctx.load_texture(format!("thumb-{id}"), img, egui::TextureOptions::LINEAR);
        self.thumbs.insert(id.to_string(), (t.clone(), mt));
        Some(t)
    }

    fn overview(&mut self, ui: &mut egui::Ui) {
        let ctx = ui.ctx().clone();
        egui::Frame::new().inner_margin(egui::Margin { left: 40, right: 40, top: 30, bottom: 14 }).show(ui, |ui| {
            ui.horizontal(|ui| {
                ui.vertical(|ui| {
                    ui.label(egui::RichText::new("OwnDraw").size(34.0).strong().color(c(style::TEXT)));
                    ui.label(egui::RichText::new("Zeichnen in Ebenen · Rechtsklick für Optionen").size(14.0).color(c(style::DIM)));
                });
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if style::accent_btn(ui, "Board importieren").clicked() {
                        self.sh.open_file_dialog(FilePurpose::ImportBoard, "Board importieren", ("OwnDraw-Board (*.owndraw)", &["owndraw"]), None);
                    }
                });
            });
        });
        let entries = self.sh.store.entries();
        let mut open: Option<String> = None;
        egui::ScrollArea::vertical().auto_shrink([false, false]).show(ui, |ui| {
            egui::Frame::new().inner_margin(egui::Margin { left: 16 + 24, right: 16 + 24, top: 6, bottom: 24 }).show(ui, |ui| {
                ui.spacing_mut().item_spacing = Vec2::new(24.0, 24.0);
                ui.horizontal_wrapped(|ui| {
                    // "Neues Board"
                    let (r, resp) = ui.allocate_exact_size(Vec2::new(290.0, 247.0), Sense::click());
                    let p = ui.painter();
                    p.rect_filled(r, CornerRadius::same(36), c(if resp.hovered() { 0x427C8CFF } else { 0x2E7C8CFF }));
                    dashed_round_rect(p, r.shrink(1.0), 36.0, Stroke::new(1.5, c(style::ACCENT)));
                    p.text(r.center() - Vec2::new(0.0, 16.0), Align2::CENTER_CENTER, "+", egui::FontId::proportional(52.0), c(style::ACCENT));
                    p.text(r.center() + Vec2::new(0.0, 36.0), Align2::CENTER_CENTER, "Neues Board", egui::FontId::proportional(16.0), c(style::TEXT));
                    if resp.clicked() {
                        let n = self.sh.store.index().len() + 1;
                        open = Some(self.sh.store.create(&format!("Board {n}")));
                    }
                    for e in &entries {
                        let (r, resp) = ui.allocate_exact_size(Vec2::new(290.0, 247.0), Sense::click());
                        let tex = self.thumb(&ctx, &e.id);
                        let p = ui.painter();
                        p.rect_filled(r, CornerRadius::same(36), c(style::CARD));
                        let img_r = Rect::from_min_size(r.min, Vec2::new(r.width(), 181.0));
                        let top = CornerRadius { nw: 36, ne: 36, sw: 0, se: 0 };
                        p.rect_filled(img_r, top, c(0xFFFBFBF8));
                        if let Some(t) = tex {
                            // auf die Kachel zuschneiden (Mitte), Bild hat 480 × 300
                            let ts = t.size_vec2();
                            let k = (img_r.width() / ts.x).max(img_r.height() / ts.y);
                            let (uw, vh) = (img_r.width() / (ts.x * k), img_r.height() / (ts.y * k));
                            let uv = Rect::from_min_max(Pos2::new((1.0 - uw) / 2.0, (1.0 - vh) / 2.0), Pos2::new((1.0 + uw) / 2.0, (1.0 + vh) / 2.0));
                            egui::Image::new(&t).uv(uv).corner_radius(top).paint_at(ui, img_r);
                        }
                        let p = ui.painter();
                        p.line_segment([img_r.left_bottom(), img_r.right_bottom()], Stroke::new(1.0, c(0x14000000)));
                        let name = elide(&e.name, 16.0, 258.0, ui);
                        p.text(Pos2::new(r.left() + 16.0, r.top() + 209.0), Align2::LEFT_CENTER, name, egui::FontId::proportional(16.0), c(style::TEXT));
                        p.text(Pos2::new(r.left() + 16.0, r.top() + 231.0), Align2::LEFT_CENTER, fmt_time(e.t), egui::FontId::proportional(12.0), c(style::DIM));
                        if resp.hovered() {
                            p.rect_stroke(r, CornerRadius::same(36), Stroke::new(1.5, c(0x55FFFFFF)), egui::StrokeKind::Inside);
                        }
                        if resp.clicked() {
                            open = Some(e.id.clone());
                        }
                        resp.context_menu(|ui| {
                            if style::menu_item(ui, "Öffnen", false) {
                                open = Some(e.id.clone());
                            }
                            if style::menu_item(ui, "Umbenennen", false) {
                                self.sh.dialog = Some(Dialog::Rename { what: RenameWhat::Board(e.id.clone()), title: "Board umbenennen", text: e.name.clone(), focus: true });
                            }
                            if style::menu_item(ui, "Duplizieren", false) {
                                self.sh.store.duplicate(&e.id);
                            }
                            if style::menu_item(ui, "Exportieren (.owndraw)…", false) {
                                self.sh.open_file_dialog(
                                    FilePurpose::ExportBoard(e.id.clone()),
                                    "Board exportieren",
                                    ("OwnDraw-Board (*.owndraw)", &["owndraw"]),
                                    Some(format!("{}.owndraw", safe_file_name(&e.name))),
                                );
                            }
                            if style::menu_item(ui, "Löschen", true) {
                                self.sh.dialog = Some(Dialog::Confirm {
                                    title: "Board löschen",
                                    text: format!("„{}“ wirklich löschen?", e.name),
                                    ok: "Löschen",
                                    action: ConfirmAction::DeleteBoard(e.id.clone()),
                                });
                            }
                        });
                    }
                });
            });
        });
        if let Some(id) = open {
            self.open_board(&id, &ctx);
        }
    }

    // ---------------------------------------------------------------- Dialoge

    fn dialogs(&mut self, ctx: &egui::Context) {
        let Some(d) = self.sh.dialog.as_mut() else { return };
        let mut close = false;
        let mut ok = false;
        let (enter, esc) = ctx.input(|i| (i.key_pressed(Key::Enter), i.key_pressed(Key::Escape)));
        egui::Modal::new(egui::Id::unique("dlg")).frame(style::card()).show(ctx, |ui| {
            ui.set_min_width(360.0);
            match d {
                Dialog::Rename { title, text, focus, .. } => {
                    ui.label(egui::RichText::new(*title).heading().strong());
                    ui.add_space(8.0);
                    let r = ui.add(egui::TextEdit::singleline(text).desired_width(320.0).font(egui::FontId::proportional(15.0)).margin(Vec2::new(10.0, 8.0)));
                    if *focus {
                        r.request_focus();
                        if let Some(mut st) = egui::TextEdit::load_state(ui.ctx(), r.id) {
                            let cr = egui::text::CCursorRange::two(egui::text::CCursor::new(0), egui::text::CCursor::new(text.chars().count()));
                            st.cursor.set_char_range(Some(cr));
                            st.store(ui.ctx(), r.id);
                        }
                        *focus = false;
                    }
                    ui.add_space(12.0);
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        ok |= style::primary_btn(ui, "OK").clicked() || enter;
                        close |= style::plain_btn(ui, "Abbrechen").clicked() || esc;
                    });
                }
                Dialog::Confirm { title, text, ok: okl, .. } => {
                    ui.label(egui::RichText::new(*title).heading().strong());
                    ui.add_space(8.0);
                    ui.label(text.as_str());
                    ui.add_space(12.0);
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        ok |= style::danger_btn(ui, okl).clicked() || enter;
                        close |= style::plain_btn(ui, "Abbrechen").clicked() || esc;
                    });
                }
                Dialog::Anim(a) => {
                    ui.label(egui::RichText::new("Animation & Deckkraft").heading().strong());
                    ui.add_space(6.0);
                    for (i, n) in ANIM_NAMES.iter().enumerate() {
                        let i = i as i32;
                        if ui.radio(a.anim == i, *n).clicked() && a.anim != i {
                            if (i == A_PARALLAX) != (a.anim == A_PARALLAX) {
                                a.speed = if i == A_PARALLAX { 0.2 } else { 1.0 };
                            }
                            a.anim = i;
                        }
                    }
                    ui.add_space(6.0);
                    let par = a.anim == A_PARALLAX;
                    let mut p = if par { a.speed * 100.0 } else { (a.speed - 0.25) / 2.75 * 100.0 };
                    let lbl = if par { "Parallax-Faktor" } else { "Tempo" };
                    ui.label(format!("{lbl}: {}×", format!("{:.1}", a.speed).replace('.', ",")));
                    if ui.add(egui::Slider::new(&mut p, 0.0..=100.0).show_value(false)).changed() {
                        let v = if par { p / 100.0 } else { 0.25 + p / 100.0 * 2.75 };
                        a.speed = (v / 0.05).round() * 0.05;
                    }
                    ui.checkbox(&mut a.hover, "Hover-Effekt");
                    let mut o = a.opacity * 100.0;
                    ui.label(format!("Deckkraft: {} %", o.round() as i32));
                    if ui.add(egui::Slider::new(&mut o, 0.0..=100.0).show_value(false)).changed() {
                        a.opacity = (o.round() / 100.0).clamp(0.0, 1.0);
                    }
                    ui.add_space(12.0);
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        ok |= style::primary_btn(ui, "Übernehmen").clicked();
                        close |= style::plain_btn(ui, "Abbrechen").clicked() || esc;
                    });
                }
            }
        });
        if ok {
            let d = self.sh.dialog.take().unwrap();
            self.apply_dialog(d);
        } else if close {
            self.sh.dialog = None;
        }
    }

    fn apply_dialog(&mut self, d: Dialog) {
        match d {
            Dialog::Rename { what, text, .. } => {
                let t = text.trim().to_string();
                if t.is_empty() {
                    return;
                }
                match what {
                    RenameWhat::Board(id) => {
                        self.sh.store.meta(&id, Some(&t), false);
                        if let Some(e) = self.editor.as_mut().filter(|e| e.id == id) {
                            e.name = t;
                        }
                    }
                    RenameWhat::Layer(i) => {
                        if let Some(e) = self.editor.as_mut() {
                            e.board.edit_layer(i, |l| l.name = t);
                        }
                    }
                }
            }
            Dialog::Confirm { action, .. } => match action {
                ConfirmAction::DeleteBoard(id) => {
                    self.sh.store.delete(&id);
                    self.thumbs.remove(&id);
                }
                ConfirmAction::DeleteLayer(i) => {
                    if let Some(e) = self.editor.as_mut() {
                        e.board.remove_layer(i);
                    }
                }
            },
            Dialog::Anim(a) => {
                if let Some(e) = self.editor.as_mut() {
                    e.board.edit_layer(a.idx, |l| {
                        l.anim = a.anim;
                        l.speed = a.speed;
                        l.hover = a.hover;
                        l.opacity = a.opacity;
                    });
                }
            }
        }
    }

    // ---------------------------------------------------------------- Dateiauswahl

    fn file_dialog(&mut self, ctx: &egui::Context) {
        let Some((d, _)) = self.sh.fdialog.as_mut() else { return };
        d.update(ctx);
        let picked = d.take_picked();
        let state_closed = matches!(d.state(), egui_file_dialog::DialogState::Cancelled | egui_file_dialog::DialogState::Closed);
        if picked.is_none() && !state_closed {
            return;
        }
        let (_, purpose) = self.sh.fdialog.take().unwrap();
        let Some(p) = picked else { return };
        if let Some(dir) = p.parent() {
            self.sh.prefs.set("lastdir", dir.display());
            self.sh.prefs.save();
        }
        self.apply_file(purpose, p);
    }

    fn apply_file(&mut self, purpose: FilePurpose, p: PathBuf) {
        let with_ext = |p: PathBuf, ext: &str| -> PathBuf {
            if p.extension().map(|e| e.eq_ignore_ascii_case(ext)).unwrap_or(false) {
                p
            } else {
                PathBuf::from(format!("{}.{ext}", p.display()))
            }
        };
        match purpose {
            FilePurpose::ImportBoard => self.import_board_file(&p),
            FilePurpose::ExportBoard(id) => {
                if let Some(e) = self.editor.as_mut().filter(|e| e.id == id) {
                    e.save(&mut self.sh, false);
                }
                let p = with_ext(p, "owndraw");
                let ok = std::fs::File::create(&p).map(|f| self.sh.store.export_board(&id, std::io::BufWriter::new(f))).unwrap_or(false);
                self.sh.toast(if ok { "Board exportiert" } else { "Export fehlgeschlagen" });
            }
            FilePurpose::ImportImage => {
                if let Some(e) = self.editor.as_mut() {
                    e.import_image(&mut self.sh, &p);
                }
            }
            FilePurpose::ExportPng | FilePurpose::ExportPdf | FilePurpose::ExportZip => {
                let Some(e) = self.editor.as_mut() else { return };
                let ok = match purpose {
                    FilePurpose::ExportPng => e.board.export_png().map(|d| std::fs::write(with_ext(p, "png"), d).is_ok()).unwrap_or(false),
                    FilePurpose::ExportPdf => e.board.export_pdf().map(|d| std::fs::write(with_ext(p, "pdf"), d).is_ok()).unwrap_or(false),
                    _ => std::fs::File::create(with_ext(p, "zip"))
                        .map(|f| owndraw_core::export::write_zip(&e.board, &e.name, std::io::BufWriter::new(f)))
                        .unwrap_or(false),
                };
                self.sh.toast(if ok { "Export gespeichert" } else { "Export fehlgeschlagen" });
            }
            FilePurpose::ExportLayer(i) => {
                let Some(e) = self.editor.as_mut() else { return };
                let ok = e
                    .board
                    .layers
                    .get(i)
                    .and_then(|l| owndraw_core::export::layer_png(&e.board, l))
                    .map(|d| std::fs::write(with_ext(p, "png"), d).is_ok())
                    .unwrap_or(false);
                self.sh.toast(if ok { "Ebene exportiert" } else { "Export fehlgeschlagen" });
            }
        }
    }

    fn toast_ui(&mut self, ctx: &egui::Context) {
        let Some((msg, until)) = self.sh.toast.clone() else { return };
        if self.sh.now >= until {
            self.sh.toast = None;
            return;
        }
        ctx.request_repaint_after(std::time::Duration::from_secs_f64((until - self.sh.now).max(0.01)));
        let off = if self.editor.is_some() { 104.0 } else { 48.0 };
        egui::Area::new(egui::Id::unique("toast")).order(egui::Order::Tooltip).anchor(Align2::CENTER_BOTTOM, Vec2::new(0.0, -off)).interactable(false).show(ctx, |ui| {
            egui::Frame::new()
                .fill(c(0xF2303138))
                .corner_radius(CornerRadius::same(20))
                .inner_margin(egui::Margin::symmetric(22, 11))
                .shadow(egui::Shadow { offset: [0, 2], blur: 8, spread: 0, color: Color32::from_black_alpha(80) })
                .show(ui, |ui| {
                    ui.label(egui::RichText::new(msg).size(14.0).color(c(style::TEXT)));
                });
        });
    }
}

impl eframe::App for App {
    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        let ctx = ui.ctx().clone();
        self.sh.now = ctx.input(|i| i.time);
        if let Some(p) = self.pending_open.take() {
            if p.extension().map(|e| e == "owndraw").unwrap_or(false) {
                self.import_board_file(&p);
            }
        }
        // per Drag & Drop abgelegte Dateien
        let dropped: Vec<PathBuf> = ctx.input(|i| i.raw.dropped_files.iter().map(|f| f.path().to_path_buf()).collect());
        for p in dropped {
            let ext = p.extension().map(|e| e.to_string_lossy().to_lowercase()).unwrap_or_default();
            if ext == "owndraw" {
                self.import_board_file(&p);
            } else if let Some(e) = self.editor.as_mut() {
                if ["png", "jpg", "jpeg", "gif", "bmp", "webp"].contains(&ext.as_str()) {
                    e.import_image(&mut self.sh, &p);
                } else {
                    self.sh.toast("Das ist keine Bilddatei");
                }
            }
        }
        if ctx.input(|i| i.viewport().close_requested()) {
            if let Some(e) = self.editor.as_mut() {
                e.save(&mut self.sh, true);
            }
        }

        let modal = self.sh.modal_open();
        let mut back = false;
        egui::CentralPanel::default().frame(egui::Frame::new().fill(c(style::BG))).show(ui, |ui| match self.editor.as_mut() {
            Some(e) => back = e.ui(ui, &mut self.sh, modal),
            None => self.overview(ui),
        });
        if back {
            self.close_board(&ctx);
        }
        self.dialogs(&ctx);
        self.file_dialog(&ctx);
        self.toast_ui(&ctx);
    }
}

/// Dateiname ohne verbotene Zeichen.
pub fn safe_file_name(n: &str) -> String {
    let s: String = n.chars().map(|ch| if "\\/:*?\"<>|".contains(ch) { '_' } else { ch }).collect();
    if s.trim().is_empty() {
        "Board".into()
    } else {
        s
    }
}

fn elide(s: &str, size: f32, max_w: f32, ui: &egui::Ui) -> String {
    let font = egui::FontId::proportional(size);
    let w = |t: &str| ui.fonts_mut(|f| f.layout_no_wrap(t.to_string(), font.clone(), Color32::WHITE).size().x);
    if w(s) <= max_w {
        return s.to_string();
    }
    let mut chars: Vec<char> = s.chars().collect();
    while !chars.is_empty() {
        chars.pop();
        let t: String = chars.iter().collect::<String>() + "…";
        if w(&t) <= max_w {
            return t;
        }
    }
    "…".into()
}

/// Lokale Zeit als "dd.MM.yyyy · HH:mm".
fn fmt_time(ms: i64) -> String {
    let off = local_offset_secs();
    let secs = ms / 1000 + off;
    let days = secs.div_euclid(86400);
    let sod = secs.rem_euclid(86400);
    // Tage seit 1970 nach Datum (H. Hinnant)
    let z = days + 719468;
    let era = z.div_euclid(146097);
    let doe = z - era * 146097;
    let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = yoe + era * 400 + if m <= 2 { 1 } else { 0 };
    format!("{d:02}.{m:02}.{y} · {:02}:{:02}", sod / 3600, sod % 3600 / 60)
}

/// Zeitzonen-Versatz aus /etc/localtime bzw. TZ (nur aktueller Versatz, ohne Abhängigkeit).
fn local_offset_secs() -> i64 {
    use std::sync::OnceLock;
    static OFF: OnceLock<i64> = OnceLock::new();
    *OFF.get_or_init(|| {
        let now = owndraw_core::store::now_ms() / 1000;
        let data = std::env::var("TZ")
            .ok()
            .filter(|t| !t.is_empty())
            .and_then(|t| std::fs::read(Path::new("/usr/share/zoneinfo").join(t.trim_start_matches(':'))).ok())
            .or_else(|| std::fs::read("/etc/localtime").ok());
        data.and_then(|d| tzif_offset(&d, now)).unwrap_or(0)
    })
}

/// Liest den zum Zeitpunkt t gültigen UTC-Versatz aus einer TZif-Datei (v1-Block).
fn tzif_offset(d: &[u8], t: i64) -> Option<i64> {
    if d.len() < 44 || &d[0..4] != b"TZif" {
        return None;
    }
    let be = |o: usize| -> Option<i64> { Some(i32::from_be_bytes(d.get(o..o + 4)?.try_into().ok()?) as i64) };
    let (isutc, isstd, leap, timecnt, typecnt, charcnt) = (be(20)?, be(24)?, be(28)?, be(32)? as usize, be(36)? as usize, be(40)?);
    let _ = (isutc, isstd, leap, charcnt);
    let trans = 44;
    let idx = trans + timecnt * 4;
    let types = idx + timecnt;
    let mut ti = 0usize;
    for i in 0..timecnt {
        if be(trans + i * 4)? <= t {
            ti = *d.get(idx + i)? as usize;
        }
    }
    if timecnt == 0 && typecnt == 0 {
        return None;
    }
    be(types + ti * 6)
}

/// Gestrichelte abgerundete Umrandung (für die "Neues Board"-Kachel).
fn dashed_round_rect(p: &egui::Painter, r: Rect, rad: f32, st: Stroke) {
    let mut pts = Vec::new();
    let n = 10;
    let corners = [(r.right() - rad, r.top() + rad, -90.0f32), (r.right() - rad, r.bottom() - rad, 0.0), (r.left() + rad, r.bottom() - rad, 90.0), (r.left() + rad, r.top() + rad, 180.0)];
    for (cx, cy, a0) in corners {
        for i in 0..=n {
            let a = (a0 + 90.0 * i as f32 / n as f32).to_radians();
            pts.push(Pos2::new(cx + rad * a.cos(), cy + rad * a.sin()));
        }
    }
    pts.push(pts[0]);
    p.extend(egui::Shape::dashed_line(&pts, st, 6.0, 6.0));
}
