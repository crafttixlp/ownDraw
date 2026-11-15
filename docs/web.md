# OwnDraw im Browser (WebAssembly, ab 4.6)

Phase 1: **Gastmodus ohne Server**. Gleiche App wie Desktop/Android (`crates/owndraw`), im Browser über
eframe/WebGL (glow). Die Hülle ist `crates/owndraw-web` (cdylib, `start(canvas_id)` → `owndraw::run_web`).
Schnittstelle zum späteren Server: [web-api.md](web-api.md).

## Bauen

```bash
rustup target add wasm32-unknown-unknown
cargo install wasm-bindgen-cli --version 0.2.130 --locked   # exakt die wasm-bindgen-Version aus Cargo.lock
./build-web.sh                                             # → dist/web/
cd dist/web && python3 -m http.server 8080                  # http://localhost:8080/
```

`build-web.sh` baut mit dem Profil `web` (wie `release`, aber ohne `strip = true`, sonst fehlt wasm-bindgen der
Abschnitt `target_features`), ruft `wasm-bindgen --target web` und – falls vorhanden – `wasm-opt -O3`. Ergebnis:
`index.html` (Canvas über die ganze Seite, Ladeanzeige, Fehlertext auf Deutsch), `owndraw_web.js`,
`owndraw_web_bg.wasm` (ca. 8,9 MB, gzip ca. 3 MB), `owndraw.png`. Ausliefern als statische Dateien; der Server
sollte `.wasm` als `application/wasm` und komprimiert (gzip/brotli) senden.

`owndraw-web` steht in `[workspace] members`, aber nicht in `default-members`; auf dem Host ist das Crate leer.

## Was geht

- Editor vollständig (Zeichnen, Formen, Text, Ebenen, Seiten, Bilder, Rückgängig, Einstellungen).
- **Öffnen** (`.owndraw`, `.xopp`, Bilder) über `<input type=file>`; Drag & Drop von Dateien auf die Seite.
- **Speichern = Download**: im Board ⋯ → „Herunterladen (.owndraw)“ oder Strg+S; Export PNG/PDF/ZIP/Ebene/
  Xournal++ ebenfalls als Download. Technisch liefert `pick_save` sofort `mem:download/<name>`, `write_atomic`
  darauf löst den Download aus.
- Boards liegen vorläufig im Browser: im Speicher als `mem:<name>`, gesichert in IndexedDB (`owndraw`/`files`),
  Autosave wie gewohnt. Einstellungen und Zuletzt-Liste in `localStorage` (`owndraw:/owndraw/config/…`).
  Boards, die nicht mehr in der Zuletzt-Liste stehen, werden beim nächsten Start aus IndexedDB entfernt.
- Zwischenablage: Kopieren/Einfügen innerhalb der App (mit Bildern), Text aus anderen Programmen wird zur Notiz,
  eingefügte Bilder (Strg+V) werden eingesetzt. Beim Kopieren landet Text/JSON in der System-Zwischenablage.
- Hinweis „Gastmodus“ auf der Startseite und in den Einstellungen; beim Verlassen der Seite warnt der Browser,
  solange Änderungen noch nicht heruntergeladen wurden (`beforeunload`).

## Grenzen

- Kein Update-Dienst (Einstellungs-Abschnitt ausgeblendet; die Web-Version kommt vom Server).
- Keine Website-Vorschau (nur Export als ZIP), kein Standard-Ordner, keine Speicherort-Wahl beim Anlegen.
- Browser-Speicher ist nicht dauerhaft garantiert (privates Fenster, Löschen der Website-Daten, Speicherquote):
  zum Sichern herunterladen. Ohne IndexedDB laufen Boards nur im Speicher.
- Rendern läuft in einem Thread (keine Web-Worker), große Boards sind langsamer als auf dem Desktop.
- Bilder werden im Browser nicht über das Clipboard-API *kopiert* (Kopieren legt nur Text/JSON ab).
- Fenstertitel = Seitentitel (`document.title`).

## Code

- `crates/owndraw/src/platform/web/mod.rs` – `WebPlatform` und `Store` (IndexedDB), Download, Upload, Drops,
  Zwischenablage, `beforeunload`.
- `crates/owndraw/src/platform/web/api.rs` – Platzhalter für Login und Boards am Server (Phase 2).
- `crates/owndraw/src/kv.rs` – Konfigurationsdateien: Datei (Desktop/Android) bzw. `localStorage` (Browser).
- Plattformweiche: `cfg(target_arch = "wasm32")`, in der Oberfläche `crate::platform::WEB`.
- Kern: Zeit über `web-time`, Bilder ohne Dateisystem über `item::put_mem_image`.
