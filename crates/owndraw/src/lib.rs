//! OwnDraw-App (egui/eframe) als Bibliothek: Desktop-Start über `run_desktop`, Android über ein eigenes Crate.

mod app;
mod autosave;
mod demo;
#[allow(dead_code)]
mod layout;
#[allow(dead_code)]
mod platform;
mod recent;
mod session;
#[allow(dead_code)]
mod settings;
mod style;
mod ui;

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

#[cfg_attr(target_os = "android", allow(dead_code))]
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
#[cfg_attr(target_os = "android", allow(dead_code))]
fn window_size() -> [f32; 2] {
    std::env::var("OWNDRAW_WINDOW")
        .ok()
        .and_then(|s| {
            let (w, h) = s.split_once(['x', 'X'])?;
            Some([w.trim().parse().ok()?, h.trim().parse().ok()?])
        })
        .unwrap_or([1280.0, 820.0])
}

#[cfg(not(target_os = "android"))]
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
            Ok(Box::new(app::App::new(platform, settings, open_file)))
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
