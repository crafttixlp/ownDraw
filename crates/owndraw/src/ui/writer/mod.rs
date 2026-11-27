//! Textverarbeitung („Textdokument“): Bearbeiten echter Seiten wie in Word, Bedienung wie in Google Docs.
//! Modell, Seitenumbruch, PDF und Word-Datei liegen im Kern (`owndraw_core::office`); hier sind Zustand,
//! Bearbeiten (Cursor, Auswahl, Rückgängig, Formatieren) und Speichern. Zeichnen/Eingabe: `view.rs`,
//! Leisten, Menüs, Suche und Fenster: `bars.rs`.

mod bars;
mod view;

use crate::app::{PickPurpose, Req, Shared};
use crate::autosave::Autosave;
use crate::platform::{FileKind, FileRef};
use crate::session;
use owndraw_core::doc::Document;
use owndraw_core::export::formats::Fmt;
use owndraw_core::office::fonts::Family;
use owndraw_core::office::{self, layout, Align, CStyle, Kind, Layout, Para, Pos, TextDoc, VAlign};

/// Alles, was man im Textdokument tun kann (Leiste, Menüs, Befehlssuche, Tastenkürzel).
#[derive(Clone, Copy, PartialEq, Debug)]
pub enum Cmd {
    Undo,
    Redo,
    Cut,
    Copy,
    Paste,
    SelectAll,
    Find,
    Replace,
    Bold,
    Italic,
    Underline,
    Strike,
    Sup,
    Sub,
    ClearFmt,
    Kind(Kind),
    Align(Align),
    Bullets,
    Numbers,
    Indent,
    Outdent,
    FontUp,
    FontDown,
    Family(Family),
    Spacing(f32),
    Color(u32),
    Highlight(u32),
    PageBreak,
    Link,
    Date,
    Special,
    HeaderFooter,
    PageNumbers,
    Toc,
    PageSetup,
    WordCount,
    Shortcuts,
    Outline,
    ZoomIn,
    ZoomOut,
    ZoomReset,
    ExportPdf,
    ExportDocx,
    ImportDocx,
    Print,
    SaveCopy,
    Rename,
    Help,
    Back,
}

/// Einträge der Befehlssuche (Strg+K): Befehl, Name, Tastenkürzel, Suchwörter.
pub const CMDS: &[(Cmd, &str, &str, &str)] = &[
    (Cmd::Undo, "Rückgängig", "Strg+Z", "zurück"),
    (Cmd::Redo, "Wiederholen", "Strg+Y", "wiederherstellen"),
    (Cmd::Cut, "Ausschneiden", "Strg+X", ""),
    (Cmd::Copy, "Kopieren", "Strg+C", ""),
    (Cmd::Paste, "Einfügen", "Strg+V", "zwischenablage"),
    (Cmd::SelectAll, "Alles auswählen", "Strg+A", "markieren"),
    (Cmd::Find, "Suchen", "Strg+F", "finden"),
    (Cmd::Replace, "Suchen und ersetzen", "Strg+H", "ersetzen"),
    (Cmd::Bold, "Fett", "Strg+B", "bold"),
    (Cmd::Italic, "Kursiv", "Strg+I", "italic"),
    (Cmd::Underline, "Unterstrichen", "Strg+U", "unterstreichen"),
    (Cmd::Strike, "Durchgestrichen", "Alt+Umschalt+5", "durchstreichen"),
    (Cmd::Sup, "Hochgestellt", "Strg+.", "exponent"),
    (Cmd::Sub, "Tiefgestellt", "Strg+,", "index"),
    (Cmd::ClearFmt, "Formatierung entfernen", "Strg+Leertaste", "löschen zurücksetzen"),
    (Cmd::Kind(Kind::Body), "Normaler Text", "Strg+Alt+0", "standard absatz"),
    (Cmd::Kind(Kind::Title), "Titel", "", "überschrift"),
    (Cmd::Kind(Kind::H1), "Überschrift 1", "Strg+Alt+1", "heading"),
    (Cmd::Kind(Kind::H2), "Überschrift 2", "Strg+Alt+2", "heading"),
    (Cmd::Kind(Kind::H3), "Überschrift 3", "Strg+Alt+3", "heading"),
    (Cmd::Kind(Kind::Quote), "Zitat", "", "einrücken"),
    (Cmd::Bullets, "Aufzählung", "Strg+Umschalt+8", "liste punkte"),
    (Cmd::Numbers, "Nummerierung", "Strg+Umschalt+7", "liste zahlen"),
    (Cmd::Indent, "Einzug vergrößern", "Tab", "einrücken"),
    (Cmd::Outdent, "Einzug verkleinern", "Umschalt+Tab", "ausrücken"),
    (Cmd::Align(Align::Left), "Linksbündig", "Strg+Umschalt+L", "ausrichten"),
    (Cmd::Align(Align::Center), "Zentriert", "Strg+Umschalt+E", "mitte ausrichten"),
    (Cmd::Align(Align::Right), "Rechtsbündig", "Strg+Umschalt+R", "ausrichten"),
    (Cmd::Align(Align::Justify), "Blocksatz", "Strg+Umschalt+J", "ausrichten"),
    (Cmd::FontUp, "Schrift größer", "Strg+Umschalt+.", "größe"),
    (Cmd::FontDown, "Schrift kleiner", "Strg+Umschalt+,", "größe"),
    (Cmd::Family(Family::Serif), "Schrift: Serif", "", "times schriftart"),
    (Cmd::Family(Family::Sans), "Schrift: Sans", "", "arial schriftart"),
    (Cmd::Spacing(1.0), "Zeilenabstand einfach", "", "zeilen"),
    (Cmd::Spacing(1.15), "Zeilenabstand 1,15", "", "zeilen"),
    (Cmd::Spacing(1.5), "Zeilenabstand 1,5", "", "zeilen"),
    (Cmd::Spacing(2.0), "Zeilenabstand doppelt", "", "zeilen"),
    (Cmd::PageBreak, "Seitenumbruch", "Strg+Enter", "neue seite"),
    (Cmd::Link, "Link einfügen", "Strg+K", "url verknüpfung"),
    (Cmd::Date, "Datum einfügen", "", "heute"),
    (Cmd::Special, "Sonderzeichen", "", "symbol zeichen"),
    (Cmd::HeaderFooter, "Kopf- und Fußzeile", "", "header footer"),
    (Cmd::PageNumbers, "Seitenzahlen", "", "nummer fußzeile"),
    (Cmd::Toc, "Inhaltsverzeichnis", "", "gliederung verzeichnis"),
    (Cmd::PageSetup, "Seite einrichten", "", "format ränder quer hoch a4"),
    (Cmd::WordCount, "Wörter zählen", "Strg+Umschalt+C", "statistik zeichen"),
    (Cmd::Outline, "Gliederung ein/aus", "", "übersicht navigation"),
    (Cmd::ZoomIn, "Vergrößern", "Strg++", "zoom"),
    (Cmd::ZoomOut, "Verkleinern", "Strg+-", "zoom"),
    (Cmd::ZoomReset, "Zoom 100 %", "Strg+0", "zoom"),
    (Cmd::ExportPdf, "Als PDF speichern", "", "export herunterladen"),
    (Cmd::ExportDocx, "Als Word-Datei speichern", "", "docx export herunterladen"),
    (Cmd::ImportDocx, "Word-Datei einfügen", "", "docx importieren öffnen"),
    (Cmd::Print, "Drucken", "Strg+P", ""),
    (Cmd::SaveCopy, "Kopie speichern", "", "duplikat"),
    (Cmd::Rename, "Umbenennen", "", "name titel"),
    (Cmd::Shortcuts, "Tastenkürzel", "", "hilfe tasten"),
    (Cmd::Help, "Anleitung & Tipps", "F1", "hilfe tutorial"),
    (Cmd::Back, "Zurück zur Startseite", "", "schließen"),
];

/// Was die letzte Änderung war (Tippen wird zu einem Rückgängig-Schritt zusammengefasst).
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Edit {
    Type,
    Delete,
    Other,
}

#[derive(Clone)]
struct Snap {
    td: TextDoc,
    cur: Pos,
    anchor: Pos,
}

/// Offenes Fenster im Textdokument.
#[derive(Clone, Debug, PartialEq)]
pub enum Dlg {
    Link(String, String),
    Special,
    HeaderFooter(String, String),
    PageSetup,
    WordCount,
    Shortcuts,
}

/// Suchleiste.
#[derive(Default)]
pub struct Find {
    pub q: String,
    pub rep: String,
    pub replace: bool,
    pub hits: Vec<(Pos, Pos)>,
    pub idx: usize,
    pub focus: bool,
    pub last_q: String,
    pub last_rev: u64,
}

pub struct Writer {
    pub doc: Document,
    pub td: TextDoc,
    lay: Layout,
    /// Änderungszähler (Layout und Suche neu berechnen).
    rev: u64,
    lay_rev: u64,
    pub autosave: Autosave,
    cur: Pos,
    anchor: Pos,
    /// Gewünschte x-Position beim Hoch/Runter (pt).
    goal_x: Option<f32>,
    /// Format für das nächste getippte Zeichen (Fett etc. ohne Auswahl).
    pending: Option<CStyle>,
    undo: Vec<Snap>,
    redo: Vec<Snap>,
    last: (Edit, f64),
    pub zoom: f32,
    pub outline: bool,
    pub find: Option<Find>,
    pub palette: Option<(String, usize)>,
    pub dlg: Option<Dlg>,
    /// IME-Vorschau (Bereich im Dokument).
    preedit: Option<(Pos, Pos)>,
    /// Interne Zwischenablage mit Formatierung (gilt, solange der Text gleich ist).
    clip: Option<(String, Vec<Para>)>,
    blink: f64,
    scroll_caret: bool,
    /// Ziehen mit der Maus wählt Wörter (nach Doppelklick).
    drag_words: bool,
    pub rename: Option<String>,
    focus_req: bool,
    /// Maßstab (Pixel je pt) des letzten Bildes.
    k: f32,
}

const MAX_UNDO: usize = 200;

impl Writer {
    /// Öffnet ein Textdokument; `file` = Ziel für Autosave.
    pub fn open(doc: Document, file: Option<FileRef>) -> Writer {
        let mut doc = doc;
        let td = doc.text.take().map(|v| TextDoc::from_json(&v)).unwrap_or_default();
        let lay = layout(&td);
        Writer {
            doc,
            td,
            lay,
            rev: 1,
            lay_rev: 1,
            autosave: Autosave::new(file),
            cur: Pos::default(),
            anchor: Pos::default(),
            goal_x: None,
            pending: None,
            undo: Vec::new(),
            redo: Vec::new(),
            last: (Edit::Other, -10.0),
            zoom: 1.0,
            outline: false,
            find: None,
            palette: None,
            dlg: None,
            preedit: None,
            clip: None,
            blink: 0.0,
            scroll_caret: false,
            drag_words: false,
            rename: None,
            focus_req: true,
            k: 1.333,
        }
    }

    pub fn name(&self) -> &str {
        &self.doc.meta.name
    }

    pub fn file(&self) -> Option<&FileRef> {
        self.autosave.target()
    }

    /// Dokument mit aktuellem Text (Speichern, Kopie).
    pub fn snapshot_doc(&self) -> Document {
        let mut d = self.doc.clone();
        d.text = Some(self.td.to_json());
        d
    }

    fn job(&mut self) -> crate::autosave::Job {
        self.doc.meta.modified = session::rfc3339_now();
        let mut d = self.doc.clone();
        let td = self.td.clone();
        let lay = self.layout().clone();
        crate::autosave::Job(Box::new(move || {
            d.thumb = office::render::thumb(&td, &lay);
            d.text = Some(td.to_json());
            session::serialize(&d).unwrap_or_default()
        }))
    }

    pub fn flush(&mut self, sh: &mut Shared) {
        if self.autosave.is_dirty() {
            let job = self.job();
            self.autosave.flush(sh.now, || job, &*sh.platform);
        }
        if let Some(e) = self.autosave.error() {
            sh.toast_for(e, 4.0);
        }
    }

    fn tick(&mut self, sh: &mut Shared) {
        if self.autosave.next_due(sh.now).is_some_and(|t| t <= sh.now) {
            let job = self.job();
            self.autosave.tick(sh.now, || job, &*sh.platform);
        }
    }

    pub fn rename_to(&mut self, name: String, now: f64) {
        if !name.trim().is_empty() && name != self.doc.meta.name {
            self.doc.meta.name = name.trim().to_string();
            self.changed(now);
        }
    }

    // ------------------------------------------------------------ Grundlagen

    fn layout(&mut self) -> &Layout {
        if self.lay_rev != self.rev {
            self.lay = layout(&self.td);
            self.lay_rev = self.rev;
        }
        &self.lay
    }

    fn changed(&mut self, now: f64) {
        self.rev += 1;
        self.autosave.mark_changed(now);
        if !matches!(self.file(), Some(FileRef::Uri(u)) if u.starts_with("srv:")) {
            crate::platform::note_unsaved(true);
        }
        self.scroll_caret = true;
        self.blink = now;
    }

    pub fn sel(&self) -> (Pos, Pos) {
        office::model::ordered(self.anchor, self.cur)
    }

    pub fn has_sel(&self) -> bool {
        self.anchor != self.cur
    }

    fn set_cur(&mut self, p: Pos, extend: bool, now: f64) {
        self.cur = self.td.clamp(p);
        if !extend {
            self.anchor = self.cur;
        }
        self.pending = None;
        self.scroll_caret = true;
        self.blink = now;
    }

    fn snap(&self) -> Snap {
        Snap { td: self.td.clone(), cur: self.cur, anchor: self.anchor }
    }

    /// Rückgängig-Punkt vor einer Änderung; Tippen/Löschen in Folge wird zusammengefasst.
    fn begin(&mut self, kind: Edit, now: f64) {
        let merge = kind != Edit::Other && self.last.0 == kind && now - self.last.1 < 1.5 && !self.has_sel();
        if !merge {
            self.undo.push(self.snap());
            if self.undo.len() > MAX_UNDO {
                self.undo.remove(0);
            }
        }
        self.redo.clear();
        self.last = (kind, now);
    }

    pub fn can_undo(&self) -> bool {
        !self.undo.is_empty()
    }
    pub fn can_redo(&self) -> bool {
        !self.redo.is_empty()
    }

    fn undo_redo(&mut self, redo: bool, now: f64) {
        let (from, to) = if redo { (&mut self.redo, &mut self.undo) } else { (&mut self.undo, &mut self.redo) };
        let Some(s) = from.pop() else { return };
        to.push(Snap { td: std::mem::replace(&mut self.td, s.td), cur: self.cur, anchor: self.anchor });
        self.cur = self.td.clamp(s.cur);
        self.anchor = self.td.clamp(s.anchor);
        self.last = (Edit::Other, now);
        self.preedit = None;
        self.changed(now);
    }

    /// Format an der Einfügestelle bzw. am Anfang der Auswahl.
    pub fn cur_style(&self) -> CStyle {
        if let Some(p) = &self.pending {
            return p.clone();
        }
        let (a, _) = self.sel();
        let at = if self.has_sel() { self.td.next(a) } else { self.cur };
        let mut st = self.td.style_at(at);
        if self.has_sel() {
            st.link = self.td.paras.get(at.p).and_then(|p| p.style_at(at.o).link);
        }
        st
    }

    pub fn cur_para(&self) -> &Para {
        let p = self.cur.p.min(self.td.paras.len().saturating_sub(1));
        &self.td.paras[p]
    }

    /// Wirksame Schriftgröße an der Einfügestelle (pt).
    pub fn cur_size(&self) -> f32 {
        self.cur_style().size.unwrap_or_else(|| layout::para_style(&self.td, self.cur_para().kind).size)
    }

    pub fn cur_family(&self) -> Family {
        self.cur_style().fam.unwrap_or_else(|| layout::para_style(&self.td, self.cur_para().kind).fam)
    }

    // ------------------------------------------------------------ Bearbeiten

    fn delete_sel(&mut self) -> bool {
        if !self.has_sel() {
            return false;
        }
        let (a, b) = self.sel();
        self.cur = self.td.delete(a, b);
        self.anchor = self.cur;
        true
    }

    pub fn insert(&mut self, s: &str, now: f64) {
        if s.is_empty() {
            return;
        }
        self.begin(Edit::Type, now);
        let st = self.pending.clone().unwrap_or_else(|| {
            let (a, _) = self.sel();
            self.td.style_at(if self.has_sel() { self.td.next(a) } else { self.cur })
        });
        self.delete_sel();
        self.cur = self.td.insert_text(self.cur, s, &st);
        self.anchor = self.cur;
        if !s.contains('\n') {
            self.pending = self.pending.is_some().then_some(st);
        } else {
            self.pending = None;
        }
        self.goal_x = None;
        self.changed(now);
    }

    fn enter(&mut self, now: f64) {
        self.begin(Edit::Other, now);
        self.delete_sel();
        self.cur = self.td.split(self.cur);
        self.anchor = self.cur;
        self.pending = None;
        self.changed(now);
    }

    fn backspace(&mut self, word: bool, now: f64) {
        self.begin(Edit::Delete, now);
        if !self.delete_sel() {
            let c = self.cur;
            let para = &self.td.paras[c.p.min(self.td.paras.len() - 1)];
            if c.o == 0 && (para.kind.is_list() || para.indent > 0 || para.kind == Kind::Quote) {
                // Wie in Word: am Anfang erst Einzug/Liste aufheben
                let k = para.kind;
                self.td.repara(c, c, &|p: &mut Para| {
                    if p.indent > 0 && k.is_list() {
                        p.indent -= 1;
                    } else if p.kind.is_list() || p.kind == Kind::Quote {
                        p.kind = Kind::Body;
                    } else {
                        p.indent = p.indent.saturating_sub(1);
                    }
                });
            } else {
                let a = if word { self.td.word_left(c) } else { self.td.prev(c) };
                self.cur = self.td.delete(a, c);
                self.anchor = self.cur;
            }
        }
        self.goal_x = None;
        self.changed(now);
    }

    fn delete_fwd(&mut self, word: bool, now: f64) {
        self.begin(Edit::Delete, now);
        if !self.delete_sel() {
            let c = self.cur;
            let b = if word { self.td.word_right(c) } else { self.td.next(c) };
            self.cur = self.td.delete(c, b);
            self.anchor = self.cur;
        }
        self.changed(now);
    }

    /// Zeichenformat ändern: Auswahl, sonst das Wort am Cursor, sonst das nächste getippte Zeichen.
    fn restyle(&mut self, now: f64, f: &dyn Fn(&mut CStyle)) {
        let (mut a, mut b) = self.sel();
        if a == b {
            let (wa, wb) = self.td.word_at(self.cur);
            if wa < self.cur && self.cur < wb {
                (a, b) = (wa, wb);
            } else {
                let mut st = self.cur_style();
                f(&mut st);
                self.pending = Some(st);
                return;
            }
        }
        self.begin(Edit::Other, now);
        self.td.restyle(a, b, f);
        self.changed(now);
    }

    fn toggle(&mut self, now: f64, get: fn(&CStyle) -> bool, set: fn(&mut CStyle, bool)) {
        let on = !get(&self.cur_style());
        self.restyle(now, &move |s: &mut CStyle| set(s, on));
    }

    fn repara(&mut self, now: f64, f: &dyn Fn(&mut Para)) {
        self.begin(Edit::Other, now);
        let (a, b) = self.sel();
        self.td.repara(a, b, f);
        self.changed(now);
    }

    fn all_kind(&self, k: Kind) -> bool {
        let (a, b) = self.sel();
        (a.p..=b.p.min(self.td.paras.len() - 1)).all(|i| self.td.paras[i].kind == k)
    }

    fn set_kind(&mut self, k: Kind, now: f64) {
        let to = if (k.is_list() || k == Kind::Quote) && self.all_kind(k) { Kind::Body } else { k };
        self.repara(now, &move |p: &mut Para| {
            if p.kind != Kind::PageBreak {
                p.kind = to;
            }
        });
    }

    fn indent(&mut self, d: i32, now: f64) {
        self.repara(now, &move |p: &mut Para| p.indent = (p.indent as i32 + d).clamp(0, 8) as u8);
    }

    fn font_step(&mut self, up: bool, now: f64) {
        const SIZES: [f32; 16] = [8.0, 9.0, 10.0, 11.0, 12.0, 14.0, 16.0, 18.0, 20.0, 24.0, 28.0, 32.0, 36.0, 48.0, 60.0, 72.0];
        let s = self.cur_size();
        let n = if up { SIZES.iter().copied().find(|v| *v > s + 0.01).unwrap_or(s + 12.0) } else { SIZES.iter().rev().copied().find(|v| *v < s - 0.01).unwrap_or((s - 1.0).max(4.0)) };
        self.set_size(n, now);
    }

    pub fn set_size(&mut self, n: f32, now: f64) {
        let n = n.clamp(4.0, 400.0);
        self.restyle(now, &move |s: &mut CStyle| s.size = Some(n));
    }

    pub fn select_all(&mut self) {
        self.anchor = Pos::default();
        self.cur = self.td.end();
    }

    fn sel_text(&self) -> String {
        let (a, b) = self.sel();
        self.td.text_range(a, b)
    }

    fn copy(&mut self, ctx: &egui::Context) {
        if !self.has_sel() {
            return;
        }
        let t = self.sel_text();
        let (a, b) = self.sel();
        self.clip = Some((t.clone(), self.td.slice(a, b)));
        ctx.copy_text(t);
    }

    fn cut(&mut self, ctx: &egui::Context, now: f64) {
        if self.has_sel() {
            self.copy(ctx);
            self.begin(Edit::Other, now);
            self.delete_sel();
            self.changed(now);
        }
    }

    /// Einfügen: eigene Kopie mit Formatierung, sonst reiner Text.
    pub fn paste(&mut self, s: &str, now: f64) {
        let norm = |t: &str| t.replace("\r\n", "\n");
        if let Some((t, paras)) = self.clip.clone().filter(|(t, _)| norm(t) == norm(s)) {
            let _ = t;
            self.begin(Edit::Other, now);
            self.delete_sel();
            self.cur = self.td.insert_paras(self.cur, &paras);
            self.anchor = self.cur;
            self.changed(now);
        } else {
            self.last = (Edit::Other, now);
            self.insert(s, now);
            self.last = (Edit::Other, now);
        }
    }

    fn insert_paras(&mut self, paras: &[Para], now: f64) {
        self.begin(Edit::Other, now);
        self.delete_sel();
        self.cur = self.td.insert_paras(self.cur, paras);
        self.anchor = self.cur;
        self.changed(now);
    }

    fn page_break(&mut self, now: f64) {
        self.begin(Edit::Other, now);
        self.delete_sel();
        let c = self.td.split(self.cur);
        // Leerer Absatz vor dem neuen wird zum Umbruch
        self.td.paras.insert(c.p, Para::new(Kind::PageBreak));
        self.cur = Pos::new(c.p + 1, 0);
        self.anchor = self.cur;
        self.changed(now);
    }

    fn set_link(&mut self, url: String, text: String, now: f64) {
        let url = url.trim().to_string();
        let link = (!url.is_empty()).then(|| if url.contains("://") || url.starts_with("mailto:") { url.clone() } else { format!("https://{url}") });
        self.begin(Edit::Other, now);
        if !self.has_sel() {
            let t = if text.trim().is_empty() { url.clone() } else { text };
            if t.is_empty() {
                return;
            }
            let mut st = self.td.style_at(self.cur);
            st.link = link;
            let a = self.cur;
            self.cur = self.td.insert_text(a, &t, &st);
            self.anchor = self.cur;
        } else {
            let (a, b) = self.sel();
            self.td.restyle(a, b, &move |s: &mut CStyle| s.link = link.clone());
        }
        self.changed(now);
    }

    /// Statisches Inhaltsverzeichnis an der Einfügestelle (Überschriften mit Seitenzahl).
    fn toc(&mut self, now: f64) {
        let lay = self.layout().clone();
        let mut paras = vec![Para::with_text(Kind::H2, "Inhalt")];
        for (i, p) in self.td.paras.iter().enumerate() {
            if let Some(l) = p.kind.outline_level().filter(|l| *l >= 1) {
                let mut q = Para::with_text(Kind::Body, &format!("{}\t{}", p.text(), lay.page_of_para(i) + 1));
                q.indent = l - 1;
                paras.push(q);
            }
        }
        if paras.len() == 1 {
            paras.push(Para::with_text(Kind::Body, "Noch keine Überschriften – formatiere Zeilen als „Überschrift 1–3“."));
        }
        paras.push(Para::new(Kind::Body));
        let c = self.cur;
        self.cur = Pos::new(c.p, 0);
        self.anchor = self.cur;
        self.insert_paras(&paras, now);
    }

    /// Gliederung: (Absatz, Ebene, Text).
    pub fn headings(&self) -> Vec<(usize, u8, String)> {
        self.td.paras.iter().enumerate().filter_map(|(i, p)| p.kind.outline_level().map(|l| (i, l, p.text()))).filter(|h| !h.2.trim().is_empty()).collect()
    }

    pub fn jump_to_para(&mut self, p: usize, now: f64) {
        self.set_cur(Pos::new(p, 0), false, now);
    }

    // ------------------------------------------------------------ Suche

    pub fn update_find(&mut self) {
        let rev = self.rev;
        if let Some(f) = self.find.as_mut() {
            if f.last_q != f.q || f.last_rev != rev {
                f.hits = if f.q.is_empty() { Vec::new() } else { self.td.find(&f.q) };
                if f.last_q != f.q {
                    f.idx = f.hits.iter().position(|h| h.0 >= self.cur).unwrap_or(0);
                }
                f.idx = f.idx.min(f.hits.len().saturating_sub(1));
                f.last_q = f.q.clone();
                f.last_rev = rev;
            }
        }
    }

    pub fn find_step(&mut self, d: i32, now: f64) {
        self.update_find();
        let Some(f) = self.find.as_mut() else { return };
        if f.hits.is_empty() {
            return;
        }
        let n = f.hits.len() as i32;
        f.idx = ((f.idx as i32 + d).rem_euclid(n)) as usize;
        let (a, b) = f.hits[f.idx];
        self.anchor = a;
        self.set_cur(b, true, now);
    }

    pub fn replace_one(&mut self, now: f64) {
        self.update_find();
        let Some(f) = self.find.as_ref() else { return };
        let Some(&(a, b)) = f.hits.get(f.idx) else { return };
        let rep = f.rep.clone();
        self.anchor = a;
        self.cur = b;
        self.last = (Edit::Other, now);
        self.insert(&rep, now);
        self.last = (Edit::Other, now);
        self.update_find();
        self.find_step(0, now);
    }

    pub fn replace_all(&mut self, now: f64) -> usize {
        self.update_find();
        let Some(f) = self.find.as_ref() else { return 0 };
        let (hits, rep) = (f.hits.clone(), f.rep.clone());
        if hits.is_empty() {
            return 0;
        }
        self.begin(Edit::Other, now);
        for &(a, b) in hits.iter().rev() {
            let st = self.td.style_at(self.td.next(a));
            let p = self.td.delete(a, b);
            self.td.insert_text(p, &rep, &st);
        }
        self.cur = self.td.clamp(self.cur);
        self.anchor = self.cur;
        self.changed(now);
        hits.len()
    }

    // ------------------------------------------------------------ Befehle

    pub fn run(&mut self, c: Cmd, sh: &mut Shared, ctx: &egui::Context) {
        let now = sh.now;
        self.focus_req = true;
        match c {
            Cmd::Undo => self.undo_redo(false, now),
            Cmd::Redo => self.undo_redo(true, now),
            Cmd::Cut => self.cut(ctx, now),
            Cmd::Copy => self.copy(ctx),
            Cmd::Paste => match sh.platform.clipboard_get() {
                Some(crate::platform::ClipData::Text(t)) => self.paste(&t, now),
                _ => match self.clip.clone() {
                    Some((t, _)) => self.paste(&t, now),
                    None => sh.toast("Zum Einfügen Strg+V drücken"),
                },
            },
            Cmd::SelectAll => self.select_all(),
            Cmd::Find | Cmd::Replace => {
                let q = if self.has_sel() && self.sel().0.p == self.sel().1.p { self.sel_text() } else { self.find.as_ref().map(|f| f.q.clone()).unwrap_or_default() };
                let f = self.find.get_or_insert_with(Find::default);
                f.q = q;
                f.replace = c == Cmd::Replace || f.replace;
                f.focus = true;
                self.focus_req = false;
            }
            Cmd::Bold => self.toggle(now, |s| s.b, |s, v| s.b = v),
            Cmd::Italic => self.toggle(now, |s| s.i, |s, v| s.i = v),
            Cmd::Underline => self.toggle(now, |s| s.u, |s, v| s.u = v),
            Cmd::Strike => self.toggle(now, |s| s.s, |s, v| s.s = v),
            Cmd::Sup => self.toggle(now, |s| s.va == VAlign::Sup, |s, v| s.va = if v { VAlign::Sup } else { VAlign::Base }),
            Cmd::Sub => self.toggle(now, |s| s.va == VAlign::Sub, |s, v| s.va = if v { VAlign::Sub } else { VAlign::Base }),
            Cmd::ClearFmt => self.restyle(now, &|s: &mut CStyle| {
                let link = s.link.take();
                *s = CStyle { link, ..CStyle::default() };
            }),
            Cmd::Kind(k) => self.set_kind(k, now),
            Cmd::Bullets => self.set_kind(Kind::Bullet, now),
            Cmd::Numbers => self.set_kind(Kind::Number, now),
            Cmd::Align(a) => self.repara(now, &move |p: &mut Para| p.align = a),
            Cmd::Indent => self.indent(1, now),
            Cmd::Outdent => self.indent(-1, now),
            Cmd::FontUp => self.font_step(true, now),
            Cmd::FontDown => self.font_step(false, now),
            Cmd::Family(f) => self.restyle(now, &move |s: &mut CStyle| s.fam = Some(f)),
            Cmd::Spacing(v) => self.repara(now, &move |p: &mut Para| p.line = v),
            Cmd::Color(col) => self.restyle(now, &move |s: &mut CStyle| s.color = col),
            Cmd::Highlight(col) => self.restyle(now, &move |s: &mut CStyle| s.hl = col),
            Cmd::PageBreak => self.page_break(now),
            Cmd::Link => {
                let st = self.cur_style();
                let text = if self.has_sel() { self.sel_text() } else { String::new() };
                self.dlg = Some(Dlg::Link(st.link.unwrap_or_default(), text));
            }
            Cmd::Date => {
                let d = session::rfc3339_now();
                let (y, m, dd) = (&d[0..4], &d[5..7], &d[8..10]);
                self.last = (Edit::Other, now);
                self.insert(&format!("{dd}.{m}.{y}"), now);
                self.last = (Edit::Other, now);
            }
            Cmd::Special => self.dlg = Some(Dlg::Special),
            Cmd::HeaderFooter => self.dlg = Some(Dlg::HeaderFooter(self.td.header.clone(), self.td.footer.clone())),
            Cmd::PageNumbers => {
                self.begin(Edit::Other, now);
                if self.td.footer.contains("{seite}") {
                    self.td.footer.clear();
                    sh.toast("Seitenzahlen entfernt");
                } else {
                    self.td.footer = "Seite {seite} von {seiten}".into();
                    sh.toast("Seitenzahlen in der Fußzeile");
                }
                self.changed(now);
            }
            Cmd::Toc => self.toc(now),
            Cmd::PageSetup => self.dlg = Some(Dlg::PageSetup),
            Cmd::WordCount => self.dlg = Some(Dlg::WordCount),
            Cmd::Shortcuts => self.dlg = Some(Dlg::Shortcuts),
            Cmd::Outline => self.outline = !self.outline,
            Cmd::ZoomIn => self.zoom = (self.zoom * 1.1).min(3.0),
            Cmd::ZoomOut => self.zoom = (self.zoom / 1.1).max(0.3),
            Cmd::ZoomReset => self.zoom = 1.0,
            Cmd::ExportPdf => self.pick_save(sh, Fmt::Pdf),
            Cmd::ExportDocx => {
                if crate::locks::locked(owndraw_core::features::OFFICE_WORD) {
                    sh.toast(crate::locks::HINT);
                } else {
                    self.pick_save(sh, Fmt::Docx)
                }
            }
            Cmd::ImportDocx => {
                if crate::locks::locked(owndraw_core::features::OFFICE_WORD) {
                    sh.toast(crate::locks::HINT);
                } else {
                    sh.reqs.push(Req::PickOpen { purpose: PickPurpose::DocxInsert, kinds: vec![FileKind::Export(Fmt::Docx)] })
                }
            }
            Cmd::Print => {
                let pdf = self.pdf_bytes();
                match sh.platform.print_pdf(&format!("{}.pdf", crate::app::safe_file_name(self.name())), &pdf) {
                    Ok(m) => sh.toast(&m),
                    Err(e) => sh.toast_for(&e, 4.0),
                }
            }
            Cmd::SaveCopy => sh.reqs.push(Req::PickSave { purpose: PickPurpose::SaveCopy, name: format!("{} (Kopie).owndraw", crate::app::safe_file_name(self.name())), kind: FileKind::Owndraw }),
            Cmd::Rename => self.rename = Some(self.name().to_string()),
            Cmd::Help => sh.reqs.push(Req::OpenTutorial),
            Cmd::Back => sh.reqs.push(Req::Back),
        }
    }

    fn pdf_bytes(&mut self) -> Vec<u8> {
        let lay = self.layout().clone();
        office::render::pdf(&self.td, &lay, &self.doc.meta.name)
    }

    fn pick_save(&self, sh: &mut Shared, f: Fmt) {
        sh.reqs.push(Req::PickSave { purpose: PickPurpose::Export(f), name: format!("{}.{}", crate::app::safe_file_name(self.name()), f.ext()), kind: FileKind::Export(f) });
    }

    /// Ergebnis eines System-Dialogs.
    pub fn apply_pick(&mut self, sh: &mut Shared, purpose: PickPurpose, f: FileRef) {
        let now = sh.now;
        let res: Result<&str, String> = match purpose {
            PickPurpose::Export(Fmt::Docx) => office::docx::export(&self.td, self.name()).and_then(|b| sh.platform.write_atomic(&f, &b).map_err(|e| e.to_string())).map(|_| "Word-Datei gespeichert"),
            PickPurpose::Export(_) => {
                let pdf = self.pdf_bytes();
                sh.platform.write_atomic(&f, &pdf).map_err(|e| e.to_string()).map(|_| "PDF gespeichert")
            }
            PickPurpose::SaveCopy => session::save_copy_as(&*sh.platform, &self.snapshot_doc(), &f).map(|_| "Kopie gespeichert"),
            PickPurpose::DocxInsert => sh.platform.read(&f).map_err(|e| e.to_string()).and_then(|b| office::docx::import(&b)).map(|d| {
                self.insert_paras(&d.paras, now);
                "Word-Datei eingefügt"
            }),
            _ => return,
        };
        match res {
            Ok(m) => sh.toast(m),
            Err(e) => sh.toast_for(&format!("Fehlgeschlagen: {e}"), 4.0),
        }
    }

    /// Ein Bild des Textdokuments.
    pub fn ui(&mut self, ui: &mut egui::Ui, sh: &mut Shared) {
        view::ensure_fonts(ui.ctx());
        self.tick(sh);
        if let Some(e) = self.autosave.error().map(String::from) {
            let _ = e;
        }
        bars::frame(self, ui, sh);
        if let Some(t) = self.autosave.next_due(sh.now) {
            ui.ctx().request_repaint_after(std::time::Duration::from_secs_f64((t - sh.now).clamp(0.05, 5.0)));
        }
    }
}

/// Dateiarten für „Öffnen“ auf der Startseite (Word-Dateien nur, wenn nicht gesperrt).
pub fn open_kinds() -> Vec<FileKind> {
    let mut v = vec![FileKind::Owndraw, FileKind::Xopp];
    if !crate::locks::locked(owndraw_core::features::OFFICE) && !crate::locks::locked(owndraw_core::features::OFFICE_WORD) {
        v.push(FileKind::Export(Fmt::Docx));
    }
    v
}

/// Textdokument aus einer Word-Datei (Öffnen von .docx).
pub fn doc_from_docx(bytes: &[u8], name: &str) -> Result<Document, String> {
    let td = office::docx::import(bytes)?;
    let mut d = Document::new_text(name);
    d.text = Some(td.to_json());
    Ok(d)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn w() -> Writer {
        Writer::open(Document::new_text("T"), None)
    }

    #[test]
    fn tippen_rueckgaengig_formatieren() {
        let mut w = w();
        w.insert("Hallo", 0.0);
        w.insert(" Welt", 0.5);
        assert_eq!(w.td.plain_text().trim(), "Hallo Welt");
        // zusammengefasst: ein Schritt
        w.undo_redo(false, 1.0);
        assert_eq!(w.td.plain_text().trim(), "");
        w.undo_redo(true, 1.1);
        assert_eq!(w.td.plain_text().trim(), "Hallo Welt");
        // Fett auf dem Wort am Cursor
        w.set_cur(Pos::new(0, 2), false, 2.0);
        w.toggle(2.0, |s| s.b, |s, v| s.b = v);
        assert!(w.td.paras[0].style_at(3).b);
        assert!(!w.td.paras[0].style_at(8).b);
        // Liste umschalten
        w.set_kind(Kind::Bullet, 3.0);
        assert_eq!(w.td.paras[0].kind, Kind::Bullet);
        w.set_kind(Kind::Bullet, 3.1);
        assert_eq!(w.td.paras[0].kind, Kind::Body);
        // Backspace am Absatzanfang hebt Liste auf
        w.set_kind(Kind::Number, 4.0);
        w.set_cur(Pos::new(0, 0), false, 4.0);
        w.backspace(false, 4.1);
        assert_eq!(w.td.paras[0].kind, Kind::Body);
    }

    #[test]
    fn suchen_ersetzen_und_einfuegen() {
        let mut w = w();
        w.insert("eins zwei eins\ndrei eins", 0.0);
        w.find = Some(Find { q: "eins".into(), rep: "1".into(), ..Default::default() });
        w.update_find();
        assert_eq!(w.find.as_ref().unwrap().hits.len(), 3);
        assert_eq!(w.replace_all(1.0), 3);
        assert_eq!(w.td.plain_text().trim(), "1 zwei 1\ndrei 1");
        // eigene Kopie behält die Formatierung
        w.select_all();
        w.run_fmt_bold();
        w.select_all();
        let ctx = egui::Context::default();
        w.copy(&ctx);
        w.set_cur(w.td.end(), false, 2.0);
        let t = w.clip.clone().unwrap().0;
        w.paste(&t, 2.0);
        assert!(w.td.paras.last().unwrap().style_at(1).b);
        // Seitenumbruch
        w.page_break(3.0);
        assert!(w.td.paras.iter().any(|p| p.kind == Kind::PageBreak));
        assert_eq!(w.layout().pages, 2);
    }

    impl Writer {
        fn run_fmt_bold(&mut self) {
            self.toggle(1.5, |s| s.b, |s, v| s.b = v);
        }
    }

    #[test]
    fn speichern_als_textdokument() {
        let mut w = w();
        w.insert("Inhalt", 0.0);
        let job = w.job();
        let bytes = (job.0)();
        let d = owndraw_core::doc::read_owndraw(&bytes).unwrap();
        assert!(d.is_text() && d.thumb.is_some());
        assert_eq!(TextDoc::from_json(d.text.as_ref().unwrap()).plain_text().trim(), "Inhalt");
    }
}
