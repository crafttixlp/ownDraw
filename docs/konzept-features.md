# OwnDraw – Konzept & Umsetzungsplan „Neue Funktionen“ (Ziel: Rust-Version)

Stand 13.11.2026. Basis: Rust-Port (Thread „OwnDraw nach Rust portieren“, Ergebnis `OwnDraw-Rust.zip`):
Cargo-Workspace `crates/owndraw-core` (UI-frei: Modell, tiny-skia-Rendering, Formerkennung, `store.rs`, `export.rs`, `item.rs`, `board.rs`)
und `crates/owndraw` (Linux-Desktop, eframe/egui 0.37). Abgestimmt mit dem Port-Thread. Noch keine Implementierung.

**Änderungen gegenüber dem Port:** `egui-file-dialog` wird durch `rfd` (System-Dialog) ersetzt, interne Zwischenablage (`thread_local` in `board.rs`) bekommt System-Anbindung über `arboard`, `store.rs` (index.json + Board-Ordner) wird durch Einzeldatei-Ablage ersetzt. Ein Android-Ziel existiert noch nicht und wird in WP5 neu gebaut (eframe + `android-activity`).

## 1. Leitlinien

- **Eine Oberfläche für alle Geräte:** eine egui-App für Linux/Windows/macOS und Android (eframe + `android-activity`).
  Plattform-Unterschiede nur in einer schmalen Schicht `platform/` (Dateidialog, Zwischenablage, Drag & Drop, Speicherpfade).
- **Eine Datei = ein Board:** keine `index.json`/Board-Ordner mehr. Die App merkt sich nur eine Liste „Zuletzt geöffnet“.
- **Simpler:** 3 Bildschirme (Start, Editor, Einstellungen), 7 Hauptwerkzeuge, alles Weitere im Kontext-Popover oder im „⋯“-Menü.
- Kern bleibt UI-frei und testbar (Unit-Tests, Round-Trip-Tests ohne Fenster).

## 2. Fachkonzept je Punkt

| # | Wunsch | Lösung |
|---|---|---|
| 1 | Board in einer `.owndraw` | ZIP-Container v2 (Abschnitt 3). Bilder, Vorschau, Einstellungen des Boards stecken in der Datei. |
| 2 | Speicherort wählbar, immer Autosave | Beim Anlegen: „Standard-Ort“ oder „Selbst wählen“ (System-Speicherdialog). In den Einstellungen: Vorgabe *immer fragen* / *immer Standard*. Autosave immer an: 2 s nach letzter Änderung, beim Verlassen, beim In-den-Hintergrund-Gehen (Android). Atomar: temporäre Datei → umbenennen (Desktop) bzw. Cache-Kopie → `ContentResolver` Modus `"wt"` (Android). Standard-Ort: Desktop `~/Dokumente/OwnDraw/` (über `dirs::document_dir`), Android: einmalig per SAF gewählter Ordner, sonst app-eigener Speicher. |
| 3 | Export als `.owndraw` | „Kopie speichern unter …“ im ⋯-Menü (gleiches Format, neue Board-ID). Altes Java-Format (index.json + Board-Ordner, altes `.owndraw` v1) wird beim Öffnen/ersten Start migriert. |
| 4 | Xournal++ lesen/bearbeiten | `.xopp` = gzip-XML. Import → normales Board im **Seitenmodus**; Export → `.xopp`. Abbildung siehe Abschnitt 4. Bearbeiten = importieren, in OwnDraw ändern, wieder als `.xopp` speichern (Autosave kann optional direkt ins `.xopp` schreiben, Standard: `.owndraw` daneben). |
| 5 | Unendlich oder Seiten | Beim „Neu“: Karte *Unendliches Board* oder *Seiten* (Format A4/A5/Letter, hoch/quer, Hintergrund blanko/liniert/kariert/punktiert). Seiten liegen in Weltkoordinaten untereinander (Abstand 24 pt), Inhalte werden an der Seitenkante abgeschnitten, Navigation per Seitenleiste (Miniaturen, + / löschen / verschieben). PDF-Export = 1 PDF-Seite je Board-Seite. |
| 6 | Maschinenbau-Pfeil | Neue Form `ArrowEng`: gefülltes Dreieck, **Öffnungswinkel 15° gesamt** (je Seite 7,5°), Spitzenlänge `max(10 × Linienbreite, 12 px)`, Linie endet an der Dreiecksbasis (scharfe Spitze). Im Form-Menü neben Pfeil/Doppelpfeil; Formerkennung bleibt beim normalen Pfeil. |
| 7 | UI passt sich an | Breakpoints nach logischer Breite (egui-Punkte) + Eingabeart: **kompakt** < 600 (Handy: Werkzeugleiste unten, Optionen als Bottom-Sheet, Ebenen als Vollbild-Sheet), **mittel** 600–1199 (Tablet: schwebende Seitenleiste, Popover), **groß** ≥ 1200 (Laptop/PC: Leisten oben/links, Ebenen-Panel angedockt, Tastaturkürzel-Hinweise). Touch-Ziele ≥ 44 pt bei Touch. Wechsel live beim Drehen/Fenstergröße. |
| 8 | Immer System-Dateimanager | Desktop: `rfd` mit xdg-desktop-portal (Linux), nativ auf Windows/macOS – kein eigener Dialog. Android: Storage Access Framework (`ACTION_OPEN_DOCUMENT`, `ACTION_CREATE_DOCUMENT`, `ACTION_OPEN_DOCUMENT_TREE`) per JNI. |
| 9 | Einstellungen | Bereiche: **Darstellung** (Hell/Dunkel/System, Akzentfarbe), **UI-Größe** (75–200 %, multipliziert mit System-DPI), **Leisten anpassen** (Position oben/unten/links/rechts je Leiste, Werkzeuge ein-/ausblenden und Reihenfolge per Ziehen im „Bearbeiten-Modus“ direkt im Editor), **Speichern** (Standard-Ort, fragen/Standard), Zurücksetzen. Gespeichert in `settings.json` im Konfig-Ordner. |
| 10 | Strg+C / Strg+V Bilder | Kopieren: Auswahl → PNG (gerendert, transparent) **und** interne Item-Daten in die Zwischenablage. Einfügen: interne Items bevorzugt, sonst Bild aus Zwischenablage als Img-Item an Mausposition/Bildmitte. Desktop `arboard` (Feature `wayland-data-control`), Android `ClipboardManager` per JNI (Bild als `content://`-URI über FileProvider). Android ohne Tastatur: „Kopieren/Einfügen“ in Auswahlleiste und Lang-Tippen-Menü. |
| 11 | Drag & Drop rein | Desktop: egui `dropped_files`/`hovered_files` mit Ablage-Hinweis. Router nach Typ: Bild (png/jpg/webp/gif) → Img-Item an Drop-Position; `.owndraw` → öffnen; `.xopp` → importieren; PDF → (später). Android: `View.OnDragListener` per JNI (Split-Screen/Tablet) + „Öffnen mit“/„Teilen an OwnDraw“-Intents. |

## 3. Dateiformat `.owndraw` v2

ZIP (MIME `application/vnd.owndraw+zip`, Fallback `application/zip`):
```
manifest.json   {"format":"owndraw","version":2,"app":"x.y","id":"<uuid>","name":"…",
                 "mode":"infinite"|"pages","page":{"w":595,"h":842,"bg":"lined"}|null,
                 "pages":[{"id":"…","bg":"…"}]|null, "modified":"<RFC3339>"}
board.json      bisheriges Board-JSON (Ebenen, Items, View-Zustand) + bei Items optional "page"
img/<sha256>.png|jpg  Bilder, inhaltsadressiert (keine Duplikate)
thumb.png       Vorschau 512 px (für Startbildschirm, kein Extra-Cache nötig)
```
Regeln: Unbekannte Felder beim Lesen behalten (Vorwärts-Kompatibilität), `version` > 2 → nur lesen mit Warnung.
v1-Dateien (`meta.json` + `board.json`) werden gelesen und beim nächsten Speichern als v2 geschrieben.

## 4. Xournal++-Abbildung (`.xopp`)

| Xournal++ | OwnDraw |
|---|---|
| `<page width height>` (Punkte, 1/72 in) | Seite im Seitenmodus, 1 pt = 1 Welteinheit |
| `<background type="solid" color style="plain/lined/ruled/graph/dotted">` | Seiten-Hintergrund (nicht unterstützte Stile → blanko) |
| `<background type="pdf">` | v1: leere Seite + Hinweis, Attribute beim Export unverändert zurückschreiben |
| `<layer>` | Ebene (Ebenen über alle Seiten gleich benannt; fehlende pro Seite leer) |
| `<stroke tool="pen" color="#RRGGBBAA" width="w p1 p2…">x y …</stroke>` | Stroke, Druckbreiten ↔ Druck je Punkt |
| `tool="highlighter"` | Marker |
| `fill="…"` | gefüllte Form (u. a. Maschinenbau-Pfeil beim Export) |
| `<text font size x y color>` | Text-/Notiz-Item |
| `<image left top right bottom>`base64-PNG | Img-Item |
| `<teximage>`, Audio-Verweise | übernehmen als Bild bzw. ignorieren, beim Export unverändert zurückschreiben, wenn unverändert |

Export aus *unendlichem* Board: Inhalt in A4-Seiten kacheln (Option) oder eine große Seite (Standard).
Formen (Linie, Ellipse, Pfeile, Herz) werden beim Export zu Strokes (ggf. `fill`). Abnahme über Round-Trip-Tests mit echten Xournal++-Beispieldateien.

## 5. Schnittstellen-Vertrag (vor Welle 1 festschreiben, alle Pakete halten sich daran)

```rust
// owndraw-core/src/doc.rs
pub enum BoardMode { Infinite, Pages(PageSetup) }
pub struct Document { pub meta: Meta, pub mode: BoardMode, pub board: Board, pub images: ImageStore }
pub fn read_owndraw(bytes: &[u8]) -> Result<Document>;
pub fn write_owndraw(doc: &Document) -> Result<Vec<u8>>;
// owndraw-core/src/xopp.rs
pub fn import_xopp(bytes: &[u8]) -> Result<Document>;
pub fn export_xopp(doc: &Document, opt: XoppExport) -> Result<Vec<u8>>;

// owndraw/src/platform/mod.rs
pub enum FileRef { Path(PathBuf), Uri(String) }            // Uri = Android SAF
pub trait Platform {
    fn pick_open(&self, kinds: &[FileKind], cb: Box<dyn FnOnce(Option<FileRef>)>);
    fn pick_save(&self, name: &str, kind: FileKind, cb: Box<dyn FnOnce(Option<FileRef>)>);
    fn pick_folder(&self, cb: Box<dyn FnOnce(Option<FileRef>)>);
    fn read(&self, f: &FileRef) -> io::Result<Vec<u8>>;
    fn write_atomic(&self, f: &FileRef, data: &[u8]) -> io::Result<()>;
    fn default_dir(&self) -> FileRef;
    fn clipboard_get(&self) -> Option<ClipData>;   // Bild (RGBA) oder interne Items
    fn clipboard_set(&self, d: ClipData);
    fn take_drops(&self) -> Vec<Drop>;            // Datei/Bild + Position
}
// owndraw/src/layout.rs
pub enum FormFactor { Compact, Medium, Expanded }
```
Asynchrone Rückrufe, weil SAF-Dialoge auf Android Activity-Ergebnisse sind.

## 5a. Update-Dienst (Wunsch 12)

**Nichts passiert automatisch.** Nur per „Nach Updates suchen“ in den Einstellungen (optional: Hinweis beim Start, Standard aus; installiert nie selbst) fragt die App die GitHub-API
(`/repos/<owner>/<repo>/releases/latest`) ab und vergleicht den Tag mit der eigenen Version (`semver`).
Bei neuer Version: Hinweis mit Änderungstext → „Jetzt aktualisieren“ / „Später“ / „Version überspringen“.

Zwei Modi (Einstellungen → Updates):
- **Aus Quellcode bauen** (gewünscht, Desktop): Quell-Tarball des Release-Tags laden → Signatur prüfen → in `~/.cache/OwnDraw/build/<tag>` entpacken → `cargo build --release --locked` (Rust-Toolchain muss vorhanden sein; fehlt sie, Hinweis mit Link zu rustup und Angebot, auf Modus „Fertiges Paket“ zu wechseln) → AppImage packen → installieren. Fortschritt und Build-Log im Dialog, Abbruch möglich.
- **Fertiges Paket** (Standard, schneller): Release-Asset (`OwnDraw-<v>-x86_64.AppImage` bzw. `.apk`) laden → Signatur prüfen → installieren.

Installieren:
- Linux-AppImage: neue Datei neben die laufende (`$APPIMAGE`) schreiben, ausführbar machen, atomar umbenennen, alte als `.bak` behalten; Neustart anbieten. Ohne Schreibrecht auf den Ordner → Hinweis.
- Android: Bauen auf dem Gerät ist nicht praktikabel, daher immer fertige APK → System-Installer (`PackageInstaller`, Nutzer bestätigt; Recht „Unbekannte Apps installieren“ nötig). APK muss mit demselben Schlüssel signiert sein.

Zurückspringen (PC): Unter Einstellungen → Updates werden alle Releases als Liste angezeigt (Version, Datum, Änderungstext). Jede ältere Version lässt sich gezielt installieren (gleicher Weg wie Update, auch Quellbau). Die zuletzt laufende Version bleibt als `OwnDraw-<v>.AppImage` in `~/.local/share/OwnDraw/versions/` (die letzten 3), „Vorherige Version wiederherstellen“ geht damit auch offline. Vor jedem Versionswechsel wird geprüft, ob die Zielversion das `.owndraw`-Format der offenen Boards lesen kann; sonst Warnung (neuere Formatversion → ältere App öffnet nur lesend).

Sicherheit (Pflicht): Jedes Release-Asset und der Quell-Tarball werden mit einem Ed25519-Schlüssel (`minisign`) signiert; der öffentliche Schlüssel ist in die App einkompiliert. Ohne gültige Signatur keine Installation. Nur HTTPS, keine Ausführung heruntergeladener Skripte außer `cargo build` im signierten Quellbaum.

Voraussetzung: Ein GitHub-Repository mit Releases und eine Release-CI (GitHub Actions: AppImage + APK bauen, signieren, hochladen) – existiert noch nicht; das Projekt hat bisher kein Remote.

## 6. Arbeitspakete

Jedes Paket hat feste Dateien („gehört“), damit parallele Agenten nicht kollidieren. Jeder Agent: `cargo build`, `cargo clippy -D warnings`, eigene Tests grün.

| WP | Ziel | Module (gehört) | Abh. | Modell | Abnahme |
|---|---|---|---|---|---|
| **0** Vertrag | Abschnitt 5 als Code-Stubs (Typen, Trait, `todo!()`), Breakpoint-Enum, Settings-Struct | `owndraw-core/src/doc.rs`, `owndraw/src/platform/mod.rs`, `owndraw/src/settings.rs` (nur Typen) | Rust-Port fertig | sonnet | Workspace baut; alle Stubs dokumentiert |
| **1** Format v2 | `.owndraw` v2 lesen/schreiben, v1-Import, Migration alter Java-Ablage | `owndraw-core/src/doc.rs`, `owndraw-core/src/migrate.rs`, `store.rs` (ersetzen) | 0 | sonnet | Round-Trip-Test (Board mit Bildern, beide Modi) bytegleich im Inhalt; v1-Datei aus Java-2.4 öffnet; Bild-Deduplizierung |
| **2** Seitenmodus | `BoardMode::Pages`: Layout, Clipping, Seiten-Hintergründe, Seiten +/−/verschieben (verschiebt Items mit), PDF-Export je Seite | `owndraw-core/src/pages.rs`, Seiten-Teil in `board.rs`/`export.rs` (WP3 ändert dort nur Pfeil-Code) | 0 | **opus** | Unit-Tests Layout/Clipping; PDF mit n Seiten; Lasso/Radierer über Seitengrenzen korrekt |
| **3** Pfeil | Form `ARROW_ENG` (15° gesamt, gefüllt), Hit-Test, Bounds, JSON-Kind 6, Web-/PDF-Export | `owndraw-core/src/item.rs` (Konstante `ARROW_ENG = 6`), `icons.rs` | 0 | haiku | Render-Snapshot-Test; Winkel numerisch 15° ± 0,1°; alte Dateien unverändert lesbar |
| **4** Desktop-Plattform | `Platform` für Desktop: `rfd` (Portal), `arboard` Bild + interne Items, `dropped_files`, atomares Schreiben, `dirs` | `owndraw/src/platform/desktop.rs` | 0 | sonnet | Manuell in Xvfb + Unit-Tests `write_atomic`; Dialog nutzt Portal (`RFD`-Feature `xdg-portal`) |
| **5** Android-Ziel + Plattform | Android-Build aufsetzen (eframe + `android-activity`, `cargo-apk`/`xbuild`, Touch/Stift/Druck an `owndraw-core` durchreichen), dann `Platform` per JNI: SAF (open/create/tree, persistente Rechte), Zwischenablage mit Bildern, Drag-Listener, „Öffnen mit/Teilen“-Intents, Manifest | `owndraw/src/platform/android.rs`, `owndraw/android/` (Manifest, ggf. kleine Java-Hilfsklasse) | 0 | **opus** | APK baut und läuft (zeichnen mit Finger/Stift); auf Emulator: Datei öffnen/speichern/Ordner wählen, Bild einfügen/kopieren, Intent öffnet `.owndraw` |
| **6** Responsive Shell + Vereinfachung | Start (Zuletzt/Neu/Öffnen, Neu-Dialog unendlich/Seiten), Editor-Gerüst mit 3 Formfaktoren, 7 Hauptwerkzeuge + Kontext-Popover + ⋯-Menü, Seitenleiste für Seiten | `owndraw/src/ui/start.rs`, `owndraw/src/ui/editor/*`, `owndraw/src/layout.rs` | 0 | sonnet | Screenshots bei 390×844, 820×1180, 1920×1080 ohne Überlappung; jede alte Funktion erreichbar (Checkliste) |
| **7** Speichern & Autosave | Neu-Dialog Speicherort (Standard/selbst), Autosave-Dienst (Debounce 2 s, Hintergrund-Thread, beim Schließen/Pausieren), „Kopie speichern unter“, Zuletzt-Liste, Fehleranzeige bei fehlenden Rechten | `owndraw/src/autosave.rs`, `owndraw/src/recent.rs` | 1, 4 (5 für Android) | sonnet | Kill-Test: Prozess während Autosave abbrechen → Datei bleibt gültig; Datei an gewähltem Ort aktualisiert sich ohne Zutun |
| **8** Xournal++ | `import_xopp`/`export_xopp` laut Abschnitt 4 | `owndraw-core/src/xopp.rs` (+ Testdaten) | 1, 2 | **opus** | Round-Trip mit ≥ 3 echten Xournal++-Dateien (Stifte mit Druck, Text, Bild, Ebenen, mehrere Seiten); exportierte Datei öffnet in Xournal++ ≥ 1.2 ohne Fehler |
| **9** Einstellungen | Einstellungsseite, UI-Größe, Theme, Leisten-Position, Bearbeiten-Modus (Werkzeuge ziehen/ausblenden), Speicher-Vorgabe, Reset; Persistenz | `owndraw/src/ui/settings.rs`, `owndraw/src/settings.rs` | 0, 6 | sonnet | Änderungen wirken sofort und überleben Neustart; Reset stellt Standard her |
| **10** Kopieren/Einfügen & Drop | Strg+C/V/X für Bilder und Items über `Platform`, Drop-Router (Bild/owndraw/xopp), Drop-Hinweis-Overlay, Touch-Menü | `owndraw/src/ui/editor/clip.rs`, `owndraw/src/ui/editor/drop.rs` | 4 (5), 6, 1, 8 (nur Router) | sonnet | Bild aus Browser/Dateimanager einfügen und ziehen; Auswahl in GIMP einfügbar; `.xopp` per Drop importiert |
| **11** Update-Dienst | Prüfen (GitHub-API, `ureq`+`semver`), Signaturprüfung (`minisign-verify`), Modus „Fertiges Paket“ und „Aus Quellcode bauen“, AppImage-Austausch, Android-PackageInstaller, Einstellungsbereich „Updates“ | `owndraw/src/update/*`, `owndraw/src/ui/update.rs` | 4, 5 (Android), 9 | sonnet (Android-Teil opus/WP5) | Gegen Test-Repo mit 2 Releases: Update wird erkannt, falsche Signatur abgelehnt, Quellbau + Austausch + Neustart klappt, Abbruch hinterlässt alte Version intakt; Downgrade auf Vorversion (online und offline aus `versions/`) klappt; ohne Klick wird nichts installiert |
| **12** Release-CI | GitHub-Actions-Workflow: bei Tag AppImage + APK + Quell-Tarball bauen, mit minisign signieren, als Release hochladen; Anleitung Schlüsselverwaltung | `.github/workflows/release.yml`, `docs/release.md` | GitHub-Repo existiert | haiku | Test-Tag erzeugt Release mit 3 Assets + `.minisig` |
| **13** Integration & QA | Zusammenführen, Konflikte lösen, README/Docs, Gesamttests, AppImage + APK bauen, Screenshot-Satz | alles (nur Integration) | alle | sonnet | `cargo test` grün; Abnahmeliste Abschnitt 7 vollständig abgehakt |

### Wellen (parallel)

- **Welle 0:** WP0 (allein, kurz).
- **Welle 1:** WP1, WP2, WP3, WP4, WP5, WP6 parallel.
- **Welle 2:** WP7, WP8, WP9, WP10, WP12 parallel.
- **Welle 3:** WP11.
- **Welle 4:** WP13.

WP5 ist das größte Risiko (Android-Ziel existiert noch nicht). Es blockiert nur die Android-Teile von WP7 und WP10; Desktop kann vorher fertig werden.

### Prompt-Vorlage für Sub-Agents

> Ziel: <WP-Ziel>. Projekt: `<Pfad Rust-Workspace>`. Lies zuerst `docs/konzept-features.md` Abschnitte 3–5.
> Du darfst NUR diese Dateien ändern/anlegen: <Liste>. Andere Dateien nur lesen; fehlt dir eine Schnittstelle, melde das statt sie zu ändern.
> Ausgeschlossen: <Nicht-Ziele>. Fertig, wenn: <Abnahmekriterien>. Ergebnis: kurze Liste geänderter Dateien, Testausgabe, offene Punkte.

## 7. Gesamtabnahme

1. Neues Board (beide Modi, beide Speicherort-Varianten) → nach Änderung ohne Klick gespeichert, nach Neustart unter „Zuletzt“.
2. `.owndraw` per Doppelklick/„Öffnen mit“/Drop öffnet; Kopie-Export öffnet auf anderem Gerät gleich.
3. `.xopp` importieren, bearbeiten, exportieren → öffnet in Xournal++ korrekt.
4. Maschinenbau-Pfeil: 15°, gefüllt, in PNG/PDF/Web-Export und `.xopp` sichtbar.
5. UI bei Handy-, Tablet-, PC-Größe bedienbar; UI-Größe und Leistenposition einstellbar.
6. Alle Dateidialoge sind die des Systems.
7. Bild per Strg+C/V rein und raus, Dateien per Drag & Drop rein.
8. Alte Java-Boards werden übernommen.
9. Update von GitHub wird erkannt, geprüft, gebaut bzw. geladen und installiert.

## 8. Annahmen (bitte bestätigen oder korrigieren)

- 15° ist der **gesamte** Öffnungswinkel der Pfeilspitze (wie DIN ISO 128), nicht je Seite.
- Seitenformat-Standard: A4 hoch, liniert.
- PDF-Hintergründe aus Xournal++ werden in v1 nicht angezeigt (nur erhalten).
- Android läuft über dieselbe egui-App wie der Desktop (Port liefert bisher nur Linux).
- Update-Repo: öffentlich, wird von Le auf GitHub angelegt (diese Umgebung kann keine Repos erstellen).
- Start erst, wenn `OwnDraw-Rust.zip` geliefert ist.

## 9. Umsetzungsnotizen (WP0, verbindlich)

- Vertrag liegt als Code vor: `owndraw-core/src/{doc,pages,xopp}.rs`, `owndraw/src/{platform/mod,layout,settings}.rs`.
- `Board` hat das Feld `board_mode: BoardMode` (Name `mode` war intern schon belegt).
- `Document::board` ist das Board-JSON (`serde_json::Value`), nicht `Board` selbst (Board ist `Rc`-basiert, nicht `Send`).
  Laden: `board.from_json(&doc.board); board.board_mode = doc.mode.clone();` – Speichern umgekehrt.
- Bilder: `ImageStore` (Name → Bytes); zum Rendern `materialize(dir)` in den Bildordner des Kerns (`item::set_img_dir`).
- `Platform`: Dialog-Ergebnisse kommen als `PlatformEvent::Picked { tag }` über `poll()` statt als Closure.
- Android-SDK/NDK ist in der Bau-Umgebung nicht ladbar → APK wird in GitHub Actions gebaut (WP12); lokal nur `cargo check --target aarch64-linux-android`.
