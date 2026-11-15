#[cfg(not(target_arch = "wasm32"))]
fn main() -> eframe::Result<()> {
    owndraw::run_desktop()
}

/// Im Browser startet `owndraw-web` die App (`owndraw::run_web`); das Programm selbst ist dort leer.
#[cfg(target_arch = "wasm32")]
fn main() {}
