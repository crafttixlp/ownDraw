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
    /// Gastzugang beenden (Bestätigung).
    LeaveGuest,
}

#[derive(Default)]
pub struct AccountUi {
    dlg: Option<Dlg>,
    /// Name für den Gastzugang im Einladungsdialog.
    invite_name: String,
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
    let compact = sh.env.ff == FormFactor::Compact;
    let h = if compact { sh.env.btn } else { 36.0 };
    let (server, me) = api::with(|n| (n.server, n.me.clone()));
    let login = |register: bool| Dlg::Login { register, email: String::new(), password: String::new(), name: String::new(), focus: true };
    match (server, me) {
        (Server::Ready, Some(me)) if me.guest => {
            let title = format!("Gast-ID: {}", me.name);
            super::start::banner(ui, compact, &title, "Als Gast siehst du nur Boards, zu denen du eingeladen wurdest. Mit einem Konto kannst du eigene Boards speichern.", |ui| {
                if style::primary_btn(ui, "Konto erstellen", h).clicked() {
                    api::with(|n| n.form_error = None);
                    st.dlg = Some(login(true));
                }
                if style::outline_btn(ui, "Gastzugang beenden", h).clicked() {
                    st.dlg = Some(Dlg::LeaveGuest);
                }
            });
        }
        (Server::Ready, Some(me)) => {
            let who = if me.name.is_empty() { me.email.clone() } else { format!("{} ({})", me.name, me.email) };
            let line = match &me.license {
                Some(l) if l.active => {
                    let until = l.expires_at.map(|e| format!("gültig bis {}", date(e))).unwrap_or_else(|| "unbefristet gültig".into());
                    format!("Lizenz {until}. {} von {} MB Speicher belegt. Neue Boards werden auf dem Server gespeichert.", mb(l.used_mb), mb(l.quota_mb))
                }
                Some(l) => format!(
                    "Lizenz abgelaufen{}. Deine Boards auf dem Server sind nur noch lesbar. Löse einen neuen Code ein, um wieder zu speichern.",
                    l.expires_at.map(|e| format!(" am {}", date(e))).unwrap_or_default()
                ),
                None => "Keine Lizenz. Neue Boards werden nur in diesem Browser gespeichert. Lade sie herunter, um sie zu sichern. Mit einer Lizenz speicherst du auf dem Server. Einladungen zu Boards anderer funktionieren auch ohne Lizenz.".into(),
            };
            let licensed = me.licensed();
            super::start::banner(ui, compact, &format!("Angemeldet als {who}"), &line, |ui| {
                if !licensed && style::primary_btn(ui, "Lizenz einlösen", h).clicked() {
                    api::with(|n| n.form_error = None);
                    st.dlg = Some(Dlg::Redeem { code: String::new(), focus: true });
                }
                if style::outline_btn(ui, "Abmelden", h).clicked() {
                    api::logout();
                }
            });
        }
        (Server::Ready, None) => {
            super::start::banner(
                ui,
                compact,
                "Du bist nicht angemeldet. Boards werden nur in diesem Browser gespeichert.",
                "Mit einem Konto wirst du zu Boards eingeladen und arbeitest live mit. Mit einer Lizenz speicherst und teilst du Boards auf dem Server.",
                |ui| {
                    if style::primary_btn(ui, "Anmelden", h).clicked() {
                        api::with(|n| n.form_error = None);
                        st.dlg = Some(login(false));
                    }
                    if style::outline_btn(ui, "Konto erstellen", h).clicked() {
                        api::with(|n| n.form_error = None);
                        st.dlg = Some(login(true));
                    }
                },
            );
        }
        _ => {
            super::start::banner(ui, compact, "Du bist nicht angemeldet.", "Boards werden nicht gespeichert. Lade sie herunter, um sie zu sichern.", |_| {});
        }
    }
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
        ui.label(egui::RichText::new("Auf dem Server").size(18.0).strong());
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
        ui.label(egui::RichText::new("Noch keine Boards auf dem Server. Erstelle mit „Neu“ eins. Es wird automatisch gespeichert.").color(c(p.dim)));
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
        "edit" => format!("Von {} · Bearbeiten erlaubt", b.owner_name),
        _ => format!("Von {} · Nur ansehen", b.owner_name),
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
    let owner = b.role == "owner";
    let menu = |ui: &mut egui::Ui, st: &mut AccountUi, ic: &mut style::Icons| {
        use owndraw_core::icons;
        let mut items = vec![style::MenuItem { icon: icons::EXPORT, label: "Öffnen", danger: false }];
        if owner {
            items.push(style::MenuItem { icon: icons::TEXT, label: "Umbenennen", danger: false });
            items.push(style::MenuItem { icon: icons::TRASH, label: "Löschen", danger: true });
        }
        match style::card_menu(ui, ic, &items) {
            Some(0) => api::open_board(&b.id),
            Some(1) => st.dlg = Some(Dlg::Rename { id: b.id.clone(), name: b.name.clone() }),
            Some(2) => st.dlg = Some(Dlg::Delete { id: b.id.clone(), name: b.name.clone() }),
            _ => {}
        }
    };
    resp.context_menu(|ui| menu(ui, st, &mut sh.icons));
    let mr = Rect::from_center_size(Pos2::new(r.right() - 22.0, r.top() + 72.0), Vec2::splat(36.0));
    let mresp = ui.interact(mr, resp.id.with("more"), Sense::click());
    if mresp.hovered() {
        ui.painter().circle_filled(mr.center(), 16.0, c(p.hover));
    }
    sh.icons.paint(ui, mr, owndraw_core::icons::MORE, p.dim, false, 18.0);
    egui::Popup::menu(&mresp).align(egui::RectAlign::BOTTOM_END).show(|ui| menu(ui, st, &mut sh.icons));
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
    // Einladung aus dem Link hat Vorrang; während Anmelden/Registrieren bleibt sie nur vorgemerkt
    if st.dlg.is_none() && api::with(|n| n.invite.is_some()) {
        invite_dialog(ctx, sh, st);
        return;
    }
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
                ui.label(egui::RichText::new(if *register { "Konto erstellen" } else { "Anmelden" }).heading().strong());
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
                        if (style::primary_btn(ui, if *register { "Konto erstellen" } else { "Anmelden" }, h).clicked() || enter) && valid {
                            if *register {
                                api::register(email, password, name);
                            } else {
                                api::login(email, password);
                            }
                        }
                    });
                    close |= style::outline_btn(ui, "Abbrechen", h).clicked();
                    if busy {
                        ui.spinner();
                    }
                });
                ui.add_space(4.0);
                let other = if *register { "Schon ein Konto? Anmelden" } else { "Noch kein Konto? Jetzt erstellen" };
                if ui.add(egui::Button::new(egui::RichText::new(other).size(12.5).color(c(p.accent))).fill(egui::Color32::TRANSPARENT)).clicked() {
                    *register = !*register;
                    api::with(|n| n.form_error = None);
                }
            }
            Dlg::Redeem { code, focus } => {
                ui.label(egui::RichText::new("Lizenz einlösen").heading().strong());
                ui.label(egui::RichText::new("Mit einer Lizenz werden deine Boards auf dem Server gespeichert, sind auf allen Geräten verfügbar und lassen sich teilen.").size(12.5).color(c(p.dim)));
                ui.add_space(6.0);
                field(ui, "Lizenzcode", code, false, *focus);
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
                    close |= style::outline_btn(ui, "Abbrechen", h).clicked();
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
                    if (style::primary_btn(ui, "Umbenennen", h).clicked() || enter) && valid {
                        api::rename_board(id, name.trim());
                        close = true;
                    }
                    close |= style::outline_btn(ui, "Abbrechen", h).clicked();
                });
            }
            Dlg::LeaveGuest => {
                ui.label(egui::RichText::new("Gastzugang beenden?").heading().strong());
                ui.label("Deine Gast-ID und der Zugriff auf eingeladene Boards gehen verloren.");
                ui.add_space(10.0);
                ui.allocate_ui_with_layout(egui::vec2(ui.available_width(), h), egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if style::danger_btn(ui, "Gastzugang beenden", h).clicked() {
                        api::logout();
                        close = true;
                    }
                    close |= style::outline_btn(ui, "Abbrechen", h).clicked();
                });
            }
            Dlg::Delete { id, name } => {
                ui.label(egui::RichText::new("Board löschen?").heading().strong());
                ui.label(format!("„{name}“ wird auf dem Server gelöscht. Das gilt auch für alle, mit denen das Board geteilt ist. Der Vorgang lässt sich nicht rückgängig machen."));
                ui.add_space(10.0);
                ui.allocate_ui_with_layout(egui::vec2(ui.available_width(), h), egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if style::danger_btn(ui, "Löschen", h).clicked() {
                        api::delete_board(id);
                        close = true;
                    }
                    close |= style::outline_btn(ui, "Abbrechen", h).clicked();
                });
            }
        }
    });
    if !close {
        st.dlg = Some(d);
    }
}

/// Einladung aus dem Link: Vorschau, Beitreten (angemeldet) bzw. Gastname eingeben oder anmelden.
fn invite_dialog(ctx: &egui::Context, sh: &mut Shared, st: &mut AccountUi) {
    let Some(inv) = api::with(|n| n.invite.clone()) else { return };
    let (server, me, busy, err) = api::with(|n| (n.server, n.me.clone(), n.busy, n.form_error.clone()));
    let h = sh.env.btn;
    let p = style::pal();
    let width = (ctx.content_rect().width() - 24.0).clamp(260.0, 420.0);
    let (enter, esc) = ctx.input(|i| (i.key_pressed(egui::Key::Enter), i.key_pressed(egui::Key::Escape)));
    let mut close = esc;
    let mut login = false;
    let name = &mut st.invite_name;
    egui::Modal::new(egui::Id::unique("invite-dlg")).frame(style::card()).show(ctx, |ui| {
        ui.set_width(width - 40.0);
        ui.label(egui::RichText::new("Einladung").heading().strong());
        ui.add_space(4.0);
        if server == Server::Offline {
            ui.label(egui::RichText::new("Ohne Verbindung zum Server sind Einladungen nicht möglich.").color(c(p.danger_text)));
        } else if let Some(e) = &inv.error {
            ui.label(egui::RichText::new(e).size(13.0).color(c(p.danger_text)));
        } else if let Some(i) = &inv.info {
            let what = if i.role == "edit" { "Du kannst das Board bearbeiten." } else { "Du kannst es nur ansehen." };
            ui.label(format!("{} lädt dich zu „{}“ ein. {what}", i.owner_name, i.board_name));
            ui.add_space(6.0);
            if me.is_none() {
                style::caption(ui, "Dein Name");
                let r = ui.add(egui::TextEdit::singleline(name).char_limit(24).desired_width(f32::INFINITY).font(egui::FontId::proportional(15.0)).margin(Vec2::new(10.0, 8.0)));
                r.request_focus();
                ui.label(egui::RichText::new("Du bekommst eine Gast-ID aus Name und Kennung, zum Beispiel Mila#K7Q2. Sie wird in diesem Browser gespeichert. Ein Konto brauchst du dafür nicht.").size(12.0).color(c(p.dim)));
            }
        } else {
            ui.spinner();
        }
        if let Some(e) = &err {
            ui.label(egui::RichText::new(e).size(12.5).color(c(p.danger_text)));
        }
        ui.add_space(10.0);
        ui.allocate_ui_with_layout(egui::vec2(ui.available_width(), h), egui::Layout::right_to_left(egui::Align::Center), |ui| {
            if inv.info.is_some() && server != Server::Offline {
                let valid = !busy && (me.is_some() || !name.trim().is_empty());
                ui.add_enabled_ui(valid, |ui| {
                    let label = if me.is_some() { "Beitreten" } else { "Als Gast beitreten" };
                    if (style::primary_btn(ui, label, h).clicked() || enter) && valid {
                        api::join_invite(&inv.token, name);
                    }
                });
                if me.is_none() {
                    login = style::plain_btn(ui, "Anmelden", h).clicked();
                }
            }
            close |= style::outline_btn(ui, "Abbrechen", h).clicked();
            if busy {
                ui.spinner();
            }
        });
    });
    if login {
        api::with(|n| n.form_error = None);
        st.dlg = Some(Dlg::Login { register: false, email: String::new(), password: String::new(), name: String::new(), focus: true });
    }
    if close {
        api::with(|n| {
            n.invite = None;
            n.form_error = None;
        });
    }
}
