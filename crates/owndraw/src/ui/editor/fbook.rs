//! Fenster „Formelsammlung“ (Werkzeug „Rechnen“, Menü „…“): Sammlungen durchsuchen, Formeln einfügen,
//! eigene Formeln/Konstanten/Einheiten anlegen, Export/Import, Teilen per Code, Sammlung abonnieren.
//! Vorschläge passender Formeln im Rechnen-Feld: `suggestions`.

use super::Editor;
use crate::app::{PickPurpose, Req, Shared};
use crate::formulas::{self, Src};
use crate::layout::min_touch;
use crate::platform::FileKind;
use crate::style::{self, c, fs, sp};
use egui::{Align2, Order, TextureHandle, Ui, Vec2};
use owndraw_core::calc::{self, library::{self, Collection, Formula, Named}};
use owndraw_core::canvas::Canvas;
use owndraw_core::math;
use std::collections::HashMap;
use owndraw_core::tiny_skia::Pixmap;

#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub enum Tab {
    #[default]
    Formulas,
    Consts,
    Units,
    Share,
}

/// Formel/Konstante im Bearbeiten-Formular.
#[derive(Clone, Debug)]
pub enum Edit {
    Formula { set: usize, idx: Option<usize>, f: Formula },
    Named { set: usize, unit: bool, idx: Option<usize>, n: Named },
    NewSet { name: String },
    Rename { set: usize, name: String },
}

#[derive(Default)]
pub struct FBook {
    pub open: bool,
    pub set: usize,
    pub tab: Tab,
    pub query: String,
    pub edit: Option<Edit>,
    pub code: String,
    /// Formel aufgeklappt (Größen/Beschreibung).
    pub expanded: Option<(usize, usize)>,
    confirm_delete: Option<usize>,
    tex: HashMap<(String, u32), (TextureHandle, Vec2)>,
}

impl FBook {
    /// Formel als Bild (gecacht je LaTeX und Farbe).
    fn formula_img(&mut self, ui: &Ui, latex: &str, size: f32, fg: u32) -> Option<(egui::TextureId, Vec2)> {
        let key = (latex.to_string(), fg);
        if !self.tex.contains_key(&key) {
            if self.tex.len() > 400 {
                self.tex.clear();
            }
            let ppp = ui.ctx().pixels_per_point();
            let f = math::layout(latex, size);
            let (pw, ph) = ((f.w * ppp).ceil().max(1.0) as u32, (f.h * ppp).ceil().max(1.0) as u32);
            let mut pm = Pixmap::new(pw.min(2048), ph.min(1024))?;
            {
                let mut cv = Canvas::new(&mut pm);
                cv.scale(ppp, ppp);
                f.draw(&mut cv, fg);
            }
            let img = egui::ColorImage::from_rgba_premultiplied([pm.width() as usize, pm.height() as usize], pm.data());
            let t = ui.ctx().load_texture("fbook-formula", img, egui::TextureOptions::LINEAR);
            self.tex.insert(key.clone(), (t, Vec2::new(pm.width() as f32 / ppp, pm.height() as f32 / ppp)));
        }
        self.tex.get(&key).map(|(t, s)| (t.id(), *s))
    }

    fn show_formula(&mut self, ui: &mut Ui, plain: &str, size: f32) {
        let fg = style::pal().text;
        match self.formula_img(ui, &calc::to_latex(plain), size, fg) {
            Some((id, s)) => {
                let w = ui.available_width().max(40.0);
                let k = (w / s.x).min(1.0);
                ui.add(egui::Image::new((id, s * k)));
            }
            None => {
                ui.label(plain);
            }
        }
    }
}

fn file_name(name: &str) -> String {
    format!("{}.{}", crate::app::safe_file_name(if name.trim().is_empty() { "Formelsammlung" } else { name.trim() }), library::EXT)
}

impl Editor {
    /// Formel als Rechenzeile einfügen: ins offene Rechnen-Feld, sonst als neue Zeile in der Mitte der Ansicht.
    pub(super) fn insert_formula(&mut self, sh: &mut Shared, expr: &str) {
        if let Some(ce) = self.calc.as_mut() {
            ce.src = expr.to_string();
            ce.focus_now();
            return;
        }
        let b = &self.board;
        let (wx, wy) = ((self.canvas.center().x - self.canvas.min.x - b.tx) / b.sc, (self.canvas.center().y - self.canvas.min.y - b.ty) / b.sc);
        let size = self.board.math_size();
        if self.board.add_math(wx - size * 4.0, wy, size, &calc::to_latex(expr)).is_some() {
            sh.toast("Formel eingefügt – mit „Rechnen“ antippen zum Bearbeiten");
        }
    }

    /// Abnahme-Demo: Fenster öffnen (`konst`, `teilen`, `neu` = Formular einer eigenen Formel, sonst Formeln).
    pub fn demo_fbook(&mut self, what: &str) {
        self.fbook.open = true;
        match what {
            "konst" => self.fbook.tab = Tab::Consts,
            "teilen" => self.fbook.tab = Tab::Share,
            "neu" => {
                let mut f = Formula { name: "Leistung Drehstrommotor".into(), expr: "P = sqrt(3)·U·I·cos(φ)·η".into(), topic: "Antriebe".into(), ..Default::default() };
                f.sync_vars();
                for (s, u, d) in [("P", "W", "abgegebene Leistung"), ("U", "V", "Außenleiterspannung"), ("I", "A", "Außenleiterstrom"), ("φ", "°", "Phasenwinkel"), ("η", "1", "Wirkungsgrad")] {
                    if let Some(v) = f.vars.iter_mut().find(|v| v.sym == s) {
                        v.unit = u.into();
                        v.desc = d.into();
                    }
                }
                self.fbook.edit = Some(Edit::Formula { set: 0, idx: None, f });
            }
            _ => self.fbook.expanded = Some((0, 13)),
        }
    }

    /// Formelsammlung öffnen (optional mit Suchbegriff).
    pub(super) fn open_fbook(&mut self, query: Option<&str>) {
        self.fbook.open = true;
        if let Some(q) = query {
            self.fbook.query = q.to_string();
            self.fbook.tab = Tab::Formulas;
        }
    }

    /// Export-/Import-Dialog beendet.
    pub(super) fn apply_fbook_pick(&mut self, sh: &mut Shared, purpose: PickPurpose, f: &crate::platform::FileRef) {
        let dir = sh.platform.config_dir();
        match purpose {
            PickPurpose::FormulasExport(i) => {
                let Some(col) = formulas::with(|l| l.sets.get(i).map(|s| s.col.clone())) else { return };
                match sh.platform.write_atomic(f, col.to_json().as_bytes()) {
                    Ok(()) => sh.toast("Formelsammlung exportiert"),
                    Err(e) => sh.toast(&format!("Speichern fehlgeschlagen: {e}")),
                }
            }
            PickPurpose::FormulasImport => match sh.platform.read(f).map_err(|e| e.to_string()).and_then(|b| Collection::from_json(&String::from_utf8_lossy(&b))) {
                Ok(mut col) => {
                    for x in col.formulas.iter_mut() {
                        x.sync_vars();
                    }
                    let n = col.formulas.len();
                    let name = col.name.clone();
                    if let Some(i) = formulas::create(col, &dir) {
                        self.fbook.set = i;
                    }
                    sh.toast(&format!("„{name}“ importiert ({n} Formeln)"));
                }
                Err(e) => sh.toast(&format!("Import fehlgeschlagen: {e}")),
            },
            _ => {}
        }
    }
}

/// Vorschläge für das Rechnen-Feld: (Formel, Hinweis) passend zur gefragten bzw. getippten Größe.
pub(super) fn suggest_list(ed: &Editor, sh: &Shared, at: (f32, f32), typed: &str) -> Vec<(String, String)> {
    let cols = formulas::active(&sh.settings);
    if cols.is_empty() {
        return Vec::new();
    }
    let (known, asked) = ed.board.calc_names_at(at.0, at.1);
    let refs: Vec<&Collection> = cols.iter().collect();
    library::suggest(&refs, &known, &asked, typed, 3)
        .into_iter()
        .map(|h| {
            let f = &refs[h.col].formulas[h.idx];
            let tip = if h.missing.is_empty() { format!("{} – alle Größen bekannt", f.name) } else { format!("{} – fehlt noch: {}", f.name, h.missing.join(", ")) };
            (f.expr.clone(), tip)
        })
        .collect()
}

pub(super) fn show(ed: &mut Editor, ctx: &egui::Context, sh: &mut Shared) {
    if !ed.fbook.open {
        return;
    }
    let canvas = ed.canvas;
    let w = (canvas.width() - 24.0).clamp(280.0, 460.0);
    let max_h = (canvas.height() - 90.0).max(240.0);
    let mut close = false;
    let r = Editor::float(ctx, "fbook", canvas.right_top() + Vec2::new(-12.0, 64.0), Align2::RIGHT_TOP, canvas.shrink(8.0), Order::Foreground, |ui| {
        style::card().show(ui, |ui| {
            ui.set_width(w);
            ui.set_max_height(max_h);
            ui.horizontal(|ui| {
                ui.label(egui::RichText::new("Formelsammlung").size(fs::SECTION).strong());
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if style::icon_btn(ui, &mut sh.icons, owndraw_core::icons::CLOSE, style::pal().dim, false, false, 28.0, "Schließen").clicked() {
                        close = true;
                    }
                });
            });
            if ed.fbook.edit.is_some() {
                edit_form(ui, ed, sh);
                return;
            }
            body(ui, ed, sh, max_h - 60.0);
        });
    });
    ed.st.popup_rects.push(r);
    if close || ctx.input_mut(|i| super::keys::consume(i, &sh.settings, "cancel")) && ed.fbook.edit.is_none() && ed.calc.is_none() {
        ed.fbook.open = false;
    }
}

fn body(ui: &mut Ui, ed: &mut Editor, sh: &mut Shared, max_h: f32) {
    let pal = style::pal();
    let h = min_touch(sh.env.touch);
    let dir = sh.platform.config_dir();
    let (sets, busy, msg) = formulas::with(|l| (l.sets.clone(), l.busy, l.msg.clone()));
    if sets.is_empty() {
        ui.label(style::hint("Wird geladen …"));
        return;
    }
    if ed.fbook.set >= sets.len() {
        ed.fbook.set = 0;
    }
    // Sammlung wählen
    ui.horizontal_wrapped(|ui| {
        ui.spacing_mut().item_spacing = Vec2::new(4.0, 4.0);
        for (i, s) in sets.iter().enumerate() {
            let on = formulas::is_active(&sh.settings, &s.key());
            let label = if on { s.col.name.clone() } else { format!("{} (aus)", s.col.name) };
            if style::chip(ui, &label, ed.fbook.set == i, h * 0.9).on_hover_text(s.origin()).clicked() {
                ed.fbook.set = i;
                ed.fbook.expanded = None;
                ed.fbook.confirm_delete = None;
            }
        }
        if style::chip(ui, "+ Neu", false, h * 0.9).on_hover_text("Eigene Sammlung anlegen").clicked() {
            ed.fbook.edit = Some(Edit::NewSet { name: String::new() });
        }
    });
    let si = ed.fbook.set;
    let set = &sets[si];
    let key = set.key();
    ui.horizontal_wrapped(|ui| {
        ui.label(egui::RichText::new(set.origin()).size(12.0).color(c(pal.dim)));
        let mut on = formulas::is_active(&sh.settings, &key);
        if ui.checkbox(&mut on, "Konstanten/Einheiten verwenden").on_hover_text("Konstanten und eigene Einheiten dieser Sammlung gelten beim Rechnen auf allen Seiten.").changed() {
            formulas::set_active(&mut sh.settings, &key, on);
        }
    });
    if let Some((m, err)) = &msg {
        ui.label(egui::RichText::new(m).size(12.0).color(c(if *err { pal.danger_text } else { pal.dim })));
    }
    if busy {
        ui.label(style::hint("Server …"));
    }
    ui.horizontal_wrapped(|ui| {
        ui.spacing_mut().item_spacing = Vec2::new(4.0, 4.0);
        for (t, l) in [(Tab::Formulas, "Formeln"), (Tab::Consts, "Konstanten"), (Tab::Units, "Einheiten"), (Tab::Share, "Teilen & Datei")] {
            if style::chip(ui, l, ed.fbook.tab == t, h * 0.85).clicked() {
                ed.fbook.tab = t;
            }
        }
    });
    style::separator(ui);
    egui::ScrollArea::vertical().max_height((max_h - 150.0).max(160.0)).auto_shrink([false, true]).show(ui, |ui| match ed.fbook.tab {
        Tab::Formulas => formula_list(ui, ed, sh, &sets, si),
        Tab::Consts => named_list(ui, ed, sh, &sets, si, false),
        Tab::Units => named_list(ui, ed, sh, &sets, si, true),
        Tab::Share => share_tab(ui, ed, sh, &sets, si, &dir),
    });
}

fn formula_list(ui: &mut Ui, ed: &mut Editor, sh: &mut Shared, sets: &[formulas::Set], si: usize) {
    let pal = style::pal();
    let h = min_touch(sh.env.touch);
    let set = &sets[si];
    ui.add(egui::TextEdit::singleline(&mut ed.fbook.query).hint_text("Suchen (Name, Größe, Thema) …").desired_width(f32::INFINITY).margin(Vec2::new(8.0, 6.0)));
    if set.editable() && style::outline_btn(ui, "+ Formel", h * 0.85).clicked() {
        ed.fbook.edit = Some(Edit::Formula { set: si, idx: None, f: Formula { topic: set.col.formulas.last().map(|f| f.topic.clone()).unwrap_or_default(), ..Default::default() } });
    }
    let hits = library::search(&[&set.col], &ed.fbook.query);
    if hits.is_empty() {
        ui.label(style::hint(if set.col.formulas.is_empty() { "Noch keine Formeln. Lege mit „+ Formel“ eine an." } else { "Keine Treffer." }));
    }
    let mut order: Vec<usize> = hits.iter().map(|h| h.idx).collect();
    if ed.fbook.query.trim().is_empty() {
        order.sort();
    }
    let mut topic = None;
    let mut insert = None;
    let mut del = None;
    for fi in order {
        let f = &set.col.formulas[fi];
        if ed.fbook.query.trim().is_empty() && topic != Some(&f.topic) {
            topic = Some(&f.topic);
            if !f.topic.is_empty() {
                style::caption(ui, &f.topic);
            }
        }
        let open = ed.fbook.expanded == Some((si, fi));
        egui::Frame::new().fill(c(if open { pal.sunken } else { 0 })).corner_radius(8).inner_margin(6).show(ui, |ui| {
            ui.set_width(ui.available_width());
            let r = ui.horizontal(|ui| {
                ui.vertical(|ui| {
                    ui.set_width((ui.available_width() - 90.0).max(120.0));
                    ui.label(egui::RichText::new(&f.name).strong());
                    ed.fbook.show_formula(ui, &f.expr, 18.0);
                });
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if style::accent_btn(ui, "Einfügen", h * 0.85).on_hover_text("Als Rechenzeile aufs Blatt").clicked() {
                        insert = Some(f.expr.clone());
                    }
                });
            });
            if r.response.interact(egui::Sense::click()).clicked() {
                ed.fbook.expanded = if open { None } else { Some((si, fi)) };
            }
            if open {
                if !f.desc.is_empty() {
                    ui.label(egui::RichText::new(&f.desc).size(12.0));
                }
                for v in &f.vars {
                    ui.label(egui::RichText::new(format!("{}  {}{}", v.sym, v.desc, if v.unit.is_empty() || v.unit == "1" { String::new() } else { format!(" in {}", v.unit) })).size(12.0).color(c(pal.dim)));
                }
                if set.editable() {
                    ui.horizontal(|ui| {
                        if style::plain_btn(ui, "Bearbeiten", h * 0.8).clicked() {
                            ed.fbook.edit = Some(Edit::Formula { set: si, idx: Some(fi), f: f.clone() });
                        }
                        if style::danger_btn(ui, "Löschen", h * 0.8).clicked() {
                            del = Some(fi);
                        }
                    });
                }
            } else if r.response.hovered() {
                ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
            }
        });
    }
    if let Some(e) = insert {
        ed.insert_formula(sh, &e);
    }
    if let Some(fi) = del {
        formulas::with(|l| {
            if let Some(s) = l.sets.get_mut(si) {
                s.col.formulas.remove(fi);
            }
        });
        ed.fbook.expanded = None;
        formulas::save(si, &sh.platform.config_dir());
    }
}

fn named_list(ui: &mut Ui, ed: &mut Editor, sh: &mut Shared, sets: &[formulas::Set], si: usize, unit: bool) {
    let pal = style::pal();
    let h = min_touch(sh.env.touch);
    let set = &sets[si];
    let list = if unit { &set.col.units } else { &set.col.consts };
    ui.label(style::hint(if unit {
        "Eigene Einheiten, z. B. kp = 9,80665 N. Sie gelten beim Rechnen, wenn die Sammlung eingeschaltet ist."
    } else {
        "Konstanten, z. B. ρ_Cu = 0,0178 Ω·mm²/m. Sie gelten auf allen Seiten, eigene Zeilen auf dem Blatt haben Vorrang."
    }));
    if set.editable() && style::outline_btn(ui, if unit { "+ Einheit" } else { "+ Konstante" }, h * 0.85).clicked() {
        ed.fbook.edit = Some(Edit::Named { set: si, unit, idx: None, n: Named::default() });
    }
    if list.is_empty() {
        ui.label(style::hint("Keine Einträge."));
    }
    let mut insert = None;
    for (i, n) in list.iter().enumerate() {
        ui.horizontal(|ui| {
            ui.vertical(|ui| {
                ui.set_width((ui.available_width() - 100.0).max(120.0));
                ed.fbook.show_formula(ui, &format!("{} = {}", n.sym, n.value), 16.0);
                if !n.desc.is_empty() {
                    ui.label(egui::RichText::new(&n.desc).size(12.0).color(c(pal.dim)));
                }
            });
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if set.editable() && style::plain_btn(ui, "Ändern", h * 0.8).clicked() {
                    ed.fbook.edit = Some(Edit::Named { set: si, unit, idx: Some(i), n: n.clone() });
                }
                if !unit && style::plain_btn(ui, "Einfügen", h * 0.8).clicked() {
                    insert = Some(format!("{}: {}", n.sym, n.value));
                }
            });
        });
    }
    if let Some(e) = insert {
        ed.insert_formula(sh, &e);
    }
}

fn share_tab(ui: &mut Ui, ed: &mut Editor, sh: &mut Shared, sets: &[formulas::Set], si: usize, dir: &std::path::Path) {
    let pal = style::pal();
    let h = min_touch(sh.env.touch);
    let set = &sets[si];
    let account = formulas::account();
    style::caption(ui, "Datei");
    ui.horizontal_wrapped(|ui| {
        if style::outline_btn(ui, "Exportieren …", h * 0.85).on_hover_text("Sammlung als Datei speichern").clicked() {
            sh.reqs.push(Req::PickSave { purpose: PickPurpose::FormulasExport(si), name: file_name(&set.col.name), kind: FileKind::Any });
        }
        if style::outline_btn(ui, "Importieren …", h * 0.85).on_hover_text("Sammlung aus Datei als neue eigene Sammlung").clicked() {
            sh.reqs.push(Req::PickOpen { purpose: PickPurpose::FormulasImport, kinds: vec![FileKind::Any] });
        }
    });
    if set.editable() {
        style::caption(ui, "Sammlung");
        ui.horizontal_wrapped(|ui| {
            if style::plain_btn(ui, "Umbenennen", h * 0.85).clicked() {
                ed.fbook.edit = Some(Edit::Rename { set: si, name: set.col.name.clone() });
            }
            if ed.fbook.confirm_delete == Some(si) {
                if style::danger_btn(ui, "Wirklich löschen", h * 0.85).clicked() {
                    formulas::delete(si, dir);
                    ed.fbook.set = 0;
                    ed.fbook.confirm_delete = None;
                }
            } else if style::danger_btn(ui, "Löschen", h * 0.85).clicked() {
                ed.fbook.confirm_delete = Some(si);
            }
        });
    } else if matches!(set.src, Src::Server { mine: false, global: false, .. }) && style::plain_btn(ui, "Nicht mehr verwenden", h * 0.85).on_hover_text("Abo der geteilten Sammlung beenden").clicked() {
        formulas::delete(si, dir);
        ed.fbook.set = 0;
    }
    style::caption(ui, "Teilen");
    match &set.src {
        Src::Local if account => {
            ui.label(style::hint("Diese Sammlung liegt nur auf diesem Gerät. Im Konto ist sie auf allen Geräten da und kann geteilt werden."));
            if style::accent_btn(ui, "Ins Konto hochladen", h * 0.85).clicked() {
                formulas::upload(set.col.clone(), Some(si), dir);
            }
        }
        Src::Local | Src::Builtin if !account => {
            ui.label(style::hint("Zum Teilen und Speichern im Konto melde dich an (Startseite, „Konto“)."));
        }
        Src::Server { mine: true, code, global, .. } => {
            match code {
                Some(code) => {
                    ui.label("Code für die Klasse:");
                    ui.horizontal(|ui| {
                        ui.label(egui::RichText::new(format!("{}-{}", &code[..4.min(code.len())], &code[4.min(code.len())..])).size(22.0).monospace().strong());
                        if style::plain_btn(ui, "Kopieren", h * 0.8).clicked() {
                            sh.platform.clipboard_set(crate::platform::ClipData::Text(code.clone()));
                            sh.toast("Code kopiert");
                        }
                    });
                    ui.label(style::hint("Wer den Code unter „Teilen & Datei“ bei „Code eingeben“ einträgt, bekommt die Sammlung (nur lesen). Änderungen sehen alle."));
                    if style::plain_btn(ui, "Teilen beenden", h * 0.85).clicked() {
                        formulas::share(si, false);
                    }
                }
                None => {
                    ui.label(style::hint("Erzeuge einen Code, z. B. für deine Klasse. Andere können die Sammlung damit verwenden, aber nicht ändern."));
                    if style::accent_btn(ui, "Code erzeugen", h * 0.85).clicked() {
                        formulas::share(si, true);
                    }
                }
            }
            if formulas::is_admin() {
                let mut g = *global;
                if ui.checkbox(&mut g, "Für alle Nutzer bereitstellen (Admin)").changed() {
                    formulas::set_global(si, g);
                }
            }
        }
        _ => {
            ui.label(style::hint("Diese Sammlung kannst du exportieren, aber nicht selbst teilen."));
        }
    }
    style::caption(ui, "Code eingeben");
    if !account && !formulas::logged_in() {
        ui.label(style::hint("Melde dich an, um geteilte Sammlungen zu verwenden. Ohne Konto: Datei importieren."));
        return;
    }
    ui.horizontal(|ui| {
        let r = ui.add(egui::TextEdit::singleline(&mut ed.fbook.code).hint_text("z. B. ABCD-2345").desired_width(150.0).margin(Vec2::new(8.0, 6.0)));
        let go = r.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter));
        if style::plain_btn(ui, "Ansehen", h * 0.85).clicked() || go {
            formulas::preview(&ed.fbook.code);
        }
    });
    if let Some(p) = formulas::with(|l| l.preview.clone()) {
        style::info_card(ui, |ui| {
            ui.label(egui::RichText::new(&p.col.name).strong());
            ui.label(egui::RichText::new(format!("von {} · {} Formeln, {} Konstanten", p.owner, p.col.formulas.len(), p.col.consts.len())).size(12.0).color(c(pal.dim)));
            for f in p.col.formulas.iter().take(4) {
                ui.label(egui::RichText::new(format!("{}: {}", f.name, f.expr)).size(12.0));
            }
            if style::accent_btn(ui, "Hinzufügen", h * 0.85).clicked() {
                formulas::subscribe(&p.code);
                ed.fbook.code.clear();
            }
        });
    }
}

fn edit_form(ui: &mut Ui, ed: &mut Editor, sh: &mut Shared) {
    let pal = style::pal();
    let h = min_touch(sh.env.touch);
    let dir = sh.platform.config_dir();
    let Some(edit) = ed.fbook.edit.as_mut() else { return };
    let mut done = false;
    let mut save = false;
    let field = |ui: &mut Ui, label: &str, s: &mut String, hint: &str| {
        ui.label(egui::RichText::new(label).size(12.0).color(c(pal.dim)));
        ui.add(egui::TextEdit::singleline(s).hint_text(hint).desired_width(f32::INFINITY).margin(Vec2::new(8.0, 6.0)));
    };
    match edit {
        Edit::NewSet { name } | Edit::Rename { name, .. } => {
            field(ui, "Name der Sammlung", name, "z. B. Elektrotechnik 2. Jahrgang");
        }
        Edit::Formula { f, .. } => {
            field(ui, "Name", &mut f.name, "z. B. Ohmsches Gesetz");
            field(ui, "Formel (wie im Rechnen-Werkzeug)", &mut f.expr, "z. B. U = R·I");
            let ok = calc::is_calc_line(&f.expr);
            if !f.expr.trim().is_empty() {
                if ok {
                    let e = f.expr.clone();
                    ed.fbook.show_formula(ui, &e, 20.0);
                } else {
                    ui.label(egui::RichText::new("Formel nicht lesbar – sie braucht ein „=“.").size(12.0).color(c(pal.danger_text)));
                }
            }
            let Some(Edit::Formula { f, .. }) = ed.fbook.edit.as_mut() else { return };
            f.sync_vars();
            field(ui, "Thema", &mut f.topic, "z. B. Elektrotechnik – Grundlagen");
            field(ui, "Beschreibung", &mut f.desc, "optional");
            if !f.vars.is_empty() {
                style::caption(ui, "Größen");
                egui::Grid::new("fbook-vars").num_columns(3).spacing(Vec2::new(6.0, 4.0)).show(ui, |ui| {
                    for v in f.vars.iter_mut() {
                        ui.label(egui::RichText::new(&v.sym).strong());
                        ui.add(egui::TextEdit::singleline(&mut v.unit).hint_text("Einheit").desired_width(70.0));
                        ui.add(egui::TextEdit::singleline(&mut v.desc).hint_text("Bedeutung").desired_width(f32::INFINITY));
                        ui.end_row();
                    }
                });
            }
        }
        Edit::Named { unit, n, .. } => {
            let u = *unit;
            field(ui, if u { "Zeichen der Einheit" } else { "Formelzeichen" }, &mut n.sym, if u { "z. B. kp" } else { "z. B. ρ_Cu" });
            field(ui, "Wert", &mut n.value, if u { "z. B. 9,80665 N" } else { "z. B. 0,0178 Ω·mm²/m" });
            field(ui, "Beschreibung", &mut n.desc, "optional");
            if !n.value.trim().is_empty() {
                if let Err(e) = calc::quantity(&n.value, &calc::Opts::default()) {
                    ui.label(egui::RichText::new(format!("Wert nicht lesbar: {e}")).size(12.0).color(c(pal.danger_text)));
                }
            }
        }
    }
    ui.add_space(sp::S);
    ui.horizontal(|ui| {
        if style::accent_btn(ui, "Speichern", h * 0.9).clicked() {
            save = true;
        }
        if style::plain_btn(ui, "Abbrechen", h * 0.9).clicked() {
            done = true;
        }
    });
    if !save {
        if done {
            ed.fbook.edit = None;
        }
        return;
    }
    let Some(edit) = ed.fbook.edit.take() else { return };
    match edit {
        Edit::NewSet { name } => {
            let name = if name.trim().is_empty() { "Meine Formeln".to_string() } else { name.trim().to_string() };
            if let Some(i) = formulas::create(Collection::new(&name), &dir) {
                ed.fbook.set = i;
            }
        }
        Edit::Rename { set, name } => {
            if !name.trim().is_empty() {
                formulas::with(|l| {
                    if let Some(s) = l.sets.get_mut(set) {
                        s.col.name = name.trim().to_string();
                    }
                });
                formulas::save(set, &dir);
            }
        }
        Edit::Formula { set, idx, mut f } => {
            if !calc::is_calc_line(&f.expr) {
                sh.toast("Formel nicht lesbar – sie braucht ein „=“");
                ed.fbook.edit = Some(Edit::Formula { set, idx, f });
                return;
            }
            if f.name.trim().is_empty() {
                f.name = f.expr.clone();
            }
            f.vars.retain(|v| f.expr.contains(v.sym.as_str()) || !v.desc.is_empty());
            formulas::with(|l| {
                if let Some(s) = l.sets.get_mut(set) {
                    match idx {
                        Some(i) if i < s.col.formulas.len() => s.col.formulas[i] = f,
                        _ => s.col.formulas.push(f),
                    }
                }
            });
            formulas::save(set, &dir);
        }
        Edit::Named { set, unit, idx, n } => {
            if n.sym.trim().is_empty() || n.value.trim().is_empty() {
                return;
            }
            formulas::with(|l| {
                if let Some(s) = l.sets.get_mut(set) {
                    let list = if unit { &mut s.col.units } else { &mut s.col.consts };
                    match idx {
                        Some(i) if i < list.len() => list[i] = n,
                        _ => list.push(n),
                    }
                }
            });
            formulas::save(set, &dir);
        }
    }
}
