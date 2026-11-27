//! Modale Dialoge: Umbenennen, Bestätigen, Ebenen-Animation und „Neu“ (unendlich/Seiten).

use crate::app::Shared;
use crate::platform::FileRef;
use crate::session::NewParams;
use super::elide;
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
    let wide_dlg = matches!(d, Dialog::New(_));
    let width = if wide_dlg { (screen.width() - 24.0).clamp(240.0, 460.0) } else { width };
    let max_body = (screen.height() - 150.0).max(160.0);
    egui::Modal::new(egui::Id::unique("dlg")).frame(style::card().inner_margin(egui::Margin::same(style::sp::XL as i8))).show(ctx, |ui| {
        ui.set_width(width - 2.0 * style::sp::XL - 2.0);
        ui.spacing_mut().item_spacing.y = style::sp::S;
        let p = style::pal();
        match &mut d {
            Dialog::Rename { title, text, focus, .. } => {
                header(ui, title, None);
                ui.add_space(style::sp::XS);
                let r = text_field(ui, text);
                if *focus {
                    select_all(ui, &r, text);
                    *focus = false;
                }
                ui.add_space(style::sp::S);
                let (o, cl) = buttons(ui, h, "OK", Kind::Primary, true);
                ok |= o || enter;
                close |= cl || esc;
            }
            Dialog::Confirm { title, text, ok: okl, action } => {
                header(ui, title, Some(text.as_str()));
                ui.add_space(style::sp::S);
                // Löschen = rot, alles andere (z. B. „Kopie speichern“) = Hauptaktion
                let kind = if matches!(action, ConfirmAction::SaveCopy) { Kind::Primary } else { Kind::Danger };
                let (o, cl) = buttons(ui, h, okl, kind, true);
                ok |= o || enter;
                close |= cl || esc;
            }
            Dialog::Anim(a) => {
                header(ui, "Animation & Deckkraft", None);
                egui::ScrollArea::vertical().max_height(max_body).auto_shrink([false, true]).show(ui, |ui| {
                    style::caption(ui, "Animation");
                    ui.horizontal_wrapped(|ui| {
                        ui.spacing_mut().item_spacing = Vec2::splat(style::sp::S);
                        for (i, n) in ANIM_NAMES.iter().enumerate() {
                            let i = i as i32;
                            if style::chip(ui, n, a.anim == i, h).clicked() && a.anim != i {
                                if (i == A_PARALLAX) != (a.anim == A_PARALLAX) {
                                    a.speed = if i == A_PARALLAX { 0.2 } else { 1.0 };
                                }
                                a.anim = i;
                            }
                        }
                    });
                    ui.add_space(style::sp::XS);
                    let par = a.anim == A_PARALLAX;
                    let mut pr = if par { a.speed * 100.0 } else { (a.speed - 0.25) / 2.75 * 100.0 };
                    let lbl = if par { "Parallax-Faktor" } else { "Tempo" };
                    ui.horizontal(|ui| {
                        ui.label(lbl);
                        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| ui.label(style::hint(&format!("{}×", format!("{:.1}", a.speed).replace('.', ",")))));
                    });
                    ui.spacing_mut().slider_width = ui.available_width();
                    if ui.add(egui::Slider::new(&mut pr, 0.0..=100.0).show_value(false)).changed() {
                        let v = if par { pr / 100.0 } else { 0.25 + pr / 100.0 * 2.75 };
                        a.speed = (v / 0.05).round() * 0.05;
                    }
                    ui.add_space(style::sp::XS);
                    let mut o = a.opacity * 100.0;
                    ui.horizontal(|ui| {
                        ui.label("Deckkraft");
                        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| ui.label(style::hint(&format!("{} %", o.round() as i32))));
                    });
                    if ui.add(egui::Slider::new(&mut o, 0.0..=100.0).show_value(false)).changed() {
                        a.opacity = (o.round() / 100.0).clamp(0.0, 1.0);
                    }
                    ui.add_space(style::sp::XS);
                    ui.checkbox(&mut a.hover, "Hover-Effekt");
                });
                ui.add_space(style::sp::S);
                let (o, cl) = buttons(ui, h, "Übernehmen", Kind::Primary, true);
                ok |= o;
                close |= cl || esc;
            }
            Dialog::New(n) => {
                header(ui, if n.params.text { "Neues Textdokument" } else { "Neues Board" }, Some("Was möchtest du erstellen?"));
                egui::ScrollArea::vertical().max_height(max_body).auto_shrink([false, true]).show(ui, |ui| {
                    // Art: Karten (Unendlich, Seiten, Textdokument)
                    let office = !crate::locks::locked(owndraw_core::features::OFFICE);
                    if !office {
                        n.params.text = false;
                    }
                    ui.horizontal(|ui| {
                        ui.spacing_mut().item_spacing.x = style::sp::S;
                        let cards: &[(u8, &str, &str)] = &[(0, "Unendlich", "Frei zeichnen, beliebig groß"), (1, "Seiten", "Feste Seiten wie Papier"), (2, "Textdokument", "Schreiben wie in Word")];
                        let cards = if office { cards } else { &cards[..2] };
                        let nc = cards.len() as f32;
                        let cw = (ui.available_width() - style::sp::S * (nc - 1.0)) / nc;
                        for &(kind, title, sub) in cards {
                            let pages = kind == 1;
                            let (r, resp) = ui.allocate_exact_size(Vec2::new(cw, 108.0), Sense::click());
                            let sel = if n.params.text { kind == 2 } else { kind < 2 && n.params.pages == pages };
                            let pt = ui.painter();
                            let rr = egui::CornerRadius::same(10);
                            pt.rect_filled(r, rr, c(if sel { p.active } else if resp.hovered() { p.hover } else { p.card }));
                            pt.rect_stroke(r, rr, Stroke::new(if sel { 1.5 } else { 1.0 }, c(if sel { p.accent } else { p.line })), egui::StrokeKind::Inside);
                            // kleines Sinnbild
                            let ic = egui::Rect::from_center_size(r.center_top() + Vec2::new(0.0, 30.0), Vec2::new(40.0, 32.0));
                            if kind == 2 {
                                let pr = egui::Rect::from_center_size(ic.center(), Vec2::new(24.0, 31.0));
                                pt.rect_filled(pr, 2.0, c(0xFFFBFBF8));
                                pt.rect_stroke(pr, 2.0, Stroke::new(1.0, c(p.dim)), egui::StrokeKind::Inside);
                                for l in 0..5 {
                                    let y = pr.top() + 7.0 + l as f32 * 4.5;
                                    let x1 = if l == 4 { pr.center().x } else { pr.right() - 4.0 };
                                    pt.line_segment([egui::pos2(pr.left() + 4.0, y), egui::pos2(x1, y)], Stroke::new(1.4, c(if l == 0 { p.accent } else { p.dim })));
                                }
                            } else if pages {
                                for k in 0..2 {
                                    let pr = egui::Rect::from_min_size(ic.min + Vec2::new(5.0 + k as f32 * 14.0, 2.0), Vec2::new(20.0, 28.0));
                                    pt.rect_filled(pr, 2.0, c(0xFFFBFBF8));
                                    pt.rect_stroke(pr, 2.0, Stroke::new(1.0, c(p.dim)), egui::StrokeKind::Inside);
                                }
                            } else {
                                pt.rect_filled(ic, 6.0, c(0xFFFBFBF8));
                                pt.rect_stroke(ic, 6.0, Stroke::new(1.0, c(p.dim)), egui::StrokeKind::Inside);
                                pt.line_segment([ic.left_bottom() + Vec2::new(6.0, -8.0), ic.right_top() + Vec2::new(-6.0, 8.0)], Stroke::new(2.0, c(p.accent)));
                            }
                            let tsub = elide(sub, style::fs::CAPTION, cw - 12.0, ui);
                            let pt = ui.painter();
                            pt.text(r.center() + Vec2::new(0.0, 26.0), egui::Align2::CENTER_CENTER, title, egui::FontId::proportional(style::fs::BODY), c(p.text));
                            pt.text(r.center() + Vec2::new(0.0, 44.0), egui::Align2::CENTER_CENTER, tsub, egui::FontId::proportional(style::fs::CAPTION), c(p.dim));
                            if resp.on_hover_cursor(egui::CursorIcon::PointingHand).clicked() {
                                n.params.text = kind == 2;
                                n.params.pages = kind != 0;
                            }
                        }
                    });
                    if n.params.pages {
                        style::caption(ui, "Format");
                        ui.horizontal_wrapped(|ui| {
                            ui.spacing_mut().item_spacing = Vec2::splat(style::sp::S);
                            for (f, l) in [(PageFormat::A4, "A4"), (PageFormat::A5, "A5"), (PageFormat::Letter, "Letter")] {
                                if style::chip(ui, l, n.params.format == f, h).clicked() {
                                    n.params.format = f;
                                }
                            }
                            ui.add_space(style::sp::S);
                            for (ls, l) in [(false, "Hoch"), (true, "Quer")] {
                                if style::chip(ui, l, n.params.landscape == ls, h).clicked() {
                                    n.params.landscape = ls;
                                }
                            }
                        });
                    }
                    if n.params.pages && !n.params.text {
                        style::caption(ui, "Hintergrund");
                        ui.horizontal_wrapped(|ui| {
                            ui.spacing_mut().item_spacing = Vec2::splat(style::sp::S);
                            for (b, l) in [(PageBg::Plain, "Blanko"), (PageBg::Lined, "Liniert"), (PageBg::Grid, "Kariert"), (PageBg::Dotted, "Punktiert")] {
                                if style::chip(ui, l, n.params.bg == b, h).clicked() {
                                    n.params.bg = b;
                                }
                            }
                        });
                    }
                    style::caption(ui, "Name");
                    let r = text_field(ui, &mut n.params.name);
                    if n.focus {
                        let t = n.params.name.clone();
                        select_all(ui, &r, &t);
                        n.focus = false;
                    }
                    style::caption(ui, "Speicherort");
                    if n.server_choice {
                        ui.horizontal_wrapped(|ui| {
                            ui.spacing_mut().item_spacing = Vec2::splat(style::sp::S);
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
                        ui.add(egui::Label::new(style::hint(&hint)).wrap());
                    } else if n.ask {
                        ui.horizontal_wrapped(|ui| {
                            ui.spacing_mut().item_spacing = Vec2::splat(style::sp::S);
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
                        ui.add(egui::Label::new(style::hint(&hint)).wrap());
                    }
                });
                ui.add_space(style::sp::S);
                let valid = !n.params.name.trim().is_empty();
                let (o, cl) = buttons(ui, h, "Erstellen", Kind::Primary, valid);
                ok |= o;
                close |= cl || esc;
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

/// Titel (und optional ein erklärender Text) eines Dialogs.
fn header(ui: &mut egui::Ui, title: &str, text: Option<&str>) {
    let p = style::pal();
    ui.label(egui::RichText::new(title).size(style::fs::SECTION + 5.0).strong().color(c(p.text)));
    if let Some(t) = text {
        ui.add_space(style::sp::XS);
        ui.add(egui::Label::new(egui::RichText::new(t).size(style::fs::BODY).color(c(p.dim))).wrap());
    }
}

/// Eingabefeld im Dialog (volle Breite, Rahmen in Akzentfarbe bei Fokus).
fn text_field(ui: &mut egui::Ui, text: &mut String) -> egui::Response {
    let p = style::pal();
    let r = ui.add(egui::TextEdit::singleline(text).desired_width(f32::INFINITY).font(egui::FontId::proportional(style::fs::BODY + 0.5)).margin(egui::Margin::symmetric(12, 10)));
    let focus = r.has_focus();
    ui.painter().rect_stroke(r.rect, egui::CornerRadius::same(style::btn_r()), Stroke::new(if focus { 1.5 } else { 1.0 }, c(if focus { p.accent } else { p.line })), egui::StrokeKind::Inside);
    r
}

/// Art der Hauptaktion eines Dialogs.
enum Kind {
    Primary,
    Danger,
}

/// Knopfreihe rechts: Hauptaktion ganz rechts, links davon „Abbrechen“ mit Rahmen. Liefert (bestätigt, abgebrochen).
fn buttons(ui: &mut egui::Ui, h: f32, ok_label: &str, kind: Kind, enabled: bool) -> (bool, bool) {
    let (mut ok, mut cancel) = (false, false);
    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
        ui.spacing_mut().item_spacing.x = style::sp::S;
        ui.add_enabled_ui(enabled, |ui| {
            ok = match kind {
                Kind::Primary => style::primary_btn(ui, ok_label, h),
                Kind::Danger => style::danger_btn(ui, ok_label, h),
            }
            .clicked();
        });
        cancel = if style::modern() { style::outline_btn(ui, "Abbrechen", h) } else { style::plain_btn(ui, "Abbrechen", h) }.clicked();
    });
    (ok, cancel)
}

fn select_all(ui: &mut egui::Ui, r: &egui::Response, text: &str) {
    r.request_focus();
    if let Some(mut st) = egui::TextEdit::load_state(ui.ctx(), r.id) {
        let cr = egui::text::CCursorRange::two(egui::text::CCursor::new(0), egui::text::CCursor::new(text.chars().count()));
        st.cursor.set_char_range(Some(cr));
        st.store(ui.ctx(), r.id);
    }
}
