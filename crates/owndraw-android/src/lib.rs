//! OwnDraw für Android: Einstiegspunkt der NativeActivity (`android_main`) und JNI-Rückrufe der
//! Java-Hilfsklasse `de.owndraw.OwnDrawActivity` (siehe `java/`, `AndroidManifest.xml`, `build-apk.sh`).
//!
//! Die eigentliche App und die Plattform-Schicht liegen im Crate `owndraw`; hier nur die dünne Hülle.
//! Die JNI-Funktionen stehen bewusst in diesem cdylib-Crate, damit sie sicher exportiert werden.
#![cfg(target_os = "android")]

use android_activity::AndroidApp;
use jni::objects::{JObject, JString};
use jni::sys::{jboolean, jfloat, jint};
use jni::JNIEnv;
use owndraw_app::android_bridge as bridge;

/// Wird von `android-activity` im eigenen Thread aufgerufen, sobald die NativeActivity steht.
#[no_mangle]
fn android_main(app: AndroidApp) {
    android_logger::init_once(
        android_logger::Config::default()
            .with_max_level(log::LevelFilter::Info)
            .with_tag("OwnDraw"),
    );
    std::panic::set_hook(Box::new(|p| log::error!("Panik: {p}")));
    if let Err(e) = owndraw_app::run_android(app) {
        log::error!("OwnDraw beendet mit Fehler: {e}");
    }
}

/// Java-String (darf null sein) → Rust.
fn opt_string(env: &mut JNIEnv, s: &JString) -> Option<String> {
    if s.is_null() {
        return None;
    }
    env.get_string(s).ok().map(Into::into)
}

/// Ergebnis eines SAF-Dialogs (`onActivityResult`): `tag` = requestCode, `uri` null bei Abbruch.
#[no_mangle]
pub extern "system" fn Java_de_owndraw_OwnDrawActivity_nativeOnPicked<'l>(
    mut env: JNIEnv<'l>,
    _this: JObject<'l>,
    tag: jint,
    uri: JString<'l>,
) {
    let uri = opt_string(&mut env, &uri);
    bridge::on_picked(tag, uri);
}

/// `onPause`: blockiert kurz, bis die App das Ereignis abgeholt hat (Speichern vor dem Hintergrund).
#[no_mangle]
pub extern "system" fn Java_de_owndraw_OwnDrawActivity_nativeOnPause<'l>(
    _env: JNIEnv<'l>,
    _this: JObject<'l>,
) {
    bridge::on_pause();
}

/// Start-Intent bzw. `onNewIntent` („Öffnen mit“, „Teilen“).
#[no_mangle]
pub extern "system" fn Java_de_owndraw_OwnDrawActivity_nativeOnOpen<'l>(
    mut env: JNIEnv<'l>,
    _this: JObject<'l>,
    uri: JString<'l>,
) {
    if let Some(u) = opt_string(&mut env, &uri) {
        bridge::on_open(u);
    }
}

/// Fallen gelassenes Element (Drag & Drop aus anderer App); `x`/`y` in Pixeln des Fensters.
#[no_mangle]
pub extern "system" fn Java_de_owndraw_OwnDrawActivity_nativeOnDrop<'l>(
    mut env: JNIEnv<'l>,
    _this: JObject<'l>,
    uri: JString<'l>,
    text: JString<'l>,
    mime: JString<'l>,
    x: jfloat,
    y: jfloat,
) {
    let uri = opt_string(&mut env, &uri);
    let text = opt_string(&mut env, &text);
    let mime = opt_string(&mut env, &mime);
    bridge::on_drop(uri, text, mime, x, y);
}

/// Ziehen über dem Fenster begonnen/beendet (Ablage-Hinweis).
#[no_mangle]
pub extern "system" fn Java_de_owndraw_OwnDrawActivity_nativeOnDragHover<'l>(
    _env: JNIEnv<'l>,
    _this: JObject<'l>,
    hover: jboolean,
) {
    bridge::on_drag_hover(hover != 0);
}
