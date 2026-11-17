//! Editor-Bildschirm: Zeichenfläche, Eingabe, Tastenkürzel, Autosave-Anbindung und das Gerüst für Leisten,
//! Optionen, ⋯-Menü, Ebenen und Seiten (je nach Formfaktor Compact/Medium/Expanded).

mod arrange;
pub mod clip;
mod convert;
mod crop;
pub mod drop;
pub mod export;
mod extras;
pub mod keys;
mod glyphs;
mod layers;
mod mathed;
mod calced;
#[cfg(not(target_os = "android"))]
pub mod live;
mod menu;
mod more;
mod options;
pub mod pages;
mod ruler;
mod pins;
mod table;
mod text;
mod toolbar;
pub mod tools;

use crate::app::{PickPurpose, Req, Shared};
use crate::autosave::{Autosave, AutosaveStatus, SaveFormat};
use crate::layout::{self, Env, FormFactor};
use crate::platform::{FileKind, FileRef};
use crate::session;
use crate::settings::{BarPos, Settings};
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
/// Board-Werkzeug der Antipp-Werkzeuge (Füllen, Pipette, Stempel): zeichnet nichts, der Editor wertet den Tipp aus.
pub const TAP: i32 = 100;
/// Stempelgrößen (klein, mittel, groß) in Punkt.
pub const STAMP_SIZES: [(f32, &str); 3] = [(22.0, "Klein"), (40.0, "Mittel"), (68.0, "Groß")];
/// Abstand schwebender Leisten zum Rand der Zeichenfläche.
const M: f32 = 8.0;
/// Innenabstand einer Leiste (Frame-Innenrand 4 + Rahmen 1).
const PAD: f32 = 5.0;
/// Breite der angedockten Panels (Expanded).
const DOCK_LAYERS: f32 = 300.0;
const DOCK_PAGES: f32 = 188.0;

/// Werkzeuge der Leiste. Name, Gruppe, Symbol, Kürzel usw. stehen in der Registry [`tools::TOOLS`].
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Tool {
    Pen,
    Marker,
    Eraser,
    Select,
    Shape,
    Text,
    Note,
    Image,
    Fill,
    Picker,
    Dim,
    Stamp,
    Ruler,
    Laser,
    Space,
    Connector,
    Table,
    Pin,
    Math,
    /// Rechnen: Rechenzeilen tippen, der Kern rechnet aus.
    Calc,
    /// Stift-Modus (Schalter): an = nur Stift/Maus zeichnen, Finger verschieben und zoomen.
    PenMode,
    /// Umwandeln: Handschrift per Lasso/Antippen erkennen und durch Text bzw. Formel ersetzen.
    Convert,
}

impl Tool {
    pub fn from_id(s: &str) -> Option<Tool> {
        tools::def(s).map(|d| d.tool)
    }
    pub fn id(self) -> &'static str {
        tools::of(self).id
    }
    pub fn label(self) -> &'static str {
        tools::of(self).name
    }
    /// Kennung der Tastenkürzel-Aktion (`tool.<id>`).
    pub fn action(self) -> String {
        keys::tool_action(self.id())
    }
    /// Hat das Werkzeug Optionen (Kontext-Popover bzw. Bottom-Sheet)?
    pub fn has_options(self) -> bool {
        !matches!(self, Tool::Image | Tool::Picker | Tool::Laser | Tool::Space | Tool::PenMode)
    }
}

/// Sichtbare Werkzeuge der Leiste (eingeschaltet, in eingestellter Reihenfolge).
pub fn visible_tools(s: &Settings) -> Vec<Tool> {
    tools::bar_tools(s)
}

/// Oberflächenzustand des Editors (Popover, Menü, Panels).
#[derive(Default)]
pub struct UiState {
    pub opts_open: bool,
    pub menu_open: bool,
    /// Popover „Weitere Werkzeuge“ offen.
    pub more_open: bool,
    /// Knopf „+“ (Weitere Werkzeuge) im letzten Frame.
    pub more_btn: Option<Rect>,
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
    /// Haftnotiz hat einen eigenen Knopf (Text ist ausgeschaltet).
    pub note_in_bar: bool,
    /// Aktives Antipp-Werkzeug (gilt, solange `board.tool == TAP`).
    pub tap: Option<Tool>,
    /// Maßlinie als eigenes Werkzeug gewählt (Form-Werkzeug mit `DIM`).
    pub dim_mode: bool,
    /// Zuletzt benutzte Zeichenwerkzeuge (Ziel der Pipette).
    pub last_draw: Option<Tool>,
    /// Werkzeug vor der Pipette.
    pub picker_back: Option<Tool>,
    /// Beginn eines Tipps (Position), None wenn kein Tipp läuft oder er zur Geste wurde.
    pub tap_down: Option<Pos2>,
    /// Füllen-Werkzeug entfernt die Füllung statt zu füllen.
    pub fill_remove: bool,
    pub stamp_kind: i32,
    pub stamp_size: usize,
    /// Werkzeug „Formel“ aktiv (Board-Werkzeug TEXT: Tippen fragt nach einer Formel statt nach Text).
    pub math_tool: bool,
    /// Werkzeug „Rechnen“ aktiv (ebenfalls Board-Werkzeug TEXT).
    pub calc_tool: bool,
    /// Export-Fenster.
    pub export: export::ExportUi,
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
    /// Offener Formel-Editor.
    pub(super) math: Option<mathed::MathEd>,
    /// Offenes Eingabefeld des Werkzeugs „Rechnen“.
    pub(super) calc: Option<calced::CalcEd>,
    /// Offenes Fenster des Werkzeugs „Umwandeln“.
    pub(super) conv: Option<convert::Conv>,
    pub st: UiState,
    pub(super) page_thumbs: pages::PageThumbs,
    /// Lineal/Geodreieck, Laserpointer, Platz schaffen.
    pub(super) ext: extras::Extras,
    canvas: Rect,
    /// Mausposition über der Zeichenfläche (Bildschirmpunkte), Ziel für Einfügen.
    hover: Option<Pos2>,
    /// Ort des letzten Rechtsklicks/Lang-Tippens (Einfügen-Menü).
    menu_pos: Option<Pos2>,
    first_frame: bool,
    /// Beim ersten Frame „Alles zeigen“ (Demo/Abnahme).
    pub fit_on_open: bool,
    /// Board seit dem letzten Abgleich mit der Live-Sitzung geändert (Browser, Server-Boards).
    pub live_dirty: bool,
    /// Live-Zusammenarbeit (nur Browser, Server-Boards `srv:<id>`).
    #[cfg(not(target_os = "android"))]
    pub live: Option<live::Live>,
    /// Verbinder, Tabelle, Kommentar, Zuschnitt (arrange.rs).
    xs: arrange::Xs,
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
        board.shape_kind = Some(geti("shape", 0) as i32).filter(|k| (0..=4).contains(k) || (item::TRIANGLE..=item::DIM).contains(k)).unwrap_or(0);
        board.pen_style = geti("penstyle", 0).clamp(0, item::PS_DOTTED as i64) as u8;
        board.arrow_style = geti("arrow", 0).clamp(0, owndraw_core::item::ARROW_STYLES.len() as i64 - 1) as i32;
        // Auf Touch-Geräten zeichnet der Finger standardmäßig (Schalter im Werkzeug-Popover).
        board.snap_cfg = sh.settings.snap.to_core();
        board.calc = sh.settings.calc.to_core();
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
            math: None,
            calc: None,
            conv: None,
            st: UiState::default(),
            page_thumbs: pages::PageThumbs::default(),
            ext: extras::Extras::default(),
            canvas: Rect::NOTHING,
            hover: None,
            menu_pos: None,
            first_frame: true,
            fit_on_open: false,
            live_dirty: false,
            #[cfg(not(target_os = "android"))]
            live: None,
            xs: arrange::Xs::default(),
        }
    }

    /// Board liegt am Server (`srv:<id>`, Web-App und Desktop)?
    pub fn is_server(&self) -> bool {
        !cfg!(target_os = "android") && matches!(self.file(), Some(FileRef::Uri(u)) if u.starts_with("srv:"))
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
            self.live_dirty = true;
            // Server-Boards speichert der Server bzw. der Autosave per PUT: keine Warnung beim Verlassen
            if !self.is_server() {
                crate::platform::note_unsaved(true);
            }
            self.autosave.mark_changed(now);
            self.page_thumbs.dirty = true;
        }
    }

    pub fn rename(&mut self, name: String, now: f64) {
        self.doc.meta.name = name;
        crate::platform::note_unsaved(true);
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
                    sh.toast("Seiten löschen ist hier noch nicht verfügbar");
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
            PickPurpose::Export(fmt) => match self.export_bytes(fmt) {
                Ok(d) => write(sh, &with_ext(f, fmt.ext()), Some(d), "Export gespeichert"),
                Err(e) => sh.toast(&e),
            },
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
                let mut doc = self.snapshot_doc();
                if crate::platform::WEB {
                    // Download ist im Browser die eigentliche Sicherung: mit Vorschaubild (für die Kachel beim Öffnen)
                    doc.thumb = self.board.thumb().encode_png().ok();
                }
                match session::save_copy_as(&*sh.platform, &doc, &f) {
                    Ok(()) if crate::platform::WEB => {
                        crate::platform::note_unsaved(false);
                        sh.toast("Board heruntergeladen");
                    }
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
        let _ = kind;
        sh.reqs.push(Req::PickSave { purpose: PickPurpose::ExportZip, name: format!("{base}-website.zip"), kind: FileKind::Zip });
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
        if crate::platform::WEB {
            sh.toast("Die Website-Vorschau ist im Browser nicht verfügbar. Exportiere die Website stattdessen als ZIP.");
            return;
        }
        match owndraw_core::export::build_preview(&sh.platform.cache_dir(), &self.board) {
            Some(index) => sh.platform.open_url(&index.display().to_string()),
            None => sh.toast("Vorschau konnte nicht erstellt werden"),
        }
    }

    // ---------------------------------------------------------------- Werkzeuge

    pub fn active_tool(&self) -> Tool {
        if let Some(t) = self.xtool() {
            return t;
        }
        if let Some(t) = self.ext.xtool {
            return t;
        }
        if self.board.tool == TAP {
            if let Some(t) = self.st.tap {
                return t;
            }
        }
        match self.board.tool {
            board::PEN => Tool::Pen,
            board::MARKER => Tool::Marker,
            board::ERASER => Tool::Eraser,
            board::SELECT => Tool::Select,
            board::SHAPE if self.st.dim_mode && self.board.shape_kind == item::DIM => Tool::Dim,
            board::SHAPE => Tool::Shape,
            board::NOTE if self.st.note_in_bar => Tool::Note,
            board::TEXT if self.st.math_tool => Tool::Math,
            board::TEXT if self.st.calc_tool => Tool::Calc,
            _ => Tool::Text,
        }
    }

    /// Werkzeug wählen (Bild öffnet die Dateiauswahl).
    pub fn select_tool(&mut self, sh: &mut Shared, t: Tool) {
        if t == Tool::Image {
            self.pick_image(sh);
            return;
        }
        if t == Tool::PenMode {
            if !crate::platform::PEN_MODE && !self.pen_mode() {
                sh.toast_for("Stift-Modus geht auf Android noch nicht: Stift und Finger sind hier nicht unterscheidbar.", 4.0);
                return;
            }
            self.toggle_pen_mode();
            sh.toast(if self.pen_mode() { "Stift-Modus an: nur der Stift zeichnet, Finger verschieben" } else { "Stift-Modus aus: auch Finger zeichnen" });
            return;
        }
        self.set_tool_ui(t);
    }

    /// Stift-Modus an/aus: an = Finger zeichnen nicht (verschieben/zoomen), Stift und Maus zeichnen.
    pub fn toggle_pen_mode(&mut self) {
        self.board.finger_draw = !self.board.finger_draw;
        self.board.invalidate();
    }

    /// Stift-Modus an? (Finger zeichnen nicht)
    pub fn pen_mode(&self) -> bool {
        !self.board.finger_draw
    }

    /// Wählt ein Werkzeug (ohne Bild, das die Dateiauswahl braucht).
    fn set_tool_ui(&mut self, t: Tool) {
        let cur = self.active_tool();
        if t != Tool::Ruler && t != Tool::PenMode {
            self.leave_xtool();
        }
        match t {
            Tool::Ruler => return self.toggle_ruler(),
            Tool::PenMode => return self.toggle_pen_mode(),
            Tool::Laser | Tool::Space | Tool::Convert => {
                self.ext.xtool = Some(t);
                self.board.invalidate();
                return;
            }
            _ => self.ext.xtool = None,
        }
        if matches!(cur, Tool::Pen | Tool::Marker | Tool::Shape | Tool::Dim) {
            self.st.last_draw = Some(cur);
        }
        if t == Tool::Picker && cur != Tool::Picker {
            self.st.picker_back = Some(cur);
        }
        self.st.dim_mode = t == Tool::Dim;
        let bt = match t {
            Tool::Fill | Tool::Picker | Tool::Stamp => {
                self.st.tap = Some(t);
                self.st.tap_down = None;
                TAP
            }
            Tool::Dim => {
                self.board.shape_kind = item::DIM;
                board::SHAPE
            }
            Tool::Pen => board::PEN,
            Tool::Marker => board::MARKER,
            Tool::Eraser => board::ERASER,
            Tool::Select => board::SELECT,
            Tool::Shape => board::SHAPE,
            Tool::Text => {
                if self.board.tool == board::NOTE && !self.st.note_in_bar {
                    board::NOTE
                } else {
                    board::TEXT
                }
            }
            Tool::Note => board::NOTE,
            Tool::Image | Tool::Ruler | Tool::Laser | Tool::Space | Tool::PenMode | Tool::Convert => return,
            Tool::Connector | Tool::Table | Tool::Pin => {
                self.select_xtool(t);
                return;
            }
            Tool::Math | Tool::Calc => board::TEXT,
        };
        self.st.math_tool = t == Tool::Math;
        self.st.calc_tool = t == Tool::Calc;
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
        // Modernes Design: am PC Werkzeuge standardmäßig links (wie auf dem Tablet), Platz für die Kopfzeile oben
        if crate::style::modern() && sh.settings.toolbar_pos.is_none() && env.ff == crate::layout::FormFactor::Expanded {
            tb = BarPos::Left;
        }
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
        let tools: Vec<Tool> = if self.st.edit_tools.is_some() { tools::ordered(&sh.settings).iter().map(|d| d.tool).collect() } else { visible_tools(&sh.settings) };
        if self.st.edit_tools.is_none() {
            self.st.note_in_bar = tools.contains(&Tool::Note);
        }
        // Knopf „+“ (Weitere Werkzeuge), solange Werkzeuge ausgeschaltet sind
        let more = usize::from(self.st.edit_tools.is_none() && !more::hidden(&sh.settings).is_empty());
        let lay = self.layout(sh, ui.max_rect(), tools.len() + more);
        let rect = lay.canvas;
        self.canvas = rect;
        let layer = ui.layer_id();
        let resp = ui.allocate_rect(rect, Sense::click_and_drag());
        let ppp = ctx.pixels_per_point();
        self.board.set_size(rect.width(), rect.height(), ppp);
        self.board.tick(sh.now);
        self.ext_frame(sh.now);
        if self.first_frame {
            self.first_frame = false;
            if self.fit_on_open {
                self.board.fit();
            }
            pages::initial_view(&mut self.board);
        }

        let modal = sh.dialog.is_some() || self.st.edit_tools.is_some() || self.math.is_some() || self.calc.is_some() || self.conv.is_some();
        // Klick auf die Zeichenfläche schließt offene Popups und zeichnet nicht.
        let popup_open = self.st.opts_open || self.st.menu_open || self.st.more_open || self.st.export.open;
        let mut swallow = false;
        if popup_open && !modal {
            let press = ctx.input(|i| i.pointer.any_pressed()).then(|| ctx.input(|i| i.pointer.interact_pos())).flatten();
            let touch = ctx.input(|i| i.events.iter().find_map(|e| if let Event::Touch { phase: TouchPhase::Start, pos, .. } = e { Some(*pos) } else { None }));
            if let Some(p) = press.or(touch) {
                let on_canvas = rect.contains(p) && ctx.layer_id_at(p).map(|l| l == layer).unwrap_or(true);
                if on_canvas {
                    self.st.opts_open = false;
                    self.st.menu_open = false;
                    self.st.more_open = false;
                    self.st.export.open = false;
                    swallow = true;
                }
            }
        }
        if !modal && !swallow {
            let pan_key = ctx.input(|i| keys::held(i, &sh.settings, "pan"));
            self.handle_pointer(&ctx, rect, layer, pan_key);
        }
        if !modal && self.text.is_none() && self.xs.crop.is_none() && !ctx.egui_wants_keyboard_input() {
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
            let space = ctx.input(|i| keys::held(i, &sh.settings, "pan")) && !ctx.egui_wants_keyboard_input();
            let cur = if self.panning.is_some() {
                CursorIcon::Grabbing
            } else if space {
                CursorIcon::Grab
            } else if let Some(t) = self.ext.xtool {
                if t == Tool::Space {
                    CursorIcon::ResizeVertical
                } else {
                    CursorIcon::Crosshair
                }
            } else if self.xs.tool.is_some() {
                CursorIcon::Crosshair
            } else {
                match self.board.tool {
                    board::PEN | board::MARKER | board::SHAPE | board::ERASER | TAP => CursorIcon::Crosshair,
                    board::TEXT | board::NOTE => CursorIcon::Text,
                    _ => CursorIcon::Default,
                }
            };
            ctx.set_cursor_icon(cur);
        }
        self.ext_paint(ui, &ctx, rect);

        // Leisten und Panels (Reihenfolge: Docks, Leisten, Popups)
        self.st.popup_rects.clear();
        self.st.tool_rects.clear();
        self.st.dock_rects.clear();
        pages::show(self, &ctx, sh, &lay);
        layers::show(self, &ctx, sh, &lay);
        toolbar::show(self, &ctx, sh, &lay, &tools);
        options::show(self, &ctx, sh, &lay);
        menu::show(self, &ctx, sh, &lay);
        export::show(self, &ctx, sh, &lay);
        more::show(self, &ctx, sh, &lay);
        self.x_frame(&ctx, sh, &lay, layer);
        self.text_editor(&ctx, &sh.settings);
        self.math_editor(&ctx, sh);
        self.calc_editor(&ctx, sh);
        self.convert_frame(&ctx, sh);
        self.close_popups_on_esc(&ctx, &sh.settings);
        self.drain_events(sh);

        // Autosave
        self.sync_modified(sh.now);
        #[cfg(not(target_os = "android"))]
        self.live_frame(ui, &ctx, sh, &lay);
        if !self.board.busy() {
            let Self { autosave, board, doc, .. } = self;
            let fmt = autosave.format();
            let before = autosave.status().clone();
            autosave.tick(sh.now, || Self::snapshot_bytes(board, doc, fmt), &*sh.platform);
            // Synchrones Schreiben (URI, Browser) ändert den Status erst nach dem Zeichnen der Leiste
            if *autosave.status() != before {
                ctx.request_repaint();
            }
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
        let mut snap = sh.settings.snap.to_core();
        if sh.settings.snap.key.held(&ctx.input(|i| i.modifiers)) {
            snap = snap.inverted();
        }
        self.board.set_snap_cfg(snap);
        let calc = sh.settings.calc.to_core();
        if self.board.calc != calc {
            self.board.calc = calc;
            self.board.invalidate();
        }

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

    fn close_popups_on_esc(&mut self, ctx: &egui::Context, s: &Settings) {
        if (self.st.opts_open || self.st.menu_open || self.st.more_open || self.st.export.open) && self.text.is_none() && ctx.input_mut(|i| keys::consume(i, s, "cancel")) {
            self.st.opts_open = false;
            self.st.menu_open = false;
            self.st.more_open = false;
            self.st.export.open = false;
        }
    }

    /// Werkzeug-Vorgaben (Modus, Breite, …) in den Einstellungen merken.
    fn sync_tool_prefs(&self, s: &mut Settings) {
        let b = &self.board;
        let vals: [(&str, serde_json::Value); 8] = [
            ("arrow", b.arrow_style.into()),
            ("recog", self.ext.saved_recog.unwrap_or(b.recog).into()),
            ("selmode", b.sel_mode.into()),
            ("erasemode", b.erase_mode.into()),
            ("w", serde_json::json!(b.pen_w)),
            ("shape", b.shape_kind.into()),
            ("finger", b.finger_draw.into()),
            ("penstyle", b.pen_style.into()),
        ];
        for (k, v) in vals {
            // Verbinder setzt Form/Pfeilstil nur vorübergehend
            if self.xs.saved.is_some() && (k == "shape" || k == "arrow") {
                continue;
            }
            if s.extra.get(k) != Some(&v) {
                s.extra.insert(k.to_string(), v);
            }
        }
    }

    fn drain_events(&mut self, sh: &mut Shared) {
        for e in std::mem::take(&mut self.board.events) {
            match e {
                board::Event::Notice(s) => sh.toast(&s),
                board::Event::AskText { wx, wy, .. } if self.st.math_tool => match self.board.math_at(wx, wy) {
                    Some(id) => self.edit_math(id),
                    None => self.start_math(wx, wy),
                },
                board::Event::AskText { wx, wy, .. } if self.st.calc_tool => self.start_calc(sh, wx, wy),
                board::Event::AskText { wx, wy, note } => self.start_text(wx, wy, note),
                board::Event::EditNote(id) if self.board.items.iter().any(|it| it.id == id && it.as_math().is_some()) => self.edit_math(id),
                board::Event::EditNote(id) => self.edit_note(id),
                board::Event::Changed | board::Event::Haptic => {}
            }
        }
    }

    // ---------------------------------------------------------------- Eingabe

    fn handle_pointer(&mut self, ctx: &egui::Context, rect: Rect, layer: egui::LayerId, pan_key: bool) {
        let events = ctx.input(|i| i.events.clone());
        let hover = ctx.input(|i| i.pointer.hover_pos());
        let space = pan_key && !ctx.egui_wants_keyboard_input() && self.text.is_none();
        let on_board = |p: Pos2| rect.contains(p) && ctx.layer_id_at(p).map(|l| l == layer).unwrap_or(true);
        let has_touch = !self.touches.is_empty() || events.iter().any(|e| matches!(e, Event::Touch { .. }));
        let rel = |p: Pos2| p - rect.min;
        let now = ctx.input(|i| i.time);
        self.hover = hover.filter(|p| on_board(*p));
        for ev in events {
            match ev {
                Event::Touch { id, phase, pos, force, .. } => {
                    // Ein Finger, der auf einer Leiste beginnt, gehört nicht zur Zeichenfläche.
                    // Während der Texteingabe beendet ein Tippen daneben nur die Eingabe.
                    if phase == TouchPhase::Start && (!on_board(pos) || self.text.is_some()) {
                        continue;
                    }
                    if self.x_touch(id.0, phase, rel(pos).to_pos2(), now) {
                        continue;
                    }
                    let ph = match phase {
                        TouchPhase::Start => extras::Ph::Down,
                        TouchPhase::Move => extras::Ph::Move,
                        TouchPhase::End => extras::Ph::Up,
                        TouchPhase::Cancel => extras::Ph::Cancel,
                    };
                    // Stift auf dem Touchscreen (Browser): zählt wie Maus/Stift, nicht wie ein Finger
                    let pen = crate::platform::pen_touch(id.0);
                    match self.ext_hook(id.0, ph, rel(pos).to_pos2(), !pen) {
                        extras::Hook::Eat => {}
                        extras::Hook::Pass(q) => self.touch(id.0, phase, q, force.unwrap_or(0.5)),
                    }
                }
                _ if has_touch && matches!(ev, Event::PointerMoved(_) | Event::PointerButton { .. }) => {}
                Event::PointerMoved(p) => {
                    self.x_move(arrange::MOUSE, rel(p).to_pos2());
                    let r = match self.ext_hook(extras::MOUSE, extras::Ph::Move, rel(p).to_pos2(), false) {
                        extras::Hook::Eat => continue,
                        extras::Hook::Pass(q) => q.to_vec2(),
                    };
                    if self.drawing {
                        self.touch_in(&Motion::single(Action::Move, r.x, r.y, 0.5, ToolType::Stylus));
                    } else if let Some((last, b)) = self.panning {
                        self.board.pan_by(p.x - last.x, p.y - last.y);
                        self.panning = Some((p, b));
                    }
                }
                Event::PointerButton { pos, button, pressed, .. } => {
                    let r = rel(pos);
                    if pressed {
                        if self.drawing || self.panning.is_some() || !on_board(pos) || self.text.is_some() || self.ext_grabs(extras::MOUSE) {
                            continue;
                        }
                        if button == PointerButton::Primary && !space && self.x_down(arrange::MOUSE, r.to_pos2(), now) {
                            // Tabelle/Kommentar setzen, Kommentar öffnen: Tippen; Ziehen verschiebt die Ansicht
                            self.panning = Some((pos, button));
                            self.pan_start = Some(pos);
                        } else if button == PointerButton::Primary && !space {
                            let r = match self.ext_hook(extras::MOUSE, extras::Ph::Down, r.to_pos2(), false) {
                                extras::Hook::Eat => continue,
                                extras::Hook::Pass(q) => q.to_vec2(),
                            };
                            self.drawing = true;
                            self.touch_in(&Motion::single(Action::Down, r.x, r.y, 0.5, ToolType::Stylus));
                        } else if matches!(button, PointerButton::Primary | PointerButton::Secondary | PointerButton::Middle) {
                            self.panning = Some((pos, button));
                            self.pan_start = Some(pos);
                        }
                    } else if button == PointerButton::Primary && self.ext_grabs(extras::MOUSE) {
                        self.ext_hook(extras::MOUSE, extras::Ph::Up, r.to_pos2(), false);
                    } else if self.drawing && button == PointerButton::Primary {
                        let r = match self.ext_hook(extras::MOUSE, extras::Ph::Up, r.to_pos2(), false) {
                            extras::Hook::Pass(q) => q.to_vec2(),
                            extras::Hook::Eat => r,
                        };
                        self.drawing = false;
                        self.touch_in(&Motion::single(Action::Up, r.x, r.y, 0.5, ToolType::Stylus));
                        self.x_up(arrange::MOUSE, r.to_pos2(), true, now);
                    } else if self.panning.map(|(_, b)| b == button).unwrap_or(false) {
                        if button == PointerButton::Primary {
                            self.x_up(arrange::MOUSE, r.to_pos2(), true, now);
                        }
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
            t.iter()
                .map(|(i, q)| {
                    let tool = if crate::platform::pen_touch(*i) { ToolType::Stylus } else { ToolType::Finger };
                    Pointer { id: *i as i64, x: q.x, y: q.y, pressure: force, tool }
                })
                .collect()
        };
        match phase {
            TouchPhase::Start => {
                self.touches.retain(|t| t.0 != id);
                self.touches.push((id, p));
                let action = if self.touches.len() == 1 { Action::Down } else { Action::PointerDown(self.touches.len() - 1) };
                self.touch_in(&Motion { action, pointers: ptrs(&self.touches), history: Vec::new(), stylus_button: false });
            }
            TouchPhase::Move => {
                if let Some(t) = self.touches.iter_mut().find(|t| t.0 == id) {
                    t.1 = p;
                    self.touch_in(&Motion { action: Action::Move, pointers: ptrs(&self.touches), history: Vec::new(), stylus_button: false });
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
                self.touch_in(&Motion { action, pointers: ptrs(&self.touches), history: Vec::new(), stylus_button: false });
                self.touches.remove(i);
            }
        }
    }

    /// Bewegung ans Board; bei Antipp-Werkzeugen wird zusätzlich der Tipp erkannt (ohne Ziehen, ohne zweiten Finger).
    fn touch_in(&mut self, m: &Motion) {
        if self.board.tool == TAP {
            let p = m.pointers.first().map(|p| Pos2::new(p.x, p.y));
            match (m.action, p) {
                (Action::Down, Some(p)) => self.st.tap_down = Some(p),
                (Action::Move, Some(p)) => {
                    if self.st.tap_down.is_some_and(|d| d.distance(p) > 10.0) {
                        self.st.tap_down = None;
                    }
                }
                (Action::Up, Some(p)) => {
                    if let Some(d) = self.st.tap_down.take() {
                        if d.distance(p) <= 10.0 {
                            self.board.on_touch(m);
                            self.tap_at(p.x, p.y);
                            return;
                        }
                    }
                }
                (Action::PointerDown(_) | Action::Cancel, _) => self.st.tap_down = None,
                _ => {}
            }
        }
        self.board.on_touch(m);
    }

    /// Antipp-Werkzeug ausführen: Punkt in Board-Bildschirmkoordinaten.
    fn tap_at(&mut self, x: f32, y: f32) {
        let Some(t) = self.st.tap else { return };
        let (wx, wy) = self.board.screen_to_world(x, y);
        let note = |b: &mut Board, s: &str| b.events.push(board::Event::Notice(s.to_string()));
        match t {
            Tool::Fill => {
                let col = if self.st.fill_remove { None } else { Some(self.board.color) };
                if self.board.fill_at(wx, wy, col).is_none() {
                    note(&mut self.board, "Hier ist keine geschlossene Form");
                }
            }
            Tool::Picker => match self.board.color_at(wx, wy) {
                Some(c) => {
                    let c = c | 0xFF000000;
                    self.board.set_color(c);
                    note(&mut self.board, &format!("Farbe #{:06X} übernommen", c & 0xFFFFFF));
                    let back = self.st.picker_back.take().filter(|b| *b != Tool::Picker).or(self.st.last_draw).unwrap_or(Tool::Pen);
                    self.set_tool_ui(back);
                }
                None => note(&mut self.board, "Hier ist keine Farbe"),
            },
            Tool::Stamp => {
                let size = STAMP_SIZES[self.st.stamp_size.min(STAMP_SIZES.len() - 1)].0 * self.board.dp;
                if self.board.stamp(self.st.stamp_kind, wx, wy, size).is_empty() {
                    note(&mut self.board, "Hier kann nicht gestempelt werden (Ebene gesperrt)");
                }
            }
            _ => {}
        }
    }

    pub(super) fn zoom_center(&mut self, f: f32) {
        let (w, h) = (self.board.width(), self.board.height());
        self.board.zoom_at(w / 2.0, h / 2.0, f);
    }

    /// Tastenkürzel: Nachschlag in der Belegung (`keys`, Einstellungen „Tastenkürzel“).
    fn shortcuts(&mut self, ctx: &egui::Context, sh: &mut Shared) {
        use keys::{Chord, Scope};
        let events = ctx.input(|i| i.events.clone());
        let mut fired: Vec<&'static keys::Action> = Vec::new();
        let ctrl = |k: Key| Chord { key: k, ctrl: true, alt: false, shift: false };
        for ev in events {
            // Kopieren/Ausschneiden/Einfügen kommen je nach Plattform als eigene Ereignisse (= Strg+C/X/V)
            let (chord, repeat, clip_event) = match ev {
                Event::Copy => (ctrl(Key::C), false, true),
                Event::Cut => (ctrl(Key::X), false, true),
                Event::Paste(_) => (ctrl(Key::V), false, true),
                Event::Key { key, pressed: true, modifiers, repeat, .. } => (Chord::from_event(key, &modifiers), repeat, false),
                _ => continue,
            };
            let Some(a) = keys::lookup(&sh.settings, chord, Scope::Editor) else { continue };
            if repeat && !a.repeat {
                continue;
            }
            // Im Browser kommt der Inhalt mit dem `paste`-Ereignis (Event::Paste), sonst doppelt.
            if a.id == "paste" && !clip_event && crate::platform::WEB && chord == ctrl(Key::V) {
                continue;
            }
            // Strg+C kommt oft als Taste UND als Event::Copy: Zwischenablage-Aktionen nur einmal je Frame
            if matches!(a.id.as_str(), "copy" | "cut" | "paste") && fired.iter().any(|f| f.id == a.id) {
                continue;
            }
            fired.push(a);
        }
        for a in fired {
            self.run_action(sh, a);
        }
    }

    /// Führt eine Aktion der Tastenkürzel aus.
    fn run_action(&mut self, sh: &mut Shared, a: &keys::Action) {
        let sel = self.board.tool == board::SELECT && !self.board.sel.is_empty();
        if let Some(t) = a.tool {
            match t {
                // T/N wählen Text bzw. Haftnotiz direkt (wie bisher)
                Tool::Text | Tool::Note => {
                    self.ext.xtool = None;
                    self.st.math_tool = false;
                    self.st.calc_tool = false;
                    self.board.set_tool(if t == Tool::Text { board::TEXT } else { board::NOTE });
                }
                _ => self.select_tool(sh, t),
            }
            return;
        }
        match a.id.as_str() {
            "group" | "ungroup" => arrange::run(self, &a.id),
            "undo" => self.board.undo(),
            "redo" => self.board.redo(),
            "copy" => clip::copy(self, &*sh.platform),
            "cut" => clip::cut(self, &*sh.platform),
            "paste" => clip::paste(self, &*sh.platform),
            "duplicate" => {
                if sel {
                    self.board.duplicate_sel();
                }
            }
            "delete" => {
                if sel {
                    self.board.delete_sel();
                }
            }
            "save" => {
                self.sync_modified(sh.now);
                self.autosave.mark_changed(sh.now);
                self.flush(sh);
                if crate::platform::WEB && !self.is_server() {
                    // Browser: Speichern lädt das Board herunter (Server-Boards speichern selbst).
                    self.apply_confirm(ConfirmAction::SaveCopy, sh);
                } else {
                    sh.toast("Gespeichert");
                }
            }
            "menu" => {
                self.st.menu_open = true;
                self.st.opts_open = false;
                self.st.export.open = false;
            }
            "export" => {
                self.st.export.open = true;
                self.st.menu_open = false;
                self.st.opts_open = false;
                self.st.more_open = false;
            }
            "help" => sh.reqs.push(crate::app::Req::OpenTutorial),
            "fit" => self.board.fit(),
            "zoom_in" => self.zoom_center(1.25),
            "zoom_out" => self.zoom_center(0.8),
            // "cancel": close_popups_on_esc; "pan"/"snap": gehalten; "text_done": Texteingabe
            _ => {}
        }
    }

    /// Statusanzeige des Autosave für die Leiste.
    pub(super) fn save_state(&self) -> (&'static str, bool) {
        #[cfg(not(target_os = "android"))]
        if let Some(st) = self.srv_save_state() {
            return st;
        }
        match self.autosave.status() {
            AutosaveStatus::Error(_) => ("Speichern fehlgeschlagen", true),
            AutosaveStatus::Dirty | AutosaveStatus::Saving => ("Änderungen werden gespeichert …", false),
            AutosaveStatus::Saved(_) if crate::platform::WEB => ("Im Browser gespeichert. Lade das Board herunter, um es zu sichern.", false),
            AutosaveStatus::Saved(_) => ("Gespeichert", false),
            AutosaveStatus::Idle => ("", false),
        }
    }

    pub(super) fn float(ctx: &egui::Context, id: &str, pos: Pos2, pivot: Align2, bounds: Rect, order: Order, content: impl FnOnce(&mut Ui)) -> Rect {
        egui::Area::new(egui::Id::unique(id)).order(order).fixed_pos(pos).pivot(pivot).constrain_to(bounds).show(ctx, content).response.rect
    }
}

#[cfg(not(target_os = "android"))]
impl Editor {
    /// Live-Sitzung für Server-Boards: Verbindung, Abgleich, fremde Zeiger, Leiste mit Teilnehmern.
    fn live_frame(&mut self, ui: &Ui, ctx: &egui::Context, sh: &mut Shared, lay: &Lay) {
        let Some(id) = self.file().and_then(crate::net::srv_id).map(String::from) else { return };
        let mut lv = self.live.take().unwrap_or_else(|| live::Live::new(&id, self));
        lv.frame(self, ctx, sh);
        lv.paint_cursors(ui, self, sh.now);
        if self.st.edit_tools.is_none() {
            lv.overlay(self, ctx, sh, lay);
        }
        self.live = Some(lv);
        // Speicherfehler des Autosave per PUT (kommen asynchron) einmal anzeigen
        let err = crate::net::with(|n| n.save_errors.get(&id).cloned());
        match err {
            Some(e) if !self.st.error_toast_shown => {
                self.st.error_toast_shown = true;
                sh.toast_for(&e, 5.0);
            }
            None if self.autosave.error().is_none() => self.st.error_toast_shown = false,
            _ => {}
        }
    }

    /// Speicheranzeige für Server-Boards.
    fn srv_save_state(&self) -> Option<(&'static str, bool)> {
        use crate::net as api;
        let id = self.file().and_then(api::srv_id)?;
        if let Some(l) = self.live.as_ref().filter(|l| l.is_live()) {
            return Some(l.status());
        }
        let (conflict, err) = api::with(|n| (n.conflicts.contains(id), n.save_errors.contains_key(id)));
        Some(if conflict {
            ("Konflikt. Das Board wurde anderswo geändert", true)
        } else if err {
            ("Speichern auf dem Server fehlgeschlagen", true)
        } else if api::saving(id) || matches!(self.autosave.status(), AutosaveStatus::Dirty | AutosaveStatus::Saving) {
            ("Änderungen werden gespeichert …", false)
        } else if let Some(l) = &self.live {
            l.status()
        } else {
            ("Auf dem Server gespeichert", false)
        })
    }
}
