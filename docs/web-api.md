# OwnDraw Web: Schnittstelle Web-App ↔ Server (Vertrag)

Gilt für `crates/owndraw-web` (Browser, WebAssembly) und `crates/owndraw-server` (axum + SQLite).
Domain im Betrieb: `https://owndraw.thepepelex.at` (Caddy davor, Auto-HTTPS). Alles läuft unter einer Origin.

## Rollen

| Rolle | Darf |
|---|---|
| Gast (kein Account) | Editor vollständig, Import per Upload, Speichern nur als Download (`.owndraw`, PNG, PDF …) |
| Account ohne Lizenz | wie Gast; kann zu fremden Boards eingeladen werden und dort mitarbeiten |
| Account mit gültiger Lizenz | zusätzlich Boards am Server speichern, Boards teilen, Kollaboration starten |
| Admin | Admin-Panel `/admin`: Nutzer, Lizenzcodes, Updates |

Lizenz gültig = eingelöst, nicht widerrufen, `expires_at` leer oder in der Zukunft. Läuft sie ab, bleiben Boards
erhalten, sind aber nur noch lesbar/herunterladbar (kein Speichern, kein Teilen).

## Auslieferung (Pfade)

- `/` → Web-App (statischer Ordner `web/`: `index.html`, `owndraw_web.js`, `owndraw_web_bg.wasm`, Icons)
- `/admin` → Admin-Panel (eigenes kleines HTML/JS, im Server-Binary eingebettet)
- `/api/...` → JSON-API, unten
- `/api/collab/{board_id}` → WebSocket (Phase 4, nicht Teil der ersten Server-Version)

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

```json
Me = {"id":"…","email":"…","name":"…","admin":false,
      "license":{"active":true,"expires_at":1790000000,"quota_mb":500,"used_mb":12.3} | null}
```

## Boards (nur mit gültiger Lizenz für Anlegen/Speichern; Lesen auch für Eingeladene)

Board-Inhalt = die `.owndraw`-Datei (ZIP, Format wie Desktop/Java 2.x) als Binärdaten.

| Methode | Pfad | |
|---|---|---|
| GET | `/api/boards` | → `[BoardInfo]` (eigene + geteilte) |
| POST | `/api/boards` | Body: rohe `.owndraw`-Bytes (`application/octet-stream`), Header `X-Board-Name` (URL-kodiert) → `BoardInfo` |
| GET | `/api/boards/{id}` | → rohe `.owndraw`-Bytes, Header `X-Board-Version` |
| PUT | `/api/boards/{id}` | rohe Bytes, Header `If-Match: <version>` → `BoardInfo`; 409 bei Versionskonflikt |
| PATCH | `/api/boards/{id}` | `{name}` → `BoardInfo` |
| DELETE | `/api/boards/{id}` | nur Besitzer → `{}` |
| GET | `/api/boards/{id}/members` | → `[{user_id,email,name,role}]` |
| POST | `/api/boards/{id}/members` | `{email, role:"edit"\|"view"}` → Liste; nur Besitzer mit Lizenz |
| DELETE | `/api/boards/{id}/members/{user_id}` | → Liste |

```json
BoardInfo = {"id":"…","name":"…","owner":{"id":"…","name":"…"},"role":"owner"|"edit"|"view",
             "version":7,"size":12345,"modified":1790000000}
```

Max. Größe pro Board 50 MB (einstellbar). Speicherquote der Lizenz zählt nur eigene Boards.

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
