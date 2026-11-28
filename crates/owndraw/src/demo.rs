//! Abnahme-Hilfen für Screenshots (nur aktiv mit Umgebungsvariablen, sonst ohne Wirkung):
//!
//! - `OWNDRAW_SCREENSHOT=pfad.png`: nach ein paar Frames Bild des Fensters speichern und beenden.
//! - `OWNDRAW_DEMO=a,b,…`: Ausgangszustand herstellen. Bausteine: `start` (Beispiel-Zuletzt-Liste), `gast` (Gast-Hinweis auf der Startseite), `editor`
//!   (unendliches Demo-Board), `pages` (Seiten-Demo), `tool=pen|marker|eraser|select|shape|text`, `options`, `menu`,
//!   `layers`, `pagelist`, `edittools` (Werkzeugleiste im Bearbeiten-Modus), `new`, `settings`, `tutorial`, `confirm`, `rename`, `tutorial`, `confirm`, `rename`, `light` (helles Theme), `classic` (klassisches Design),
//!   `autosave` (Board im Standard-Ort mit Strich), `textnew`, `textedit`, `textsel` (Texteingabe auf dem Blatt), `textlines` (Text auf den Linien einer linierten Seite, mit `pages`), `dock` (Leisten seitlich angedockt), `docklate` (dasselbe erst nach 10 Frames), `docktop` (bündig oben, Seiten-Kachel links), `math` (Formel-Werkzeug mit offenem Editor), `more` (Popover „Weitere Werkzeuge“), `settingstools`/`settingskeys` (Einstellungen, Unterseite), `settings=<look|bars|snap|hw|save|account|account-loeschen|update|about>` (Kategorie-Unterseite), `ruler` (Lineal), `laser` (Laserpunkte, eigener und fremde), `mathinline` (Formel direkt auf dem Blatt bearbeiten), `calc` (Werkzeug „Rechnen“ mit Beispielzeilen, `steps` = mit Rechenweg), `convert` (Werkzeug „Umwandeln“ mit Fenster und festen Alternativen), `stamps` (alle Stempel), `fsugg` (Formel-Vorschläge), `fbook[=konst|teilen|neu]` (Formelsammlung), `export` (Export-Fenster), `writer[,wfind,wimg,wfull]` (Textdokument; `wfull` = Tabelle, Diagramm, Fußnote, Kommentar, Änderung), mit `OWNDRAW_SERVER`: `login`, `einladung`, `srv=<id>`, `share`.

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

/// Leisten seitlich andocken (Demo `dock`; `docklate` erst nach 10 Frames, wie nach einem Ziehen).
pub fn apply_dock(settings: &mut crate::settings::Settings) {
    use crate::settings::{BarPos, Dock};
    for ff in ["Expanded", "Medium", "Compact"] {
        for (id, e, f) in [("bar-tools", BarPos::Left, [0.0, 0.3]), ("bar-tr", BarPos::Right, [0.0, 0.0]), ("bar-sel", BarPos::Right, [0.0, 1.0])] {
            let key = format!("{id}@{ff}");
            settings.tile_pos.insert(key.clone(), f);
            settings.tile_dock.insert(key, Dock { edge: Some(e), vert: true });
        }
    }
}

pub struct Shot {
    path: PathBuf,
    frames: u32,
    done: bool,
}

impl Shot {
    pub fn from_env() -> Option<Shot> {
        std::env::var("OWNDRAW_SCREENSHOT").ok().filter(|s| !s.is_empty()).map(|p| Shot { path: PathBuf::from(p), frames: 0, done: false })
    }

    pub fn frames(&self) -> u32 {
        self.frames
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
    if has("classic") {
        crate::style::set_modern(&mut app.sh.settings, false);
    }
    // Andocken: Werkzeugleiste links, Rückgängig-Leiste rechts, Auswahlleiste rechts (senkrecht)
    if has("dock") {
        apply_dock(&mut app.sh.settings);
    }
    // Bündig oben: Titel- und Werkzeugleiste am oberen Rand, Seiten-Kachel links senkrecht
    if has("docktop") {
        use crate::settings::{BarPos, Dock};
        for ff in ["Expanded", "Medium", "Compact"] {
            for (id, e, f) in [("bar-tl", BarPos::Top, [0.0, 0.0]), ("bar-tools", BarPos::Top, [0.5, 0.0]), ("pages-chip", BarPos::Left, [0.0, 0.4])] {
                let key = format!("{id}@{ff}");
                app.sh.settings.tile_pos.insert(key.clone(), f);
                app.sh.settings.tile_dock.insert(key, Dock { edge: Some(e), vert: matches!(e, BarPos::Left | BarPos::Right) });
            }
        }
    }
    let ms = owndraw_core::store::now_ms();
    if has("writer") {
        app.open_writer_demo(ctx, writer_sample(), has("wfind"));
        return;
    }
    if has("start") {
        let names = [("Maschinenbau Skizze", false), ("Analysis Vorlesung", true), ("Ideen", false), ("Einkaufsliste", true), ("Logo-Entwurf", false), ("Statik Übung 3", true), ("Whiteboard Montag", false)];
        let (sh, start, recent) = app.parts();
        let _ = sh;
        for (i, (n, pages)) in names.iter().enumerate().rev() {
            // Dateien wirklich anlegen (leer), damit nur „Logo-Entwurf“ als fehlend erscheint
            let dir = std::env::temp_dir().join("owndraw-demo");
            let path = dir.join(format!("{n}.owndraw"));
            if i != 4 {
                let _ = std::fs::create_dir_all(&dir);
                let _ = std::fs::write(&path, b"");
            }
            let f = FileRef::Path(path);
            recent.add(RecentEntry { file: f.clone(), name: n.to_string(), modified_ms: ms - (i as i64) * 86_400_000 - 3_600_000, pages: *pages, text: false });
            if i != 4 {
                let b = demo_board(i as u32, 0.0, 0.0);
                start.set_thumb(&f, thumb_texture(ctx, &b, &format!("demo-thumb-{i}")));
            }
        }
    }
    // Konto (mit `OWNDRAW_SERVER`): `login`, `einladung` (Dialoge), `srv=<id>` (Server-Board öffnen),
    // `share` (Teilen-Dialog im Server-Board)
    for t in &toks {
        if let Some(id) = t.strip_prefix("srv=") {
            crate::net::open_board(id);
        } else if matches!(*t, "login" | "einladung") {
            crate::ui::account::demo_dialog(&mut app.parts().1.account, t);
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
        for x in ["xtable", "xpin", "xsel", "xcrop", "xconn"] {
            if has(x) {
                ed.demo_x(x);
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
        if has("textlines") {
            // Text auf den Linien: fertiger Text und laufende Eingabe darunter
            ed.board.set_tool(board::TEXT);
            let r = ed.board.new_note_rect(80.0, 170.0, false);
            ed.board.add_note_rect("Auf den Linien geschrieben\nzweite Zeile, nächste Linie", r, false);
            ed.demo_text(80.0, 270.0, "Hier wird gerade getippt|");
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
        // Formel-Werkzeug: eingeschaltet, eine Formel auf dem Blatt und der Editor mit Vorschau offen
        if has("math") {
            app.parts().0.settings.tool_on.insert("math".into(), true);
            ed.board.add_math(520.0, 300.0, 34.0, "\\sum_{k=1}^{n} k = \\frac{n(n+1)}{2}");
            ed.select_tool(app.parts().0, crate::ui::editor::Tool::Math);
            ed.demo_math(520.0, 120.0, "x_{1,2} = \\frac{-b \\pm \\sqrt{b^2-4ac}}{2a}");
        }
        if has("calc") {
            // Werkzeug „Rechnen“: Beispiel von Le, Umstellen, quadratische Gleichung, Gleichungssystem
            let sh = app.parts().0;
            sh.settings.tool_on.insert("calc".into(), true);
            if has("steps") {
                sh.settings.calc.steps = true;
            }
            ed.board.calc = sh.settings.calc.to_core();
            ed.select_tool(sh, crate::ui::editor::Tool::Calc);
            let mut l: Vec<(f32, f32, &str)> = Vec::new();
            for (i, s) in ["a: 1 N", "b: 2 m", "M = a·b", "M = kN·m", "M ="].into_iter().enumerate() {
                l.push((120.0, 140.0 + i as f32 * 55.0, s));
            }
            for (i, s) in ["F = 10 kN", "p = 5 MPa", "p = F/A", "A = mm²", "z² - 5z + 6 = 0", "z =", "U = 12 V + 3 m"].into_iter().enumerate() {
                l.push((560.0, 140.0 + i as f32 * 55.0, s));
            }
            for (i, s) in ["x + y = 10", "x - y = 2", "x =", "y ="].into_iter().enumerate() {
                l.push((1000.0, 140.0 + i as f32 * 55.0, s));
            }
            ed.demo_calc(sh, &l, (120.0, 470.0), "v = 72 km/h");
        }
        if has("fsugg") {
            // Formel-Vorschläge im Rechnen-Feld
            let sh = app.parts().0;
            sh.settings.tool_on.insert("calc".into(), true);
            ed.board.calc = sh.settings.calc.to_core();
            ed.select_tool(sh, crate::ui::editor::Tool::Calc);
            let l: Vec<(f32, f32, &str)> = ["U: 230 V", "I: 2 A"].iter().enumerate().map(|(i, s)| (120.0, 140.0 + i as f32 * 55.0, *s)).collect();
            ed.demo_calc(sh, &l, (120.0, 250.0), "P =");
        }
        if let Some(t) = toks.iter().find_map(|s| s.strip_prefix("fbook")) {
            // Formelsammlung: fbook (Formeln), fbook=konst, fbook=teilen, fbook=neu (Formular)
            ed.demo_fbook(t.trim_start_matches('='));
        }
        if has("mathinline") {
            // Formel direkt auf dem Blatt bearbeiten (ohne Formel-Fenster)
            let sh = app.parts().0;
            sh.settings.tool_on.insert("calc".into(), true);
            ed.board.calc = sh.settings.calc.to_core();
            let size = ed.board.math_size() * 1.3;
            let mut id = 0;
            for (i, s) in ["F = 10 kN", "A = 20 mm²", "σ = F/A", "σ = MPa"].into_iter().enumerate() {
                let k = ed.board.add_math(560.0, 150.0 + i as f32 * 70.0, size, &owndraw_core::calc::to_latex(s)).unwrap_or(0);
                if i == 2 {
                    id = k;
                }
            }
            ed.board.set_tool(board::SELECT);
            ed.edit_math_inline(id);
        }
        if has("convert") {
            // Werkzeug „Umwandeln“: Handschrift „F = 2 kN“ und das Fenster mit Alternativen (ohne Modell, feste Ergebnisse)
            let sh = app.parts().0;
            sh.settings.tool_on.insert("convert".into(), true);
            ed.select_tool(sh, crate::ui::editor::Tool::Convert);
            ed.board.add_math(120.0, 120.0, 30.0, &owndraw_core::calc::to_latex("F: 2 kN"));
            let col = 0xFF3E63DD;
            let n0 = ed.board.items.len();
            stroke(&mut ed.board, col, 3.0, false, &[(120.0, 240.0), (120.0, 290.0)]);
            stroke(&mut ed.board, col, 3.0, false, &[(120.0, 240.0), (145.0, 240.0)]);
            stroke(&mut ed.board, col, 3.0, false, &[(120.0, 263.0), (140.0, 263.0)]);
            stroke(&mut ed.board, col, 3.0, false, &[(165.0, 258.0), (190.0, 258.0)]);
            stroke(&mut ed.board, col, 3.0, false, &[(165.0, 272.0), (190.0, 272.0)]);
            stroke(&mut ed.board, col, 3.0, false, &[(212.0, 250.0), (225.0, 240.0), (238.0, 250.0), (212.0, 290.0), (242.0, 290.0)]);
            stroke(&mut ed.board, col, 3.0, false, &[(262.0, 240.0), (262.0, 290.0)]);
            stroke(&mut ed.board, col, 3.0, false, &[(282.0, 258.0), (264.0, 274.0), (284.0, 290.0)]);
            stroke(&mut ed.board, col, 3.0, false, &[(296.0, 290.0), (296.0, 240.0), (322.0, 290.0), (322.0, 240.0)]);
            let ids: Vec<u64> = ed.board.items[n0..].iter().map(|i| i.id).collect();
            use owndraw_core::hw::Art;
            ed.demo_convert(
                ids,
                &[
                    (Art::Formel, vec![("F=2\\mathrm{kN}", -0.21), ("F=2kN", -0.35), ("F=2\\,\\mathrm{kN}", -0.4), ("E=2kN", -0.5)]),
                    (Art::Text, vec![("F = 2 kN", -0.3), ("F = 2kN", -0.4), ("F=2 KN", -0.6)]),
                    (Art::Zahl, vec![("F=2kN", -0.9), ("2 kN", -1.2)]),
                ],
                Art::Formel,
            );
        }
        if has("ruler") {
            ed.demo_ruler();
        }
        if has("laser") {
            let sh = app.parts().0;
            crate::ui::editor::tools::set_enabled(&mut sh.settings, "laser", true);
            ed.select_tool(sh, crate::ui::editor::Tool::Laser);
            ed.demo_laser();
        }
        if has("stamps") {
            // alle Stempel nebeneinander (Größenvergleich ? und ! mit Haken/Kreuz/Stern)
            for (i, k) in [board::STAMP_CHECK, board::STAMP_CROSS, board::STAMP_STAR, board::STAMP_QUESTION, board::STAMP_EXCLAIM].into_iter().enumerate() {
                ed.board.stamp(k, 900.0 + i as f32 * 70.0, 520.0, 48.0);
            }
        }
        if has("edittools") {
            let sh = app.parts().0;
            ed.begin_edit_tools(sh);
        }
        ed.st.opts_open = has("options");
        ed.st.menu_open = has("menu");
        ed.st.more_open = has("more");
        ed.st.export.open = has("export");
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
        app.sh.dialog = Some(Dialog::new_doc(true, None));
    }
    if has("settings") || has("settingstools") || has("settingskeys") || toks.iter().any(|s| s.starts_with("settings=")) {
        if has("settingstools") {
            crate::ui::settings::open_tools_page(ctx);
        }
        if has("settingskeys") {
            crate::ui::settings::open_keys_page(ctx);
        }
        if let Some(t) = toks.iter().find_map(|s| s.strip_prefix("settings=")) {
            crate::ui::settings::open_page_named(ctx, t);
        }
        app.set_screen(Screen::Settings);
    }
    if has("tutorial") {
        app.set_screen(Screen::Tutorial);
    }
    if has("confirm") {
        app.sh.dialog = Some(Dialog::Confirm { title: "Ebene löschen?", text: "Die Ebene „Hintergrund“ und ihr Inhalt werden entfernt. Das lässt sich mit Rückgängig wieder herstellen.".into(), ok: "Löschen", action: crate::ui::dialogs::ConfirmAction::DeleteLayer(0) });
    }
    if has("rename") {
        app.sh.dialog = Some(Dialog::Rename { what: crate::ui::dialogs::RenameWhat::Doc, title: "Board umbenennen", text: "Maschinenbau Skizze".into(), focus: true });
    }
}

/// Beispiel-Textdokument für Screenshots.
fn writer_sample() -> owndraw_core::doc::Document {
    use owndraw_core::office::{CStyle, Kind, Para, Run, TextDoc};
    let mut td = TextDoc::new();
    let run = |t: &str, st: CStyle| Run { text: t.into(), st };
    let n = CStyle::default;
    let mut ps = vec![
        Para::with_text(Kind::Title, "Projektbericht Brückenbau"),
        Para::with_text(Kind::H1, "1  Ausgangslage"),
    ];
    let mut p = Para::new(Kind::Body);
    p.runs = vec![
        run("Die Fußgängerbrücke über die Ilm wird ", n()),
        run("bis Ende März", CStyle { b: true, ..n() }),
        run(" erneuert. Dieser Bericht fasst ", n()),
        run("Statik, Kosten und Zeitplan", CStyle { hl: 0xFFFDE68A, ..n() }),
        run(" zusammen und dient als Grundlage für die Sitzung am Donnerstag. Alle Werte sind gerundet; die genauen Zahlen stehen im Anhang.", n()),
    ];
    ps.push(p);
    ps.push(Para::with_text(Kind::H2, "1.1  Ziele"));
    for t in ["Tragfähigkeit von 5 kN/m² nachweisen", "Bauzeit unter sechs Wochen halten", "Kosten im Rahmen von 180 000 € bleiben"] {
        ps.push(Para::with_text(Kind::Bullet, t));
    }
    ps.push(Para::with_text(Kind::H2, "1.2  Nächste Schritte"));
    for t in ["Angebote einholen", "Gutachten beauftragen", "Termin mit der Stadt vereinbaren"] {
        ps.push(Para::with_text(Kind::Number, t));
    }
    let mut q = Para::with_text(Kind::Quote, "„Gute Planung spart doppelt so viel Zeit, wie sie kostet.“");
    q.align = owndraw_core::office::Align::Center;
    ps.push(q);
    let mut p = Para::new(Kind::Body);
    p.runs = vec![
        run("Weitere Informationen gibt es auf ", n()),
        run("owndraw.de", CStyle { link: Some("https://owndraw.de".into()), ..n() }),
        run(". Die Formel für die Durchbiegung lautet w = 5qL", n()),
        run("4", CStyle { va: owndraw_core::office::VAlign::Sup, ..n() }),
        run("/384EI.", n()),
    ];
    ps.push(p);
    td.paras = ps;
    td.footer = "Seite {seite} von {seiten}".into();
    if std::env::var("OWNDRAW_DEMO").is_ok_and(|d| d.split(',').any(|t| t.trim() == "wfull")) {
        writer_full(&mut td);
    }
    let mut d = owndraw_core::doc::Document::new_text("Projektbericht Brückenbau");
    d.text = Some(td.to_json());
    d
}

/// Demo `wfull`: Tabelle, Diagramm mit Umfluss, Beschriftung, Fußnote, Kommentar und nachverfolgte Änderung.
fn writer_full(td: &mut owndraw_core::office::TextDoc) {
    use owndraw_core::office::objects::{Cell, Comment, Field, Obj, Table, Wrap, OBJ};
    use owndraw_core::office::{CStyle, Kind, Para};
    let w = td.page.content_w();
    // Fußnote und Kommentar im ersten Absatz
    let id = td.ex.id();
    td.ex.comments.push(Comment { id, author: "Mia Berger".into(), time: "2026-11-28T09:12".into(), text: "Termin mit dem Bauamt abstimmen?".into(), replies: Vec::new(), done: false });
    let p = &mut td.paras[2];
    let n = p.len();
    p.insert(n, &OBJ.to_string(), &CStyle { field: Some(Field::Note("Laut Ausschreibung vom 3. November.".into())), ..CStyle::default() });
    p.restyle(4, 17, &|s: &mut CStyle| s.cmt = id);
    let rev = td.ex.rev("Jonas Kern", "2026-11-28T10:05");
    let p = &mut td.paras[2];
    p.insert(4, "neue ", &CStyle { ins: rev, ..CStyle::default() });
    // Diagramm rechts mit Textumfluss und Beschriftung
    let mut o = Para::new(Kind::Object);
    let mut ch = Obj::chart();
    ch.w = 220.0;
    ch.h = 150.0;
    o.obj = Some(ch);
    o.wrap = Wrap::Right;
    td.paras.insert(3, o);
    // Tabelle mit Kopfzeile und Beschriftung am Ende
    let mut t = Table::new(4, 3, w);
    for (r, row) in [["Posten", "Menge", "Kosten"], ["Stahlträger", "12", "48 000 €"], ["Holzbelag", "160 m²", "22 400 €"], ["Montage", "6 Wochen", "61 000 €"]].iter().enumerate() {
        for (c, v) in row.iter().enumerate() {
            t.rows[r][c] = Cell::new(v);
        }
    }
    t.header = true;
    t.stripes = true;
    let mut tp = Para::new(Kind::Table);
    tp.table = Some(t);
    let mut cap = Para::with_text(Kind::Caption, "Kostenübersicht");
    cap.cap = "Tabelle".into();
    td.paras.insert(5, tp);
    td.paras.insert(6, cap);
}
