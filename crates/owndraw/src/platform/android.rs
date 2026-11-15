//! Android-Plattform (WP5): Storage Access Framework, ContentResolver, Zwischenablage, Drag & Drop
//! und „Öffnen mit/Teilen“ per JNI.
//!
//! Aufbau:
//! - Rust → Java: JNI-Aufrufe über `ndk_context::android_context()` (JavaVM + Activity-Objekt).
//!   SAF-Dialoge, Lesen/Schreiben und Anlegen laufen direkt über Android-Klassen, Zwischenablage,
//!   URL-Öffnen und Dateifreigabe über Hilfsmethoden der Java-Klasse `de.owndraw.OwnDrawActivity`
//!   (`crates/owndraw-android/java`).
//! - Java → Rust: die Java-Klasse ruft native Methoden (`nativeOnPicked`, `nativeOnPause`, `nativeOnOpen`,
//!   `nativeOnDrop`, `nativeOnDragHover`, definiert im Crate `owndraw-android`), die hier über
//!   [`bridge`] in eine globale Warteschlange schreiben; [`Platform::poll`]/[`Platform::take_drops`] leeren sie.

use super::{ClipData, Drop, FileKind, FileRef, Platform, PlatformEvent, Tag};
use jni::objects::{JObject, JObjectArray, JString, JValue};
use jni::{JNIEnv, JavaVM};
use std::io::{self, Read, Write};
use std::path::{Path, PathBuf};
use std::sync::{Condvar, Mutex, OnceLock};
use std::time::{Duration, Instant};

/// Präfix für interne OwnDraw-Items als zusätzlicher Text-Eintrag in der System-Zwischenablage.
pub const ITEMS_PREFIX: &str = "owndraw-items:";

/// Wie lange `onPause` höchstens wartet, bis die App das Pause-Ereignis verarbeitet hat.
const PAUSE_WAIT: Duration = Duration::from_millis(2000);

// ------------------------------------------------------------------ Warteschlange Java → Rust

#[derive(Default)]
struct Queue {
    events: Vec<PlatformEvent>,
    /// Drops mit Position in Pixeln (Umrechnung in Punkte erst in `take_drops`).
    drops: Vec<Drop>,
    hover: bool,
    /// Zähler der `poll`-Aufrufe (für das Warten in `on_pause`).
    polls: u64,
    /// Pause-Ereignis liegt noch ungelesen in `events`.
    pause_pending: bool,
}

fn queue() -> &'static (Mutex<Queue>, Condvar) {
    static Q: OnceLock<(Mutex<Queue>, Condvar)> = OnceLock::new();
    Q.get_or_init(|| (Mutex::new(Queue::default()), Condvar::new()))
}

fn lock() -> std::sync::MutexGuard<'static, Queue> {
    queue().0.lock().unwrap_or_else(|e| e.into_inner())
}

static CTX: OnceLock<egui::Context> = OnceLock::new();

fn repaint() {
    if let Some(c) = CTX.get() {
        c.request_repaint();
    }
}

/// Einsprungpunkte für die JNI-Rückrufe (aufgerufen aus `owndraw-android`, im UI-Thread von Android).
pub mod bridge {
    use super::*;

    /// egui-Kontext merken, damit Rückrufe einen neuen Frame anstoßen können.
    pub fn set_context(ctx: &egui::Context) {
        let _ = CTX.set(ctx.clone());
    }

    pub fn on_picked(tag: i32, uri: Option<String>) {
        let file = uri.map(|u| uri_to_ref(&u));
        lock().events.push(PlatformEvent::Picked {
            tag: tag as Tag,
            file,
        });
        repaint();
    }

    /// Legt `Pause` in die Warteschlange und wartet (höchstens [`PAUSE_WAIT`]), bis die App es per
    /// `poll()` abgeholt und den nächsten Frame begonnen hat – so ist synchrones Speichern im Frame
    /// abgeschlossen, bevor Android die App in den Hintergrund schiebt.
    pub fn on_pause() {
        let (m, cv) = queue();
        let mut q = m.lock().unwrap_or_else(|e| e.into_inner());
        q.events.push(PlatformEvent::Pause);
        q.pause_pending = true;
        let start_polls = q.polls;
        drop(q);
        repaint();
        let deadline = Instant::now() + PAUSE_WAIT;
        let mut q = m.lock().unwrap_or_else(|e| e.into_inner());
        // Abgeholt (Frame mit Pause beginnt) und ein weiterer poll (Frame davor ist fertig).
        while q.pause_pending || q.polls < start_polls + 2 {
            let now = Instant::now();
            if now >= deadline {
                log::warn!("onPause: App hat das Pause-Ereignis nicht rechtzeitig verarbeitet");
                break;
            }
            let (g, _) = cv
                .wait_timeout(q, deadline - now)
                .unwrap_or_else(|e| e.into_inner());
            q = g;
            if !q.pause_pending {
                repaint();
            }
        }
    }

    pub fn on_open(uri: String) {
        lock()
            .events
            .push(PlatformEvent::OpenFile(uri_to_ref(&uri)));
        repaint();
    }

    pub fn on_drop(
        uri: Option<String>,
        text: Option<String>,
        mime: Option<String>,
        x: f32,
        y: f32,
    ) {
        let pos = Some((x, y));
        let d = if let Some(u) = uri {
            let file = uri_to_ref(&u);
            let name = match &file {
                FileRef::Path(_) => file.display_name(),
                FileRef::Uri(u) => query_name(u).unwrap_or_else(|| file.display_name()),
            };
            Drop {
                name: with_ext(name, mime.as_deref()),
                file: Some(file),
                bytes: None,
                pos,
            }
        } else if let Some(t) = text {
            Drop {
                file: None,
                bytes: Some(t.into_bytes()),
                name: "text.txt".into(),
                pos,
            }
        } else {
            return;
        };
        let mut q = lock();
        q.drops.push(d);
        q.hover = false;
        drop(q);
        repaint();
    }

    pub fn on_drag_hover(hover: bool) {
        lock().hover = hover;
        repaint();
    }
}

/// `content://…` bleibt URI, `file://…` bzw. absoluter Pfad wird zum Pfad.
fn uri_to_ref(u: &str) -> FileRef {
    if let Some(p) = u.strip_prefix("file://") {
        FileRef::Path(PathBuf::from(super::percent_decode(p)))
    } else if u.starts_with('/') {
        FileRef::Path(PathBuf::from(u))
    } else {
        FileRef::Uri(u.to_string())
    }
}

/// Hängt eine Endung passend zum MIME-Typ an, falls der Name keine bekannte hat
/// (z. B. `content://media/…/1234` ohne Dateinamen), damit der Drop-Router die Art erkennt.
fn with_ext(name: String, mime: Option<&str>) -> String {
    if FileKind::of_name(&name) != FileKind::Any {
        return name;
    }
    let ext = match mime.unwrap_or("") {
        "image/png" => "png",
        "image/jpeg" => "jpg",
        "image/webp" => "webp",
        "image/gif" => "gif",
        "image/bmp" => "bmp",
        "application/pdf" => "pdf",
        "application/x-xopp" => "xopp",
        "application/vnd.owndraw+zip" => "owndraw",
        _ => return name,
    };
    let base = if name.is_empty() { "drop" } else { &name };
    format!("{base}.{ext}")
}

// ------------------------------------------------------------------ JNI-Hilfen

/// Fehler innerhalb eines JNI-Abschnitts (die ausstehende Java-Ausnahme wird in [`with_env`] ausgewertet).
struct Fail(io::Error);

impl From<jni::errors::Error> for Fail {
    fn from(e: jni::errors::Error) -> Self {
        Fail(io::Error::other(e.to_string()))
    }
}
impl From<io::Error> for Fail {
    fn from(e: io::Error) -> Self {
        Fail(e)
    }
}

type R<T> = Result<T, Fail>;

/// Führt `f` mit angehängtem Thread, Activity-Objekt und eigenem Local-Frame aus
/// (der Rust-Thread bleibt dauerhaft angehängt, lokale Referenzen werden mit dem Frame freigegeben).
fn with_env<T>(f: impl FnOnce(&mut JNIEnv, &JObject) -> R<T>) -> io::Result<T> {
    let ctx = ndk_context::android_context();
    // SAFETY: android-activity setzt den Kontext vor `android_main`; VM und Activity-Global-Ref leben so lange wie die App.
    let vm = unsafe { JavaVM::from_raw(ctx.vm().cast()) }
        .map_err(|e| io::Error::other(e.to_string()))?;
    let mut env = vm
        .attach_current_thread_permanently()
        .map_err(|e| io::Error::other(e.to_string()))?;
    // SAFETY: globale Referenz der Activity (gehört android-activity, wird hier nicht freigegeben).
    let activity = unsafe { JObject::from_raw(ctx.context().cast()) };
    let r = env.with_local_frame(64, |env| {
        let r = f(env, &activity);
        // Ausstehende Ausnahme hier (noch im Frame) in eine io-Fehlermeldung übersetzen.
        r.map_err(|Fail(e)| Fail(take_exception(env).unwrap_or(e)))
    });
    r.map_err(|Fail(e)| e)
}

/// Holt und löscht eine ausstehende Java-Ausnahme und übersetzt sie in einen io-Fehler.
fn take_exception(env: &mut JNIEnv) -> Option<io::Error> {
    if !env.exception_check().unwrap_or(false) {
        return None;
    }
    let t = env.exception_occurred().ok();
    let _ = env.exception_clear();
    let t = t?;
    let msg = env
        .call_method(&t, "toString", "()Ljava/lang/String;", &[])
        .and_then(|v| v.l())
        .ok()
        .and_then(|s| env.get_string(&JString::from(s)).ok().map(String::from));
    let _ = env.exception_clear();
    let msg = msg.unwrap_or_else(|| "Java-Ausnahme".into());
    let kind = if msg.starts_with("java.lang.SecurityException") {
        io::ErrorKind::PermissionDenied
    } else if msg.starts_with("java.io.FileNotFoundException") {
        io::ErrorKind::NotFound
    } else if msg.starts_with("java.lang.UnsupportedOperationException")
        || msg.starts_with("java.lang.IllegalArgumentException")
    {
        io::ErrorKind::Unsupported
    } else {
        io::ErrorKind::Other
    };
    log::warn!("JNI: {msg}");
    Some(io::Error::new(kind, msg))
}

/// Ruft `close()` auf, ohne eine bereits ausstehende Ausnahme zu verlieren.
fn close_quietly(env: &mut JNIEnv, obj: &JObject) {
    if obj.is_null() {
        return;
    }
    let pending = if env.exception_check().unwrap_or(false) {
        let t = env.exception_occurred().ok();
        let _ = env.exception_clear();
        t
    } else {
        None
    };
    if env.call_method(obj, "close", "()V", &[]).is_err() {
        let _ = env.exception_clear();
    }
    if let Some(t) = pending {
        let _ = env.throw(t);
    }
}

fn jstr<'l>(env: &mut JNIEnv<'l>, s: &str) -> R<JObject<'l>> {
    Ok(env.new_string(s)?.into())
}

fn rstr(env: &mut JNIEnv, o: JObject) -> R<Option<String>> {
    if o.is_null() {
        return Ok(None);
    }
    Ok(Some(env.get_string(&JString::from(o))?.into()))
}

fn parse_uri<'l>(env: &mut JNIEnv<'l>, s: &str) -> R<JObject<'l>> {
    let js = jstr(env, s)?;
    Ok(env
        .call_static_method(
            "android/net/Uri",
            "parse",
            "(Ljava/lang/String;)Landroid/net/Uri;",
            &[JValue::Object(&js)],
        )?
        .l()?)
}

fn uri_string(env: &mut JNIEnv, uri: &JObject) -> R<Option<String>> {
    if uri.is_null() {
        return Ok(None);
    }
    let s = env
        .call_method(uri, "toString", "()Ljava/lang/String;", &[])?
        .l()?;
    rstr(env, s)
}

fn resolver<'l>(env: &mut JNIEnv<'l>, act: &JObject) -> R<JObject<'l>> {
    Ok(env
        .call_method(
            act,
            "getContentResolver",
            "()Landroid/content/ContentResolver;",
            &[],
        )?
        .l()?)
}

fn string_array<'l>(env: &mut JNIEnv<'l>, items: &[&str]) -> R<JObjectArray<'l>> {
    let arr = env.new_object_array(items.len() as i32, "java/lang/String", JObject::null())?;
    for (i, s) in items.iter().enumerate() {
        let js = env.new_string(s)?;
        env.set_object_array_element(&arr, i as i32, js)?;
    }
    Ok(arr)
}

fn new_intent<'l>(env: &mut JNIEnv<'l>, action: &str) -> R<JObject<'l>> {
    let a = jstr(env, action)?;
    Ok(env.new_object(
        "android/content/Intent",
        "(Ljava/lang/String;)V",
        &[JValue::Object(&a)],
    )?)
}

fn intent_str<'l>(env: &mut JNIEnv<'l>, intent: &JObject, method: &str, value: &str) -> R<()> {
    let v = jstr(env, value)?;
    env.call_method(
        intent,
        method,
        "(Ljava/lang/String;)Landroid/content/Intent;",
        &[JValue::Object(&v)],
    )?;
    Ok(())
}

fn intent_extra_str(env: &mut JNIEnv, intent: &JObject, key: &str, value: &str) -> R<()> {
    let k = jstr(env, key)?;
    let v = jstr(env, value)?;
    env.call_method(
        intent,
        "putExtra",
        "(Ljava/lang/String;Ljava/lang/String;)Landroid/content/Intent;",
        &[JValue::Object(&k), JValue::Object(&v)],
    )?;
    Ok(())
}

// Konstanten aus android.content.Intent
const FLAG_GRANT_READ_URI_PERMISSION: i32 = 0x0000_0001;
const FLAG_GRANT_WRITE_URI_PERMISSION: i32 = 0x0000_0002;
const FLAG_GRANT_PERSISTABLE_URI_PERMISSION: i32 = 0x0000_0040;

fn start_for_result(env: &mut JNIEnv, act: &JObject, intent: &JObject, tag: Tag) -> R<()> {
    let flags = FLAG_GRANT_READ_URI_PERMISSION
        | FLAG_GRANT_WRITE_URI_PERMISSION
        | FLAG_GRANT_PERSISTABLE_URI_PERMISSION;
    env.call_method(
        intent,
        "addFlags",
        "(I)Landroid/content/Intent;",
        &[JValue::Int(flags)],
    )?;
    // requestCode muss ≥ 0 sein; Tags > i32::MAX werden abgeschnitten.
    let code = (tag & 0x7fff_ffff) as i32;
    env.call_method(
        act,
        "startActivityForResult",
        "(Landroid/content/Intent;I)V",
        &[JValue::Object(intent), JValue::Int(code)],
    )?;
    Ok(())
}

/// MIME-Typen für den Öffnen-Dialog. `.owndraw`/`.xopp` haben bei vielen Anbietern keinen eigenen Typ
/// (dann `application/octet-stream` bzw. ZIP/GZIP), deshalb werden diese zusätzlich zugelassen.
fn open_mimes(kinds: &[FileKind]) -> Vec<&'static str> {
    let mut v: Vec<&'static str> = Vec::new();
    let mut add = |m: &'static str| {
        if !v.contains(&m) {
            v.push(m);
        }
    };
    for k in kinds {
        match k {
            FileKind::Any => return vec!["*/*"],
            FileKind::Owndraw => {
                add(k.mime());
                add("application/zip");
                add("application/octet-stream");
            }
            FileKind::Xopp => {
                add(k.mime());
                add("application/gzip");
                add("application/x-gzip");
                add("application/octet-stream");
            }
            _ => add(k.mime()),
        }
    }
    if v.is_empty() {
        v.push("*/*");
    }
    v
}

/// MIME-Typ für neu angelegte Dateien (kein Platzhalter erlaubt).
fn create_mime(kind: FileKind, name: &str) -> &'static str {
    match kind {
        FileKind::Image => match name
            .rsplit('.')
            .next()
            .unwrap_or("")
            .to_ascii_lowercase()
            .as_str()
        {
            "jpg" | "jpeg" => "image/jpeg",
            "webp" => "image/webp",
            _ => "image/png",
        },
        FileKind::Any => "application/octet-stream",
        k => k.mime(),
    }
}

/// Dokument-URI zu einem Baum-URI (`…/tree/<id>`) bzw. unverändert, wenn schon Dokument-URI.
fn document_uri<'l>(env: &mut JNIEnv<'l>, act: &JObject, uri: &JObject) -> R<JObject<'l>> {
    let is_doc = env
        .call_static_method(
            "android/provider/DocumentsContract",
            "isDocumentUri",
            "(Landroid/content/Context;Landroid/net/Uri;)Z",
            &[JValue::Object(act), JValue::Object(uri)],
        )?
        .z()?;
    if is_doc {
        return Ok(env.new_local_ref(uri)?);
    }
    let id = env
        .call_static_method(
            "android/provider/DocumentsContract",
            "getTreeDocumentId",
            "(Landroid/net/Uri;)Ljava/lang/String;",
            &[JValue::Object(uri)],
        )?
        .l()?;
    Ok(env
        .call_static_method(
            "android/provider/DocumentsContract",
            "buildDocumentUriUsingTree",
            "(Landroid/net/Uri;Ljava/lang/String;)Landroid/net/Uri;",
            &[JValue::Object(uri), JValue::Object(&id)],
        )?
        .l()?)
}

/// Fragt die erste Spalte `column` zu `uri` ab (None, wenn nicht vorhanden).
fn query_column(env: &mut JNIEnv, act: &JObject, uri: &JObject, column: &str) -> R<Option<String>> {
    let res = resolver(env, act)?;
    let proj = string_array(env, &[column])?;
    let null = JObject::null();
    let cur = env
        .call_method(
            &res,
            "query",
            "(Landroid/net/Uri;[Ljava/lang/String;Ljava/lang/String;[Ljava/lang/String;Ljava/lang/String;)Landroid/database/Cursor;",
            &[JValue::Object(uri), JValue::Object(&proj), JValue::Object(&null), JValue::Object(&null), JValue::Object(&null)],
        )?
        .l()?;
    if cur.is_null() {
        return Ok(None);
    }
    let r = (|| -> R<Option<String>> {
        if !env.call_method(&cur, "moveToFirst", "()Z", &[])?.z()? {
            return Ok(None);
        }
        let s = env
            .call_method(
                &cur,
                "getString",
                "(I)Ljava/lang/String;",
                &[JValue::Int(0)],
            )?
            .l()?;
        rstr(env, s)
    })();
    close_quietly(env, &cur);
    r
}

/// Anzeigename (`OpenableColumns.DISPLAY_NAME`) eines content-URI.
fn query_name(uri: &str) -> Option<String> {
    with_env(|env, act| {
        let u = parse_uri(env, uri)?;
        query_column(env, act, &u, "_display_name")
    })
    .ok()
    .flatten()
}

fn read_uri(uri: &str) -> io::Result<Vec<u8>> {
    with_env(|env, act| {
        let u = parse_uri(env, uri)?;
        let res = resolver(env, act)?;
        let is = env
            .call_method(
                &res,
                "openInputStream",
                "(Landroid/net/Uri;)Ljava/io/InputStream;",
                &[JValue::Object(&u)],
            )?
            .l()?;
        if is.is_null() {
            return Err(Fail(io::Error::new(
                io::ErrorKind::NotFound,
                "openInputStream lieferte null",
            )));
        }
        const N: usize = 64 * 1024;
        let buf = env.new_byte_array(N as i32)?;
        let mut out: Vec<u8> = Vec::new();
        let mut tmp = vec![0i8; N];
        let r = (|| -> R<()> {
            loop {
                let n = env
                    .call_method(&is, "read", "([B)I", &[JValue::Object(&buf)])?
                    .i()?;
                if n < 0 {
                    return Ok(());
                }
                let n = (n as usize).min(N);
                env.get_byte_array_region(&buf, 0, &mut tmp[..n])?;
                out.extend(tmp[..n].iter().map(|&b| b as u8));
            }
        })();
        close_quietly(env, &is);
        r.map(|_| out)
    })
}

/// Schreibt erst in eine Cache-Datei, dann über den ContentResolver (Modus "wt", sonst "rwt") in den URI.
/// Die Cache-Kopie bleibt bei einem Fehler erhalten (Pfad steht in der Fehlermeldung).
fn write_uri(cache: &Path, uri: &str, data: &[u8]) -> io::Result<()> {
    let dir = cache.join("saf-write");
    std::fs::create_dir_all(&dir)?;
    let tmp = dir.join(format!("{:016x}.tmp", fnv1a(uri.as_bytes())));
    {
        let mut f = std::fs::File::create(&tmp)?;
        f.write_all(data)?;
        f.sync_all()?;
    }
    let r = with_env(|env, act| {
        let u = parse_uri(env, uri)?;
        let res = resolver(env, act)?;
        let mut os = JObject::null();
        let mut last = None;
        for mode in ["wt", "rwt"] {
            let m = jstr(env, mode)?;
            match env
                .call_method(
                    &res,
                    "openOutputStream",
                    "(Landroid/net/Uri;Ljava/lang/String;)Ljava/io/OutputStream;",
                    &[JValue::Object(&u), JValue::Object(&m)],
                )
                .and_then(|v| v.l())
            {
                Ok(o) if !o.is_null() => {
                    os = o;
                    break;
                }
                Ok(_) => last = Some(io::Error::other("openOutputStream lieferte null")),
                Err(e) => {
                    last =
                        Some(take_exception(env).unwrap_or_else(|| io::Error::other(e.to_string())))
                }
            }
        }
        if os.is_null() {
            return Err(Fail(last.unwrap_or_else(|| {
                io::Error::other("openOutputStream fehlgeschlagen")
            })));
        }
        const N: usize = 64 * 1024;
        let buf = env.new_byte_array(N as i32)?;
        let r = (|| -> R<()> {
            let mut f = std::fs::File::open(&tmp)?;
            let mut chunk = vec![0u8; N];
            loop {
                let n = f.read(&mut chunk)?;
                if n == 0 {
                    break;
                }
                // SAFETY: u8 und i8 haben gleiche Größe und Ausrichtung.
                let signed = unsafe { std::slice::from_raw_parts(chunk.as_ptr().cast::<i8>(), n) };
                env.set_byte_array_region(&buf, 0, signed)?;
                env.call_method(
                    &os,
                    "write",
                    "([BII)V",
                    &[JValue::Object(&buf), JValue::Int(0), JValue::Int(n as i32)],
                )?;
            }
            env.call_method(&os, "flush", "()V", &[])?;
            Ok(())
        })();
        if r.is_err() {
            close_quietly(env, &os);
            return r;
        }
        // close() kann bei Cloud-Anbietern erst den Upload auslösen → Fehler hier melden.
        env.call_method(&os, "close", "()V", &[])?;
        Ok(())
    });
    match r {
        Ok(()) => {
            let _ = std::fs::remove_file(&tmp);
            Ok(())
        }
        Err(e) => Err(io::Error::new(
            e.kind(),
            format!("{e} (Kopie: {})", tmp.display()),
        )),
    }
}

fn fnv1a(b: &[u8]) -> u64 {
    b.iter().fold(0xcbf2_9ce4_8422_2325u64, |h, &c| {
        (h ^ c as u64).wrapping_mul(0x0100_0000_01b3)
    })
}

/// Schreibt atomar über eine temporäre Datei im selben Ordner und Umbenennen.
fn write_path_atomic(p: &Path, data: &[u8]) -> io::Result<()> {
    let dir = p
        .parent()
        .filter(|d| !d.as_os_str().is_empty())
        .unwrap_or(Path::new("."));
    std::fs::create_dir_all(dir)?;
    let name = p
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_default();
    let tmp = dir.join(format!(".{name}.{}.tmp", std::process::id()));
    let r = (|| {
        let mut f = std::fs::File::create(&tmp)?;
        f.write_all(data)?;
        f.sync_all()?;
        std::fs::rename(&tmp, p)
    })();
    if r.is_err() {
        let _ = std::fs::remove_file(&tmp);
    }
    r
}

/// Freier Dateiname im Ordner: `name`, sonst `Basis (2).ext`, `Basis (3).ext`, …
fn unique_path(dir: &Path, name: &str) -> PathBuf {
    let p = dir.join(name);
    if !p.exists() {
        return p;
    }
    let (base, ext) = match name.rfind('.') {
        Some(i) if i > 0 => (&name[..i], &name[i..]),
        _ => (name, ""),
    };
    (2..)
        .map(|n| dir.join(format!("{base} ({n}){ext}")))
        .find(|p| !p.exists())
        .expect("unendliche Folge")
}

// ------------------------------------------------------------------ Plattform

pub struct AndroidPlatform {
    /// App-interner Speicher (`internal_data_path`), zugleich Konfig-Ordner.
    data_dir: PathBuf,
    cache_dir: PathBuf,
}

impl AndroidPlatform {
    /// `data_dir` = `AndroidApp::internal_data_path()`.
    pub fn new(data_dir: PathBuf) -> AndroidPlatform {
        let cache_dir = with_env(|env, act| {
            let f = env
                .call_method(act, "getCacheDir", "()Ljava/io/File;", &[])?
                .l()?;
            let s = env
                .call_method(&f, "getAbsolutePath", "()Ljava/lang/String;", &[])?
                .l()?;
            rstr(env, s)
        })
        .ok()
        .flatten()
        .map(PathBuf::from)
        .unwrap_or_else(|| data_dir.parent().unwrap_or(&data_dir).join("cache"));
        AndroidPlatform {
            data_dir,
            cache_dir,
        }
    }

    /// Gewählter Standard-Ort aus `settings.json` (Feld `default_dir`), falls gesetzt.
    fn configured_dir(&self) -> Option<FileRef> {
        let text = std::fs::read_to_string(self.config_dir().join("settings.json")).ok()?;
        let v: serde_json::Value = serde_json::from_str(&text).ok()?;
        FileRef::from_key(v.get("default_dir")?.as_str()?)
    }

    /// PNG in `cache/clip/` ablegen (ältere Zwischenablage-Bilder werden gelöscht).
    fn clip_png(&self, w: u32, h: u32, rgba: Vec<u8>) -> Option<PathBuf> {
        let img = image::RgbaImage::from_raw(w, h, rgba)?;
        let mut png = Vec::new();
        image::DynamicImage::ImageRgba8(img)
            .write_to(&mut io::Cursor::new(&mut png), image::ImageFormat::Png)
            .ok()?;
        let dir = self.cache_dir.join("clip");
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).ok()?;
        let ms = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_millis())
            .unwrap_or(0);
        let p = dir.join(format!("owndraw-{ms}.png"));
        std::fs::write(&p, png).ok()?;
        Some(p)
    }
}

fn decode_rgba(bytes: &[u8]) -> Option<(u32, u32, Vec<u8>)> {
    let img = image::load_from_memory(bytes).ok()?.into_rgba8();
    let (w, h) = img.dimensions();
    Some((w, h, img.into_raw()))
}

fn log_err<T>(what: &str, r: io::Result<T>) -> Option<T> {
    r.map_err(|e| log::warn!("{what}: {e}")).ok()
}

impl Platform for AndroidPlatform {
    fn pick_open(&self, kinds: &[FileKind], tag: Tag) {
        let mimes = open_mimes(kinds);
        let r = with_env(|env, act| {
            let i = new_intent(env, "android.intent.action.OPEN_DOCUMENT")?;
            intent_str(env, &i, "addCategory", "android.intent.category.OPENABLE")?;
            if mimes.len() == 1 {
                intent_str(env, &i, "setType", mimes[0])?;
            } else {
                intent_str(env, &i, "setType", "*/*")?;
                let k = jstr(env, "android.intent.extra.MIME_TYPES")?;
                let arr = string_array(env, &mimes)?;
                env.call_method(
                    &i,
                    "putExtra",
                    "(Ljava/lang/String;[Ljava/lang/String;)Landroid/content/Intent;",
                    &[JValue::Object(&k), JValue::Object(&arr)],
                )?;
            }
            start_for_result(env, act, &i, tag)
        });
        if log_err("Öffnen-Dialog", r).is_none() {
            on_dialog_failed(tag);
        }
    }

    fn pick_save(&self, name: &str, kind: FileKind, tag: Tag) {
        let r = with_env(|env, act| {
            let i = new_intent(env, "android.intent.action.CREATE_DOCUMENT")?;
            intent_str(env, &i, "addCategory", "android.intent.category.OPENABLE")?;
            intent_str(env, &i, "setType", create_mime(kind, name))?;
            intent_extra_str(env, &i, "android.intent.extra.TITLE", name)?;
            start_for_result(env, act, &i, tag)
        });
        if log_err("Speichern-Dialog", r).is_none() {
            on_dialog_failed(tag);
        }
    }

    fn pick_folder(&self, tag: Tag) {
        // Dauerhafte Rechte (takePersistableUriPermission) holt OwnDrawActivity.onActivityResult.
        let r = with_env(|env, act| {
            let i = new_intent(env, "android.intent.action.OPEN_DOCUMENT_TREE")?;
            start_for_result(env, act, &i, tag)
        });
        if log_err("Ordner-Dialog", r).is_none() {
            on_dialog_failed(tag);
        }
    }

    fn poll(&self) -> Vec<PlatformEvent> {
        let (m, cv) = queue();
        let mut q = m.lock().unwrap_or_else(|e| e.into_inner());
        q.polls += 1;
        q.pause_pending = false;
        let ev = std::mem::take(&mut q.events);
        drop(q);
        cv.notify_all();
        ev
    }

    fn read(&self, f: &FileRef) -> io::Result<Vec<u8>> {
        match f {
            FileRef::Path(p) => std::fs::read(p),
            FileRef::Uri(u) => read_uri(u),
        }
    }

    fn write_atomic(&self, f: &FileRef, data: &[u8]) -> io::Result<()> {
        match f {
            FileRef::Path(p) => write_path_atomic(p, data),
            FileRef::Uri(u) => write_uri(&self.cache_dir, u, data),
        }
    }

    fn create_in(&self, dir: &FileRef, name: &str, kind: FileKind) -> io::Result<FileRef> {
        match dir {
            FileRef::Path(d) => {
                std::fs::create_dir_all(d)?;
                let p = unique_path(d, name);
                std::fs::OpenOptions::new()
                    .write(true)
                    .create_new(true)
                    .open(&p)?;
                Ok(FileRef::Path(p))
            }
            FileRef::Uri(tree) => {
                let mime = create_mime(kind, name);
                let s = with_env(|env, act| {
                    let t = parse_uri(env, tree)?;
                    let parent = document_uri(env, act, &t)?;
                    let res = resolver(env, act)?;
                    let m = jstr(env, mime)?;
                    let n = jstr(env, name)?;
                    let u = env
                        .call_static_method(
                            "android/provider/DocumentsContract",
                            "createDocument",
                            "(Landroid/content/ContentResolver;Landroid/net/Uri;Ljava/lang/String;Ljava/lang/String;)Landroid/net/Uri;",
                            &[JValue::Object(&res), JValue::Object(&parent), JValue::Object(&m), JValue::Object(&n)],
                        )?
                        .l()?;
                    uri_string(env, &u)
                })?;
                s.map(FileRef::Uri)
                    .ok_or_else(|| io::Error::other("createDocument lieferte null"))
            }
        }
    }

    fn exists(&self, f: &FileRef) -> bool {
        match f {
            FileRef::Path(p) => p.exists(),
            FileRef::Uri(u) => with_env(|env, act| {
                let uri = parse_uri(env, u)?;
                let d = document_uri(env, act, &uri).unwrap_or(uri);
                let _ = env.exception_clear();
                Ok(query_column(env, act, &d, "_display_name")?.is_some())
            })
            .unwrap_or(false),
        }
    }

    fn default_dir(&self) -> FileRef {
        self.configured_dir()
            .unwrap_or_else(|| FileRef::Path(self.data_dir.join("boards")))
    }

    fn config_dir(&self) -> PathBuf {
        self.data_dir.clone()
    }

    fn cache_dir(&self) -> PathBuf {
        self.cache_dir.clone()
    }

    fn clipboard_get(&self) -> Option<ClipData> {
        // Einträge von OwnDrawActivity.clipGet(): "t<Text>" bzw. "u<MIME>\n<URI>".
        let entries = log_err(
            "Zwischenablage lesen",
            with_env(|env, act| {
                let arr = env
                    .call_method(act, "clipGet", "()[Ljava/lang/String;", &[])?
                    .l()?;
                if arr.is_null() {
                    return Ok(Vec::new());
                }
                let arr = JObjectArray::from(arr);
                let n = env.get_array_length(&arr)?;
                let mut v = Vec::new();
                for i in 0..n {
                    let o = env.get_object_array_element(&arr, i)?;
                    if let Some(s) = rstr(env, o)? {
                        v.push(s);
                    }
                }
                Ok(v)
            }),
        )?;
        let (mut json, mut text, mut image) = (None, None, None);
        for e in entries {
            if let Some(t) = e.strip_prefix('t') {
                match t.strip_prefix(ITEMS_PREFIX) {
                    Some(j) => json = Some(j.to_string()),
                    None if text.is_none() => text = Some(t.to_string()),
                    None => {}
                }
            } else if let Some(u) = e.strip_prefix('u') {
                let (mime, uri) = u.split_once('\n').unwrap_or(("", u));
                if image.is_none() && (mime.is_empty() || mime.starts_with("image/")) {
                    image = self
                        .read(&uri_to_ref(uri))
                        .ok()
                        .and_then(|b| decode_rgba(&b));
                }
            }
        }
        match (json, image, text) {
            (Some(json), image, _) => Some(ClipData::Items { json, image }),
            (None, Some((w, h, rgba)), _) => Some(ClipData::Image { w, h, rgba }),
            (None, None, Some(t)) => Some(ClipData::Text(t)),
            _ => None,
        }
    }

    fn clipboard_set(&self, d: ClipData) {
        let (text, png) = match d {
            ClipData::Text(t) => (Some(t), None),
            ClipData::Image { w, h, rgba } => (None, self.clip_png(w, h, rgba)),
            ClipData::Items { json, image } => (
                Some(format!("{ITEMS_PREFIX}{json}")),
                image.and_then(|(w, h, rgba)| self.clip_png(w, h, rgba)),
            ),
        };
        let png = png.map(|p| p.to_string_lossy().into_owned());
        log_err(
            "Zwischenablage setzen",
            with_env(|env, act| {
                let t = match &text {
                    Some(t) => jstr(env, t)?,
                    None => JObject::null(),
                };
                let p = match &png {
                    Some(p) => jstr(env, p)?,
                    None => JObject::null(),
                };
                env.call_method(
                    act,
                    "clipSet",
                    "(Ljava/lang/String;Ljava/lang/String;)V",
                    &[JValue::Object(&t), JValue::Object(&p)],
                )?;
                Ok(())
            }),
        );
    }

    fn take_drops(&self, ctx: &egui::Context) -> Vec<Drop> {
        let ppp = ctx.pixels_per_point().max(0.01);
        let mut v = std::mem::take(&mut lock().drops);
        for d in &mut v {
            d.pos = d.pos.map(|(x, y)| (x / ppp, y / ppp));
        }
        v
    }

    fn drag_hover(&self, _ctx: &egui::Context) -> bool {
        lock().hover
    }

    fn open_url(&self, url: &str) {
        // Java-Seite: http(s) direkt, Dateien im App-Speicher über den eigenen FileProvider.
        log_err(
            "URL öffnen",
            with_env(|env, act| {
                let u = jstr(env, url)?;
                env.call_method(
                    act,
                    "openUrl",
                    "(Ljava/lang/String;)V",
                    &[JValue::Object(&u)],
                )?;
                Ok(())
            }),
        );
    }

    fn is_touch(&self) -> bool {
        true
    }
}

// ---- Update-Dienst (WP11): HTTP über die Java-Seite, Installation über den System-Installer ----

/// Textantwort per `HttpURLConnection` (nur HTTPS, Java prüft das erneut).
pub fn http_get(url: &str) -> io::Result<String> {
    with_env(|env, act| {
        let u = jstr(env, url)?;
        let r = env
            .call_method(
                act,
                "httpGet",
                "(Ljava/lang/String;)Ljava/lang/String;",
                &[JValue::Object(&u)],
            )?
            .l()?;
        Ok(rstr(env, r)?.unwrap_or_default())
    })
}

/// Lädt nach `dest`. Der blockierende Java-Aufruf läuft in einem eigenen Thread; dieser hier meldet den
/// Fortschritt (Dateigröße) und gibt den Abbruch weiter.
pub fn http_download(
    url: &str,
    dest: &Path,
    cancelled: &dyn Fn() -> bool,
    progress: &mut dyn FnMut(u64, Option<u64>),
) -> io::Result<()> {
    let done = std::sync::atomic::AtomicBool::new(false);
    std::thread::scope(|s| {
        let h = s.spawn(|| {
            let r = with_env(|env, act| {
                let u = jstr(env, url)?;
                let d = jstr(env, &dest.to_string_lossy())?;
                env.call_method(
                    act,
                    "httpDownload",
                    "(Ljava/lang/String;Ljava/lang/String;)J",
                    &[JValue::Object(&u), JValue::Object(&d)],
                )?;
                Ok(())
            });
            done.store(true, std::sync::atomic::Ordering::SeqCst);
            r
        });
        let mut sent_cancel = false;
        while !done.load(std::sync::atomic::Ordering::SeqCst) {
            std::thread::sleep(Duration::from_millis(150));
            progress(std::fs::metadata(dest).map(|m| m.len()).unwrap_or(0), None);
            if !sent_cancel && cancelled() {
                sent_cancel = true;
                log_err(
                    "Download abbrechen",
                    with_env(|env, act| {
                        env.call_method(act, "updateCancel", "()V", &[])?;
                        Ok(())
                    }),
                );
            }
        }
        h.join()
            .unwrap_or_else(|_| Err(io::Error::other("Download-Thread abgestürzt")))
    })
}

/// Übergibt die (bereits signaturgeprüfte) APK an den System-Installer; liefert einen Hinweistext.
pub fn install_apk(apk: &Path) -> io::Result<String> {
    with_env(|env, act| {
        let p = jstr(env, &apk.to_string_lossy())?;
        let r = env
            .call_method(
                act,
                "installApk",
                "(Ljava/lang/String;)Ljava/lang/String;",
                &[JValue::Object(&p)],
            )?
            .l()?;
        Ok(rstr(env, r)?.unwrap_or_default())
    })
}

/// Dialog konnte nicht gestartet werden → als Abbruch melden, damit die App nicht wartet.
fn on_dialog_failed(tag: Tag) {
    lock()
        .events
        .push(PlatformEvent::Picked { tag, file: None });
    repaint();
}
