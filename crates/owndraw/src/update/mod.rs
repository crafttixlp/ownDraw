//! Update-Dienst (Wunsch 12, Konzept 5a).
//!
//! Nichts passiert automatisch: Der Konstruktor [`Updater::new`] startet keine Netzwerkanfrage; geprüft wird nur
//! auf Klick (oder, wenn eingestellt, einmal beim Start – dann nur als Hinweis, installiert wird nie von selbst).
//! Netzwerk, Bau und Installation laufen in Hintergrund-Threads; die Oberfläche liest [`Status`].
//! Ohne gültige minisign-Signatur (`.minisig` neben jedem Asset) wird nichts installiert.
#![cfg_attr(target_os = "android", allow(dead_code))]

#[cfg(not(target_os = "android"))]
pub mod build;
pub mod github;
pub mod http;
pub mod install;
pub mod verify;
pub mod versions;

use crate::settings::{Settings, UpdateMode};
use github::{Api, Release};
use http::Http;
use install::Installed;
use semver::Version;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, OnceLock};

/// Fehler des Update-Dienstes (Text auf Deutsch, direkt anzeigbar).
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Error {
    Cancelled,
    /// Platzhalter-Schlüssel: Updates sind nicht eingerichtet.
    NotConfigured,
    BadSignature(String),
    /// Kein (beschreibbares) AppImage zum Ersetzen.
    NoAppImage(String),
    /// Keine Rust-Toolchain für den Quellbau.
    NoToolchain,
    /// Keine Verbindung (DNS/Verbindungsaufbau gescheitert).
    Offline,
    /// Server antwortet nicht rechtzeitig.
    Timeout,
    /// GitHub-Abfragegrenze ohne Anmeldung (60/Stunde) erreicht (HTTP 403/429).
    RateLimit,
    /// Sonstiger HTTP-Status (404 wird von der API-Schicht ausgewertet).
    Status(u16),
    Other(String),
}

impl std::fmt::Display for Error {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Error::Cancelled => write!(f, "Abgebrochen. Die installierte Version bleibt unverändert."),
            Error::NotConfigured => write!(f, "Updates sind nicht eingerichtet. Diese Version enthält noch keinen gültigen Signaturschlüssel."),
            Error::BadSignature(m) => write!(f, "Die Signatur ist ungültig. Es wurde nichts installiert ({m})."),
            Error::NoAppImage(m) | Error::Other(m) => write!(f, "{m}"),
            Error::NoToolchain => write!(f, "Keine Rust-Toolchain (cargo) gefunden."),
            Error::Offline => write!(f, "Offline. Keine Verbindung zu GitHub. Prüfe deine Internetverbindung und suche erneut."),
            Error::Timeout => write!(f, "Zeitüberschreitung. GitHub antwortet nicht. Versuche es später erneut."),
            Error::RateLimit => write!(f, "GitHub-Abfragegrenze erreicht (60 Abfragen pro Stunde ohne Anmeldung). Versuche es in etwa einer Stunde erneut."),
            Error::Status(c) => write!(f, "GitHub meldet einen Fehler (HTTP {c}). Versuche es später erneut."),
        }
    }
}

impl From<std::io::Error> for Error {
    fn from(e: std::io::Error) -> Self {
        Error::Other(format!("Dateifehler: {e}"))
    }
}

/// Alles, was ein Lauf braucht (injizierbar für Tests).
#[derive(Clone, Debug)]
pub struct Config {
    /// GitHub-API-Basis, normal `https://api.github.com`.
    pub api_base: String,
    pub repo: String,
    pub current: Version,
    /// Inhalt der öffentlichen minisign-Schlüsseldatei.
    pub pubkey: String,
    /// Downloads (`<cache>/update`).
    pub work_dir: PathBuf,
    /// Quellbäume (`<cache>/build`).
    pub build_dir: PathBuf,
    /// Gespeicherte Vorversionen (`~/.local/share/OwnDraw/versions`).
    pub versions_dir: PathBuf,
    /// Laufendes AppImage (`$APPIMAGE`).
    pub target: Option<PathBuf>,
    pub skip: Option<String>,
    pub mode: UpdateMode,
    /// Nur für Tests mit lokalem HTTP-Server.
    pub allow_http: bool,
    /// `cargo` für den Quellbau (None = suchen).
    pub cargo: Option<PathBuf>,
    #[cfg(not(target_os = "android"))]
    pub spec: build::BuildSpec,
    /// Anzahl zuletzt geöffneter Boards (nur für Hinweistexte).
    pub recent_boards: usize,
}

impl Config {
    pub fn new(settings: &Settings, platform: &dyn crate::platform::Platform) -> Config {
        let cache = platform.cache_dir();
        Config {
            api_base: "https://api.github.com".into(),
            repo: settings.update_repo.clone(),
            current: Version::parse(env!("CARGO_PKG_VERSION")).unwrap_or_else(|_| Version::new(0, 0, 0)),
            pubkey: verify::PUBKEY.to_string(),
            work_dir: cache.join("update"),
            build_dir: cache.join("build"),
            versions_dir: crate::data_dir().join("versions"),
            target: install::running_appimage(),
            skip: settings.update_skip.clone(),
            mode: if cfg!(target_os = "android") { UpdateMode::Package } else { settings.update_mode },
            allow_http: false,
            cargo: None,
            #[cfg(not(target_os = "android"))]
            spec: build::BuildSpec::owndraw(),
            recent_boards: 0,
        }
    }

    fn api<'a>(&'a self, http: &'a dyn Http) -> Api<'a> {
        Api { http, base: &self.api_base, repo: &self.repo }
    }

    /// Ob ein echter Signaturschlüssel einkompiliert ist.
    pub fn configured(&self) -> bool {
        !verify::is_placeholder(&self.pubkey)
    }
}

/// Zustand und Abbruch eines Laufs; wird vom Worker beschrieben und von der Oberfläche gelesen.
#[derive(Clone, Default)]
pub struct Ctl {
    pub cancel: Arc<AtomicBool>,
    st: Arc<Mutex<Status>>,
    ctx: Arc<Mutex<Option<egui::Context>>>,
}

/// Momentaufnahme für die Oberfläche.
#[derive(Clone, Debug, Default)]
pub struct Status {
    /// Beschreibung der laufenden Arbeit (None = nichts läuft).
    pub busy: Option<String>,
    pub step: String,
    /// (geladen, gesamt) beim Download.
    pub progress: Option<(u64, Option<u64>)>,
    /// Build-Log (die letzten Zeilen).
    pub log: Vec<String>,
    pub check: Option<CheckResult>,
    /// Hinweis zum Update wurde mit „Später“ weggeklickt.
    pub dismissed: bool,
    pub releases: Option<Vec<Release>>,
    pub error: Option<Error>,
    pub done: Option<Installed>,
    /// Start-Check hat eine neue Version gefunden: Die App fragt den Nutzer in einem Dialog
    /// ([`crate::ui::update::start_prompt`]); installiert wird erst nach „Jetzt aktualisieren“.
    pub prompt: bool,
}

const LOG_MAX: usize = 400;

impl Ctl {
    pub fn status(&self) -> Status {
        self.st.lock().map(|s| s.clone()).unwrap_or_default()
    }
    fn with(&self, f: impl FnOnce(&mut Status)) {
        if let Ok(mut s) = self.st.lock() {
            f(&mut s);
        }
        if let Ok(c) = self.ctx.lock() {
            if let Some(c) = c.as_ref() {
                c.request_repaint();
            }
        }
    }
    pub fn cancelled(&self) -> bool {
        self.cancel.load(Ordering::Relaxed)
    }
    pub fn step(&self, s: &str) {
        self.with(|st| {
            st.step = s.to_string();
            st.progress = None;
        });
    }
    pub fn progress(&self, done: u64, total: Option<u64>) {
        self.with(|st| st.progress = Some((done, total)));
    }
    pub fn log(&self, line: &str) {
        self.with(|st| {
            st.log.push(line.to_string());
            if st.log.len() > LOG_MAX {
                st.log.drain(..st.log.len() - LOG_MAX);
            }
        });
    }
}

/// Ergebnis der Versionsabfrage.
#[derive(Clone, Debug, PartialEq)]
pub struct CheckResult {
    pub latest: Option<Release>,
    /// Neuer als die laufende Version.
    pub newer: bool,
    /// Neuer, aber vom Nutzer übersprungen.
    pub skipped: bool,
}

/// Fragt `/releases/latest` ab und vergleicht mit der laufenden Version.
pub fn check(cfg: &Config, http: &dyn Http) -> Result<CheckResult, Error> {
    let latest = cfg.api(http).latest()?;
    let newer = latest.as_ref().map(|r| r.version > cfg.current && !r.prerelease).unwrap_or(false);
    let skipped = newer && latest.as_ref().and_then(|r| cfg.skip.as_deref().and_then(versions::parse_tag).map(|s| s == r.version)).unwrap_or(false);
    Ok(CheckResult { latest, newer, skipped })
}

/// Alle Releases (neueste zuerst).
pub fn list_releases(cfg: &Config, http: &dyn Http) -> Result<Vec<Release>, Error> {
    cfg.api(http).list()
}

/// Name des Paket-Assets dieser Plattform (None: kein fertiges Paket für diese Architektur).
pub fn package_asset(v: &Version) -> Option<String> {
    if cfg!(target_os = "android") {
        Some(format!("OwnDraw-{v}.apk"))
    } else if cfg!(all(target_os = "linux", target_arch = "x86_64")) {
        Some(format!("OwnDraw-{v}-x86_64.AppImage"))
    } else {
        None
    }
}

pub fn source_asset(v: &Version) -> String {
    format!("OwnDraw-{v}-src.tar.gz")
}

fn check_url(cfg: &Config, url: &str) -> Result<(), Error> {
    if url.starts_with("https://") || (cfg.allow_http && url.starts_with("http://")) {
        Ok(())
    } else {
        Err(Error::Other("Die Download-Adresse nutzt kein HTTPS und wurde abgelehnt.".into()))
    }
}

/// Lädt Asset + `.minisig` nach `dir`, prüft die Signatur und liefert den Pfad der geprüften Datei.
fn fetch_verified(cfg: &Config, http: &dyn Http, rel: &Release, name: &str, dir: &Path, ctl: &Ctl) -> Result<PathBuf, Error> {
    if !cfg.configured() {
        return Err(Error::NotConfigured);
    }
    let asset = rel.asset(name).ok_or_else(|| Error::Other(format!("Das Release {} enthält die Datei „{name}“ nicht.", rel.tag)))?;
    let sig = rel
        .asset(&format!("{name}.minisig"))
        .ok_or_else(|| Error::BadSignature(format!("Zu {name} fehlt die .minisig-Datei")))?;
    check_url(cfg, &asset.url)?;
    check_url(cfg, &sig.url)?;
    std::fs::create_dir_all(dir)?;
    ctl.step("Signatur wird geladen …");
    let sig_text = http.get_text(&sig.url)?;
    if ctl.cancelled() {
        return Err(Error::Cancelled);
    }
    ctl.step(&format!("{name} wird geladen …"));
    let part = dir.join(format!("{name}.part"));
    let r = http.download(&asset.url, &part, &ctl.cancel, &mut |d, t| ctl.progress(d, t));
    if let Err(e) = r {
        let _ = std::fs::remove_file(&part);
        return Err(e);
    }
    ctl.step("Signatur wird geprüft …");
    if let Err(e) = verify::verify_file(&cfg.pubkey, &part, &sig_text) {
        let _ = std::fs::remove_file(&part);
        return Err(e);
    }
    let fin = dir.join(name);
    std::fs::rename(&part, &fin)?;
    Ok(fin)
}

/// Lädt und installiert `rel` im Modus `cfg.mode` (Android: immer fertige APK).
pub fn install_release(cfg: &Config, http: &dyn Http, rel: &Release, ctl: &Ctl) -> Result<Installed, Error> {
    let dir = cfg.work_dir.join(&rel.tag);
    #[cfg(target_os = "android")]
    {
        let name = package_asset(&rel.version).unwrap_or_default();
        let apk = fetch_verified(cfg, http, rel, &name, &dir, ctl)?;
        ctl.step("System-Installer wird geöffnet …");
        let note = crate::platform::android::install_apk(&apk).map_err(|e| Error::Other(format!("Installer konnte nicht gestartet werden: {e}")))?;
        ctl.log(&note);
        // Fehlt das Recht „Unbekannte Apps installieren“, öffnet Java die Systemeinstellung statt des Installers.
        if note.starts_with("Erlauben") {
            return Err(Error::Other(note));
        }
        Ok(Installed { version: rel.version.clone(), path: apk.clone(), backup: apk, restart: false })
    }
    #[cfg(not(target_os = "android"))]
    {
        // Vorab prüfen, damit nichts umsonst geladen wird.
        let target = install::require_target(cfg.target.as_deref())?;
        if !cfg.configured() {
            return Err(Error::NotConfigured);
        }
        let new_file = match cfg.mode {
            UpdateMode::Package => {
                let name = package_asset(&rel.version).ok_or_else(|| Error::Other("Für diese Prozessorarchitektur gibt es kein fertiges Paket. Wähle „Aus Quellcode bauen“.".to_string()))?;
                fetch_verified(cfg, http, rel, &name, &dir, ctl)?
            }
            UpdateMode::Source => {
                let cargo = cfg.cargo.clone().or_else(build::find_cargo).ok_or(Error::NoToolchain)?;
                let tar = fetch_verified(cfg, http, rel, &source_asset(&rel.version), &dir, ctl)?;
                ctl.step("Quellcode wird entpackt …");
                // Ältere Quellbäume aufräumen (sparen Platz), nur der aktuelle bleibt.
                if let Ok(rd) = std::fs::read_dir(&cfg.build_dir) {
                    for e in rd.flatten() {
                        if e.file_name().to_string_lossy() != rel.tag {
                            let _ = std::fs::remove_dir_all(e.path());
                        }
                    }
                }
                let root = build::extract(&tar, &cfg.build_dir.join(&rel.tag))?;
                if ctl.cancelled() {
                    return Err(Error::Cancelled);
                }
                ctl.step("App wird gebaut. Das kann einige Minuten dauern …");
                build::build(&root, &cfg.spec, &cargo, &ctl.cancel, &|l| ctl.log(l))?
            }
        };
        if ctl.cancelled() {
            return Err(Error::Cancelled);
        }
        ctl.step("Update wird installiert …");
        let r = install::install_appimage(&new_file, &rel.version, &target, &cfg.current, &cfg.versions_dir)?;
        let _ = std::fs::remove_dir_all(&dir);
        Ok(r)
    }
}

/// Installiert eine gespeicherte Vorversion (offline).
pub fn install_saved(cfg: &Config, saved: &versions::Saved, ctl: &Ctl) -> Result<Installed, Error> {
    let target = install::require_target(cfg.target.as_deref())?;
    ctl.step("Vorherige Version wird wiederhergestellt …");
    install::install_appimage(&saved.path, &saved.version, &target, &cfg.current, &cfg.versions_dir)
}

/// Hintergrunddienst für die Oberfläche. Der Konstruktor tut nichts im Netz.
#[derive(Clone)]
pub struct Updater {
    ctl: Ctl,
    http: Arc<dyn Http>,
    busy: Arc<AtomicBool>,
}

static GLOBAL: OnceLock<Updater> = OnceLock::new();

/// Der Dienst der laufenden App (wird erst beim ersten Zugriff angelegt, ohne Netzwerkzugriff).
pub fn global() -> &'static Updater {
    GLOBAL.get_or_init(Updater::new)
}

impl Default for Updater {
    fn default() -> Self {
        Self::new()
    }
}

impl Updater {
    pub fn new() -> Updater {
        Updater::with_http(Arc::from(http::default_http(false)))
    }

    pub fn with_http(http: Arc<dyn Http>) -> Updater {
        Updater { ctl: Ctl::default(), http, busy: Arc::new(AtomicBool::new(false)) }
    }

    pub fn status(&self) -> Status {
        self.ctl.status()
    }

    /// Damit der Worker die Oberfläche zum Neuzeichnen weckt.
    pub fn set_ctx(&self, ctx: &egui::Context) {
        if let Ok(mut c) = self.ctl.ctx.lock() {
            if c.is_none() {
                *c = Some(ctx.clone());
            }
        }
    }

    #[cfg(test)]
    pub fn is_busy(&self) -> bool {
        self.busy.load(Ordering::SeqCst)
    }

    pub fn cancel(&self) {
        self.ctl.cancel.store(true, Ordering::SeqCst);
    }

    /// Meldung/Fehler wegklicken (schließt auch den Start-Dialog).
    pub fn dismiss_message(&self) {
        self.ctl.with(|s| {
            s.error = None;
            s.done = None;
            s.prompt = false;
        });
    }

    /// Start-Dialog schließen („Später“/„Überspringen“).
    pub fn close_prompt(&self) {
        self.ctl.with(|s| {
            s.prompt = false;
            s.dismissed = true;
        });
    }

    /// „Später“: Hinweis ausblenden.
    pub fn dismiss_update(&self) {
        self.ctl.with(|s| s.dismissed = true);
    }

    fn spawn(&self, what: &str, f: impl FnOnce(&Ctl, &dyn Http) -> Result<(), Error> + Send + 'static) -> bool {
        if self.busy.swap(true, Ordering::SeqCst) {
            return false;
        }
        self.ctl.cancel.store(false, Ordering::SeqCst);
        self.ctl.with(|s| {
            s.busy = Some(what.to_string());
            s.step.clear();
            s.progress = None;
            s.log.clear();
            s.error = None;
            s.done = None;
        });
        let (ctl, http, busy) = (self.ctl.clone(), self.http.clone(), self.busy.clone());
        std::thread::spawn(move || {
            let r = f(&ctl, &*http);
            ctl.with(|s| {
                s.busy = None;
                s.progress = None;
                if let Err(e) = r {
                    s.error = Some(e);
                }
            });
            busy.store(false, Ordering::SeqCst);
        });
        true
    }

    /// „Nach Updates suchen“ (Klick). `silent`: Start-Check; bei neuer Version wird nur der Dialog angefordert
    /// ([`Status::prompt`]), Fehler werden verschluckt.
    pub fn check(&self, cfg: Config, silent: bool) -> bool {
        self.spawn("Suche nach Updates …", move |ctl, http| match check(&cfg, http) {
            Ok(r) => {
                ctl.with(|s| {
                    if silent && r.newer && !r.skipped {
                        s.prompt = true;
                    }
                    s.dismissed = false;
                    s.check = Some(r);
                });
                Ok(())
            }
            Err(e) if silent => {
                let _ = e;
                Ok(())
            }
            Err(e) => Err(e),
        })
    }

    /// Release-Liste laden (für „Zurückspringen“).
    pub fn load_list(&self, cfg: Config) -> bool {
        self.spawn("Versionen laden …", move |ctl, http| {
            let l = list_releases(&cfg, http)?;
            ctl.with(|s| s.releases = Some(l));
            Ok(())
        })
    }

    /// Lädt und installiert (Update oder gezielt ältere Version).
    pub fn install(&self, cfg: Config, rel: Release) -> bool {
        self.spawn(&format!("Version {} installieren", rel.version), move |ctl, http| {
            let r = install_release(&cfg, http, &rel, ctl)?;
            ctl.with(|s| s.done = Some(r));
            Ok(())
        })
    }

    /// Gespeicherte Vorversion zurückholen (offline).
    pub fn restore(&self, cfg: Config, saved: versions::Saved) -> bool {
        self.spawn(&format!("Version {} wiederherstellen", saved.version), move |ctl, _| {
            let r = install_saved(&cfg, &saved, ctl)?;
            ctl.with(|s| s.done = Some(r));
            Ok(())
        })
    }
}

static START_DONE: AtomicBool = AtomicBool::new(false);

/// Ob diese Installation sich selbst aktualisieren kann (Android-APK oder laufendes, beschreibbares AppImage).
pub fn self_update_possible(cfg: &Config) -> bool {
    cfg!(target_os = "android") || cfg.target.as_deref().is_some_and(|t| install::check_writable(t).is_ok())
}

/// Einmal pro Frame aus der App-Hülle aufrufen. Beim ersten Aufruf: Start-Check, falls `update_ask_on_start`
/// eingeschaltet ist (Standard an) und die App sich selbst aktualisieren kann. Gefunden wird nur – der Dialog
/// fragt den Nutzer, installiert wird nie von selbst.
pub fn frame_hook(settings: &Settings, platform: &dyn crate::platform::Platform, ctx: &egui::Context) {
    if START_DONE.swap(true, Ordering::SeqCst) || !settings.update_ask_on_start {
        return;
    }
    let cfg = Config::new(settings, platform);
    if cfg.configured() && self_update_possible(&cfg) {
        global().set_ctx(ctx);
        global().check(cfg, true);
    }
}

#[cfg(test)]
mod tests;
