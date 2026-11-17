//! Office-Dokumente: PowerPoint (PPTX, eine Folie je Seite) und Word (DOCX, ein Bild je Seite) sowie
//! eine einzelne HTML-Datei mit eingebetteten Bildern.

use std::io::{Cursor, Write};
use zip::write::SimpleFileOptions;

/// Eine Seite als PNG mit Größe in Punkt.
pub struct PageImg {
    pub png: Vec<u8>,
    pub w: f32,
    pub h: f32,
}

const EMU_PT: f64 = 12700.0;
const NS_R: &str = "http://schemas.openxmlformats.org/officeDocument/2006/relationships";
const NS_A: &str = "http://schemas.openxmlformats.org/drawingml/2006/main";
const NS_P: &str = "http://schemas.openxmlformats.org/presentationml/2006/main";
const NS_W: &str = "http://schemas.openxmlformats.org/wordprocessingml/2006/main";
const NS_PKG_REL: &str = "http://schemas.openxmlformats.org/package/2006/relationships";
const NS_CT: &str = "http://schemas.openxmlformats.org/package/2006/content-types";
const REL: &str = "http://schemas.openxmlformats.org/officeDocument/2006/relationships";
const XML: &str = "<?xml version=\"1.0\" encoding=\"UTF-8\" standalone=\"yes\"?>\n";

fn esc(s: &str) -> String {
    let mut o = String::with_capacity(s.len());
    for c in s.chars() {
        match c {
            '&' => o.push_str("&amp;"),
            '<' => o.push_str("&lt;"),
            '>' => o.push_str("&gt;"),
            '"' => o.push_str("&quot;"),
            '\'' => o.push_str("&apos;"),
            c if (c as u32) < 0x20 && !matches!(c, '\t' | '\n' | '\r') => {}
            c => o.push(c),
        }
    }
    o
}

fn sane(v: f32) -> f64 {
    if v.is_finite() && v > 0.0 { v as f64 } else { 595.0 }
}

/// Einpassen von (w,h) in (bw,bh), zentriert: (x, y, breite, höhe).
fn fit(w: f64, h: f64, bw: i64, bh: i64) -> (i64, i64, i64, i64) {
    let k = (bw as f64 / w).min(bh as f64 / h);
    let (fw, fh) = (((w * k).round() as i64).max(1), ((h * k).round() as i64).max(1));
    ((bw - fw) / 2, (bh - fh) / 2, fw, fh)
}

struct Pkg {
    z: zip::ZipWriter<Cursor<Vec<u8>>>,
}

impl Pkg {
    fn new() -> Self {
        Pkg { z: zip::ZipWriter::new(Cursor::new(Vec::new())) }
    }
    fn add(&mut self, name: &str, data: &[u8]) -> Result<(), String> {
        let o = SimpleFileOptions::default().compression_method(zip::CompressionMethod::Deflated);
        self.z.start_file(name, o).map_err(|e| e.to_string())?;
        self.z.write_all(data).map_err(|e| e.to_string())
    }
    fn xml(&mut self, name: &str, body: &str) -> Result<(), String> {
        self.add(name, format!("{XML}{body}").as_bytes())
    }
    fn finish(self) -> Result<Vec<u8>, String> {
        Ok(self.z.finish().map_err(|e| e.to_string())?.into_inner())
    }
}

fn core_xml(title: &str) -> String {
    format!(
        "<cp:coreProperties xmlns:cp=\"http://schemas.openxmlformats.org/package/2006/metadata/core-properties\" \
xmlns:dc=\"http://purl.org/dc/elements/1.1/\" xmlns:dcterms=\"http://purl.org/dc/terms/\" \
xmlns:dcmitype=\"http://purl.org/dc/dcmitype/\" xmlns:xsi=\"http://www.w3.org/2001/XMLSchema-instance\">\
<dc:title>{}</dc:title><dc:creator>ownDraw</dc:creator></cp:coreProperties>",
        esc(title)
    )
}

const THEME: &str = r#"<a:theme xmlns:a="http://schemas.openxmlformats.org/drawingml/2006/main" name="Office"><a:themeElements><a:clrScheme name="Office"><a:dk1><a:sysClr val="windowText" lastClr="000000"/></a:dk1><a:lt1><a:sysClr val="window" lastClr="FFFFFF"/></a:lt1><a:dk2><a:srgbClr val="44546A"/></a:dk2><a:lt2><a:srgbClr val="E7E6E6"/></a:lt2><a:accent1><a:srgbClr val="4472C4"/></a:accent1><a:accent2><a:srgbClr val="ED7D31"/></a:accent2><a:accent3><a:srgbClr val="A5A5A5"/></a:accent3><a:accent4><a:srgbClr val="FFC000"/></a:accent4><a:accent5><a:srgbClr val="5B9BD5"/></a:accent5><a:accent6><a:srgbClr val="70AD47"/></a:accent6><a:hlink><a:srgbClr val="0563C1"/></a:hlink><a:folHlink><a:srgbClr val="954F72"/></a:folHlink></a:clrScheme><a:fontScheme name="Office"><a:majorFont><a:latin typeface="Calibri"/><a:ea typeface=""/><a:cs typeface=""/></a:majorFont><a:minorFont><a:latin typeface="Calibri"/><a:ea typeface=""/><a:cs typeface=""/></a:minorFont></a:fontScheme><a:fmtScheme name="Office"><a:fillStyleLst><a:solidFill><a:schemeClr val="phClr"/></a:solidFill><a:solidFill><a:schemeClr val="phClr"/></a:solidFill><a:solidFill><a:schemeClr val="phClr"/></a:solidFill></a:fillStyleLst><a:lnStyleLst><a:ln w="6350" cap="flat" cmpd="sng" algn="ctr"><a:solidFill><a:schemeClr val="phClr"/></a:solidFill><a:prstDash val="solid"/></a:ln><a:ln w="12700" cap="flat" cmpd="sng" algn="ctr"><a:solidFill><a:schemeClr val="phClr"/></a:solidFill><a:prstDash val="solid"/></a:ln><a:ln w="19050" cap="flat" cmpd="sng" algn="ctr"><a:solidFill><a:schemeClr val="phClr"/></a:solidFill><a:prstDash val="solid"/></a:ln></a:lnStyleLst><a:effectStyleLst><a:effectStyle><a:effectLst/></a:effectStyle><a:effectStyle><a:effectLst/></a:effectStyle><a:effectStyle><a:effectLst/></a:effectStyle></a:effectStyleLst><a:bgFillStyleLst><a:solidFill><a:schemeClr val="phClr"/></a:solidFill><a:solidFill><a:schemeClr val="phClr"/></a:solidFill><a:solidFill><a:schemeClr val="phClr"/></a:solidFill></a:bgFillStyleLst></a:fmtScheme></a:themeElements></a:theme>"#;

fn rels(items: &[(String, &str, String)]) -> String {
    let mut s = format!("<Relationships xmlns=\"{NS_PKG_REL}\">");
    for (id, ty, target) in items {
        s.push_str(&format!("<Relationship Id=\"{id}\" Type=\"{REL}/{ty}\" Target=\"{target}\"/>"));
    }
    s.push_str("</Relationships>");
    s
}

pub fn pptx(pages: &[PageImg], title: &str) -> Result<Vec<u8>, String> {
    if pages.is_empty() {
        return Err("keine Seiten".into());
    }
    // Foliengröße = erste Seite in EMU, auf erlaubten Bereich skaliert
    let (w0, h0) = (sane(pages[0].w) * EMU_PT, sane(pages[0].h) * EMU_PT);
    let mut k = 1.0f64;
    let mn = w0.min(h0);
    if mn < 914400.0 {
        k = 914400.0 / mn;
    }
    if (w0.max(h0) * k) > 51206400.0 {
        k = 51206400.0 / w0.max(h0);
    }
    let (sw, sh) = (((w0 * k).round() as i64).clamp(914400, 51206400), ((h0 * k).round() as i64).clamp(914400, 51206400));
    let n = pages.len();
    let mut z = Pkg::new();

    let mut ct = format!(
        "<Types xmlns=\"{NS_CT}\"><Default Extension=\"rels\" ContentType=\"application/vnd.openxmlformats-package.relationships+xml\"/>\
<Default Extension=\"xml\" ContentType=\"application/xml\"/><Default Extension=\"png\" ContentType=\"image/png\"/>\
<Override PartName=\"/ppt/presentation.xml\" ContentType=\"application/vnd.openxmlformats-officedocument.presentationml.presentation.main+xml\"/>\
<Override PartName=\"/ppt/slideMasters/slideMaster1.xml\" ContentType=\"application/vnd.openxmlformats-officedocument.presentationml.slideMaster+xml\"/>\
<Override PartName=\"/ppt/slideLayouts/slideLayout1.xml\" ContentType=\"application/vnd.openxmlformats-officedocument.presentationml.slideLayout+xml\"/>\
<Override PartName=\"/ppt/theme/theme1.xml\" ContentType=\"application/vnd.openxmlformats-officedocument.theme+xml\"/>\
<Override PartName=\"/docProps/core.xml\" ContentType=\"application/vnd.openxmlformats-package.core-properties+xml\"/>\
<Override PartName=\"/docProps/app.xml\" ContentType=\"application/vnd.openxmlformats-officedocument.extended-properties+xml\"/>"
    );
    for i in 1..=n {
        ct.push_str(&format!(
            "<Override PartName=\"/ppt/slides/slide{i}.xml\" ContentType=\"application/vnd.openxmlformats-officedocument.presentationml.slide+xml\"/>"
        ));
    }
    ct.push_str("</Types>");
    z.xml("[Content_Types].xml", &ct)?;
    z.xml(
        "_rels/.rels",
        &rels(&[
            ("rId1".into(), "officeDocument", "ppt/presentation.xml".into()),
            ("rId2".into(), "extended-properties", "docProps/app.xml".into()),
        ])
        .replace("</Relationships>", "<Relationship Id=\"rId3\" Type=\"http://schemas.openxmlformats.org/package/2006/relationships/metadata/core-properties\" Target=\"docProps/core.xml\"/></Relationships>")
        ,
    )?;
    z.xml("docProps/core.xml", &core_xml(title))?;
    z.xml(
        "docProps/app.xml",
        &format!(
            "<Properties xmlns=\"http://schemas.openxmlformats.org/officeDocument/2006/extended-properties\"><Application>ownDraw</Application><Slides>{n}</Slides></Properties>"
        ),
    )?;

    let mut sld_ids = String::new();
    let mut prels = vec![("rId1".to_string(), "slideMaster", "slideMasters/slideMaster1.xml".to_string())];
    for i in 1..=n {
        sld_ids.push_str(&format!("<p:sldId id=\"{}\" r:id=\"rId{}\"/>", 255 + i, i + 1));
        prels.push((format!("rId{}", i + 1), "slide", format!("slides/slide{i}.xml")));
    }
    prels.push((format!("rId{}", n + 2), "theme", "theme/theme1.xml".into()));
    z.xml(
        "ppt/presentation.xml",
        &format!(
            "<p:presentation xmlns:a=\"{NS_A}\" xmlns:r=\"{NS_R}\" xmlns:p=\"{NS_P}\" saveSubsetFonts=\"1\">\
<p:sldMasterIdLst><p:sldMasterId id=\"2147483648\" r:id=\"rId1\"/></p:sldMasterIdLst><p:sldIdLst>{sld_ids}</p:sldIdLst>\
<p:sldSz cx=\"{sw}\" cy=\"{sh}\"/><p:notesSz cx=\"6858000\" cy=\"9144000\"/></p:presentation>"
        ),
    )?;
    z.xml("ppt/_rels/presentation.xml.rels", &rels(&prels))?;

    let grp = "<p:nvGrpSpPr><p:cNvPr id=\"1\" name=\"\"/><p:cNvGrpSpPr/><p:nvPr/></p:nvGrpSpPr><p:grpSpPr><a:xfrm><a:off x=\"0\" y=\"0\"/><a:ext cx=\"0\" cy=\"0\"/><a:chOff x=\"0\" y=\"0\"/><a:chExt cx=\"0\" cy=\"0\"/></a:xfrm></p:grpSpPr>";
    z.xml(
        "ppt/slideMasters/slideMaster1.xml",
        &format!(
            "<p:sldMaster xmlns:a=\"{NS_A}\" xmlns:r=\"{NS_R}\" xmlns:p=\"{NS_P}\"><p:cSld><p:bg><p:bgRef idx=\"1001\"><a:schemeClr val=\"bg1\"/></p:bgRef></p:bg><p:spTree>{grp}</p:spTree></p:cSld>\
<p:clrMap bg1=\"lt1\" tx1=\"dk1\" bg2=\"lt2\" tx2=\"dk2\" accent1=\"accent1\" accent2=\"accent2\" accent3=\"accent3\" accent4=\"accent4\" accent5=\"accent5\" accent6=\"accent6\" hlink=\"hlink\" folHlink=\"folHlink\"/>\
<p:sldLayoutIdLst><p:sldLayoutId id=\"2147483649\" r:id=\"rId1\"/></p:sldLayoutIdLst></p:sldMaster>"
        ),
    )?;
    z.xml(
        "ppt/slideMasters/_rels/slideMaster1.xml.rels",
        &rels(&[
            ("rId1".into(), "slideLayout", "../slideLayouts/slideLayout1.xml".into()),
            ("rId2".into(), "theme", "../theme/theme1.xml".into()),
        ]),
    )?;
    z.xml(
        "ppt/slideLayouts/slideLayout1.xml",
        &format!(
            "<p:sldLayout xmlns:a=\"{NS_A}\" xmlns:r=\"{NS_R}\" xmlns:p=\"{NS_P}\" type=\"blank\" preserve=\"1\"><p:cSld name=\"Leer\"><p:spTree>{grp}</p:spTree></p:cSld><p:clrMapOvr><a:masterClrMapping/></p:clrMapOvr></p:sldLayout>"
        ),
    )?;
    z.xml(
        "ppt/slideLayouts/_rels/slideLayout1.xml.rels",
        &rels(&[("rId1".into(), "slideMaster", "../slideMasters/slideMaster1.xml".into())]),
    )?;
    z.xml("ppt/theme/theme1.xml", THEME)?;

    for (i, p) in pages.iter().enumerate() {
        let i = i + 1;
        let (x, y, w, h) = fit(sane(p.w), sane(p.h), sw, sh);
        z.xml(
            &format!("ppt/slides/slide{i}.xml"),
            &format!(
                "<p:sld xmlns:a=\"{NS_A}\" xmlns:r=\"{NS_R}\" xmlns:p=\"{NS_P}\"><p:cSld><p:spTree>{grp}\
<p:pic><p:nvPicPr><p:cNvPr id=\"2\" name=\"Seite {i}\" descr=\"Seite {i}\"/><p:cNvPicPr><a:picLocks noChangeAspect=\"1\"/></p:cNvPicPr><p:nvPr/></p:nvPicPr>\
<p:blipFill><a:blip r:embed=\"rId2\"/><a:stretch><a:fillRect/></a:stretch></p:blipFill>\
<p:spPr><a:xfrm><a:off x=\"{x}\" y=\"{y}\"/><a:ext cx=\"{w}\" cy=\"{h}\"/></a:xfrm><a:prstGeom prst=\"rect\"><a:avLst/></a:prstGeom></p:spPr></p:pic>\
</p:spTree></p:cSld><p:clrMapOvr><a:masterClrMapping/></p:clrMapOvr></p:sld>"
            ),
        )?;
        z.xml(
            &format!("ppt/slides/_rels/slide{i}.xml.rels"),
            &rels(&[
                ("rId1".into(), "slideLayout", "../slideLayouts/slideLayout1.xml".into()),
                ("rId2".into(), "image", format!("../media/image{i}.png")),
            ]),
        )?;
        z.add(&format!("ppt/media/image{i}.png"), &p.png)?;
    }
    z.finish()
}

pub fn docx(pages: &[PageImg], title: &str) -> Result<Vec<u8>, String> {
    if pages.is_empty() {
        return Err("keine Seiten".into());
    }
    let (pw, ph) = (sane(pages[0].w), sane(pages[0].h));
    // Seitengröße in Twips (Word erlaubt 1 Zoll min bis 22 Zoll max sinnvoll; hier nur Untergrenze)
    let (tw, th) = (((pw * 20.0).round() as i64).clamp(720, 31680), ((ph * 20.0).round() as i64).clamp(720, 31680));
    // Bildfläche in EMU (1 Twip = 635 EMU), minimal kleiner als die Seite, damit kein Leerseitenüberlauf entsteht
    let (bw, bh) = ((tw * 635 - 12700).max(635), (th * 635 - 25400).max(635));
    let mut z = Pkg::new();
    z.xml(
        "[Content_Types].xml",
        &format!(
            "<Types xmlns=\"{NS_CT}\"><Default Extension=\"rels\" ContentType=\"application/vnd.openxmlformats-package.relationships+xml\"/>\
<Default Extension=\"xml\" ContentType=\"application/xml\"/><Default Extension=\"png\" ContentType=\"image/png\"/>\
<Override PartName=\"/word/document.xml\" ContentType=\"application/vnd.openxmlformats-officedocument.wordprocessingml.document.main+xml\"/>\
<Override PartName=\"/word/styles.xml\" ContentType=\"application/vnd.openxmlformats-officedocument.wordprocessingml.styles+xml\"/>\
<Override PartName=\"/docProps/core.xml\" ContentType=\"application/vnd.openxmlformats-package.core-properties+xml\"/></Types>"
        ),
    )?;
    z.xml(
        "_rels/.rels",
        &rels(&[("rId1".into(), "officeDocument", "word/document.xml".into())]).replace(
            "</Relationships>",
            "<Relationship Id=\"rId2\" Type=\"http://schemas.openxmlformats.org/package/2006/relationships/metadata/core-properties\" Target=\"docProps/core.xml\"/></Relationships>",
        ),
    )?;
    z.xml("docProps/core.xml", &core_xml(title))?;
    z.xml(
        "word/styles.xml",
        &format!(
            "<w:styles xmlns:w=\"{NS_W}\"><w:docDefaults><w:rPrDefault><w:rPr><w:sz w:val=\"22\"/></w:rPr></w:rPrDefault>\
<w:pPrDefault><w:pPr><w:spacing w:after=\"0\" w:line=\"240\" w:lineRule=\"auto\"/></w:pPr></w:pPrDefault></w:docDefaults>\
<w:style w:type=\"paragraph\" w:default=\"1\" w:styleId=\"Standard\"><w:name w:val=\"Normal\"/><w:qFormat/></w:style></w:styles>"
        ),
    )?;
    let mut body = String::new();
    let mut drels = vec![("rId1".to_string(), "styles", "styles.xml".to_string())];
    for (i, p) in pages.iter().enumerate() {
        let i = i + 1;
        let (_, _, w, h) = fit(sane(p.w), sane(p.h), bw, bh);
        let br = if i > 1 { "<w:pageBreakBefore/>" } else { "" };
        body.push_str(&format!(
            "<w:p><w:pPr>{br}<w:jc w:val=\"center\"/><w:rPr><w:sz w:val=\"2\"/></w:rPr></w:pPr><w:r><w:drawing>\
<wp:inline distT=\"0\" distB=\"0\" distL=\"0\" distR=\"0\"><wp:extent cx=\"{w}\" cy=\"{h}\"/><wp:docPr id=\"{i}\" name=\"Seite {i}\" descr=\"Seite {i}\"/>\
<wp:cNvGraphicFramePr><a:graphicFrameLocks noChangeAspect=\"1\"/></wp:cNvGraphicFramePr>\
<a:graphic><a:graphicData uri=\"http://schemas.openxmlformats.org/drawingml/2006/picture\">\
<pic:pic><pic:nvPicPr><pic:cNvPr id=\"{i}\" name=\"image{i}.png\"/><pic:cNvPicPr/></pic:nvPicPr>\
<pic:blipFill><a:blip r:embed=\"rId{rid}\"/><a:stretch><a:fillRect/></a:stretch></pic:blipFill>\
<pic:spPr><a:xfrm><a:off x=\"0\" y=\"0\"/><a:ext cx=\"{w}\" cy=\"{h}\"/></a:xfrm><a:prstGeom prst=\"rect\"><a:avLst/></a:prstGeom></pic:spPr></pic:pic>\
</a:graphicData></a:graphic></wp:inline></w:drawing></w:r></w:p>",
            rid = i + 1
        ));
        drels.push((format!("rId{}", i + 1), "image", format!("media/image{i}.png")));
        z.add(&format!("word/media/image{i}.png"), &p.png)?;
    }
    z.xml(
        "word/document.xml",
        &format!(
            "<w:document xmlns:w=\"{NS_W}\" xmlns:r=\"{NS_R}\" xmlns:a=\"{NS_A}\" \
xmlns:pic=\"http://schemas.openxmlformats.org/drawingml/2006/picture\" \
xmlns:wp=\"http://schemas.openxmlformats.org/drawingml/2006/wordprocessingDrawing\"><w:body>{body}\
<w:sectPr><w:pgSz w:w=\"{tw}\" w:h=\"{th}\"/><w:pgMar w:top=\"0\" w:right=\"0\" w:bottom=\"0\" w:left=\"0\" w:header=\"0\" w:footer=\"0\" w:gutter=\"0\"/></w:sectPr></w:body></w:document>"
        ),
    )?;
    z.xml("word/_rels/document.xml.rels", &rels(&drels))?;
    z.finish()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Read;

    fn pages() -> Vec<PageImg> {
        let mut pm = tiny_skia::Pixmap::new(20, 10).unwrap();
        pm.fill(tiny_skia::Color::from_rgba8(200, 30, 30, 255));
        let png = pm.encode_png().unwrap();
        vec![PageImg { png: png.clone(), w: 842.0, h: 595.0 }, PageImg { png, w: 300.0, h: 400.0 }]
    }

    fn check(bytes: Vec<u8>, must: &[&str]) {
        let mut z = zip::ZipArchive::new(Cursor::new(bytes)).unwrap();
        for m in must {
            assert!(z.by_name(m).is_ok(), "fehlt: {m}");
        }
        for i in 0..z.len() {
            let mut f = z.by_index(i).unwrap();
            let name = f.name().unwrap().to_string();
            if name.ends_with(".xml") || name.ends_with(".rels") {
                let mut s = String::new();
                f.read_to_string(&mut s).unwrap();
                roxmltree::Document::parse(&s).unwrap_or_else(|e| panic!("{name}: {e}"));
            }
        }
    }

    #[test]
    fn pptx_paket() {
        let b = pptx(&pages(), "A & <B>").unwrap();
        check(
            b.clone(),
            &["[Content_Types].xml", "_rels/.rels", "docProps/core.xml", "docProps/app.xml", "ppt/presentation.xml",
              "ppt/_rels/presentation.xml.rels", "ppt/slideMasters/slideMaster1.xml", "ppt/slideLayouts/slideLayout1.xml",
              "ppt/theme/theme1.xml", "ppt/slides/slide1.xml", "ppt/slides/slide2.xml", "ppt/media/image2.png"],
        );
        let mut z = zip::ZipArchive::new(Cursor::new(b)).unwrap();
        let mut s = String::new();
        z.by_name("docProps/core.xml").unwrap().read_to_string(&mut s).unwrap();
        assert!(s.contains("A &amp; &lt;B&gt;"));
        s.clear();
        z.by_name("ppt/presentation.xml").unwrap().read_to_string(&mut s).unwrap();
        assert!(s.contains("cx=\"10693400\" cy=\"7556500\""));
    }

    #[test]
    fn pptx_groesse_begrenzt() {
        let mut p = pages();
        p[0].w = 10.0;
        p[0].h = 5.0;
        let b = pptx(&p, "t").unwrap();
        let mut z = zip::ZipArchive::new(Cursor::new(b)).unwrap();
        let mut s = String::new();
        z.by_name("ppt/presentation.xml").unwrap().read_to_string(&mut s).unwrap();
        assert!(s.contains("cy=\"914400\""));
    }

    #[test]
    fn docx_paket() {
        let b = docx(&pages(), "Titel").unwrap();
        check(
            b,
            &["[Content_Types].xml", "_rels/.rels", "docProps/core.xml", "word/document.xml", "word/_rels/document.xml.rels",
              "word/styles.xml", "word/media/image1.png", "word/media/image2.png"],
        );
    }

    #[test]
    fn leer_ist_fehler() {
        assert!(pptx(&[], "x").is_err());
        assert!(docx(&[], "x").is_err());
    }
}
