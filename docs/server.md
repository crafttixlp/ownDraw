# OwnDraw-Server: Betrieb und Installation

Der Server (`crates/owndraw-server`, axum + SQLite) liefert die Web-App aus, bietet die JSON-API aus
[`web-api.md`](web-api.md) und das Admin-Panel unter `/admin`. Davor läuft Caddy mit automatischem HTTPS.
Alles lebt unter einer Origin: `https://owndraw.thepepelex.at`.

```
Browser ──HTTPS──> Caddy (80/443) ──> owndraw-server (8080, intern)
                                         ├─ /            Web-App (Ordner web/)
                                         ├─ /admin       Admin-Panel (im Binary eingebettet)
                                         ├─ /api/...     JSON-API
                                         └─ data/        owndraw.db, boards/, versions/, current, backups/
```

## 1. DNS

In der DNS-Zone `thepepelex.at` einen Eintrag für den Hostnamen `owndraw` anlegen, der auf den Server zeigt:

| Name | Typ | Wert |
|---|---|---|
| `owndraw` | A | IPv4-Adresse des Servers |
| `owndraw` | AAAA | IPv6-Adresse des Servers (falls vorhanden) |

Prüfen: `dig +short owndraw.thepepelex.at`. Caddy holt das Zertifikat erst, wenn der Name auf den Server zeigt und die
Ports 80 und 443 von außen erreichbar sind (Firewall/Security-Group freigeben).

## 2. Server

| | Minimum | Empfohlen |
|---|---|---|
| CPU | 1 vCPU | 2 vCPU |
| RAM | 1 GB | 2–4 GB |
| Platte | 10 GB | 20 GB+ (Boards + Sicherungen) |
| System | Debian 12 oder Ubuntu 24.04 | dasselbe |

Der RAM-Bedarf des laufenden Servers ist klein (Größenordnung 50 MB plus Board-Uploads); Docker-Build und
Anmeldungen (Argon2) brauchen kurzzeitig mehr. Mit 1 GB RAM zusätzlich 1–2 GB Swap einrichten.
Benötigt: Docker mit Compose-Plugin (`docker compose`), `git`.

```bash
# Docker (Debian/Ubuntu): offizielles Skript, danach Compose-Plugin prüfen
curl -fsSL https://get.docker.com | sh
docker compose version
```

## 3. Installation (Schritt für Schritt)

**Schnellweg: Installer.** `packaging/server/install.sh` erledigt alle Schritte unten (alle Pakete inkl. Docker + Compose über apt, dnf/yum, zypper, apk oder pacman, Repo nach
`/opt/owndraw`, Web-App, `.env`, Rechte, Swap bei wenig RAM, Start, Prüfung) und ist mehrfach ausführbar:

```bash
# Skript und OwnDraw-Web-<v>-statisch.zip in denselben Ordner legen, dann:
sudo sh install.sh              # fragt Domain, Admin-E-Mail und Passwort ab
```

Ohne Zip baut der Installer die Web-App in einem Docker-Container. Optionen: `bash install.sh --help`.

**Von Hand:**

1. **Repository holen** und die Web-App bauen (`dist/web/` wird vom Dockerfile kopiert, ohne sie schlägt der
   Image-Build fehl):

   ```bash
   git clone https://github.com/crafttixlp/ownDraw.git /opt/owndraw
   cd /opt/owndraw
   ./build-web.sh            # erzeugt dist/web/ (index.html, owndraw_web.js, owndraw_web_bg.wasm, Icons)
   ```

2. **Konfiguration** anlegen:

   ```bash
   cd packaging/server
   cp .env.example .env
   nano .env                 # OWNDRAW_ADMIN_EMAIL und OWNDRAW_ADMIN_PASSWORD setzen
   mkdir -p data && chown 10001:10001 data   # der Container läuft als Benutzer 10001
   ```

3. **Starten** (baut das Image, startet Server und Caddy):

   ```bash
   docker compose up -d --build
   docker compose logs -f owndraw     # Start beobachten, mit Strg+C verlassen
   ```

4. **Prüfen**: `curl https://owndraw.thepepelex.at/api/health` liefert `{"ok":true,"version":"…"}`;
   `https://owndraw.thepepelex.at/` zeigt die Web-App, `/admin` das Admin-Panel.

Der erste Build dauert einige Minuten (Rust-Kompilierung). Danach starten Container nach Neustart des Servers von
selbst (`restart: unless-stopped`).

## 4. Erster Admin

- **Beim ersten Start** per `.env`: `OWNDRAW_ADMIN_EMAIL` und `OWNDRAW_ADMIN_PASSWORD`. Der Admin wird nur angelegt,
  wenn es noch keinen gibt. Danach die Passwortzeile aus `.env` entfernen.
- **Oder von Hand** (Passwort wird von stdin gelesen, mindestens 8 Zeichen; existiert die Adresse schon, wird das
  Konto zum Admin befördert und das Passwort neu gesetzt):

  ```bash
  docker compose exec -T owndraw /app/owndraw-server admin create admin@example.org <<< 'mein-langes-passwort'
  ```

  (Ist der Container noch nicht gestartet: `docker compose run --rm -T --entrypoint /app/owndraw-server owndraw admin create …`.)
  Dabei läuft das mitgelieferte Binary; für diese Verwaltungsaufgabe genügt das.

Dann `https://owndraw.thepepelex.at/admin` öffnen und anmelden.

### Zwei-Faktor-Anmeldung (Pflicht ab 4.16.0)

Die Verwaltung ist nur mit Passwort **und** Code aus einer Authenticator-App (TOTP, 6 Stellen, 30 s; z. B. Aegis,
2FAS, Google Authenticator) erreichbar. Das Passwort allein gibt nie Admin-Rechte; normale Konten sind nicht betroffen.

1. **Einrichten** (erster Login unter `/admin` nach dem Update, auch für jeden neuen Admin): E-Mail und Passwort
   eingeben, QR-Code scannen (oder den Schlüssel abtippen), den angezeigten Code eingeben.
2. Danach erscheinen **10 Wiederherstellungscodes** (`XXXXX-XXXXX`). Jeder gilt einmal, falls das Handy fehlt. Sie
   werden nur jetzt angezeigt: ausdrucken oder im Passwortmanager speichern. Neue Codes gibt es unter **Sicherheit**
   (oben rechts, bestätigt mit einem App-Code); die alten werden dabei ungültig.
3. **Jede weitere Anmeldung**: Passwort, dann Code aus der App (oder ein Wiederherstellungscode).

Schutz: Ein Code gilt nur einmal (auch nicht in derselben 30-s-Periode erneut), erlaubt ist ±30 s Zeitabweichung
(Serveruhr per NTP synchron halten). Nach 5 falschen Codes ist das Konto für 15 Minuten gesperrt; zusätzlich gilt das
IP-Limit wie beim Login. Das Geheimnis liegt verschlüsselt in `data/owndraw.db`, der Schlüssel in `data/secret.key`
(Rechte 0600); Wiederherstellungscodes nur als Hash.

**Notfall: ausgesperrt** (Handy weg, keine Wiederherstellungscodes). Auf dem Server:

```bash
cd /opt/owndraw/packaging/server
docker compose exec owndraw sh -c 'B=/app/data/current/owndraw-server; [ -x "$B" ] || B=/app/owndraw-server; "$B" admin reset-2fa admin@example.org'
```

(Der Umweg über `data/current` nimmt die per Panel-Update installierte Version; das mitgelieferte Binary kann älter
sein und kennt `reset-2fa` dann noch nicht.)

Das löscht die 2FA des Kontos und beendet alle seine Sitzungen; beim nächsten Login unter `/admin` wird sie neu
eingerichtet. Wer Zugriff auf den Server hat, kann das immer; also SSH gut schützen.

Im Panel:

- **Nutzer**: sperren/entsperren, Admin-Recht geben/entziehen, löschen (mit Bestätigung; löscht auch die Boards),
  Lizenz und Speicherverbrauch ansehen. **Sperren löscht sofort und unwiderruflich alle Boards des Nutzers am Server**
  (samt Freigaben, Einladungslinks und laufenden Live-Sitzungen; vorher kommt eine Bestätigungsabfrage). Entsperren
  bringt sie nicht zurück.
- **Lizenz aktivieren/deaktivieren** (Nutzer-Tab): Deaktivieren nimmt die Lizenzrechte (Speichern am Server, Teilen,
  Live), Boards bleiben erhalten. Aktivieren schaltet eine deaktivierte Lizenz wieder ein; hat der Nutzer keine oder nur
  eine abgelaufene, wählst du einen freien Code oder lässt einen neuen erzeugen (Laufzeit, Speicherplatz).
- **Lizenzen**: mehrere Codes `OWND-XXXX-XXXX-XXXX` erzeugen (Laufzeit in Tagen ab Einlösen oder unbegrenzt,
  Speicherplatz pro Nutzer in MB, Notiz), kopieren, einem Nutzer zuweisen (sofort aktiv), deaktivieren/aktivieren, Ablauf ändern. Die Codes gibst du den Kundinnen und Kunden; sie lösen
  sie in der Web-App ein.
- **Updates**: siehe Abschnitt 7.

## 5. Konfiguration (Umgebungsvariablen)

| Variable | Standard | Bedeutung |
|---|---|---|
| `OWNDRAW_DATA` | `./data` (im Container `/app/data`) | Datenordner |
| `OWNDRAW_WEB` | `./web` (im Container `/app/web`) | Ordner der Web-App |
| `OWNDRAW_BIND` | `0.0.0.0:8080` | Adresse/Port des Servers |
| `OWNDRAW_ADMIN_EMAIL`, `OWNDRAW_ADMIN_PASSWORD` | – | legt beim Start einen Admin an, falls keiner existiert |
| `OWNDRAW_MAX_BOARD_MB` | `50` | maximale Größe eines Boards. Bei Erhöhung `max_size` im `Caddyfile` mit anheben |
| `OWNDRAW_DOMAIN` | `owndraw.thepepelex.at` | Domain für Caddy (`.env`) |
| `OWNDRAW_RELEASE_URL` | GitHub-API `…/releases/latest` | nur für Tests/Spiegel des Update-Servers |

Anmeldungen, Registrierungen und Lizenz-Einlösungen sind pro Client-Adresse begrenzt (10 Versuche je 10 Minuten,
im Speicher). Hinter Caddy wird die Adresse aus `X-Forwarded-For` gelesen; der Server deshalb nie ohne Caddy
direkt ins Internet stellen (Port 8080 ist im Compose-Setup nur intern erreichbar).

## 6. Datenablage und Backup

```
data/owndraw.db        Konten, Sitzungen, Lizenzen, Board-Metadaten, Update-Einstellungen (SQLite, WAL)
data/boards/<id>.owndraw   Board-Dateien
data/secret.key        Schlüssel für die 2FA-Geheimnisse der Admins (mitsichern; ohne ihn: reset-2fa)
data/downloads/        App-Dateien für /download (APK, AppImage, manifest.json; lädt der Server selbst neu)
data/backups/          automatische DB-Sicherungen vor jedem Update
data/bugreports/<id>/  Screenshots der Fehlerberichte (ab 4.10.0, Text in der DB)
data/versions/<v>/     per Self-Update installierte Versionen (Binary + web/)
data/current           Symlink auf die aktive Version (data/previous = die vorige)
```

**Sicherung** (konsistent, auch im laufenden Betrieb):

```bash
cd /opt/owndraw/packaging/server
mkdir -p backup
sqlite3 data/owndraw.db "VACUUM INTO '$PWD/backup/owndraw-$(date +%F).db'"     # (sqlite3 auf dem Host)
rsync -a data/boards/ backup/boards/
cp -p data/secret.key backup/
rsync -a data/bugreports/ backup/bugreports/
```

Ohne `sqlite3` auf dem Host: `docker compose stop owndraw`, den ganzen Ordner `data/` kopieren
(`tar czf backup-$(date +%F).tar.gz data`), `docker compose start owndraw`. Täglich per Cron/Systemd-Timer und
mindestens eine Kopie außerhalb des Servers aufbewahren. Die Zertifikate (Volume `caddy_data`) lassen sich
jederzeit neu ausstellen und brauchen kein Backup.

**Wiederherstellen**: Container stoppen, `data/owndraw.db` (und die zugehörigen `-wal`/`-shm`-Dateien löschen) und
`data/boards/` und `data/bugreports/` aus der Sicherung zurückspielen, Container starten.

## 7. Updates

Releases liegen auf GitHub (`crafttixlp/ownDraw`). Ein Server-Release enthält das Asset
`owndraw-server-<version>-x86_64-linux.tar.gz` (Binary `owndraw-server` + Ordner `web/`) und die Signatur
`….tar.gz.minisig` (gleicher minisign-Schlüssel wie die Desktop-App,
`crates/owndraw/src/update/minisign.pub`).

**Im Admin-Panel (Tab Updates)**: installierte und neueste Version, „Jetzt prüfen“, „Jetzt updaten“.
Ablauf beim Update: herunterladen, Signatur prüfen (ohne gültige Signatur passiert nichts), Datenbank sichern
(`VACUUM INTO data/backups/owndraw-<alt>-<zeit>.db`), nach `data/versions/<v>/` entpacken, `data/current` umstellen,
Prozess beenden. Docker startet den Container neu; der Start-Wrapper `packaging/server/entrypoint.sh` nimmt dann
`data/current/owndraw-server` samt `web/`. Besteht die neue Version den Health-Check (`/api/health`) nicht
innerhalb von 30 s, stellt der Wrapper `data/current` auf die vorige Version zurück (oder entfernt den Link, dann gilt
das im Image mitgelieferte Binary) und startet neu. Die DB-Sicherung vor dem Update bleibt erhalten, falls eine neue
Version das Schema geändert hat und man zurück will (Backup einspielen, siehe oben).

Wird das Image neu gebaut (`docker compose up -d --build`) und ist das mitgelieferte Binary neuer als
`data/current`, nimmt der Wrapper das neuere und entfernt `data/current`.

**Auto-Update** ist standardmäßig **aus**. Eingeschaltet prüft der Server einmal täglich zur eingestellten Stunde und
installiert eine neue Version selbst. Die **Uhrzeit gilt in UTC** (Wiener Zeit: Winter UTC+1, Sommer UTC+2; 03:00 UTC
entspricht 04:00/05:00 Uhr in Wien). Das Protokoll im Panel zeigt jede Prüfung und Installation.

**Release erstellen**: normalerweise automatisch per Tag `v<version>` (GitHub Actions, siehe `docs/release.md`; braucht die Secrets `MINISIGN_KEY` und `MINISIGN_PASSWORD`). Solange kein Release existiert, zeigt das Panel „kein Release auf GitHub“.

Von Hand (Entwicklungsrechner, im Repo): `./build-web.sh`, dann

```bash
packaging/server/release-server.sh --docker -k ~/.minisign/owndraw.key
```

Das erzeugt `dist/server/owndraw-server-<v>-x86_64-linux.tar.gz` (+ `.sha256`, + `.minisig`, wenn `-k` angegeben ist).
`--docker` baut das Binary im gleichen Debian-12-Stage wie das Laufzeit-Image (passende glibc); ohne `--docker`
wird lokal mit `cargo` gebaut. Beide Dateien (`.tar.gz` und `.minisig`) als Assets an das GitHub-Release hängen. Ein
Release ohne diese Assets wird vom Server erkannt und nicht installiert.

## 8. Download-Seite (`/download`, ab 4.17.0)

`https://owndraw.thepepelex.at/download` bietet die Android-App (`.apk`) und die Linux-App (`.AppImage`) zum
Herunterladen an, mit Version, Größe, SHA-256 und Kurzanleitung. Verlinkt ist sie in der Web-App auf der Startseite
(„Apps“) und im Editor-Menü („Apps herunterladen“).

- **Quelle**: das neueste GitHub-Release. Der Server prüft kurz nach dem Start und danach stündlich, lädt neue
  Dateien samt `.minisig`, prüft die Signatur (Schlüssel wie beim Self-Update) und legt sie in `data/downloads/` ab.
  Ausgeliefert wird nur von dort, Nutzer kommen nicht zu GitHub. Ohne gültige Signatur wird nichts angeboten.
- **Fehlende Dateien**: Hat ein Release keine APK (Secrets `ANDROID_KEYSTORE_B64`/`_PASS`/`ANDROID_KEY_ALIAS` nicht
  gesetzt, siehe `release.md`), bleibt die zuletzt geladene APK stehen; gab es nie eine, zeigt die Seite
  „Noch nicht verfügbar“. Sobald ein Release die Datei enthält, erscheint sie spätestens nach einer Stunde
  (sofort: Container neu starten).
- **Schutz**: höchstens 30 Downloads je Adresse in 10 Minuten (danach 429); im Wartungsmodus sehen Nicht-Admins die
  Wartungsseite. Ausgeliefert werden nur Dateinamen aus `manifest.json`.

## 9. Betrieb

```bash
docker compose ps                 # Status
docker compose logs --tail 100 owndraw
docker compose restart owndraw
docker compose pull caddy && docker compose up -d      # Caddy aktualisieren
```

Ohne Docker (z. B. zum Testen): `OWNDRAW_DATA=./data OWNDRAW_WEB=dist/web owndraw-server`. Nach einem
Self-Update beendet sich der Prozess nur; einen Neustart muss dann ein Supervisor (systemd `Restart=always`) übernehmen.

**Mitteilungen und Wartung (ab 4.8.0).** Im Admin-Panel unter **Mitteilungen** lassen sich Hinweise (z. B. zu
Updates) anlegen, bearbeiten, (de)aktivieren und löschen; Typ Info oder Warnung, optional mit Zeitraum. Jeder
Besucher der Web-App, auch Gäste, sieht aktive Mitteilungen bei jedem Laden der Seite (Wegklicken gilt nur bis zum nächsten Neuladen). Unter **Wartung** schaltet
man den Wartungsmodus ein (optional mit eigener Nachricht und Endzeit, danach endet er automatisch): Nutzer und Gäste
sehen dann eine Wartungsseite, die API antwortet ihnen mit 503 `maintenance`, Live-Verbindungen werden getrennt.
Admins arbeiten weiter; `/admin` und `/api/health` bleiben erreichbar. Details: [`web-api.md`](web-api.md).
Tipp für Updates: vorher Wartung einschalten, nach dem Neustart prüfen, dann ausschalten.

**Fehlerberichte (ab 4.10.0).** In der Web-App können Nutzer und Gäste über „Fehler melden“ eine Beschreibung mit bis zu
5 Screenshots senden. Im Admin-Panel unter **Fehlerberichte** stehen alle Meldungen (Filter Offen/Erledigt/Alle) mit
Datum, Absender, Text, Screenshots und technischen Angaben; dort lassen sie sich als erledigt markieren oder löschen.
Grenzen: je Bild 5 MB (PNG/JPEG/WebP), 5 Meldungen je Adresse in 10 Minuten, insgesamt 1000 Meldungen bzw. 1 GB
Screenshots. Im Wartungsmodus nimmt der Server keine Meldungen an.

**Übersicht (ab 4.13.0).** Der erste Tab im Admin-Panel zeigt Kacheln, jeweils getrennt nach Konten und Gästen, und
aktualisiert sich alle 5 Sekunden, solange er sichtbar ist:
- *Gerade online*: Personen mit einer angemeldeten Anfrage in den letzten 5 Minuten oder einer offenen
  Live-Verbindung. Wird nur im Arbeitsspeicher gezählt und beginnt nach einem Neustart bei null.
- *Nutzer gesamt*: Anzahl der Konten bzw. Gast-IDs (Gäste per Einladungslink).
- *Erstellte Boards*: Konten = Boards am Server (Besitzer ist immer ein Konto). Gäste können keine eigenen Server-Boards
  anlegen, deshalb zählt bei Gästen jedes Board, in dem mindestens ein Gast Mitglied ist. Zusätzlich „Insgesamt
  erstellt“ inklusive gelöschter Boards (Zähler beginnt beim ersten neuen Board nach dem Update mit dem Bestand).
- *Gerade bearbeitet*: Boards mit laufender Live-Sitzung (mit mindestens einem Konto bzw. Gast), darunter eine Liste
  mit Name, Besitzer und Anzahl der Teilnehmer.
- *Am Server gespeichert*: Anzahl der Boards und belegter Speicher (Summe der Board-Dateien).

## 10. Hinweise zum Vertrag (`web-api.md`)

- Umgesetzt wie beschrieben, einschließlich Live-Zusammenarbeit über `/api/collab/{board_id}` (WebSocket).
  Pro offenem Board hält der Server einen **Raum** im Speicher (`src/collab.rs`): Dokument, laufende `seq`,
  Teilnehmer. Operationen werden in Ankunftsreihenfolge angewendet und an alle verteilt. Gespeichert wird ein
  geänderter Raum höchstens alle 5 s, wenn der letzte Teilnehmer geht und beim Beenden bzw. vor einem Update
  (`flush_all`); jede Speicherung erhöht `version` und erneuert das Vorschaubild. Schlägt Speichern fehl (Speicherplatz voll,
  Lizenz), bleibt der Raum im Speicher und versucht es erneut; die Teilnehmer bekommen `error`.
- Board löschen, Rolle ändern oder Einladung entfernen trennt betroffene Teilnehmer sofort (Close-Code 4000).
  Die Lizenz der Besitzerin/des Besitzers wird alle 30 s neu geprüft (`access`).
- Bilder in Live-Sitzungen: `PUT/GET /api/boards/{id}/images/{sha256}.{ext}` (inhaltsadressiert).
- Zusätzliche Felder/Codes, die den Vertrag nicht verletzen: `GET /api/health`; `License` enthält zusätzlich
  `active`; die 409-Antwort bei Versionskonflikt enthält die aktuelle `version`; `GET /api/admin/update` enthält
  `installing`; Fehlercodes `internal` (500), `not_implemented` (501), `upstream_error` (502, GitHub nicht erreichbar).
- Einladungslinks und Gäste (`src/invites.rs`, ab 4.7.0): Gäste sind Zeilen in `users` mit `guest=1`, Platzhalter-
  Adresse `gast:<uuid>` (nie angezeigt, nicht anmeldbar) und Anzeige-ID `<Name>#XXXX`; Links in der Tabelle `invites`
  (nur SHA-256 des Tokens). Ältere Datenbanken bekommen die Spalte `users.guest` beim Start (`db::migrate`).
  Im Admin-Panel tragen Gäste den Chip „Gast“ und können nicht Admin werden.
- Zugriff auf fremde Boards ohne Einladung liefert `404 not_found` (nicht `403`), damit Board-IDs nichts verraten.
- Ein Konto hat höchstens eine gültige Lizenz zur selben Zeit; ein zweiter Code kann erst nach Ablauf/Widerruf
  eingelöst werden. Ist die Lizenz der **Besitzerin oder des Besitzers** abgelaufen, ist das Board für alle nur noch
  lesbar (`license_required` beim Speichern).
