//! Hauptfenster: Bildschirmwechsel (Start, Editor, Einstellungen), Dokumente öffnen/anlegen, Dialog-Ergebnisse,
//! Hinweise (Toast) und das Anwenden der Einstellungen. Alle Plattform-Zugriffe laufen über `Platform`.

use crate::layout::Env;
use crate::autosave::{AutosaveStatus, SaveFormat};
use crate::platform::{FileKind, FileRef, Platform, PlatformEvent, Tag};
use crate::recent::{Recent, RecentEntry};
use crate::session::{self, NewParams};
use crate::settings::{SaveChoice, Settings};
use crate::style::{self, c, Icons};
use crate::ui::dialogs::{self, Dialog, Outcome};
use crate::ui::editor::{drop as drops, Editor};
use crate::ui::settings::SettingsAction;
use crate::ui::start::StartState;
use egui::{Align2, Color32, CornerRadius, Vec2};
use owndraw_core::doc::Document;
use std::collections::HashMap;
use std::path::PathBuf;

/// Wofür ein System-Dialog geöffnet wurde.
#[derive(Clone, Debug, PartialEq)]
pub enum PickPurpose {
    ExportPng,
    ExportPdf,
    ExportZip,
    ExportLayer(usize),
    SaveCopy,
    ExportXopp,
    ImportImage,
    OpenDoc,
    NewDoc,
    /// Standard-Ort der Einstellungen.
    DefaultDir,
}

/// Aufträge der Oberfläche an die Shell (werden am Frameende abgearbeitet).
pub enum Req {
    /// Zurück zum Start (Dokument wird gespeichert).
    Back,
    OpenSettings,
    /// Bearbeiten-Modus der Werkzeugleiste beendet (zurück zu den Einstellungen).
    ToolsEdited,
    NewDialog,
    OpenFile(FileRef),
    RemoveRecent(FileRef),
    PickOpen { purpose: PickPurpose, kinds: Vec<FileKind> },
    PickSave { purpose: PickPurpose, name: String, kind: FileKind },
}

/// Von allen Bildschirmen gemeinsam genutzter Zustand.
pub struct Shared {
    pub platform: Box<dyn Platform>,
    pub settings: Settings,
    pub icons: Icons,
    pub env: Env,
    /// UI-Uhr in Sekunden.
    pub now: f64,
    toast: Option<(String, f64)>,
    pub dialog: Option<Dialog>,
    pub reqs: Vec<Req>,
}

impl Shared {
    pub fn toast(&mut self, s: &str) {
        self.toast_for(s, 2.2);
    }
    pub fn toast_for(&mut self, s: &str, secs: f64) {
        self.toast = Some((s.to_string(), self.now + secs));
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Screen {
    Start,
    Editor,
    Settings,
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

/// Lädt `settings.json` aus dem Konfigurationsordner (fehlt sie oder ist sie kaputt: Vorgaben).
pub fn load_settings(platform: &dyn Platform) -> Settings {
    Settings::load(platform)
}

pub struct App {
    pub sh: Shared,
    screen: Screen,
    /// Bildschirm vor den Einstellungen.
    back_screen: Screen,
    editor: Option<Editor>,
    start: StartState,
    recent: Recent,
    pending: HashMap<Tag, (PickPurpose, Option<NewParams>)>,
    /// Importiertes Dokument, das auf die Wahl seines Speicherorts wartet (Tag des Speichern-Dialogs).
    pending_docs: HashMap<Tag, Document>,
    /// Zeitpunkt des letzten Autosave-Erfolgs, der in „Zuletzt“ eingetragen wurde.
    last_saved: f64,
    /// Zeitpunkt der letzten Prüfung auf fehlende Dateien der Zuletzt-Liste.
    last_missing_check: f64,
    next_tag: Tag,
    applied: Option<Settings>,
    /// Editor wurde nur für den Werkzeug-Bearbeiten-Modus angelegt (leeres Board ohne Datei).
    scratch_editor: bool,
    /// Bearbeiten-Modus wurde aus den Einstellungen gestartet.
    edit_from_settings: bool,
    open_at_start: Option<FileRef>,
    /// Screenshot-Hilfe für die Abnahme (Umgebungsvariablen `OWNDRAW_SCREENSHOT`, `OWNDRAW_DEMO`).
    shot: Option<crate::demo::Shot>,
    demo: Option<String>,
}

impl App {
    pub fn new(platform: Box<dyn Platform>, settings: Settings, open_file: Option<FileRef>) -> App {
        let recent = Recent::load(&platform.config_dir());
        App {
            sh: Shared { platform, settings, icons: Icons::default(), env: Env::new(1000.0, false), now: 0.0, toast: None, dialog: None, reqs: Vec::new() },
            screen: Screen::Start,
            back_screen: Screen::Start,
            editor: None,
            start: StartState::default(),
            recent,
            pending: HashMap::new(),
            pending_docs: HashMap::new(),
            last_saved: -1.0,
            last_missing_check: -100.0,
            next_tag: 1,
            applied: None,
            scratch_editor: false,
            edit_from_settings: false,
            open_at_start: open_file,
            shot: crate::demo::Shot::from_env(),
            demo: std::env::var("OWNDRAW_DEMO").ok().filter(|s| !s.is_empty()),
        }
    }

    /// Migration der alten Ablage (siehe `lib.rs`): übernommene Dateien in die Zuletzt-Liste, Hinweis anzeigen.
    #[cfg_attr(target_arch = "wasm32", allow(dead_code))]
    pub fn add_migrated(&mut self, files: Vec<PathBuf>) {
        let n = files.len();
        for p in files {
            let f = FileRef::Path(p);
            let (name, pages) = session::read_summary(&*self.sh.platform, &f).unwrap_or_else(|| (f.display_name(), false));
            self.recent.add(RecentEntry { file: f, name, modified_ms: owndraw_core::store::now_ms(), pages });
        }
        if n > 0 {
            self.sh.toast_for(&format!("{n} alte Boards übernommen"), 5.0);
        }
    }

    /// Server-Boards (Browser, `srv:<id>`) stehen in der Liste „Am Server“, nicht unter „Zuletzt geöffnet“.
    fn in_recent(f: &FileRef) -> bool {
        !matches!(f, FileRef::Uri(u) if crate::platform::WEB && u.starts_with("srv:"))
    }

    fn recent_entry(f: FileRef, e: &Editor) -> RecentEntry {
        RecentEntry { file: f, name: e.name().to_string(), modified_ms: owndraw_core::store::now_ms(), pages: crate::ui::editor::pages::is_pages(&e.board) }
    }

    /// Speichert das offene Dokument sofort (Beenden, Pause).
    fn flush_editor(&mut self) {
        if let Some(e) = self.editor.as_mut() {
            e.flush(&mut self.sh);
        }
    }

    fn title(&self, ctx: &egui::Context) {
        let t = match (&self.editor, self.screen) {
            (Some(e), Screen::Editor) => format!("OwnDraw – {}", e.name()),
            _ => "OwnDraw".to_string(),
        };
        // Browser: Seitentitel (das Web-Backend kennt ViewportCommand::Title nicht)
        #[cfg(target_arch = "wasm32")]
        if let Some(d) = web_sys::window().and_then(|w| w.document()) {
            d.set_title(&t);
            return;
        }
        ctx.send_viewport_cmd(egui::ViewportCommand::Title(t));
    }

    fn tag(&mut self, purpose: PickPurpose, params: Option<NewParams>) -> Tag {
        let t = self.next_tag;
        self.next_tag += 1;
        self.pending.insert(t, (purpose, params));
        t
    }

    // ---------------------------------------------------------------- Dokumente

    fn default_dir(&self) -> FileRef {
        self.sh.settings.default_dir.as_deref().and_then(FileRef::from_key).unwrap_or_else(|| self.sh.platform.default_dir())
    }

    /// Schließt das offene Dokument (speichert zuerst) und merkt es in „Zuletzt“.
    fn close_editor(&mut self, ctx: &egui::Context) {
        if let Some(mut e) = self.editor.take() {
            e.flush(&mut self.sh);
            // Browser: zwischengespeicherte Bytes eines Server-Boards freigeben (beim Öffnen neu geladen)
            #[cfg(target_arch = "wasm32")]
            if let Some(FileRef::Uri(u)) = e.file().filter(|f| crate::platform::web::api::srv_id(f).is_some()) {
                crate::platform::web::api::with(|n| n.files.remove(u));
            }
            if let Some(f) = e.file().cloned().filter(Self::in_recent) {
                self.recent.add(Self::recent_entry(f.clone(), &e));
                self.start.forget(&f);
            }
        }
        // Browser: Liste der Server-Boards und Speicherplatz auffrischen
        #[cfg(target_arch = "wasm32")]
        {
            crate::platform::web::api::refresh_boards();
            crate::platform::web::api::refresh_me();
        }
        self.screen = Screen::Start;
        self.title(ctx);
    }

    fn enter_editor(&mut self, ctx: &egui::Context, ed: Editor) {
        if self.editor.is_some() {
            self.close_editor(ctx);
        }
        if let Some(f) = ed.file().cloned().filter(Self::in_recent) {
            self.recent.add(Self::recent_entry(f, &ed));
        }
        self.editor = Some(ed);
        self.screen = Screen::Editor;
        self.title(ctx);
    }

    /// Öffnet `.owndraw` oder importiert `.xopp`.
    pub fn open_ref(&mut self, ctx: &egui::Context, f: FileRef) {
        if self.editor.as_ref().and_then(|e| e.file()) == Some(&f) {
            self.screen = Screen::Editor;
            return;
        }
        let doc = match session::open_document(&*self.sh.platform, &f) {
            Ok(d) => d,
            Err(e) => {
                self.sh.toast_for(&e, 4.0);
                if !self.sh.platform.exists(&f) {
                    self.recent.remove(&f);
                }
                return;
            }
        };
        let mut doc = doc;
        if FileKind::of_name(&f.display_name()) == FileKind::Xopp {
            // Xournal++-Import: Original bleibt unverändert; Autosave schreibt `<name>.owndraw` daneben
            // (oder direkt ins .xopp, wenn eingestellt).
            if doc.meta.name.is_empty() {
                doc.meta.name = f.display_name().trim_end_matches(".xopp").trim_end_matches(".XOPP").to_string();
            }
            if self.sh.settings.xopp_autosave_direct {
                let mut ed = Editor::open(&mut self.sh, doc, Some(f));
                ed.autosave.set_format(SaveFormat::Xopp);
                self.sh.toast("Xournal++-Datei geöffnet (Änderungen gehen direkt ins .xopp)");
                self.enter_editor(ctx, ed);
                return;
            }
            let name = format!("{}.owndraw", safe_file_name(&doc.meta.name));
            match &f {
                FileRef::Path(p) => {
                    let beside = p.parent().filter(|d| !d.as_os_str().is_empty()).map(|d| FileRef::Path(d.to_path_buf()));
                    let target = beside
                        .and_then(|d| self.sh.platform.create_in(&d, &name, FileKind::Owndraw).ok().filter(writable_dir))
                        .or_else(|| self.sh.platform.create_in(&self.default_dir(), &name, FileKind::Owndraw).ok());
                    match target {
                        Some(t) => self.finish_import(ctx, doc, t),
                        None => self.sh.toast_for("Kein Speicherort für den Import verfügbar", 4.0),
                    }
                }
                // Browser: das Board landet neben den anderen im Browser-Speicher.
                FileRef::Uri(u) if crate::platform::WEB && u.starts_with("mem:") => match self.sh.platform.create_in(&self.default_dir(), &name, FileKind::Owndraw) {
                    Ok(t) => self.finish_import(ctx, doc, t),
                    Err(e) => self.sh.toast_for(&format!("Kein Speicherort für den Import verfügbar: {e}"), 4.0),
                },
                FileRef::Uri(_) => {
                    // Ohne Ordnerzugriff: Speicherort erfragen.
                    let tag = self.tag(PickPurpose::NewDoc, None);
                    self.pending_docs.insert(tag, doc);
                    self.sh.platform.pick_save(&name, FileKind::Owndraw, tag);
                }
            }
            return;
        }
        let read_only = doc.meta.read_only;
        let mut ed = Editor::open(&mut self.sh, doc, Some(f));
        if read_only {
            ed.autosave.set_read_only(true);
            self.sh.toast_for(session::READ_ONLY_HINT, 7.0);
        }
        self.enter_editor(ctx, ed);
    }

    /// Importiertes Dokument unter `target` (.owndraw) weiterführen und sofort einmal speichern.
    fn finish_import(&mut self, ctx: &egui::Context, doc: Document, target: FileRef) {
        let mut ed = Editor::open(&mut self.sh, doc, Some(target));
        ed.autosave.mark_changed(self.sh.now);
        ed.flush(&mut self.sh);
        self.sh.toast("Xournal++-Datei importiert");
        self.enter_editor(ctx, ed);
    }

    fn create(&mut self, ctx: &egui::Context, params: NewParams, custom: bool) {
        // Browser mit gültiger Lizenz: neue Boards am Server anlegen (öffnet sich, sobald der Server antwortet)
        #[cfg(target_arch = "wasm32")]
        if crate::platform::web::api::licensed() {
            let doc = session::new_document(&params);
            match session::serialize(&doc) {
                Ok(b) => crate::platform::web::api::create_board(&params.name, b),
                Err(e) => self.sh.toast_for(&e, 4.0),
            }
            let _ = (ctx, custom);
            return;
        }
        let fname = format!("{}.owndraw", safe_file_name(&params.name));
        // Im Browser gibt es keinen Speicherort zum Wählen (Speichern unter = Download).
        if custom && !crate::platform::WEB {
            let tag = self.tag(PickPurpose::NewDoc, Some(params));
            self.sh.platform.pick_save(&fname, FileKind::Owndraw, tag);
            return;
        }
        match self.sh.platform.create_in(&self.default_dir(), &fname, FileKind::Owndraw) {
            Ok(f) => self.finish_create(ctx, params, f),
            Err(e) => self.sh.toast_for(&format!("Speicherort nicht verfügbar: {e}"), 4.0),
        }
    }

    /// Startbildschirm: Bild abgelegt → neues unendliches Board mit dem Bild.
    fn new_with_image(&mut self, ctx: &egui::Context, bytes: &[u8]) {
        let params = NewParams { name: "Bild".into(), pages: false, ..NewParams::default() };
        let before = self.editor.as_ref().map(|e| e.doc.meta.id.clone());
        self.create(ctx, params, false);
        if let Some(e) = self.editor.as_mut().filter(|e| Some(&e.doc.meta.id) != before.as_ref()) {
            if let Err(m) = e.import_image_bytes(bytes) {
                self.sh.toast(&m);
            }
        }
    }

    fn finish_create(&mut self, ctx: &egui::Context, params: NewParams, f: FileRef) {
        let doc = session::new_document(&params);
        if let Err(e) = session::write_document(&*self.sh.platform, &f, &doc) {
            self.sh.toast_for(&e, 4.0);
        }
        // Datei ist angelegt und einmal geschrieben; der Autosave übernimmt ab der ersten Änderung.
        let ed = Editor::open(&mut self.sh, doc, Some(f));
        self.enter_editor(ctx, ed);
    }

    // ---------------------------------------------------------------- Ereignisse

    fn handle_pick(&mut self, ctx: &egui::Context, tag: Tag, file: Option<FileRef>) {
        let Some((purpose, params)) = self.pending.remove(&tag) else { return };
        let Some(f) = file else {
            self.pending_docs.remove(&tag);
            return;
        };
        if let Some(doc) = self.pending_docs.remove(&tag) {
            let f = match f {
                FileRef::Path(p) if p.extension().is_none_or(|e| !e.eq_ignore_ascii_case("owndraw")) => FileRef::Path(PathBuf::from(format!("{}.owndraw", p.display()))),
                other => other,
            };
            self.finish_import(ctx, doc, f);
            return;
        }
        match purpose {
            PickPurpose::OpenDoc => self.open_ref(ctx, f),
            PickPurpose::DefaultDir => self.sh.settings.default_dir = Some(f.to_key()),
            PickPurpose::NewDoc => {
                if let Some(p) = params {
                    self.finish_create(ctx, p, f);
                }
            }
            other => {
                if let Some(e) = self.editor.as_mut() {
                    e.apply_pick(&mut self.sh, other, f);
                }
            }
        }
    }

    fn handle_reqs(&mut self, ctx: &egui::Context) {
        for r in std::mem::take(&mut self.sh.reqs) {
            match r {
                Req::Back => self.close_editor(ctx),
                Req::OpenSettings => {
                    if self.screen != Screen::Settings {
                        self.back_screen = self.screen;
                        self.screen = Screen::Settings;
                    }
                }
                Req::ToolsEdited => {
                    if let Some(e) = self.editor.as_mut() {
                        e.st.edit_tools = None;
                    }
                    if self.edit_from_settings {
                        self.edit_from_settings = false;
                        if self.scratch_editor {
                            self.editor = None;
                        }
                        self.scratch_editor = false;
                        self.screen = Screen::Settings;
                        self.title(ctx);
                    }
                }
                Req::NewDialog => self.sh.dialog = Some(Dialog::new_doc(self.sh.settings.save_choice == SaveChoice::Ask && !crate::platform::WEB)),
                Req::OpenFile(f) => self.open_ref(ctx, f),
                Req::RemoveRecent(f) => {
                    self.recent.remove(&f);
                    self.start.forget(&f);
                }
                Req::PickOpen { purpose, kinds } => {
                    let tag = self.tag(purpose, None);
                    self.sh.platform.pick_open(&kinds, tag);
                }
                Req::PickSave { purpose, name, kind } => {
                    let tag = self.tag(purpose, None);
                    self.sh.platform.pick_save(&name, kind, tag);
                }
            }
        }
    }

    fn handle_dialog(&mut self, ctx: &egui::Context) {
        let Some(out) = dialogs::show(ctx, &mut self.sh) else { return };
        match out {
            Outcome::Create(p, custom) => self.create(ctx, p, custom),
            Outcome::Rename(what, text) => {
                let now = self.sh.now;
                if let Some(e) = self.editor.as_mut() {
                    // Browser: Server-Board auch in der Liste umbenennen (nur Besitzer)
                    #[cfg(target_arch = "wasm32")]
                    if let (crate::ui::dialogs::RenameWhat::Doc, Some(id)) = (&what, e.file().and_then(crate::platform::web::api::srv_id)) {
                        if crate::platform::web::api::board(id).is_some_and(|b| b.role == "owner") {
                            crate::platform::web::api::rename_board(id, &text);
                        }
                    }
                    e.apply_rename(what, text, now);
                    self.title(ctx);
                }
            }
            Outcome::Confirm(a) => {
                if let Some(e) = self.editor.as_mut() {
                    e.apply_confirm(a, &mut self.sh);
                }
            }
            Outcome::Anim(a) => {
                if let Some(e) = self.editor.as_mut() {
                    e.apply_anim(a);
                }
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
        let (anchor, off) = if self.screen == Screen::Editor { (Align2::CENTER_TOP, Vec2::new(0.0, 76.0)) } else { (Align2::CENTER_BOTTOM, Vec2::new(0.0, -40.0)) };
        let maxw = (ctx.content_rect().width() - 32.0).max(120.0);
        egui::Area::new(egui::Id::unique("toast")).order(egui::Order::Tooltip).anchor(anchor, off).interactable(false).show(ctx, |ui| {
            egui::Frame::new()
                .fill(c(0xF2303138))
                .corner_radius(CornerRadius::same(20))
                .inner_margin(egui::Margin::symmetric(20, 10))
                .shadow(egui::Shadow { offset: [0, 2], blur: 8, spread: 0, color: Color32::from_black_alpha(80) })
                .show(ui, |ui| {
                    ui.set_max_width(maxw - 40.0);
                    ui.label(egui::RichText::new(msg).size(14.0).color(c(0xFFE9EAEE)));
                });
        });
    }
}

impl eframe::App for App {
    /// Beim Beenden der App noch einmal synchron speichern.
    fn on_exit(&mut self, _gl: Option<&eframe::glow::Context>) {
        self.flush_editor();
    }

    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        let ctx = ui.ctx().clone();
        self.sh.now = ctx.input(|i| i.time);
        // Einstellungen anwenden (Theme, UI-Größe, Akzent) und bei Änderung speichern
        if self.applied.as_ref() != Some(&self.sh.settings) {
            style::apply(&ctx, &self.sh.settings);
            if self.applied.is_some() {
                let _ = self.sh.settings.save(&*self.sh.platform);
            }
            self.applied = Some(self.sh.settings.clone());
        }
        self.sh.env = Env::new(ctx.content_rect().width(), self.sh.platform.is_touch());
        // Update-Dienst: optionaler Start-Hinweis (nur Hinweis, installiert nie von selbst; im Browser aus)
        #[cfg(not(target_arch = "wasm32"))]
        if let Some(n) = crate::update::frame_hook(&self.sh.settings, &*self.sh.platform, &ctx) {
            self.sh.toast_for(&n, 6.0);
        }

        if let Some(f) = self.open_at_start.take() {
            self.open_ref(&ctx, f);
        }
        if let Some(d) = self.demo.take() {
            crate::demo::setup(self, &ctx, &d);
        }
        #[cfg(target_arch = "wasm32")]
        {
            let (open, notes) = crate::platform::web::api::with(|n| (std::mem::take(&mut n.open), std::mem::take(&mut n.notices)));
            for m in notes {
                self.sh.toast_for(&m, 4.0);
            }
            for f in open {
                self.open_ref(&ctx, f);
            }
        }
        for ev in self.sh.platform.poll() {
            match ev {
                PlatformEvent::Picked { tag, file } => self.handle_pick(&ctx, tag, file),
                PlatformEvent::OpenFile(f) => self.open_ref(&ctx, f),
                PlatformEvent::Pause => self.flush_editor(),
            }
        }
        for o in drops::handle_drops(&ctx, &*self.sh.platform, self.editor.as_mut()) {
            match o {
                drops::DropOutcome::Open(f) => self.open_ref(&ctx, f),
                drops::DropOutcome::NewWithImage(b) => self.new_with_image(&ctx, &b),
                drops::DropOutcome::Toast(m) => self.sh.toast(&m),
            }
        }
        if ctx.input(|i| i.viewport().close_requested()) {
            self.flush_editor();
        }
        self.recent_housekeeping();

        let bg = c(style::pal().bg);
        let mut edit_tools = false;
        egui::CentralPanel::default().frame(egui::Frame::new().fill(bg)).show(ui, |ui| match (self.screen, self.editor.as_mut()) {
            (Screen::Editor, Some(e)) => e.ui(ui, &mut self.sh),
            (Screen::Settings, _) => {
                let sh = &mut self.sh;
                match crate::ui::settings::show(ui, &mut sh.settings, &*sh.platform) {
                    SettingsAction::Back => {
                        self.screen = if self.back_screen == Screen::Editor && self.editor.is_none() { Screen::Start } else { self.back_screen };
                    }
                    SettingsAction::PickFolder => {
                        let tag = self.tag(PickPurpose::DefaultDir, None);
                        self.sh.platform.pick_folder(tag);
                    }
                    SettingsAction::EditTools => edit_tools = true,
                    SettingsAction::Changed | SettingsAction::None => {}
                }
            }
            _ => {
                self.screen = Screen::Start;
                crate::ui::start::show(ui, &mut self.sh, &mut self.start, &self.recent);
            }
        });
        if edit_tools {
            self.start_edit_tools(&ctx);
        }
        self.handle_reqs(&ctx);
        self.handle_dialog(&ctx);
        self.toast_ui(&ctx);
        if let Some(s) = self.shot.as_mut() {
            s.frame(&ctx);
            if s.frames() == 10 && std::env::var("OWNDRAW_DEMO").is_ok_and(|d| d.split(',').any(|t| t.trim() == "docklate")) {
                crate::demo::apply_dock(&mut self.sh.settings);
            }
        }
    }
}

/// Ordner der Datei beschreibbar? (Rechte-Probe für den Xournal++-Import daneben; nur Pfade.)
fn writable_dir(f: &FileRef) -> bool {
    match f {
        FileRef::Path(p) => p.parent().and_then(|d| std::fs::metadata(d).ok()).is_some_and(|m| !m.permissions().readonly()),
        FileRef::Uri(_) => true,
    }
}

impl App {
    /// Zuletzt-Liste pflegen: Eintrag nach jedem erfolgreichen Autosave aktualisieren, fehlende Dateien erkennen.
    fn recent_housekeeping(&mut self) {
        if let Some(e) = &self.editor {
            if let (AutosaveStatus::Saved(t), Some(f)) = (e.autosave.status(), e.file().filter(|f| Self::in_recent(f))) {
                if *t != self.last_saved {
                    self.last_saved = *t;
                    self.recent.add(Self::recent_entry(f.clone(), e));
                }
            }
        }
        if self.screen == Screen::Start && self.sh.now - self.last_missing_check >= 3.0 {
            self.last_missing_check = self.sh.now;
            let pf = &*self.sh.platform;
            self.recent.check_missing(|f| pf.exists(f));
        }
    }

    /// Öffnet den Editor im Bearbeiten-Modus der Werkzeugleiste (ohne offenes Board: leeres Board ohne Datei).
    fn start_edit_tools(&mut self, ctx: &egui::Context) {
        self.edit_from_settings = true;
        if self.editor.is_none() {
            let mut doc = session::new_document(&NewParams::default());
            doc.meta.name = "Werkzeuge anpassen".into();
            let ed = Editor::open(&mut self.sh, doc, None);
            self.editor = Some(ed);
            self.scratch_editor = true;
        }
        if let Some(e) = self.editor.as_mut() {
            e.begin_edit_tools(&self.sh);
        }
        self.screen = Screen::Editor;
        self.title(ctx);
    }

    /// Für Demo/Tests: Zugriff auf Teile des Zustands.
    pub fn parts(&mut self) -> (&mut Shared, &mut StartState, &mut Recent) {
        (&mut self.sh, &mut self.start, &mut self.recent)
    }

    pub fn set_screen(&mut self, s: Screen) {
        self.screen = s;
    }

    pub fn open_editor(&mut self, ctx: &egui::Context, ed: Editor) {
        self.enter_editor(ctx, ed);
    }
}
