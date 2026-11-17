//! Werkzeug „Umwandeln“ (Handschrift-Erkennung): Striche per Lasso umkreisen oder antippen, erkennen lassen
//! und durch Text bzw. Formel ersetzen.
//!
//! - Lasso/Antippen: [`super::extras`] sammelt den Weg; beim Loslassen wählt das Board die Striche
//!   (`Board::ink_in_lasso`, `Board::ink_at`: Strich + Nachbarn derselben Zeile).
//! - Die Striche werden als Graustufenbild gerendert (`owndraw_core::hw::render`) und im Browser erkannt
//!   (`crate::hw`, je Art ein Modell-Paket der eingestellten Stufe). Die Alternativen bewertet der Kern mit dem
//!   Wissen der Seite neu (bekannte Variablen, Einheiten, Wörter) und wählt den Reiter vor.
//! - Fenster: Reiter Text / Formel / Zahl, Alternativen (Formeln gerendert), Korrekturfeld (Formel als LaTeX oder
//!   Rechen-Klartext), „Ersetzen“ (ein Rückgängig-Schritt, `Board::replace_ink`) und „Abbrechen“.

use super::Editor;
use crate::app::Shared;
use crate::hw;
use crate::layout::min_touch;
use crate::style::{self, c};
use egui::{Align2, Color32, Order, Pos2, Rect, Stroke, Vec2};
use owndraw_core::board::Ersatz;
use owndraw_core::canvas::Canvas;
use owndraw_core::hw::{self as core_hw, Art, Ink, Kandidat, Kontext, Stufe};
use owndraw_core::tiny_skia::Pixmap;
use owndraw_core::{calc, item, math};

/// Alternativen je Art.
const NBEST: usize = 5;

/// Stand einer Art (Reiter).
#[derive(Clone, Debug)]
enum Res {
    /// Wartet auf Paketliste bzw. Laufzeit.
    Pending,
    /// Läuft (Auftragsnummer, Paket).
    Run(u32, String),
    /// Fertig: Alternativen (neu bewertet), Rechenzeit.
    Done(Vec<Kandidat>, u32),
    /// Fehler bei der Erkennung.
    Err(String),
    /// Kein Modell (nicht auf dem Server oder nicht geladen).
    Missing(String),
}

pub struct Conv {
    pub ids: Vec<u64>,
    ink: Ink,
    ctx: Kontext,
    stufe: Stufe,
    pub tab: Art,
    /// Reiter noch nicht von Hand gewählt (folgt der Vorauswahl).
    auto_tab: bool,
    res: [Res; 3],
    pick: [usize; 3],
    edit: [String; 3],
    /// Korrekturfeld von Hand geändert (Alternativen überschreiben es dann nicht mehr).
    touched: [bool; 3],
    /// Formel als Rechen-Klartext bearbeiten (sonst LaTeX).
    plain: bool,
    /// Gerenderte Formeln: (LaTeX, Farbe, Pixel je Punkt) → Textur und Größe.
    tex: Vec<(String, u32, f32, egui::TextureHandle, Vec2)>,
    started: f64,
    focus: bool,
    /// Fensterhöhe im letzten Bild (für die Platzierung).
    last_h: f32,
}

fn ai(a: Art) -> usize {
    match a {
        Art::Text => 0,
        Art::Formel => 1,
        Art::Zahl => 2,
    }
}

/// Lexikon (Einheiten) des Pakets.
fn lexikon(m: &core_hw::Manifest) -> Vec<String> {
    m.raw.get("lexikon").and_then(|x| x.as_array()).map(|a| a.iter().filter_map(|x| x.as_str().map(String::from)).collect()).unwrap_or_default()
}

/// Text eines Ergebnisses für das Korrekturfeld (Formel als LaTeX bzw. Klartext).
fn field_text(art: Art, k: &str, plain: bool) -> String {
    match art {
        Art::Formel if plain => calc::to_plain(k),
        Art::Zahl => core_hw::zahl_klartext(k),
        _ => k.to_string(),
    }
}

impl Conv {
    fn new(ids: Vec<u64>, ink: Ink, ctx: Kontext, stufe: Stufe, now: f64) -> Conv {
        Conv {
            ids,
            ink,
            ctx,
            stufe,
            tab: Art::Text,
            auto_tab: true,
            res: [Res::Pending, Res::Pending, Res::Pending],
            pick: [0; 3],
            edit: Default::default(),
            touched: [false; 3],
            plain: false,
            tex: Vec::new(),
            started: now,
            focus: false,
            last_h: 0.0,
        }
    }

    /// Startet fehlende Aufträge, holt Ergebnisse ab. true = es läuft noch etwas.
    fn step(&mut self) -> bool {
        let Some(info) = hw::info() else {
            for r in self.res.iter_mut() {
                *r = Res::Err("Die Erkennung wird noch geladen …".into());
            }
            return true;
        };
        if info.liste == "leer" {
            hw::refresh_list();
            return true;
        }
        if info.liste == "laedt" {
            return true;
        }
        if info.liste == "fehler" {
            for r in self.res.iter_mut() {
                if !matches!(r, Res::Done(..)) {
                    *r = Res::Missing(format!("Modelle nicht abrufbar: {}", info.fehler));
                }
            }
            return false;
        }
        let ms = info.manifests();
        let rt_ok = core_hw::laufzeit(&ms).map(|m| info.paket(&m.id).is_some_and(|p| p.da()));
        let mut busy = false;
        // Zahl zuerst (schnell), dann Formel und Text
        for art in [Art::Zahl, Art::Formel, Art::Text] {
            let i = ai(art);
            match &self.res[i] {
                Res::Pending | Res::Missing(_) => {
                    let Some(m) = core_hw::paket_fuer(&ms, art, self.stufe) else {
                        self.res[i] = Res::Missing(format!("Auf dem Server gibt es kein Modell für {}.", art.label()));
                        continue;
                    };
                    let da = info.paket(&m.id).is_some_and(|p| p.da());
                    if rt_ok != Some(true) || !da {
                        let laedt = info.paket(&m.id).is_some_and(|p| p.zustand == "laedt");
                        busy |= laedt;
                        self.res[i] = Res::Missing(if rt_ok.is_none() { "Auf dem Server fehlt die Laufzeit (onnxruntime-web).".into() } else { format!("Modell „{}“ ist noch nicht geladen.", m.name) });
                        continue;
                    }
                    match hw::start(&m.id, NBEST, &self.ink) {
                        Some(job) => self.res[i] = Res::Run(job, m.id.clone()),
                        None => self.res[i] = Res::Err("Erkennung konnte nicht starten.".into()),
                    }
                    busy = true;
                }
                Res::Run(job, id) => match hw::poll(*job) {
                    None => busy = true,
                    Some(Err(e)) => self.res[i] = Res::Err(e),
                    Some(Ok((k, ms_))) => {
                        let lex = ms.iter().find(|m| m.id == *id).map(lexikon).unwrap_or_default();
                        let k = core_hw::neu_bewerten(art, k, &self.ctx, &lex);
                        if !self.touched[i] {
                            self.edit[i] = k.first().map(|b| field_text(art, &b.text, self.plain)).unwrap_or_default();
                            self.pick[i] = 0;
                        }
                        self.res[i] = Res::Done(k, ms_);
                    }
                },
                _ => {}
            }
        }
        if self.auto_tab {
            let best = |a: Art| match &self.res[ai(a)] {
                Res::Done(k, _) => k.first(),
                _ => None,
            };
            let all_done = self.res.iter().all(|r| !matches!(r, Res::Run(..) | Res::Pending));
            // erst mit einem Ergebnis entscheiden (fehlen noch Modelle, wird nach dem Laden gewählt)
            if self.res.iter().any(|r| matches!(r, Res::Done(..))) {
                self.tab = core_hw::vorauswahl(&[(Art::Text, best(Art::Text)), (Art::Formel, best(Art::Formel)), (Art::Zahl, best(Art::Zahl))]);
                if all_done {
                    self.auto_tab = false;
                    self.focus = true;
                }
            }
        }
        busy
    }

    /// Ersatz für die Striche aus dem Korrekturfeld des aktiven Reiters.
    fn ersatz(&self) -> Option<Ersatz> {
        let t = self.edit[ai(self.tab)].trim();
        if t.is_empty() {
            return None;
        }
        Some(match self.tab {
            Art::Text => Ersatz::Text(t.to_string()),
            Art::Formel if self.plain => Ersatz::Formel(calc::to_latex(t)),
            Art::Formel => Ersatz::Formel(t.to_string()),
            Art::Zahl => Ersatz::Formel(calc::to_latex(&core_hw::zahl_klartext(t))),
        })
    }

    /// Formel als Textur (zwischengespeichert).
    fn math_tex(&mut self, ctx: &egui::Context, src: &str, fg: u32, max_w: f32, size: f32) -> Option<(egui::TextureId, Vec2)> {
        let ppp = ctx.pixels_per_point();
        if let Some(t) = self.tex.iter().find(|t| t.0 == src && t.1 == fg && t.2 == ppp) {
            return Some((t.3.id(), t.4));
        }
        let f = math::layout(src, size);
        let scale = (ppp * max_w / f.w.max(1.0)).min(ppp).max(0.05);
        let (pw, ph) = ((f.w * scale).ceil().max(1.0) as u32, (f.h * scale).ceil().max(1.0) as u32);
        let mut pm = Pixmap::new(pw.min(4096), ph.min(4096))?;
        {
            let mut cv = Canvas::new(&mut pm);
            cv.scale(scale, scale);
            f.draw(&mut cv, fg);
        }
        let img = egui::ColorImage::from_rgba_premultiplied([pm.width() as usize, pm.height() as usize], pm.data());
        let t = ctx.load_texture("hw-formel", img, egui::TextureOptions::LINEAR);
        let sz = Vec2::new(pm.width() as f32 / ppp, pm.height() as f32 / ppp);
        let id = t.id();
        if self.tex.len() > 24 {
            drop(self.tex.remove(0));
        }
        self.tex.push((src.to_string(), fg, ppp, t, sz));
        Some((id, sz))
    }
}

impl Editor {
    /// Auswahl des Werkzeugs fertig (Lasso oder Antippen): Fenster öffnen und Erkennung starten.
    pub(super) fn convert_pick(&mut self, sh: &mut Shared, ids: Vec<u64>) {
        if ids.is_empty() {
            sh.toast("Hier ist keine Handschrift. Umkreise Striche oder tippe auf sie.");
            return;
        }
        let stufe = sh.settings.hw.stufe();
        if stufe == Stufe::Aus {
            sh.toast_for("Die Handschrift-Erkennung ist aus (Einstellungen → Handschrift-Erkennung).", 4.0);
            return;
        }
        if !hw::SUPPORTED {
            sh.toast_for("Handschrift-Erkennung ist nur im Browser verfügbar (Web-App).", 4.0);
            return;
        }
        let Some(ink) = self.board.render_ink(&ids) else { return };
        let ctx = self.board.hw_context(&ids);
        self.conv = Some(Conv::new(ids, ink, ctx, stufe, sh.now));
        self.board.invalidate();
    }

    /// Abnahme-Demo (demo.rs): Fenster für die Striche `ids` mit vorgegebenen Ergebnissen.
    pub fn demo_convert(&mut self, ids: Vec<u64>, results: &[(Art, Vec<(&str, f32)>)], tab: Art) {
        let Some(ink) = self.board.render_ink(&ids) else { return };
        let ctx = self.board.hw_context(&ids);
        let mut cv = Conv::new(ids, ink, ctx, Stufe::Mittel, 0.0);
        for (art, ks) in results {
            let k = core_hw::neu_bewerten(*art, ks.iter().map(|(t, s)| Kandidat::new(t, *s)).collect(), &cv.ctx, &[]);
            cv.edit[ai(*art)] = k.first().map(|b| field_text(*art, &b.text, false)).unwrap_or_default();
            cv.res[ai(*art)] = Res::Done(k, 640);
        }
        cv.tab = tab;
        cv.auto_tab = false;
        self.conv = Some(cv);
    }

    /// Fenster schließen; `ok` ersetzt die Striche.
    fn finish_convert(&mut self, sh: &mut Shared, ok: bool) {
        let Some(cv) = self.conv.take() else { return };
        if ok {
            if let Some(e) = cv.ersatz() {
                match self.board.replace_ink(&cv.ids, &e) {
                    Some(_) => sh.toast("Umgewandelt. Rückgängig stellt die Striche wieder her."),
                    None => sh.toast("Ersetzen nicht möglich (Ebene gesperrt?)"),
                }
            }
        }
        self.board.invalidate();
    }

    /// Lasso und Auswahl des Werkzeugs zeichnen (über dem Board).
    pub(super) fn convert_paint(&self, pt: &egui::Painter, rect: Rect) {
        let b = &self.board;
        let w2s = |x: f32, y: f32| rect.min + Vec2::new(x * b.sc + b.tx, y * b.sc + b.ty);
        let acc = c(style::pal().accent);
        if let Some((_, pts)) = &self.ext.lasso {
            let v: Vec<Pos2> = pts.iter().map(|(x, y)| w2s(*x, *y)).collect();
            if v.len() > 1 {
                pt.add(egui::Shape::dashed_line(&v, Stroke::new(2.0, acc), 6.0, 4.0));
            }
        }
        if let Some(cv) = &self.conv {
            if let Some(r) = b.bounds_of(&cv.ids) {
                let rr = Rect::from_min_max(w2s(r.left, r.top), w2s(r.right, r.bottom)).expand(6.0);
                pt.rect_filled(rr, 6.0, acc.gamma_multiply(0.08));
                pt.rect_stroke(rr, 6.0, Stroke::new(1.5, acc), egui::StrokeKind::Outside);
            }
        }
    }

    /// Je Frame: neue Auswahl übernehmen, Erkennung abfragen, Fenster zeichnen.
    pub(super) fn convert_frame(&mut self, ctx: &egui::Context, sh: &mut Shared) {
        if let Some(ids) = self.ext.convert_pick.take() {
            self.convert_pick(sh, ids);
        }
        let Some(cv) = self.conv.as_mut() else { return };
        if cv.step() {
            ctx.request_repaint_after(std::time::Duration::from_millis(120));
        }
        let b = &self.board;
        let canvas = self.canvas;
        let pal = style::pal();
        let h = min_touch(sh.env.touch).max(30.0);
        let w = (canvas.width() - 24.0).clamp(260.0, 460.0);
        // unter (bzw. über) den Strichen, innerhalb der Zeichenfläche
        let r = b.bounds_of(&cv.ids).unwrap_or(owndraw_core::geom::RectF::new(0.0, 0.0, 0.0, 0.0));
        let sp = canvas.min + Vec2::new(r.left * b.sc + b.tx, r.bottom * b.sc + b.ty + 14.0);
        let top = canvas.min.y + r.top * b.sc + b.ty - 14.0;
        // Höhe des Fensters aus dem letzten Bild; passt es weder darunter noch darüber, daneben (Striche bleiben sichtbar)
        let wh = cv.last_h.max(200.0);
        let right = canvas.min.x + r.right * b.sc + b.tx + 14.0;
        let left = canvas.min.x + r.left * b.sc + b.tx - 14.0;
        let (pos, pivot) = if sp.y + wh < canvas.bottom() - 8.0 {
            (sp, Align2::LEFT_TOP)
        } else if top - wh > canvas.top() + 8.0 {
            (Pos2::new(sp.x, top), Align2::LEFT_BOTTOM)
        } else if right + w + 40.0 < canvas.right() {
            (Pos2::new(right, canvas.top() + 8.0), Align2::LEFT_TOP)
        } else if left - w - 40.0 > canvas.left() {
            (Pos2::new(left, canvas.top() + 8.0), Align2::RIGHT_TOP)
        } else {
            (Pos2::new(sp.x, sp.y.min(canvas.bottom() - 120.0)), Align2::LEFT_TOP)
        };
        let mut done: Option<bool> = None;
        if ctx.input_mut(|i| super::keys::consume(i, &sh.settings, "cancel")) {
            done = Some(false);
        } else if ctx.input_mut(|i| super::keys::consume(i, &sh.settings, "text_done")) {
            done = Some(true);
        }
        let fg = item::ink(item::INK, pal.dark);
        let now = sh.now;
        let mut open_settings = false;
        let mut load: Option<Vec<String>> = None;
        let rect = Editor::float(ctx, "hw-conv", pos, pivot, canvas.shrink(8.0), Order::Foreground, |ui| {
            style::card().show(ui, |ui| {
                ui.set_width(w);
                ui.horizontal(|ui| {
                    ui.label(egui::RichText::new("Umwandeln").size(15.0).strong());
                    ui.label(egui::RichText::new(format!("{} Striche · {}", cv.ids.len(), cv.stufe.label())).size(12.0).color(c(pal.dim)));
                });
                ui.add_space(4.0);
                // Reiter
                ui.horizontal(|ui| {
                    for a in Art::ALL {
                        let lbl = match &cv.res[ai(a)] {
                            Res::Run(..) | Res::Pending => format!("{} …", a.label()),
                            _ => a.label().to_string(),
                        };
                        if style::chip(ui, &lbl, cv.tab == a, h * 0.9).clicked() {
                            cv.tab = a;
                            cv.auto_tab = false;
                            cv.focus = true;
                        }
                    }
                });
                ui.add_space(6.0);
                let i = ai(cv.tab);
                let res = cv.res[i].clone();
                match &res {
                    Res::Pending | Res::Run(..) => {
                        ui.horizontal(|ui| {
                            ui.spinner();
                            ui.label(egui::RichText::new(format!("Erkenne … {:.1} s", (now - cv.started).max(0.0))).color(c(pal.dim)));
                        });
                    }
                    Res::Err(e) => {
                        ui.label(egui::RichText::new(e).color(c(pal.danger)));
                    }
                    Res::Missing(msg) => {
                        ui.label(egui::RichText::new(msg).color(c(pal.dim)));
                        if let Some(info) = hw::info() {
                            let ms = info.manifests();
                            let need: Vec<&core_hw::Manifest> = core_hw::pakete_fuer_stufe(&ms, cv.stufe).into_iter().filter(|m| !info.paket(&m.id).is_some_and(|p| p.da())).collect();
                            let busy: Vec<&hw::Paket> = need.iter().filter_map(|m| info.paket(&m.id)).filter(|p| p.zustand == "laedt").collect();
                            if !busy.is_empty() {
                                // Fortschritt über alle Pakete der Stufe (werden nacheinander geladen)
                                let alle: Vec<&hw::Paket> = core_hw::pakete_fuer_stufe(&ms, cv.stufe).iter().filter_map(|m| info.paket(&m.id)).collect();
                                let (g, t) = alle.iter().fold((0u64, 0u64), |a, p| (a.0 + if p.da() { p.gesamt } else { p.geladen }, a.1 + p.gesamt));
                                ui.add(egui::ProgressBar::new(g as f32 / t.max(1) as f32).text(format!("{} von {}", core_hw::fmt_mb(g), core_hw::fmt_mb(t))));
                            } else if !need.is_empty() {
                                let sum: u64 = need.iter().map(|m| m.groesse()).sum();
                                ui.horizontal(|ui| {
                                    if style::accent_btn(ui, &format!("Modelle laden ({})", core_hw::fmt_mb(sum)), h).on_hover_text("Einmal herunterladen, danach im Browser gespeichert").clicked() {
                                        load = Some(need.iter().map(|m| m.id.clone()).collect());
                                    }
                                    if style::plain_btn(ui, "Einstellungen …", h).clicked() {
                                        open_settings = true;
                                    }
                                });
                            }
                        }
                    }
                    Res::Done(k, ms) => {
                        if k.is_empty() {
                            ui.label(egui::RichText::new("Nichts erkannt.").color(c(pal.dim)));
                        }
                        ui.label(egui::RichText::new(format!("Alternativen ({ms} ms)")).size(12.0).color(c(pal.dim)));
                        for (j, cand) in k.iter().enumerate().take(NBEST) {
                            let sel = cv.pick[i] == j;
                            let fill = if sel { c(pal.accent_soft) } else { Color32::TRANSPARENT };
                            let resp = egui::Frame::new().fill(fill).corner_radius(6).inner_margin(egui::Margin::symmetric(6, 3)).show(ui, |ui| {
                                ui.set_width(ui.available_width());
                                ui.horizontal(|ui| {
                                    ui.label(egui::RichText::new(format!("{:>2.0} %", cand.conf() * 100.0)).size(11.0).color(c(pal.dim)).monospace());
                                    if cv.tab == Art::Formel {
                                        if let Some((tid, sz)) = cv.math_tex(ui.ctx(), &cand.text, fg, w - 90.0, 22.0) {
                                            ui.add(egui::Image::new((tid, sz)));
                                        }
                                    } else {
                                        ui.label(egui::RichText::new(&cand.text).size(16.0));
                                    }
                                });
                                if cv.tab == Art::Formel {
                                    ui.label(egui::RichText::new(&cand.text).size(11.0).monospace().color(c(pal.dim)));
                                }
                            });
                            let resp = resp.response.interact(egui::Sense::click());
                            if resp.clicked() {
                                cv.pick[i] = j;
                                cv.edit[i] = field_text(cv.tab, &cand.text, cv.plain);
                                cv.touched[i] = false;
                                cv.focus = true;
                            }
                        }
                    }
                }
                // Korrekturfeld
                ui.add_space(6.0);
                ui.horizontal(|ui| {
                    ui.label(egui::RichText::new("Korrektur").size(12.0).color(c(pal.dim)));
                    if cv.tab == Art::Formel {
                        ui.add_space(8.0);
                        for (plain, l, tip) in [(false, "LaTeX", "Quelltext der Formel"), (true, "Rechnen", "Rechenzeile in einfacher Schreibweise (a: 2 N, M = a·b)")] {
                            if style::chip(ui, l, cv.plain == plain, 24.0).on_hover_text(tip).clicked() && cv.plain != plain {
                                let e = &mut cv.edit[1];
                                *e = if plain { calc::to_plain(e) } else { calc::to_latex(e) };
                                cv.plain = plain;
                            }
                        }
                    }
                });
                let id = egui::Id::unique("hw-korrektur");
                let edit = &mut cv.edit[i];
                let resp = ui.add(egui::TextEdit::singleline(edit).id(id).font(if cv.tab == Art::Formel { egui::TextStyle::Monospace } else { egui::TextStyle::Body }).desired_width(f32::INFINITY).hint_text(match cv.tab {
                    Art::Text => "erkannter Text",
                    Art::Formel => "z. B. M = a \\cdot b",
                    Art::Zahl => "z. B. 12,5 kN",
                }));
                if resp.changed() {
                    cv.touched[i] = true;
                }
                if resp.lost_focus() && ui.input(|x| x.key_pressed(egui::Key::Enter)) {
                    done = Some(true);
                }
                if cv.focus {
                    resp.request_focus();
                    cv.focus = false;
                }
                if cv.tab != Art::Text {
                    let latex = match cv.tab {
                        Art::Formel if cv.plain => calc::to_latex(&cv.edit[1]),
                        Art::Formel => cv.edit[1].clone(),
                        _ => calc::to_latex(&core_hw::zahl_klartext(&cv.edit[2])),
                    };
                    if !latex.trim().is_empty() {
                        ui.add_space(4.0);
                        egui::Frame::new().fill(c(pal.sunken)).corner_radius(8).inner_margin(4).show(ui, |ui| {
                            ui.set_min_size(Vec2::new(ui.available_width(), 34.0));
                            if let Some((tid, sz)) = cv.math_tex(ui.ctx(), &latex, fg, w - 24.0, 26.0) {
                                ui.add(egui::Image::new((tid, sz)));
                            }
                        });
                    }
                }
                ui.label(egui::RichText::new("Krakelig geschrieben? Wähle eine Alternative oder korrigiere hier.").size(11.0).color(c(pal.dim)));
                ui.add_space(6.0);
                ui.horizontal(|ui| {
                    let ok = !cv.edit[i].trim().is_empty();
                    if ui.add_enabled_ui(ok, |ui| style::accent_btn(ui, "Ersetzen", h)).inner.on_hover_text("Enter · ersetzt die Striche (Rückgängig stellt sie wieder her)").clicked() {
                        done = Some(true);
                    }
                    if style::plain_btn(ui, "Abbrechen", h).on_hover_text("Esc").clicked() {
                        done = Some(false);
                    }
                });
            });
        });
        if let Some(cv) = self.conv.as_mut() {
            cv.last_h = cv.last_h.max(rect.height()); // nur wachsen: das Fenster springt beim Reiterwechsel nicht
        }
        self.st.popup_rects.push(rect);
        if let Some(ids) = load {
            hw::download(&ids);
        }
        if open_settings {
            self.conv = None;
            sh.reqs.push(crate::app::Req::OpenSettings);
            return;
        }
        if let Some(ok) = done {
            self.finish_convert(sh, ok);
        }
    }
}

/// Optionen des Werkzeugs (Popover): kurze Anleitung und Stufe der Erkennung.
pub(super) fn options(ui: &mut egui::Ui, sh: &mut Shared) {
    let pal = style::pal();
    ui.label(egui::RichText::new("Handschrift umkreisen oder antippen (Tippen nimmt die ganze Zeile).").size(12.0).color(c(pal.dim)));
    ui.add_space(4.0);
    ui.label(egui::RichText::new("Erkennung").size(12.0).color(c(pal.dim)));
    ui.horizontal_wrapped(|ui| {
        let cur = sh.settings.hw.stufe();
        for s in Stufe::ALL {
            if style::chip(ui, s.label(), cur == s, 28.0).clicked() {
                sh.settings.hw.set(s);
            }
        }
    });
    if !hw::SUPPORTED {
        ui.label(egui::RichText::new("Nur im Browser verfügbar.").size(12.0).color(c(pal.danger)));
    }
    if style::plain_btn(ui, "Modelle und Lizenzen …", 28.0).clicked() {
        sh.reqs.push(crate::app::Req::OpenSettings);
    }
}
