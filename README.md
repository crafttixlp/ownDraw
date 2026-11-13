# OwnDraw

Zeichen- und Whiteboard-App mit Stift für Handy, Tablet und PC: unendliche Fläche **oder Seiten** (A4/A5/Letter),
Ebenen, Formerkennung, Lasso-Auswahl, Teil-Radierer, Export als PNG, PDF, Website und **Xournal++ (`.xopp`)**.
Komplett in Rust (egui/eframe), eine Oberfläche für Linux/Windows/macOS und Android.

## Neu in 4.0

- **Ein Board = eine Datei** (`.owndraw` v2, ZIP mit Bildern und Vorschau), Speicherort frei wählbar,
  **Autosave immer an** (2 s nach der letzten Änderung, beim Schließen und beim Wechsel in den Hintergrund, atomar).
- **Zuletzt geöffnet** auf dem Startbildschirm; alte Boards (Java 2.x / Rust 3.0) werden beim ersten Start übernommen.
- **Seitenmodus**: Seiten untereinander, Hintergrund blanko/liniert/kariert/punktiert, Seitenleiste, PDF mit einer Seite je Board-Seite.
- **Xournal++** öffnen, bearbeiten, als `.xopp` exportieren (Stifte mit Druck, Marker, Text, Bilder, Ebenen, PDF-Hintergründe bleiben erhalten).
- **Maschinenbau-Pfeil**: gefüllte Spitze, 15° Öffnungswinkel (DIN ISO 128).
- **Anpassende Oberfläche**: Handy (Leiste unten, Bottom-Sheets), Tablet (schwebende Leiste, Popover), PC (Leisten oben, Panels angedockt).
  7 Hauptwerkzeuge, alles Weitere im Kontext-Popover oder im „⋯“-Menü.
- **Einstellungen**: Hell/Dunkel/System, Akzentfarbe, UI-Größe 75–200 %, Leistenposition, Werkzeuge umordnen/ausblenden, Speicherort.
- **System-Dateidialoge** (xdg-desktop-portal unter Linux, Storage Access Framework unter Android).
- **Strg+C / Strg+V** für Objekte und Bilder (auch in/aus anderen Programmen), **Drag & Drop** von Bildern, `.owndraw` und `.xopp`.
- **Updates von GitHub** nur auf Klick: fertiges Paket oder aus dem Quellcode bauen, Signaturprüfung (minisign),
  Zurückspringen auf ältere Versionen (auch offline aus den letzten 3 Versionen).

## Aufbau

| Ordner | Inhalt |
| --- | --- |
| `crates/owndraw-core/` | Kern ohne Oberfläche: Board, Seiten, Formerkennung, Rendering (tiny-skia), `.owndraw` v2 (`doc.rs`), Migration, Xournal++ (`xopp.rs`), Export. |
| `crates/owndraw/` | App (egui/eframe) als Bibliothek + Desktop-Binary: `ui/` (Start, Editor, Einstellungen), `platform/` (Desktop, Android), Autosave, Zuletzt-Liste, Update-Dienst. |
| `crates/owndraw-android/` | Android-Einstieg (cdylib), Java-Hilfsklassen, Manifest, `build-apk.sh`. |
| `packaging/appimage/` | AppImage-Runtime und AppDir-Vorlage. |
| `.github/workflows/` | CI (Tests, AppImage, APK als Artefakte) und Release (signierte Assets). |
| `docs/` | `architektur.md`, `konzept-features.md`, `ui-checkliste.md`, `android.md`, `release.md`. |

## Bauen

Voraussetzung: Rust ≥ 1.85.

```sh
cargo run --release      # Desktop starten
cargo test               # Tests
./build-appimage.sh      # Linux-AppImage (braucht gcc, zlib1g-dev, squashfs-tools) → OwnDraw-4.0.0-x86_64.AppImage
crates/owndraw-android/build-apk.sh   # APK (braucht Android SDK + NDK, siehe docs/android.md)
```

Die APK baut auch die GitHub-Actions-CI bei jedem Push auf `main` (Artefakt „apk“).

Laufzeit Linux: glibc ≥ 2.39, X11 oder Wayland, OpenGL, `libxkbcommon-x11` (X11). Eine Datei als Argument wird geöffnet.

## Daten

- Boards: wo du sie speicherst; Standard `~/Dokumente/OwnDraw/` (Android: gewählter Ordner oder App-Speicher).
- Einstellungen und Zuletzt-Liste: `~/.config/OwnDraw/` (`settings.json`, `recent.json`), Cache `~/.cache/OwnDraw/`.
- `OWNDRAW_DATA=<ordner>` legt alles darunter ab (Tests).

## Bedienung am PC

- Linke Maustaste zeichnet, rechte/mittlere Taste oder Leertaste + Ziehen verschiebt, Mausrad zoomt.
- Tasten: P Stift, M Marker, E Radierer, S Auswahl, R Form, T Text, 0 alles zeigen, +/− Zoom;
  Strg+Z/Y/C/X/V/D, Entf löscht die Auswahl. Tooltips zeigen die Kürzel.

## Stand

Version 4.0.0. Unter Linux (X11/Xvfb) automatisiert getestet. Android-Code ist geprüft (`cargo check`), die APK
entsteht in CI; auf echten Geräten noch ungetestet. Für Updates muss vor dem ersten Release der echte
minisign-Schlüssel eingetragen werden (`docs/release.md`).
