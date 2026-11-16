#!/bin/bash
# OwnDraw-Server-Installer (Debian 12 / Ubuntu 24.04, als root).
#
# Installiert Docker, holt das Repo nach /opt/owndraw, legt die Web-App nach dist/web/, schreibt .env,
# startet Server + Caddy (Auto-HTTPS) per docker compose und prüft den Start. Mehrfach ausführbar:
# vorhandene Einstellungen (.env) und Daten (data/) bleiben erhalten, das Repo wird aktualisiert.
#
# Aufruf:
#   sudo bash install.sh                                  # fragt Domain, Admin-E-Mail, Passwort ab
#   sudo bash install.sh --web-zip OwnDraw-Web-4.6.0-statisch.zip
#   sudo bash install.sh --domain owndraw.thepepelex.at --email ich@example.org --password '…' --yes
#
# Optionen:
#   --dir PFAD        Installationsordner (Standard /opt/owndraw)
#   --branch NAME     Git-Branch (Standard main)
#   --web-zip DATEI   fertige Web-App (Zip mit Ordner web/). Ohne Angabe: OwnDraw-Web-*.zip neben diesem
#                     Skript bzw. im aktuellen Ordner; sonst wird die Web-App in einem Docker-Container gebaut.
#   --domain D        Domain (Standard owndraw.thepepelex.at)
#   --email E         Admin-E-Mail
#   --password P      Admin-Passwort (leer = zufällig erzeugen und anzeigen)
#   --yes             keine Rückfragen (Swap anlegen, vorhandene .env behalten)
#   --rebuild-web     Web-App neu bauen/entpacken, auch wenn dist/web/ schon existiert
set -euo pipefail

REPO_URL="https://github.com/crafttixlp/ownDraw.git"
DIR=/opt/owndraw
BRANCH=main
WEB_ZIP=""
DOMAIN=""
EMAIL=""
PASSWORD=""
YES=0
REBUILD_WEB=0
SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"

while [ $# -gt 0 ]; do
  case "$1" in
    --dir) DIR="${2:?Ordner fehlt}"; shift 2 ;;
    --branch) BRANCH="${2:?Branch fehlt}"; shift 2 ;;
    --web-zip) WEB_ZIP="$(realpath "${2:?Datei fehlt}")"; shift 2 ;;
    --domain) DOMAIN="${2:?Domain fehlt}"; shift 2 ;;
    --email) EMAIL="${2:?E-Mail fehlt}"; shift 2 ;;
    --password) PASSWORD="${2-}"; shift 2 ;;
    --yes|-y) YES=1; shift ;;
    --rebuild-web) REBUILD_WEB=1; shift ;;
    -h|--help) sed -n '2,24p' "$0" | sed 's/^# \{0,1\}//'; exit 0 ;;
    *) echo "Unbekannte Option: $1 (Hilfe: --help)" >&2; exit 2 ;;
  esac
done

schritt() { printf '\n\033[1;34m==> %s\033[0m\n' "$*"; }
ok()      { printf '\033[32m✓ %s\033[0m\n' "$*"; }
warn()    { printf '\033[33m! %s\033[0m\n' "$*" >&2; }
fehler()  { printf '\033[31m✗ %s\033[0m\n' "$*" >&2; exit 1; }
frage() { # frage "Text" Standard → Antwort
  local a
  if [ "$YES" = 1 ] || [ ! -t 0 ]; then echo "$2"; return; fi
  read -r -p "$1 [$2]: " a </dev/tty || true
  echo "${a:-$2}"
}
ja() { # ja "Frage" → 0 bei ja (Standard ja)
  [ "$YES" = 1 ] && return 0
  [ -t 0 ] || return 0
  local a; read -r -p "$1 [J/n]: " a </dev/tty || true
  [[ ! "$a" =~ ^[nN] ]]
}

[ "$(id -u)" = 0 ] || fehler "Bitte als root ausführen (sudo bash install.sh)."
[ "$(uname -m)" = x86_64 ] || warn "Getestet nur auf x86_64, gefunden: $(uname -m)."

# ---------------------------------------------------------------- Pakete + Docker
schritt "Pakete und Docker"
export DEBIAN_FRONTEND=noninteractive
need=()
for p in git curl unzip openssl; do
  command -v "$p" >/dev/null 2>&1 || need+=("$p")
done
dpkg -s ca-certificates >/dev/null 2>&1 || need+=(ca-certificates)
if [ ${#need[@]} -gt 0 ]; then
  apt-get update -qq
  apt-get install -y -qq "${need[@]}" >/dev/null
fi
if ! command -v docker >/dev/null 2>&1; then
  echo "Docker wird installiert (offizielles Skript get.docker.com) …"
  curl -fsSL https://get.docker.com | sh >/dev/null
fi
docker compose version >/dev/null 2>&1 || fehler "docker compose fehlt (Paket docker-compose-plugin installieren)."
systemctl enable --now docker >/dev/null 2>&1 || true
ok "$(docker --version)"

# ---------------------------------------------------------------- Swap bei wenig RAM
mem_mb=$(awk '/MemTotal/{print int($2/1024)}' /proc/meminfo)
swap_mb=$(awk '/SwapTotal/{print int($2/1024)}' /proc/meminfo)
if [ "$mem_mb" -lt 3000 ] && [ "$swap_mb" -lt 1500 ] && [ ! -f /swapfile ]; then
  if ja "Nur ${mem_mb} MB RAM und kaum Swap – für den Rust-Build 2 GB Swap-Datei /swapfile anlegen?"; then
    fallocate -l 2G /swapfile 2>/dev/null || dd if=/dev/zero of=/swapfile bs=1M count=2048 status=none
    chmod 600 /swapfile && mkswap /swapfile >/dev/null && swapon /swapfile
    grep -q '^/swapfile ' /etc/fstab || echo '/swapfile none swap sw 0 0' >> /etc/fstab
    ok "Swap angelegt"
  fi
fi

# ---------------------------------------------------------------- Repo
schritt "Repository nach $DIR"
if [ -d "$DIR/.git" ]; then
  git -C "$DIR" fetch -q origin "$BRANCH"
  git -C "$DIR" checkout -q "$BRANCH" 2>/dev/null || git -C "$DIR" checkout -q -b "$BRANCH" "origin/$BRANCH"
  git -C "$DIR" merge -q --ff-only "origin/$BRANCH" || warn "Lokale Änderungen in $DIR – Repo nicht aktualisiert."
else
  [ -e "$DIR" ] && [ -n "$(ls -A "$DIR" 2>/dev/null)" ] && fehler "$DIR existiert und ist kein Git-Repo."
  git clone -q --branch "$BRANCH" "$REPO_URL" "$DIR"
fi
ok "Stand $(git -C "$DIR" log -1 --format='%h %s')"
SRV="$DIR/packaging/server"

# ---------------------------------------------------------------- Web-App
schritt "Web-App (dist/web/)"
if [ -z "$WEB_ZIP" ]; then
  WEB_ZIP=$(ls -t "$SCRIPT_DIR"/OwnDraw-Web-*.zip "$PWD"/OwnDraw-Web-*.zip 2>/dev/null | head -n1 || true)
fi
if [ -f "$DIR/dist/web/owndraw_web_bg.wasm" ] && [ "$REBUILD_WEB" = 0 ] && [ -z "$WEB_ZIP" ]; then
  ok "vorhanden (neu erzeugen mit --rebuild-web)"
elif [ -n "$WEB_ZIP" ]; then
  [ -f "$WEB_ZIP" ] || fehler "Zip nicht gefunden: $WEB_ZIP"
  tmp=$(mktemp -d)
  unzip -q "$WEB_ZIP" -d "$tmp"
  src=$(dirname "$(find "$tmp" -name owndraw_web_bg.wasm | head -n1)")
  [ -f "$src/index.html" ] || fehler "Zip enthält keine Web-App (index.html + owndraw_web_bg.wasm)."
  rm -rf "$DIR/dist/web" && mkdir -p "$DIR/dist" && cp -r "$src" "$DIR/dist/web"
  rm -rf "$tmp"
  ok "aus $(basename "$WEB_ZIP")"
else
  echo "Keine fertige Web-App gefunden – baue sie in einem Docker-Container (dauert einige Minuten) …"
  WB_VER=$(awk '/^name = "wasm-bindgen"$/{getline; gsub(/version = |"/,""); print; exit}' "$DIR/Cargo.lock")
  docker volume create owndraw-web-build >/dev/null
  docker run --rm -v "$DIR":/src -v owndraw-web-build:/cache -w /src \
    -e CARGO_HOME=/cache/cargo -e CARGO_TARGET_DIR=/cache/target rust:1-bookworm bash -ec "
      rustup target add wasm32-unknown-unknown >/dev/null
      command -v wasm-bindgen >/dev/null && [ \"\$(wasm-bindgen --version | awk '{print \$2}')\" = '$WB_VER' ] \
        || cargo install -q wasm-bindgen-cli --version '$WB_VER' --locked
      ./build-web.sh"
  [ -f "$DIR/dist/web/owndraw_web_bg.wasm" ] || fehler "Bau der Web-App fehlgeschlagen."
  ok "gebaut"
fi

# ---------------------------------------------------------------- .env
schritt "Konfiguration ($SRV/.env)"
envget() { [ -f "$SRV/.env" ] && sed -n "s/^$1=//p" "$SRV/.env" | tail -n1 || true; }
neu_env=1
if [ -f "$SRV/.env" ]; then
  if [ -z "$DOMAIN$EMAIL$PASSWORD" ] && { [ "$YES" = 1 ] || ja "Vorhandene .env behalten?"; }; then
    neu_env=0
    ok "vorhandene .env behalten"
  fi
fi
if [ "$neu_env" = 1 ]; then
  DOMAIN=${DOMAIN:-$(frage "Domain" "$(envget OWNDRAW_DOMAIN | grep . || echo owndraw.thepepelex.at)")}
  EMAIL=${EMAIL:-$(frage "Admin-E-Mail" "$(envget OWNDRAW_ADMIN_EMAIL | grep -v example || true)")}
  [ -n "$EMAIL" ] || fehler "Admin-E-Mail fehlt (--email)."
  gen_pw=0
  if [ -z "$PASSWORD" ] && [ "$YES" = 0 ] && [ -t 0 ]; then
    read -r -s -p "Admin-Passwort (mind. 8 Zeichen, leer = zufällig): " PASSWORD </dev/tty || true; echo
  fi
  if [ -z "$PASSWORD" ]; then PASSWORD=$(openssl rand -base64 18 | tr -d '/+=' | cut -c1-20); gen_pw=1; fi
  [ ${#PASSWORD} -ge 8 ] || fehler "Passwort zu kurz (mind. 8 Zeichen)."
  [ -f "$SRV/.env" ] && cp "$SRV/.env" "$SRV/.env.bak"
  umask 077
  cat > "$SRV/.env" <<EOF
# Von install.sh geschrieben. Der Admin wird beim ersten Start angelegt, falls es noch keinen gibt.
OWNDRAW_DOMAIN=$DOMAIN
OWNDRAW_ADMIN_EMAIL=$EMAIL
OWNDRAW_ADMIN_PASSWORD=$PASSWORD
OWNDRAW_MAX_BOARD_MB=$(envget OWNDRAW_MAX_BOARD_MB | grep . || echo 50)
EOF
  umask 022
  ok ".env geschrieben"
fi
DOMAIN=$(envget OWNDRAW_DOMAIN); DOMAIN=${DOMAIN:-owndraw.thepepelex.at}
mkdir -p "$SRV/data"
chown -R 10001:10001 "$SRV/data"

# ---------------------------------------------------------------- DNS-Hinweis
myip=$(curl -4 -fsS --max-time 5 https://api.ipify.org 2>/dev/null || true)
dnsip=$(getent ahostsv4 "$DOMAIN" 2>/dev/null | awk 'NR==1{print $1}' || true)
if [ -z "$dnsip" ]; then
  warn "$DOMAIN löst (noch) nicht auf. A-Eintrag auf ${myip:-die Server-IP} setzen, sonst bekommt Caddy kein Zertifikat."
elif [ -n "$myip" ] && [ "$dnsip" != "$myip" ]; then
  warn "$DOMAIN zeigt auf $dnsip, dieser Server hat $myip. A-Eintrag prüfen."
else
  ok "DNS: $DOMAIN → $dnsip"
fi

# ---------------------------------------------------------------- Start
schritt "Server bauen und starten (erster Build dauert einige Minuten)"
cd "$SRV"
docker compose up -d --build
schritt "Start prüfen"
health=""
for _ in $(seq 1 60); do
  health=$(docker compose exec -T caddy wget -qO- http://owndraw:8080/api/health 2>/dev/null || true)
  [[ "$health" == *'"ok":true'* ]] && break
  sleep 2
done
[[ "$health" == *'"ok":true'* ]] || { docker compose logs --tail 40 owndraw >&2; fehler "Server antwortet nicht."; }
ok "Server läuft: $health"
if curl -fsS --max-time 20 "https://$DOMAIN/api/health" >/dev/null 2>&1; then
  ok "HTTPS erreichbar: https://$DOMAIN"
else
  warn "https://$DOMAIN noch nicht erreichbar (DNS/Zertifikat brauchen evtl. ein paar Minuten; Log: docker compose -f $SRV/docker-compose.yml logs caddy)."
fi

schritt "Fertig"
echo "  App:          https://$DOMAIN"
echo "  Admin-Panel:  https://$DOMAIN/admin"
if [ "$neu_env" = 1 ]; then
  echo "  Admin-Login:  $EMAIL"
  [ "${gen_pw:-0}" = 1 ] && echo "  Passwort:     $PASSWORD   (bitte notieren und nach dem ersten Login ändern)"
fi
echo "  Daten/Backup: $SRV/data/"
echo "  Logs:         cd $SRV && docker compose logs -f"
echo "  Neu ausführen aktualisiert das Repo und startet neu; Daten bleiben erhalten."
