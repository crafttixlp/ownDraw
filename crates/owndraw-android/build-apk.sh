#!/usr/bin/env bash
# Baut das OwnDraw-APK ohne Gradle: cargo-ndk (.so) → javac + d8 (classes.dex) → aapt2 (Manifest, Icon)
# → zip → zipalign → apksigner. Anleitung: docs/android.md.
#
# Benötigt: Rust mit Zielen aarch64-linux-android (+ x86_64-linux-android), cargo-ndk, JDK ≥ 17,
# Android SDK (platforms;android-34, build-tools) und NDK.
#
# Umgebung:
#   ANDROID_HOME / ANDROID_SDK_ROOT   SDK-Ordner (Pflicht)
#   ANDROID_NDK_HOME                  NDK-Ordner (sonst neuester unter $ANDROID_HOME/ndk/)
#   OWNDRAW_ABIS                      ABIs, Standard "arm64-v8a" (z. B. "arm64-v8a x86_64" für den Emulator)
#   OWNDRAW_PROFILE                   release (Standard) oder debug
#   OWNDRAW_OUT                       Ausgabe-APK, Standard target/apk/OwnDraw-<version>.apk (Name wie im Release erwartet)
#   BUILD_TOOLS_VERSION               build-tools-Version, Standard: neueste installierte
#   Signatur (fehlt OWNDRAW_KEYSTORE, wird ~/.android/debug.keystore benutzt bzw. angelegt):
#   OWNDRAW_KEYSTORE                  Pfad zum Keystore (.jks/.p12)
#   OWNDRAW_KEYSTORE_PASS             Passwort des Keystores
#   OWNDRAW_KEY_ALIAS                 Schlüssel-Alias
#   OWNDRAW_KEY_PASS                  Passwort des Schlüssels (Standard: wie Keystore)
set -euo pipefail

HERE="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
ROOT="$(cd "$HERE/../.." && pwd)"
SDK="${ANDROID_HOME:-${ANDROID_SDK_ROOT:-}}"
[ -n "$SDK" ] && [ -d "$SDK" ] || { echo "ANDROID_HOME/ANDROID_SDK_ROOT nicht gesetzt" >&2; exit 1; }
if [ -z "${ANDROID_NDK_HOME:-}" ]; then
    ANDROID_NDK_HOME="$(ls -d "$SDK"/ndk/* 2>/dev/null | sort -V | tail -n1 || true)"
fi
[ -n "$ANDROID_NDK_HOME" ] && [ -d "$ANDROID_NDK_HOME" ] || { echo "NDK nicht gefunden (ANDROID_NDK_HOME)" >&2; exit 1; }
export ANDROID_NDK_HOME

PLATFORM=34
MIN_SDK=26
ANDROID_JAR="$SDK/platforms/android-$PLATFORM/android.jar"
[ -f "$ANDROID_JAR" ] || { echo "Fehlt: $ANDROID_JAR (sdkmanager \"platforms;android-$PLATFORM\")" >&2; exit 1; }
BT_VER="${BUILD_TOOLS_VERSION:-$(ls "$SDK/build-tools" | sort -V | tail -n1)}"
BT="$SDK/build-tools/$BT_VER"
for t in aapt2 d8 zipalign apksigner; do
    [ -x "$BT/$t" ] || { echo "Fehlt: $BT/$t" >&2; exit 1; }
done
command -v cargo-ndk >/dev/null || { echo "cargo-ndk fehlt (cargo install cargo-ndk)" >&2; exit 1; }

# Version aus dem Workspace: 3.1.2 → versionCode 30102
VERSION="$(sed -n '/^\[workspace.package\]/,/^\[/s/^version *= *"\(.*\)"/\1/p' "$ROOT/Cargo.toml" | head -n1)"
IFS=. read -r V_MAJ V_MIN V_PAT <<<"${VERSION%%-*}"
VERSION_CODE=$(( V_MAJ * 10000 + V_MIN * 100 + ${V_PAT:-0} ))

PROFILE="${OWNDRAW_PROFILE:-release}"
ABIS="${OWNDRAW_ABIS:-arm64-v8a}"
WORK="$ROOT/target/apk-build"
OUT="${OWNDRAW_OUT:-$ROOT/target/apk/OwnDraw-$VERSION.apk}"
rm -rf "$WORK"
mkdir -p "$WORK/stage/lib" "$WORK/classes" "$WORK/res/mipmap-xxxhdpi" "$(dirname "$OUT")"

echo "== 1/6 Rust-Bibliothek ($ABIS, $PROFILE)"
NDK_ARGS=()
for abi in $ABIS; do NDK_ARGS+=(-t "$abi"); done
CARGO_FLAGS=()
[ "$PROFILE" = "release" ] && CARGO_FLAGS+=(--release)
(cd "$ROOT" && cargo ndk "${NDK_ARGS[@]}" -P "$MIN_SDK" -o "$WORK/stage/lib" build -p owndraw-android "${CARGO_FLAGS[@]}")
for abi in $ABIS; do
    [ -f "$WORK/stage/lib/$abi/libowndraw.so" ] || { echo "libowndraw.so für $abi fehlt" >&2; exit 1; }
done

echo "== 2/6 Java-Hilfsklassen (javac + d8)"
javac -Xlint:-options -source 8 -target 8 -encoding UTF-8 \
    -bootclasspath "$ANDROID_JAR" -classpath "$ANDROID_JAR" \
    -d "$WORK/classes" $(find "$HERE/java" -name '*.java')
"$BT/d8" --release --min-api "$MIN_SDK" --lib "$ANDROID_JAR" --output "$WORK/stage" \
    $(find "$WORK/classes" -name '*.class')

echo "== 3/6 Ressourcen und Manifest (aapt2)"
cp "$ROOT/assets/owndraw.png" "$WORK/res/mipmap-xxxhdpi/ic_launcher.png"
"$BT/aapt2" compile --dir "$WORK/res" -o "$WORK/res.zip"
"$BT/aapt2" link -o "$WORK/base.apk" \
    -I "$ANDROID_JAR" \
    --manifest "$HERE/AndroidManifest.xml" \
    --min-sdk-version "$MIN_SDK" --target-sdk-version "$PLATFORM" \
    --version-code "$VERSION_CODE" --version-name "$VERSION" --replace-version \
    "$WORK/res.zip"

echo "== 4/6 classes.dex und lib/ hinzufügen"
(cd "$WORK/stage" && zip -q -r "$WORK/base.apk" classes.dex lib)

echo "== 5/6 zipalign"
"$BT/zipalign" -f -p 4 "$WORK/base.apk" "$WORK/aligned.apk"

echo "== 6/6 Signieren"
if [ -n "${OWNDRAW_KEYSTORE:-}" ]; then
    KS="$OWNDRAW_KEYSTORE"
    : "${OWNDRAW_KEYSTORE_PASS:?OWNDRAW_KEYSTORE_PASS fehlt}"
    : "${OWNDRAW_KEY_ALIAS:?OWNDRAW_KEY_ALIAS fehlt}"
    export OWNDRAW_KEY_PASS="${OWNDRAW_KEY_PASS:-$OWNDRAW_KEYSTORE_PASS}"
    ALIAS="$OWNDRAW_KEY_ALIAS"
else
    KS="$HOME/.android/debug.keystore"
    if [ ! -f "$KS" ]; then
        echo "Lege Debug-Keystore an: $KS"
        mkdir -p "$(dirname "$KS")"
        keytool -genkeypair -keystore "$KS" -storepass android -keypass android \
            -alias androiddebugkey -keyalg RSA -keysize 2048 -validity 10000 \
            -dname "CN=Android Debug,O=Android,C=US" >/dev/null
    fi
    export OWNDRAW_KEYSTORE_PASS=android OWNDRAW_KEY_PASS=android
    ALIAS=androiddebugkey
    echo "Hinweis: Debug-Signatur (nicht für Releases)."
fi
"$BT/apksigner" sign --ks "$KS" --ks-key-alias "$ALIAS" \
    --ks-pass env:OWNDRAW_KEYSTORE_PASS --key-pass env:OWNDRAW_KEY_PASS \
    --out "$OUT" "$WORK/aligned.apk"
"$BT/apksigner" verify "$OUT"
echo "Fertig: $OUT (Version $VERSION, versionCode $VERSION_CODE)"
