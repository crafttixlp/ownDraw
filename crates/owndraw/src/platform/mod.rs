//! Plattform-Schicht (WP0: Vertrag; WP4: Desktop, WP5: Android).
//!
//! Abweichung vom Konzept (Abschnitt 5): statt Rückruf-Closures liefern Dialoge ihr Ergebnis als
//! [`PlatformEvent::Picked`] über [`Platform::poll`] (pro Frame aufrufen). So braucht die egui-App keine
//! geteilten mutablen Zustände in Closures; auf Android kommen Activity-Ergebnisse genauso an.

#[cfg(target_os = "android")]
pub mod android;
#[cfg(target_arch = "wasm32")]
pub mod web;

/// Läuft im Browser (Gastmodus: kein Dateisystem, keine Updates, Speichern = Download).
pub const WEB: bool = cfg!(target_arch = "wasm32");

/// Merkt, ob es Änderungen gibt, die noch nicht heruntergeladen wurden (Browser: Warnung beim Verlassen der
/// Seite). Außerhalb des Browsers ohne Wirkung.
pub fn note_unsaved(_unsaved: bool) {
    #[cfg(target_arch = "wasm32")]
    web::set_unsaved(_unsaved);
}

/// Stammt die Touch-Berührung `id` (egui `TouchId`) von einem Stift? (Stift-Modus)
///
/// Nur im Browser erkennbar. Desktop (X11/Wayland) meldet einen Grafiktablett- oder Touchscreen-Stift als Maus,
/// das gilt dort ohnehin als Stift. Android: winit gibt die Zeigerart nicht weiter, ein Stift kommt wie ein Finger an.
pub fn pen_touch(_id: u64) -> bool {
    #[cfg(target_arch = "wasm32")]
    return web::pen_touch(_id);
    #[cfg(not(target_arch = "wasm32"))]
    false
}

/// Kann der Stift-Modus Stift und Finger hier unterscheiden? (Android: nein, siehe [`pen_touch`])
pub const PEN_MODE: bool = !cfg!(target_os = "android");

use std::io;
use std::path::PathBuf;

pub mod stub;

/// Verweis auf eine Datei: Pfad (Desktop) oder SAF-URI (Android, `content://…`).
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub enum FileRef {
    Path(PathBuf),
    Uri(String),
}

impl FileRef {
    /// Anzeigename (Dateiname ohne Ordner).
    pub fn display_name(&self) -> String {
        match self {
            FileRef::Path(p) => p.file_name().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default(),
            FileRef::Uri(u) => {
                let d = percent_decode(u);
                d.rsplit(['/', ':']).next().unwrap_or(&d).to_string()
            }
        }
    }
    /// Serialisierte Form für Einstellungen/Zuletzt-Liste ("path:" bzw. "uri:" + Wert).
    pub fn to_key(&self) -> String {
        match self {
            FileRef::Path(p) => format!("path:{}", p.display()),
            FileRef::Uri(u) => format!("uri:{u}"),
        }
    }
    pub fn from_key(s: &str) -> Option<FileRef> {
        if let Some(p) = s.strip_prefix("path:") {
            Some(FileRef::Path(PathBuf::from(p)))
        } else {
            s.strip_prefix("uri:").map(|u| FileRef::Uri(u.to_string()))
        }
    }
}

fn percent_decode(s: &str) -> String {
    let b = s.as_bytes();
    let mut out = Vec::with_capacity(b.len());
    let mut i = 0;
    while i < b.len() {
        if b[i] == b'%' && i + 2 < b.len() {
            if let Ok(v) = u8::from_str_radix(&s[i + 1..i + 3], 16) {
                out.push(v);
                i += 3;
                continue;
            }
        }
        out.push(b[i]);
        i += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}

/// Dateiarten für Dialog-Filter und Drop-Router.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FileKind {
    Owndraw,
    Xopp,
    Image,
    Png,
    Pdf,
    /// Website-Export (ZIP).
    Zip,
    /// Ein Export-Format des Kerns (Export-Fenster).
    Export(owndraw_core::export::formats::Fmt),
    Any,
}

/// Endungen eines Export-Formats als statische Liste.
fn fmt_exts(f: owndraw_core::export::formats::Fmt) -> &'static [&'static str] {
    match f.ext() {
        "png" => &["png"],
        "jpg" => &["jpg", "jpeg"],
        "webp" => &["webp"],
        "avif" => &["avif"],
        "tiff" => &["tiff", "tif"],
        "bmp" => &["bmp"],
        "svg" => &["svg"],
        "pdf" => &["pdf"],
        "eps" => &["eps"],
        "emf" => &["emf"],
        "pptx" => &["pptx"],
        "docx" => &["docx"],
        "excalidraw" => &["excalidraw"],
        "drawio" => &["drawio"],
        "ora" => &["ora"],
        "psd" => &["psd"],
        "inkml" => &["inkml"],
        "json" => &["json"],
        "html" => &["html"],
        "gif" => &["gif"],
        _ => &[],
    }
}

impl FileKind {
    /// Dateiendungen (klein, ohne Punkt).
    pub fn extensions(self) -> &'static [&'static str] {
        match self {
            FileKind::Owndraw => &["owndraw"],
            FileKind::Xopp => &["xopp"],
            FileKind::Image => &["png", "jpg", "jpeg", "webp", "gif", "bmp"],
            FileKind::Png => &["png"],
            FileKind::Pdf => &["pdf"],
            FileKind::Zip => &["zip"],
            FileKind::Export(f) => fmt_exts(f),
            FileKind::Any => &[],
        }
    }
    pub fn mime(self) -> &'static str {
        match self {
            FileKind::Owndraw => "application/vnd.owndraw+zip",
            FileKind::Xopp => "application/x-xopp",
            FileKind::Image => "image/*",
            FileKind::Png => "image/png",
            FileKind::Pdf => "application/pdf",
            FileKind::Zip => "application/zip",
            FileKind::Export(f) => f.mime(),
            FileKind::Any => "*/*",
        }
    }
    /// Art anhand des Dateinamens.
    pub fn of_name(name: &str) -> FileKind {
        let ext = name.rsplit('.').next().unwrap_or("").to_ascii_lowercase();
        for k in [FileKind::Owndraw, FileKind::Xopp, FileKind::Image, FileKind::Pdf, FileKind::Zip] {
            if k.extensions().contains(&ext.as_str()) {
                return k;
            }
        }
        FileKind::Any
    }
}

/// Inhalt der Zwischenablage.
#[derive(Clone, Debug)]
pub enum ClipData {
    /// Bild als RGBA8 (nicht vormultipliziert).
    Image { w: u32, h: u32, rgba: Vec<u8> },
    /// Interne OwnDraw-Items (JSON-Text, Format von board.rs) plus gerenderte Vorschau als Bild.
    Items { json: String, image: Option<(u32, u32, Vec<u8>)> },
    Text(String),
}

/// Fallen gelassene Datei oder Daten.
#[derive(Clone, Debug)]
pub struct Drop {
    pub file: Option<FileRef>,
    /// Inhalt, falls ohne Datei geliefert (z. B. Bild aus Browser).
    pub bytes: Option<Vec<u8>>,
    pub name: String,
    /// Position in logischen Bildschirmpunkten, falls bekannt.
    pub pos: Option<(f32, f32)>,
}

/// Wofür ein Dialog geöffnet wurde (vom Aufrufer frei vergeben, kommt in `Picked` zurück).
pub type Tag = u32;

/// Ereignisse der Plattform an die App.
#[derive(Clone, Debug)]
pub enum PlatformEvent {
    /// Ergebnis eines Datei-/Ordnerdialogs (None = abgebrochen).
    Picked { tag: Tag, file: Option<FileRef> },
    /// Datei soll geöffnet werden (Kommandozeile, „Öffnen mit“, Teilen-Intent).
    OpenFile(FileRef),
    /// App geht in den Hintergrund (Android onPause) → sofort speichern.
    Pause,
}

pub trait Platform {
    /// Öffnen-Dialog des Systems. Ergebnis als `Picked { tag }`.
    fn pick_open(&self, kinds: &[FileKind], tag: Tag);
    /// Speichern-Dialog des Systems mit vorgeschlagenem Namen (inkl. Endung).
    fn pick_save(&self, name: &str, kind: FileKind, tag: Tag);
    /// Ordner wählen (Android: ACTION_OPEN_DOCUMENT_TREE mit dauerhaften Rechten).
    fn pick_folder(&self, tag: Tag);
    /// Liefert angefallene Ereignisse (pro Frame aufrufen).
    fn poll(&self) -> Vec<PlatformEvent>;

    fn read(&self, f: &FileRef) -> io::Result<Vec<u8>>;
    /// Schreibt atomar (temporäre Datei → umbenennen bzw. Cache-Kopie → ContentResolver "wt").
    fn write_atomic(&self, f: &FileRef, data: &[u8]) -> io::Result<()>;
    /// Legt in Ordner `dir` eine neue Datei `name` an (Standard-Ort, Android: SAF-Baum) und liefert ihren Verweis.
    fn create_in(&self, dir: &FileRef, name: &str, kind: FileKind) -> io::Result<FileRef>;
    /// Ob die Datei (noch) existiert/erreichbar ist.
    fn exists(&self, f: &FileRef) -> bool;
    /// Standard-Ablageort (Desktop `~/Dokumente/OwnDraw`, Android gewählter SAF-Ordner oder App-Speicher).
    fn default_dir(&self) -> FileRef;
    /// Ordner für Konfiguration (`settings.json`, Zuletzt-Liste).
    fn config_dir(&self) -> PathBuf;
    /// Ordner für Caches (Bilder geöffneter Boards, Update-Downloads).
    fn cache_dir(&self) -> PathBuf;

    fn clipboard_get(&self) -> Option<ClipData>;
    fn clipboard_set(&self, d: ClipData);
    /// Dateien/Daten, die seit dem letzten Aufruf fallen gelassen wurden.
    fn take_drops(&self, ctx: &egui::Context) -> Vec<Drop>;
    /// Dateien werden gerade über das Fenster gezogen (für Ablage-Hinweis).
    fn drag_hover(&self, ctx: &egui::Context) -> bool;
    /// URL/Datei im System öffnen (Browser, Website-Vorschau).
    fn open_url(&self, url: &str);
    /// Touch-Gerät ohne Maus (Android/Tablet)?
    fn is_touch(&self) -> bool;

    /// Kann Dateien an andere Apps teilen (Android: Teilen-Menü, Browser: Web Share)?
    fn can_share(&self) -> bool {
        false
    }
    /// Teilt eine Datei (Name mit Endung, MIME-Typ, Inhalt).
    fn share_file(&self, _name: &str, _mime: &str, _data: &[u8]) -> Result<(), String> {
        Err("Teilen ist hier nicht verfügbar".into())
    }
    /// Druckt ein PDF. Vorgabe: im PDF-Betrachter des Systems öffnen (dort drucken). Ok = Hinweis für die Anzeige.
    fn print_pdf(&self, name: &str, data: &[u8]) -> Result<String, String> {
        let dir = self.cache_dir().join("drucken");
        std::fs::create_dir_all(&dir).map_err(|e| format!("Drucken fehlgeschlagen: {e}"))?;
        let p = dir.join(name.replace(['/', '\\'], "_"));
        std::fs::write(&p, data).map_err(|e| format!("Drucken fehlgeschlagen: {e}"))?;
        self.open_url(&p.display().to_string());
        Ok("PDF geöffnet – dort drucken".into())
    }
}

#[cfg(not(any(target_os = "android", target_arch = "wasm32")))]
pub mod desktop;
