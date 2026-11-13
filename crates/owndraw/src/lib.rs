//! OwnDraw-App (egui/eframe) als Bibliothek: Desktop-Start über `run_desktop`, Android über ein eigenes Crate.

mod app;
mod editor;
mod layers;
#[allow(dead_code)]
mod layout;
#[allow(dead_code)]
mod platform;
#[allow(dead_code)]
mod settings;
mod prefs;
mod style;

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

pub fn run_desktop() -> eframe::Result<()> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    if args.iter().any(|a| a == "--version" || a == "-V") {
        println!("OwnDraw {}", env!("CARGO_PKG_VERSION"));
        return Ok(());
    }
    let open_file = args.iter().find(|a| !a.starts_with('-')).map(PathBuf::from);
    let mut vp = egui::ViewportBuilder::default()
        .with_title("OwnDraw")
        .with_app_id("OwnDraw")
        .with_inner_size([1280.0, 820.0])
        .with_min_inner_size([820.0, 560.0])
        .with_drag_and_drop(true);
    if let Some(i) = icon() {
        vp = vp.with_icon(i);
    }
    let opts = eframe::NativeOptions { viewport: vp, centered: true, ..Default::default() };
    eframe::run_native(
        "OwnDraw",
        opts,
        Box::new(move |cc| {
            style::setup_visuals(&cc.egui_ctx);
            Ok(Box::new(app::App::new(data_dir(), open_file)))
        }),
    )
}

/// Android-Start (aus `android_main` im Crate `owndraw-android`): gleiche App wie `run_desktop`,
/// Datenordner = App-interner Speicher, Plattform = [`platform::android::AndroidPlatform`].
#[cfg(target_os = "android")]
pub fn run_android(app: android_activity::AndroidApp) -> eframe::Result<()> {
    let data = app.internal_data_path().unwrap_or_else(|| PathBuf::from("/data/local/tmp/owndraw"));
    let _ = std::fs::create_dir_all(&data);
    let platform = platform::android::AndroidPlatform::new(data.clone());
    let opts = eframe::NativeOptions { android_app: Some(app), ..Default::default() };
    eframe::run_native(
        "OwnDraw",
        opts,
        Box::new(move |cc| {
            platform::android::bridge::set_context(&cc.egui_ctx);
            style::setup_visuals(&cc.egui_ctx);
            // TODO(WP6/WP13): Plattform an die App übergeben (z. B. `App::new(…, Box<dyn Platform>)`)
            // und in jedem Frame `poll()`/`take_drops()` abfragen.
            let _platform: Box<dyn platform::Platform> = Box::new(platform);
            Ok(Box::new(app::App::new(data, None)))
        }),
    )
}
