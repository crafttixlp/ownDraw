//! OwnDraw für den Linux-Desktop (egui/eframe).

mod app;
mod editor;
mod layers;
mod prefs;
mod style;

use std::path::PathBuf;

/// Datenordner: $XDG_DATA_HOME/OwnDraw bzw. ~/.local/share/OwnDraw (wie die Java-Version).
fn data_dir() -> PathBuf {
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

fn main() -> eframe::Result<()> {
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
