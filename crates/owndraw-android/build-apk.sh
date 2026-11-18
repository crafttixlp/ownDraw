#!/usr/bin/env bash
# Baut das OwnDraw-APK ohne Gradle: cargo-ndk (.so) → Maven-Abhängigkeiten (ML Kit, fest gepinnt)
# → aapt2 (Manifest, Icon, Bibliotheks-Ressourcen, R-Klassen) → javac + d8 (classes*.dex) → zip → zipalign
# → apksigner. Anleitung: docs/android.md.
#
# Benötigt: Rust mit Zielen aarch64-linux-android (+ x86_64-linux-android), cargo-ndk, JDK ≥ 17,
# Android SDK (platforms;android-34, build-tools) und NDK.
#
# Umgebung:
#   ANDROID_HOME / ANDROID_SDK_ROOT   SDK-Ordner (Pflicht)
#   ANDROID_NDK_HOME                  NDK-Ordner (sonst neuester unter $ANDROID_HOME/ndk/)
#   OWNDRAW_ABIS                      ABIs, Standard "arm64-v8a" (z. B. "arm64-v8a x86_64" für den Emulator)
#   OWNDRAW_PROFILE                   release (Standard) oder debug
#   OWNDRAW_OUT                       Ausgabe-APK, Standard target/apk/owndraw-<version>.apk
#   BUILD_TOOLS_VERSION               build-tools-Version, Standard: neueste installierte
#   OWNDRAW_DEPS_CACHE                Ablage der heruntergeladenen AAR/JAR, Standard target/android-deps
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
for t in curl sha256sum unzip zip; do
    command -v "$t" >/dev/null || { echo "$t fehlt" >&2; exit 1; }
done

# Handschrift-Erkennung: Google ML Kit Digital Ink Recognition und alle transitiven Abhängigkeiten
# (compile+runtime, je Artefakt die höchste geforderte Version wie bei Gradle). Feste Versionen + sha256;
# eine neue Version heißt: Liste neu auflösen und Prüfsummen eintragen (docs/android.md).
# Format: <Repo> <Gruppe>:<Artefakt>:<Version>@<aar|jar> <sha256>
MAVEN_google=https://dl.google.com/android/maven2
MAVEN_central=https://repo1.maven.org/maven2
DEPS=(
    "google androidx.activity:activity:1.6.0@aar 7f08723ecabefba616d60c714b0e9a31301bd4d0792fcc7946c1479c57fd2d28"
    "google androidx.annotation:annotation:1.3.0@jar 97dc45afefe3a1e421da42b8b6e9f90491477c45fc6178203e3a5e8a05ee8553"
    "google androidx.annotation:annotation-experimental:1.3.0@aar abfd29c8556e5bd0325a9f769ab9e9d154ff4a5515c476cdd5a2a8285b1b19dc"
    "google androidx.appcompat:appcompat:1.6.1@aar 7ea5573b93ababd3bd32312451c6ea48a662b03a140dda81aebe75776a20a422"
    "google androidx.appcompat:appcompat-resources:1.6.1@aar db915dbf49357863de1669ff9fdd8e9008d65fe357af6cce9ae63043ad5f6617"
    "google androidx.arch.core:core-common:2.1.0@jar fe1237bf029d063e7f29fe39aeaf73ef74c8b0a3658486fc29d3c54326653889"
    "google androidx.arch.core:core-runtime:2.1.0@aar dd77615bd3dd275afb11b62df25bae46b10b4a117cd37943af45bdcbf8755852"
    "google androidx.collection:collection:1.1.0@jar 632a0e5407461de774409352940e292a291037724207a787820c77daf7d33b72"
    "google androidx.concurrent:concurrent-futures:1.0.0@jar 5595a40e278a7b39fa78a09490e3d7f3faa95c7b01447148bd38b5ade0605c35"
    "google androidx.core:core:1.9.0@aar 8bda3ee3a88887d54f6679fb6b6cd788629f73234ac91c8bbed924e721ec85b8"
    "google androidx.core:core-ktx:1.8.0@aar 1790bb9b0b3efe6a27fb1ba3a3530b0b9aa6654e58bf8a9ef783f767e50d31b4"
    "google androidx.cursoradapter:cursoradapter:1.0.0@aar a81c8fe78815fa47df5b749deb52727ad11f9397da58b16017f4eb2c11e28564"
    "google androidx.customview:customview:1.0.0@aar 20e5b8f6526a34595a604f56718da81167c0b40a7a94a57daa355663f2594df2"
    "google androidx.drawerlayout:drawerlayout:1.0.0@aar 9402442cdc5a43cf62fb14f8cf98c63342d4d9d9b805c8033c6cf7e802749ac1"
    "google androidx.emoji2:emoji2:1.2.0@aar f31a06c150ecb03073f55a6f7b0b74a240a6a8d727c14ce76726d020570dfa8c"
    "google androidx.emoji2:emoji2-views-helper:1.2.0@aar 7ffa4d464d9db259fca0cdb50fbd4ab63d6872bcda59468b9f7555504c7d5ac4"
    "google androidx.fragment:fragment:1.3.6@aar 12f0831b4f08092d5dda272c1923c11a022ff20ceffed3e801751e21bb8d1c1e"
    "google androidx.interpolator:interpolator:1.0.0@aar 33193135a64fe21fa2c35eec6688f1a76e512606c0fc83dc1b689e37add7732a"
    "google androidx.lifecycle:lifecycle-common:2.5.1@jar 20ad1520f625cf455e6afd7290988306d3a9886efa993e0860fbabf4bb3f7bda"
    "google androidx.lifecycle:lifecycle-livedata:2.1.0@aar 242e446bed3db36f0df0aab0cb7f91060bd2dab7adcad1117adf54e724cd1d26"
    "google androidx.lifecycle:lifecycle-livedata-core:2.5.1@aar ee792103ca248bfaf150c45a93871e4cf7e8cebab990e0f62f7de5d4ff2f209f"
    "google androidx.lifecycle:lifecycle-process:2.4.1@aar db649b3efa24e31052145310b002db91da346b3f89c093ec38c3046db45e794e"
    "google androidx.lifecycle:lifecycle-runtime:2.5.1@aar 33b0d73dc2f028fceb3599bacabe563c3db6d26f3513d889595863536a4ac8c0"
    "google androidx.lifecycle:lifecycle-service:2.1.0@aar 23516745f34f16ff7850bb1eadd55cf193dd789cba428de4bca120433e3bfd69"
    "google androidx.lifecycle:lifecycle-viewmodel:2.5.1@aar 14a27d5fb8a1436b1b7dec18bebaa66c830cdbbc8e28a81ce5f85b7c33b3ae9d"
    "google androidx.lifecycle:lifecycle-viewmodel-savedstate:2.5.1@aar 8481141f97f0e6213dd33fcc89a784c4bd11a6ff7d4779a1cf6a0e9fbdbf24e0"
    "google androidx.loader:loader:1.0.0@aar 11f735cb3b55c458d470bed9e25254375b518b4b1bad6926783a7026db0f5025"
    "google androidx.resourceinspection:resourceinspection-annotation:1.0.1@jar 8cff870ec6fb31db48a52f4a792335b4bf8de07e03bd37823181526433ccd5cb"
    "google androidx.room:room-common:2.2.5@jar 2b130dd4a1d3d91b6701ed33096d389f01c4fc1197a7acd6b91724ddc5acfc06"
    "google androidx.room:room-runtime:2.2.5@aar 24a5549b796e43e337513d2908adac67f45350d9a90bca7e2e6120692140bb14"
    "google androidx.savedstate:savedstate:1.2.0@aar 2de528d6898e95ef020d22d9ffdf9d1f77cbdd93f92d39dfaa5d5c43b0c311c8"
    "google androidx.sqlite:sqlite:2.1.0@aar 8341ff092d6060d62a07227f29237155fff36fb16f96c95fbd9a884e375db912"
    "google androidx.sqlite:sqlite-framework:2.1.0@aar 8673737fdb2efbad91aeaeed1927ebb29212d36a867d93b9639c8069019f8a1e"
    "google androidx.startup:startup-runtime:1.1.1@aar e0a6329a371262fe4c450372b70fdaf33b769ef6917094723787cfce896b1dd3"
    "google androidx.tracing:tracing:1.0.0@aar 07b8b6139665b884a162eccf97891ca50f7f56831233bf25168ae04f7b568612"
    "google androidx.vectordrawable:vectordrawable:1.1.0@aar 46fd633ac01b49b7fcabc263bf098c5a8b9e9a69774d234edcca04fb02df8e26"
    "google androidx.vectordrawable:vectordrawable-animated:1.1.0@aar 76da2c502371d9c38054df5e2b248d00da87809ed058f3363eae87ce5e2403f8"
    "google androidx.versionedparcelable:versionedparcelable:1.1.1@aar 57e8d93260d18d5b9007c9eed3c64ad159de90c8609ebfc74a347cbd514535a4"
    "google androidx.viewpager:viewpager:1.0.0@aar 147af4e14a1984010d8f155e5e19d781f03c1d70dfed02a8e0d18428b8fc8682"
    "google androidx.work:work-runtime:2.7.0@aar 88823e00ba9aae6dc6436aed5928c070d68a62c2581de96e655ca86f030e72a9"
    "google com.google.android.datatransport:transport-api:2.2.1@aar e62f73ba1c7f5d15f5d86d8e9a298c42ccd0648e7ce60b9e312f2ecd123b5ed0"
    "google com.google.android.datatransport:transport-backend-cct:2.3.3@aar b22a14d560a590dc875e902424b29a13a02d1faf297b7c39a9007967ffadccbd"
    "google com.google.android.datatransport:transport-runtime:2.2.6@aar d83ad23c134429691161c92476a3c83df236cd1f8badbfd4f00723b4eccd741c"
    "google com.google.android.gms:play-services-base:18.5.0@aar 59a5c0c2da12311d75d965ce1f419498536b1a167fb28ff7dfc2dfd9cefa4157"
    "google com.google.android.gms:play-services-basement:18.4.0@aar ce5c936fd66814b3602f5c6a5e929911ff973d4b05f6de9996da596bef97cad2"
    "google com.google.android.gms:play-services-tasks:18.2.0@aar 7f2aaa8f502068eaf54356ca92aec04271d6e7c416c52c45c0d23440fcbd1654"
    "google com.google.firebase:firebase-annotations:16.0.0@jar 8830f2e8a5e434cbc5e4bd18133073b6e8518a8ce8ec2ce6f3b273c91d9664c2"
    "google com.google.firebase:firebase-components:16.1.0@aar 82e91aa5ed186eedaf1f2210f5d241ba6e4456cf0c32949d5cdbe152b0e2e88b"
    "google com.google.firebase:firebase-encoders:16.1.0@jar 8f89a74298bbccc2c543385208a2f0c67f603ce67e6ec21cc772e98e688f7cf5"
    "google com.google.firebase:firebase-encoders-json:17.1.0@aar 8295c755bcadc87a2aaee5b9e568f27fd138fd45ff1159a8ee747cf4c3a38d85"
    "central com.google.guava:listenablefuture:1.0@jar e4ad7607e5c0477c6f890ef26a49cb8d1bb4dffb650bab4502afee64644e3069"
    "google com.google.mlkit:common:18.11.0@aar 66408a08dbb2465687ff5dd0c8f6410b56549ce3ecf449d9096f3712282131d0"
    "google com.google.mlkit:digital-ink-common:16.0.0@aar 5f844db91ab9aaed88c5ffe66069491126e3300792dd2c435b2a508585a53e3f"
    "google com.google.mlkit:digital-ink-recognition:19.0.0@aar a6d98fca778439b82e2ae4651cbf9bc08e6c7b512fc1d7e7a4b29f9aa60acdd6"
    "central com.squareup.okhttp3:okhttp:3.12.1@jar 07c3d82ca7eaf4722f00b2da807dc7860f6169ae60cfedcf5d40218f90880a46"
    "central com.squareup.okio:okio:2.0.0@jar 79b948cf77504750fdf7aeaf362b5060415136ab6635e5113bd22925e0e9e737"
    "central javax.inject:javax.inject:1@jar 91c77044a50c481636c32d916fd89c9118a72195390452c81065080f957de7ff"
    "central org.jetbrains:annotations:13.0@jar ace2a10dc8e2d5fd34925ecac03e4988b2c0f851650c94b8cef49ba1bd111478"
    "central org.jetbrains.kotlin:kotlin-stdlib:1.7.20@jar 7779ec96b9acbf92ca023858ac04543f9d2c3bdf1722425fff42f25ff3acfc9b"
    "central org.jetbrains.kotlin:kotlin-stdlib-common:1.7.20@jar e0e91962bc0007338bf5b1739f62927ac32d14ba3d827fa608ab4e5351729d5d"
    "central org.jetbrains.kotlin:kotlin-stdlib-jdk7:1.7.20@jar 524da3c1a2ad56fd52c4ae2272ef3de421de8d2047ab1c51fc306d351243f2f5"
    "central org.jetbrains.kotlin:kotlin-stdlib-jdk8:1.6.0@jar ab0547c495953214a5f2b28150014f4e02133678d52b77d76375ea235e443dbd"
    "central org.jetbrains.kotlinx:kotlinx-coroutines-android:1.6.1@jar 961ebde8138779a299430ca325a86e28c490527a87ba518362fa45c44c7e7e95"
    "central org.jetbrains.kotlinx:kotlinx-coroutines-core-jvm:1.6.1@jar 3a93ffd052844643c0fef950ae5578db47cccbe9e7176d681333182e232bb0f1"
)
DEPS_CACHE="${OWNDRAW_DEPS_CACHE:-$ROOT/target/android-deps}"

# Version aus dem Workspace: 3.1.2 → versionCode 30102
VERSION="$(sed -n '/^\[workspace.package\]/,/^\[/s/^version *= *"\(.*\)"/\1/p' "$ROOT/Cargo.toml" | head -n1)"
IFS=. read -r V_MAJ V_MIN V_PAT <<<"${VERSION%%-*}"
VERSION_CODE=$(( V_MAJ * 10000 + V_MIN * 100 + ${V_PAT:-0} ))

PROFILE="${OWNDRAW_PROFILE:-release}"
ABIS="${OWNDRAW_ABIS:-arm64-v8a}"
WORK="$ROOT/target/apk-build"
OUT="${OWNDRAW_OUT:-$ROOT/target/apk/owndraw-$VERSION.apk}"
rm -rf "$WORK"
mkdir -p "$WORK/stage/lib" "$WORK/classes" "$WORK/res/mipmap-xxxhdpi" "$(dirname "$OUT")"

echo "== 1/7 Rust-Bibliothek ($ABIS, $PROFILE)"
NDK_ARGS=()
for abi in $ABIS; do NDK_ARGS+=(-t "$abi"); done
CARGO_FLAGS=()
[ "$PROFILE" = "release" ] && CARGO_FLAGS+=(--release)
(cd "$ROOT" && cargo ndk "${NDK_ARGS[@]}" -P "$MIN_SDK" -o "$WORK/stage/lib" build -p owndraw-android "${CARGO_FLAGS[@]}")
for abi in $ABIS; do
    [ -f "$WORK/stage/lib/$abi/libowndraw.so" ] || { echo "libowndraw.so für $abi fehlt" >&2; exit 1; }
done

echo "== 2/7 Abhängigkeiten (${#DEPS[@]} Artefakte, Cache $DEPS_CACHE)"
mkdir -p "$DEPS_CACHE" "$WORK/aar" "$WORK/assets" "$WORK/gen"
DEP_JARS=()
RES_ARGS=()
PKGS=()
for d in "${DEPS[@]}"; do
    read -r repo coord sum <<<"$d"
    IFS=: read -r g a v <<<"${coord%@*}"
    ext="${coord##*@}"
    file="$DEPS_CACHE/$g-$a-$v.$ext"
    if ! { [ -f "$file" ] && echo "$sum  $file" | sha256sum -c --status; }; then
        base="MAVEN_$repo"
        url="${!base}/${g//.//}/$a/$v/$a-$v.$ext"
        curl -fsSL --retry 5 --retry-delay 5 -o "$file.part" "$url" || { echo "Download fehlgeschlagen: $url" >&2; exit 1; }
        echo "$sum  $file.part" | sha256sum -c --status || { echo "Prüfsumme falsch: $url" >&2; rm -f "$file.part"; exit 1; }
        mv "$file.part" "$file"
    fi
    if [ "$ext" = jar ]; then
        DEP_JARS+=("$file")
        continue
    fi
    x="$WORK/aar/$g-$a"
    unzip -q -o "$file" -d "$x"
    [ -f "$x/classes.jar" ] && DEP_JARS+=("$x/classes.jar")
    for j in "$x"/libs/*.jar; do [ -f "$j" ] && DEP_JARS+=("$j"); done
    pkg="$(sed -n 's/.*package="\([^"]*\)".*/\1/p' "$x/AndroidManifest.xml" | head -n1)"
    [ -n "$pkg" ] && PKGS+=("$pkg")
    if [ -d "$x/res" ] && [ -n "$(ls -A "$x/res")" ]; then
        "$BT/aapt2" compile --dir "$x/res" -o "$WORK/aar/$g-$a.res.zip"
        RES_ARGS+=(-R "$WORK/aar/$g-$a.res.zip")
    fi
    [ -d "$x/assets" ] && cp -r "$x/assets/." "$WORK/assets/"
    for abi in $ABIS; do
        if [ -d "$x/jni/$abi" ]; then cp "$x/jni/$abi/"*.so "$WORK/stage/lib/$abi/"; fi
    done
done
EXTRA_PKGS="$(printf '%s\n' "${PKGS[@]}" | sort -u | grep -vx 'de.owndraw' | paste -sd: -)"

echo "== 3/7 Ressourcen und Manifest (aapt2)"
cp "$ROOT/assets/owndraw.png" "$WORK/res/mipmap-xxxhdpi/ic_launcher.png"
"$BT/aapt2" compile --dir "$WORK/res" -o "$WORK/res.zip"
# Bibliotheks-Ressourcen als Overlays; R-Klassen für alle Bibliothekspakete (deren Code liest R-Felder zur Laufzeit)
"$BT/aapt2" link -o "$WORK/base.apk" \
    -I "$ANDROID_JAR" \
    --manifest "$HERE/AndroidManifest.xml" \
    --min-sdk-version "$MIN_SDK" --target-sdk-version "$PLATFORM" \
    --version-code "$VERSION_CODE" --version-name "$VERSION" --replace-version \
    --auto-add-overlay "${RES_ARGS[@]}" \
    --extra-packages "$EXTRA_PKGS" --java "$WORK/gen" \
    -A "$WORK/assets" \
    "$WORK/res.zip"

echo "== 4/7 Java (javac + d8)"
CP="$ANDROID_JAR:$(IFS=:; echo "${DEP_JARS[*]}")"
javac -Xlint:-options -source 8 -target 8 -encoding UTF-8 \
    -bootclasspath "$ANDROID_JAR" -classpath "$CP" \
    -d "$WORK/classes" $(find "$HERE/java" "$WORK/gen" -name '*.java')
# Ab minSdk 21 verteilt d8 auf mehrere classes*.dex (natives Multidex)
"$BT/d8" --release --min-api "$MIN_SDK" --lib "$ANDROID_JAR" --output "$WORK/stage" \
    $(find "$WORK/classes" -name '*.class') "${DEP_JARS[@]}"

echo "== 5/7 classes*.dex und lib/ hinzufügen"
(cd "$WORK/stage" && zip -q -r "$WORK/base.apk" classes*.dex lib)

echo "== 6/7 zipalign"
"$BT/zipalign" -f -p 4 "$WORK/base.apk" "$WORK/aligned.apk"

echo "== 7/7 Signieren"
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
