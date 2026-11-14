//! Installation eines AppImage: neue Datei neben die laufende schreiben, atomar umbenennen, alte als
//! `.bak` und in `versions/` behalten. Bei jedem Fehler bleibt die alte Datei unangetastet.

use super::versions::{prune, saved_name, KEEP_VERSIONS};
use super::Error;
use semver::Version;
use std::path::{Path, PathBuf};

/// Pfad der laufenden AppImage-Datei (`$APPIMAGE`), falls vorhanden.
pub fn running_appimage() -> Option<PathBuf> {
    let p = PathBuf::from(std::env::var_os("APPIMAGE").filter(|s| !s.is_empty())?);
    p.is_file().then_some(p)
}

fn sibling(target: &Path, suffix: &str) -> PathBuf {
    let mut n = target.file_name().map(|s| s.to_os_string()).unwrap_or_default();
    n.push(suffix);
    target.with_file_name(n)
}

/// Prüft vorab, ob neben `target` geschrieben werden darf (verständlicher Hinweis statt Fehler mittendrin).
pub fn check_writable(target: &Path) -> Result<(), Error> {
    let dir = target.parent().filter(|p| !p.as_os_str().is_empty()).unwrap_or(Path::new("."));
    let probe = dir.join(format!(".owndraw-write-test-{}", std::process::id()));
    match std::fs::File::create(&probe) {
        Ok(_) => {
            let _ = std::fs::remove_file(&probe);
            Ok(())
        }
        Err(e) => Err(Error::NoAppImage(format!(
            "Keine Schreibrechte im Ordner {} ({e}). Verschieben Sie das AppImage in einen beschreibbaren Ordner oder installieren Sie das Update von Hand.",
            dir.display()
        ))),
    }
}

/// Zielpfad prüfen: `$APPIMAGE` fehlt (z. B. `cargo run`) → verständlicher Hinweis.
pub fn require_target(target: Option<&Path>) -> Result<PathBuf, Error> {
    let t = target.ok_or_else(|| {
        Error::NoAppImage("OwnDraw läuft nicht als AppImage (z. B. direkt aus cargo). Automatisches Aktualisieren ist nur für das AppImage möglich.".into())
    })?;
    check_writable(t)?;
    Ok(t.to_path_buf())
}

/// Ergebnis einer Installation.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Installed {
    pub version: Version,
    pub path: PathBuf,
    pub backup: PathBuf,
    /// Neustart nötig/angeboten (AppImage), sonst false (Android: System-Installer übernimmt).
    pub restart: bool,
}

/// Ersetzt `target` durch `new_file`. `old_version` ist die bisher laufende Version (für `versions/`).
pub fn install_appimage(new_file: &Path, new_version: &Version, target: &Path, old_version: &Version, versions_dir: &Path) -> Result<Installed, Error> {
    check_writable(target)?;
    let staged = sibling(target, ".new");
    let bak = sibling(target, ".bak");
    let r = (|| -> Result<(), Error> {
        // 1. Neue Datei daneben schreiben (gleiches Dateisystem → atomares Umbenennen), ausführbar machen.
        let _ = std::fs::remove_file(&staged);
        std::fs::copy(new_file, &staged)?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&staged, std::fs::Permissions::from_mode(0o755))?;
        }
        std::fs::File::open(&staged)?.sync_all()?;
        // 2. Alte Version in versions/ sichern (nur, wenn sie existiert).
        if target.is_file() {
            std::fs::create_dir_all(versions_dir)?;
            let keep = versions_dir.join(saved_name(old_version));
            let tmp = versions_dir.join(format!(".{}.tmp", saved_name(old_version)));
            std::fs::copy(target, &tmp)?;
            std::fs::rename(&tmp, &keep)?;
            prune(versions_dir, KEEP_VERSIONS);
            // 3. .bak: Hardlink (kein Zeitfenster ohne Datei), sonst Kopie.
            let _ = std::fs::remove_file(&bak);
            if std::fs::hard_link(target, &bak).is_err() {
                std::fs::copy(target, &bak)?;
            }
        }
        // 4. Atomar ersetzen.
        std::fs::rename(&staged, target)?;
        Ok(())
    })();
    if let Err(e) = r {
        let _ = std::fs::remove_file(&staged);
        return Err(e);
    }
    Ok(Installed { version: new_version.clone(), path: target.to_path_buf(), backup: bak, restart: true })
}

/// Startet die (neu installierte) Datei und lässt die laufende App danach schließen.
pub fn spawn_new(path: &Path) -> std::io::Result<()> {
    std::process::Command::new(path).spawn().map(|_| ())
}
