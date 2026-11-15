//! Einmalige Migration der alten Ablage (index.json + Board-Ordner + img/) in `.owndraw`-Dateien (v2).

use crate::doc::{read_owndraw, write_owndraw};
use crate::store::{export_board_files, index_in, write_atomic};
use serde_json::Value;
use std::fs;
use std::io::Cursor;
use std::path::{Path, PathBuf};

/// Name der Markierungsdatei im alten Datenordner.
pub const MARKER: &str = "migrated-v4";

/// Bereinigt einen Board-Namen zu einem Dateinamen (ohne Endung).
fn clean_name(n: &str) -> String {
    let s: String = n
        .chars()
        .map(|c| {
            if c.is_control() || "/\\:*?\"<>|".contains(c) {
                '_'
            } else {
                c
            }
        })
        .collect();
    let s = s.trim().trim_matches('.').trim().to_string();
    let s: String = s.chars().take(80).collect();
    if s.is_empty() {
        "Board".into()
    } else {
        s
    }
}

fn free_path(dir: &Path, name: &str) -> PathBuf {
    let mut p = dir.join(format!("{name}.owndraw"));
    let mut i = 2;
    while p.exists() {
        p = dir.join(format!("{name} ({i}).owndraw"));
        i += 1;
    }
    p
}

/// Migriert alle Boards der alten Ablage nach `target_dir` und liefert die geschriebenen Dateien.
/// `old_data_dir` ist der Datenordner der App (Boards in `files/boards`, Fallback `boards`).
/// Läuft nur einmal (Markierungsdatei `migrated-v4`); die alten Daten bleiben unangetastet.
pub fn migrate_store(old_data_dir: &Path, target_dir: &Path) -> Vec<PathBuf> {
    let mut out = Vec::new();
    if old_data_dir.join(MARKER).exists() {
        return out;
    }
    let boards = [
        old_data_dir.join("files").join("boards"),
        old_data_dir.join("boards"),
    ]
    .into_iter()
    .find(|d| d.join("index.json").is_file());
    let Some(boards) = boards else { return out };
    let img = boards.join("img");
    let _ = fs::create_dir_all(target_dir);
    for o in index_in(&boards) {
        let Some(id) = o.get("id").and_then(Value::as_str) else {
            continue;
        };
        let name = o.get("name").and_then(Value::as_str).unwrap_or("Board");
        // Über das v1-Austauschformat laufen lassen: liest Board, Vorschau und Bilder einheitlich.
        let mut buf = Cursor::new(Vec::new());
        if !export_board_files(&boards, &img, id, name, &mut buf) {
            continue;
        }
        let Ok(mut doc) = read_owndraw(buf.get_ref()) else {
            continue;
        };
        doc.meta.name = name.to_string();
        let Ok(bytes) = write_owndraw(&doc) else {
            continue;
        };
        let p = free_path(target_dir, &clean_name(name));
        if write_atomic(&p, &bytes).is_ok() {
            out.push(p);
        }
    }
    let _ = fs::write(old_data_dir.join(MARKER), b"1");
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::store::Store;

    #[test]
    fn migration_erzeugt_owndraw_dateien() {
        let base = std::env::temp_dir().join(format!("od_mig_{}", crate::store::random_hex(10)));
        let old = base.join("data");
        let new = base.join("neu");
        let st = Store::new(&old.join("files"));
        let a = st.create("Mathe/Heft");
        let b = st.create("Mathe/Heft");
        let png = {
            let mut v = Vec::new();
            image::DynamicImage::new_rgba8(2, 2)
                .write_to(&mut Cursor::new(&mut v), image::ImageFormat::Png)
                .unwrap();
            v
        };
        let img = st.save_image_png(&png).unwrap();
        let board = serde_json::json!({"bg":0,"items":[{"t":"i","f":img}],"layers":[]});
        st.save(&a, &board.to_string(), None, true);
        st.save(
            &b,
            &serde_json::json!({"items":[]}).to_string(),
            Some(&png),
            true,
        );
        let r = migrate_store(&old, &new);
        assert_eq!(r.len(), 2);
        let names: Vec<String> = r
            .iter()
            .map(|p| p.file_name().unwrap().to_string_lossy().into_owned())
            .collect();
        assert!(
            names.contains(&"Mathe_Heft.owndraw".to_string())
                && names.contains(&"Mathe_Heft (2).owndraw".to_string())
        );
        let docs: Vec<_> = r
            .iter()
            .map(|p| read_owndraw(&fs::read(p).unwrap()).unwrap())
            .collect();
        assert!(docs.iter().any(|d| d.images.files.len() == 1));
        assert!(docs.iter().all(|d| d.meta.version == 2));
        assert!(old.join(MARKER).exists() && old.join("files/boards/index.json").exists());
        assert!(migrate_store(&old, &new).is_empty());
        let _ = fs::remove_dir_all(&base);
    }
}
