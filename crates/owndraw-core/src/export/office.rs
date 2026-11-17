//! Office-Dokumente: PowerPoint (PPTX, eine Folie je Seite) und Word (DOCX, ein Bild je Seite) sowie
//! eine einzelne HTML-Datei mit eingebetteten Bildern.

/// Eine Seite als PNG mit Größe in Punkt.
pub struct PageImg {
    pub png: Vec<u8>,
    pub w: f32,
    pub h: f32,
}

pub fn pptx(pages: &[PageImg], title: &str) -> Result<Vec<u8>, String> {
    let _ = (pages, title);
    Err("noch nicht umgesetzt".into())
}

pub fn docx(pages: &[PageImg], title: &str) -> Result<Vec<u8>, String> {
    let _ = (pages, title);
    Err("noch nicht umgesetzt".into())
}
