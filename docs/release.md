# Release-Anleitung

Releases entstehen automatisch: Ein Tag `v*` startet `.github/workflows/release.yml`. Die Action baut AppImage, APK und Quell-Tarball, signiert alle Assets mit minisign und legt das GitHub-Release an. Ohne Signaturschlüssel oder Android-Keystore bricht der Lauf sofort ab; es entstehen keine unsignierten Releases.

## 1. minisign-Schlüssel erzeugen (einmalig)

```sh
minisign -G -p minisign.pub -s minisign.key
```

- Passwort wählen und sicher aufbewahren.
- `minisign.pub` (öffentlich) in die App einkompilieren: Datei `crates/owndraw/src/update/minisign.pub` ins Repository committen (wird per `include_str!` einkompiliert).
- `minisign.key` (geheim) niemals committen.

## 2. Secrets anlegen

GitHub → Settings → Secrets and variables → Actions:

| Secret | Inhalt |
|---|---|
| `MINISIGN_KEY` | kompletter Inhalt von `minisign.key` (beide Zeilen) |
| `MINISIGN_PASSWORD` | Passwort des minisign-Schlüssels |
| `ANDROID_KEYSTORE_B64` | Release-Keystore, base64-kodiert (siehe 3.) |
| `ANDROID_KEYSTORE_PASS` | Keystore-Passwort (wird auch für den Schlüssel verwendet) |
| `ANDROID_KEY_ALIAS` | Alias des Schlüssels im Keystore |

Der CI-Lauf (`ci.yml`, Push/PR auf `main`) braucht die Secrets nicht. Fehlt `ANDROID_KEYSTORE_B64` dort, wird die APK mit dem Debug-Schlüssel gebaut und das im Log gewarnt.

## 3. Android-Keystore erzeugen (einmalig)

```sh
keytool -genkeypair -v -keystore owndraw-release.jks -alias owndraw \
  -keyalg RSA -keysize 4096 -validity 10000
base64 -w0 owndraw-release.jks > owndraw-release.jks.b64   # Inhalt als Secret ANDROID_KEYSTORE_B64
```

- Den Keystore sichern (Passwortmanager, Offline-Backup). **Verlust bedeutet: bestehende Installationen können nicht mehr aktualisiert werden.**
- Den Keystore-Inhalt nicht committen. Die `.b64`-Datei nach dem Anlegen des Secrets löschen.

## 4. Release auslösen

1. Version in `Cargo.toml` unter `[workspace.package] version` anheben (z. B. `3.0.1`), committen und auf `main` mergen.
2. Tag setzen und pushen:
   ```sh
   git tag v3.0.1 && git push origin v3.0.1
   ```
3. Die Release-Action prüft zuerst, ob Tag und Workspace-Version übereinstimmen. Bei Abweichung bricht sie ab.
4. Release-Text: Abschnitt `## [3.0.1]` (oder `## v3.0.1`) aus `CHANGELOG.md`. Fehlt er, erzeugt GitHub den Text automatisch.

## 5. Assets eines Releases

| Datei | Zweck |
|---|---|
| `OwnDraw-<v>-x86_64.AppImage` | Linux-Desktop |
| `OwnDraw-<v>.apk` | Android (mit Release-Schlüssel signiert) |
| `OwnDraw-<v>-src.tar.gz` | Quellcode für „Aus Quellcode bauen“ |
| `<datei>.minisig` | minisign-Signatur zu jedem der drei Assets |

Die App prüft die Signatur gegen den einkompilierten öffentlichen Schlüssel, bevor sie etwas installiert.

## 6. Öffentlicher Schlüssel in der App (Pflicht vor dem ersten Release)

`crates/owndraw/src/update/minisign.pub` enthält den echten öffentlichen Release-Schlüssel (Schlüssel-ID `FE8B7DCAA4C21501`, erzeugt am 14.11.2026). Der geheime Schlüssel gehört nur in das GitHub-Secret `MINISIGN_KEY`, das Passwort in `MINISIGN_PASSWORD`. Bei einem Schlüsselwechsel die neue `minisign.pub` hierher kopieren; Apps mit dem alten Schlüssel können Releases mit neuem Schlüssel nicht mehr installieren. Der Test-Platzhalter liegt in `placeholder.pub` (nur für Tests).

Die Update-Tests erzeugen ihre Schlüssel zur Laufzeit (Crate `minisign` als dev-dependency) und brauchen den echten Schlüssel nicht.

## Hinweise

- Schlüsselwechsel (minisign oder Android-Keystore) bricht Updates für bestehende Installationen. Nur im Notfall, mit Ankündigung.
- Lokaler Test des AppImage: `./build-appimage.sh`; APK: `crates/owndraw-android/build-apk.sh` (braucht `ANDROID_HOME`, `ANDROID_NDK_HOME`, `cargo-ndk`).
