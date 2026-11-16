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
    /// Icons (Suche, Zurück …) der Seite.
    icons: style::Icons,
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

/// Maximale Textbreite der Einträge.
const BODY_W: f32 = 720.0;
/// Breite der Themenliste links (PC).
const NAV_W: f32 = 232.0;

/// Zeichnet die Anleitung; liefert true, wenn „Zurück“ gewählt wurde.
/// Breit (PC/Tablet quer): Themenliste links, Einträge rechts. Schmal (Handy): Suche und Themen-Chips oben.
pub fn show(ui: &mut Ui, env: &Env, st: &mut TutorialState, settings: &Settings) -> bool {
    use crate::ui::start::{icon_label_btn, BtnKind};
    use style::sp;
    let compact = env.ff == FormFactor::Compact;
    let touch_h = if compact || env.touch { env.btn.max(crate::layout::min_touch(env.touch)) } else { 36.0 };
    let side = if compact { sp::L } else { sp::XL };
    let full = ui.available_rect_before_wrap();
    let wide = full.width() >= 860.0;
    // Gesamtbreite der Inhaltsspalten, mittig
    let avail = (full.width() - 2.0 * side).max(100.0);
    let body_w = if wide { (avail - NAV_W - sp::XXL).clamp(300.0, BODY_W) } else { avail.min(BODY_W) };
    let total = if wide { NAV_W + sp::XXL + body_w } else { body_w };
    let x0 = full.center().x - total / 2.0;
    let mut back = false;

    // Kopf: Zurück + Titel
    let head = egui::Rect::from_min_size(egui::pos2(x0, full.top() + sp::M), Vec2::new(total, touch_h));
    ui.scope_builder(egui::UiBuilder::new().max_rect(head).layout(egui::Layout::left_to_right(egui::Align::Center)), |ui| {
        ui.spacing_mut().item_spacing.x = sp::S;
        let label = if compact { "" } else { "Zurück" };
        back = icon_label_btn(ui, &mut st.icons, owndraw_core::icons::BACK, label, "Zurück zur vorherigen Seite", if style::modern() { BtnKind::Outline } else { BtnKind::Ghost }, touch_h).clicked();
        ui.add_space(sp::XS);
        ui.label(style::title("Anleitung & Tipps"));
    });
    if ui.input(|i| i.key_pressed(egui::Key::Escape)) && !ui.ctx().egui_wants_keyboard_input() {
        back = true;
    }
    let top = full.top() + sp::M + touch_h + sp::L;
    let bottom = full.bottom();
    let d = data();

    if wide {
        // Themenliste links
        let nav_r = egui::Rect::from_min_max(egui::pos2(x0, top), egui::pos2(x0 + NAV_W, bottom));
        ui.scope_builder(egui::UiBuilder::new().max_rect(nav_r).layout(egui::Layout::top_down(egui::Align::Min)), |ui| {
            egui::ScrollArea::vertical().id_salt("tut-nav").auto_shrink([false, false]).show(ui, |ui| {
                ui.set_width(NAV_W);
                nav(ui, d, st);
                ui.add_space(sp::XL);
            });
        });
        // Suche (fest) und Einträge (scrollen)
        let bx = x0 + NAV_W + sp::XXL;
        let search_r = egui::Rect::from_min_size(egui::pos2(bx, top), Vec2::new(body_w, touch_h));
        ui.scope_builder(egui::UiBuilder::new().max_rect(search_r), |ui| {
            search_field(ui, st, body_w, touch_h);
        });
        let scroll_r = egui::Rect::from_min_max(egui::pos2(bx, top + touch_h + sp::L), egui::pos2(full.right(), bottom));
        ui.scope_builder(egui::UiBuilder::new().max_rect(scroll_r), |ui| {
            scroll_content(ui, env, st, settings, body_w, true);
        });
    } else {
        // Schmal: Suche, Chips, darunter die Einträge
        let col = egui::Rect::from_min_max(egui::pos2(x0, top), egui::pos2(x0 + body_w, bottom));
        ui.scope_builder(egui::UiBuilder::new().max_rect(col).layout(egui::Layout::top_down(egui::Align::Min)), |ui| {
            ui.set_width(body_w);
            search_field(ui, st, body_w, touch_h);
            ui.add_space(sp::S);
            egui::ScrollArea::horizontal().id_salt("tut-chips").auto_shrink([false, true]).scroll_bar_visibility(egui::scroll_area::ScrollBarVisibility::AlwaysHidden).show(ui, |ui| {
                ui.horizontal(|ui| {
                    ui.spacing_mut().item_spacing.x = sp::S;
                    if style::chip(ui, "Alle", st.cat.is_none(), touch_h).clicked() {
                        st.cat = None;
                    }
                    for (i, cat) in d.categories.iter().enumerate() {
                        if style::chip(ui, &cat.title, st.cat == Some(i), touch_h).clicked() {
                            st.cat = if st.cat == Some(i) { None } else { Some(i) };
                        }
                    }
                });
            });
            ui.add_space(sp::S);
            let r = ui.available_rect_before_wrap();
            let scroll_r = egui::Rect::from_min_max(egui::pos2(full.left(), r.top()), egui::pos2(full.right(), bottom));
            ui.scope_builder(egui::UiBuilder::new().max_rect(scroll_r), |ui| {
                // Spalte im Scrollbereich wieder mittig (der Scrollbalken sitzt am Fensterrand)
                scroll_content(ui, env, st, settings, body_w, false);
            });
        });
    }
    back
}

/// Suchfeld mit Lupe (und Leeren-Knopf).
fn search_field(ui: &mut Ui, st: &mut TutorialState, w: f32, h: f32) {
    let p = style::pal();
    let te = egui::TextEdit::singleline(&mut st.query)
        .hint_text(if w > 420.0 { "Anleitung durchsuchen (z. B. Pfeil, Zoom, Strg)" } else { "Anleitung durchsuchen …" })
        .desired_width(w)
        .min_size(Vec2::new(w, h))
        .align(egui::Align2::LEFT_CENTER)
        .margin(Margin { left: 36, right: 36, top: 4, bottom: 4 });
    let resp = ui.add(te);
    let r = resp.rect;
    let rad = CornerRadius::same(style::btn_r());
    let focus = resp.has_focus();
    ui.painter().rect_stroke(r, rad, Stroke::new(if focus { 1.5 } else { 1.0 }, c(if focus { p.accent } else { p.line })), egui::StrokeKind::Inside);
    st.icons.paint(ui, egui::Rect::from_center_size(egui::pos2(r.left() + 18.0, r.center().y), Vec2::splat(24.0)), owndraw_core::icons::SEARCH, p.dim, false, 18.0);
    if !st.query.is_empty() {
        let xr = egui::Rect::from_center_size(egui::pos2(r.right() - 18.0, r.center().y), Vec2::splat(28.0));
        let xresp = ui.interact(xr, resp.id.with("clear"), egui::Sense::click()).on_hover_text("Suche leeren");
        if xresp.hovered() {
            ui.painter().rect_filled(xr, CornerRadius::same(6), c(p.hover));
        }
        st.icons.paint(ui, xr, owndraw_core::icons::CLOSE, p.dim, false, 14.0);
        if xresp.clicked() {
            st.query.clear();
            resp.request_focus();
        }
    }
}

/// Themenliste (PC): „Alle Themen“ und jede Kategorie mit Trefferzahl.
fn nav(ui: &mut Ui, d: &Data, st: &mut TutorialState) {
    use style::{fs, sp};
    let p = style::pal();
    ui.spacing_mut().item_spacing.y = 2.0;
    ui.label(egui::RichText::new("THEMEN").size(fs::CAPTION).strong().color(c(p.dim)));
    ui.add_space(sp::XS);
    let searching = !st.query.trim().is_empty();
    let count = |cat: &Category| cat.entries.iter().filter(|e| matches(cat, e, &st.query)).count();
    let total: usize = d.categories.iter().map(count).sum();
    let mut pick: Option<Option<usize>> = None;
    #[allow(unused_mut)]
    let mut row = |ui: &mut Ui, label: &str, n: usize, sel: bool| -> bool {
        let (r, resp) = ui.allocate_exact_size(Vec2::new(NAV_W, 36.0), egui::Sense::click());
        let empty = searching && n == 0;
        let pt = ui.painter();
        let rr = CornerRadius::same(8);
        if sel {
            pt.rect_filled(r, rr, c(p.active));
        } else if resp.hovered() {
            pt.rect_filled(r, rr, c(p.hover));
        }
        let col = if sel { p.accent } else if empty { p.disabled } else { p.text };
        let font = egui::FontId::proportional(fs::BODY);
        let num = n.to_string();
        let ng = pt.layout_no_wrap(num, egui::FontId::proportional(fs::CAPTION), c(if sel { p.accent } else { p.dim }));
        let tw = NAV_W - 2.0 * sp::M - ng.size().x - sp::S;
        let t = super::elide(label, fs::BODY, tw, ui);
        let pt = ui.painter();
        pt.text(egui::pos2(r.left() + sp::M, r.center().y), egui::Align2::LEFT_CENTER, t, font, c(col));
        pt.galley(egui::pos2(r.right() - sp::M - ng.size().x, r.center().y - ng.size().y / 2.0), ng, c(p.dim));
        resp.on_hover_cursor(egui::CursorIcon::PointingHand).clicked()
    };
    if row(ui, "Alle Themen", total, st.cat.is_none()) {
        pick = Some(None);
    }
    for (i, cat) in d.categories.iter().enumerate() {
        if row(ui, &cat.title, count(cat), st.cat == Some(i)) {
            pick = Some(Some(i));
        }
    }
    if let Some(c) = pick {
        st.cat = c;
    }
}

/// Scrollbereich mit den Einträgen; der Text steht in einer Spalte von höchstens `body_w`.
/// `ui` reicht bis zum rechten Fensterrand (Scrollbalken dort); die Spalte beginnt am linken Rand von `ui`
/// (`left`, breite Ansicht) oder steht mittig (schmale Ansicht).
fn scroll_content(ui: &mut Ui, env: &Env, st: &mut TutorialState, settings: &Settings, body_w: f32, left: bool) {
    egui::ScrollArea::vertical().id_salt(("tut-body", st.cat)).auto_shrink([false, false]).show(ui, |ui| {
        let r = ui.available_rect_before_wrap();
        let x = if left { r.left() } else { r.center().x - body_w.min(r.width()) / 2.0 };
        let col = egui::Rect::from_min_size(egui::pos2(x, r.top()), Vec2::new(body_w.min(r.width()), f32::INFINITY));
        ui.scope_builder(egui::UiBuilder::new().max_rect(col).layout(egui::Layout::top_down(egui::Align::Min)), |ui| {
            ui.set_max_width(body_w);
            content(ui, env, st, settings);
            ui.add_space(style::sp::XXL);
        });
    });
}

fn content(ui: &mut Ui, _env: &Env, st: &mut TutorialState, settings: &Settings) {
    use style::{fs, sp};
    let p = style::pal();
    let d = data();
    let mut shown = 0;
    let searching = !st.query.trim().is_empty();
    if !searching && st.cat.is_none() {
        ui.add(egui::Label::new(style::hint("Alle Funktionen des Editors auf einen Blick – mit Tastenkürzeln, Gesten und Tipps.")).wrap());
    }
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
        if !cat.intro.is_empty() && !searching {
            ui.add(egui::Label::new(egui::RichText::new(&cat.intro).size(fs::BODY).color(c(p.dim))).wrap());
            ui.add_space(sp::XS);
        }
        for e in hits {
            ui.add_space(sp::S);
            entry(ui, e, settings);
        }
    }
    if shown == 0 {
        ui.add_space(sp::XXL);
        ui.vertical_centered(|ui| {
            let (r, _) = ui.allocate_exact_size(Vec2::splat(64.0), egui::Sense::hover());
            ui.painter().circle_filled(r.center(), 32.0, c(p.accent_soft));
            st.icons.paint(ui, r, owndraw_core::icons::SEARCH, p.accent, false, 28.0);
            ui.add_space(sp::M);
            ui.label(egui::RichText::new("Nichts gefunden").size(fs::SECTION + 3.0).strong().color(c(p.text)));
            ui.label(style::hint("Versuche ein anderes Stichwort oder wähle „Alle Themen“."));
            ui.add_space(sp::M);
            if style::outline_btn(ui, "Suche zurücksetzen", 36.0).clicked() {
                st.query.clear();
                st.cat = None;
            }
        });
    }
}

fn heading(ui: &mut Ui, title: &str) {
    use style::{fs, sp};
    let p = style::pal();
    if style::modern() {
        ui.add_space(sp::XL);
        ui.label(egui::RichText::new(title).size(fs::SECTION + 4.0).strong().color(c(p.text)));
    } else {
        ui.add_space(sp::L);
        ui.label(egui::RichText::new(title.to_uppercase()).size(13.0).strong().color(c(p.dim)));
    }
    ui.add_space(sp::XS);
}

fn entry(ui: &mut Ui, e: &Entry, settings: &Settings) {
    use style::{fs, sp};
    let p = style::pal();
    style::card().show(ui, |ui| {
        ui.set_width(ui.available_width());
        ui.spacing_mut().item_spacing.y = sp::S;
        ui.horizontal_wrapped(|ui| {
            ui.spacing_mut().item_spacing = Vec2::new(sp::S, sp::XS);
            ui.label(egui::RichText::new(&e.title).size(fs::SECTION + 1.0).strong().color(c(p.text)));
            if !e.only.is_empty() {
                style::badge(ui, &e.only);
            }
        });
        ui.add(egui::Label::new(egui::RichText::new(&e.text).size(fs::BODY).line_height(Some(21.0)).color(c(p.text))).wrap());
        if !e.how.is_empty() {
            ui.spacing_mut().item_spacing.y = sp::XS + 1.0;
            for h in &e.how {
                ui.horizontal_top(|ui| {
                    ui.spacing_mut().item_spacing.x = sp::S;
                    ui.label(egui::RichText::new("•").color(c(p.accent)).strong());
                    ui.add(egui::Label::new(egui::RichText::new(h).size(fs::BODY - 0.5).line_height(Some(20.0)).color(c(p.dim))).wrap());
                });
            }
        }
        let caps = key_caps(e, settings);
        if !caps.is_empty() {
            ui.add_space(sp::XS);
            ui.horizontal_wrapped(|ui| {
                ui.spacing_mut().item_spacing = Vec2::new(sp::S + 2.0, sp::S);
                for k in &caps {
                    key_combo(ui, k);
                }
            });
        }
    });
}

/// Tastenkürzel als Kappen: „Strg+Z Rückgängig“ → [Strg] + [Z] und dahinter der Zusatztext.
fn key_combo(ui: &mut Ui, text: &str) {
    let p = style::pal();
    let (combo, rest) = text.split_once(' ').unwrap_or((text, ""));
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = 3.0;
        for (i, part) in combo.split('+').enumerate() {
            if i > 0 {
                ui.label(egui::RichText::new("+").size(style::fs::SMALL).color(c(p.dim)));
            }
            if !part.is_empty() {
                key_cap(ui, part);
            }
        }
        if !rest.is_empty() {
            ui.add_space(3.0);
            ui.label(egui::RichText::new(rest).size(style::fs::SMALL).color(c(p.dim)));
        }
    });
}

/// Taste als „Tastenkappe“ (mit dickerem unteren Rand wie eine echte Taste).
fn key_cap(ui: &mut Ui, text: &str) {
    let p = style::pal();
    let g = ui.painter().layout_no_wrap(text.to_string(), egui::FontId::proportional(style::fs::SMALL), c(p.text));
    let size = Vec2::new((g.size().x + 14.0).max(26.0), g.size().y + 8.0);
    let (r, _) = ui.allocate_exact_size(size + Vec2::new(0.0, 2.0), egui::Sense::hover());
    let cap = egui::Rect::from_min_size(r.min, size);
    let rad = CornerRadius::same(6);
    let pt = ui.painter();
    pt.rect_filled(cap.translate(Vec2::new(0.0, 2.0)), rad, c(p.line));
    pt.rect_filled(cap, rad, c(if p.dark { p.pill } else { p.card }));
    pt.rect_stroke(cap, rad, Stroke::new(1.0, c(p.line)), egui::StrokeKind::Inside);
    pt.galley(cap.center() - g.size() / 2.0, g, c(p.text));
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
        // Zeilen haben die Form `menu_row_ic(ui, &mut sh.icons, ic::X, "Text", …)`: Icon-Argument überspringen
        let mut rows = String::new();
        for part in include_str!("editor/menu.rs").split("menu_row_ic(ui, &mut sh.icons, ").skip(1) {
            if let Some(i) = part.find(", ") {
                rows.push_str("menu_row(ui, ");
                rows.push_str(&part[i + 2..]);
            }
        }
        for l in literals_after(&rows, "menu_row(ui, ") {
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
