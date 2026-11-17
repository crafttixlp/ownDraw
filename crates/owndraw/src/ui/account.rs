//! Konto auf der Startseite (Web-App mit Server und Desktop): Anmelden/Registrieren/Abmelden, Lizenz einlösen und
//! -status, Liste der Boards am Server (eigene + geteilte) mit Öffnen/Umbenennen/Löschen. Ohne Server
//! (statisches Hosting) bleibt der Gastmodus-Hinweis. Desktop: Server-Adresse im Anmeldedialog, Einladungslink
//! einfügen statt Link im Browser öffnen, ohne Verbindung bleibt alles Lokale nutzbar.

use crate::app::Shared;
use crate::layout::FormFactor;
use crate::net::{self as api, BoardInfo, Server};
use crate::style::{self, c};
use crate::ui::{local_in, wd};
use egui::{Align2, CornerRadius, Pos2, Rect, Sense, Stroke, Vec2};

enum Dlg {
    /// `server`: Desktop, Adresse des Servers (Web-App: immer die eigene Origin).
    Login { register: bool, email: String, password: String, name: String, server: String, focus: bool },
    /// Desktop: Einladungslink einfügen.
    InviteLink { link: String, focus: bool },
    Redeem { code: String, focus: bool },
    Rename { id: String, name: String },
    Delete { id: String, name: String },
    /// Gastzugang beenden (Bestätigung).
    LeaveGuest,
    /// Lokales Board auf den Server verschieben (Bestätigung, Größe in Bytes).
    ToServer { file: crate::platform::FileRef, name: String, size: i64 },
    /// Server-Board lokal speichern; `owner` = verschieben (am Server löschen), sonst nur Kopie.
    ToLocal { id: String, name: String, owner: bool },
}

/// Kartenmenü eines lokalen Boards: „Auf den Server verschieben“ (Bestätigungsdialog öffnen).
pub fn ask_to_server(sh: &mut Shared, st: &mut AccountUi, file: &crate::platform::FileRef, name: &str) {
    match sh.platform.read(file) {
        Ok(b) => st.dlg = Some(Dlg::ToServer { file: file.clone(), name: name.to_string(), size: b.len() as i64 }),
        Err(e) => sh.toast_for(&format!("Board nicht lesbar: {e}"), 4.0),
    }
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

/// Desktop: Server-Adresse ohne Schema (für Hinweise).
fn host() -> String {
    #[cfg(not(target_arch = "wasm32"))]
    {
        let u = api::base_url();
        u.split_once("://").map(|(_, r)| r.to_string()).unwrap_or(u)
    }
    #[cfg(target_arch = "wasm32")]
    String::new()
}

/// Vorbelegung des Server-Felds im Anmeldedialog.
fn server_text() -> String {
    #[cfg(not(target_arch = "wasm32"))]
    return api::base_url();
    #[cfg(target_arch = "wasm32")]
    String::new()
}

/// Demo/Screenshots: Dialog `login` oder `einladung` öffnen.
pub fn demo_dialog(st: &mut AccountUi, which: &str) {
    st.dlg = Some(match which {
        "einladung" => Dlg::InviteLink { link: String::new(), focus: true },
        _ => Dlg::Login { register: false, email: String::new(), password: String::new(), name: String::new(), server: server_text(), focus: true },
    });
}

/// Zerlegt einen Einladungslink (`https://server/#invite=<token>`, auch nur das Token) in Server und Token.
pub fn parse_invite_link(link: &str) -> Option<(Option<String>, String)> {
    let link = link.trim();
    let (base, token) = match link.split_once("#invite=") {
        Some((b, t)) => {
            let b = b.trim_end_matches('/');
            (if b.is_empty() { None } else { Some(b.to_string()) }, t)
        }
        None if !link.contains('/') && !link.contains(' ') => (None, link),
        None => return None,
    };
    let token = token.trim();
    (!token.is_empty()).then(|| (base, token.to_string()))
}

fn mb(v: f64) -> String {
    format!("{v:.1}").replace('.', ",")
}

/// Hinweis-/Kontokarte unter dem Kopf der Startseite.
pub fn header(ui: &mut egui::Ui, sh: &mut Shared, st: &mut AccountUi) {
    let compact = sh.env.ff == FormFactor::Compact;
    let h = if compact { sh.env.btn } else { 36.0 };
    let (server, me) = api::with(|n| (n.server, n.me.clone()));
    let login = |register: bool| Dlg::Login { register, email: String::new(), password: String::new(), name: String::new(), server: server_text(), focus: true };
    // Desktop: Einladungslink einfügen (im Browser öffnet der Link die Web-App direkt)
    let invite_btn = |ui: &mut egui::Ui, st: &mut AccountUi| {
        if !crate::platform::WEB && style::outline_btn(ui, "Einladungslink …", h).clicked() {
            api::with(|n| n.form_error = None);
            st.dlg = Some(Dlg::InviteLink { link: String::new(), focus: true });
        }
    };
    if !crate::platform::WEB && !api::started() {
        return;
    }
    match (server, me) {
        (Server::Ready, Some(me)) if me.guest => {
            let title = format!("Gast-ID: {}", me.name);
            super::start::banner(ui, compact, &title, "Als Gast siehst du nur Boards, zu denen du eingeladen wurdest. Mit einem Konto kannst du eigene Boards speichern.", |ui| {
                if style::primary_btn(ui, "Konto erstellen", h).clicked() {
                    api::with(|n| n.form_error = None);
                    st.dlg = Some(login(true));
                }
                invite_btn(ui, st);
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
                    let wo = if sh.settings.new_on_server { "auf dem Server" } else { local_in() };
                    format!("Lizenz {until}. {} von {} MB Speicher belegt. Neue Boards werden standardmäßig {wo} gespeichert. Beim Erstellen und im Kartenmenü kannst du den Speicherort wählen.", mb(l.used_mb), mb(l.quota_mb))
                }
                Some(l) => format!(
                    "Lizenz abgelaufen{}. Deine Boards auf dem Server sind nur noch lesbar. Löse einen neuen Code ein, um wieder zu speichern.",
                    l.expires_at.map(|e| format!(" am {}", date(e))).unwrap_or_default()
                ),
                None => wd(
                    "Keine Lizenz. Neue Boards werden nur in diesem Browser gespeichert. Lade sie herunter, um sie zu sichern. Mit einer Lizenz speicherst du auf dem Server. Einladungen zu Boards anderer funktionieren auch ohne Lizenz.",
                    "Keine Lizenz. Neue Boards werden auf diesem Computer gespeichert. Mit einer Lizenz speicherst und teilst du Boards auf dem Server. Einladungen zu Boards anderer funktionieren auch ohne Lizenz.",
                )
                .into(),
            };
            let licensed = me.licensed();
            let title = if crate::platform::WEB { format!("Angemeldet als {who}") } else { format!("Angemeldet als {who} · {}", host()) };
            super::start::banner(ui, compact, &title, &line, |ui| {
                if !licensed && style::primary_btn(ui, "Lizenz einlösen", h).clicked() {
                    api::with(|n| n.form_error = None);
                    st.dlg = Some(Dlg::Redeem { code: String::new(), focus: true });
                }
                invite_btn(ui, st);
                if style::outline_btn(ui, "Abmelden", h).clicked() {
                    api::logout();
                }
            });
        }
        (Server::Ready, None) => {
            super::start::banner(
                ui,
                compact,
                wd("Du bist nicht angemeldet. Boards werden nur in diesem Browser gespeichert.", "Du bist nicht angemeldet. Boards werden auf diesem Computer gespeichert."),
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
                    invite_btn(ui, st);
                },
            );
        }
        // Desktop: Server noch nicht geantwortet bzw. nicht erreichbar – lokal geht alles weiter
        (Server::Unknown, _) if !crate::platform::WEB => {
            super::start::banner(ui, compact, &format!("Verbinde mit {} …", host()), "Konto, Boards auf dem Server und Live-Zusammenarbeit werden geladen.", |ui| {
                ui.spinner();
            });
        }
        (Server::Offline, _) if !crate::platform::WEB => {
            let title = format!("Server {} nicht erreichbar", host());
            super::start::banner(ui, compact, &title, "Du arbeitest offline. Alle Boards auf diesem Computer bleiben voll nutzbar. Konto, Boards auf dem Server und Live-Zusammenarbeit gehen wieder, sobald der Server erreichbar ist.", |ui| {
                if style::primary_btn(ui, "Erneut versuchen", h).clicked() {
                    api::retry();
                }
                if style::outline_btn(ui, "Anderer Server …", h).clicked() {
                    api::with(|n| n.form_error = None);
                    st.dlg = Some(login(false));
                }
                invite_btn(ui, st);
            });
        }
        _ => {
            super::start::banner(ui, compact, "Du bist nicht angemeldet.", "Boards werden nicht gespeichert. Lade sie herunter, um sie zu sichern.", |_| {});
        }
    }
}

/// Abschnitt „Am Server“ (nur angemeldet, mit Lizenz oder geteilten Boards). Liefert, ob er angezeigt wurde.
pub fn boards(ui: &mut egui::Ui, sh: &mut Shared, st: &mut AccountUi) -> bool {
    let (me, list, loaded, opening) = api::with(|n| (n.me.clone(), n.boards.clone(), n.boards_loaded, n.opening || n.moving));
    let Some(me) = me else { return false };
    if !me.licensed() && list.is_empty() {
        return false;
    }
    let p = style::pal();
    let compact = sh.env.ff == FormFactor::Compact;
    ui.add_space(style::sp::XL);
    ui.horizontal(|ui| {
        ui.label(egui::RichText::new("Auf dem Server").size(style::fs::SECTION + 3.0).strong());
        if opening {
            ui.spinner();
        }
    });
    ui.add_space(style::sp::S);
    if !loaded {
        ui.spinner();
        return true;
    }
    if list.is_empty() {
        ui.label(egui::RichText::new(wd("Noch keine Boards auf dem Server. Erstelle ein neues oder verschiebe eins aus dem Browser (Kartenmenü „…“).", "Noch keine Boards auf dem Server. Erstelle ein neues oder verschiebe eins von diesem Computer (Kartenmenü „…“).")).color(c(p.dim)));
        return true;
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
    true
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
    // Wolken-Symbol als Icon (das Zeichen „☁“ fehlt in der Schrift)
    sh.icons.paint(ui, Rect::from_center_size(head.left_center() + Vec2::new(24.0, 0.0), Vec2::splat(24.0)), owndraw_core::icons::CLOUD, p.accent, false, 20.0);
    let pt = ui.painter();
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
        #[derive(Clone, Copy)]
        enum A {
            Open,
            Rename,
            Local,
            Delete,
        }
        let mut acts = vec![(A::Open, style::MenuItem { icon: icons::EXPORT, label: "Öffnen", danger: false })];
        if owner {
            acts.push((A::Rename, style::MenuItem { icon: icons::TEXT, label: "Umbenennen", danger: false }));
            acts.push((A::Local, style::MenuItem { icon: icons::COPY, label: wd("In Browser verschieben", "Auf diesen Computer verschieben"), danger: false }));
            acts.push((A::Delete, style::MenuItem { icon: icons::TRASH, label: "Löschen", danger: true }));
        } else {
            acts.push((A::Local, style::MenuItem { icon: icons::COPY, label: wd("Kopie im Browser", "Kopie auf diesem Computer"), danger: false }));
        }
        let items: Vec<style::MenuItem> = acts.iter().map(|a| style::MenuItem { icon: a.1.icon, label: a.1.label, danger: a.1.danger }).collect();
        match style::card_menu(ui, ic, &items).map(|i| acts[i].0) {
            Some(A::Open) => api::open_board(&b.id),
            Some(A::Rename) => st.dlg = Some(Dlg::Rename { id: b.id.clone(), name: b.name.clone() }),
            Some(A::Local) => st.dlg = Some(Dlg::ToLocal { id: b.id.clone(), name: b.name.clone(), owner }),
            Some(A::Delete) => st.dlg = Some(Dlg::Delete { id: b.id.clone(), name: b.name.clone() }),
            None => {}
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
    // Sperrmeldung (siehe `blocked_dialog`) ersetzt offene Konto-Dialoge, z. B. „Anmelden“
    if api::with(|n| n.blocked.is_some()) {
        st.dlg = None;
        return;
    }
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
    egui::Modal::new(egui::Id::unique("account-dlg")).frame(style::card().inner_margin(egui::Margin::same(style::sp::XL as i8))).show(ctx, |ui| {
        ui.set_width(width - 2.0 * style::sp::XL - 2.0);
        ui.spacing_mut().item_spacing.y = style::sp::S;
        match &mut d {
            Dlg::Login { register, email, password, name, server, focus } => {
                ui.label(egui::RichText::new(if *register { "Konto erstellen" } else { "Anmelden" }).size(style::fs::SECTION + 5.0).strong());
                ui.add_space(6.0);
                if !crate::platform::WEB {
                    field(ui, "Server", server, false, false);
                }
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
                            #[cfg(not(target_arch = "wasm32"))]
                            if !server.trim().is_empty() && server.trim().trim_end_matches('/') != api::base_url() {
                                api::change_server(server);
                                sh.settings.server_url = api::base_url();
                                *server = api::base_url();
                            }
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
            Dlg::InviteLink { link, focus } => {
                ui.label(egui::RichText::new("Einladung öffnen").size(style::fs::SECTION + 5.0).strong());
                ui.label(egui::RichText::new("Füge den Einladungslink ein, den du bekommen hast. Danach siehst du, wer dich zu welchem Board einlädt.").size(12.5).color(c(p.dim)));
                ui.add_space(6.0);
                field(ui, "Einladungslink", link, false, *focus);
                *focus = false;
                let parsed = parse_invite_link(link);
                #[cfg(not(target_arch = "wasm32"))]
                if let Some((Some(b), _)) = &parsed {
                    if b.trim_end_matches('/') != api::base_url() {
                        ui.label(egui::RichText::new(format!("Der Link gehört zu {b}. OwnDraw verbindet sich dafür mit diesem Server und meldet dich beim bisherigen ab.")).size(12.5).color(c(p.dim)));
                    }
                }
                if !link.trim().is_empty() && parsed.is_none() {
                    ui.label(egui::RichText::new("Das sieht nicht wie ein Einladungslink aus. Er enthält „#invite=“.").size(12.5).color(c(p.danger_text)));
                }
                ui.add_space(10.0);
                ui.allocate_ui_with_layout(egui::vec2(ui.available_width(), h), egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    ui.add_enabled_ui(parsed.is_some(), |ui| {
                        if (style::primary_btn(ui, "Weiter", h).clicked() || enter) && parsed.is_some() {
                            if let Some((_base, token)) = parsed.clone() {
                                #[cfg(not(target_arch = "wasm32"))]
                                if let Some(b) = _base.filter(|b| b.trim_end_matches('/') != api::base_url()) {
                                    api::change_server(&b);
                                    sh.settings.server_url = api::base_url();
                                }
                                api::invite_info(&token);
                                close = true;
                            }
                        }
                    });
                    close |= style::outline_btn(ui, "Abbrechen", h).clicked();
                });
            }
            Dlg::Redeem { code, focus } => {
                ui.label(egui::RichText::new("Lizenz einlösen").size(style::fs::SECTION + 5.0).strong());
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
                ui.label(egui::RichText::new("Board umbenennen").size(style::fs::SECTION + 5.0).strong());
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
                ui.label(egui::RichText::new("Gastzugang beenden?").size(style::fs::SECTION + 5.0).strong());
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
            Dlg::ToServer { file, name, size } => {
                ui.label(egui::RichText::new("Auf den Server verschieben?").size(style::fs::SECTION + 5.0).strong());
                let free = api::free_bytes();
                let fits = free.is_some_and(|f| *size <= f);
                ui.label(if crate::platform::WEB {
                    format!("„{name}“ wird auf den Server hochgeladen und danach aus diesem Browser entfernt. Erst wenn der Server das Board gespeichert hat, wird es hier entfernt. So gehen keine Daten verloren.")
                } else {
                    format!("„{name}“ wird auf den Server hochgeladen. Danach öffnest du es über „Auf dem Server“. Die Datei auf diesem Computer bleibt als Sicherung erhalten, wird aber aus „Zuletzt geöffnet“ entfernt.")
                });
                ui.add_space(4.0);
                let used = format!("Größe: {} · Frei auf dem Server: {}", super::mb_text(*size), free.map(super::mb_text).unwrap_or_else(|| "–".into()));
                ui.label(egui::RichText::new(used).size(12.5).color(c(p.dim)));
                ui.label(egui::RichText::new("Auf dem Server ist das Board auf allen Geräten verfügbar und du kannst es teilen und live gemeinsam bearbeiten.").size(12.5).color(c(p.dim)));
                if !fits {
                    ui.add_space(4.0);
                    let msg = if free.is_none() { "Für Boards auf dem Server brauchst du eine gültige Lizenz.".to_string() } else { format!("Nicht genug Speicherplatz auf dem Server. Lösche Boards auf dem Server oder lass dieses Board {}.", local_in()) };
                    ui.add(egui::Label::new(egui::RichText::new(msg).size(12.5).color(c(p.danger_text))).wrap());
                }
                ui.add_space(10.0);
                ui.allocate_ui_with_layout(egui::vec2(ui.available_width(), h), egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    ui.add_enabled_ui(fits, |ui| {
                        if style::primary_btn(ui, "Verschieben", h).clicked() {
                            match sh.platform.read(file) {
                                Ok(b) => api::move_to_server(name, b, file.clone()),
                                Err(e) => sh.toast_for(&format!("Board nicht lesbar: {e}"), 4.0),
                            }
                            close = true;
                        }
                    });
                    close |= style::outline_btn(ui, "Abbrechen", h).clicked();
                });
            }
            Dlg::ToLocal { id, name, owner, .. } => {
                if *owner {
                    ui.label(egui::RichText::new(wd("In diesem Browser speichern?", "Auf diesem Computer speichern?")).size(style::fs::SECTION + 5.0).strong());
                    ui.label(format!("„{name}“ wird {} gespeichert und danach vom Server gelöscht. Erst wenn es hier gespeichert ist, wird es am Server gelöscht.", local_in()));
                    ui.add_space(4.0);
                    ui.add(egui::Label::new(egui::RichText::new(format!("Danach ist es nur noch {}. Teilen und Live-Zusammenarbeit gehen nur mit Boards auf dem Server: Wer das Board mit dir geteilt bekommen hat, verliert den Zugriff. Einladungslinks werden ungültig.", local_in())).size(12.5).color(c(p.dim))).wrap());
                } else {
                    ui.label(egui::RichText::new(wd("Kopie im Browser speichern?", "Kopie auf diesem Computer speichern?")).size(style::fs::SECTION + 5.0).strong());
                    ui.label(format!("Eine Kopie von „{name}“ wird {} gespeichert. Das Board auf dem Server bleibt unverändert. Änderungen an der Kopie sieht niemand sonst.", local_in()));
                }
                ui.add_space(10.0);
                ui.allocate_ui_with_layout(egui::vec2(ui.available_width(), h), egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if style::primary_btn(ui, if *owner { "Verschieben" } else { "Kopie speichern" }, h).clicked() {
                        api::fetch_to_local(id, name, *owner);
                        close = true;
                    }
                    close |= style::outline_btn(ui, "Abbrechen", h).clicked();
                });
            }
            Dlg::Delete { id, name } => {
                ui.label(egui::RichText::new("Board löschen?").size(style::fs::SECTION + 5.0).strong());
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

/// Meldung „Konto gesperrt“ (mit Nachricht des Admins); über allem, auf jeder Seite. Der Nutzer ist danach abgemeldet.
pub fn blocked_dialog(ctx: &egui::Context, sh: &Shared) {
    let Some(msg) = api::with(|n| n.blocked.clone()) else { return };
    let h = sh.env.btn;
    let p = style::pal();
    let width = (ctx.content_rect().width() - 24.0).clamp(260.0, 420.0);
    let mut close = ctx.input(|i| i.key_pressed(egui::Key::Enter) || i.key_pressed(egui::Key::Escape));
    egui::Modal::new(egui::Id::unique("blocked-dlg")).frame(style::card().inner_margin(egui::Margin::same(style::sp::XL as i8))).show(ctx, |ui| {
        ui.set_width(width - 2.0 * style::sp::XL - 2.0);
        ui.spacing_mut().item_spacing.y = style::sp::S;
        ui.label(egui::RichText::new("Dein Konto ist gesperrt").size(style::fs::SECTION + 5.0).strong());
        ui.label("Ein Administrator hat dein Konto gesperrt. Du wurdest abgemeldet und kannst dich vorerst nicht mehr anmelden.");
        if !msg.is_empty() {
            ui.add_space(4.0);
            style::caption(ui, "Nachricht vom Administrator");
            style::info_card(ui, |ui| {
                ui.label(egui::RichText::new(&msg).color(c(p.text)));
            });
        }
        ui.add_space(10.0);
        ui.allocate_ui_with_layout(egui::vec2(ui.available_width(), h), egui::Layout::right_to_left(egui::Align::Center), |ui| {
            close |= style::primary_btn(ui, "OK", h).clicked();
        });
    });
    if close {
        api::with(|n| n.blocked = None);
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
    egui::Modal::new(egui::Id::unique("invite-dlg")).frame(style::card().inner_margin(egui::Margin::same(style::sp::XL as i8))).show(ctx, |ui| {
        ui.set_width(width - 2.0 * style::sp::XL - 2.0);
        ui.spacing_mut().item_spacing.y = style::sp::S;
        ui.label(egui::RichText::new("Einladung").size(style::fs::SECTION + 5.0).strong());
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
                ui.label(egui::RichText::new(wd("Du bekommst eine Gast-ID aus Name und Kennung, zum Beispiel Mila#K7Q2. Sie wird in diesem Browser gespeichert. Ein Konto brauchst du dafür nicht.", "Du bekommst eine Gast-ID aus Name und Kennung, zum Beispiel Mila#K7Q2. Sie wird auf diesem Computer gespeichert. Ein Konto brauchst du dafür nicht.")).size(12.0).color(c(p.dim)));
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
        st.dlg = Some(Dlg::Login { register: false, email: String::new(), password: String::new(), name: String::new(), server: server_text(), focus: true });
    }
    if close {
        api::with(|n| {
            n.invite = None;
            n.form_error = None;
        });
    }
}
