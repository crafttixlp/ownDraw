#!/usr/bin/env bash
# Baut das Server-Release-Paket owndraw-server-<version>-x86_64-linux.tar.gz (Binary + web/) und signiert es
# mit minisign, wenn ein Schlüssel angegeben ist. Das Paket (und die .minisig) werden als Asset an das
# GitHub-Release gehängt; der Server-Self-Update lädt sie von dort.
#
# Verwendung (im Repo-Wurzelordner, vorher ./build-web.sh):
#   packaging/server/release-server.sh [-k geheimer.key] [-w dist/web] [-o dist/server] [--docker]
#
#   -k  minisign-Geheimschlüssel (minisign -S -s <Schlüssel>); ohne -k wird nicht signiert
#   -w  Ordner der gebauten Web-App (Standard dist/web, muss index.html enthalten)
#   -o  Ausgabeordner (Standard dist/server)
#   --docker  Binary im Docker-Build-Stage bauen (gleiche glibc wie das Laufzeit-Image, empfohlen)
set -euo pipefail

KEY=""; WEB="dist/web"; OUT="dist/server"; USE_DOCKER=0
while [ $# -gt 0 ]; do
  case "$1" in
    -k|--key) KEY="${2:?Schlüssel fehlt}"; shift 2 ;;
    -w|--web) WEB="${2:?Ordner fehlt}"; shift 2 ;;
    -o|--out) OUT="${2:?Ordner fehlt}"; shift 2 ;;
    --docker) USE_DOCKER=1; shift ;;
    -h|--help) sed -n '2,13p' "$0" | sed 's/^# \{0,1\}//'; exit 0 ;;
    *) echo "Unbekannte Option: $1" >&2; exit 2 ;;
  esac
done

cd "$(dirname "$0")/../.."
VERSION=$(sed -n '/^\[workspace.package\]/,/^\[/{s/^version *= *"\(.*\)"/\1/p}' Cargo.toml | head -n 1)
[ -n "$VERSION" ] || { echo "Version nicht in Cargo.toml gefunden" >&2; exit 1; }
[ -f "$WEB/index.html" ] || { echo "$WEB/index.html fehlt: erst ./build-web.sh ausführen (oder -w angeben)" >&2; exit 1; }
[ -z "$KEY" ] || [ -f "$KEY" ] || { echo "Schlüssel $KEY nicht gefunden" >&2; exit 1; }

NAME="owndraw-server-${VERSION}-x86_64-linux.tar.gz"
STAGE=$(mktemp -d)
trap 'rm -rf "$STAGE"' EXIT
mkdir -p "$OUT" "$STAGE/pkg"

if [ "$USE_DOCKER" -eq 1 ]; then
  echo "== Binary im Docker-Build bauen"
  docker build -f packaging/server/Dockerfile --target binary --output "type=local,dest=$STAGE/bin" .
  cp "$STAGE/bin/owndraw-server" "$STAGE/pkg/owndraw-server"
else
  echo "== Binary bauen (cargo build --release)"
  cargo build --release --locked -p owndraw-server
  cp target/release/owndraw-server "$STAGE/pkg/owndraw-server"
fi
chmod 755 "$STAGE/pkg/owndraw-server"
cp -r "$WEB" "$STAGE/pkg/web"

echo "== Paket $NAME"
tar --sort=name --owner=0 --group=0 --numeric-owner --mtime='UTC 2020-01-01' \
    -czf "$OUT/$NAME" -C "$STAGE/pkg" owndraw-server web
( cd "$OUT" && sha256sum "$NAME" > "$NAME.sha256" )

if [ -n "$KEY" ]; then
  command -v minisign >/dev/null || { echo "minisign nicht installiert" >&2; exit 1; }
  echo "== Signieren (minisign)"
  minisign -S -s "$KEY" -m "$OUT/$NAME" -x "$OUT/$NAME.minisig" -t "owndraw-server $VERSION"
else
  echo "Hinweis: ohne -k nicht signiert. Der Server-Update verlangt eine gültige .minisig."
fi

echo "Fertig:"
ls -l "$OUT/$NAME"*
