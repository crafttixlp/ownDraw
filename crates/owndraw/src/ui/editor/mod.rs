//! Editor-Bildschirm: Zeichenfläche, Eingabe, Tastenkürzel, Autosave-Anbindung und das Gerüst für Leisten,
//! Optionen, ⋯-Menü, Ebenen und Seiten (je nach Formfaktor Compact/Medium/Expanded).

pub mod clip;
pub mod drop;
mod layers;
mod menu;
mod options;
pub mod pages;
mod text;
mod toolbar;

use crate::app::{PickPurpose, Req, Shared};
use crate::autosave::{Autosave, AutosaveStatus, SaveFormat};
use crate::layout::{self, Env, FormFactor};
use crate::platform::{FileKind, FileRef};
use crate::session;
use crate::settings::{BarPos, Settings, ALL_TOOLS, DEFAULT_TOOLS};
use crate::ui::dialogs::{AnimDlg, ConfirmAction, RenameWhat};
use egui::{Align2, Color32, CursorIcon, Event, Key, Order, PointerButton, Pos2, Rect, Sense, TextureHandle, TouchPhase, Ui};
use owndraw_core::board::{self, Action, Board, Motion, Pointer, ToolType};
use owndraw_core::doc::Document;
use owndraw_core::item::{self, INK};
use owndraw_core::tiny_skia::Pixmap;
use std::path::PathBuf;

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
/// Abstand schwebender Leisten zum Rand der Zeichenfläche.
const M: f32 = 8.0;
/// Innenabstand einer Leiste (Frame-Innenrand 4 + Rahmen 1).
const PAD: f32 = 5.0;
/// Breite der angedockten Panels (Expanded).
const DOCK_LAYERS: f32 = 300.0;
const DOCK_PAGES: f32 = 188.0;

/// Die sieben Hauptwerkzeuge der Leiste.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Tool {
    Pen,
    Marker,
    Eraser,
    Select,
    Shape,
    Text,
    Image,
}

impl Tool {
    pub fn from_id(s: &str) -> Option<Tool> {
        Some(match s {
            "pen" => Tool::Pen,
            "marker" => Tool::Marker,
            "eraser" => Tool::Eraser,
            "select" => Tool::Select,
            "shape" => Tool::Shape,
            "text" | "note" => Tool::Text,
            "image" => Tool::Image,
            _ => return None,
        })
    }
    pub fn label(self) -> &'static str {
        match self {
            Tool::Pen => "Stift",
            Tool::Marker => "Marker",
            Tool::Eraser => "Radierer",
            Tool::Select => "Auswahl",
            Tool::Shape => "Form",
            Tool::Text => "Text",
            Tool::Image => "Bild",
        }
    }
    pub fn key(self) -> &'static str {
        match self {
            Tool::Pen => "P",
            Tool::Marker => "M",
            Tool::Eraser => "E",
            Tool::Select => "S",
            Tool::Shape => "R",
            Tool::Text => "T",
            Tool::Image => "I",
        }
    }
    /// Hat das Werkzeug Optionen (Kontext-Popover bzw. Bottom-Sheet)?
    pub fn has_options(self) -> bool {
        self != Tool::Image
    }
}

/// Sichtbare Werkzeuge in der Reihenfolge der Einstellungen (unbekannte Kennungen werden übersprungen).
pub fn visible_tools(s: &Settings) -> Vec<Tool> {
    let mut v: Vec<Tool> = Vec::new();
    for id in &s.tools {
        if let Some(t) = Tool::from_id(id) {
            if !v.contains(&t) {
                v.push(t);
            }
        }
    }
    if v.is_empty() {
        v = DEFAULT_TOOLS.iter().filter_map(|i| Tool::from_id(i)).collect();
    }
    v
}

/// Oberflächenzustand des Editors (Popover, Menü, Panels).
#[derive(Default)]
pub struct UiState {
    pub opts_open: bool,
    pub menu_open: bool,
    pub layers_open: bool,
    /// None = Vorgabe nach Formfaktor (Expanded: offen).
    pub pages_open: Option<bool>,
    /// Rechtecke der Werkzeugknöpfe des letzten Frames (Popover-Anker).
    pub tool_rects: Vec<(Tool, Rect)>,
    pub menu_btn: Option<Rect>,
    /// Bereiche der offenen Popups (zum Schließen bei Klick daneben).
    pub popup_rects: Vec<Rect>,
    pub error_toast_shown: bool,
    /// Bearbeiten-Modus der Werkzeugleiste (WP9): `Some` solange aktiv.
    pub edit_tools: Option<toolbar::EditTools>,
    /// Zuletzt gemessene Größe der verschiebbaren Kacheln.
    pub tile_sizes: std::collections::HashMap<String, egui::Vec2>,
    /// Laufendes Verschieben einer Kachel: (Schlüssel, linke obere Ecke).
    pub tile_drag: Option<(String, Pos2)>,
    /// Letzte Zeigerposition beim Ziehen einer Kachel (Touch liefert beim Loslassen keine).
    pub tile_ptr: Option<Pos2>,
    /// In diesem Frame bereits platzierte, an einem Rand liegende Kacheln (zum Stapeln).
    pub dock_rects: Vec<(BarPos, Rect)>,
    /// Werkzeugleiste im letzten Frame senkrecht angeordnet.
    pub tools_vert: bool,
}

/// Berechnete Lage der schwebenden Elemente (pro Frame).
pub(super) struct Lay {
    pub canvas: Rect,
    pub env: Env,
    pub tb: BarPos,
    /// Rand, an dem die Werkzeugleiste tatsächlich liegt (None = frei verschoben).
    pub tb_edge: Option<BarPos>,
    /// Werkzeugleiste in zweiter Reihe unter den oberen Pillen (nur bei `tb == Top`).
    pub row2: bool,
    /// y-Koordinate unter der obersten Reihe (Start von seitlichen Leisten/Panels).
    pub y_below: f32,
    /// Höhe/Breite der Werkzeugleiste (quer zur Leiste).
    pub thick: f32,
    /// Länge der Werkzeugleiste.
    pub dock_layers: Option<Rect>,
    pub dock_pages: Option<Rect>,
    pub pages_mode: bool,
}

pub struct Editor {
    pub doc: Document,
    pub board: Board,
    pub autosave: Autosave,
    img_dir: PathBuf,
    tex: Option<TextureHandle>,
    pix: Option<Pixmap>,
    /// Textur gerade bilinear gefiltert (waehrend Zoom-Gesten skaliert gezeigt).
    tex_linear: bool,
    /// Zoom-Geste: Textur skaliert zeigen (siehe `Frame::scaled`).
    scaled: Option<(f32, f32, f32)>,
    /// Ursprung des Ring-Speichers der Textur.
    org: (u32, u32),
    drawing: bool,
    panning: Option<(Pos2, PointerButton)>,
    /// Startpunkt des Verschiebens (mittlere Maustaste ohne Bewegung = Objekt auswählen).
    pan_start: Option<Pos2>,
    touches: Vec<(u64, Pos2)>,
    pub(super) text: Option<text::TextEdState>,
    pub st: UiState,
    pub(super) page_thumbs: pages::PageThumbs,
    canvas: Rect,
    /// Mausposition über der Zeichenfläche (Bildschirmpunkte), Ziel für Einfügen.
    hover: Option<Pos2>,
    /// Ort des letzten Rechtsklicks/Lang-Tippens (Einfügen-Menü).
    menu_pos: Option<Pos2>,
    first_frame: bool,
    /// Beim ersten Frame „Alles zeigen“ (Demo/Abnahme).
    pub fit_on_open: bool,
}

impl Editor {
    /// Öffnet ein Dokument; `file` = Zieldatei für Autosave (None: nicht speichern).
    pub fn open(sh: &mut Shared, doc: Document, file: Option<FileRef>) -> Editor {
        let img_dir = sh.platform.cache_dir().join("img").join(&doc.meta.id);
        if let Err(e) = session::prepare_images(&doc, &img_dir) {
            sh.toast(&format!("Bilder konnten nicht vorbereitet werden: {e}"));
        }
        item::set_img_dir(img_dir.clone());
        let mut board = Board::new(1.0);
        board.from_json(&doc.board);
        board.board_mode = doc.mode.clone();
        let x = &sh.settings.extra;
        let geti = |k: &str, d: i64| x.get(k).and_then(|v| v.as_i64()).unwrap_or(d);
        board.recog = geti("recog", 0).clamp(0, 2) as i32;
        board.sel_mode = geti("selmode", 0).clamp(0, 1) as i32;
        board.erase_mode = geti("erasemode", 0).clamp(0, 1) as i32;
        board.pen_w = x.get("w").and_then(|v| v.as_f64()).map(|v| v as f32).unwrap_or(3.0);
        if !WIDTHS.contains(&board.pen_w) {
            board.pen_w = 3.0;
        }
        board.shape_kind = geti("shape", 0).clamp(0, 4) as i32;
        board.arrow_style = geti("arrow", 0).clamp(0, owndraw_core::item::ARROW_STYLES.len() as i64 - 1) as i32;
        // Auf Touch-Geräten zeichnet der Finger standardmäßig (Schalter im Werkzeug-Popover).
        board.finger_draw = x.get("finger").and_then(|v| v.as_bool()).unwrap_or_else(|| sh.platform.is_touch());
        Editor {
            doc,
            board,
            autosave: Autosave::new(file),
            img_dir,
            tex: None,
            tex_linear: false,
            scaled: None,
            org: (0, 0),
            pix: None,
            drawing: false,
            panning: None,
            pan_start: None,
            touches: Vec::new(),
            text: None,
            st: UiState::default(),
            page_thumbs: pages::PageThumbs::default(),
            canvas: Rect::NOTHING,
            hover: None,
            menu_pos: None,
            first_frame: true,
            fit_on_open: false,
        }
    }

    pub fn name(&self) -> &str {
        &self.doc.meta.name
    }

    pub fn file(&self) -> Option<&FileRef> {
        self.autosave.target()
    }

    /// Dokument mit dem aktuellen Stand des Boards (für Speichern, Kopie, Export).
    pub fn snapshot_doc(&self) -> Document {
        let mut d = self.doc.clone();
        d.board = self.board.to_json();
        d.mode = self.board.board_mode.clone();
        d
    }

    /// Speicherauftrag: im UI-Thread nur Board-JSON und Vorschaubild (Objekte sind nicht threadfähig),
    /// PNG-Kodierung und Serialisieren/Packen im Schreib-Thread.
    fn snapshot_bytes(board: &Board, doc: &mut Document, fmt: SaveFormat) -> crate::autosave::Job {
        doc.board = board.to_json();
        doc.mode = board.board_mode.clone();
        doc.meta.modified = session::rfc3339_now();
        let thumb = board.thumb();
        let mut d = doc.clone();
        crate::autosave::Job(Box::new(move || {
            d.thumb = thumb.encode_png().ok();
            session::serialize_as(&d, fmt).unwrap_or_default()
        }))
    }

    /// Speichert sofort, falls Änderungen anstehen (Zurück, Schließen, App pausiert).
    pub fn flush(&mut self, sh: &mut Shared) {
        self.sync_modified(sh.now);
        let Self { autosave, board, doc, .. } = self;
        let fmt = autosave.format();
        autosave.flush(sh.now, || Self::snapshot_bytes(board, doc, fmt), &*sh.platform);
        if let Some(e) = self.autosave.error() {
            sh.toast_for(e, 4.0);
        }
    }

    /// Fehlermeldung: bei fehlenden Rechten ein Dialog mit der Aktion „Kopie speichern unter …“, sonst ein Hinweis.
    fn show_save_error(&mut self, sh: &mut Shared) {
        let e = self.autosave.error().unwrap_or("").to_string();
        if self.autosave.permission_error() && sh.dialog.is_none() {
            sh.dialog = Some(crate::ui::dialogs::Dialog::Confirm { title: "Speichern nicht möglich", text: e, ok: "Kopie speichern unter …", action: ConfirmAction::SaveCopy });
        } else {
            sh.toast_for(&e, 5.0);
        }
    }

    fn sync_modified(&mut self, now: f64) {
        if self.board.modified {
            self.board.modified = false;
            self.autosave.mark_changed(now);
            self.page_thumbs.dirty = true;
        }
    }

    pub fn rename(&mut self, name: String, now: f64) {
        self.doc.meta.name = name;
        self.autosave.mark_changed(now);
    }

    // ---------------------------------------------------------------- Aufträge der Shell

    pub fn apply_rename(&mut self, what: RenameWhat, text: String, now: f64) {
        match what {
            RenameWhat::Doc => self.rename(text, now),
            RenameWhat::Layer(i) => self.board.edit_layer(i, |l| l.name = text),
        }
    }

    pub fn apply_confirm(&mut self, a: ConfirmAction, sh: &mut Shared) {
        match a {
            ConfirmAction::SaveCopy => sh.reqs.push(crate::app::Req::PickSave {
                purpose: PickPurpose::SaveCopy,
                name: format!("{}.owndraw", crate::app::safe_file_name(&self.doc.meta.name)),
                kind: FileKind::Owndraw,
            }),
            ConfirmAction::DeleteLayer(i) => self.board.remove_layer(i),
            ConfirmAction::DeletePage(i) => {
                if !pages::delete_page(&mut self.board, i) {
                    sh.toast("Seite löschen: folgt mit dem Seitenmodus (WP2)");
                }
            }
        }
    }

    pub fn apply_anim(&mut self, a: AnimDlg) {
        self.board.edit_layer(a.idx, |l| {
            l.anim = a.anim;
            l.speed = a.speed;
            l.hover = a.hover;
            l.opacity = a.opacity;
        });
    }

    /// Ergebnis eines System-Dialogs (Speichern unter, Export, Bildauswahl).
    pub fn apply_pick(&mut self, sh: &mut Shared, purpose: PickPurpose, f: FileRef) {
        let with_ext = |f: FileRef, ext: &str| -> FileRef {
            match f {
                FileRef::Path(p) if !p.extension().map(|e| e.eq_ignore_ascii_case(ext)).unwrap_or(false) => FileRef::Path(PathBuf::from(format!("{}.{ext}", p.display()))),
                other => other,
            }
        };
        let write = |sh: &mut Shared, f: &FileRef, data: Option<Vec<u8>>, ok: &str| match data {
            Some(d) => match sh.platform.write_atomic(f, &d) {
                Ok(()) => sh.toast(ok),
                Err(e) => sh.toast(&format!("Speichern fehlgeschlagen: {e}")),
            },
            None => sh.toast("Export fehlgeschlagen"),
        };
        match purpose {
            PickPurpose::ExportPng => write(sh, &with_ext(f, "png"), self.board.export_png(), "Export gespeichert"),
            PickPurpose::ExportPdf => write(sh, &with_ext(f, "pdf"), self.board.export_pdf(), "Export gespeichert"),
            PickPurpose::ExportZip => {
                let mut cur = std::io::Cursor::new(Vec::new());
                let ok = owndraw_core::export::write_zip(&self.board, &self.doc.meta.name, &mut cur);
                write(sh, &with_ext(f, "zip"), ok.then(|| cur.into_inner()), "Export gespeichert");
            }
            PickPurpose::ExportLayer(i) => {
                let png = self.board.layers.get(i).and_then(|l| owndraw_core::export::layer_png(&self.board, l));
                write(sh, &with_ext(f, "png"), png, "Ebene exportiert");
            }
            PickPurpose::SaveCopy => {
                let f = with_ext(f, "owndraw");
                match session::save_copy_as(&*sh.platform, &self.snapshot_doc(), &f) {
                    Ok(()) => sh.toast("Kopie gespeichert"),
                    Err(e) => sh.toast(&e),
                }
            }
            PickPurpose::ExportXopp => {
                let f = with_ext(f, "xopp");
                match session::export_xopp(&*sh.platform, &self.snapshot_doc(), &f) {
                    Ok(()) => sh.toast("Xournal++-Datei gespeichert"),
                    Err(e) => sh.toast(&e),
                }
            }
            PickPurpose::ImportImage => match sh.platform.read(&f) {
                Ok(b) => {
                    if let Err(e) = self.import_image_bytes(&b) {
                        sh.toast(&e);
                    }
                }
                Err(e) => sh.toast(&format!("Bild konnte nicht geladen werden: {e}")),
            },
            PickPurpose::OpenDoc | PickPurpose::NewDoc | PickPurpose::DefaultDir => {}
        }
    }

    /// Bild aus Rohdaten einfügen (Datei, Drop, Zwischenablage).
    pub fn import_image_bytes(&mut self, bytes: &[u8]) -> Result<(), String> {
        clip::insert_image_bytes(self, bytes, None)
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
        let base = crate::app::safe_file_name(&self.doc.meta.name);
        let (purpose, name, k) = match kind {
            0 => (PickPurpose::ExportPng, format!("{base}.png"), FileKind::Png),
            1 => (PickPurpose::ExportPdf, format!("{base}.pdf"), FileKind::Pdf),
            _ => (PickPurpose::ExportZip, format!("{base}-website.zip"), FileKind::Zip),
        };
        sh.reqs.push(Req::PickSave { purpose, name, kind: k });
    }

    pub fn export_layer(&mut self, sh: &mut Shared, i: usize) {
        let Some(l) = self.board.layers.get(i) else { return };
        if !self.board.items.iter().any(|it| it.layer == l.id) {
            sh.toast(&format!("Die Ebene „{}“ ist leer", l.name));
        } else {
            let name = format!("{}.png", l.file_name());
            sh.reqs.push(Req::PickSave { purpose: PickPurpose::ExportLayer(i), name, kind: FileKind::Png });
        }
    }

    pub fn pick_image(&self, sh: &mut Shared) {
        sh.reqs.push(Req::PickOpen { purpose: PickPurpose::ImportImage, kinds: vec![FileKind::Image] });
    }

    pub fn preview(&mut self, sh: &mut Shared) {
        if self.empty_hint(sh) {
            return;
        }
        match owndraw_core::export::build_preview(&sh.platform.cache_dir(), &self.board) {
            Some(index) => sh.platform.open_url(&index.display().to_string()),
            None => sh.toast("Vorschau konnte nicht erstellt werden"),
        }
    }

    // ---------------------------------------------------------------- Werkzeuge

    pub fn active_tool(&self) -> Tool {
        match self.board.tool {
            board::PEN => Tool::Pen,
            board::MARKER => Tool::Marker,
            board::ERASER => Tool::Eraser,
            board::SELECT => Tool::Select,
            board::SHAPE => Tool::Shape,
            _ => Tool::Text,
        }
    }

    /// Werkzeug wählen (Bild öffnet die Dateiauswahl).
    pub fn select_tool(&mut self, sh: &mut Shared, t: Tool) {
        let bt = match t {
            Tool::Pen => board::PEN,
            Tool::Marker => board::MARKER,
            Tool::Eraser => board::ERASER,
            Tool::Select => board::SELECT,
            Tool::Shape => board::SHAPE,
            Tool::Text => {
                if self.board.tool == board::NOTE {
                    board::NOTE
                } else {
                    board::TEXT
                }
            }
            Tool::Image => {
                self.pick_image(sh);
                return;
            }
        };
        if self.board.tool != bt {
            self.board.set_tool(bt);
        }
        self.board.invalidate();
    }

    // ---------------------------------------------------------------- Layout

    fn layout(&self, sh: &Shared, full: Rect, ntools: usize) -> Lay {
        let env = sh.env;
        let pages_mode = matches!(self.board.board_mode, owndraw_core::pages::BoardMode::Pages(_));
        let mut canvas = full;
        let mut dock_layers = None;
        let mut dock_pages = None;
        if env.ff == FormFactor::Expanded {
            if self.st.layers_open {
                let r = Rect::from_min_max(Pos2::new(canvas.right() - DOCK_LAYERS, canvas.top()), canvas.max);
                canvas.max.x -= DOCK_LAYERS;
                dock_layers = Some(r);
            }
            if pages_mode && self.st.pages_open.unwrap_or_else(|| sh.settings.extra.get("pages_dock").and_then(|v| v.as_bool()).unwrap_or(true)) {
                let r = Rect::from_min_max(canvas.min, Pos2::new(canvas.left() + DOCK_PAGES, canvas.bottom()));
                canvas.min.x += DOCK_PAGES;
                dock_pages = Some(r);
            }
        }
        let btn = env.btn;
        let thick = btn + 2.0 * PAD;
        let length = ntools as f32 * btn + 2.0 * PAD;
        // Breite der oberen Pillen schätzen, um zu entscheiden, ob die Werkzeugleiste in dieselbe Reihe passt.
        let name_w = (self.doc.meta.name.chars().count() as f32 * 8.5).clamp(70.0, 220.0);
        let wl = btn + name_w + 2.0 * PAD + 24.0;
        let wr = 3.0 * btn + 64.0 + 2.0 * PAD;
        let fits_row = canvas.width() >= wl + length + wr + 6.0 * M;
        let mut tb = layout::toolbar_pos(env.ff, sh.settings.toolbar_pos);
        let mut tb_edge = Some(tb);
        // per Griff angedockte/verschobene Werkzeugleiste
        let key = toolbar::tile_key("bar-tools", env.ff);
        let custom = if let Some(d) = sh.settings.tile_dock.get(&key) {
            tb = d.edge.unwrap_or(if d.vert { BarPos::Left } else { BarPos::Top });
            tb_edge = d.edge;
            true
        } else if sh.settings.tile_pos.contains_key(&key) {
            tb_edge = None;
            true
        } else {
            false
        };
        let row2 = tb == BarPos::Top && !fits_row;
        let mut y_below = canvas.top() + M + thick + M;
        if row2 {
            y_below += thick + M;
        }
        // Seitliche Leiste, die nicht in die Höhe passt (Handy quer): automatisch nach unten, solange nichts Festes gewählt ist.
        if !custom && matches!(tb, BarPos::Left | BarPos::Right) && sh.settings.toolbar_pos.is_none() && y_below + length + M + (thick + M) > canvas.bottom() {
            tb = BarPos::Bottom;
            tb_edge = Some(tb);
        }
        Lay { canvas, env, tb, tb_edge, row2, y_below, thick, dock_layers, dock_pages, pages_mode }
    }

    // ---------------------------------------------------------------- Haupt-UI

    /// Zeichnet den Editor.
    pub fn ui(&mut self, ui: &mut Ui, sh: &mut Shared) {
        let ctx = ui.ctx().clone();
        let tools = if self.st.edit_tools.is_some() { ALL_TOOLS.iter().filter_map(|i| Tool::from_id(i)).collect() } else { visible_tools(&sh.settings) };
        let lay = self.layout(sh, ui.max_rect(), tools.len());
        let rect = lay.canvas;
        self.canvas = rect;
        let layer = ui.layer_id();
        let resp = ui.allocate_rect(rect, Sense::click_and_drag());
        let ppp = ctx.pixels_per_point();
        self.board.set_size(rect.width(), rect.height(), ppp);
        self.board.tick(sh.now);
        if self.first_frame {
            self.first_frame = false;
            if self.fit_on_open {
                self.board.fit();
            }
            pages::initial_view(&mut self.board);
        }

        let modal = sh.dialog.is_some() || self.st.edit_tools.is_some();
        // Klick auf die Zeichenfläche schließt offene Popups und zeichnet nicht.
        let popup_open = self.st.opts_open || self.st.menu_open;
        let mut swallow = false;
        if popup_open && !modal {
            let press = ctx.input(|i| i.pointer.any_pressed()).then(|| ctx.input(|i| i.pointer.interact_pos())).flatten();
            let touch = ctx.input(|i| i.events.iter().find_map(|e| if let Event::Touch { phase: TouchPhase::Start, pos, .. } = e { Some(*pos) } else { None }));
            if let Some(p) = press.or(touch) {
                let on_canvas = rect.contains(p) && ctx.layer_id_at(p).map(|l| l == layer).unwrap_or(true);
                if on_canvas {
                    self.st.opts_open = false;
                    self.st.menu_open = false;
                    swallow = true;
                }
            }
        }
        if !modal && !swallow {
            self.handle_pointer(&ctx, rect, layer);
        }
        if !modal && self.text.is_none() && !ctx.egui_wants_keyboard_input() {
            self.shortcuts(&ctx, sh);
        }
        self.drain_events(sh);
        if !modal && self.text.is_none() && (self.board.tool == board::SELECT || !self.board.finger_draw || !sh.platform.is_touch()) {
            clip::context_menu(self, &resp, sh);
        }

        // Fläche zeichnen: nur geänderte Bereiche zur GPU (Dirty-Rects), Zoom-Gesten skaliert die GPU
        if self.board.needs_redraw || self.tex.is_none() {
            let (w, h) = (((rect.width() * ppp).round() as u32).max(1), ((rect.height() * ppp).round() as u32).max(1));
            if self.pix.as_ref().map(|p| p.width() != w || p.height() != h).unwrap_or(true) {
                self.pix = Pixmap::new(w, h);
                self.board.force_full_frame();
            }
            if self.tex.is_none() {
                self.board.force_full_frame();
            }
            if let Some(pm) = self.pix.as_mut() {
                let fr = self.board.render(pm);
                self.scaled = fr.scaled;
                self.org = fr.org;
                let linear = fr.scaled.is_some();
                let opts = if linear { egui::TextureOptions::LINEAR } else { egui::TextureOptions::NEAREST };
                let full = fr.damage.len() == 1 && fr.damage[0] == board::IRect { x: 0, y: 0, w, h };
                match self.tex.as_mut() {
                    None => self.tex = Some(ctx.load_texture("board", egui::ColorImage::from_rgba_premultiplied([w as usize, h as usize], pm.data()), opts)),
                    Some(t) if full => t.set(egui::ColorImage::from_rgba_premultiplied([w as usize, h as usize], pm.data()), opts),
                    Some(t) => {
                        // Teilbilder (Zielbild ist ein Ring-Speicher); beim Filterwechsel mindestens ein Pixel,
                        // damit die Option ankommt
                        let mut dmg = fr.damage.clone();
                        if dmg.is_empty() && linear != self.tex_linear {
                            dmg.push(board::IRect { x: 0, y: 0, w: 1, h: 1 });
                        }
                        for d in dmg {
                            for (_, p) in board::torus_parts(d, fr.org, w, h) {
                                let mut buf = Vec::with_capacity(p.w as usize * p.h as usize * 4);
                                let row = w as usize * 4;
                                for y in p.y as usize..(p.y + p.h) as usize {
                                    let o = y * row + p.x as usize * 4;
                                    buf.extend_from_slice(&pm.data()[o..o + p.w as usize * 4]);
                                }
                                t.set_partial([p.x as usize, p.y as usize], egui::ColorImage::from_rgba_premultiplied([p.w as usize, p.h as usize], &buf), opts);
                            }
                        }
                    }
                }
                self.tex_linear = linear;
            }
        }
        if let (Some(t), Some(pm)) = (&self.tex, &self.pix) {
            // Textur ist ein Ring-Speicher: bis zu vier Stücke an ihre Stelle der Ansicht zeichnen
            let (w, h) = (pm.width(), pm.height());
            let (k, off) = match self.scaled {
                Some((k, dx, dy)) => (k, egui::vec2(dx, dy)),
                None => (1.0, egui::Vec2::ZERO),
            };
            let p = ui.painter().with_clip_rect(rect);
            if self.scaled.is_some() {
                let c = self.board.backdrop_color();
                p.rect_filled(rect, 0.0, Color32::from_rgb((c >> 16) as u8, (c >> 8) as u8, c as u8));
            }
            let (sx, sy) = (rect.width() / w as f32, rect.height() / h as f32);
            for (v, q) in board::torus_parts(board::IRect { x: 0, y: 0, w, h }, self.org, w, h) {
                let min = rect.min + off + egui::vec2(v.x as f32 * sx, v.y as f32 * sy) * k;
                let max = rect.min + off + egui::vec2((v.x + v.w) as f32 * sx, (v.y + v.h) as f32 * sy) * k;
                let uv = Rect::from_min_max(Pos2::new(q.x as f32 / w as f32, q.y as f32 / h as f32), Pos2::new((q.x + q.w) as f32 / w as f32, (q.y + q.h) as f32 / h as f32));
                p.image(t.id(), Rect::from_min_max(min, max), uv, Color32::WHITE);
            }
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
                    board::TEXT | board::NOTE => CursorIcon::Text,
                    _ => CursorIcon::Default,
                }
            };
            ctx.set_cursor_icon(cur);
        }

        // Leisten und Panels (Reihenfolge: Docks, Leisten, Popups)
        self.st.popup_rects.clear();
        self.st.tool_rects.clear();
        self.st.dock_rects.clear();
        pages::show(self, &ctx, sh, &lay);
        layers::show(self, &ctx, sh, &lay);
        toolbar::show(self, &ctx, sh, &lay, &tools);
        options::show(self, &ctx, sh, &lay);
        menu::show(self, &ctx, sh, &lay);
        self.text_editor(&ctx);
        self.close_popups_on_esc(&ctx);
        self.drain_events(sh);

        // Autosave
        self.sync_modified(sh.now);
        if !self.board.busy() {
            let Self { autosave, board, doc, .. } = self;
            let fmt = autosave.format();
            autosave.tick(sh.now, || Self::snapshot_bytes(board, doc, fmt), &*sh.platform);
        }
        if self.autosave.error().is_some() {
            if !self.st.error_toast_shown {
                self.st.error_toast_shown = true;
                self.show_save_error(sh);
            }
        } else {
            self.st.error_toast_shown = false;
        }
        self.sync_tool_prefs(&mut sh.settings);

        // Zeitgesteuertes Neuzeichnen (Halte-Timer, Ende von Zoom-Gesten, Autosave)
        let mut wait = 30.0f64;
        if let Some(d) = self.autosave.next_due(sh.now) {
            wait = wait.min(d + 0.02);
        }
        if let Some(d) = self.board.next_deadline() {
            wait = wait.min((d - sh.now).max(0.0) + 0.005);
        }
        if self.board.needs_redraw {
            ctx.request_repaint();
        } else {
            ctx.request_repaint_after(std::time::Duration::from_secs_f64(wait.max(0.02)));
        }
    }

    fn close_popups_on_esc(&mut self, ctx: &egui::Context) {
        if (self.st.opts_open || self.st.menu_open) && self.text.is_none() && ctx.input_mut(|i| i.consume_key(egui::Modifiers::NONE, Key::Escape)) {
            self.st.opts_open = false;
            self.st.menu_open = false;
        }
    }

    /// Werkzeug-Vorgaben (Modus, Breite, …) in den Einstellungen merken.
    fn sync_tool_prefs(&self, s: &mut Settings) {
        let b = &self.board;
        let vals: [(&str, serde_json::Value); 7] = [
            ("arrow", b.arrow_style.into()),
            ("recog", b.recog.into()),
            ("selmode", b.sel_mode.into()),
            ("erasemode", b.erase_mode.into()),
            ("w", serde_json::json!(b.pen_w)),
            ("shape", b.shape_kind.into()),
            ("finger", b.finger_draw.into()),
        ];
        for (k, v) in vals {
            if s.extra.get(k) != Some(&v) {
                s.extra.insert(k.to_string(), v);
            }
        }
    }

    fn drain_events(&mut self, sh: &mut Shared) {
        for e in std::mem::take(&mut self.board.events) {
            match e {
                board::Event::Notice(s) => sh.toast(&s),
                board::Event::AskText { wx, wy, note } => self.start_text(wx, wy, note),
                board::Event::EditNote(id) => self.edit_note(id),
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
        self.hover = hover.filter(|p| on_board(*p));
        for ev in events {
            match ev {
                Event::Touch { id, phase, pos, force, .. } => {
                    // Ein Finger, der auf einer Leiste beginnt, gehört nicht zur Zeichenfläche.
                    // Während der Texteingabe beendet ein Tippen daneben nur die Eingabe.
                    if phase == TouchPhase::Start && (!on_board(pos) || self.text.is_some()) {
                        continue;
                    }
                    self.touch(id.0, phase, rel(pos).to_pos2(), force.unwrap_or(0.5))
                }
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
                            self.pan_start = Some(pos);
                        }
                    } else if self.drawing && button == PointerButton::Primary {
                        self.drawing = false;
                        self.board.on_touch(&Motion::single(Action::Up, r.x, r.y, 0.5, ToolType::Stylus));
                    } else if self.panning.map(|(_, b)| b == button).unwrap_or(false) {
                        self.panning = None;
                        // Mittlere Maustaste ohne Verschieben: Objekt darunter auswählen (erweitertes Menü)
                        let still = self.pan_start.take().map(|s| s.distance(pos) < 10.0).unwrap_or(false);
                        if button == PointerButton::Middle && still && !self.board.select_at_screen(r.x, r.y, false) && self.board.tool == board::SELECT {
                            self.board.deselect();
                        }
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
    fn touch(&mut self, id: u64, phase: TouchPhase, p: Pos2, force: f32) {
        let ptrs = |t: &Vec<(u64, Pos2)>| -> Vec<Pointer> {
            t.iter().map(|(i, q)| Pointer { id: *i as i64, x: q.x, y: q.y, pressure: force, tool: ToolType::Finger }).collect()
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

    pub(super) fn zoom_center(&mut self, f: f32) {
        let (w, h) = (self.board.width(), self.board.height());
        self.board.zoom_at(w / 2.0, h / 2.0, f);
    }

    /// Tastenkürzel.
    fn shortcuts(&mut self, ctx: &egui::Context, sh: &mut Shared) {
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
                            self.sync_modified(sh.now);
                            self.autosave.mark_changed(sh.now);
                            self.flush(sh);
                            sh.toast("Gespeichert");
                        }
                        Key::E if cmd => {
                            self.st.menu_open = true;
                            self.st.opts_open = false;
                        }
                        Key::Delete | Key::Backspace if !repeat => {
                            if sel {
                                self.board.delete_sel();
                            }
                        }
                        _ if cmd || modifiers.alt || repeat => {}
                        Key::P => self.select_tool(sh, Tool::Pen),
                        Key::M => self.select_tool(sh, Tool::Marker),
                        Key::E => self.select_tool(sh, Tool::Eraser),
                        Key::S => self.select_tool(sh, Tool::Select),
                        Key::R => self.select_tool(sh, Tool::Shape),
                        Key::N => {
                            self.board.set_tool(board::NOTE);
                        }
                        Key::T => {
                            self.board.set_tool(board::TEXT);
                        }
                        Key::I => self.select_tool(sh, Tool::Image),
                        Key::Num0 => self.board.fit(),
                        Key::Plus | Key::Equals => self.zoom_center(1.25),
                        Key::Minus => self.zoom_center(0.8),
                        _ => {}
                    }
                }
                _ => {}
            }
        }
        if copy {
            clip::copy(self, &*sh.platform);
        }
        if cut {
            clip::cut(self, &*sh.platform);
        }
        if paste {
            clip::paste(self, &*sh.platform);
        }
    }

    /// Statusanzeige des Autosave für die Leiste.
    pub(super) fn save_state(&self) -> (&'static str, bool) {
        match self.autosave.status() {
            AutosaveStatus::Error(_) => ("Speichern fehlgeschlagen", true),
            AutosaveStatus::Dirty | AutosaveStatus::Saving => ("Änderungen werden gespeichert …", false),
            AutosaveStatus::Saved(_) => ("Gespeichert", false),
            AutosaveStatus::Idle => ("", false),
        }
    }

    pub(super) fn float(ctx: &egui::Context, id: &str, pos: Pos2, pivot: Align2, bounds: Rect, order: Order, content: impl FnOnce(&mut Ui)) -> Rect {
        egui::Area::new(egui::Id::unique(id)).order(order).fixed_pos(pos).pivot(pivot).constrain_to(bounds).show(ctx, content).response.rect
    }
}
