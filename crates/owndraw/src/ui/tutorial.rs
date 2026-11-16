//! Anleitung („Anleitung & Tipps“): erklärt alle Editor-Funktionen und sammelt Tipps & Tricks.
//!
//! Der Inhalt steht an EINER Stelle: `crates/owndraw/tutorial.json` (Kategorien mit Einträgen). Neue
//! Editor-Funktionen werden dort eingetragen – der Test `tutorial_deckt_editor_ab` unten prüft, dass jedes
//! Werkzeug, jede Tastenkürzel-Aktion, jeder Menüeintrag des Editors und jede Editor-Einstellung einen Eintrag hat
//! (Feld `covers`). Siehe `CONTRIBUTING.md`.

use crate::layout::{Env, FormFactor};
use crate::settings::Settings;
use crate::ui::editor::keys;
use crate::style::{self, c};
use egui::{CornerRadius, Margin, Stroke, Ui, Vec2};
use serde::Deserialize;
use std::sync::OnceLock;

/// Inhalt der Anleitung (eingebettet, damit sie überall ohne Netz funktioniert).
pub const SOURCE: &str = include_str!("../../tutorial.json");

#[derive(Deserialize, Debug)]
pub struct Data {
    pub categories: Vec<Category>,
}

#[derive(Deserialize, Debug)]
pub struct Category {
    #[allow(dead_code)] // nur im Test geprüft (eindeutig)
    pub id: String,
    pub title: String,
    #[serde(default)]
    pub intro: String,
    pub entries: Vec<Entry>,
}

#[derive(Deserialize, Debug)]
pub struct Entry {
    #[allow(dead_code)] // nur im Test geprüft (eindeutig)
    pub id: String,
    pub title: String,
    pub text: String,
    /// Schritte bzw. Wege, die Funktion auszulösen.
    #[serde(default)]
    pub how: Vec<String>,
    /// Weitere Tasten/Gesten zur Anzeige, z. B. "Strg+Mausrad" (belegbare Kürzel stehen in `action`).
    #[serde(default)]
    pub keys: Vec<String>,
    /// Aktionen der Tastenkürzel (`undo`, `tool.pen`, …): Die Seite zeigt deren aktuelle Belegung aus den
    /// Einstellungen. Ein String oder eine Liste.
    #[serde(default, deserialize_with = "one_or_many")]
    pub action: Vec<String>,
    /// Einschränkung, z. B. "Nur am PC" oder "Nur in der Web-App".
    #[serde(default)]
    pub only: String,
    /// Maschinenlesbare Kennungen, die der Test mit dem Code abgleicht (`tool:pen`, `key:Z`, `menu:Als PDF …`, `setting:Taste`).
    #[serde(default)]
    #[allow(dead_code)] // nur im Test (Abgleich mit dem Code)
    pub covers: Vec<String>,
}

fn one_or_many<'de, D: serde::Deserializer<'de>>(d: D) -> Result<Vec<String>, D::Error> {
    #[derive(Deserialize)]
    #[serde(untagged)]
    enum V {
        One(String),
        Many(Vec<String>),
    }
    Ok(match V::deserialize(d)? {
        V::One(s) => vec![s],
        V::Many(v) => v,
    })
}

/// Angezeigte Tasten eines Eintrags: aktuelle Belegung der Aktionen, dann die festen `keys`.
pub fn key_caps(e: &Entry, s: &Settings) -> Vec<String> {
    let mut v: Vec<String> = Vec::new();
    for id in &e.action {
        let Some(a) = keys::find(id) else { continue };
        if !keys::available(s, a) {
            continue;
        }
        for l in keys::labels(s, id) {
            let l = match a.kind {
                keys::Kind::Hold => format!("{l} + Ziehen"),
                keys::Kind::Modifier => format!("{l} halten"),
                keys::Kind::Press => l,
            };
            // mehrere Aktionen: Name dazu („Strg+Z Rückgängig“)
            let l = if e.action.len() > 1 { format!("{l} {}", a.name) } else { l };
            if !v.contains(&l) {
                v.push(l);
            }
        }
    }
    v.extend(e.keys.iter().cloned());
    v
}

pub fn data() -> &'static Data {
    static D: OnceLock<Data> = OnceLock::new();
    D.get_or_init(|| serde_json::from_str(SOURCE).expect("tutorial.json ist ungültig"))
}

/// Zustand der Seite (Suche, gewählte Kategorie).
#[derive(Default)]
pub struct TutorialState {
    pub query: String,
    /// `None` = alle Kategorien.
    pub cat: Option<usize>,
}

/// Web-App: `/#tutorial` (bzw. `/tutorial` am Server, leitet dorthin um) öffnet direkt die Anleitung.
/// Das Fragment wird danach aus der Adresszeile entfernt.
pub fn requested_by_url() -> bool {
    #[cfg(target_arch = "wasm32")]
    {
        let Some(w) = web_sys::window() else { return false };
        let loc = w.location();
        if loc.hash().ok().as_deref() != Some("#tutorial") {
            return false;
        }
        if let (Ok(path), Ok(search), Ok(h)) = (loc.pathname(), loc.search(), w.history()) {
            let _ = h.replace_state_with_url(&wasm_bindgen::JsValue::NULL, "", Some(&format!("{path}{search}")));
        }
        true
    }
    #[cfg(not(target_arch = "wasm32"))]
    {
        false
    }
}

/// Kleinbuchstaben, Umlaute vereinfacht – damit „uber“ auch „Über“ findet.
fn norm(s: &str) -> String {
    s.to_lowercase().replace('ä', "a").replace('ö', "o").replace('ü', "u").replace('ß', "ss")
}

/// Passt der Eintrag zur Suche? Alle Wörter müssen irgendwo vorkommen.
pub fn matches(cat: &Category, e: &Entry, query: &str) -> bool {
    let q = norm(query);
    if q.trim().is_empty() {
        return true;
    }
    let caps = key_caps(e, &Settings::default());
    let hay = norm(&format!("{} {} {} {} {} {}", cat.title, e.title, e.text, e.how.join(" "), caps.join(" "), e.only));
    q.split_whitespace().all(|w| hay.contains(w))
}

const MAX_W: f32 = 820.0;

/// Zeichnet die Anleitung; liefert true, wenn „Zurück“ gewählt wurde.
pub fn show(ui: &mut Ui, env: &Env, st: &mut TutorialState, settings: &Settings) -> bool {
    let compact = env.ff == FormFactor::Compact;
    let side = if compact { 16 } else { 32 };
    let mut back = false;
    // Kopf bündig mit der Inhaltsspalte
    let full = ui.available_rect_before_wrap();
    let w = (full.width() - 2.0 * side as f32).clamp(100.0, MAX_W);
    let head = egui::Rect::from_min_size(egui::pos2(full.center().x - w / 2.0, full.top() + 12.0), Vec2::new(w, env.btn));
    ui.scope_builder(egui::UiBuilder::new().max_rect(head).layout(egui::Layout::left_to_right(egui::Align::Center)), |ui| {
        let r = if style::modern() { style::outline_btn(ui, "‹ Zurück", env.btn) } else { style::plain_btn(ui, "‹ Zurück", env.btn) };
        back = r.clicked();
        ui.label(egui::RichText::new("Anleitung & Tipps").size(22.0).strong());
    });
    ui.add_space(4.0);
    if ui.input(|i| i.key_pressed(egui::Key::Escape)) && !ui.ctx().egui_wants_keyboard_input() {
        back = true;
    }
    egui::ScrollArea::vertical().auto_shrink(false).show(ui, |ui| {
        // Spalte mittig (wie die moderne Startseite), nie breiter als der Bildschirm
        let full = ui.available_rect_before_wrap();
        let w = (full.width() - 2.0 * side as f32).clamp(100.0, MAX_W);
        let col = egui::Rect::from_min_size(egui::pos2(full.center().x - w / 2.0, full.top()), Vec2::new(w, f32::INFINITY));
        ui.scope_builder(egui::UiBuilder::new().max_rect(col).layout(egui::Layout::top_down(egui::Align::Min)), |ui| {
            ui.set_max_width(w);
            content(ui, env, st, settings);
            ui.add_space(32.0);
        });
    });
    back
}

fn content(ui: &mut Ui, env: &Env, st: &mut TutorialState, settings: &Settings) {
    let p = style::pal();
    let d = data();
    ui.add_space(8.0);
    ui.label(egui::RichText::new("Alle Funktionen des Editors auf einen Blick – mit Tastenkürzeln, Gesten und Tipps. Suche nach einem Stichwort oder wähle einen Bereich.").color(c(p.dim)));
    ui.add_space(10.0);
    let te = egui::TextEdit::singleline(&mut st.query).hint_text("Anleitung durchsuchen … (z. B. Pfeil, Zoom, Strg)").desired_width(ui.available_width()).margin(egui::Margin::symmetric(12, 9));
    ui.add(te);
    ui.add_space(8.0);
    let chip_h = if env.touch { env.btn } else { 30.0 };
    ui.horizontal_wrapped(|ui| {
        if style::chip(ui, "Alle", st.cat.is_none(), chip_h).clicked() {
            st.cat = None;
        }
        for (i, cat) in d.categories.iter().enumerate() {
            if style::chip(ui, &cat.title, st.cat == Some(i), chip_h).clicked() {
                st.cat = if st.cat == Some(i) { None } else { Some(i) };
            }
        }
    });
    let mut shown = 0;
    for (i, cat) in d.categories.iter().enumerate() {
        if st.cat.is_some_and(|s| s != i) {
            continue;
        }
        let hits: Vec<&Entry> = cat.entries.iter().filter(|e| matches(cat, e, &st.query)).collect();
        if hits.is_empty() {
            continue;
        }
        shown += hits.len();
        heading(ui, &cat.title);
        if !cat.intro.is_empty() && st.query.trim().is_empty() {
            ui.label(egui::RichText::new(&cat.intro).color(c(p.dim)));
            ui.add_space(6.0);
        }
        for e in hits {
            entry(ui, e, settings);
            ui.add_space(8.0);
        }
    }
    if shown == 0 {
        ui.add_space(24.0);
        ui.vertical_centered(|ui| {
            ui.label(egui::RichText::new("Nichts gefunden.").strong());
            ui.label(egui::RichText::new("Versuche ein anderes Stichwort oder wähle „Alle“.").color(c(p.dim)));
        });
    }
}

fn heading(ui: &mut Ui, title: &str) {
    let p = style::pal();
    if style::modern() {
        ui.add_space(22.0);
        ui.label(egui::RichText::new(title).size(17.0).strong().color(c(p.text)));
    } else {
        ui.add_space(16.0);
        ui.label(egui::RichText::new(title.to_uppercase()).size(13.0).strong().color(c(p.dim)));
    }
    ui.add_space(6.0);
}

fn entry(ui: &mut Ui, e: &Entry, settings: &Settings) {
    let p = style::pal();
    style::card().show(ui, |ui| {
        ui.set_width(ui.available_width());
        ui.spacing_mut().item_spacing.y = 5.0;
        ui.horizontal_wrapped(|ui| {
            ui.label(egui::RichText::new(&e.title).size(15.5).strong().color(c(p.text)));
            if !e.only.is_empty() {
                badge(ui, &e.only);
            }
        });
        ui.add(egui::Label::new(egui::RichText::new(&e.text).color(c(p.text))).wrap());
        for h in &e.how {
            ui.horizontal_top(|ui| {
                ui.label(egui::RichText::new("•").color(c(p.accent)).strong());
                ui.add(egui::Label::new(egui::RichText::new(h).color(c(p.dim))).wrap());
            });
        }
        let caps = key_caps(e, settings);
        if !caps.is_empty() {
            ui.horizontal_wrapped(|ui| {
                ui.spacing_mut().item_spacing = Vec2::new(6.0, 6.0);
                for k in &caps {
                    key_cap(ui, k);
                }
            });
        }
    });
}

/// Hinweis-Plakette (z. B. „Nur am PC“).
fn badge(ui: &mut Ui, text: &str) {
    let p = style::pal();
    egui::Frame::new()
        .fill(c(p.accent_soft))
        .corner_radius(CornerRadius::same(if style::modern() { 6 } else { 255 }))
        .inner_margin(Margin::symmetric(8, 2))
        .show(ui, |ui| {
            ui.label(egui::RichText::new(text).size(12.0).color(c(p.accent)));
        });
}

/// Taste als „Tastenkappe“.
fn key_cap(ui: &mut Ui, text: &str) {
    let p = style::pal();
    egui::Frame::new()
        .fill(c(p.hover))
        .stroke(Stroke::new(1.0, c(p.line)))
        .corner_radius(CornerRadius::same(6))
        .inner_margin(Margin::symmetric(8, 3))
        .show(ui, |ui| {
            ui.label(egui::RichText::new(text).monospace().size(12.5).color(c(p.text)));
        });
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashSet;

    /// Alle string-Literale, die direkt nach `marker` beginnen (bis zum nächsten `,` außerhalb von Literalen
    /// bzw. bei `if … { "A" } else { "B" }` beide Zweige).
    fn literals_after(src: &str, marker: &str) -> Vec<String> {
        let mut out = Vec::new();
        for (pos, _) in src.match_indices(marker) {
            let rest = &src[pos + marker.len()..];
            // Argument bis zum ersten Komma auf oberster Ebene
            let mut depth = 0i32;
            let mut in_str = false;
            let mut end = rest.len();
            for (i, ch) in rest.char_indices() {
                match ch {
                    '"' => in_str = !in_str,
                    '{' | '(' if !in_str => depth += 1,
                    '}' | ')' if !in_str => {
                        if depth == 0 {
                            end = i;
                            break;
                        }
                        depth -= 1
                    }
                    ',' if !in_str && depth == 0 => {
                        end = i;
                        break;
                    }
                    _ => {}
                }
            }
            let arg = &rest[..end];
            let mut parts = arg.split('"');
            parts.next();
            while let Some(lit) = parts.next() {
                out.push(lit.to_string());
                parts.next();
            }
        }
        out
    }

    /// Alles, was der Editor anbietet und in der Anleitung stehen muss.
    fn editor_features() -> HashSet<String> {
        let mut f = HashSet::new();
        // Werkzeuge (Registry)
        for d in crate::ui::editor::tools::TOOLS {
            f.insert(format!("tool:{}", d.name));
        }
        // Tastenkürzel: jede Aktion der Registry (auch die der Werkzeuge)
        for a in keys::all() {
            f.insert(format!("action:{}", a.id));
        }
        // Menüeinträge des „⋯“-Menüs
        for l in literals_after(include_str!("editor/menu.rs"), "menu_row(ui, ") {
            f.insert(format!("menu:{l}"));
        }
        // Editor-Einstellungen (Abschnitte „Werkzeugleisten“ und „Einrasten“)
        let s = include_str!("settings.rs");
        let a = s.find("section(ui, \"Werkzeugleisten\"").expect("Abschnitt Werkzeugleisten");
        let b = s[a..].find("// ---- Speichern").map(|i| a + i).expect("Ende Einrasten");
        for l in literals_after(&s[a..b], "row(ui, env, ") {
            if !l.is_empty() {
                f.insert(format!("setting:{l}"));
            }
        }
        f
    }

    #[test]
    fn json_gueltig_und_eindeutig() {
        let d = data();
        assert!(!d.categories.is_empty());
        let mut ids = HashSet::new();
        for cat in &d.categories {
            assert!(ids.insert(cat.id.clone()), "doppelte Kategorie {}", cat.id);
            assert!(!cat.entries.is_empty(), "Kategorie {} ist leer", cat.id);
            for e in &cat.entries {
                assert!(ids.insert(e.id.clone()), "doppelte ID {}", e.id);
                assert!(!e.title.trim().is_empty() && !e.text.trim().is_empty(), "Eintrag {} braucht Titel und Text", e.id);
            }
        }
    }

    /// Neues Editor-Feature ⇒ Eintrag in `tutorial.json` (Feld `covers`). Schlägt fehl, wenn ein Werkzeug,
    /// Tastenkürzel, Menüeintrag oder eine Editor-Einstellung in der Anleitung fehlt.
    #[test]
    fn tutorial_deckt_editor_ab() {
        let covered: HashSet<String> = data().categories.iter().flat_map(|c| c.entries.iter()).flat_map(|e| e.covers.iter().cloned()).collect();
        let mut missing: Vec<String> = editor_features().into_iter().filter(|f| !covered.contains(f)).collect();
        missing.sort();
        assert!(
            missing.is_empty(),
            "Diese Editor-Funktionen fehlen in der Anleitung (crates/owndraw/tutorial.json, Feld \"covers\"):\n  {}\nBitte einen Eintrag ergänzen oder einen bestehenden erweitern (siehe CONTRIBUTING.md).",
            missing.join("\n  ")
        );
    }

    /// Umgekehrt: `covers` darf nur auf Dinge zeigen, die es (noch) gibt – sonst veraltet die Anleitung.
    #[test]
    fn tutorial_ohne_veraltete_eintraege() {
        let feats = editor_features();
        let mut stale = Vec::new();
        for e in data().categories.iter().flat_map(|c| c.entries.iter()) {
            for cv in &e.covers {
                // `extra:` sind Funktionen ohne maschinell prüfbares Gegenstück (Gesten, Stift, Collab …)
                if !cv.starts_with("extra:") && !feats.contains(cv) {
                    stale.push(format!("{} → {cv}", e.id));
                }
            }
        }
        assert!(stale.is_empty(), "Veraltete Kennungen in tutorial.json:\n  {}", stale.join("\n  "));
    }

    /// Der Abgleich selbst muss greifen (sonst wäre der Test oben wertlos).
    #[test]
    fn abgleich_findet_funktionen() {
        let f = editor_features();
        for k in ["tool:Bild", "tool:Haftnotiz", "action:undo", "action:help", "action:pan", "action:snap", "action:tool.pen", "menu:Als PDF …", "menu:Board: dunkel", "menu:Anleitung & Tipps", "setting:Taste", "setting:Winkelraster"] {
            assert!(f.contains(k), "{k} nicht erkannt");
        }
        assert!(f.len() > 50, "nur {} Funktionen erkannt", f.len());
    }

    /// Ein Eintrag mit `action` zeigt die aktuelle Belegung, und jede `action` gibt es.
    #[test]
    fn eintraege_zeigen_belegung() {
        for e in data().categories.iter().flat_map(|c| c.entries.iter()) {
            for id in &e.action {
                assert!(keys::find(id).is_some(), "{}: unbekannte Aktion {id}", e.id);
            }
        }
        let e = data().categories.iter().flat_map(|c| c.entries.iter()).find(|e| e.action.contains(&"undo".to_string())).expect("Eintrag für undo");
        let mut s = Settings::default();
        assert!(key_caps(e, &s).iter().any(|k| k.starts_with("Strg+Z")));
        keys::assign(&mut s, "undo", Some(0), keys::Chord::parse("Ctrl+U").unwrap());
        let caps = key_caps(e, &s);
        assert!(caps.iter().any(|k| k.starts_with("Strg+U")) && !caps.iter().any(|k| k.starts_with("Strg+Z ") || k == "Strg+Z"), "{caps:?}");
    }

    #[test]
    fn suche() {
        let d = data();
        let cat = &d.categories[0];
        let e = &cat.entries[0];
        assert!(matches(cat, e, ""));
        assert!(matches(cat, e, &e.title.to_uppercase()));
        assert!(!matches(cat, e, "xyzzy-gibt-es-nicht"));
        assert_eq!(norm("Über Größe"), "uber grosse");
    }

    #[test]
    fn literale() {
        let v = literals_after("menu_row(ui, if a { \"A\" } else { \"B\" }, false, h) menu_row(ui, \"C …\", false, h)", "menu_row(ui, ");
        assert_eq!(v, vec!["A", "B", "C …"]);
    }
}
