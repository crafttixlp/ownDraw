# OwnDraw Web: Schnittstelle Web-App ↔ Server (Vertrag)

Gilt für `crates/owndraw-web` (Browser, WebAssembly) und `crates/owndraw-server` (axum + SQLite).
Domain im Betrieb: `https://owndraw.thepepelex.at` (Caddy davor, Auto-HTTPS). Alles läuft unter einer Origin.

## Rollen

| Rolle | Darf |
|---|---|
| Gast (kein Account) | Editor vollständig, Import per Upload, Speichern nur als Download (`.owndraw`, PNG, PDF …) |
| Gast mit Gast-ID | über einen Einladungslink beigetreten: eigene ID `<Name>#XXXX`, arbeitet an den eingeladenen Boards live mit (Rolle des Links); keine eigenen Boards am Server, kein Passwort, keine Lizenz |
| Account ohne Lizenz | wie Gast; kann zu fremden Boards eingeladen werden und dort mitarbeiten |
| Account mit gültiger Lizenz | zusätzlich Boards am Server speichern, Boards teilen, Kollaboration starten |
| Admin | Admin-Panel `/admin`: Nutzer, Lizenzcodes, Updates |

Lizenz gültig = eingelöst, nicht widerrufen, `expires_at` leer oder in der Zukunft. Läuft sie ab, bleiben Boards
erhalten, sind aber nur noch lesbar/herunterladbar (kein Speichern, kein Teilen).

## Auslieferung (Pfade)

- `/` → Web-App (statischer Ordner `web/`: `index.html`, `owndraw_web.js`, `owndraw_web_bg.wasm`, Icons)
- `/admin` → Admin-Panel (eigenes kleines HTML/JS, im Server-Binary eingebettet)
- `/api/...` → JSON-API, unten
- `/api/collab/{board_id}` → WebSocket für Live-Zusammenarbeit (siehe „Kollaboration“)

## Allgemein

- JSON (`Content-Type: application/json`), UTF-8. Fehler: HTTP-Status + `{"error":"code","message":"Text auf Deutsch"}`.
  Codes: `unauthorized` (401), `forbidden` (403), `license_required` (403), `not_found` (404), `conflict` (409),
  `invalid` (400), `too_large` (413), `quota` (507), `rate_limited` (429).
- Anmeldung per Cookie `owndraw_session` (HttpOnly, Secure, SameSite=Lax, 30 Tage). Schreibende Anfragen
  brauchen zusätzlich Header `X-OwnDraw: 1` (CSRF-Schutz; einfache Formulare können ihn nicht setzen).
- Zeiten: Unix-Sekunden (i64). IDs: UUID-Strings.

## Konto

| Methode | Pfad | Body → Antwort |
|---|---|---|
| POST | `/api/auth/register` | `{email, password, name}` → `Me` (setzt Cookie). Passwort ≥ 8 Zeichen. |
| POST | `/api/auth/login` | `{email, password}` → `Me` (setzt Cookie) |
| POST | `/api/auth/logout` | – → `{}` |
| GET | `/api/me` | → `Me` oder 401 |
| POST | `/api/me/password` | `{old, new}` → `{}` |
| POST | `/api/license/redeem` | `{code}` → `Me` |

Gäste (siehe „Einladungslinks“) melden sich nicht an: `login` lehnt sie ab, `me/password` und `license/redeem`
antworten 403. `logout` eines Gastes löscht ihn samt Mitgliedschaften. `register`/`login` mit Gast-Cookie
übernehmen dessen Mitgliedschaften ins Konto (bei doppelter gewinnt `edit`) und löschen den Gast.

```json
Me = {"id":"…","email":"…" (Gast: ""),"name":"…" (Gast: "Mila#K7Q2"),"admin":false,"guest":false,
      "license":{"active":true,"expires_at":1790000000,"quota_mb":500,"used_mb":12.3} | null}
```

## Boards (nur mit gültiger Lizenz für Anlegen/Speichern; Lesen auch für Eingeladene)

Board-Inhalt = die `.owndraw`-Datei (ZIP, Format wie Desktop/Java 2.x) als Binärdaten.

| Methode | Pfad | |
|---|---|---|
| GET | `/api/boards` | → `[BoardInfo]` (eigene + geteilte) |
| POST | `/api/boards` | Body: rohe `.owndraw`-Bytes (`application/octet-stream`), Header `X-Board-Name` (URL-kodiert) → `BoardInfo` |
| GET | `/api/boards/{id}` | → rohe `.owndraw`-Bytes, Header `X-Board-Version` |
| PUT | `/api/boards/{id}` | rohe Bytes, Header `If-Match: <version>` → `BoardInfo`; 409 bei Versionskonflikt (`{…,"version":n}`), 409 mit `"live":true`, solange eine Live-Sitzung läuft |
| PATCH | `/api/boards/{id}` | `{name}` → `BoardInfo` |
| DELETE | `/api/boards/{id}` | nur Besitzer → `{}` |
| GET | `/api/boards/{id}/members` | → `[{user_id,email,name,role,guest}]` (Gäste mit `email:""`) |
| POST | `/api/boards/{id}/members` | `{email, role:"edit"\|"view"}` → Liste; nur Besitzer mit Lizenz |
| DELETE | `/api/boards/{id}/members/{user_id}` | → Liste |
| PUT | `/api/boards/{id}/images/{sha256}.{ext}` | rohe Bildbytes, nur während einer Live-Sitzung; `owner`/`edit`, Lizenz der Besitzerin/des Besitzers |
| GET | `/api/boards/{id}/images/{sha256}.{ext}` | → Bild (aus der Live-Sitzung oder der gespeicherten Datei), 404 solange noch nicht hochgeladen |

```json
BoardInfo = {"id":"…","name":"…","owner":{"id":"…","name":"…"},"role":"owner"|"edit"|"view",
             "version":7,"size":12345,"modified":1790000000}
```

## Einladungslinks (auch für Gäste ohne Konto)

Link = `https://<host>/#invite=<token>` (Token: 48 Hex-Zeichen, im Fragment, damit er nicht in Server-Logs landet;
in der DB nur sein SHA-256). Verwalten nur Besitzer mit gültiger Lizenz, höchstens 20 Links je Board.

| Methode | Pfad | |
|---|---|---|
| GET | `/api/boards/{id}/invites` | → `[Invite]` (ohne Token) |
| POST | `/api/boards/{id}/invites` | `{role:"edit"\|"view", days}` oder `{role, hours}` → `Invite` mit `token` (nur hier). `days` fehlt → 7, `null` → unbefristet, sonst 1–365; `hours` 1–8760 (hat Vorrang) |
| DELETE | `/api/boards/{id}/invites/{invite_id}` | → `[Invite]` (Link ungültig; schon Beigetretene bleiben Mitglied) |
| GET | `/api/invites/{token}` | ohne Anmeldung → `{board:{id,name}, owner:{name}, role}`; 404 wenn ungültig/abgelaufen |
| POST | `/api/invites/{token}/join` | `{name}` → `{me: Me, board: BoardInfo}`. Angemeldet: wird Mitglied (eine `edit`-Rolle wird nicht herabgestuft). Nicht angemeldet: neuer Gast, `name` 1–24 Zeichen (ohne `#`), ID `<name>#XXXX` (eindeutig), setzt Cookie (30 Tage). 403 `license_required`, wenn die Lizenz der Besitzerin/des Besitzers abgelaufen ist |

```json
Invite = {"id":"…","role":"edit","created":1790000000,"expires_at":1790604800|null,"uses":2}
```

Missbrauchsschutz: neue Gäste zählen auf das Registrierungs-Limit je IP (10 je 10 Minuten),
`GET /api/invites/…` und `join` auf ein fünffaches Limit. Gäste ohne gültige Sitzung werden beim Anlegen neuer
Gäste gelöscht.

Max. Größe pro Board 50 MB (einstellbar). Der Speicherplatz der Lizenz (`quota_mb`) zählt nur eigene Boards.

## Kollaboration (WebSocket `GET /api/collab/{board_id}`)

Voraussetzungen: angemeldet (Cookie), Board sichtbar (sonst 404), `Origin` gleich `Host` (sonst 403). Mitlesen darf
jede Rolle; schreiben dürfen `owner`/`edit`, und nur solange die **Besitzerin/der Besitzer** eine gültige Lizenz hat
(geprüft beim Verbinden und alle 30 s). Alle Nachrichten sind JSON-Textframes mit Feld `t`.

**Objekt-IDs**: jedes Objekt hat in der `.owndraw`-Datei ein Feld `"uid"` (16 Hex-Zeichen, zufällig). Ältere
Dateien ohne `uid` (oder mit doppelten) bekommen beim Laden neue. Andere Programme ignorieren das Feld.

Client → Server:

| `t` | Felder | |
|---|---|---|
| `ops` | `cid` (beliebig, vom Client), `ops: [Op]` | Änderungen; Antwort an alle (auch Absender) als `ops`, bei Fehler `nack` nur an den Absender |
| `cursor` | `x`, `y` (Board-Koordinaten, `null` = ausblenden) | wird an die anderen weitergereicht |
| `ping` | – | → `pong` |

Server → Client:

| `t` | Felder |
|---|---|
| `hello` | `proto:1`, `you:{peer,user_id,name,color,role}`, `can_write`, `seq`, `version`, `peers:[Peer]`, `board:{items,layers,bg,dark,…}` (wie `board.json`), `mode` (Seitenmodus wie im Manifest) |
| `ops` | `seq` (fortlaufend), `peer`, `cid`, `ops` – in der Reihenfolge, in der der Server sie angewendet hat |
| `nack` | `cid`, `code` (`forbidden`, `license_required`, `invalid`), `message` |
| `join` / `leave` | `peer` (Objekt bzw. Nummer) |
| `cursor` | `peer`, `x`, `y` |
| `access` | `can_write` (z. B. Lizenz abgelaufen oder wieder gültig) |
| `saved` | `version` (Board wurde gespeichert) |
| `error` | `code`, `message`; danach ggf. Schließen mit Code `4000`, Grund `not_found` (Board gelöscht), `role_changed`, `removed` |
| `pong` | – |

```json
Peer = {"peer":3,"user_id":"…","name":"Ben","color":"#3e63dd","role":"edit"}
Op   = {"o":"up","item":{… Objekt-JSON mit "uid" …},"after":"<uid>"|null}   // einfügen/ersetzen, über "after" (null = ganz unten)
     | {"o":"mv","uid":"…","after":"<uid>"|null}                            // Reihenfolge ändern
     | {"o":"del","uid":"…"}
     | {"o":"board","layers":[…],"mode":{…},"bg":"…","dark":false}         // Ebenen, Seiten, Hintergrund
```

Regeln: Der Server ist die Reihenfolge-Instanz; gleiches Objekt gleichzeitig bearbeitet → letzter gewinnt. Fehlt der
Vorgänger von `after`, bleibt ein vorhandenes Objekt an seinem Platz bzw. kommt ein neues nach oben. Bilder
(`image`-Objekte, Name `<sha256>.<ext>`) lädt der Client vor der Operation per `PUT …/images/…` hoch. Der Server
speichert geänderte Räume höchstens alle 5 s, wenn der letzte Teilnehmer geht und beim Beenden/Update (neue
`version`, Vorschaubild). Während eine Sitzung läuft, antwortet `PUT /api/boards/{id}` mit 409 `live:true`.

## Admin (`admin:true`)

| Methode | Pfad | |
|---|---|---|
| GET | `/api/admin/users` | → `[{id,email,name,admin,disabled,created,license,used_mb,boards}]` |
| PATCH | `/api/admin/users/{id}` | `{disabled?, admin?}` |
| DELETE | `/api/admin/users/{id}` | löscht Nutzer + eigene Boards |
| GET | `/api/admin/licenses` | → `[License]` |
| POST | `/api/admin/licenses` | `{count, days:null\|n, quota_mb, note}` → `[License]` (neu erzeugte Codes) |
| PATCH | `/api/admin/licenses/{code}` | `{revoked?, expires_at?, quota_mb?, note?}` |
| GET | `/api/admin/update` | → `{current, latest, available, auto:false, auto_hour:3, last_check, log:[…]}` |
| POST | `/api/admin/update/check` | sucht neue Version |
| POST | `/api/admin/update/install` | installiert neueste Version (Backup, Austausch, Neustart) |
| PATCH | `/api/admin/update` | `{auto, auto_hour}` |

```json
License = {"code":"OWND-XXXX-XXXX-XXXX","created":…,"days":365|null,"expires_at":…|null,"quota_mb":500,
           "note":"…","revoked":false,"redeemed_by":{"id","email"}|null,"redeemed_at":…|null}
```

Laufzeit `days` beginnt beim Einlösen. Erster Admin: Umgebungsvariablen `OWNDRAW_ADMIN_EMAIL` +
`OWNDRAW_ADMIN_PASSWORD` beim ersten Start (oder `owndraw-server admin create`).

## Server-Update

Releases auf GitHub `crafttixlp/ownDraw`, Asset `owndraw-server-<v>-x86_64-linux.tar.gz` (+ `.minisig`, gleicher
minisign-Schlüssel wie die Desktop-App, `crates/owndraw/src/update/minisign.pub`). Inhalt: Binary `owndraw-server` +
Ordner `web/`. Ablauf: herunterladen → Signatur prüfen → DB-Backup → nach `data/versions/<v>/` entpacken →
`data/current` umstellen → Prozess beendet sich, Docker (`restart: unless-stopped`) startet neu. Ein kleiner
Start-Wrapper nimmt `data/current`; startet die neue Version nicht (Health-Check fehlschlägt), zurück auf die alte.
Auto-Update standardmäßig **aus**.
