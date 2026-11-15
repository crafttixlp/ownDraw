//! Abnahme-Hilfen für Screenshots (nur aktiv mit Umgebungsvariablen, sonst ohne Wirkung):
//!
//! - `OWNDRAW_SCREENSHOT=pfad.png`: nach ein paar Frames Bild des Fensters speichern und beenden.
//! - `OWNDRAW_DEMO=a,b,…`: Ausgangszustand herstellen. Bausteine: `start` (Beispiel-Zuletzt-Liste), `editor`
//!   (unendliches Demo-Board), `pages` (Seiten-Demo), `tool=pen|marker|eraser|select|shape|text`, `options`, `menu`,
//!   `layers`, `pagelist`, `edittools` (Werkzeugleiste im Bearbeiten-Modus), `new`, `settings`, `light` (helles Theme),
//!   `autosave` (Board im Standard-Ort mit Strich), `textnew`, `textedit`, `textsel` (Texteingabe auf dem Blatt), `dock` (Leisten seitlich angedockt).

use crate::app::{App, Screen};
use crate::platform::FileRef;
use crate::recent::RecentEntry;
use crate::session::{self, NewParams};
use crate::settings::Theme;
use crate::ui::dialogs::Dialog;
use crate::ui::editor::Editor;
use owndraw_core::board::{self, Board};
use owndraw_core::item::{self, Item};
use std::path::PathBuf;
use std::rc::Rc;

pub struct Shot {
    path: PathBuf,
    frames: u32,
    done: bool,
}

impl Shot {
    pub fn from_env() -> Option<Shot> {
        std::env::var("OWNDRAW_SCREENSHOT").ok().filter(|s| !s.is_empty()).map(|p| Shot { path: PathBuf::from(p), frames: 0, done: false })
    }

    /// Pro Frame aufrufen.
    pub fn frame(&mut self, ctx: &egui::Context) {
        self.frames += 1;
        ctx.request_repaint();
        if self.frames == 30 && !self.done {
            self.done = true;
            let path = self.path.clone();
            ctx.request_screenshot(move |img| {
                let (w, h) = (img.size[0] as u32, img.size[1] as u32);
                if let Some(mut pm) = owndraw_core::tiny_skia::Pixmap::new(w, h) {
                    for (dst, px) in pm.data_mut().chunks_exact_mut(4).zip(img.pixels.iter()) {
                        dst.copy_from_slice(&px.to_array());
                    }
                    let _ = pm.save_png(&path);
                }
                std::process::exit(0);
            });
        }
    }
}

fn stroke(b: &mut Board, col: u32, w: f32, hl: bool, pts: &[(f32, f32)]) {
    let mut it = Item::stroke(col, w, hl, false);
    // Linienzug dicht abtasten
    for win in pts.windows(2) {
        for k in 0..8 {
            let t = k as f32 / 8.0;
            it.add_point(win[0].0 + (win[1].0 - win[0].0) * t, win[0].1 + (win[1].1 - win[0].1) * t, 0.5);
        }
    }
    if let Some(l) = pts.last() {
        it.add_point(l.0, l.1, 0.5);
    }
    b.items.push(Rc::new(it));
}

/// Beispiel-Inhalt; `ox, oy` verschiebt (Seitenmodus: innerhalb der ersten Seite).
fn demo_board(variant: u32, ox: f32, oy: f32) -> Board {
    let mut b = Board::new(1.0);
    let p = |x: f32, y: f32| (x + ox, y + oy);
    match variant % 3 {
        0 => {
            let (a, c1) = (p(60.0, 80.0), p(260.0, 200.0));
            b.items.push(Rc::new(Item::shape(item::POLY, vec![a.0, a.1, c1.0, a.1, c1.0, c1.1, a.0, c1.1], 0xFF3E63DD, 3.0)));
            b.items.push(Rc::new(Item::shape(item::ELLIPSE, vec![p(300.0, 90.0).0, p(300.0, 90.0).1, p(440.0, 90.0).0, p(300.0, 190.0).1], 0xFFE5484D, 3.0)));
            b.items.push(Rc::new(Item::shape(item::ARROW, vec![p(260.0, 140.0).0, p(260.0, 140.0).1, p(300.0, 140.0).0, p(300.0, 140.0).1], 0xFF111111, 3.0)));
            b.items.push(Rc::new(Item::shape(item::ARROW_ENG, vec![p(100.0, 260.0).0, p(100.0, 260.0).1, p(400.0, 260.0).0, p(330.0, 260.0).1], 0xFF111111, 3.0)));
            stroke(&mut b, 0xFF111111, 3.0, false, &[p(70.0, 300.0), p(120.0, 330.0), p(180.0, 290.0), p(240.0, 335.0), p(300.0, 295.0), p(360.0, 330.0)]);
            stroke(&mut b, 0xFFFFC53D, 14.0, true, &[p(60.0, 380.0), p(300.0, 384.0)]);
            b.items.push(Rc::new(Item::note("Welle 2: Speichern", 0xFF1F2025, 40.0, p(80.0, 420.0).0, p(80.0, 420.0).1, 280.0, 400.0, 0xFFFFE08A)));
        }
        1 => {
            for i in 0..6 {
                let y = 100.0 + i as f32 * 28.0;
                stroke(&mut b, 0xFF3E63DD, 2.4, false, &[p(60.0, y), p(110.0, y - 8.0), p(170.0, y + 5.0), p(240.0, y - 6.0), p(330.0 - (i as f32) * 20.0, y)]);
            }
            b.items.push(Rc::new(Item::shape(item::ELLIPSE, vec![p(300.0, 300.0).0, p(300.0, 300.0).1, p(420.0, 300.0).0, p(380.0, 300.0).1], 0xFF30A46C, 3.0)));
            stroke(&mut b, 0xFFE5484D, 4.0, false, &[p(80.0, 300.0), p(140.0, 380.0), p(200.0, 300.0), p(260.0, 380.0)]);
        }
        _ => {
            b.items.push(Rc::new(Item::note("Ideen sammeln", 0xFF1F2025, 40.0, p(60.0, 80.0).0, p(60.0, 80.0).1, 220.0, 300.0, 0xFFA5D8FF)));
            b.items.push(Rc::new(Item::note("Logo?", 0xFF1F2025, 40.0, p(320.0, 140.0).0, p(320.0, 140.0).1, 180.0, 300.0, 0xFFFFE08A)));
            stroke(&mut b, 0xFF8E4EC6, 5.5, false, &[p(80.0, 300.0), p(160.0, 260.0), p(240.0, 340.0), p(320.0, 280.0), p(400.0, 360.0)]);
        }
    }
    b.board_mode = Default::default();
    b
}

fn thumb_texture(ctx: &egui::Context, b: &Board, name: &str) -> egui::TextureHandle {
    let pm = b.thumb();
    let img = egui::ColorImage::from_rgba_premultiplied([pm.width() as usize, pm.height() as usize], pm.data());
    ctx.load_texture(name, img, egui::TextureOptions::LINEAR)
}

pub fn setup(app: &mut App, ctx: &egui::Context, spec: &str) {
    let toks: Vec<&str> = spec.split(',').map(str::trim).collect();
    let has = |t: &str| toks.contains(&t);
    if has("light") {
        app.sh.settings.theme = Theme::Light;
    }
    // Andocken: Werkzeugleiste links, Rückgängig-Leiste rechts, Auswahlleiste rechts (senkrecht)
    if has("dock") {
        use crate::settings::{BarPos, Dock};
        for ff in ["Expanded", "Medium", "Compact"] {
            for (id, e, f) in [("bar-tools", BarPos::Left, [0.0, 0.3]), ("bar-tr", BarPos::Right, [0.0, 0.0]), ("bar-sel", BarPos::Right, [0.0, 1.0])] {
                let key = format!("{id}@{ff}");
                app.sh.settings.tile_pos.insert(key.clone(), f);
                app.sh.settings.tile_dock.insert(key, Dock { edge: Some(e), vert: true });
            }
        }
    }
    let ms = owndraw_core::store::now_ms();
    if has("start") {
        let names = [("Maschinenbau Skizze", false), ("Analysis Vorlesung", true), ("Ideen", false), ("Einkaufsliste", true), ("Logo-Entwurf", false), ("Statik Übung 3", true), ("Whiteboard Montag", false)];
        let (sh, start, recent) = app.parts();
        let _ = sh;
        for (i, (n, pages)) in names.iter().enumerate().rev() {
            let f = FileRef::Path(PathBuf::from(format!("/home/demo/Dokumente/OwnDraw/{n}.owndraw")));
            recent.add(RecentEntry { file: f.clone(), name: n.to_string(), modified_ms: ms - (i as i64) * 86_400_000 - 3_600_000, pages: *pages });
            if i != 4 {
                let b = demo_board(i as u32, 0.0, 0.0);
                start.set_thumb(&f, thumb_texture(ctx, &b, &format!("demo-thumb-{i}")));
            }
        }
    }
    if has("editor") || has("pages") {
        let pages = has("pages");
        let params = NewParams { name: if pages { "Analysis Vorlesung".into() } else { "Maschinenbau Skizze".into() }, pages, ..NewParams::default() };
        let mut doc = session::new_document(&params);
        let mut b = demo_board(if pages { 1 } else { 0 }, 0.0, 0.0);
        b.board_mode = doc.mode.clone();
        doc.board = b.to_json();
        let sh = app.parts().0;
        let mut ed = Editor::open(sh, doc, None);
        ed.fit_on_open = !pages;
        for t in &toks {
            if let Some(n) = t.strip_prefix("tool=") {
                ed.board.set_tool(match n {
                    "marker" => board::MARKER,
                    "eraser" => board::ERASER,
                    "select" => board::SELECT,
                    "shape" => board::SHAPE,
                    "text" => board::TEXT,
                    _ => board::PEN,
                });
            }
        }
        if has("shape4") {
            ed.board.shape_kind = 4;
        }
        if has("sel") {
            ed.board.set_tool(board::SELECT);
            let ids: Vec<u64> = ed.board.items.iter().take(2).map(|i| i.id).collect();
            ed.board.sel = ids;
        }
        // Texteingabe auf dem Blatt: neuer freier Text (textnew), Haftnotiz bearbeiten (textedit),
        // Text ausgewählt mit erweitertem Menü (textsel)
        if has("textnew") || has("textsel") {
            ed.board.items.push(Rc::new(Item::note("Freier Text\nzweite Zeile", 0xFFE5484D, 34.0, 520.0, 260.0, 0.0, 0.0, 0)));
        }
        if has("textnew") {
            ed.board.set_tool(board::TEXT);
            ed.demo_text(520.0, 120.0, "Direkt hier tippen|");
        }
        if has("textedit") {
            ed.board.set_tool(board::TEXT);
            if let Some(id) = ed.board.items.iter().find(|i| i.as_note().map(|n| n.bg != 0).unwrap_or(false)).map(|i| i.id) {
                ed.edit_note(id);
                ed.demo_append("\nmit Cursor am Ort");
            }
        }
        if has("textsel") {
            ed.board.set_tool(board::TEXT);
            let id = ed.board.items.last().map(|i| i.id).unwrap_or(0);
            ed.board.set_tool(board::SELECT);
            ed.board.select_only(id);
        }
        if has("edittools") {
            let sh = app.parts().0;
            ed.begin_edit_tools(sh);
        }
        ed.st.opts_open = has("options");
        ed.st.menu_open = has("menu");
        ed.st.layers_open = has("layers");
        if has("pagesclosed") {
            ed.st.pages_open = Some(false);
        }
        if has("pagelist") {
            ed.st.pages_open = Some(true);
        }
        app.open_editor(ctx, ed);
    }
    if has("autosave") {
        // Abnahme Autosave: neues Board im Standard-Ort, danach ein Strich → Datei muss nach > 2 s aktualisiert sein.
        let params = NewParams { name: "Autosave-Test".into(), pages: false, ..NewParams::default() };
        let doc = session::new_document(&params);
        let sh = app.parts().0;
        if let Ok(f) = sh.platform.create_in(&sh.platform.default_dir(), "Autosave-Test.owndraw", crate::platform::FileKind::Owndraw) {
            let _ = session::write_document(&*sh.platform, &f, &doc);
            let mut ed = Editor::open(sh, doc, Some(f));
            stroke(&mut ed.board, 0xFF111111, 3.0, false, &[(60.0, 80.0), (200.0, 160.0), (320.0, 90.0)]);
            ed.board.modified = true;
            app.open_editor(ctx, ed);
        }
    }
    if has("new") {
        app.sh.dialog = Some(Dialog::new_doc(true));
    }
    if has("settings") {
        app.set_screen(Screen::Settings);
    }
}
