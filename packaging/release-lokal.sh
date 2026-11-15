#!/usr/bin/env bash
# Lokaler Ersatz für .github/workflows/release.yml (z. B. wenn GitHub Actions gesperrt ist).
# Sammelt AppImage, APK (optional) und Quell-Tarball, signiert alles mit minisign und legt – nur mit
# --veroeffentlichen – das GitHub-Release v<VERSION> an. Ohne den Schalter: nur Probelauf (dist/ bleibt liegen).
#
#   MINISIGN_KEY_FILE=/pfad/minisign.key [GITHUB_TOKEN=…] packaging/release-lokal.sh [--apk DATEI] [--veroeffentlichen]
#
# Braucht: minisign, git, python3, curl (nur beim Veröffentlichen). Der geheime Schlüssel bleibt außerhalb des Repos.
set -euo pipefail
cd "$(dirname "$0")/.."

REPO="${OWNDRAW_REPO:-crafttixlp/ownDraw}"
APK=""
PUBLISH=0
while [ $# -gt 0 ]; do
  case "$1" in
    --apk) APK="$2"; shift 2 ;;
    --veroeffentlichen) PUBLISH=1; shift ;;
    *) echo "Unbekannte Option: $1" >&2; exit 2 ;;
  esac
done

VERSION=$(python3 -c 'import tomllib; print(tomllib.load(open("Cargo.toml","rb"))["workspace"]["package"]["version"])')
TAG="v$VERSION"
PUB=crates/owndraw/src/update/minisign.pub
: "${MINISIGN_KEY_FILE:?MINISIGN_KEY_FILE (geheimer minisign-Schlüssel) fehlt}"
command -v minisign >/dev/null || { echo "minisign fehlt (z. B. apt install minisign)" >&2; exit 1; }
[ -z "$(git status --porcelain)" ] || { echo "Arbeitsbaum nicht sauber – erst committen." >&2; exit 1; }

DIST=dist/$TAG
rm -rf "$DIST"; mkdir -p "$DIST"
SHA=$(git rev-parse HEAD)
git archive --format=tar.gz --prefix="OwnDraw-$VERSION/" -o "$DIST/OwnDraw-$VERSION-src.tar.gz" "$SHA"

APPIMAGE="OwnDraw-$VERSION-x86_64.AppImage"
[ -f "$APPIMAGE" ] || ./build-appimage.sh
cp "$APPIMAGE" "$DIST/"
if [ -n "$APK" ]; then
  cp "$APK" "$DIST/OwnDraw-$VERSION.apk"
else
  echo "Hinweis: ohne --apk enthält das Release keine APK (Android findet dann kein Update)." >&2
fi

for f in "$DIST"/*; do
  case "$f" in *.minisig) continue ;; esac
  n=$(basename "$f")
  minisign -S -s "$MINISIGN_KEY_FILE" -m "$f" -t "OwnDraw $VERSION $n"
  minisign -V -p "$PUB" -m "$f" >/dev/null
done
ls -la "$DIST"

if [ "$PUBLISH" != 1 ]; then
  echo "Probelauf fertig: $DIST. Veröffentlichen mit --veroeffentlichen (braucht GITHUB_TOKEN)."
  exit 0
fi
: "${GITHUB_TOKEN:?GITHUB_TOKEN (Recht contents:write) fehlt}"

NOTES=""
[ -f CHANGELOG.md ] && NOTES=$(awk -v v="$VERSION" '/^## /{if(f)exit; h=$0; sub(/^## +\[?v?/,"",h); sub(/[] (].*$/,"",h); if(h==v)f=1; next} f{print}' CHANGELOG.md)
BODY=$(TAG="$TAG" SHA="$SHA" NOTES="$NOTES" python3 -c 'import json,os; print(json.dumps({"tag_name":os.environ["TAG"],"target_commitish":os.environ["SHA"],"name":"OwnDraw "+os.environ["TAG"][1:],"body":os.environ["NOTES"],"generate_release_notes":not os.environ["NOTES"]}))')
AUTH=(-H "Authorization: Bearer $GITHUB_TOKEN" -H "Accept: application/vnd.github+json")
REL=$(curl -fsS "${AUTH[@]}" -X POST "https://api.github.com/repos/$REPO/releases" -d "$BODY")
ID=$(printf '%s' "$REL" | python3 -c 'import json,sys; print(json.load(sys.stdin)["id"])')
for f in "$DIST"/*; do
  n=$(basename "$f")
  echo "Lade hoch: $n"
  curl -fsS "${AUTH[@]}" -H "Content-Type: application/octet-stream" --data-binary @"$f" \
    "https://uploads.github.com/repos/$REPO/releases/$ID/assets?name=$n" >/dev/null
done
echo "Release $TAG veröffentlicht: https://github.com/$REPO/releases/tag/$TAG"
