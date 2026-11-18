# OwnDraw für Android

OwnDraw läuft auf Android als dieselbe egui-App wie auf dem Desktop (eframe mit `android-activity`,
NativeActivity). Gebaut wird ohne Gradle mit dem Skript `crates/owndraw-android/build-apk.sh`.

## Aufbau

| Teil | Datei |
|---|---|
| Einstiegspunkt `android_main`, JNI-Rückrufe (`Java_de_owndraw_OwnDrawActivity_*`) | `crates/owndraw-android/src/lib.rs` (cdylib → `libowndraw.so`) |
| App-Start `owndraw::run_android` | `crates/owndraw/src/lib.rs` |
| Plattform-Schicht `AndroidPlatform` (SAF, ContentResolver, Zwischenablage, Drag & Drop) | `crates/owndraw/src/platform/android.rs` |
| Java-Hülle `de.owndraw.OwnDrawActivity` (erbt `NativeActivity`): `onActivityResult`, `onPause`, Intents, Drag-Listener, Zwischenablage, `openUrl` | `crates/owndraw-android/java/de/owndraw/OwnDrawActivity.java` |
| `de.owndraw.FileShareProvider`: minimaler FileProvider (ohne androidx) für Bilder in der Zwischenablage und „Öffnen in anderer App“ | `crates/owndraw-android/java/de/owndraw/FileShareProvider.java` |
| `de.owndraw.InkRecognizer`: Handschrift-Erkennung mit Google ML Kit (Werkzeug „Umwandeln“), Aufruf über `OwnDrawActivity.hw` | `crates/owndraw-android/java/de/owndraw/InkRecognizer.java`, Rust-Seite `crates/owndraw/src/hw.rs` |
| Manifest (minSdk 26, targetSdk 34, Intent-Filter, von Hand übernommene Einträge der ML-Kit-Bibliotheken) | `crates/owndraw-android/AndroidManifest.xml` |

Dateizugriff läuft ausschließlich über das Storage Access Framework (`ACTION_OPEN_DOCUMENT`,
`ACTION_CREATE_DOCUMENT`, `ACTION_OPEN_DOCUMENT_TREE` mit `takePersistableUriPermission`), daher braucht
die App keine Speicher-Rechte. Ohne gewählten Ordner liegt der Standard-Ort im App-Speicher
(`<internal_data_path>/boards`). Angefragte Rechte: `INTERNET` und `REQUEST_INSTALL_PACKAGES`
(Update-Dienst; `INTERNET` auch für den Modell-Download von ML Kit) sowie die Rechte, die die ML-Kit-Bibliotheken
mitbringen (`ACCESS_NETWORK_STATE`, `WAKE_LOCK`, `RECEIVE_BOOT_COMPLETED`, `FOREGROUND_SERVICE` aus WorkManager).

## Handschrift-Erkennung (Google ML Kit)

Das Werkzeug „Umwandeln“ nutzt auf Android **Google ML Kit Digital Ink Recognition**
(`com.google.mlkit:digital-ink-recognition:19.0.0`): strichbasiert, Sprachmodell Deutsch (`de`). Das Modell
(ca. 20 MB) lädt ML Kit nur, wenn man in den Einstellungen bzw. im Umwandeln-Fenster auf „Laden“ tippt, von
Googles Servern; danach wird offline auf dem Gerät erkannt. Formeln kann ML Kit nicht: der Formel-Reiter macht aus
dem erkannten Text eine Rechenzeile (`owndraw_core::hw::formel_aus_text`), Zahl und Kontext-Bewertung kommen
aus dem Kern wie im Browser. Punkte werden ohne Zeitstempel übergeben (OwnDraw speichert keine).

**Lizenz:** ML Kit ist kein Open Source, sondern unterliegt den
[ML Kit Terms of Service](https://developers.google.com/ml-kit/terms) (dazu die Google APIs Terms of Service).
Die AARs bringen Lizenzhinweise Dritter mit (`third_party_licenses.txt` im AAR). Die übrigen Abhängigkeiten
(AndroidX, Kotlin, OkHttp, Okio, Firebase-Components/Encoders, Play-Services-Basis) stehen unter Apache-2.0 bzw.
den Android-SDK-Bedingungen von Google. ML Kit kann laut Google anonyme Nutzungs-/Leistungsdaten an Google senden
(datatransport/Firelog); die Handschrift selbst bleibt auf dem Gerät.

**Abhängigkeiten ohne Gradle:** `build-apk.sh` lädt ML Kit und alle transitiven Abhängigkeiten (64 AAR/JAR,
ca. 25 MB, Liste `DEPS` mit festen Versionen und sha256) von maven.google.com bzw. Maven Central in
`target/android-deps` (`OWNDRAW_DEPS_CACHE`, in CI gecacht) und baut daraus:

- `classes.jar` und `libs/*.jar` der AARs → javac-Classpath und d8 (mehrere `classes*.dex`, natives Multidex)
- `res/` der AARs → `aapt2 compile`, beim Linken als Overlays (`-R … --auto-add-overlay`); R-Klassen aller
  Bibliothekspakete per `--extra-packages` und `--java`, mitkompiliert
- `assets/` → `-A`, `jni/<abi>/*.so` (`libdigitalink.so`, ca. 7 MB für arm64) → `lib/<abi>/`
- Manifest-Einträge (Provider, Services, Receiver, Rechte) stehen von Hand in `AndroidManifest.xml`
  (kein Manifest-Merger); `${applicationId}` ist dort als `de.owndraw` ausgeschrieben.

Neue ML-Kit-Version: Abhängigkeiten neu auflösen (POMs: compile+runtime, je Artefakt die höchste Version),
`DEPS` samt sha256 ersetzen und die `AndroidManifest.xml` der AARs mit dem eigenen Manifest vergleichen.

`owndraw-android` gehört nicht zu den `default-members` des Workspaces: `cargo build`/`cargo test`
auf dem Desktop bauen es nicht mit (auf Nicht-Android-Zielen ist das Crate leer).

## Voraussetzungen

- Rust mit den Zielen: `rustup target add aarch64-linux-android x86_64-linux-android`
- `cargo install cargo-ndk`
- JDK 17 oder neuer (`javac`, `keytool`), `zip`, `curl`, `unzip`, `sha256sum`
  (mit JDK 21 build-tools 35 oder neuer nehmen: d8 aus build-tools 34 stürzt an javac-21-Bridge-Methoden ab)
- Android SDK mit `platforms;android-34`, `build-tools;34.0.0` (oder neuer) und NDK (r26 oder neuer):

```sh
sdkmanager "platforms;android-34" "build-tools;34.0.0" "ndk;26.3.11579264"
export ANDROID_HOME=$HOME/Android/Sdk
export ANDROID_NDK_HOME=$ANDROID_HOME/ndk/26.3.11579264   # optional, sonst neuester NDK
```

## Bauen

```sh
crates/owndraw-android/build-apk.sh                              # arm64-v8a, release, Debug-Signatur
OWNDRAW_ABIS="arm64-v8a x86_64" crates/owndraw-android/build-apk.sh   # zusätzlich für den Emulator
OWNDRAW_PROFILE=debug crates/owndraw-android/build-apk.sh        # schneller, mit Debug-Infos
```

Ergebnis: `target/apk/owndraw-<version>.apk`. Schritte des Skripts:

1. `cargo ndk -t <abi> -P 26 -o … build -p owndraw-android --release` → `lib/<abi>/libowndraw.so`
2. Abhängigkeiten (ML Kit) laden, sha256 prüfen, AARs entpacken (siehe oben)
3. `aapt2 compile` (Icon aus `assets/owndraw.png`, Bibliotheks-Ressourcen) und `aapt2 link` mit Manifest und
   R-Klassen; `versionName`/`versionCode` kommen aus der Workspace-Version (`3.1.2` → `30102`)
4. `javac -source 8 -target 8 -bootclasspath android.jar` für `java/**/*.java` und die R-Klassen, dann
   `d8 --min-api 26` mit allen Bibliotheks-JARs → `classes*.dex`
5. `classes*.dex` und `lib/` ins APK packen
6. `zipalign -p 4`
7. `apksigner sign` + `apksigner verify`

Nur prüfen, ob der Rust-Teil kompiliert (ohne NDK, ohne Linken):

```sh
cargo check --target aarch64-linux-android -p owndraw-android
cargo clippy --target aarch64-linux-android -p owndraw-android -p owndraw --lib --no-deps -- -D warnings
```

## Signatur

Ohne Angaben signiert das Skript mit `~/.android/debug.keystore` (legt ihn bei Bedarf an; Alias
`androiddebugkey`, Passwort `android`). Für Releases:

```sh
keytool -genkeypair -keystore owndraw-release.jks -alias owndraw -keyalg RSA -keysize 4096 -validity 10000
export OWNDRAW_KEYSTORE=$PWD/owndraw-release.jks
export OWNDRAW_KEYSTORE_PASS=…   # Keystore-Passwort
export OWNDRAW_KEY_ALIAS=owndraw
export OWNDRAW_KEY_PASS=…        # optional, Standard = Keystore-Passwort
crates/owndraw-android/build-apk.sh
```

Wichtig: Updates lassen sich nur installieren, wenn das neue APK mit **demselben** Schlüssel signiert ist.
Keystore und Passwörter sicher aufbewahren (Verlust = Nutzer müssen deinstallieren und neu installieren).

## CI (GitHub Actions, WP12)

Skizze für den Release-Workflow (`ubuntu-latest` hat SDK, `sdkmanager` und Java bereits):

```yaml
- uses: actions/setup-java@v4
  with: { distribution: temurin, java-version: 17 }
- uses: android-actions/setup-android@v3
- run: sdkmanager "platforms;android-34" "build-tools;34.0.0" "ndk;26.3.11579264"
- run: rustup target add aarch64-linux-android x86_64-linux-android && cargo install cargo-ndk
- name: APK bauen
  env:
    ANDROID_NDK_HOME: ${{ env.ANDROID_HOME }}/ndk/26.3.11579264
    OWNDRAW_ABIS: arm64-v8a x86_64
    OWNDRAW_KEYSTORE: ${{ runner.temp }}/release.jks
    OWNDRAW_KEYSTORE_PASS: ${{ secrets.ANDROID_KEYSTORE_PASS }}
    OWNDRAW_KEY_ALIAS: ${{ secrets.ANDROID_KEY_ALIAS }}
    OWNDRAW_KEY_PASS: ${{ secrets.ANDROID_KEY_PASS }}
  run: |
    echo "${{ secrets.ANDROID_KEYSTORE_B64 }}" | base64 -d > "$OWNDRAW_KEYSTORE"
    crates/owndraw-android/build-apk.sh
```

Der Keystore liegt base64-kodiert als Secret `ANDROID_KEYSTORE_B64` im Repository.

## Test im Emulator

```sh
sdkmanager "system-images;android-34;google_apis;x86_64" "emulator" "platform-tools"
avdmanager create avd -n owndraw34 -k "system-images;android-34;google_apis;x86_64" -d pixel_tablet
emulator -avd owndraw34 &
OWNDRAW_ABIS="arm64-v8a x86_64" OWNDRAW_PROFILE=debug crates/owndraw-android/build-apk.sh
adb install -r target/apk/owndraw-*.apk
adb logcat -s OwnDraw:V RustStdoutStderr:V AndroidRuntime:E
```

Prüfliste:

- **Zeichnen**: mit Maus (Emulator) bzw. Finger/Stift auf dem Gerät; Druckstärke nur mit echtem Stift sichtbar.
- **Öffnen/Speichern/Ordner**: Dateidialog des Systems erscheint; nach Ordnerwahl und App-Neustart ist der
  Ordner weiter beschreibbar (dauerhafte Rechte, `adb shell dumpsys activity permissions` bzw. erneutes Öffnen).
- **Öffnen mit**: Datei aufs Gerät legen und per Intent öffnen:
  ```sh
  adb push test.owndraw /sdcard/Download/
  adb shell am start -a android.intent.action.VIEW -t application/vnd.owndraw+zip \
      -d content://com.android.externalstorage.documents/document/primary%3ADownload%2Ftest.owndraw de.owndraw
  ```
  (oder im Dateimanager „Dateien“ antippen → OwnDraw).
- **Teilen**: Bild in „Fotos“ → Teilen → OwnDraw.
- **Zwischenablage**: Auswahl kopieren, in einer anderen App (z. B. Notizen/Messenger) als Bild einfügen;
  Bild in einer anderen App kopieren und in OwnDraw einfügen.
- **Drag & Drop**: Tablet-Gerät, Split-Screen mit „Dateien“ oder „Fotos“, Bild in OwnDraw ziehen.
- **Hintergrund**: Home-Taste während einer Änderung → nach Rückkehr bzw. Prozessende ist gespeichert.

## Bekannte Grenzen

- Werkzeugtyp (Finger/Stift/Radierer-Ende) liefert egui/winit 0.30 nicht; nur der Druck (`force`) kommt an.
- `openUrl` mit Dateien: nur Dateien im App-Speicher (Cache-/Datenordner) und nur die Datei selbst –
  relative Verweise einer exportierten Website (Bilder, CSS) kann der Browser über `content://` nicht nachladen.
- Debug-Logs: Tag `OwnDraw` (`android_logger`).
