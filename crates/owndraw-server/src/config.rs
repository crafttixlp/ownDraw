//! Konfiguration aus Umgebungsvariablen.

use std::path::PathBuf;

pub const GITHUB_LATEST: &str = "https://api.github.com/repos/crafttixlp/ownDraw/releases/latest";

#[derive(Clone, Debug)]
pub struct Config {
    /// `OWNDRAW_DATA`: Datenordner (`owndraw.db`, `boards/`, `versions/`, `current`).
    pub data: PathBuf,
    /// `OWNDRAW_WEB`: Ordner der Web-App.
    pub web: PathBuf,
    /// `OWNDRAW_BIND`: Adresse, z. B. `0.0.0.0:8080`.
    pub bind: String,
    /// `OWNDRAW_MAX_BOARD_MB` (Standard 50).
    pub max_board_bytes: u64,
    /// Versuche pro Fenster für Login/Registrierung/Einlösen (je Client-Adresse).
    pub rate_limit: u32,
    pub rate_window_secs: u64,
    /// GitHub-Releases-API (überschreibbar mit `OWNDRAW_RELEASE_URL`, v. a. für Tests).
    pub release_url: String,
}

impl Config {
    pub fn from_env() -> Self {
        let env = |k: &str| std::env::var(k).ok().filter(|v| !v.trim().is_empty());
        let mb: u64 = env("OWNDRAW_MAX_BOARD_MB").and_then(|v| v.trim().parse().ok()).filter(|v| *v > 0).unwrap_or(50);
        Self {
            data: env("OWNDRAW_DATA").unwrap_or_else(|| "./data".into()).into(),
            web: env("OWNDRAW_WEB").unwrap_or_else(|| "./web".into()).into(),
            bind: env("OWNDRAW_BIND").unwrap_or_else(|| "0.0.0.0:8080".into()),
            max_board_bytes: mb * 1024 * 1024,
            rate_limit: 10,
            rate_window_secs: 600,
            release_url: env("OWNDRAW_RELEASE_URL").unwrap_or_else(|| GITHUB_LATEST.into()),
        }
    }

    pub fn boards_dir(&self) -> PathBuf {
        self.data.join("boards")
    }
    pub fn db_path(&self) -> PathBuf {
        self.data.join("owndraw.db")
    }
}
