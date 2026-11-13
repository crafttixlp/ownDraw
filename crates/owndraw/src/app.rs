//! Hauptfenster: Bildschirmwechsel (Start, Editor, Einstellungen), Dokumente öffnen/anlegen, Dialog-Ergebnisse,
//! Hinweise (Toast) und das Anwenden der Einstellungen. Alle Plattform-Zugriffe laufen über `Platform`.

use crate::layout::Env;
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
use std::collections::HashMap;

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
}

/// Aufträge der Oberfläche an die Shell (werden am Frameende abgearbeitet).
pub enum Req {
    /// Zurück zum Start (Dokument wird gespeichert).
    Back,
    OpenSettings,
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
/// TODO(WP9): durch `Settings::load` ersetzen.
pub fn load_settings(platform: &dyn Platform) -> Settings {
    let p = FileRef::Path(platform.config_dir().join("settings.json"));
    platform.read(&p).ok().and_then(|b| serde_json::from_slice(&b).ok()).unwrap_or_default()
}

fn save_settings(platform: &dyn Platform, s: &Settings) {
    let p = FileRef::Path(platform.config_dir().join("settings.json"));
    if let Ok(b) = serde_json::to_vec_pretty(s) {
        let _ = platform.write_atomic(&p, &b);
    }
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
    next_tag: Tag,
    applied: Option<Settings>,
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
            next_tag: 1,
            applied: None,
            open_at_start: open_file,
            shot: crate::demo::Shot::from_env(),
            demo: std::env::var("OWNDRAW_DEMO").ok().filter(|s| !s.is_empty()),
        }
    }

    fn title(&self, ctx: &egui::Context) {
        let t = match (&self.editor, self.screen) {
            (Some(e), Screen::Editor) => format!("OwnDraw – {}", e.name()),
            _ => "OwnDraw".to_string(),
        };
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
            if let Some(f) = e.file().cloned() {
                self.recent.add(RecentEntry { file: f.clone(), name: e.name().to_string(), modified_ms: owndraw_core::store::now_ms(), pages: crate::ui::editor::pages::is_pages(&e.board) });
                self.start.forget(&f);
            }
        }
        self.screen = Screen::Start;
        self.title(ctx);
    }

    fn enter_editor(&mut self, ctx: &egui::Context, ed: Editor) {
        if self.editor.is_some() {
            self.close_editor(ctx);
        }
        if let Some(f) = ed.file().cloned() {
            self.recent.add(RecentEntry { file: f, name: ed.name().to_string(), modified_ms: owndraw_core::store::now_ms(), pages: crate::ui::editor::pages::is_pages(&ed.board) });
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
        let imported = FileKind::of_name(&f.display_name()) == FileKind::Xopp;
        let target = if imported {
            // Xournal++-Import: als .owndraw im Standard-Ort weiterarbeiten (Original bleibt unverändert).
            if doc.meta.name.is_empty() {
                doc.meta.name = f.display_name().trim_end_matches(".xopp").to_string();
            }
            let name = format!("{}.owndraw", safe_file_name(&doc.meta.name));
            match self.sh.platform.create_in(&self.default_dir(), &name, FileKind::Owndraw) {
                Ok(t) => Some(t),
                Err(e) => {
                    self.sh.toast_for(&format!("Kein Speicherort für den Import: {e}"), 4.0);
                    None
                }
            }
        } else {
            Some(f)
        };
        let mut ed = Editor::open(&mut self.sh, doc, target);
        if imported {
            ed.autosave.mark_changed(self.sh.now);
            self.sh.toast("Xournal++-Datei importiert");
        }
        self.enter_editor(ctx, ed);
    }

    fn create(&mut self, ctx: &egui::Context, params: NewParams, custom: bool) {
        let fname = format!("{}.owndraw", safe_file_name(&params.name));
        if custom {
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
        let mut ed = Editor::open(&mut self.sh, doc, Some(f));
        ed.autosave.mark_changed(self.sh.now);
        self.enter_editor(ctx, ed);
    }

    // ---------------------------------------------------------------- Ereignisse

    fn handle_pick(&mut self, ctx: &egui::Context, tag: Tag, file: Option<FileRef>) {
        let Some((purpose, params)) = self.pending.remove(&tag) else { return };
        let Some(f) = file else { return };
        match purpose {
            PickPurpose::OpenDoc => self.open_ref(ctx, f),
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
                Req::NewDialog => self.sh.dialog = Some(Dialog::new_doc(self.sh.settings.save_choice == SaveChoice::Ask)),
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
    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        let ctx = ui.ctx().clone();
        self.sh.now = ctx.input(|i| i.time);
        // Einstellungen anwenden (Theme, UI-Größe, Akzent) und bei Änderung speichern
        if self.applied.as_ref() != Some(&self.sh.settings) {
            style::apply(&ctx, &self.sh.settings);
            if self.applied.is_some() {
                save_settings(&*self.sh.platform, &self.sh.settings);
            }
            self.applied = Some(self.sh.settings.clone());
        }
        self.sh.env = Env::new(ctx.content_rect().width(), self.sh.platform.is_touch());

        if let Some(f) = self.open_at_start.take() {
            self.open_ref(&ctx, f);
        }
        if let Some(d) = self.demo.take() {
            crate::demo::setup(self, &ctx, &d);
        }
        for ev in self.sh.platform.poll() {
            match ev {
                PlatformEvent::Picked { tag, file } => self.handle_pick(&ctx, tag, file),
                PlatformEvent::OpenFile(f) => self.open_ref(&ctx, f),
                PlatformEvent::Pause => {
                    if let Some(e) = self.editor.as_mut() {
                        e.flush(&mut self.sh);
                    }
                }
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
            if let Some(e) = self.editor.as_mut() {
                e.flush(&mut self.sh);
            }
        }

        let bg = c(style::pal().bg);
        egui::CentralPanel::default().frame(egui::Frame::new().fill(bg)).show(ui, |ui| match (self.screen, self.editor.as_mut()) {
            (Screen::Editor, Some(e)) => e.ui(ui, &mut self.sh),
            (Screen::Settings, _) => {
                let sh = &mut self.sh;
                match crate::ui::settings::show(ui, &mut sh.settings, &*sh.platform) {
                    SettingsAction::Back => {
                        self.screen = if self.back_screen == Screen::Editor && self.editor.is_none() { Screen::Start } else { self.back_screen };
                    }
                    SettingsAction::Changed | SettingsAction::None => {}
                }
            }
            _ => {
                self.screen = Screen::Start;
                crate::ui::start::show(ui, &mut self.sh, &mut self.start, &self.recent);
            }
        });
        self.handle_reqs(&ctx);
        self.handle_dialog(&ctx);
        self.toast_ui(&ctx);
        if let Some(s) = self.shot.as_mut() {
            s.frame(&ctx);
        }
    }
}

impl App {
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
