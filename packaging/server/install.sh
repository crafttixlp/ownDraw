#!/bin/bash
# OwnDraw-Server-Installer (als root). Gedacht für Debian 12 / Ubuntu 24.04; installiert alle nötigen Pakete selbst,
# auch unter Fedora/RHEL (dnf/yum), openSUSE (zypper), Alpine (apk) und Arch (pacman).
#
# Installiert Docker, holt das Repo nach /opt/owndraw, legt die Web-App nach dist/web/, schreibt .env,
# startet Server + Caddy (Auto-HTTPS) per docker compose und prüft den Start. Mehrfach ausführbar:
# vorhandene Einstellungen (.env) und Daten (data/) bleiben erhalten, das Repo wird aktualisiert.
#
# Aufruf:
#   sudo sh install.sh                                    # fragt Domain, Admin-E-Mail, Passwort ab
#   sudo bash install.sh --web-zip OwnDraw-Web-4.8.0-statisch.zip
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

# Ohne bash (z. B. Alpine, Aufruf mit "sh install.sh"): bash nachinstallieren und neu starten.
if [ -z "${BASH_VERSION:-}" ]; then
  if command -v apk >/dev/null 2>&1; then apk add --no-cache -q bash; fi
  command -v bash >/dev/null 2>&1 || { echo "bash fehlt – bitte installieren." >&2; exit 1; }
  exec bash "$0" "$@"
fi
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
    -h|--help) awk 'NR>1 && /^#/ {sub(/^# ?/, ""); print; next} NR>1 {exit}' "$0"; exit 0 ;;
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
PM=""
for c in apt-get dnf yum zypper apk pacman; do command -v "$c" >/dev/null 2>&1 && { PM=$c; break; }; done
[ -n "$PM" ] || fehler "Kein unterstützter Paketmanager gefunden (apt, dnf, yum, zypper, apk, pacman)."
pm_aktualisiert=0
pm_install() { # pm_install paket … (still, ohne Rückfragen)
  [ $# -gt 0 ] || return 0
  echo "Installiere: $*"
  case "$PM" in
    apt-get)
      [ "$pm_aktualisiert" = 1 ] || { apt-get update -qq; pm_aktualisiert=1; }
      apt-get install -y -qq --no-install-recommends "$@" >/dev/null ;;
    dnf|yum) "$PM" install -y -q "$@" >/dev/null ;;
    zypper)  zypper --non-interactive --quiet install --no-recommends "$@" >/dev/null ;;
    apk)     apk add --no-cache -q "$@" >/dev/null ;;
    pacman)
      [ "$pm_aktualisiert" = 1 ] || { pacman -Sy --noconfirm >/dev/null; pm_aktualisiert=1; }
      pacman -S --noconfirm --needed "$@" >/dev/null ;;
  esac
}
# Programm → Paketname je Paketmanager
paket_fuer() {
  case "$1:$PM" in
    getent:apk) echo musl-utils ;;
    getent:*) echo "" ;;                                  # gehört zu glibc, immer da
    mkswap:*|fallocate:*) echo util-linux ;;
    find:*) echo findutils ;;
    realpath:*|mktemp:*) echo coreutils ;;
    *) echo "$1" ;;
  esac
}
need=()
for prog in git curl unzip openssl awk sed find realpath mktemp getent fallocate mkswap; do
  command -v "$prog" >/dev/null 2>&1 && continue
  pk=$(paket_fuer "$prog"); [ -n "$pk" ] && need+=("$pk")
done
# CA-Zertifikate (für HTTPS-Downloads) und Hilfsprogramme, die manche Minimal-Images nicht haben
[ -s /etc/ssl/certs/ca-certificates.crt ] || [ -s /etc/pki/tls/certs/ca-bundle.crt ] || need+=(ca-certificates)
if [ ${#need[@]} -gt 0 ]; then
  # Doppelte entfernen
  mapfile -t need < <(printf '%s\n' "${need[@]}" | awk '!s[$0]++')
  pm_install "${need[@]}" || fehler "Pakete konnten nicht installiert werden: ${need[*]}"
fi
ok "Grundpakete vorhanden"

docker_dienst_starten() {
  if command -v systemctl >/dev/null 2>&1 && [ -d /run/systemd/system ]; then
    systemctl enable --now docker >/dev/null 2>&1 || true
  elif command -v rc-update >/dev/null 2>&1; then
    rc-update add docker default >/dev/null 2>&1 || true
    rc-service docker start >/dev/null 2>&1 || true
  else
    service docker start >/dev/null 2>&1 || true
  fi
}
if ! command -v docker >/dev/null 2>&1; then
  echo "Docker wird installiert …"
  case "$PM" in
    apk)    pm_install docker docker-cli-compose ;;
    pacman) pm_install docker docker-compose ;;
    zypper) pm_install docker docker-compose ;;
    *)      curl -fsSL https://get.docker.com | sh >/dev/null 2>&1 \
              || { warn "get.docker.com fehlgeschlagen, versuche Pakete der Distribution."
                   if [ "$PM" = apt-get ]; then pm_install docker.io docker-compose-v2 || pm_install docker.io docker-compose-plugin
                   else pm_install docker docker-compose-plugin || pm_install moby-engine docker-compose; fi; } ;;
  esac
  command -v docker >/dev/null 2>&1 || fehler "Docker konnte nicht installiert werden."
fi
docker_dienst_starten
if ! docker compose version >/dev/null 2>&1; then
  echo "Compose-Plugin fehlt – wird installiert …"
  case "$PM" in
    apt-get) pm_install docker-compose-plugin || pm_install docker-compose-v2 || true ;;
    dnf|yum) pm_install docker-compose-plugin || true ;;
    apk)     pm_install docker-cli-compose || true ;;
    pacman|zypper) pm_install docker-compose || true ;;
  esac
fi
if ! docker compose version >/dev/null 2>&1; then
  # Letzter Ausweg: Compose-Plugin direkt von GitHub
  arch=$(uname -m); [ "$arch" = aarch64 ] || arch=x86_64
  mkdir -p /usr/local/lib/docker/cli-plugins
  curl -fsSL "https://github.com/docker/compose/releases/latest/download/docker-compose-linux-$arch" \
    -o /usr/local/lib/docker/cli-plugins/docker-compose \
    && chmod +x /usr/local/lib/docker/cli-plugins/docker-compose
fi
docker compose version >/dev/null 2>&1 || fehler "docker compose ließ sich nicht installieren."
for _ in $(seq 1 15); do docker info >/dev/null 2>&1 && break; sleep 1; done
docker info >/dev/null 2>&1 || fehler "Docker-Dienst läuft nicht (Log: journalctl -u docker)."
ok "$(docker --version), $(docker compose version --short 2>/dev/null || echo compose)"

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
# Version des Repo-Stands; die Web-App muss dazu passen (sonst liefert der Server eine alte Web-App aus).
REPO_VER=$(awk -F'"' '/^\[workspace.package\]/{w=1} w && /^version *=/{print $2; exit}' "$DIR/Cargo.toml")
WEB_VER=$(cat "$DIR/dist/web/version.txt" 2>/dev/null || true)
if [ -z "$WEB_ZIP" ]; then
  # Nur ein Zip derselben Version automatisch nehmen (ältere Zips im Ordner werden ignoriert)
  WEB_ZIP=$(ls -t "$SCRIPT_DIR"/OwnDraw-Web-"$REPO_VER"-*.zip "$PWD"/OwnDraw-Web-"$REPO_VER"-*.zip 2>/dev/null | head -n1 || true)
  if [ -z "$WEB_ZIP" ] && ls "$SCRIPT_DIR"/OwnDraw-Web-*.zip "$PWD"/OwnDraw-Web-*.zip >/dev/null 2>&1; then
    warn "Gefundene OwnDraw-Web-*.zip passen nicht zu Version $REPO_VER und werden ignoriert."
  fi
fi
if [ -f "$DIR/dist/web/owndraw_web_bg.wasm" ] && [ "$REBUILD_WEB" = 0 ] && [ -z "$WEB_ZIP" ] && [ "$WEB_VER" = "$REPO_VER" ]; then
  ok "Version $WEB_VER vorhanden (neu erzeugen mit --rebuild-web)"
elif [ -n "$WEB_ZIP" ]; then
  [ -f "$WEB_ZIP" ] || fehler "Zip nicht gefunden: $WEB_ZIP"
  tmp=$(mktemp -d)
  unzip -q "$WEB_ZIP" -d "$tmp"
  src=$(dirname "$(find "$tmp" -name owndraw_web_bg.wasm | head -n1)")
  [ -f "$src/index.html" ] || fehler "Zip enthält keine Web-App (index.html + owndraw_web_bg.wasm)."
  zip_ver=$(cat "$src/version.txt" 2>/dev/null || true)
  [ "$zip_ver" = "$REPO_VER" ] || warn "Web-App im Zip hat Version ${zip_ver:-unbekannt}, der Server ist $REPO_VER."
  rm -rf "$DIR/dist/web" && mkdir -p "$DIR/dist" && cp -r "$src" "$DIR/dist/web"
  rm -rf "$tmp"
  ok "aus $(basename "$WEB_ZIP") (Version ${zip_ver:-unbekannt})"
else
  [ -f "$DIR/dist/web/owndraw_web_bg.wasm" ] && [ "$WEB_VER" != "$REPO_VER" ] \
    && echo "Vorhandene Web-App (Version ${WEB_VER:-unbekannt}) ist veraltet, Server-Stand ist $REPO_VER."
  echo "Keine fertige Web-App gefunden – baue sie in einem Docker-Container (dauert einige Minuten) …"
  WB_VER=$(awk '/^name = "wasm-bindgen"$/{getline; gsub(/version = |"/,""); print; exit}' "$DIR/Cargo.lock")
  docker volume create owndraw-web-build >/dev/null
  docker run --rm -v "$DIR":/src -v owndraw-web-build:/cache -w /src \
    -e CARGO_HOME=/cache/cargo -e CARGO_TARGET_DIR=/cache/target rust:1-bookworm bash -ec "
      export PATH=/cache/cargo/bin:\$PATH   # cargo install landet in CARGO_HOME/bin
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
echo "  Hinweis:      Seite im Browser einmal mit Strg+Umschalt+R neu laden (alte Version im Cache)."
if [ "$neu_env" = 1 ]; then
  echo "  Admin-Login:  $EMAIL"
  [ "${gen_pw:-0}" = 1 ] && echo "  Passwort:     $PASSWORD   (bitte notieren und nach dem ersten Login ändern)"
fi
echo "  Daten/Backup: $SRV/data/"
echo "  Logs:         cd $SRV && docker compose logs -f"
echo "  Neu ausführen aktualisiert das Repo und startet neu; Daten bleiben erhalten."
