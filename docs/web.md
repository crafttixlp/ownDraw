# OwnDraw im Browser (WebAssembly, ab 4.6)

Gleiche App wie Desktop/Android (`crates/owndraw`), im Browser über eframe/WebGL (glow); ohne WebGL mit eigener
Software-Darstellung (siehe [Ohne WebGL](#ohne-webgl-software-darstellung-ab-461)). Die Hülle ist
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

## Ohne WebGL (Software-Darstellung, ab 4.6.1)

Ist im Browser die Hardwarebeschleunigung aus (neuere Chrome-Versionen haben dann keinen SwiftShader-Ersatz mehr),
liefert `getContext("webgl")` nichts und eframe startet nicht („WebGL isn't supported“). OwnDraw prüft deshalb beim
Start mit einem Test-Canvas, ob WebGL2 oder WebGL1 geht:

- **ja** → bisheriger Weg über eframe (`WebRunner`, glow), unverändert. Scheitert eframe trotzdem, bevor die App
  gebaut ist, geht es mit der Software-Darstellung weiter (Canvas wird bei Bedarf durch eine frische Kopie ersetzt).
- **nein** → eigener kleiner Runner ohne eframe-Painter (`platform/web/soft`): dieselbe App (`App::frame_ui`, von
  `eframe::App::ui` und vom Runner aufgerufen), Eingaben aus DOM-Ereignissen (Zeiger, Touch mit Druck, Rad/Pinch,
  Tastatur, Texteingabe und IME über ein verstecktes `<input>`, Kopieren/Ausschneiden/Einfügen, Ablegen von Dateien,
  Größe/`devicePixelRatio`, Fokus, Hell/Dunkel). Der Ereigniscode ist aus eframe 0.37 übernommen (MIT/Apache-2.0),
  verhält sich also wie im WebGL-Weg (auch: Stiftdruck nur über Touch-`force`, wie bei eframe).
- Darstellung: egui tesselieren, mit `softraster.rs` auf der CPU in einen RGBA-Puffer rastern, per `putImageData`
  auf ein 2D-Canvas. Wie `egui_glow`: Gamma-Raum, vormultipliziertes Alpha, Pixelmitten mit Top-Left-Regel,
  Scissor-Rundung, Filter nächster/bilinear, Wrap-Modi (keine Mipmaps, kein Dithering). Gemalt wird nur bei Bedarf
  (`request_repaint`/`request_repaint_after`) und nur, was sich geändert hat: Meshes werden inhaltsabhängig in
  Stücke von ~32 Dreiecken geteilt und mit dem letzten Frame verglichen (gemeinsame Teilfolge); Textur-Teilupdates
  (Board-Pixmap, Schrift-Atlas) werden über die UV-Abbildung der Dreiecke auf den Bildschirm umgerechnet. Achsen-
  parallele Rechtecke werden ohne Dreiecks-Rasterung gezeichnet, bei 1:1-Abbildung (Board, Schrift) als Kopie.
  Unit-Tests vergleichen u. a. inkrementelles mit vollständigem Neuzeichnen einer echten egui-Oberfläche.
- Unten erscheint einmal der Hinweis „Ohne Grafikbeschleunigung – Zeichnen kann langsamer sein. Tipp:
  Hardwarebeschleunigung im Browser aktivieren.“; ✕ blendet ihn aus (gemerkt in `localStorage`
  `owndraw.softwareHint`).
- Erzwingen zum Testen: `?renderer=software` bzw. `?renderer=webgl`. Gewählter Weg: `window.owndrawRenderer`,
  `<html data-renderer>`; im Software-Weg Messwerte in `window.owndrawSoftStats` (Frames, ms für Logik/Malen,
  gerasterte Pixel).
- Geht gar nichts (auch kein 2D-Canvas, oder `?renderer=webgl` ohne WebGL), zeigt `index.html` eine deutsche
  Fehlerseite mit Anleitung (Chrome/Edge: Einstellungen → System → Grafikbeschleunigung; Firefox: Einstellungen →
  Allgemein → Leistung) und den technischen Details zum Aufklappen. Ein Absturz im Software-Weg landet über
  `window.owndrawFehler` ebenfalls dort.

Gemessen (Chromium headless ohne GPU, `--disable-gpu --disable-webgl --disable-3d-apis`, 1280 × 800, Faktor 1):
Zeichnen mit der Maus ~53–56 Bilder/s (begrenzt durch die Eingaberate), Malzeit ~0,3 ms je Frame (nur der
Strich); Ansicht verschieben (alles neu) ~58 Bilder/s bei ~3 ms Malzeit, in 1920 × 1080 ~6 ms. Logik/Board-Rendern
wie im WebGL-Weg. Auf langsamen Rechnern und bei hoher Pixeldichte entsprechend mehr.

Grenzen: keine Mipmaps (stark verkleinerte Bilder flimmern etwas mehr), keine Barrierefreiheit (wie eframe ohne
`web_screen_reader`), kein `App::on_exit` (wie eframe im Browser).

## Mit Server (Konto, Boards am Server, Live)

Beim Start fragt die App `GET /api/me` (gleiche Origin, `credentials: include`, schreibende Anfragen mit
`X-OwnDraw: 1`). Antwortet dort kein OwnDraw-Server, bleibt es beim Gastmodus.

- **Startseite**: Konto-Karte oben – Anmelden/Registrieren, angemeldet Name, E-Mail, Lizenzstatus
  (Ablauf, Speicherplatz belegt/maximal), „Lizenz einlösen“, „Abmelden“. Darunter „Am Server“: eigene und geteilte
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
- **Fehler melden** (ab 4.10.0, nur mit Server): Startseite oben „Fehler melden“ bzw. Board ⋯ → „Fehler melden …“.
  Der Dialog liegt als HTML über der App (`index.html`, `window.owndrawFehlerMelden`, geöffnet über
  `platform::web::open_bug_report`): Beschreibung, bis zu 5 Screenshots (Datei wählen, Strg+V, Ziehen und Ablegen;
  zu große Bilder oder GIF/BMP verkleinert der Browser als JPEG), Häkchen „Technische Angaben mitsenden“ (Version,
  Browser, Fenstergröße, Pfad ohne `#…`). Solange er offen ist, bekommt die App keine Tasten-, Einfüge- oder
  Ablege-Ereignisse. Auch Gäste ohne Konto können melden. Desktop, Android und die statische Web-App ohne Server
  zeigen den Menüpunkt nicht.

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
- `crates/owndraw/src/platform/web/soft/` – Software-Runner ohne WebGL (Frame-Schleife, Ereignisse, Text-Agent,
  Hinweis); `crates/owndraw/src/softraster.rs` – CPU-Rasterizer mit Änderungserkennung (auf allen Zielen übersetzt,
  damit `cargo test` ihn prüft). Auswahl des Wegs in `owndraw::run_web` (`lib.rs`).
- `crates/owndraw/src/platform/web/api.rs` – Server-Anbindung: `fetch`, Konto, Boards, Mitglieder, Bilder, WebSocket.
- `crates/owndraw/src/ui/account.rs` – Konto-Karte, „Am Server“-Liste, Dialoge (Anmelden, Lizenz, Teilen, …).
- `crates/owndraw/src/ui/editor/live.rs` – Live-Sitzung im Editor (Abgleich, Zeiger, Teilnehmer, Neuverbindung).
- `crates/owndraw-core/src/collab.rs` – Operationen, Diff, Anwenden, `Sync` (Schatten + ausstehende Änderungen).
- `crates/owndraw/src/kv.rs` – Konfigurationsdateien: Datei (Desktop/Android) bzw. `localStorage` (Browser).
- Plattformweiche: `cfg(target_arch = "wasm32")`, in der Oberfläche `crate::platform::WEB`.
- Kern: Zeit über `web-time`, Bilder ohne Dateisystem über `item::put_mem_image`.
