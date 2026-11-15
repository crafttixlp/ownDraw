//! Zwischenablage im Editor (WP10).
//!
//! Kopieren legt die Auswahl als transparentes PNG (zum Einfügen in andere Programme) **und** als interne
//! Item-Daten (JSON inkl. referenzierter Bilder als base64) in die System-Zwischenablage. Einfügen bevorzugt die
//! Items, dann ein Bild, dann Text. Die Einfüge-Logik hängt nicht an `Platform` (`insert_clip`) und ist so testbar.

use super::Editor;
use crate::app::Shared;
use crate::platform::{ClipData, Platform};
use crate::session;
use egui::Pos2;
use owndraw_core::board::{self, Event};
use owndraw_core::item::{Item, Kind};
use owndraw_core::tiny_skia::Pixmap;
use serde_json::{json, Value};
use std::collections::BTreeMap;

/// Kennung im JSON, damit fremder Text nicht als Items missverstanden wird.
const MAGIC: &str = "owndraw-items";
/// Auflösung des Zwischenablage-Bildes relativ zu Weltpunkten und Obergrenze der Kantenlänge.
const COPY_SCALE: f32 = 2.0;
const COPY_MAX_PX: u32 = 4096;

// ---------------------------------------------------------------- base64 (ohne Zusatz-Abhängigkeit)

const B64: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";

fn b64_encode(d: &[u8]) -> String {
    let mut o = String::with_capacity(d.len().div_ceil(3) * 4);
    for c in d.chunks(3) {
        let n = (c[0] as u32) << 16
            | (*c.get(1).unwrap_or(&0) as u32) << 8
            | *c.get(2).unwrap_or(&0) as u32;
        o.push(B64[(n >> 18) as usize & 63] as char);
        o.push(B64[(n >> 12) as usize & 63] as char);
        o.push(if c.len() > 1 {
            B64[(n >> 6) as usize & 63] as char
        } else {
            '='
        });
        o.push(if c.len() > 2 {
            B64[n as usize & 63] as char
        } else {
            '='
        });
    }
    o
}

fn b64_decode(s: &str) -> Option<Vec<u8>> {
    let mut o = Vec::with_capacity(s.len() / 4 * 3);
    let (mut acc, mut bits) = (0u32, 0);
    for ch in s.bytes() {
        let v = match ch {
            b'A'..=b'Z' => ch - b'A',
            b'a'..=b'z' => ch - b'a' + 26,
            b'0'..=b'9' => ch - b'0' + 52,
            b'+' => 62,
            b'/' => 63,
            b'=' | b'\n' | b'\r' | b' ' => continue,
            _ => return None,
        };
        acc = acc << 6 | v as u32;
        bits += 6;
        if bits >= 8 {
            bits -= 8;
            o.push((acc >> bits) as u8);
            acc &= (1 << bits) - 1;
        }
    }
    Some(o)
}

// ---------------------------------------------------------------- Items <-> JSON

/// Dateiname eines Bildes darf kein Pfad sein (die Daten stammen aus der Zwischenablage).
fn safe_name(n: &str) -> bool {
    !n.is_empty() && n.len() < 200 && !n.contains(['/', '\\', ':']) && !n.starts_with('.')
}

/// Items der Auswahl als JSON samt Bilddaten (`images`: Name → base64).
pub fn items_json(editor: &Editor) -> Option<String> {
    let sel = editor.board.sel_items();
    if sel.is_empty() {
        return None;
    }
    let mut images = serde_json::Map::new();
    for it in &sel {
        if let Kind::Img(m) = &it.kind {
            if images.contains_key(&m.file) || !safe_name(&m.file) {
                continue;
            }
            let bytes = editor
                .doc
                .images
                .files
                .get(&m.file)
                .cloned()
                .or_else(|| std::fs::read(editor.img_dir.join(&m.file)).ok());
            if let Some(b) = bytes {
                images.insert(m.file.clone(), Value::String(b64_encode(&b)));
            }
        }
    }
    let items: Vec<Value> = sel.iter().map(|it| it.json()).collect();
    Some(json!({"owndraw": MAGIC, "v": 1, "items": items, "images": images}).to_string())
}

/// Objekte und mitgelieferte Bilder (Name → Bytes).
pub type ParsedItems = (Vec<Item>, BTreeMap<String, Vec<u8>>);

/// Liest Items-JSON; liefert die Objekte und die mitgelieferten Bilder.
pub fn parse_items(json: &str) -> Option<ParsedItems> {
    let v: Value = serde_json::from_str(json).ok()?;
    if v.get("owndraw")?.as_str()? != MAGIC {
        return None;
    }
    let items: Vec<Item> = v
        .get("items")?
        .as_array()?
        .iter()
        .filter_map(Item::from_json)
        .collect();
    if items.is_empty() {
        return None;
    }
    let mut images = BTreeMap::new();
    if let Some(m) = v.get("images").and_then(Value::as_object) {
        for (k, b) in m {
            if let (true, Some(d)) = (safe_name(k), b.as_str().and_then(b64_decode)) {
                images.insert(k.clone(), d);
            }
        }
    }
    Some((items, images))
}

/// Gerendertes Bild (vormultipliziert) → RGBA mit geraden Alphawerten.
pub fn pixmap_rgba(pm: &Pixmap) -> (u32, u32, Vec<u8>) {
    let mut out = Vec::with_capacity(pm.data().len());
    for p in pm.pixels() {
        let c = p.demultiply();
        out.extend_from_slice(&[c.red(), c.green(), c.blue(), c.alpha()]);
    }
    (pm.width(), pm.height(), out)
}

// ---------------------------------------------------------------- Kopieren / Ausschneiden

fn notice(editor: &mut Editor, s: &str) {
    editor.board.events.push(Event::Notice(s.to_string()));
}

/// Daten für die Zwischenablage aus der Auswahl (None ohne Auswahl).
pub fn clip_data(editor: &Editor) -> Option<ClipData> {
    let json = items_json(editor)?;
    let image = editor
        .board
        .render_selection(COPY_SCALE, COPY_MAX_PX)
        .map(|pm| pixmap_rgba(&pm));
    Some(ClipData::Items { json, image })
}

pub fn copy(editor: &mut Editor, platform: &dyn Platform) {
    if editor.board.sel.is_empty() {
        return;
    }
    // interne Zwischenablage des Kerns bleibt als Rückfall (z. B. ohne System-Zwischenablage)
    editor.board.copy_sel();
    if let Some(d) = clip_data(editor) {
        platform.clipboard_set(d);
    }
}

pub fn cut(editor: &mut Editor, platform: &dyn Platform) {
    if editor.board.sel.is_empty() {
        return;
    }
    copy(editor, platform);
    editor.board.delete_sel();
}

// ---------------------------------------------------------------- Einfügen

/// Weltpunkt unter einem Bildschirmpunkt (nur über der Zeichenfläche).
pub fn world_at(editor: &Editor, p: Pos2) -> Option<(f32, f32)> {
    if !editor.canvas.contains(p) {
        return None;
    }
    let r = p - editor.canvas.min;
    Some(editor.board.screen_to_world(r.x, r.y))
}

/// Einfügen an der Mausposition (Strg+V).
pub fn paste(editor: &mut Editor, platform: &dyn Platform) {
    let at = editor.hover.and_then(|p| world_at(editor, p));
    paste_world(editor, platform, at);
}

/// Einfügen in die Ansichtsmitte (Knopf in der Auswahlleiste).
pub fn paste_center(editor: &mut Editor, platform: &dyn Platform) {
    paste_world(editor, platform, None);
}

fn paste_world(editor: &mut Editor, platform: &dyn Platform, at: Option<(f32, f32)>) {
    match platform.clipboard_get() {
        Some(d) => {
            if let Err(m) = insert_clip(editor, d, at) {
                notice(editor, &m);
            }
        }
        None if editor.board.has_clip() => editor.board.paste(),
        None => notice(editor, "Die Zwischenablage ist leer"),
    }
}

/// Fügt Zwischenablage-Inhalt ein: Items vor Bild vor Text. `at` = Mittelpunkt in Weltkoordinaten.
pub fn insert_clip(editor: &mut Editor, d: ClipData, at: Option<(f32, f32)>) -> Result<(), String> {
    match d {
        ClipData::Items { json, image } => match insert_items_json(editor, &json, at) {
            Ok(()) => Ok(()),
            // unlesbare Items: Vorschaubild ist immer noch besser als nichts
            Err(e) => match image {
                Some((w, h, rgba)) => insert_image_rgba(editor, w, h, &rgba, at),
                None => Err(e),
            },
        },
        ClipData::Image { w, h, rgba } => insert_image_rgba(editor, w, h, &rgba, at),
        ClipData::Text(t) => {
            // Items-JSON, das als Text angekommen ist (anderer Prozess)
            if insert_items_json(editor, &t, at).is_ok() {
                return Ok(());
            }
            if t.trim().is_empty() {
                return Err("Die Zwischenablage ist leer".into());
            }
            let (x, y) = editor.board.paste_center(at);
            editor.board.add_note(x, y, t.trim_end(), false);
            Ok(())
        }
    }
}

/// Fügt Items aus dem JSON ein und nimmt fehlende Bilder in das Dokument mit.
pub fn insert_items_json(
    editor: &mut Editor,
    json: &str,
    at: Option<(f32, f32)>,
) -> Result<(), String> {
    let (items, images) = parse_items(json)
        .ok_or_else(|| "Keine einfügbaren Objekte in der Zwischenablage".to_string())?;
    for (name, bytes) in images {
        if editor.doc.images.files.contains_key(&name) {
            continue;
        }
        let _ = std::fs::create_dir_all(&editor.img_dir);
        let _ = std::fs::write(editor.img_dir.join(&name), &bytes);
        editor.doc.images.files.insert(name, bytes);
    }
    let n = editor.board.paste_items(&items, at);
    if n == 0 {
        return Err("Die Ebene ist gesperrt".into());
    }
    if n > 1 {
        notice(editor, &format!("{n} Objekte eingefügt"));
    }
    Ok(())
}

/// RGBA8 (nicht vormultipliziert) als Bild einfügen.
pub fn insert_image_rgba(
    editor: &mut Editor,
    w: u32,
    h: u32,
    rgba: &[u8],
    at: Option<(f32, f32)>,
) -> Result<(), String> {
    let buf = image::RgbaImage::from_raw(w, h, rgba.to_vec())
        .ok_or("Ungültige Bilddaten in der Zwischenablage")?;
    let mut png = Vec::new();
    image::DynamicImage::ImageRgba8(buf)
        .write_to(&mut std::io::Cursor::new(&mut png), image::ImageFormat::Png)
        .map_err(|e| e.to_string())?;
    insert_image_bytes(editor, &png, at)
}

/// Bild aus Dateiinhalt einfügen (auf höchstens 2048 px verkleinert, Größe passend zur Ansicht).
pub fn insert_image_bytes(
    editor: &mut Editor,
    bytes: &[u8],
    at: Option<(f32, f32)>,
) -> Result<(), String> {
    let (name, w, h) = session::import_image(&mut editor.doc.images, &editor.img_dir, bytes)?;
    if editor.board.add_image_at(&name, w, h, at) {
        Ok(())
    } else {
        Err("Die Ebene ist gesperrt".into())
    }
}

// ---------------------------------------------------------------- Menü (Rechtsklick / Lang-Tippen)

/// Kontextmenü der Zeichenfläche mit Kopieren/Ausschneiden/Einfügen (für Touch ohne Tastatur).
pub fn context_menu(editor: &mut Editor, resp: &egui::Response, sh: &mut Shared) {
    if resp.secondary_clicked() || resp.long_touched() {
        editor.menu_pos = resp.interact_pointer_pos();
    }
    let has_sel = !editor.board.sel.is_empty() && editor.board.tool == board::SELECT;
    let mut action = 0u8;
    resp.context_menu(|ui| {
        if has_sel {
            if ui.button("Kopieren").clicked() {
                action = 1;
            }
            if ui.button("Ausschneiden").clicked() {
                action = 2;
            }
        }
        if ui.button("Einfügen").clicked() {
            action = 3;
        }
        if action != 0 {
            ui.close();
        }
    });
    match action {
        1 => copy(editor, &*sh.platform),
        2 => cut(editor, &*sh.platform),
        3 => {
            let at = editor.menu_pos.and_then(|p| world_at(editor, p));
            paste_world(editor, &*sh.platform, at);
        }
        _ => {}
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use crate::platform::stub::StubPlatform;
    use crate::session::{new_document, NewParams};
    use owndraw_core::item;
    use std::rc::Rc;

    pub(crate) fn editor(pages: bool) -> Editor {
        let mut app = crate::app::App::new(
            Box::new(StubPlatform::new()),
            crate::settings::Settings::default(),
            None,
        );
        let doc = new_document(&NewParams {
            name: "T".into(),
            pages,
            ..NewParams::default()
        });
        let mut ed = Editor::open(app.parts().0, doc, None);
        ed.board.set_size(800.0, 600.0, 1.0);
        ed
    }

    fn png(w: u32, h: u32) -> Vec<u8> {
        let mut o = Vec::new();
        let img = image::RgbaImage::from_fn(w, h, |x, y| image::Rgba([x as u8, y as u8, 7, 200]));
        image::DynamicImage::ImageRgba8(img)
            .write_to(&mut std::io::Cursor::new(&mut o), image::ImageFormat::Png)
            .unwrap();
        o
    }

    #[test]
    fn base64_roundtrip() {
        for n in 0..9usize {
            let d: Vec<u8> = (0..n as u8).map(|i| i.wrapping_mul(37)).collect();
            assert_eq!(b64_decode(&b64_encode(&d)).unwrap(), d);
        }
        assert!(b64_decode("a$b").is_none());
    }

    #[test]
    fn items_json_roundtrip_verschoben() {
        let mut ed = editor(false);
        ed.board.items.push(Rc::new(Item::shape(
            item::ELLIPSE,
            vec![10.0, 10.0, 90.0, 10.0, 10.0, 60.0],
            0xFF3E63DD,
            3.0,
        )));
        ed.board.items.push(Rc::new(Item::note(
            "Hallo", 0xFF1F2025, 20.0, 200.0, 200.0, 100.0, 100.0, 0,
        )));
        let b = png(16, 8);
        ed.import_image_bytes(&b).unwrap();
        ed.board.sel = ed.board.items.iter().map(|i| i.id).collect();
        let before = ed.board.items.len();
        let ob = ed.board.sel_bounds().unwrap();
        let json = items_json(&ed).unwrap();
        let (items, images) = parse_items(&json).unwrap();
        assert_eq!(items.len(), 3);
        assert_eq!(images.len(), 1);
        // Einfügen in ein anderes Dokument: Bild wird mitgenommen
        let mut ed2 = editor(false);
        insert_items_json(&mut ed2, &json, Some((1000.0, 1000.0))).unwrap();
        assert_eq!(ed2.board.items.len(), before);
        assert_eq!(ed2.doc.images.files.len(), 1);
        let nb = ed2.board.sel_bounds().unwrap();
        assert!((nb.center_x() - 1000.0).abs() < 1.0 && (nb.center_y() - 1000.0).abs() < 1.0);
        assert!((nb.width() - ob.width()).abs() < 0.5);
        assert_eq!(ed2.board.sel.len(), 3);
    }

    #[test]
    fn rgba_wird_img_item() {
        let mut ed = editor(false);
        let rgba: Vec<u8> = (0..10 * 6 * 4).map(|i| i as u8).collect();
        insert_clip(
            &mut ed,
            ClipData::Image { w: 10, h: 6, rgba },
            Some((50.0, 60.0)),
        )
        .unwrap();
        assert_eq!(ed.board.items.len(), 1);
        assert!(matches!(ed.board.items[0].kind, Kind::Img(_)));
        assert_eq!(ed.doc.images.files.len(), 1);
        let b = ed.board.items[0].bounds();
        assert!((b.center_x() - 50.0).abs() < 1.0 && (b.center_y() - 60.0).abs() < 1.0);
    }

    #[test]
    fn text_wird_textobjekt_und_fremdes_json_nicht_items() {
        let mut ed = editor(false);
        insert_clip(&mut ed, ClipData::Text("Hallo Welt".into()), None).unwrap();
        assert_eq!(ed.board.items.len(), 1);
        assert!(ed.board.items[0].as_note().is_some());
        assert!(insert_clip(&mut ed, ClipData::Text("  ".into()), None).is_err());
        assert!(parse_items("{\"items\":[]}").is_none());
    }

    #[test]
    fn seitenmodus_klemmt_auf_seite() {
        let ed = editor(true);
        let (x, y) = ed.board.paste_center(Some((-5000.0, 100.0)));
        assert!((0.0..=595.0).contains(&x) && y == 100.0);
    }

    #[test]
    fn kopieren_setzt_png_und_items() {
        let mut ed = editor(false);
        ed.board.items.push(Rc::new(Item::shape(
            item::ELLIPSE,
            vec![10.0, 10.0, 90.0, 10.0, 10.0, 60.0],
            0xFF3E63DD,
            3.0,
        )));
        ed.board.sel = vec![ed.board.items[0].id];
        let pf = StubPlatform::new();
        copy(&mut ed, &pf);
        match pf.clipboard_get() {
            Some(ClipData::Items {
                image: Some((w, h, rgba)),
                ..
            }) => {
                assert!(w > 0 && h > 0 && w.max(h) <= COPY_MAX_PX);
                assert_eq!(rgba.len(), (w * h * 4) as usize);
                // transparent: Ecke ohne Inhalt ist durchsichtig
                assert_eq!(rgba[3], 0);
            }
            other => panic!("unerwartet: {other:?}"),
        }
        let mut ed2 = editor(false);
        paste_center(&mut ed2, &pf);
        assert_eq!(ed2.board.items.len(), 1);
    }

    /// Echte X11-Zwischenablage (Xvfb + xclip): `cargo test -- --ignored clipboard_x11`.
    #[test]
    #[ignore]
    fn clipboard_x11_xclip() {
        use std::process::Command;
        let pf = crate::platform::desktop::DesktopPlatform::new(None);
        // 1) xclip → Einfügen
        let dir = std::env::temp_dir().join("owndraw-wp10-xclip");
        std::fs::create_dir_all(&dir).unwrap();
        let f = dir.join("in.png");
        std::fs::write(&f, png(24, 12)).unwrap();
        let st = Command::new("sh")
            .arg("-c")
            .arg(format!(
                "xclip -selection clipboard -t image/png -i {}",
                f.display()
            ))
            .status()
            .unwrap();
        assert!(st.success());
        let mut ed = editor(false);
        paste_center(&mut ed, &pf);
        assert!(matches!(
            ed.board.items.last().map(|i| &i.kind),
            Some(Kind::Img(_))
        ));
        // 2) Auswahl kopieren → xclip -o liefert PNG
        ed.board.sel = vec![ed.board.items[0].id];
        copy(&mut ed, &pf);
        let out = Command::new("xclip")
            .args(["-selection", "clipboard", "-o", "-t", "image/png"])
            .output()
            .unwrap();
        assert!(
            out.stdout.starts_with(&[0x89, b'P', b'N', b'G']),
            "kein PNG: {} Bytes",
            out.stdout.len()
        );
        let img = image::load_from_memory(&out.stdout).unwrap();
        assert!(img.width() >= 24);
    }
}
