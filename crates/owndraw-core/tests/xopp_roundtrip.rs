//! Round-Trip-Tests für den Xournal++-Import/-Export (WP8) mit echten Xournal++-Dateien (siehe tests/xopp/QUELLEN.txt).

use owndraw_core::doc::Document;
use owndraw_core::item::{Item, Kind, ARROW_ENG};
use owndraw_core::pages::{BoardMode, PageSetup};
use owndraw_core::xopp::{export_xopp, import_xopp, XoppExport};
use serde_json::{json, Value};
use std::io::Read;
use std::path::PathBuf;

const DATEIEN: [&str; 10] = [
    "strokes.xopp",
    "pages.xopp",
    "layers.xopp",
    "image-fileversion-4.xopp",
    "latex-fileversion-4.xopp",
    "text-fileversion-4.xopp",
    "suite.xopp",
    "test1.unzipped.xoj",
    "packaged-test.xopp",
    "pdf-hintergrund.xopp",
];

fn lade(name: &str) -> Vec<u8> {
    let p = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/xopp").join(name);
    std::fs::read(&p).unwrap_or_else(|e| panic!("{}: {e}", p.display()))
}

fn entpacke(gz: &[u8]) -> String {
    assert_eq!(&gz[..2], &[0x1f, 0x8b], "Export ist nicht gzip-komprimiert");
    let mut s = String::new();
    flate2::read::GzDecoder::new(gz).read_to_string(&mut s).expect("gzip lesbar");
    s
}

fn seiten(d: &Document) -> &PageSetup {
    match &d.mode {
        BoardMode::Pages(s) => s,
        BoardMode::Infinite => panic!("Import muss Seitenmodus liefern"),
    }
}

/// Ein Objekt, seitenlokal, zum Vergleich.
#[derive(Debug)]
struct Obj {
    page: usize,
    layer: i32,
    art: &'static str,
    color: u32,
    width: f32,
    /// Strich: Tripel (x, y, Druck); Text/Bild: x, y, (w, h)
    zahlen: Vec<f32>,
    text: String,
    bild: Vec<u8>,
    attrs: Value,
}

fn objekte(d: &Document) -> Vec<Obj> {
    let s = seiten(d);
    let rs = s.rects();
    d.board["items"]
        .as_array()
        .unwrap()
        .iter()
        .map(|v| {
            let it = Item::from_json(v).expect("Item lesbar");
            let page = s.page_of(&it).unwrap();
            let (ox, oy) = (rs[page].left, rs[page].top);
            let mut o = Obj {
                page,
                layer: it.layer,
                art: "",
                color: it.color,
                width: it.width,
                zahlen: Vec::new(),
                text: String::new(),
                bild: Vec::new(),
                attrs: v.get("xa").cloned().unwrap_or(Value::Null),
            };
            match &it.kind {
                Kind::Stroke(st) => {
                    o.art = if st.hl { "marker" } else { "stift" };
                    for t in st.p.chunks(3) {
                        o.zahlen.extend_from_slice(&[t[0] - ox, t[1] - oy, t[2]]);
                    }
                }
                Kind::Note(n) => {
                    o.art = "text";
                    o.zahlen = vec![n.x - ox, n.y - oy, n.size];
                    o.text = n.text.clone();
                }
                Kind::Img(m) => {
                    o.art = "bild";
                    o.zahlen = vec![m.x - ox, m.y - oy, m.w, m.h];
                    o.bild = d.images.files[&m.file].clone();
                    o.attrs = v.get("xraw").cloned().unwrap_or(Value::Null);
                }
                Kind::Shape(_) => o.art = "form",
            }
            o
        })
        .collect()
}

fn vergleiche(name: &str, a: &Document, b: &Document) {
    let (sa, sb) = (seiten(a), seiten(b));
    assert_eq!(sa.pages.len(), sb.pages.len(), "{name}: Seitenzahl");
    for (i, (p, q)) in sa.pages.iter().zip(&sb.pages).enumerate() {
        assert!((p.w - q.w).abs() < 0.01 && (p.h - q.h).abs() < 0.01, "{name}: Seitengröße {i}");
        assert_eq!(p.bg, q.bg, "{name}: Hintergrund {i}");
        assert_eq!(p.extra["xopp"]["bg"], q.extra["xopp"]["bg"], "{name}: Hintergrund-Attribute {i}");
        assert_eq!(p.extra["xopp"]["raw"], q.extra["xopp"]["raw"], "{name}: unbekannte Elemente {i}");
    }
    assert_eq!(a.board["layers"], b.board["layers"], "{name}: Ebenen");
    let (oa, ob) = (objekte(a), objekte(b));
    assert_eq!(oa.len(), ob.len(), "{name}: Objektzahl");
    // Zahl je Seite, Ebene und Art
    for o in &oa {
        let z = |v: &[Obj]| v.iter().filter(|x| x.page == o.page && x.layer == o.layer && x.art == o.art).count();
        assert_eq!(z(&oa), z(&ob), "{name}: Anzahl {} auf Seite {} Ebene {}", o.art, o.page, o.layer);
    }
    for (k, (x, y)) in oa.iter().zip(&ob).enumerate() {
        let w = format!("{name}: Objekt {k} ({})", x.art);
        assert_eq!((x.page, x.layer, x.art), (y.page, y.layer, y.art), "{w}");
        assert_eq!(x.color, y.color, "{w}: Farbe");
        assert!((x.width - y.width).abs() < 0.01, "{w}: Breite {} / {}", x.width, y.width);
        assert_eq!(x.zahlen.len(), y.zahlen.len(), "{w}: Punktzahl");
        for (u, v) in x.zahlen.iter().zip(&y.zahlen) {
            assert!((u - v).abs() <= 0.0101, "{w}: Koordinate/Druck {u} / {v}");
        }
        assert_eq!(x.text, y.text, "{w}: Text");
        assert!(x.bild == y.bild, "{w}: Bilddaten");
        assert_eq!(x.attrs, y.attrs, "{w}: Zusatzattribute");
    }
}

/// import → export → import; liefert (erster Import, exportiertes XML, zweiter Import).
fn rundreise(name: &str) -> (Document, String, Document) {
    let a = import_xopp(&lade(name)).unwrap_or_else(|e| panic!("{name}: {e}"));
    let gz = export_xopp(&a, XoppExport::default()).unwrap();
    let xml = entpacke(&gz);
    roxmltree::Document::parse(&xml).unwrap_or_else(|e| panic!("{name}: Export ist kein gültiges XML: {e}"));
    if let Some(dir) = std::env::var_os("XOPP_EXPORT_DIR") {
        std::fs::write(PathBuf::from(dir).join(name.replace(".xoj", ".xopp")), &gz).unwrap();
    }
    let b = import_xopp(&gz).unwrap_or_else(|e| panic!("{name} (Reimport): {e}"));
    vergleiche(name, &a, &b);
    (a, xml, b)
}

#[test]
fn alle_dateien_rundreise() {
    for n in DATEIEN {
        rundreise(n);
    }
}

#[test]
fn striche_druck_und_farben() {
    let (a, xml, _) = rundreise("strokes.xopp");
    let os = objekte(&a);
    assert_eq!(os.len(), 12);
    // 1. Strich: Grundbreite 2.26, erste Streckenbreite 0.41932136
    let s = &os[0];
    assert_eq!(s.color, 0xFF002E99);
    assert!((s.width - 1.13).abs() < 1e-4);
    let p0 = (2.0 * 0.41932136 / 2.26 - 0.35) / 1.3;
    assert!((s.zahlen[2] - p0).abs() < 0.006, "Druck {}", s.zahlen[2]);
    // Marker, Füllung, Radierer und Strichart bleiben erhalten
    assert_eq!(os.iter().filter(|o| o.art == "marker").count(), 4);
    assert!(xml.contains(r##"tool="highlighter" color="#ffa1547f" width="19.84""##));
    assert!(xml.contains(r#"fill="128""#));
    assert!(xml.contains(r#"tool="eraser""#));
    assert!(xml.contains(r#"style="dashdot""#));
    // Druckbreiten werden exakt zurückgeschrieben
    // Druckbreiten: Grundbreite exakt, Streckenbreiten bis auf die Rundung des Drucks (0,01) im Item-JSON
    let w: Vec<f32> = xml.split("<stroke").nth(1).unwrap().split("width=\"").nth(1).unwrap().split('"').next().unwrap().split(' ').map(|v| v.parse().unwrap()).collect();
    assert_eq!(w.len(), 69);
    assert_eq!(w[0], 2.26);
    assert!((w[1] - 0.41932136).abs() < 2.26 * 0.65 * 0.0051 + 1e-4, "{}", w[1]);
}

#[test]
fn seiten_hintergruende() {
    let (a, xml, _) = rundreise("pages.xopp");
    let s = seiten(&a);
    assert_eq!(s.pages.len(), 11);
    let names: Vec<&str> = s.pages.iter().map(|p| p.bg.name()).collect();
    assert_eq!(names, ["plain", "lined", "lined", "lined", "plain", "grid", "dotted", "plain", "plain", "grid", "plain"]);
    // Seite i liegt im Seitenmodus untereinander: Text "p2" bei y = 100 + Seite 1
    let os = objekte(&a);
    assert_eq!(os[1].text, "p2");
    assert!((os[1].zahlen[1] - 100.0).abs() < 0.01);
    for t in [
        r#"style="ruled""#,
        r#"style="lined" config="m1=-72""#,
        r#"style="staves""#,
        r#"style="isograph""#,
        r#"style="graph" config="m1=40,rm=1""#,
        r#"<background type="pixmap" domain="attach" filename="bg_1.png"/>"#,
    ] {
        assert!(xml.contains(t), "fehlt: {t}");
    }
}

#[test]
fn ebenen_nach_position() {
    let (a, xml, _) = rundreise("layers.xopp");
    let l = a.board["layers"].as_array().unwrap();
    assert_eq!(l.len(), 3);
    assert_eq!(l[1]["n"], json!("Named layer"));
    let os = objekte(&a);
    assert_eq!(os.iter().map(|o| o.layer).collect::<Vec<_>>(), [1, 2, 3]);
    assert_eq!(xml.matches("<layer").count(), 3);
    assert!(xml.contains(r#"<layer name="Named layer">"#));
}

#[test]
fn texte_schrift_und_leerraum() {
    let (a, xml, _) = rundreise("text-fileversion-4.xopp");
    let os = objekte(&a);
    assert_eq!(os.len(), 6);
    assert_eq!(os[0].color, 0xFFFF0000);
    assert_eq!(os[4].text, " \n odd  whitespace\ttext\n\n");
    assert!((os[5].zahlen[2] - 12.34).abs() < 1e-4);
    assert!(xml.contains(r#"font="Times New Roman, Bold" size="12.34""#));
}

#[test]
fn bilder_und_latex() {
    let (a, xml, _) = rundreise("image-fileversion-4.xopp");
    assert_eq!(a.images.files.len(), 1);
    let os = objekte(&a);
    assert_eq!(os[0].art, "bild");
    assert!(os[0].bild.starts_with(b"\x89PNG"));
    let x = roxmltree::Document::parse(&xml).unwrap();
    let im = x.descendants().find(|n| n.has_tag_name("image")).unwrap();
    for (k, v) in [("left", 41.637795), ("top", 164.94488), ("right", 553.63779), ("bottom", 676.94488)] {
        assert!((im.attribute(k).unwrap().parse::<f64>().unwrap() - v).abs() < 0.01, "{k}");
    }
    // LaTeX (PDF-Daten) bleibt roh erhalten
    let (l, xml, _) = rundreise("latex-fileversion-4.xopp");
    assert!(objekte(&l).is_empty());
    assert!(xml.contains(r#"<teximage text="x^2" left="14.937" top="15.715" right="35.063" bottom="34.285">JVBERi0xLjcK"#));
}

#[test]
fn pdf_hintergrund_unveraendert() {
    let (a, xml, _) = rundreise("pdf-hintergrund.xopp");
    let s = seiten(&a);
    assert_eq!(s.pages.len(), 3);
    assert!(xml.contains(r#"<background type="pdf" domain="absolute" filename="/home/user/skript.pdf" pageno="1ll"/>"#));
    assert!(xml.contains(r#"<background type="pdf" pageno="2ll"/>"#));
    assert!(xml.contains(r##"<background type="solid" color="#fffff0ff" style="dotted"/>"##));
    assert!(xml.contains(r#"<unbekannt art="test">roh &amp; erhalten</unbekannt>"#));
    assert_eq!(s.pages[1].extra["xopp"]["layers"], json!(2));
    let os = objekte(&a);
    let arten: Vec<(usize, i32, &str)> = os.iter().map(|o| (o.page, o.layer, o.art)).collect();
    assert_eq!(arten, [(0, 1, "stift"), (0, 1, "marker"), (0, 1, "text"), (0, 2, "bild"), (0, 2, "stift"), (1, 1, "stift"), (2, 1, "text")]);
    assert_eq!(os[2].text, "Hallo & <Welt>\nzweite Zeile");
    assert_eq!(a.board["layers"][1]["n"], json!("Notizen"));
    // seitenlokale Koordinaten: Seite 1 liegt unter Seite 0
    let rs = s.rects();
    let it = Item::from_json(&a.board["items"][5]).unwrap();
    assert!(it.bounds().top > rs[1].top && it.bounds().top < rs[1].top + 20.0);
    assert!(xml.contains("<stroke tool=\"pen\" color=\"#008000ff\" width=\"1.41 0.50"));
    assert!(xml.contains("Hallo &amp; &lt;Welt&gt;\nzweite Zeile</text>"));
}

#[test]
fn unkomprimiert_und_zip() {
    let a = import_xopp(&lade("test1.unzipped.xoj")).unwrap();
    assert_eq!(objekte(&a)[0].text, "12345");
    let z = import_xopp(&lade("packaged-test.xopp")).unwrap();
    assert_eq!(objekte(&z)[0].text, "12345");
    assert!(import_xopp(b"<html/>").is_err());
    assert!(import_xopp(b"\x1f\x8b kaputt").is_err());
}

fn unendlich(items: Vec<Item>) -> Document {
    let mut d = Document::new("Unendlich", BoardMode::Infinite);
    d.board["items"] = Value::Array(items.iter().map(Item::json).collect());
    d
}

#[test]
fn unendliches_board_mit_maschinenbaupfeil() {
    let pfeil = Item::shape(ARROW_ENG, vec![100.0, 100.0, 300.0, 150.0], 0xFF0000FF, 2.0);
    let mut strich = Item::stroke(0xFF111111, 3.0, false, true);
    for i in 0..10 {
        strich.add_point(50.0 + i as f32 * 10.0, 400.0, i as f32 / 10.0);
    }
    let notiz = Item::note("Notiz", 0xFF111111, 18.0, 400.0, 300.0, 200.0, 120.0, 0xFFFFF59D);
    let d = unendlich(vec![pfeil, strich, notiz]);
    let gz = export_xopp(&d, XoppExport::default()).unwrap();
    if let Some(dir) = std::env::var_os("XOPP_EXPORT_DIR") {
        std::fs::write(PathBuf::from(dir).join("unendlich.xopp"), &gz).unwrap();
    }
    let xml = entpacke(&gz);
    let x = roxmltree::Document::parse(&xml).unwrap();
    let root = x.root_element();
    assert_eq!(root.attribute("fileversion"), Some("4"));
    assert!(root.attribute("creator").unwrap().starts_with("OwnDraw "));
    assert_eq!(root.children().filter(|n| n.has_tag_name("title")).count(), 1);
    assert_eq!(root.children().filter(|n| n.has_tag_name("page")).count(), 1);
    let filled: Vec<_> = x.descendants().filter(|n| n.has_tag_name("stroke") && n.attribute("fill") == Some("255")).collect();
    assert!(filled.iter().any(|n| n.attribute("color") == Some("#0000ffff")), "Pfeilspitze als gefüllter Stroke fehlt");
    // Rand 20 pt: Seite beginnt 20 pt vor dem Inhalt
    let a = import_xopp(&export_xopp(&d, XoppExport::default()).unwrap()).unwrap();
    assert_eq!(seiten(&a).pages.len(), 1);
    let os = objekte(&a);
    let minx = os.iter().filter(|o| o.art == "stift").flat_map(|o| o.zahlen.chunks(3).map(|t| t[0])).fold(f32::MAX, f32::min);
    assert!(minx > 19.0 && minx < 40.0, "Rand {minx}");
    // Druck bleibt erhalten
    let st = os.iter().find(|o| o.zahlen.len() == 30).unwrap();
    assert!((st.width - 3.0).abs() < 1e-4);
    assert!((st.zahlen[5] - 0.1).abs() < 0.011);
    // A4-Kacheln
    let weit = unendlich(vec![Item::shape(ARROW_ENG, vec![0.0, 0.0, 1500.0, 2000.0], 0xFF000000, 2.0)]);
    let xml = entpacke(&export_xopp(&weit, XoppExport { tile_a4: true }).unwrap());
    let x = roxmltree::Document::parse(&xml).unwrap();
    let pages: Vec<_> = x.descendants().filter(|n| n.has_tag_name("page")).collect();
    assert_eq!(pages.len(), 3 * 3);
    assert!(pages.iter().all(|p| p.attribute("width") == Some("595") && p.attribute("height") == Some("842")));
    // leeres Board → eine A4-Seite
    let leer = entpacke(&export_xopp(&unendlich(Vec::new()), XoppExport::default()).unwrap());
    assert_eq!(leer.matches("<page ").count(), 1);
}

#[test]
fn seitenmodus_export_owndraw() {
    // OwnDraw-Board im Seitenmodus ohne Xournal-Zusatzdaten
    let mut d = Document::new("Seiten", BoardMode::Pages(PageSetup::new(595.0, 842.0, owndraw_core::pages::PageBg::Grid, 2)));
    let rs = seiten(&d).rects();
    let mut it = Item::stroke(0xFF00FF00, 2.0, true, false);
    it.add_point(10.0, rs[1].top + 10.0, 0.5);
    it.add_point(50.0, rs[1].top + 30.0, 0.5);
    d.board["items"] = json!([it.json()]);
    let xml = entpacke(&export_xopp(&d, XoppExport::default()).unwrap());
    assert_eq!(xml.matches("<page ").count(), 2);
    assert_eq!(xml.matches(r#"style="graph""#).count(), 2);
    assert!(xml.contains(r##"<stroke tool="highlighter" color="#00ff007f" width="2">10 10 50 30</stroke>"##));
}
