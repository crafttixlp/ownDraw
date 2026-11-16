# OwnDraw im Browser (WebAssembly, ab 4.6)

Gleiche App wie Desktop/Android (`crates/owndraw`), im Browser über eframe/WebGL (glow). Die Hülle ist
`crates/owndraw-web` (cdylib, `start(canvas_id)` → `owndraw::run_web`). Ausgeliefert vom `owndraw-server`
([server.md](server.md)) spricht die App mit dessen API ([web-api.md](web-api.md)); ohne erreichbaren Server
(z. B. rein statisch gehostet) läuft sie im **Gastmodus**.

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

## Mit Server (Konto, Boards am Server, Live)

Beim Start fragt die App `GET /api/me` (gleiche Origin, `credentials: include`, schreibende Anfragen mit
`X-OwnDraw: 1`). Antwortet dort kein OwnDraw-Server, bleibt es beim Gastmodus.

- **Startseite**: Konto-Karte oben – Anmelden/Registrieren, angemeldet Name, E-Mail, Lizenzstatus
  (Ablauf, Speicher belegt/Quote), „Lizenz einlösen“, „Abmelden“. Darunter „Am Server“: eigene und geteilte
  Boards (mit Besitzer und Rolle), Menü Öffnen/Umbenennen/Teilen/Löschen. Die bisherige Liste heißt im Browser
  „Zuletzt geöffnet (in diesem Browser)“.
- **Ohne Lizenz** verhält sich ein Konto wie ein Gast (neue Boards im Browser, Speichern als Download, Hinweis auf
  die Lizenz), sieht aber Boards, zu denen es eingeladen wurde, und arbeitet dort mit.
- **Mit Lizenz** werden neue Boards am Server angelegt (`POST /api/boards`). Server-Boards sind intern
  `FileRef::Uri("srv:<id>")`; Autosave schreibt per `PUT` mit `If-Match`. Bei 409 (anderswo geändert) meldet die App
  das und bietet „Neu laden“ an; die eigene Fassung lässt sich weiter herunterladen.
- **Teilen** (Board ⋯ → „Teilen …“, nur Besitzer mit Lizenz): Einladen per E-Mail mit Rolle „Bearbeiten“/
  „Ansehen“, Rolle ändern, entfernen.
- **Live**: Ein geöffnetes Server-Board verbindet sich per WebSocket (`/api/collab/{id}`). Änderungen gehen als
  Objekt-Operationen hin und her (Abgleich über die `uid` jedes Objekts, `owndraw_core::collab`), Rückgängig wirkt
  nur auf eigene Änderungen. Oben rechts: Verbindungsstatus und Teilnehmer (Farbe, Name); fremde Zeiger mit Namen
  auf der Zeichenfläche. Bei Abbruch verbindet die App automatisch neu (Abstand 1, 2, 4 … bis 30 s) und gleicht dann ab;
  ist keine Live-Verbindung möglich, speichert sie wie oben per `PUT`. Gespeichert wird bei Live-Boards vom Server.
  Rolle „Ansehen“ oder abgelaufene Lizenz der Besitzerin/des Besitzers → nur lesen.

## Grenzen

- Kein Update-Dienst (Einstellungs-Abschnitt ausgeblendet; die Web-Version kommt vom Server).
- Keine Website-Vorschau (nur Export als ZIP), kein Standard-Ordner, keine Speicherort-Wahl beim Anlegen.
- Browser-Speicher ist nicht dauerhaft garantiert (privates Fenster, Löschen der Website-Daten, Speicherquote):
  zum Sichern herunterladen. Ohne IndexedDB laufen Boards nur im Speicher.
- Rendern läuft in einem Thread (keine Web-Worker), große Boards sind langsamer als auf dem Desktop.
- Bilder werden im Browser nicht über das Clipboard-API *kopiert* (Kopieren legt nur Text/JSON ab).
- Fenstertitel = Seitentitel (`document.title`).
- Live: gleiches Objekt gleichzeitig bearbeitet → letzter gewinnt. Verschieben/Ändern einer Auswahl wird erst am
  Ende der Geste übertragen; fremde Änderungen während einer eigenen Geste kommen danach. Rückgängig deckt
  fremde Änderungen an Objekten ab, nicht an Ebenen/Seiten (kann fremde Ebenen-Änderungen zurücknehmen).
- Board-Name wird nicht live übertragen (nur Besitzer, per `PATCH`); Startseiten-Kacheln ohne Vorschaubild.
- Bilder werden beim Einfügen asynchron hochgeladen; andere sehen sie, sobald der Upload fertig ist.

## Code

- `crates/owndraw/src/platform/web/mod.rs` – `WebPlatform` und `Store` (IndexedDB), Download, Upload, Drops,
  Zwischenablage, `beforeunload`.
- `crates/owndraw/src/platform/web/api.rs` – Server-Anbindung: `fetch`, Konto, Boards, Mitglieder, Bilder, WebSocket.
- `crates/owndraw/src/ui/account.rs` – Konto-Karte, „Am Server“-Liste, Dialoge (Anmelden, Lizenz, Teilen, …).
- `crates/owndraw/src/ui/editor/live.rs` – Live-Sitzung im Editor (Abgleich, Zeiger, Teilnehmer, Neuverbindung).
- `crates/owndraw-core/src/collab.rs` – Operationen, Diff, Anwenden, `Sync` (Schatten + ausstehende Änderungen).
- `crates/owndraw/src/kv.rs` – Konfigurationsdateien: Datei (Desktop/Android) bzw. `localStorage` (Browser).
- Plattformweiche: `cfg(target_arch = "wasm32")`, in der Oberfläche `crate::platform::WEB`.
- Kern: Zeit über `web-time`, Bilder ohne Dateisystem über `item::put_mem_image`.
