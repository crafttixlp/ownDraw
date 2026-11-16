//! Konto auf der Startseite (nur Browser mit Server): Anmelden/Registrieren/Abmelden, Lizenz einlösen und
//! -status, Liste der Boards am Server (eigene + geteilte) mit Öffnen/Umbenennen/Löschen. Ohne Server
//! (statisches Hosting) bleibt der Gastmodus-Hinweis.

use crate::app::Shared;
use crate::layout::FormFactor;
use crate::platform::web::api::{self, BoardInfo, Server};
use crate::style::{self, c};
use egui::{Align2, CornerRadius, Pos2, Rect, Sense, Stroke, Vec2};

enum Dlg {
    Login { register: bool, email: String, password: String, name: String, focus: bool },
    Redeem { code: String, focus: bool },
    Rename { id: String, name: String },
    Delete { id: String, name: String },
}

#[derive(Default)]
pub struct AccountUi {
    dlg: Option<Dlg>,
}

fn date(secs: i64) -> String {
    let s = owndraw_core::doc::rfc3339(secs * 1000);
    // JJJJ-MM-TT → TT.MM.JJJJ
    format!("{}.{}.{}", &s[8..10], &s[5..7], &s[0..4])
}

fn mb(v: f64) -> String {
    format!("{v:.1}").replace('.', ",")
}

/// Hinweis-/Kontokarte unter dem Kopf der Startseite.
pub fn header(ui: &mut egui::Ui, sh: &mut Shared, st: &mut AccountUi) {
    let p = style::pal();
    let h = sh.env.btn;
    let (server, me) = api::with(|n| (n.server, n.me.clone()));
    ui.add_space(8.0);
    egui::Frame::new().fill(c(p.accent_soft)).corner_radius(CornerRadius::same(10)).inner_margin(egui::Margin::symmetric(12, 8)).show(ui, |ui| {
        ui.set_width(ui.available_width());
        match (server, me) {
            (Server::Ready, Some(me)) => {
                let who = if me.name.is_empty() { me.email.clone() } else { format!("{} ({})", me.name, me.email) };
                ui.horizontal_wrapped(|ui| {
                    ui.label(egui::RichText::new(format!("Angemeldet als {who}")).size(13.5).strong().color(c(p.text)));
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        if style::plain_btn(ui, "Abmelden", h - 8.0).clicked() {
                            api::logout();
                        }
                        if !me.licensed() && style::accent_btn(ui, "Lizenzcode einlösen", h - 8.0).clicked() {
                            api::with(|n| n.form_error = None);
                            st.dlg = Some(Dlg::Redeem { code: String::new(), focus: true });
                        }
                    });
                });
                let line = match &me.license {
                    Some(l) if l.active => {
                        let until = l.expires_at.map(|e| format!("gültig bis {}", date(e))).unwrap_or_else(|| "unbefristet".into());
                        format!("Lizenz {until} · Speicher {} von {} MB belegt · neue Boards werden am Server gespeichert", mb(l.used_mb), mb(l.quota_mb))
                    }
                    Some(l) => format!(
                        "Lizenz abgelaufen{} – deine Boards am Server sind nur noch lesbar. Mit einem neuen Code wieder am Server speichern.",
                        l.expires_at.map(|e| format!(" am {}", date(e))).unwrap_or_default()
                    ),
                    None => "Keine Lizenz – neue Boards bleiben in diesem Browser (zum Sichern herunterladen). Mit Lizenz am Server speichern; Einladungen zu fremden Boards gehen auch ohne.".into(),
                };
                ui.label(egui::RichText::new(line).size(12.5).color(c(p.text)));
            }
            (Server::Ready, None) => {
                ui.horizontal_wrapped(|ui| {
                    ui.label(egui::RichText::new("Gastmodus – Boards bleiben in diesem Browser, bitte herunterladen (im Board: ⋯ → Herunterladen).").size(13.0).color(c(p.text)));
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        if style::primary_btn(ui, "Anmelden", h - 8.0).clicked() {
                            api::with(|n| n.form_error = None);
                            st.dlg = Some(Dlg::Login { register: false, email: String::new(), password: String::new(), name: String::new(), focus: true });
                        }
                        if style::plain_btn(ui, "Registrieren", h - 8.0).clicked() {
                            api::with(|n| n.form_error = None);
                            st.dlg = Some(Dlg::Login { register: true, email: String::new(), password: String::new(), name: String::new(), focus: true });
                        }
                    });
                });
                ui.label(egui::RichText::new("Mit Konto zu Boards eingeladen werden und live mitarbeiten; mit Lizenz Boards am Server speichern und teilen.").size(12.0).color(c(p.dim)));
            }
            _ => {
                ui.label(egui::RichText::new("Gastmodus – Boards werden nicht gespeichert, bitte herunterladen (im Board: ⋯ → Herunterladen).").size(13.0).color(c(p.text)));
            }
        }
    });
}

/// Abschnitt „Am Server“ (nur angemeldet, mit Lizenz oder geteilten Boards).
pub fn boards(ui: &mut egui::Ui, sh: &mut Shared, st: &mut AccountUi) {
    let (me, list, loaded, opening) = api::with(|n| (n.me.clone(), n.boards.clone(), n.boards_loaded, n.opening));
    let Some(me) = me else { return };
    if !me.licensed() && list.is_empty() {
        return;
    }
    let p = style::pal();
    let compact = sh.env.ff == FormFactor::Compact;
    ui.add_space(22.0);
    ui.horizontal(|ui| {
        ui.label(egui::RichText::new("Am Server").size(18.0).strong());
        if opening {
            ui.spinner();
        }
    });
    ui.add_space(8.0);
    if !loaded {
        ui.spinner();
        return;
    }
    if list.is_empty() {
        ui.label(egui::RichText::new("Noch keine Boards am Server. Lege mit „Neu“ eins an – es wird automatisch gespeichert.").color(c(p.dim)));
        return;
    }
    let gap = if compact { 12.0 } else { 18.0 };
    let avail = ui.available_width();
    let min_w = if compact { 140.0 } else { 200.0 };
    let cols = (((avail + gap) / (min_w + gap)).floor() as usize).max(2);
    let tw = ((avail - gap * (cols as f32 - 1.0)) / cols as f32).floor();
    let th = 112.0;
    ui.spacing_mut().item_spacing = Vec2::splat(gap);
    ui.horizontal_wrapped(|ui| {
        for b in &list {
            tile(ui, sh, st, b, tw, th);
        }
    });
}

fn role_text(b: &BoardInfo) -> String {
    match b.role.as_str() {
        "owner" => "Eigenes Board".into(),
        "edit" => format!("Von {} · bearbeiten", b.owner_name),
        _ => format!("Von {} · ansehen", b.owner_name),
    }
}

fn tile(ui: &mut egui::Ui, sh: &mut Shared, st: &mut AccountUi, b: &BoardInfo, tw: f32, th: f32) {
    let p = style::pal();
    let (r, resp) = ui.allocate_exact_size(Vec2::new(tw, th), Sense::click());
    let pt = ui.painter();
    pt.rect_filled(r, CornerRadius::same(16), c(p.card));
    // Kopf mit Wolken-Sinnbild und Rollenfarbe
    let head = Rect::from_min_size(r.min, Vec2::new(tw, 44.0));
    let col = match b.role.as_str() {
        "owner" => p.accent_soft,
        "edit" => 0x2230A46C,
        _ => 0x228B8D98,
    };
    pt.rect_filled(head, CornerRadius { nw: 16, ne: 16, sw: 0, se: 0 }, c(col));
    pt.text(head.left_center() + Vec2::new(14.0, 0.0), Align2::LEFT_CENTER, "☁", egui::FontId::proportional(20.0), c(p.accent));
    let badge = match b.role.as_str() {
        "owner" => "Eigenes",
        "edit" => "Geteilt",
        _ => "Nur ansehen",
    };
    pt.text(head.right_center() - Vec2::new(14.0, 0.0), Align2::RIGHT_CENTER, badge, egui::FontId::proportional(11.5), c(p.dim));
    let name = super::elide(&b.name, 15.0, tw - 32.0, ui);
    let pt = ui.painter();
    pt.text(Pos2::new(r.left() + 14.0, r.top() + 62.0), Align2::LEFT_CENTER, name, egui::FontId::proportional(15.0), c(p.text));
    let sub = super::elide(&role_text(b), 12.0, tw - 60.0, ui);
    let pt = ui.painter();
    pt.text(Pos2::new(r.left() + 14.0, r.top() + 82.0), Align2::LEFT_CENTER, sub, egui::FontId::proportional(12.0), c(p.dim));
    pt.text(Pos2::new(r.left() + 14.0, r.top() + 99.0), Align2::LEFT_CENTER, super::fmt_time(b.modified * 1000), egui::FontId::proportional(11.0), c(p.dim));
    if resp.hovered() {
        pt.rect_stroke(r, CornerRadius::same(16), Stroke::new(1.5, c(p.accent)), egui::StrokeKind::Inside);
    }
    if resp.clicked() {
        api::open_board(&b.id);
    }
    let menu = |ui: &mut egui::Ui, st: &mut AccountUi| {
        if style::menu_row(ui, "Öffnen", false, 34.0) {
            api::open_board(&b.id);
            ui.close();
        }
        if b.role == "owner" {
            if style::menu_row(ui, "Umbenennen …", false, 34.0) {
                st.dlg = Some(Dlg::Rename { id: b.id.clone(), name: b.name.clone() });
                ui.close();
            }
            if style::menu_row(ui, "Löschen …", true, 34.0) {
                st.dlg = Some(Dlg::Delete { id: b.id.clone(), name: b.name.clone() });
                ui.close();
            }
        }
    };
    resp.context_menu(|ui| menu(ui, st));
    let mr = Rect::from_center_size(Pos2::new(r.right() - 22.0, r.top() + 72.0), Vec2::splat(36.0));
    let mresp = ui.interact(mr, resp.id.with("more"), Sense::click());
    if mresp.hovered() {
        ui.painter().circle_filled(mr.center(), 16.0, c(p.hover));
    }
    sh.icons.paint(ui, mr, owndraw_core::icons::MORE, p.dim, false, 18.0);
    egui::Popup::menu(&mresp).align(egui::RectAlign::BOTTOM_END).show(|ui| menu(ui, st));
}

fn field(ui: &mut egui::Ui, label: &str, text: &mut String, password: bool, focus: bool) -> egui::Response {
    style::caption(ui, label);
    let r = ui.add(egui::TextEdit::singleline(text).password(password).desired_width(f32::INFINITY).font(egui::FontId::proportional(15.0)).margin(Vec2::new(10.0, 8.0)));
    if focus {
        r.request_focus();
    }
    r
}

/// Dialoge (Anmelden, Registrieren, Lizenz, Umbenennen, Löschen).
pub fn dialogs(ctx: &egui::Context, sh: &mut Shared, st: &mut AccountUi) {
    let Some(mut d) = st.dlg.take() else { return };
    let (busy, err, ok) = api::with(|n| (n.busy, n.form_error.clone(), std::mem::take(&mut n.form_ok)));
    if ok && matches!(d, Dlg::Login { .. } | Dlg::Redeem { .. }) {
        return; // erfolgreich: Dialog zu
    }
    let h = sh.env.btn;
    let p = style::pal();
    let width = (ctx.content_rect().width() - 24.0).clamp(260.0, 420.0);
    let (enter, esc) = ctx.input(|i| (i.key_pressed(egui::Key::Enter), i.key_pressed(egui::Key::Escape)));
    let mut close = esc;
    egui::Modal::new(egui::Id::unique("account-dlg")).frame(style::card()).show(ctx, |ui| {
        ui.set_width(width - 40.0);
        match &mut d {
            Dlg::Login { register, email, password, name, focus } => {
                ui.label(egui::RichText::new(if *register { "Konto anlegen" } else { "Anmelden" }).heading().strong());
                ui.add_space(6.0);
                if *register {
                    field(ui, "Name", name, false, *focus);
                    field(ui, "E-Mail", email, false, false);
                } else {
                    field(ui, "E-Mail", email, false, *focus);
                }
                field(ui, if *register { "Passwort (mindestens 8 Zeichen)" } else { "Passwort" }, password, true, false);
                *focus = false;
                if let Some(e) = &err {
                    ui.label(egui::RichText::new(e).size(12.5).color(c(p.danger_text)));
                }
                ui.add_space(10.0);
                let valid = email.contains('@') && !password.is_empty() && !busy;
                ui.allocate_ui_with_layout(egui::vec2(ui.available_width(), h), egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    ui.add_enabled_ui(valid, |ui| {
                        if (style::primary_btn(ui, if *register { "Registrieren" } else { "Anmelden" }, h).clicked() || enter) && valid {
                            if *register {
                                api::register(email, password, name);
                            } else {
                                api::login(email, password);
                            }
                        }
                    });
                    close |= style::plain_btn(ui, "Abbrechen", h).clicked();
                    if busy {
                        ui.spinner();
                    }
                });
                ui.add_space(4.0);
                let other = if *register { "Schon ein Konto? Anmelden" } else { "Noch kein Konto? Registrieren" };
                if ui.add(egui::Button::new(egui::RichText::new(other).size(12.5).color(c(p.accent))).fill(egui::Color32::TRANSPARENT)).clicked() {
                    *register = !*register;
                    api::with(|n| n.form_error = None);
                }
            }
            Dlg::Redeem { code, focus } => {
                ui.label(egui::RichText::new("Lizenzcode einlösen").heading().strong());
                ui.label(egui::RichText::new("Mit einer Lizenz werden deine Boards am Server gespeichert, sind auf allen Geräten da und lassen sich teilen.").size(12.5).color(c(p.dim)));
                ui.add_space(6.0);
                field(ui, "Code", code, false, *focus);
                *focus = false;
                if let Some(e) = &err {
                    ui.label(egui::RichText::new(e).size(12.5).color(c(p.danger_text)));
                }
                ui.add_space(10.0);
                let valid = code.trim().len() >= 4 && !busy;
                ui.allocate_ui_with_layout(egui::vec2(ui.available_width(), h), egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    ui.add_enabled_ui(valid, |ui| {
                        if (style::primary_btn(ui, "Einlösen", h).clicked() || enter) && valid {
                            api::redeem(code);
                        }
                    });
                    close |= style::plain_btn(ui, "Abbrechen", h).clicked();
                });
            }
            Dlg::Rename { id, name } => {
                ui.label(egui::RichText::new("Board umbenennen").heading().strong());
                ui.add_space(6.0);
                let r = ui.add(egui::TextEdit::singleline(name).desired_width(f32::INFINITY).font(egui::FontId::proportional(15.0)).margin(Vec2::new(10.0, 8.0)));
                r.request_focus();
                ui.add_space(10.0);
                ui.allocate_ui_with_layout(egui::vec2(ui.available_width(), h), egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    let valid = !name.trim().is_empty();
                    if (style::primary_btn(ui, "OK", h).clicked() || enter) && valid {
                        api::rename_board(id, name.trim());
                        close = true;
                    }
                    close |= style::plain_btn(ui, "Abbrechen", h).clicked();
                });
            }
            Dlg::Delete { id, name } => {
                ui.label(egui::RichText::new("Board löschen?").heading().strong());
                ui.label(format!("„{name}“ wird am Server gelöscht – auch für alle, mit denen es geteilt ist. Das lässt sich nicht rückgängig machen."));
                ui.add_space(10.0);
                ui.allocate_ui_with_layout(egui::vec2(ui.available_width(), h), egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if style::danger_btn(ui, "Löschen", h).clicked() {
                        api::delete_board(id);
                        close = true;
                    }
                    close |= style::plain_btn(ui, "Abbrechen", h).clicked();
                });
            }
        }
    });
    if !close {
        st.dlg = Some(d);
    }
}
