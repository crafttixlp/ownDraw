//! OwnDraw-App (egui/eframe) als Bibliothek: Desktop-Start über `run_desktop`, Android und Browser (`run_web`)
//! über eigene Crates (`owndraw-android`, `owndraw-web`).

mod app;
mod autosave;
mod demo;
/// Handschrift-Erkennung: Anbindung an die Browser-Seite (hw.js).
mod hw;
mod kv;
#[allow(dead_code)]
mod layout;
/// Server-Anbindung (Konto, Server-Boards, Live-Zusammenarbeit): Web-App und Desktop, nicht Android.
#[cfg(not(target_os = "android"))]
mod net;
#[allow(dead_code)]
mod platform;
mod recent;
mod session;
#[allow(dead_code)]
mod settings;
/// CPU-Rasterizer für den Browser ohne WebGL (überall übersetzt, damit `cargo test` ihn prüft).
#[cfg_attr(not(target_arch = "wasm32"), allow(dead_code))]
mod softraster;
mod style;
mod ui;
/// Update-Dienst (GitHub-Releases, Signaturprüfung); im Browser aus.
#[cfg(not(target_arch = "wasm32"))]
mod update;

/// JNI-Einsprungpunkte für das Crate `owndraw-android` (Java → Rust).
#[cfg(target_os = "android")]
pub use platform::android::bridge as android_bridge;

use std::path::PathBuf;

/// Datenordner: $XDG_DATA_HOME/OwnDraw bzw. ~/.local/share/OwnDraw (wie die Java-Version).
pub fn data_dir() -> PathBuf {
    if let Ok(x) = std::env::var("OWNDRAW_DATA") {
        if !x.is_empty() {
            return PathBuf::from(x);
        }
    }
    if let Ok(x) = std::env::var("XDG_DATA_HOME") {
        if !x.is_empty() {
            return PathBuf::from(x).join("OwnDraw");
        }
    }
    let home = std::env::var("HOME").map(PathBuf::from).unwrap_or_else(|_| PathBuf::from("."));
    home.join(".local/share/OwnDraw")
}

#[cfg_attr(any(target_os = "android", target_arch = "wasm32"), allow(dead_code))]
fn icon() -> Option<egui::IconData> {
    let pm = owndraw_core::tiny_skia::Pixmap::decode_png(include_bytes!("../../../assets/owndraw.png")).ok()?;
    let (w, h) = (pm.width(), pm.height());
    let mut rgba = Vec::with_capacity((w * h * 4) as usize);
    for p in pm.pixels() {
        let c = p.demultiply();
        rgba.extend_from_slice(&[c.red(), c.green(), c.blue(), c.alpha()]);
    }
    Some(egui::IconData { rgba, width: w, height: h })
}

/// Fenstergröße aus `OWNDRAW_WINDOW=BxH` (für Tests und Screenshots), sonst 1280 × 820.
#[cfg_attr(any(target_os = "android", target_arch = "wasm32"), allow(dead_code))]
fn window_size() -> [f32; 2] {
    std::env::var("OWNDRAW_WINDOW")
        .ok()
        .and_then(|s| {
            let (w, h) = s.split_once(['x', 'X'])?;
            Some([w.trim().parse().ok()?, h.trim().parse().ok()?])
        })
        .unwrap_or([1280.0, 820.0])
}

#[cfg(not(any(target_os = "android", target_arch = "wasm32")))]
pub fn run_desktop() -> eframe::Result<()> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    if args.iter().any(|a| a == "--version" || a == "-V") {
        println!("OwnDraw {}", env!("CARGO_PKG_VERSION"));
        return Ok(());
    }
    let open_file = args.iter().find(|a| !a.starts_with('-')).map(|p| platform::FileRef::Path(PathBuf::from(p)));
    let size = window_size();
    let mut vp = egui::ViewportBuilder::default()
        .with_title("OwnDraw")
        .with_app_id("OwnDraw")
        .with_inner_size(size)
        .with_min_inner_size([320.0, 420.0])
        .with_drag_and_drop(true);
    if let Some(i) = icon() {
        vp = vp.with_icon(i);
    }
    let opts = eframe::NativeOptions { viewport: vp, centered: true, ..Default::default() };
    eframe::run_native(
        "OwnDraw",
        opts,
        Box::new(move |cc| {
            let platform = Box::new(platform::desktop::DesktopPlatform::new(None));
            let settings = app::load_settings(&*platform);
            style::apply(&cc.egui_ctx, &settings);
            // Einmalige Migration der alten Ablage (index.json + Board-Ordner) in den Standard-Ort.
            let target = settings.default_dir.as_deref().and_then(platform::FileRef::from_key).unwrap_or_else(|| platform::Platform::default_dir(&*platform));
            let migrated = match target {
                platform::FileRef::Path(dir) => owndraw_core::migrate::migrate_store(&data_dir(), &dir),
                platform::FileRef::Uri(_) => Vec::new(),
            };
            // Server-Anbindung (Konto, Server-Boards, Live): Adresse aus den Einstellungen bzw. `OWNDRAW_SERVER`.
            // Screenshot-Demos ohne `OWNDRAW_SERVER` bleiben ohne Netz.
            let server = std::env::var("OWNDRAW_SERVER").ok().filter(|s| !s.trim().is_empty());
            net::set_config_dir(platform::Platform::config_dir(&*platform));
            net::set_base_url(server.as_deref().unwrap_or(&settings.server_url));
            if server.is_some() || std::env::var_os("OWNDRAW_DEMO").is_none() {
                net::init(&cc.egui_ctx);
            }
            let mut app = app::App::new(platform, settings, open_file);
            app.add_migrated(migrated);
            Ok(Box::new(app))
        }),
    )
}

/// Android-Start (aus `android_main` im Crate `owndraw-android`): gleiche App wie `run_desktop`,
/// Datenordner = App-interner Speicher, Plattform = [`platform::android::AndroidPlatform`].
#[cfg(target_os = "android")]
pub fn run_android(app: android_activity::AndroidApp) -> eframe::Result<()> {
    let data = app.internal_data_path().unwrap_or_else(|| PathBuf::from("/data/local/tmp/owndraw"));
    let _ = std::fs::create_dir_all(&data);
    let platform = platform::android::AndroidPlatform::new(data);
    let opts = eframe::NativeOptions { android_app: Some(app), ..Default::default() };
    eframe::run_native(
        "OwnDraw",
        opts,
        Box::new(move |cc| {
            platform::android::bridge::set_context(&cc.egui_ctx);
            let platform: Box<dyn platform::Platform> = Box::new(platform);
            let settings = app::load_settings(&*platform);
            style::apply(&cc.egui_ctx, &settings);
            Ok(Box::new(app::App::new(platform, settings, None)))
        }),
    )
}

/// Browser-Start (aus `start` im Crate `owndraw-web`): gleiche App, Plattform = [`platform::web::WebPlatform`]
/// (Gastmodus: Öffnen per Upload, Speichern als Download, Boards in IndexedDB; mit Server: Konto, Boards am
/// Server, Live-Zusammenarbeit, siehe `net/`).
///
/// Darstellung: mit WebGL über eframe (wie bisher); ohne WebGL (Hardwarebeschleunigung aus) per CPU in ein
/// 2D-Canvas ([`platform::web::soft`]). URL-Parameter `?renderer=software` bzw. `?renderer=webgl` erzwingt einen Weg.
#[cfg(target_arch = "wasm32")]
pub async fn run_web(canvas: web_sys::HtmlCanvasElement) -> Result<(), wasm_bindgen::JsValue> {
    use std::cell::RefCell;
    use std::rc::Rc;
    let store = platform::web::Store::load().await;
    platform::web::install_page_hooks();
    let forced = web_renderer_param();
    let use_gl = match forced.as_deref() {
        Some("software") => false,
        Some("webgl") => true,
        _ => webgl_available(),
    };
    // Die App braucht den Store; schlägt eframe fehl, bevor sie gebaut ist, bekommt ihn der Software-Weg.
    let store = Rc::new(RefCell::new(Some(store)));
    if use_gl {
        let st = store.clone();
        let runner = eframe::WebRunner::new();
        let r = runner
            .start(
                canvas.clone(),
                eframe::WebOptions::default(),
                Box::new(move |cc| {
                    let store = st.borrow_mut().take().ok_or("Browser-Speicher fehlt")?;
                    Ok(Box::new(make_web_app(&cc.egui_ctx, store)))
                }),
            )
            .await;
        match r {
            Ok(()) => {
                set_web_renderer("webgl");
                return Ok(());
            }
            Err(e) if forced.as_deref() == Some("webgl") || store.borrow().is_none() => return Err(e),
            Err(e) => web_sys::console::warn_1(&format!("OwnDraw: WebGL-Start fehlgeschlagen ({}), Software-Darstellung", e.as_string().unwrap_or_default()).into()),
        }
    }
    let store = store.borrow_mut().take().ok_or("Browser-Speicher fehlt")?;
    platform::web::soft::start(canvas, move |ctx| make_web_app(ctx, store)).await?;
    set_web_renderer("software");
    Ok(())
}

/// App für den Browser bauen (beide Darstellungswege).
#[cfg(target_arch = "wasm32")]
fn make_web_app(ctx: &egui::Context, store: platform::web::Store) -> app::App {
    let platform: Box<dyn platform::Platform> = Box::new(platform::web::WebPlatform::new(ctx.clone(), store));
    // Server-Anbindung: Anmeldung prüfen (ohne Server bleibt alles im Gastmodus)
    platform::web::api::init(ctx);
    let settings = app::load_settings(&*platform);
    style::apply(ctx, &settings);
    app::App::new(platform, settings, None)
}

/// `renderer` aus der Adresse (`?renderer=software|webgl`).
#[cfg(target_arch = "wasm32")]
fn web_renderer_param() -> Option<String> {
    let search = web_sys::window()?.location().search().ok()?;
    search.trim_start_matches('?').split('&').find_map(|kv| kv.strip_prefix("renderer=")).map(|v| v.to_ascii_lowercase())
}

/// Liefert ein Test-Canvas einen WebGL2- oder WebGL1-Kontext?
#[cfg(target_arch = "wasm32")]
fn webgl_available() -> bool {
    use wasm_bindgen::JsCast;
    let Some(doc) = web_sys::window().and_then(|w| w.document()) else { return false };
    let Some(c) = doc.create_element("canvas").ok().and_then(|e| e.dyn_into::<web_sys::HtmlCanvasElement>().ok()) else { return false };
    ["webgl2", "webgl", "experimental-webgl"].iter().any(|k| c.get_context(k).ok().flatten().is_some())
}

/// Gewählte Darstellung für Seite und Tests: `window.owndrawRenderer`, `<html data-renderer>`.
#[cfg(target_arch = "wasm32")]
fn set_web_renderer(r: &str) {
    let Some(w) = web_sys::window() else { return };
    let _ = js_sys::Reflect::set(&w, &"owndrawRenderer".into(), &r.into());
    if let Some(el) = w.document().and_then(|d| d.document_element()) {
        let _ = el.set_attribute("data-renderer", r);
    }
}
