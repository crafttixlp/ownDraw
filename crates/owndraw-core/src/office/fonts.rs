//! Schriften der Textverarbeitung: „OwnDraw Sans“ und „OwnDraw Serif“ (Teilmengen der Liberation Fonts, SIL OFL 1.1,
//! siehe `fonts/OFL.txt`), je normal, fett, kursiv und fett-kursiv. Maßgleich zu Arial bzw. Times New Roman, damit
//! der Umbruch zu Word-Dokumenten passt. Fehlende Zeichen (Emoji, Schriften außerhalb Latein/Griechisch) kommen aus
//! der Ausweichschrift von [`crate::text`].

use tiny_skia::PathBuilder;
use ttf_parser::{Face, GlyphId, OutlineBuilder};

/// Schriftfamilie.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Default)]
pub enum Family {
    #[default]
    Serif,
    Sans,
}

impl Family {
    pub fn name(self) -> &'static str {
        match self {
            Family::Serif => "serif",
            Family::Sans => "sans",
        }
    }
    pub fn from_name(s: &str) -> Family {
        if s == "sans" {
            Family::Sans
        } else {
            Family::Serif
        }
    }
    /// Anzeigename in der Oberfläche.
    pub fn label(self) -> &'static str {
        match self {
            Family::Serif => "Serif",
            Family::Sans => "Sans",
        }
    }
    /// Name der maßgleichen Word-Schrift (DOCX-Export).
    pub fn word_name(self) -> &'static str {
        match self {
            Family::Serif => "Times New Roman",
            Family::Sans => "Arial",
        }
    }
}

/// Eine Schrift: Familie + Schnitt.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Default)]
pub struct FontKey {
    pub fam: Family,
    pub bold: bool,
    pub italic: bool,
}

impl FontKey {
    fn idx(self) -> usize {
        (if self.fam == Family::Sans { 4 } else { 0 }) + usize::from(self.bold) * 2 + usize::from(self.italic)
    }
    /// Rohdaten der Schrift (für die Oberfläche, die dieselbe Datei laden muss).
    pub fn data(self) -> &'static [u8] {
        DATA[self.idx()]
    }
    /// Stabiler Name der Schrift (z. B. als egui-Familienname).
    pub fn id(self) -> &'static str {
        NAMES[self.idx()]
    }
    /// Alle acht Schriften.
    pub fn all() -> [FontKey; 8] {
        let mut out = [FontKey::default(); 8];
        for (i, o) in out.iter_mut().enumerate() {
            *o = FontKey { fam: if i >= 4 { Family::Sans } else { Family::Serif }, bold: i & 2 != 0, italic: i & 1 != 0 };
        }
        out
    }
}

const DATA: [&[u8]; 8] = [
    include_bytes!("../../fonts/OwnDrawSerif-Regular.ttf"),
    include_bytes!("../../fonts/OwnDrawSerif-Italic.ttf"),
    include_bytes!("../../fonts/OwnDrawSerif-Bold.ttf"),
    include_bytes!("../../fonts/OwnDrawSerif-BoldItalic.ttf"),
    include_bytes!("../../fonts/OwnDrawSans-Regular.ttf"),
    include_bytes!("../../fonts/OwnDrawSans-Italic.ttf"),
    include_bytes!("../../fonts/OwnDrawSans-Bold.ttf"),
    include_bytes!("../../fonts/OwnDrawSans-BoldItalic.ttf"),
];
const NAMES: [&str; 8] = ["od-serif", "od-serif-i", "od-serif-b", "od-serif-bi", "od-sans", "od-sans-i", "od-sans-b", "od-sans-bi"];

thread_local! {
    static FACES: Vec<Option<Face<'static>>> = DATA.iter().map(|d| Face::parse(d, 0).ok()).collect();
}

fn with_face<R>(k: FontKey, f: impl FnOnce(Option<&Face<'static>>) -> R) -> R {
    FACES.with(|v| f(v.get(k.idx()).and_then(Option::as_ref)))
}

/// Hat die Schrift ein eigenes Zeichen für `ch`?
pub fn has(k: FontKey, ch: char) -> bool {
    with_face(k, |f| f.and_then(|f| f.glyph_index(ch)).is_some())
}

/// Vorschub eines Zeichens in pt bei Größe `size`.
pub fn advance(k: FontKey, ch: char, size: f32) -> f32 {
    with_face(k, |f| {
        if let Some(f) = f {
            if let Some(g) = f.glyph_index(ch) {
                return f.glyph_hor_advance(g).unwrap_or(0) as f32 * size / f.units_per_em() as f32;
            }
        }
        if ch.is_control() {
            0.0
        } else {
            crate::text::measure(&ch.to_string(), size)
        }
    })
}

/// (Oberlänge, Unterlänge) in pt, beide positiv.
pub fn metrics(k: FontKey, size: f32) -> (f32, f32) {
    with_face(k, |f| match f {
        Some(f) => {
            let s = size / f.units_per_em() as f32;
            (f.ascender() as f32 * s, -(f.descender() as f32) * s)
        }
        None => (size * 0.9, size * 0.22),
    })
}

struct Sink<'a> {
    pb: &'a mut PathBuilder,
    k: f32,
    ox: f32,
    oy: f32,
}

impl OutlineBuilder for Sink<'_> {
    fn move_to(&mut self, x: f32, y: f32) {
        self.pb.move_to(self.ox + x * self.k, self.oy - y * self.k);
    }
    fn line_to(&mut self, x: f32, y: f32) {
        self.pb.line_to(self.ox + x * self.k, self.oy - y * self.k);
    }
    fn quad_to(&mut self, x1: f32, y1: f32, x: f32, y: f32) {
        self.pb.quad_to(self.ox + x1 * self.k, self.oy - y1 * self.k, self.ox + x * self.k, self.oy - y * self.k);
    }
    fn curve_to(&mut self, x1: f32, y1: f32, x2: f32, y2: f32, x: f32, y: f32) {
        self.pb.cubic_to(self.ox + x1 * self.k, self.oy - y1 * self.k, self.ox + x2 * self.k, self.oy - y2 * self.k, self.ox + x * self.k, self.oy - y * self.k);
    }
    fn close(&mut self) {
        self.pb.close();
    }
}

/// Hängt den Umriss von `ch` an (Grundlinie bei `y`).
pub fn outline(pb: &mut PathBuilder, k: FontKey, ch: char, x: f32, y: f32, size: f32) {
    let done = with_face(k, |f| {
        let Some(f) = f else { return false };
        let Some(g) = f.glyph_index(ch) else { return false };
        let mut sink = Sink { pb, k: size / f.units_per_em() as f32, ox: x, oy: y };
        let _ = f.outline_glyph(GlyphId(g.0), &mut sink);
        true
    });
    if !done && !ch.is_whitespace() && !ch.is_control() {
        crate::text::append_text(pb, &ch.to_string(), x, y, size);
    }
}
