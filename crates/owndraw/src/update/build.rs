//! Quellbau: Tarball entpacken, `cargo build --release --locked`, Paket erzeugen. Abbruch tötet den Prozess.

use super::Error;
use std::io::{BufRead, BufReader};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc;
use std::time::Duration;

/// Was gebaut wird und wie das Ergebnis entsteht (Tests nutzen ein Mini-Projekt statt OwnDraw).
#[derive(Clone, Debug)]
pub struct BuildSpec {
    /// `-p <package>`
    pub package: String,
    /// Binärname unter `target/release/`
    pub bin: String,
    /// Packen mit `build-appimage.sh` des Quellbaums (sonst wird die Binärdatei direkt installiert).
    pub appimage_script: bool,
}

impl BuildSpec {
    pub fn owndraw() -> BuildSpec {
        BuildSpec {
            package: "owndraw".into(),
            bin: "owndraw".into(),
            appimage_script: true,
        }
    }
}

/// Entpackt `archive` (.tar.gz) nach `dest` (vorher geleert) und liefert die Wurzel des Quellbaums.
pub fn extract(archive: &Path, dest: &Path) -> Result<PathBuf, Error> {
    let _ = std::fs::remove_dir_all(dest);
    std::fs::create_dir_all(dest)?;
    let f = std::fs::File::open(archive)?;
    let mut ar = tar::Archive::new(flate2::read::GzDecoder::new(f));
    // `unpack` verwirft Pfade mit `..` und absolute Pfade.
    ar.unpack(dest)
        .map_err(|e| Error::Other(format!("Archiv nicht entpackbar: {e}")))?;
    if dest.join("Cargo.toml").is_file() {
        return Ok(dest.to_path_buf());
    }
    let dirs: Vec<PathBuf> = std::fs::read_dir(dest)?
        .flatten()
        .map(|e| e.path())
        .filter(|p| p.is_dir())
        .collect();
    match dirs.as_slice() {
        [d] if d.join("Cargo.toml").is_file() => Ok(d.clone()),
        _ => Err(Error::Other("Im Quell-Archiv fehlt die Cargo.toml.".into())),
    }
}

/// Findet `cargo` (PATH, sonst `~/.cargo/bin`). `None` = keine Toolchain.
pub fn find_cargo() -> Option<PathBuf> {
    let ok = |p: &Path| {
        Command::new(p)
            .arg("--version")
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status()
            .map(|s| s.success())
            .unwrap_or(false)
    };
    let plain = PathBuf::from("cargo");
    if ok(&plain) {
        return Some(plain);
    }
    let home = std::env::var_os("HOME").map(PathBuf::from)?;
    let p = home.join(".cargo/bin/cargo");
    ok(&p).then_some(p)
}

/// Startet `cmd`, reicht jede Ausgabezeile an `log` weiter und beendet den Prozess bei `cancel`.
fn run_logged(mut cmd: Command, cancel: &AtomicBool, log: &dyn Fn(&str)) -> Result<(), Error> {
    cmd.stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .stdin(Stdio::null());
    // Eigene Prozessgruppe, damit beim Abbruch auch rustc & Co. enden.
    #[cfg(unix)]
    {
        use std::os::unix::process::CommandExt;
        cmd.process_group(0);
    }
    let mut child = cmd
        .spawn()
        .map_err(|e| Error::Other(format!("Prozess nicht startbar: {e}")))?;
    let (tx, rx) = mpsc::channel::<String>();
    let mut readers = Vec::new();
    if let Some(o) = child.stdout.take() {
        let tx = tx.clone();
        readers.push(std::thread::spawn(move || {
            BufReader::new(o)
                .lines()
                .map_while(Result::ok)
                .for_each(|l| drop(tx.send(l)))
        }));
    }
    if let Some(e) = child.stderr.take() {
        let tx = tx.clone();
        readers.push(std::thread::spawn(move || {
            BufReader::new(e)
                .lines()
                .map_while(Result::ok)
                .for_each(|l| drop(tx.send(l)))
        }));
    }
    drop(tx);
    let status = loop {
        while let Ok(l) = rx.try_recv() {
            log(&l);
        }
        if cancel.load(Ordering::Relaxed) {
            #[cfg(unix)]
            let _ = Command::new("kill")
                .args(["-KILL", "--", &format!("-{}", child.id())])
                .status();
            let _ = child.kill();
            let _ = child.wait();
            // Kindprozesse (rustc) enden mit dem Schließen der Pipes; Leser nicht abwarten.
            return Err(Error::Cancelled);
        }
        match child.try_wait() {
            Ok(Some(s)) => break s,
            Ok(None) => std::thread::sleep(Duration::from_millis(40)),
            Err(e) => return Err(Error::Other(e.to_string())),
        }
    };
    for r in readers {
        let _ = r.join();
    }
    while let Ok(l) = rx.try_recv() {
        log(&l);
    }
    if status.success() {
        Ok(())
    } else {
        Err(Error::Other(format!(
            "Der Bau ist fehlgeschlagen ({status}). Details im Protokoll."
        )))
    }
}

/// Baut im Quellbaum `root` und liefert die fertige Datei (AppImage oder Binärdatei).
pub fn build(
    root: &Path,
    spec: &BuildSpec,
    cargo: &Path,
    cancel: &AtomicBool,
    log: &dyn Fn(&str),
) -> Result<PathBuf, Error> {
    log(&format!(
        "$ cargo build --release --locked -p {}",
        spec.package
    ));
    let mut c = Command::new(cargo);
    c.current_dir(root)
        .args(["build", "--release", "--locked", "-p", &spec.package]);
    // Ein von der laufenden Sitzung geerbtes Zielverzeichnis würde fremde Artefakte vermischen.
    c.env_remove("CARGO_TARGET_DIR");
    run_logged(c, cancel, log)?;
    let bin = root.join("target/release").join(&spec.bin);
    if !bin.is_file() {
        return Err(Error::Other(format!(
            "Nach dem Bau fehlt {}.",
            bin.display()
        )));
    }
    if !spec.appimage_script {
        return Ok(bin);
    }
    let script = root.join("build-appimage.sh");
    if !script.is_file() {
        return Err(Error::Other(
            "Im Quellbaum fehlt build-appimage.sh zum Packen des AppImage.".into(),
        ));
    }
    log("$ ./build-appimage.sh");
    let mut c = Command::new("bash");
    c.current_dir(root).arg(&script);
    c.env_remove("CARGO_TARGET_DIR");
    run_logged(c, cancel, log).map_err(|e| match e {
        Error::Other(m) => Error::Other(format!(
            "{m} (Das Packen braucht gcc, zlib1g-dev und squashfs-tools.)"
        )),
        e => e,
    })?;
    let img = std::fs::read_dir(root)?
        .flatten()
        .map(|e| e.path())
        .find(|p| {
            p.file_name()
                .map(|n| n.to_string_lossy().ends_with("-x86_64.AppImage"))
                .unwrap_or(false)
        })
        .ok_or_else(|| Error::Other("build-appimage.sh hat kein AppImage erzeugt.".into()))?;
    Ok(img)
}
