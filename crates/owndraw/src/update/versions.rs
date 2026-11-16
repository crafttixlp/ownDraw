//! Versionslogik: Tags, Formatversion, gespeicherte Vorversionen.

use semver::Version;
use std::path::{Path, PathBuf};

/// Wie viele Vorversionen in `versions/` bleiben.
pub const KEEP_VERSIONS: usize = 3;

/// `v1.2.3` oder `1.2.3` → Version.
pub fn parse_tag(tag: &str) -> Option<Version> {
    Version::parse(tag.trim().trim_start_matches(['v', 'V'])).ok()
}

/// `.owndraw`-Formatversion, die eine App-Version schreibt (ab 4.0.0: 2).
pub fn format_of(v: &Version) -> i64 {
    if v.major >= 4 {
        2
    } else {
        1
    }
}

/// Warnung vor einem Wechsel auf `target`, falls die Zielversion die Boards der aktuellen App nicht lesen kann.
/// `recent` = Anzahl zuletzt geöffneter Boards (nur für den Text).
pub fn format_warning(target: &Version, recent: usize) -> Option<String> {
    let cur = owndraw_core::doc::FORMAT_VERSION;
    if format_of(target) < cur {
        let boards = if recent > 0 { format!(" Das betrifft auch deine {recent} zuletzt geöffneten Boards.") } else { String::new() };
        Some(format!(
            "Version {target} kann neue Boards nicht öffnen (Format {cur}, ab 4.0.0).{boards} Sichere wichtige Boards vor dem Wechsel."
        ))
    } else {
        None
    }
}

/// Eine gespeicherte Vorversion.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Saved {
    pub version: Version,
    pub path: PathBuf,
}

/// Dateiname einer gespeicherten Vorversion.
pub fn saved_name(v: &Version) -> String {
    format!("OwnDraw-{v}.AppImage")
}

/// Alle gespeicherten Vorversionen, neueste Version zuerst.
pub fn list_saved(dir: &Path) -> Vec<Saved> {
    let mut v: Vec<Saved> = std::fs::read_dir(dir)
        .into_iter()
        .flatten()
        .flatten()
        .filter_map(|e| {
            let n = e.file_name().to_string_lossy().into_owned();
            let ver = Version::parse(n.strip_prefix("OwnDraw-")?.strip_suffix(".AppImage")?).ok()?;
            Some(Saved { version: ver, path: e.path() })
        })
        .collect();
    v.sort_by(|a, b| b.version.cmp(&a.version));
    v
}

/// Löscht alles außer den zuletzt geschriebenen `keep` Dateien (nach Änderungszeit).
pub fn prune(dir: &Path, keep: usize) {
    let mut v: Vec<(std::time::SystemTime, PathBuf)> = list_saved(dir)
        .into_iter()
        .map(|s| (std::fs::metadata(&s.path).and_then(|m| m.modified()).unwrap_or(std::time::UNIX_EPOCH), s.path))
        .collect();
    v.sort_by_key(|b| std::cmp::Reverse(b.0));
    for (_, p) in v.into_iter().skip(keep) {
        let _ = std::fs::remove_file(p);
    }
}

/// Die zuletzt gespeicherte Vorversion (die, die zuletzt abgelöst wurde).
pub fn newest_saved(dir: &Path) -> Option<Saved> {
    list_saved(dir).into_iter().max_by_key(|s| std::fs::metadata(&s.path).and_then(|m| m.modified()).unwrap_or(std::time::UNIX_EPOCH))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tags_und_format() {
        assert_eq!(parse_tag("v4.0.1"), Some(Version::new(4, 0, 1)));
        assert_eq!(parse_tag("x"), None);
        assert!(format_warning(&Version::new(3, 0, 0), 2).is_some());
        assert!(format_warning(&Version::new(4, 0, 0), 2).is_none());
    }
}
