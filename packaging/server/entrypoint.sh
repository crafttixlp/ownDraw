#!/bin/sh
# Start-Wrapper für den OwnDraw-Server im Container.
#
# - Nimmt data/current/owndraw-server (per Self-Update installiert), falls vorhanden und nicht älter als das
#   mitgelieferte Binary (z. B. nach einem neuen Docker-Image); sonst das mitgelieferte Binary.
# - Wartet nach dem Start auf den Health-Check GET /api/health -> {"ok":true,"version":...}.
# - Startet die neue Version nicht (Prozess endet oder Health-Check schlägt fehl), wird data/current auf die
#   vorige Version (data/previous) zurückgestellt bzw. entfernt (dann gilt das mitgelieferte Binary) und neu gestartet.
# - Nach einem Update beendet sich der Server selbst; der Wrapper endet mit demselben Code, und Docker
#   (restart: unless-stopped) startet den Container neu.

DATA="${OWNDRAW_DATA:-/app/data}"
BUNDLED_BIN="${OWNDRAW_BUNDLED_BIN:-/app/owndraw-server}"
BUNDLED_WEB="${OWNDRAW_BUNDLED_WEB:-/app/web}"
BIND="${OWNDRAW_BIND:-0.0.0.0:8080}"
PORT="${BIND##*:}"
HEALTH_URL="http://127.0.0.1:${PORT}/api/health"
HEALTH_TIMEOUT="${OWNDRAW_HEALTH_TIMEOUT:-30}"
CUR="$DATA/current"
PREV="$DATA/previous"

log() { echo "[entrypoint] $*" >&2; }

version_of() { "$1" --version 2>/dev/null | head -n 1; }

# Wahr, wenn Version $1 neuer ist als $2.
newer() {
  [ "$1" != "$2" ] && [ "$(printf '%s\n%s\n' "$1" "$2" | sort -V | tail -n 1)" = "$1" ]
}

# Setzt BIN, WEBDIR, SRC.
pick() {
  BIN="$BUNDLED_BIN"; WEBDIR="$BUNDLED_WEB"; SRC=bundled
  [ -x "$CUR/owndraw-server" ] || return 0
  cv=$(version_of "$CUR/owndraw-server")
  bv=$(version_of "$BUNDLED_BIN")
  if [ -z "$cv" ]; then
    log "data/current ist nicht lauffähig, nehme das mitgelieferte Binary"
  elif [ -n "$bv" ] && newer "$bv" "$cv"; then
    log "Mitgeliefertes Binary ($bv) ist neuer als data/current ($cv); data/current wird entfernt"
    rm -f "$CUR"
  else
    BIN="$CUR/owndraw-server"; SRC=current
    [ -d "$CUR/web" ] && WEBDIR="$CUR/web"
  fi
}

# Stellt data/current auf die vorige Version zurück (oder entfernt es).
rollback() {
  failed=$(readlink "$CUR" 2>/dev/null)
  if [ -L "$PREV" ] && [ -x "$PREV/owndraw-server" ]; then
    target=$(readlink "$PREV")
    ln -s "$target" "$CUR.new" && mv -T "$CUR.new" "$CUR"
    rm -f "$PREV"
    log "Zurück auf $target (fehlgeschlagen: $failed). Datenbank-Sicherungen liegen in $DATA/backups/"
  else
    rm -f "$CUR"
    log "Zurück auf das mitgelieferte Binary (fehlgeschlagen: $failed). Datenbank-Sicherungen liegen in $DATA/backups/"
  fi
}

# Startet den Server. Rückgabe: Exit-Code des Servers; 200, wenn er nie gesund wurde.
run_once() {
  OWNDRAW_WEB="$WEBDIR" "$BIN" &
  PID=$!
  trap 'kill -TERM "$PID" 2>/dev/null' TERM INT
  healthy=0; i=0
  while [ "$i" -lt "$HEALTH_TIMEOUT" ]; do
    kill -0 "$PID" 2>/dev/null || break
    if curl -fsS --max-time 2 "$HEALTH_URL" 2>/dev/null | grep -q '"ok":true'; then healthy=1; break; fi
    sleep 1; i=$((i + 1))
  done
  if [ "$healthy" -ne 1 ]; then
    kill -TERM "$PID" 2>/dev/null
    wait "$PID" 2>/dev/null
    trap - TERM INT
    return 200
  fi
  wait "$PID"; rc=$?
  # Ein weitergeleitetes Signal unterbricht wait; auf das Ende des Servers warten.
  while kill -0 "$PID" 2>/dev/null; do wait "$PID"; rc=$?; done
  trap - TERM INT
  return "$rc"
}

attempt=0
while [ "$attempt" -lt 3 ]; do
  attempt=$((attempt + 1))
  pick
  log "Starte $BIN ($SRC, Version $(version_of "$BIN"))"
  run_once
  rc=$?
  if [ "$rc" -ne 200 ]; then
    exit "$rc"
  fi
  log "Server wurde nicht gesund (Health-Check $HEALTH_URL)"
  if [ "$SRC" = "bundled" ]; then
    log "Das mitgelieferte Binary startet nicht; Abbruch"
    exit 1
  fi
  rollback
done
log "Zu viele fehlgeschlagene Starts; Abbruch"
exit 1
