//! Modale Dialoge: Umbenennen, Bestätigen, Ebenen-Animation und „Neu“ (unendlich/Seiten).

use crate::app::Shared;
use crate::platform::FileRef;
use crate::session::NewParams;
use crate::style::{self, c};
use egui::{Key, Sense, Stroke, Vec2};
use owndraw_core::layer::{A_PARALLAX, ANIM_NAMES};
use owndraw_core::pages::{PageBg, PageFormat};

pub enum RenameWhat {
    Doc,
    Layer(usize),
}

pub enum ConfirmAction {
    /// Speichern nicht möglich → „Kopie speichern unter …“ anbieten.
    SaveCopy,
    DeleteLayer(usize),
    DeletePage(usize),
}

pub struct AnimDlg {
    pub idx: usize,
    pub anim: i32,
    pub speed: f32,
    pub hover: bool,
    pub opacity: f32,
}

pub struct NewDlg {
    pub params: NewParams,
    /// true = „Selbst wählen“ (System-Speicherdialog), false = Standard-Ort.
    pub custom: bool,
    /// Einstellung „immer fragen“: Auswahl Standard/selbst anzeigen; sonst nur den Standard-Ort nennen.
    pub ask: bool,
    pub focus: bool,
    /// Browser mit Lizenz: Auswahl Server/Browser anzeigen.
    pub server_choice: bool,
    /// Auf dem Server anlegen (nur mit `server_choice`).
    pub server: bool,
}

pub enum Dialog {
    Rename { what: RenameWhat, title: &'static str, text: String, focus: bool },
    Confirm { title: &'static str, text: String, ok: &'static str, action: ConfirmAction },
    Anim(AnimDlg),
    New(NewDlg),
}

/// Ergebnis eines bestätigten Dialogs.
pub enum Outcome {
    Rename(RenameWhat, String),
    Confirm(ConfirmAction),
    Anim(AnimDlg),
    /// Neues Dokument anlegen (Parameter, Speicherort selbst wählen?, auf dem Server?).
    Create(NewParams, bool, bool),
}

impl Dialog {
    /// Neu-Dialog mit den Vorgaben (A4 hoch liniert, Standard-Ort); `ask` = Einstellung „immer fragen“.
    /// `server`: Some(Vorgabe) = Browser mit Lizenz, Speicherort Server/Browser wählbar.
    pub fn new_doc(ask: bool, server: Option<bool>) -> Dialog {
        let params = NewParams { name: "Unbenannt".into(), ..NewParams::default() };
        Dialog::New(NewDlg { params, custom: false, ask, focus: true, server_choice: server.is_some(), server: server.unwrap_or(false) })
    }
}

/// Zeichnet den offenen Dialog; liefert das Ergebnis, wenn bestätigt (Abbrechen schließt ohne Ergebnis).
pub fn show(ctx: &egui::Context, sh: &mut Shared) -> Option<Outcome> {
    let mut d = sh.dialog.take()?;
    let mut close = false;
    let mut ok = false;
    let (enter, esc) = ctx.input(|i| (i.key_pressed(Key::Enter), i.key_pressed(Key::Escape)));
    let h = sh.env.btn;
    let screen = ctx.content_rect();
    let width = (screen.width() - 24.0).clamp(240.0, 420.0);
    let default_dir = sh.platform.default_dir();
    let default_name = match &default_dir {
        _ if crate::platform::WEB => "Wird nur in diesem Browser gespeichert. Lade das Board herunter, um es zu sichern.".to_string(),
        FileRef::Path(p) => p.display().to_string(),
        FileRef::Uri(u) => u.clone(),
    };
    egui::Modal::new(egui::Id::unique("dlg")).frame(style::card()).show(ctx, |ui| {
        ui.set_width(width - 40.0);
        let p = style::pal();
        match &mut d {
            Dialog::Rename { title, text, focus, .. } => {
                ui.label(egui::RichText::new(*title).heading().strong());
                ui.add_space(8.0);
                let r = ui.add(egui::TextEdit::singleline(text).desired_width(f32::INFINITY).font(egui::FontId::proportional(15.0)).margin(Vec2::new(10.0, 8.0)));
                if *focus {
                    select_all(ui, &r, text);
                    *focus = false;
                }
                ui.add_space(12.0);
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    ok |= style::primary_btn(ui, "OK", h).clicked() || enter;
                    close |= style::plain_btn(ui, "Abbrechen", h).clicked() || esc;
                });
            }
            Dialog::Confirm { title, text, ok: okl, .. } => {
                ui.label(egui::RichText::new(*title).heading().strong());
                ui.add_space(8.0);
                ui.label(text.as_str());
                ui.add_space(12.0);
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    ok |= style::danger_btn(ui, okl, h).clicked() || enter;
                    close |= style::plain_btn(ui, "Abbrechen", h).clicked() || esc;
                });
            }
            Dialog::Anim(a) => {
                ui.label(egui::RichText::new("Animation & Deckkraft").heading().strong());
                ui.add_space(6.0);
                for (i, n) in ANIM_NAMES.iter().enumerate() {
                    let i = i as i32;
                    if ui.radio(a.anim == i, *n).clicked() && a.anim != i {
                        if (i == A_PARALLAX) != (a.anim == A_PARALLAX) {
                            a.speed = if i == A_PARALLAX { 0.2 } else { 1.0 };
                        }
                        a.anim = i;
                    }
                }
                ui.add_space(6.0);
                let par = a.anim == A_PARALLAX;
                let mut pr = if par { a.speed * 100.0 } else { (a.speed - 0.25) / 2.75 * 100.0 };
                let lbl = if par { "Parallax-Faktor" } else { "Tempo" };
                ui.label(format!("{lbl}: {}×", format!("{:.1}", a.speed).replace('.', ",")));
                if ui.add(egui::Slider::new(&mut pr, 0.0..=100.0).show_value(false)).changed() {
                    let v = if par { pr / 100.0 } else { 0.25 + pr / 100.0 * 2.75 };
                    a.speed = (v / 0.05).round() * 0.05;
                }
                ui.checkbox(&mut a.hover, "Hover-Effekt");
                let mut o = a.opacity * 100.0;
                ui.label(format!("Deckkraft: {} %", o.round() as i32));
                if ui.add(egui::Slider::new(&mut o, 0.0..=100.0).show_value(false)).changed() {
                    a.opacity = (o.round() / 100.0).clamp(0.0, 1.0);
                }
                ui.add_space(12.0);
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    ok |= style::primary_btn(ui, "Übernehmen", h).clicked();
                    close |= style::plain_btn(ui, "Abbrechen", h).clicked() || esc;
                });
            }
            Dialog::New(n) => {
                ui.label(egui::RichText::new("Neu").heading().strong());
                ui.add_space(8.0);
                // Art: zwei Karten
                ui.horizontal(|ui| {
                    let cw = (ui.available_width() - 8.0) / 2.0;
                    for (pages, title, sub) in [(false, "Unendliches Board", "Frei zeichnen, beliebig groß"), (true, "Seiten", "Feste Seiten wie Papier")] {
                        let (r, resp) = ui.allocate_exact_size(Vec2::new(cw, 92.0), Sense::click());
                        let sel = n.params.pages == pages;
                        let pt = ui.painter();
                        pt.rect_filled(r, 12.0, c(if sel { p.active } else { p.hover }));
                        pt.rect_stroke(r, 12.0, Stroke::new(if sel { 1.5 } else { 1.0 }, c(if sel { p.accent } else { p.line })), egui::StrokeKind::Inside);
                        // kleines Sinnbild
                        let ic = egui::Rect::from_center_size(r.center_top() + Vec2::new(0.0, 28.0), Vec2::new(40.0, 32.0));
                        if pages {
                            for k in 0..2 {
                                let pr = egui::Rect::from_min_size(ic.min + Vec2::new(5.0 + k as f32 * 14.0, 2.0 - k as f32 * 0.0), Vec2::new(20.0, 28.0));
                                pt.rect_filled(pr, 2.0, c(0xFFFBFBF8));
                                pt.rect_stroke(pr, 2.0, Stroke::new(1.0, c(p.dim)), egui::StrokeKind::Inside);
                            }
                        } else {
                            pt.rect_filled(ic, 6.0, c(0xFFFBFBF8));
                            pt.rect_stroke(ic, 6.0, Stroke::new(1.0, c(p.dim)), egui::StrokeKind::Inside);
                            pt.line_segment([ic.left_bottom() + Vec2::new(6.0, -8.0), ic.right_top() + Vec2::new(-6.0, 8.0)], Stroke::new(2.0, c(p.accent)));
                        }
                        pt.text(r.center() + Vec2::new(0.0, 22.0), egui::Align2::CENTER_CENTER, title, egui::FontId::proportional(14.5), c(p.text));
                        pt.text(r.center() + Vec2::new(0.0, 40.0), egui::Align2::CENTER_CENTER, sub, egui::FontId::proportional(11.0), c(p.dim));
                        if resp.clicked() {
                            n.params.pages = pages;
                        }
                    }
                });
                ui.add_space(6.0);
                if n.params.pages {
                    style::caption(ui, "Format");
                    ui.horizontal_wrapped(|ui| {
                        for (f, l) in [(PageFormat::A4, "A4"), (PageFormat::A5, "A5"), (PageFormat::Letter, "Letter")] {
                            if style::chip(ui, l, n.params.format == f, h).clicked() {
                                n.params.format = f;
                            }
                        }
                        ui.add_space(8.0);
                        for (ls, l) in [(false, "Hoch"), (true, "Quer")] {
                            if style::chip(ui, l, n.params.landscape == ls, h).clicked() {
                                n.params.landscape = ls;
                            }
                        }
                    });
                    style::caption(ui, "Hintergrund");
                    ui.horizontal_wrapped(|ui| {
                        for (b, l) in [(PageBg::Plain, "Blanko"), (PageBg::Lined, "Liniert"), (PageBg::Grid, "Kariert"), (PageBg::Dotted, "Punktiert")] {
                            if style::chip(ui, l, n.params.bg == b, h).clicked() {
                                n.params.bg = b;
                            }
                        }
                    });
                    ui.add_space(4.0);
                }
                style::caption(ui, "Name");
                let r = ui.add(egui::TextEdit::singleline(&mut n.params.name).desired_width(f32::INFINITY).font(egui::FontId::proportional(15.0)).margin(Vec2::new(10.0, 8.0)));
                if n.focus {
                    let t = n.params.name.clone();
                    select_all(ui, &r, &t);
                    n.focus = false;
                }
                ui.add_space(4.0);
                style::caption(ui, "Speicherort");
                if n.server_choice {
                    ui.horizontal_wrapped(|ui| {
                        if style::chip(ui, "Auf dem Server", n.server, h).clicked() {
                            n.server = true;
                        }
                        if style::chip(ui, "In diesem Browser", !n.server, h).clicked() {
                            n.server = false;
                        }
                    });
                    let hint = if n.server {
                        let free = crate::ui::server_free_text().map(|f| format!(" Frei: {f}.")).unwrap_or_default();
                        format!("Auf allen Geräten verfügbar, teilbar und live gemeinsam bearbeitbar.{free}")
                    } else {
                        "Nur in diesem Browser. Teilen und Live-Zusammenarbeit gehen nur mit Boards auf dem Server. Du kannst das Board später verschieben.".to_string()
                    };
                    ui.add(egui::Label::new(egui::RichText::new(hint).size(11.5).color(c(p.dim))).wrap());
                } else if n.ask {
                    ui.horizontal_wrapped(|ui| {
                        if style::chip(ui, "Standard-Ort", !n.custom, h).clicked() {
                            n.custom = false;
                        }
                        if style::chip(ui, "Selbst wählen", n.custom, h).clicked() {
                            n.custom = true;
                        }
                    });
                }
                if !n.server_choice {
                    let hint = if n.custom { "Beim Erstellen öffnet sich der Speicherdialog des Systems.".to_string() } else { default_name.clone() };
                    ui.label(egui::RichText::new(hint).size(11.5).color(c(p.dim)));
                }
                ui.add_space(10.0);
                let valid = !n.params.name.trim().is_empty();
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    ui.add_enabled_ui(valid, |ui| {
                        ok |= style::primary_btn(ui, "Erstellen", h).clicked();
                    });
                    close |= style::plain_btn(ui, "Abbrechen", h).clicked() || esc;
                });
                if enter && valid {
                    ok = true;
                }
            }
        }
    });
    if ok {
        match d {
            Dialog::Rename { what, text, .. } => {
                let t = text.trim().to_string();
                if t.is_empty() {
                    sh.dialog = None;
                    return None;
                }
                Some(Outcome::Rename(what, t))
            }
            Dialog::Confirm { action, .. } => Some(Outcome::Confirm(action)),
            Dialog::Anim(a) => Some(Outcome::Anim(a)),
            Dialog::New(mut n) => {
                n.params.name = n.params.name.trim().to_string();
                Some(Outcome::Create(n.params, n.custom, n.server_choice && n.server))
            }
        }
    } else if close {
        None
    } else {
        sh.dialog = Some(d);
        None
    }
}

fn select_all(ui: &mut egui::Ui, r: &egui::Response, text: &str) {
    r.request_focus();
    if let Some(mut st) = egui::TextEdit::load_state(ui.ctx(), r.id) {
        let cr = egui::text::CCursorRange::two(egui::text::CCursor::new(0), egui::text::CCursor::new(text.chars().count()));
        st.cursor.set_char_range(Some(cr));
        st.store(ui.ctx(), r.id);
    }
}
