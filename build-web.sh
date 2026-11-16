#!/bin/bash
# Baut OwnDraw für den Browser (WebAssembly, Gastmodus) nach dist/web/ (index.html, owndraw_web.js,
# owndraw_web_bg.wasm, owndraw.png). Kein trunk.
# Voraussetzungen: rustup target add wasm32-unknown-unknown
#                  cargo install wasm-bindgen-cli --version <Version von wasm-bindgen aus Cargo.lock>
#                  optional: wasm-opt (binaryen) zum Verkleinern
# Test: cd dist/web && python3 -m http.server 8080  →  http://localhost:8080/
set -euo pipefail
cd "$(dirname "$0")"
OUT=dist/web
TARGET_DIR=${CARGO_TARGET_DIR:-target}

# wasm-bindgen-CLI muss exakt zur Bibliothek im Cargo.lock passen
WB_VER=$(awk '/^name = "wasm-bindgen"$/{getline; gsub(/version = |"/,""); print; exit}' Cargo.lock)
if ! command -v wasm-bindgen >/dev/null || [ "$(wasm-bindgen --version | awk '{print $2}')" != "$WB_VER" ]; then
  echo "wasm-bindgen $WB_VER fehlt oder passt nicht: cargo install wasm-bindgen-cli --version $WB_VER --locked" >&2
  exit 1
fi

cargo build --profile web -p owndraw-web --target wasm32-unknown-unknown
rm -rf "$OUT"
mkdir -p "$OUT"
wasm-bindgen --target web --no-typescript --out-dir "$OUT" "$TARGET_DIR/wasm32-unknown-unknown/web/owndraw_web.wasm"
if command -v wasm-opt >/dev/null; then
  wasm-opt -O3 --enable-bulk-memory --enable-nontrapping-float-to-int --enable-sign-ext --enable-mutable-globals \
    "$OUT/owndraw_web_bg.wasm" -o "$OUT/owndraw_web_bg.opt.wasm"
  mv "$OUT/owndraw_web_bg.opt.wasm" "$OUT/owndraw_web_bg.wasm"
else
  echo "Hinweis: wasm-opt nicht gefunden, .wasm bleibt unoptimiert." >&2
fi
# Version (aus Cargo.toml) in index.html eintragen und als version.txt ablegen (Server: /api/health, Installer)
VERSION=$(awk -F'"' '/^\[workspace.package\]/{w=1} w && /^version *=/{print $2; exit}' Cargo.toml)
[ -n "$VERSION" ] || { echo "Version in Cargo.toml nicht gefunden" >&2; exit 1; }
sed "s/__OWNDRAW_VERSION__/$VERSION/g" crates/owndraw-web/index.html > "$OUT/index.html"
echo "$VERSION" > "$OUT/version.txt"
cp assets/owndraw.png "$OUT/"
ls -la "$OUT"
