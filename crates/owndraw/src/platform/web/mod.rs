//! Browser-Plattform (WebAssembly, Gastmodus ohne Server).
//!
//! - Dateien: virtuelles Dateisystem im Speicher, Verweise `FileRef::Uri("mem:<name>")`. Boards (`mem:<name>`)
//!   werden zusätzlich in IndexedDB (Datenbank `owndraw`, Ablage `files`) gesichert und beim Start geladen
//!   ([`Store::load`]); Hochgeladenes, das kein Board ist, liegt unter `mem:upload/…` nur im Speicher.
//! - Öffnen: `<input type=file>`; das Ergebnis kommt als `PlatformEvent::Picked` mit einem `mem:`-Verweis.
//! - Speichern unter / Export: `pick_save` liefert sofort `mem:download/<name>`; `write_atomic` darauf löst einen
//!   Download (Blob + `<a download>`) aus.
//! - Konfiguration (`settings.json`, `recent.json`, Pfade unter [`CONFIG`]) in `localStorage` (siehe `kv.rs`).
//! - Zwischenablage: intern + Text über egui (`navigator.clipboard`); eingefügte Bilder über ein eigenes
//!   `paste`-Ereignis (kommen als Drop an). Drops über egui `dropped_files` (Bytes, asynchron gelesen).
//! - Warnung beim Verlassen der Seite, solange es nicht heruntergeladene Änderungen gibt ([`set_unsaved`]).
//!
//! Server-Anbindung (Login, Boards am Server, siehe `docs/web-api.md`) folgt in [`api`].

pub mod api;

use super::{ClipData, Drop, FileKind, FileRef, Platform, PlatformEvent, Tag};
use std::cell::RefCell;
use std::collections::{HashMap, HashSet};
use std::io;
use std::path::PathBuf;
use std::rc::Rc;
use std::sync::atomic::{AtomicBool, Ordering};
use wasm_bindgen::prelude::*;
use wasm_bindgen::JsCast;
use wasm_bindgen_futures::{spawn_local, JsFuture};
use web_sys::{IdbDatabase, IdbRequest, IdbTransactionMode};

/// Wurzel der „Pfade“ für Konfiguration und Cache (liegen in `localStorage`, siehe `kv.rs`).
pub const CONFIG: &str = "/owndraw/config";
const CACHE: &str = "/owndraw/cache";
const DB_NAME: &str = "owndraw";
const DB_STORE: &str = "files";
const MEM: &str = "mem:";
const DOWNLOAD: &str = "mem:download/";
const UPLOAD: &str = "mem:upload/";

static UNSAVED: AtomicBool = AtomicBool::new(false);

/// Gibt es Änderungen, die noch nicht heruntergeladen wurden? (für die Warnung beim Verlassen)
pub fn set_unsaved(v: bool) {
    UNSAVED.store(v, Ordering::Relaxed);
}

fn warn(msg: &str) {
    web_sys::console::warn_1(&JsValue::from_str(msg));
}

fn js_err(e: JsValue) -> io::Error {
    io::Error::other(e.as_string().unwrap_or_else(|| format!("{e:?}")))
}

fn document() -> Option<web_sys::Document> {
    web_sys::window()?.document()
}

/// Seiten-Ereignisse: Warnung beim Schließen/Neuladen, solange nicht heruntergeladene Änderungen bestehen.
pub fn install_page_hooks() {
    let Some(w) = web_sys::window() else { return };
    let cb = Closure::<dyn FnMut(web_sys::BeforeUnloadEvent)>::new(|e: web_sys::BeforeUnloadEvent| {
        if UNSAVED.load(Ordering::Relaxed) {
            e.prevent_default();
            e.set_return_value("Ungespeicherte Änderungen: Das Board wurde noch nicht heruntergeladen.");
        }
    });
    let _ = w.add_event_listener_with_callback("beforeunload", cb.as_ref().unchecked_ref());
    cb.forget();
}

// ---------------------------------------------------------------- IndexedDB

/// Wartet auf ein IndexedDB-Ergebnis (`onsuccess`/`onerror`).
async fn idb_wait(req: &IdbRequest) -> Result<JsValue, JsValue> {
    let r = req.clone();
    let p = js_sys::Promise::new(&mut |resolve: js_sys::Function, reject: js_sys::Function| {
        let r2 = r.clone();
        let ok = Closure::once_into_js(move |_e: web_sys::Event| {
            let _ = resolve.call1(&JsValue::NULL, &r2.result().unwrap_or(JsValue::UNDEFINED));
        });
        r.set_onsuccess(Some(ok.unchecked_ref()));
        let err = Closure::once_into_js(move |_e: web_sys::Event| {
            let _ = reject.call1(&JsValue::NULL, &JsValue::from_str("IndexedDB-Fehler"));
        });
        r.set_onerror(Some(err.unchecked_ref()));
    });
    JsFuture::from(p).await
}

async fn idb_open() -> Result<IdbDatabase, JsValue> {
    let f = web_sys::window().ok_or("kein window")?.indexed_db()?.ok_or("kein IndexedDB")?;
    let req = f.open_with_u32(DB_NAME, 1)?;
    let up = Closure::once_into_js(move |e: web_sys::IdbVersionChangeEvent| {
        let db = e.target().and_then(|t| t.dyn_into::<IdbRequest>().ok()).and_then(|r| r.result().ok()).and_then(|d| d.dyn_into::<IdbDatabase>().ok());
        if let Some(db) = db {
            if !db.object_store_names().contains(DB_STORE) {
                let _ = db.create_object_store(DB_STORE);
            }
        }
    });
    req.set_onupgradeneeded(Some(up.unchecked_ref()));
    let db = idb_wait(&req).await?;
    db.dyn_into::<IdbDatabase>()
}

async fn idb_load_all(db: &IdbDatabase) -> Result<HashMap<String, Vec<u8>>, JsValue> {
    let tx = db.transaction_with_str(DB_STORE)?;
    let st = tx.object_store(DB_STORE)?;
    // beide Anfragen vor dem ersten Warten stellen (sonst schließt die Transaktion)
    let kr = st.get_all_keys()?;
    let vr = st.get_all()?;
    let keys: js_sys::Array = idb_wait(&kr).await?.dyn_into()?;
    let vals: js_sys::Array = idb_wait(&vr).await?.dyn_into()?;
    let mut out = HashMap::new();
    for (k, v) in keys.iter().zip(vals.iter()) {
        if let (Some(k), Ok(v)) = (k.as_string(), v.dyn_into::<js_sys::Uint8Array>()) {
            out.insert(k, v.to_vec());
        }
    }
    Ok(out)
}

fn idb_put(db: &IdbDatabase, key: &str, data: &[u8]) -> Result<(), JsValue> {
    let tx = db.transaction_with_str_and_mode(DB_STORE, IdbTransactionMode::Readwrite)?;
    let req = tx.object_store(DB_STORE)?.put_with_key(&js_sys::Uint8Array::from(data), &JsValue::from_str(key))?;
    let k = key.to_string();
    let err = Closure::once_into_js(move |_e: web_sys::Event| warn(&format!("OwnDraw: „{k}“ konnte nicht im Browser gesichert werden (Speicher voll?)")));
    req.set_onerror(Some(err.unchecked_ref()));
    Ok(())
}

fn idb_delete(db: &IdbDatabase, key: &str) {
    let r = db.transaction_with_str_and_mode(DB_STORE, IdbTransactionMode::Readwrite).and_then(|tx| tx.object_store(DB_STORE)).and_then(|st| st.delete(&JsValue::from_str(key)));
    if r.is_err() {
        warn("OwnDraw: Eintrag in IndexedDB nicht löschbar");
    }
}

/// Beim Start geladener Browser-Speicher (Boards aus IndexedDB).
pub struct Store {
    db: Option<IdbDatabase>,
    files: HashMap<String, Vec<u8>>,
}

impl Store {
    /// Öffnet IndexedDB und lädt alle Boards. Ohne IndexedDB (z. B. privates Fenster): nur im Speicher.
    /// Boards, die nicht mehr in der Zuletzt-Liste stehen, werden entfernt (sie wären nicht mehr erreichbar).
    pub async fn load() -> Store {
        let db = match idb_open().await {
            Ok(db) => Some(db),
            Err(e) => {
                warn(&format!("OwnDraw: IndexedDB nicht verfügbar, Boards nur im Speicher ({e:?})"));
                None
            }
        };
        let mut files = match &db {
            Some(db) => idb_load_all(db).await.unwrap_or_else(|e| {
                warn(&format!("OwnDraw: Boards aus IndexedDB nicht lesbar ({e:?})"));
                HashMap::new()
            }),
            None => HashMap::new(),
        };
        if let Ok(b) = crate::kv::read(&PathBuf::from(CONFIG).join("recent.json")) {
            if let Ok(serde_json::Value::Array(a)) = serde_json::from_slice::<serde_json::Value>(&b) {
                let keep: HashSet<String> = a.iter().filter_map(|e| e.get("file")?.as_str()?.strip_prefix("uri:").map(String::from)).collect();
                let gone: Vec<String> = files.keys().filter(|k| !keep.contains(*k)).cloned().collect();
                for k in gone {
                    files.remove(&k);
                    if let Some(db) = &db {
                        idb_delete(db, &k);
                    }
                }
            }
        }
        Store { db, files }
    }
}

// ---------------------------------------------------------------- Plattform

#[derive(Default)]
struct Inner {
    files: HashMap<String, Vec<u8>>,
    events: Vec<PlatformEvent>,
    drops: Vec<Drop>,
    clip: Option<ClipData>,
    /// Text, den wir zuletzt in die System-Zwischenablage gelegt haben (zum Wiedererkennen beim Einfügen).
    clip_text: Option<String>,
    /// Text aus dem `paste`-Ereignis dieses Frames.
    paste: Option<String>,
    last_pointer: Option<(f32, f32)>,
    /// Letzte Position beim Ziehen von Dateien über die Seite (CSS-Pixel), für den Ablageort.
    drag_css: Option<(f32, f32)>,
}

pub struct WebPlatform {
    ctx: egui::Context,
    db: Option<IdbDatabase>,
    inner: Rc<RefCell<Inner>>,
}

impl WebPlatform {
    pub fn new(ctx: egui::Context, store: Store) -> WebPlatform {
        let inner = Rc::new(RefCell::new(Inner { files: store.files, ..Inner::default() }));
        let p = WebPlatform { ctx, db: store.db, inner };
        p.install_image_paste();
        p.install_drag_pos();
        p
    }

    /// Merkt die Mausposition beim Ziehen (egui bekommt währenddessen keine Zeigerbewegung).
    fn install_drag_pos(&self) {
        let Some(w) = web_sys::window() else { return };
        let inner = self.inner.clone();
        let cb = Closure::<dyn FnMut(web_sys::MouseEvent)>::new(move |e: web_sys::MouseEvent| {
            inner.borrow_mut().drag_css = Some((e.client_x() as f32, e.client_y() as f32));
        });
        for ev in ["dragover", "drop"] {
            let _ = w.add_event_listener_with_callback_and_bool(ev, cb.as_ref().unchecked_ref(), true);
        }
        cb.forget();
    }

    /// Eindeutiger `mem:`-Name (wie „Board (2).owndraw“).
    fn unique(files: &HashMap<String, Vec<u8>>, prefix: &str, name: &str) -> String {
        let name = name.replace('/', "_");
        let (stem, ext) = match name.rsplit_once('.') {
            Some((s, e)) if !s.is_empty() => (s.to_string(), format!(".{e}")),
            _ => (name.to_string(), String::new()),
        };
        let mut k = format!("{prefix}{name}");
        let mut n = 2;
        while files.contains_key(&k) {
            k = format!("{prefix}{stem} ({n}){ext}");
            n += 1;
        }
        k
    }

    /// Legt hochgeladene/abgelegte Bytes ab: Boards (`.owndraw`) dauerhaft unter `mem:<name>`, alles andere
    /// nur im Speicher unter `mem:upload/<name>`. Liefert den Verweis.
    fn store_upload(inner: &Rc<RefCell<Inner>>, db: &Option<IdbDatabase>, name: &str, bytes: Vec<u8>) -> FileRef {
        let mut g = inner.borrow_mut();
        let board = FileKind::of_name(name) == FileKind::Owndraw;
        let key = Self::unique(&g.files, if board { MEM } else { UPLOAD }, name);
        if board {
            if let Some(db) = db {
                let _ = idb_put(db, &key, &bytes);
            }
        }
        g.files.insert(key.clone(), bytes);
        FileRef::Uri(key)
    }

    /// Bilder aus der Zwischenablage (Strg+V mit Bild, ohne Text) kommen als Drop an.
    fn install_image_paste(&self) {
        let Some(doc) = document() else { return };
        let inner = self.inner.clone();
        let ctx = self.ctx.clone();
        let cb = Closure::<dyn FnMut(web_sys::ClipboardEvent)>::new(move |e: web_sys::ClipboardEvent| {
            let Some(dt) = e.clipboard_data() else { return };
            if !dt.get_data("text").unwrap_or_default().is_empty() {
                return;
            }
            let items = dt.items();
            for i in 0..items.length() {
                let Some(it) = items.get(i) else { continue };
                if it.kind() != "file" || !it.type_().starts_with("image/") {
                    continue;
                }
                let Ok(Some(file)) = it.get_as_file() else { continue };
                let (inner, ctx) = (inner.clone(), ctx.clone());
                spawn_local(async move {
                    let Ok(buf) = JsFuture::from(file.array_buffer()).await else { return };
                    let bytes = js_sys::Uint8Array::new(&buf).to_vec();
                    let mut g = inner.borrow_mut();
                    let pos = g.last_pointer;
                    let name = if file.name().is_empty() { "Bild.png".to_string() } else { file.name() };
                    g.drops.push(Drop { file: None, bytes: Some(bytes), name, pos });
                    drop(g);
                    ctx.request_repaint();
                });
                e.prevent_default();
                break;
            }
        });
        let _ = doc.add_event_listener_with_callback("paste", cb.as_ref().unchecked_ref());
        cb.forget();
    }

    fn picked(&self, tag: Tag, file: Option<FileRef>) {
        self.inner.borrow_mut().events.push(PlatformEvent::Picked { tag, file });
        self.ctx.request_repaint();
    }

    /// Löst einen Download aus (Blob + `<a download>`).
    fn download(name: &str, data: &[u8]) -> io::Result<()> {
        let mime = match FileKind::of_name(name) {
            FileKind::Image => match name.rsplit('.').next().unwrap_or("").to_ascii_lowercase().as_str() {
                "jpg" | "jpeg" => "image/jpeg",
                "webp" => "image/webp",
                "gif" => "image/gif",
                _ => "image/png",
            },
            FileKind::Any => "application/octet-stream",
            k => k.mime(),
        };
        let doc = document().ok_or_else(|| io::Error::other("kein document"))?;
        let parts = js_sys::Array::of1(&js_sys::Uint8Array::from(data));
        let opts = web_sys::BlobPropertyBag::new();
        opts.set_type(mime);
        let blob = web_sys::Blob::new_with_u8_array_sequence_and_options(&parts, &opts).map_err(js_err)?;
        let url = web_sys::Url::create_object_url_with_blob(&blob).map_err(js_err)?;
        let a: web_sys::HtmlAnchorElement = doc.create_element("a").map_err(js_err)?.dyn_into().map_err(|_| io::Error::other("kein <a>"))?;
        a.set_href(&url);
        a.set_download(name);
        let _ = a.style().set_property("display", "none");
        if let Some(b) = doc.body() {
            let _ = b.append_child(&a);
        }
        a.click();
        a.remove();
        // Adresse später freigeben (der Download braucht sie noch einen Moment)
        if let Some(w) = web_sys::window() {
            let u = url.clone();
            let f = Closure::once_into_js(move || {
                let _ = web_sys::Url::revoke_object_url(&u);
            });
            let _ = w.set_timeout_with_callback_and_timeout_and_arguments_0(f.unchecked_ref(), 60_000);
        }
        Ok(())
    }
}

fn accept(kinds: &[FileKind]) -> String {
    let mut v: Vec<String> = Vec::new();
    for k in kinds {
        match k {
            FileKind::Any => return String::new(),
            FileKind::Image => v.push("image/*".into()),
            _ => {}
        }
        v.extend(k.extensions().iter().map(|e| format!(".{e}")));
    }
    v.join(",")
}

impl Platform for WebPlatform {
    fn pick_open(&self, kinds: &[FileKind], tag: Tag) {
        let Some(doc) = document() else { return self.picked(tag, None) };
        let Ok(input) = doc.create_element("input").map(|e| e.unchecked_into::<web_sys::HtmlInputElement>()) else { return self.picked(tag, None) };
        input.set_type("file");
        input.set_accept(&accept(kinds));
        let _ = input.style().set_property("display", "none");
        if let Some(b) = doc.body() {
            let _ = b.append_child(&input);
        }
        let (inner, ctx, db, inp) = (self.inner.clone(), self.ctx.clone(), self.db.clone(), input.clone());
        let done = Rc::new(std::cell::Cell::new(false));
        let d1 = done.clone();
        let on_change = Closure::once_into_js(move || {
            d1.set(true);
            let file = inp.files().and_then(|l| l.get(0));
            inp.remove();
            spawn_local(async move {
                let picked = match file {
                    Some(f) => match JsFuture::from(f.array_buffer()).await {
                        Ok(buf) => Some(WebPlatform::store_upload(&inner, &db, &f.name(), js_sys::Uint8Array::new(&buf).to_vec())),
                        Err(e) => {
                            warn(&format!("OwnDraw: Datei nicht lesbar ({e:?})"));
                            None
                        }
                    },
                    None => None,
                };
                inner.borrow_mut().events.push(PlatformEvent::Picked { tag, file: picked });
                ctx.request_repaint();
            });
        });
        let _ = input.add_event_listener_with_callback("change", on_change.unchecked_ref());
        let (inner, ctx, inp) = (self.inner.clone(), self.ctx.clone(), input.clone());
        let on_cancel = Closure::once_into_js(move || {
            if done.get() {
                return;
            }
            inp.remove();
            inner.borrow_mut().events.push(PlatformEvent::Picked { tag, file: None });
            ctx.request_repaint();
        });
        let _ = input.add_event_listener_with_callback("cancel", on_cancel.unchecked_ref());
        input.click();
    }
    fn pick_save(&self, name: &str, _kind: FileKind, tag: Tag) {
        // Kein Speicherdialog: das Ziel ist ein Download mit dem vorgeschlagenen Namen.
        self.picked(tag, Some(FileRef::Uri(format!("{DOWNLOAD}{}", name.replace('/', "_")))));
    }
    fn pick_folder(&self, tag: Tag) {
        self.picked(tag, None);
    }
    fn poll(&self) -> Vec<PlatformEvent> {
        std::mem::take(&mut self.inner.borrow_mut().events)
    }

    fn read(&self, f: &FileRef) -> io::Result<Vec<u8>> {
        match f {
            FileRef::Uri(u) => self.inner.borrow().files.get(u).cloned().ok_or_else(|| io::Error::new(io::ErrorKind::NotFound, "nicht im Browser-Speicher")),
            FileRef::Path(p) => crate::kv::read(p),
        }
    }
    fn write_atomic(&self, f: &FileRef, data: &[u8]) -> io::Result<()> {
        match f {
            FileRef::Uri(u) if u.starts_with(DOWNLOAD) => Self::download(&f.display_name(), data),
            FileRef::Uri(u) if u.starts_with(MEM) => {
                if !u.starts_with(UPLOAD) {
                    if let Some(db) = &self.db {
                        idb_put(db, u, data).map_err(js_err)?;
                    }
                }
                self.inner.borrow_mut().files.insert(u.clone(), data.to_vec());
                Ok(())
            }
            FileRef::Uri(_) => Err(io::Error::new(io::ErrorKind::Unsupported, "Adresse im Browser nicht unterstützt")),
            FileRef::Path(p) => crate::kv::write(p, data),
        }
    }
    fn create_in(&self, _dir: &FileRef, name: &str, _kind: FileKind) -> io::Result<FileRef> {
        Ok(FileRef::Uri(Self::unique(&self.inner.borrow().files, MEM, name)))
    }
    fn exists(&self, f: &FileRef) -> bool {
        match f {
            FileRef::Uri(u) => self.inner.borrow().files.contains_key(u),
            FileRef::Path(p) => crate::kv::exists(p),
        }
    }
    fn default_dir(&self) -> FileRef {
        FileRef::Uri(MEM.into())
    }
    fn config_dir(&self) -> PathBuf {
        PathBuf::from(CONFIG)
    }
    fn cache_dir(&self) -> PathBuf {
        PathBuf::from(CACHE)
    }

    fn clipboard_get(&self) -> Option<ClipData> {
        let mut g = self.inner.borrow_mut();
        match g.paste.take() {
            // eigener Inhalt kam zurück: interne Daten (mit Bildern) nehmen
            Some(t) if Some(&t) == g.clip_text.as_ref() => g.clip.clone(),
            Some(t) => Some(ClipData::Text(t)),
            None => g.clip.clone(),
        }
    }
    fn clipboard_set(&self, d: ClipData) {
        let text = match &d {
            ClipData::Items { json, .. } => Some(json.clone()),
            ClipData::Text(t) => Some(t.clone()),
            ClipData::Image { w, h, rgba } => {
                self.ctx.copy_image(egui::ColorImage::from_rgba_unmultiplied([*w as usize, *h as usize], rgba));
                None
            }
        };
        if let Some(t) = &text {
            self.ctx.copy_text(t.clone());
        }
        let mut g = self.inner.borrow_mut();
        g.clip_text = text;
        g.clip = Some(d);
    }
    fn take_drops(&self, ctx: &egui::Context) -> Vec<Drop> {
        let (files, pos, paste) = ctx.input(|i| {
            let paste = i.events.iter().rev().find_map(|e| match e {
                egui::Event::Paste(t) => Some(t.clone()),
                _ => None,
            });
            (i.raw.dropped_files.clone(), i.pointer.latest_pos(), paste)
        });
        let mut g = self.inner.borrow_mut();
        g.paste = paste;
        if let Some(p) = pos {
            g.last_pointer = Some((p.x, p.y));
        }
        let zoom = ctx.zoom_factor().max(0.1);
        let pos = match g.drag_css.take() {
            Some((x, y)) if !files.is_empty() => Some((x / zoom, y / zoom)),
            _ => g.last_pointer,
        };
        drop(g);
        for f in files {
            let (inner, ctx, db) = (self.inner.clone(), ctx.clone(), self.db.clone());
            spawn_local(async move {
                let name = f.path().file_name().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default();
                match f.bytes_async().await {
                    Ok(bytes) => {
                        let kind = FileKind::of_name(&name);
                        let file = matches!(kind, FileKind::Owndraw | FileKind::Xopp).then(|| WebPlatform::store_upload(&inner, &db, &name, bytes.clone()));
                        inner.borrow_mut().drops.push(Drop { file, bytes: Some(bytes), name, pos });
                    }
                    Err(e) => warn(&format!("OwnDraw: abgelegte Datei nicht lesbar: {e}")),
                }
                ctx.request_repaint();
            });
        }
        std::mem::take(&mut self.inner.borrow_mut().drops)
    }
    fn drag_hover(&self, ctx: &egui::Context) -> bool {
        ctx.input(|i| !i.raw.hovered_files.is_empty())
    }
    fn open_url(&self, url: &str) {
        if let Some(w) = web_sys::window() {
            let _ = w.open_with_url_and_target(url, "_blank");
        }
    }
    fn is_touch(&self) -> bool {
        web_sys::window().and_then(|w| w.match_media("(pointer: coarse)").ok().flatten()).map(|m| m.matches()).unwrap_or(false)
    }
}
