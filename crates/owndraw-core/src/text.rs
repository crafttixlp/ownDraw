//! Textmessung, Zeilenumbruch und Glyphen-Umrisse (eingebettete Schrift Ubuntu Light, Emoji-Ausweichschrift).

use crate::canvas::{Canvas, Paint, Style};
use tiny_skia::PathBuilder;
use ttf_parser::{Face, GlyphId, OutlineBuilder};

thread_local! {
    static FACES: Vec<Face<'static>> = {
        let mut v = Vec::new();
        for data in [epaint_default_fonts::UBUNTU_LIGHT, epaint_default_fonts::NOTO_EMOJI_REGULAR] {
            if let Ok(f) = Face::parse(data, 0) {
                v.push(f);
            }
        }
        v
    };
}

/// Rohdaten der UI-/Textschrift (fuer Oberflaechen, die dieselbe Schrift nutzen wollen).
pub fn font_data() -> &'static [u8] {
    epaint_default_fonts::UBUNTU_LIGHT
}

fn glyph(faces: &[Face<'static>], ch: char) -> Option<(usize, GlyphId)> {
    for (i, f) in faces.iter().enumerate() {
        if let Some(g) = f.glyph_index(ch) {
            return Some((i, g));
        }
    }
    None
}

fn advance(faces: &[Face<'static>], ch: char, size: f32) -> f32 {
    if ch == '\t' {
        return advance(faces, ' ', size) * 4.0;
    }
    match glyph(faces, ch) {
        Some((i, g)) => {
            let f = &faces[i];
            f.glyph_hor_advance(g).unwrap_or(0) as f32 * size / f.units_per_em() as f32
        }
        None => {
            if ch.is_control() {
                0.0
            } else {
                size * 0.5
            }
        }
    }
}

/// Breite einer Textzeile bei Schriftgroesse size.
pub fn measure(s: &str, size: f32) -> f32 {
    FACES.with(|faces| s.chars().map(|c| advance(faces, c, size)).sum())
}

/// (Oberlaenge, Unterlaenge) als positive Werte bei Schriftgroesse size.
pub fn metrics(size: f32) -> (f32, f32) {
    FACES.with(|faces| match faces.first() {
        Some(f) => {
            let k = size / f.units_per_em() as f32;
            (f.ascender() as f32 * k, -(f.descender() as f32) * k)
        }
        None => (size * 0.93, size * 0.24),
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
        self.pb.cubic_to(
            self.ox + x1 * self.k,
            self.oy - y1 * self.k,
            self.ox + x2 * self.k,
            self.oy - y2 * self.k,
            self.ox + x * self.k,
            self.oy - y * self.k,
        );
    }
    fn close(&mut self) {
        self.pb.close();
    }
}

/// Haengt die Umrisse von s (Grundlinie bei y) an den Pfad an.
pub fn append_text(pb: &mut PathBuilder, s: &str, x: f32, y: f32, size: f32) {
    FACES.with(|faces| {
        let mut pen = x;
        for ch in s.chars() {
            if let Some((i, g)) = glyph(faces, ch) {
                if ch != '\t' {
                    let f = &faces[i];
                    let mut sink = Sink { pb, k: size / f.units_per_em() as f32, ox: pen, oy: y };
                    let _ = f.outline_glyph(g, &mut sink);
                }
            }
            pen += advance(faces, ch, size);
        }
    });
}

/// Mehrzeiliger Text mit Wortumbruch (Gegenstueck zu android.text.StaticLayout).
#[derive(Clone, Debug)]
pub struct TextLayout {
    pub lines: Vec<String>,
    pub size: f32,
    pub line_h: f32,
    pub ascent: f32,
}

impl TextLayout {
    pub fn new(text: &str, size: f32, max_w: f32, spacing_mult: f32) -> Self {
        let (asc, desc) = metrics(size);
        let mut lines = Vec::new();
        for para in text.split('\n') {
            wrap(para, size, max_w, &mut lines);
        }
        if lines.is_empty() {
            lines.push(String::new());
        }
        TextLayout { lines, size, line_h: (asc + desc) * spacing_mult, ascent: asc }
    }

    pub fn height(&self) -> f32 {
        self.lines.len() as f32 * self.line_h
    }

    /// Zeichnet mit der linken oberen Ecke im Ursprung.
    pub fn draw(&self, c: &mut Canvas, color: u32) {
        let mut pb = PathBuilder::new();
        for (i, l) in self.lines.iter().enumerate() {
            append_text(&mut pb, l, 0.0, i as f32 * self.line_h + self.ascent, self.size);
        }
        if let Some(path) = pb.finish() {
            let p = Paint { color, style: Style::Fill, ..Paint::default() };
            c.draw_path(&path, &p);
        }
    }
}

fn wrap(para: &str, size: f32, max_w: f32, out: &mut Vec<String>) {
    FACES.with(|faces| {
        let mut line = String::new();
        let mut line_w = 0.0f32;
        // letzte Umbruchstelle (Byte-Position hinter dem Leerzeichen) und Breite bis dorthin
        let mut brk: Option<(usize, f32)> = None;
        for ch in para.chars() {
            let w = advance(faces, ch, size);
            if line_w + w > max_w && !line.is_empty() && ch != ' ' {
                if let Some((pos, bw)) = brk {
                    let rest = line.split_off(pos);
                    out.push(line.trim_end().to_string());
                    line = rest;
                    line_w -= bw;
                } else {
                    out.push(std::mem::take(&mut line));
                    line_w = 0.0;
                }
                brk = None;
            }
            line.push(ch);
            line_w += w;
            if ch == ' ' {
                brk = Some((line.len(), line_w));
            }
        }
        out.push(line);
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn wraps_and_measures() {
        assert!(measure("Hallo", 40.0) > 40.0);
        let l = TextLayout::new("eins zwei drei vier fuenf sechs", 40.0, 200.0, 1.15);
        assert!(l.lines.len() >= 3);
        for s in &l.lines {
            assert!(measure(s, 40.0) <= 200.0 + 0.1, "{s}");
        }
        assert_eq!(TextLayout::new("a\n\nb", 40.0, 500.0, 1.0).lines.len(), 3);
    }
}
