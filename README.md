# OwnDraw

Zeichen- und Whiteboard-App mit Stift: unendliche Fläche, Ebenen, Formerkennung, Lasso-Auswahl,
Teil-Radierer und Export als PNG, PDF oder animierte Website. Ab Version 3.0 vollständig in **Rust**
geschrieben (vorher Java). Läuft als Linux-Desktop-App (AppImage). Boards bleiben als `.owndraw`-Datei
mit der Java-Version (2.x, Android und Desktop) austauschbar.

## Aufbau

Cargo-Workspace mit zwei Crates:

| Ordner | Inhalt |
| --- | --- |
| `crates/owndraw-core/` | Plattformneutraler Kern ohne Oberfläche: Objekte, Ebenen, Board-Logik und Eingabe, Formerkennung, Rendering (tiny-skia), Speichern, Import/Export (`.owndraw`, PNG, PDF, Website-ZIP), Icons. |
| `crates/owndraw/` | Desktop-Oberfläche mit egui/eframe: Übersicht, Editor, Leisten, Ebenen-Panel, Dialoge. |
| `packaging/appimage/` | AppImage-Runtime (C) und Paketierung, AppDir-Vorlage. |
| `assets/` | App-Icon. |
| `docs/architektur.md` | Aufbau im Detail. |

## Bauen

Voraussetzung: Rust ≥ 1.85 (`rustup`).

```sh
cargo run --release              # starten
cargo test                       # Tests (Kern: Board, Store, Export, Formerkennung)
cargo run -p owndraw-core --example compat -- <datei.owndraw>   # Kompatibilitätsprobe
```

**Linux-AppImage:** zusätzlich `gcc`, `zlib1g-dev`, `squashfs-tools`, dann `./build-appimage.sh`
→ `OwnDraw-3.0.0-x86_64.AppImage` (ca. 7 MB).

Laufzeit auf dem Ziel-PC: glibc ≥ 2.39 (Ubuntu 24.04, Fedora 40, Debian 13), X11 oder Wayland, OpenGL,
`libxkbcommon-x11` (unter X11). Eine `.owndraw`-Datei als Argument wird beim Start importiert.

## Daten

`$OWNDRAW_DATA`, sonst `$XDG_DATA_HOME/OwnDraw`, sonst `~/.local/share/OwnDraw/`:
`files/boards/` (`index.json`, `<id>.json`, `<id>.png`, `img/`), `cache/`, `prefs.txt`.
Gleicher Ort und gleiches Format wie die Java-Desktop-Version: vorhandene Boards werden direkt übernommen.

## Bedienung am PC

- Linke Maustaste zeichnet, rechte/mittlere Taste oder Leertaste + Ziehen verschiebt, Mausrad zoomt,
  Touchpad: zwei Finger verschieben, Strg + Scrollen oder Zusammenziehen zoomt. Touchscreens werden unterstützt.
- Tasten: P Stift, M Marker, E Radierer, S Auswahl, R Form, N Notiz, T Text, 0 alles zeigen, +/− Zoom;
  Strg+Z/Y/C/X/V/D/S/E, Entf löscht die Auswahl. Erneuter Klick auf Radierer, Auswahl oder Form wechselt den Modus.
- Formerkennung: Strich zeichnen und kurz halten (Stift/Form-Modus) – Linie, Pfeil, Doppelpfeil, Kreis/Ellipse,
  Dreieck, Rechteck/Quadrat, Raute, Fünf-/Sechseck, Stern, Herz.
- Bilder per Knopf „Bild“ oder Drag & Drop; `.owndraw`-Dateien per Drag & Drop in die Übersicht importieren.

## Kompatibilität

Das `.owndraw`-Format (ZIP mit `meta.json`, `board.json`, `thumb.png`, `img/`) und die JSON-Felder sind
unverändert. Geprüft: Java → Rust → Java ergibt identische Objekte und Ebenen.

## Android

Noch kein Ziel der Rust-Version. Der Kern (`owndraw-core`) ist plattformneutral; der naheliegende Weg ist
eframe mit `android-activity` plus Stiftdruck aus den Android-Touch-Ereignissen. Bis dahin bleibt die
Java-Android-App 2.4 (Backup `OwnDraw-Projekt.zip`) nutzbar; Boards lassen sich zwischen beiden austauschen.

## Stand

Version 3.0.0. Unter Linux (X11/Xvfb) mit automatisierten UI-Abläufen und Tests geprüft, kaum auf echten Geräten.
Offen: Stiftdruck am Linux-PC (winit liefert keinen Druck, Stift zeichnet mit fester Stärke), Android-Port,
Formerkennung bei sehr schnellen Strichen auf echten Geräten testen.
