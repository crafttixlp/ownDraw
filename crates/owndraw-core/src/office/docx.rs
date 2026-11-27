//! Word-Dateien (DOCX): Export mit Formatvorlagen, Listen, Links, Kopf-/Fußzeile und Seitenzahlen; Import von
//! Absätzen, Überschriften, Listen, Zeichenformat, Links, Seitenumbrüchen und Seitengröße. Tabellen werden beim
//! Import zu Absätzen (Zellen durch Tabulator getrennt), Bilder fehlen noch.

use super::fonts::Family;
use super::model::{Align, CStyle, Kind, PageSpec, Para, Run, TextDoc, VAlign};
use std::collections::HashMap;
use std::io::{Cursor, Read, Write};
use zip::write::SimpleFileOptions;

fn esc(s: &str) -> String {
    let mut o = String::with_capacity(s.len());
    for c in s.chars() {
        match c {
            '&' => o.push_str("&amp;"),
            '<' => o.push_str("&lt;"),
            '>' => o.push_str("&gt;"),
            '"' => o.push_str("&quot;"),
            c if (c as u32) < 0x20 && c != '\t' => {}
            c => o.push(c),
        }
    }
    o
}

fn hex(c: u32) -> String {
    format!("{:06X}", c & 0xFF_FFFF)
}

fn tw(pt: f32) -> i64 {
    (pt * 20.0).round() as i64
}

const W: &str = "http://schemas.openxmlformats.org/wordprocessingml/2006/main";
const R: &str = "http://schemas.openxmlformats.org/officeDocument/2006/relationships";

fn run_xml(r: &Run, out: &mut String) {
    let st = &r.st;
    let mut pr = String::new();
    if let Some(f) = st.fam {
        let n = f.word_name();
        pr.push_str(&format!(r#"<w:rFonts w:ascii="{n}" w:hAnsi="{n}" w:cs="{n}"/>"#));
    }
    if st.b {
        pr.push_str("<w:b/>");
    }
    if st.i {
        pr.push_str("<w:i/>");
    }
    if st.s {
        pr.push_str("<w:strike/>");
    }
    if st.color != 0 {
        pr.push_str(&format!(r#"<w:color w:val="{}"/>"#, hex(st.color)));
    } else if st.link.is_some() {
        pr.push_str(r#"<w:color w:val="1A56DB"/>"#);
    }
    if let Some(sz) = st.size {
        pr.push_str(&format!(r#"<w:sz w:val="{0}"/><w:szCs w:val="{0}"/>"#, (sz * 2.0).round() as i64));
    }
    if st.u || st.link.is_some() {
        pr.push_str(r#"<w:u w:val="single"/>"#);
    }
    if st.hl != 0 {
        pr.push_str(&format!(r#"<w:shd w:val="clear" w:color="auto" w:fill="{}"/>"#, hex(st.hl)));
    }
    match st.va {
        VAlign::Sup => pr.push_str(r#"<w:vertAlign w:val="superscript"/>"#),
        VAlign::Sub => pr.push_str(r#"<w:vertAlign w:val="subscript"/>"#),
        VAlign::Base => {}
    }
    out.push_str("<w:r>");
    if !pr.is_empty() {
        out.push_str("<w:rPr>");
        out.push_str(&pr);
        out.push_str("</w:rPr>");
    }
    for (i, part) in r.text.split('\t').enumerate() {
        if i > 0 {
            out.push_str("<w:tab/>");
        }
        if !part.is_empty() {
            out.push_str(&format!(r#"<w:t xml:space="preserve">{}</w:t>"#, esc(part)));
        }
    }
    out.push_str("</w:r>");
}

fn style_id(k: Kind) -> Option<&'static str> {
    match k {
        Kind::Title => Some("Title"),
        Kind::H1 => Some("Heading1"),
        Kind::H2 => Some("Heading2"),
        Kind::H3 => Some("Heading3"),
        Kind::Quote => Some("Quote"),
        Kind::Bullet | Kind::Number => Some("ListParagraph"),
        _ => None,
    }
}

fn hf_xml(tag: &str, text: &str, center: bool) -> String {
    let mut body = String::new();
    let mut rest = text;
    while !rest.is_empty() {
        let (pos, field, len) = match (rest.find("{seite}"), rest.find("{seiten}")) {
            (Some(a), Some(b)) if b < a => (b, "NUMPAGES", 8),
            (Some(a), _) => (a, "PAGE", 7),
            (None, Some(b)) => (b, "NUMPAGES", 8),
            (None, None) => (rest.len(), "", 0),
        };
        if pos > 0 {
            body.push_str(&format!(r#"<w:r><w:t xml:space="preserve">{}</w:t></w:r>"#, esc(&rest[..pos])));
        }
        if !field.is_empty() {
            body.push_str(&format!(r#"<w:fldSimple w:instr="{field}"><w:r><w:t>1</w:t></w:r></w:fldSimple>"#));
        }
        rest = &rest[(pos + len).min(rest.len())..];
    }
    let jc = if center { r#"<w:pPr><w:jc w:val="center"/></w:pPr>"# } else { "" };
    format!(r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?><w:{tag} xmlns:w="{W}" xmlns:r="{R}"><w:p>{jc}{body}</w:p></w:{tag}>"#)
}

/// Schreibt eine DOCX-Datei.
pub fn export(doc: &TextDoc, title: &str) -> Result<Vec<u8>, String> {
    let mut rels = vec![
        (r#"Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/styles" Target="styles.xml""#.to_string()),
        (r#"Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/numbering" Target="numbering.xml""#.to_string()),
    ];
    let mut body = String::new();
    let mut nums: Vec<(usize, bool)> = Vec::new(); // (numId, Aufzählung?)
    let mut cur_list: Option<usize> = None;
    for p in &doc.paras {
        if p.kind == Kind::PageBreak {
            body.push_str(r#"<w:p><w:r><w:br w:type="page"/></w:r></w:p>"#);
            cur_list = None;
            continue;
        }
        let mut ppr = String::new();
        if let Some(s) = style_id(p.kind) {
            ppr.push_str(&format!(r#"<w:pStyle w:val="{s}"/>"#));
        }
        if p.kind.is_list() {
            let bullet = p.kind == Kind::Bullet;
            let id = match cur_list {
                Some(i) if nums[i].1 == bullet => nums[i].0,
                _ => {
                    nums.push((nums.len() + 1, bullet));
                    cur_list = Some(nums.len() - 1);
                    nums.len()
                }
            };
            ppr.push_str(&format!(r#"<w:numPr><w:ilvl w:val="{}"/><w:numId w:val="{id}"/></w:numPr>"#, p.indent.min(8)));
        } else {
            cur_list = None;
            if p.indent > 0 {
                ppr.push_str(&format!(r#"<w:ind w:left="{}"/>"#, tw(p.indent as f32 * super::layout::INDENT)));
            }
        }
        if p.line > 0.0 {
            ppr.push_str(&format!(r#"<w:spacing w:line="{}" w:lineRule="auto"/>"#, (p.line * 240.0).round() as i64));
        }
        match p.align {
            Align::Center => ppr.push_str(r#"<w:jc w:val="center"/>"#),
            Align::Right => ppr.push_str(r#"<w:jc w:val="right"/>"#),
            Align::Justify => ppr.push_str(r#"<w:jc w:val="both"/>"#),
            Align::Left => {}
        }
        body.push_str("<w:p>");
        if !ppr.is_empty() {
            body.push_str(&format!("<w:pPr>{ppr}</w:pPr>"));
        }
        let mut i = 0;
        while i < p.runs.len() {
            if let Some(l) = &p.runs[i].st.link {
                rels.push(format!(r#"Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/hyperlink" Target="{}" TargetMode="External""#, esc(l)));
                body.push_str(&format!(r#"<w:hyperlink r:id="rId{}">"#, rels.len()));
                while i < p.runs.len() && p.runs[i].st.link.as_ref() == Some(l) {
                    run_xml(&p.runs[i], &mut body);
                    i += 1;
                }
                body.push_str("</w:hyperlink>");
            } else {
                run_xml(&p.runs[i], &mut body);
                i += 1;
            }
        }
        body.push_str("</w:p>");
    }
    let pg = &doc.page;
    let mut sect = String::new();
    let mut parts: Vec<(String, String)> = Vec::new();
    for (tag, text, file, center) in [("hdr", &doc.header, "header1.xml", false), ("ftr", &doc.footer, "footer1.xml", true)] {
        if text.trim().is_empty() {
            continue;
        }
        let kind = if tag == "hdr" { "header" } else { "footer" };
        rels.push(format!(r#"Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/{kind}" Target="{file}""#));
        sect.push_str(&format!(r#"<w:{kind}Reference w:type="default" r:id="rId{}"/>"#, rels.len()));
        parts.push((format!("word/{file}"), hf_xml(tag, text, center)));
    }
    sect.push_str(&format!(
        r#"<w:pgSz w:w="{}" w:h="{}"{}/><w:pgMar w:top="{}" w:right="{}" w:bottom="{}" w:left="{}" w:header="567" w:footer="567" w:gutter="0"/>"#,
        tw(pg.w),
        tw(pg.h),
        if pg.w > pg.h { r#" w:orient="landscape""# } else { "" },
        tw(pg.mt),
        tw(pg.mr),
        tw(pg.mb),
        tw(pg.ml)
    ));
    let document = format!(r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?><w:document xmlns:w="{W}" xmlns:r="{R}"><w:body>{body}<w:sectPr>{sect}</w:sectPr></w:body></w:document>"#);
    let font = doc.family.word_name();
    let hs = |id: &str, name: &str, sz: i64, before: i64, after: i64, lvl: i64| {
        format!(
            r#"<w:style w:type="paragraph" w:styleId="{id}"><w:name w:val="{name}"/><w:basedOn w:val="Normal"/><w:next w:val="Normal"/><w:qFormat/><w:pPr><w:keepNext/><w:spacing w:before="{before}" w:after="{after}"/><w:outlineLvl w:val="{lvl}"/></w:pPr><w:rPr><w:rFonts w:ascii="Arial" w:hAnsi="Arial" w:cs="Arial"/><w:b/><w:sz w:val="{sz}"/><w:szCs w:val="{sz}"/></w:rPr></w:style>"#
        )
    };
    let styles = format!(
        r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?><w:styles xmlns:w="{W}"><w:docDefaults><w:rPrDefault><w:rPr><w:rFonts w:ascii="{font}" w:hAnsi="{font}" w:cs="{font}" w:eastAsia="{font}"/><w:sz w:val="{sz}"/><w:szCs w:val="{sz}"/><w:lang w:val="de-AT"/></w:rPr></w:rPrDefault><w:pPrDefault><w:pPr><w:spacing w:after="160" w:line="276" w:lineRule="auto"/></w:pPr></w:pPrDefault></w:docDefaults><w:style w:type="paragraph" w:default="1" w:styleId="Normal"><w:name w:val="Normal"/><w:qFormat/></w:style>{t}{h1}{h2}{h3}<w:style w:type="paragraph" w:styleId="Quote"><w:name w:val="Quote"/><w:basedOn w:val="Normal"/><w:qFormat/><w:pPr><w:ind w:left="360"/></w:pPr><w:rPr><w:i/><w:color w:val="4A4A55"/></w:rPr></w:style><w:style w:type="paragraph" w:styleId="ListParagraph"><w:name w:val="List Paragraph"/><w:basedOn w:val="Normal"/><w:qFormat/><w:pPr><w:spacing w:after="60"/><w:contextualSpacing/></w:pPr></w:style></w:styles>"#,
        sz = (doc.size * 2.0).round() as i64,
        t = hs("Title", "Title", 52, 0, 280, 0).replace(r#"<w:outlineLvl w:val="0"/>"#, ""),
        h1 = hs("Heading1", "heading 1", 36, 360, 120, 0),
        h2 = hs("Heading2", "heading 2", 28, 280, 80, 1),
        h3 = hs("Heading3", "heading 3", 24, 200, 60, 2),
    );
    let mut numbering = format!(r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?><w:numbering xmlns:w="{W}">"#);
    for (aid, bullet) in [(0, true), (1, false)] {
        numbering.push_str(&format!(r#"<w:abstractNum w:abstractNumId="{aid}"><w:multiLevelType w:val="hybridMultilevel"/>"#));
        for l in 0..9 {
            let (fmt, txt) = if bullet {
                ("bullet", ["•", "◦", "▪"][l % 3].to_string())
            } else {
                (["decimal", "lowerLetter", "lowerRoman"][l % 3], format!("%{}{}", l + 1, if l % 3 == 1 { ")" } else { "." }))
            };
            numbering.push_str(&format!(
                r#"<w:lvl w:ilvl="{l}"><w:start w:val="1"/><w:numFmt w:val="{fmt}"/><w:lvlText w:val="{txt}"/><w:lvlJc w:val="left"/><w:pPr><w:ind w:left="{}" w:hanging="360"/></w:pPr></w:lvl>"#,
                400 + l * 480
            ));
        }
        numbering.push_str("</w:abstractNum>");
    }
    for (id, bullet) in &nums {
        numbering.push_str(&format!(r#"<w:num w:numId="{id}"><w:abstractNumId w:val="{}"/><w:lvlOverride w:ilvl="0"><w:startOverride w:val="1"/></w:lvlOverride></w:num>"#, if *bullet { 0 } else { 1 }));
    }
    numbering.push_str("</w:numbering>");
    let rels_xml = format!(
        r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?><Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships">{}</Relationships>"#,
        rels.iter().enumerate().map(|(i, r)| format!(r#"<Relationship Id="rId{}" {r}/>"#, i + 1)).collect::<String>()
    );
    let mut ct = String::from(r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?><Types xmlns="http://schemas.openxmlformats.org/package/2006/content-types"><Default Extension="rels" ContentType="application/vnd.openxmlformats-package.relationships+xml"/><Default Extension="xml" ContentType="application/xml"/><Override PartName="/word/document.xml" ContentType="application/vnd.openxmlformats-officedocument.wordprocessingml.document.main+xml"/><Override PartName="/word/styles.xml" ContentType="application/vnd.openxmlformats-officedocument.wordprocessingml.styles+xml"/><Override PartName="/word/numbering.xml" ContentType="application/vnd.openxmlformats-officedocument.wordprocessingml.numbering+xml"/><Override PartName="/docProps/core.xml" ContentType="application/vnd.openxmlformats-package.core-properties+xml"/>"#);
    for (n, _) in &parts {
        let kind = if n.contains("header") { "header" } else { "footer" };
        ct.push_str(&format!(r#"<Override PartName="/{n}" ContentType="application/vnd.openxmlformats-officedocument.wordprocessingml.{kind}+xml"/>"#));
    }
    ct.push_str("</Types>");
    let core = format!(
        r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?><cp:coreProperties xmlns:cp="http://schemas.openxmlformats.org/package/2006/metadata/core-properties" xmlns:dc="http://purl.org/dc/elements/1.1/"><dc:title>{}</dc:title><dc:creator>OwnDraw</dc:creator></cp:coreProperties>"#,
        esc(title)
    );
    let root_rels = r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?><Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships"><Relationship Id="rId1" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/officeDocument" Target="word/document.xml"/><Relationship Id="rId2" Type="http://schemas.openxmlformats.org/package/2006/relationships/metadata/core-properties" Target="docProps/core.xml"/></Relationships>"#;
    let mut z = zip::ZipWriter::new(Cursor::new(Vec::new()));
    let opt = SimpleFileOptions::default().compression_method(zip::CompressionMethod::Deflated);
    let mut files: Vec<(String, String)> = vec![
        ("[Content_Types].xml".into(), ct),
        ("_rels/.rels".into(), root_rels.into()),
        ("docProps/core.xml".into(), core),
        ("word/document.xml".into(), document),
        ("word/styles.xml".into(), styles),
        ("word/numbering.xml".into(), numbering),
        ("word/_rels/document.xml.rels".into(), rels_xml),
    ];
    files.extend(parts);
    for (n, c) in files {
        z.start_file(n, opt).map_err(|e| e.to_string())?;
        z.write_all(c.as_bytes()).map_err(|e| e.to_string())?;
    }
    Ok(z.finish().map_err(|e| e.to_string())?.into_inner())
}

// ------------------------------------------------------------------ Import

fn read_entry(z: &mut zip::ZipArchive<Cursor<&[u8]>>, name: &str) -> Option<String> {
    let mut e = z.by_name(name).ok()?;
    let mut s = String::new();
    e.by_ref().take(64 << 20).read_to_string(&mut s).ok()?;
    Some(s)
}

fn wattr<'a>(n: roxmltree::Node<'a, 'a>, name: &str) -> Option<&'a str> {
    n.attributes().find(|a| a.name() == name && a.namespace().is_none_or(|ns| ns == W)).map(|a| a.value())
}

fn child<'a>(n: roxmltree::Node<'a, 'a>, name: &str) -> Option<roxmltree::Node<'a, 'a>> {
    n.children().find(|c| c.is_element() && c.tag_name().name() == name)
}

fn on(n: Option<roxmltree::Node>) -> bool {
    n.is_some_and(|n| !matches!(wattr(n, "val"), Some("0" | "false" | "none")))
}

fn is_sans(name: &str) -> bool {
    let l = name.to_ascii_lowercase();
    ["arial", "calibri", "helvetica", "verdana", "segoe", "sans", "aptos", "tahoma", "trebuchet", "roboto", "open sans", "liberation sans", "carlito", "arimo"].iter().any(|s| l.contains(s))
}

fn highlight(name: &str) -> u32 {
    match name {
        "yellow" => 0xFFFDE68A,
        "green" => 0xFFBBF7D0,
        "cyan" => 0xFFA5F3FC,
        "magenta" => 0xFFFBCFE8,
        "blue" => 0xFFBFDBFE,
        "red" => 0xFFFECACA,
        "lightGray" | "darkGray" => 0xFFE5E7EB,
        "none" => 0,
        _ => 0xFFFDE68A,
    }
}

fn color_of(v: &str) -> u32 {
    u32::from_str_radix(v, 16).ok().filter(|_| v.len() == 6).map(|c| 0xFF00_0000 | c).unwrap_or(0)
}

fn run_style(rpr: Option<roxmltree::Node>, base: &CStyle) -> CStyle {
    let mut st = base.clone();
    let Some(r) = rpr else { return st };
    if let Some(n) = child(r, "b") {
        st.b = on(Some(n));
    }
    if let Some(n) = child(r, "i") {
        st.i = on(Some(n));
    }
    if let Some(n) = child(r, "u") {
        st.u = on(Some(n));
    }
    if child(r, "strike").is_some() || child(r, "dstrike").is_some() {
        st.s = on(child(r, "strike").or(child(r, "dstrike")));
    }
    if let Some(c) = child(r, "color").and_then(|n| wattr(n, "val")).filter(|v| *v != "auto") {
        let c = color_of(c);
        st.color = if c == 0xFF000000 { 0 } else { c };
    }
    if let Some(sz) = child(r, "sz").and_then(|n| wattr(n, "val")).and_then(|v| v.parse::<f32>().ok()) {
        st.size = Some((sz / 2.0).clamp(1.0, 400.0));
    }
    if let Some(h) = child(r, "highlight").and_then(|n| wattr(n, "val")) {
        st.hl = highlight(h);
    }
    if let Some(f) = child(r, "shd").and_then(|n| wattr(n, "fill")).filter(|v| *v != "auto") {
        st.hl = color_of(f);
    }
    if let Some(v) = child(r, "vertAlign").and_then(|n| wattr(n, "val")) {
        st.va = match v {
            "superscript" => VAlign::Sup,
            "subscript" => VAlign::Sub,
            _ => VAlign::Base,
        };
    }
    if let Some(f) = child(r, "rFonts").and_then(|n| wattr(n, "ascii").or(wattr(n, "hAnsi"))) {
        st.fam = Some(if is_sans(f) { Family::Sans } else { Family::Serif });
    }
    st
}

struct Ctx {
    /// styleId → (Vorlage, Zeichenformat der Vorlage)
    styles: HashMap<String, (Option<Kind>, CStyle)>,
    /// numId → (abstractNumId-Formate je Ebene: Aufzählung?)
    nums: HashMap<String, Vec<bool>>,
    links: HashMap<String, String>,
    doc_family: Family,
}

fn runs_of(n: roxmltree::Node, cx: &Ctx, base: &CStyle, link: Option<&str>, out: &mut Vec<Run>, page_break: &mut bool) {
    for c in n.children().filter(|c| c.is_element()) {
        match c.tag_name().name() {
            "r" => {
                let mut st = run_style(child(c, "rPr"), base);
                if let Some(id) = child(c, "rPr").and_then(|r| child(r, "rStyle")).and_then(|s| wattr(s, "val")) {
                    if let Some((_, cs)) = cx.styles.get(id) {
                        st = run_style(child(c, "rPr"), &merge(base, cs));
                    }
                }
                st.link = link.map(String::from);
                let mut text = String::new();
                for t in c.children().filter(|t| t.is_element()) {
                    match t.tag_name().name() {
                        "t" => text.push_str(t.text().unwrap_or("")),
                        "tab" => text.push('\t'),
                        "br" if wattr(t, "type") == Some("page") => *page_break = true,
                        "br" | "cr" => text.push(' '),
                        "noBreakHyphen" => text.push('-'),
                        _ => {}
                    }
                }
                if !text.is_empty() {
                    out.push(Run { text, st });
                }
            }
            "hyperlink" => {
                let target = c.attributes().find(|a| a.name() == "id").and_then(|a| cx.links.get(a.value())).cloned();
                runs_of(c, cx, base, target.as_deref().or(link), out, page_break);
            }
            "smartTag" | "ins" | "sdt" | "sdtContent" | "fldSimple" => runs_of(c, cx, base, link, out, page_break),
            _ => {}
        }
    }
}

fn merge(a: &CStyle, b: &CStyle) -> CStyle {
    CStyle {
        b: a.b || b.b,
        i: a.i || b.i,
        u: a.u || b.u,
        s: a.s || b.s,
        size: b.size.or(a.size),
        color: if b.color != 0 { b.color } else { a.color },
        hl: if b.hl != 0 { b.hl } else { a.hl },
        fam: b.fam.or(a.fam),
        va: if b.va != VAlign::Base { b.va } else { a.va },
        link: None,
    }
}

fn para_of(n: roxmltree::Node, cx: &Ctx, out: &mut Vec<Para>) {
    let ppr = child(n, "pPr");
    let mut kind = Kind::Body;
    let mut base = CStyle::default();
    let mut indent = 0u8;
    if let Some(sid) = ppr.and_then(|p| child(p, "pStyle")).and_then(|s| wattr(s, "val")) {
        if let Some((k, cs)) = cx.styles.get(sid) {
            kind = k.unwrap_or(Kind::Body);
            base = cs.clone();
            // Vorlagen von Überschriften bringen ihr Format über die eigene Vorlage mit
            if kind.is_heading() {
                base = CStyle { color: base.color, ..CStyle::default() };
            }
        }
    }
    if let Some(np) = ppr.and_then(|p| child(p, "numPr")) {
        let lvl = child(np, "ilvl").and_then(|x| wattr(x, "val")).and_then(|v| v.parse::<usize>().ok()).unwrap_or(0);
        if let Some(fmts) = child(np, "numId").and_then(|x| wattr(x, "val")).filter(|v| *v != "0").and_then(|v| cx.nums.get(v)) {
            kind = if fmts.get(lvl).copied().unwrap_or(true) { Kind::Bullet } else { Kind::Number };
            indent = lvl.min(8) as u8;
        }
    }
    let align = match ppr.and_then(|p| child(p, "jc")).and_then(|j| wattr(j, "val")) {
        Some("center") => Align::Center,
        Some("right" | "end") => Align::Right,
        Some("both" | "distribute") => Align::Justify,
        _ => Align::Left,
    };
    if !kind.is_list() {
        if let Some(l) = ppr.and_then(|p| child(p, "ind")).and_then(|i| wattr(i, "left").or(wattr(i, "start"))).and_then(|v| v.parse::<f32>().ok()) {
            indent = ((l / 20.0) / super::layout::INDENT).round().clamp(0.0, 8.0) as u8;
        }
    }
    let line = ppr
        .and_then(|p| child(p, "spacing"))
        .filter(|s| wattr(*s, "lineRule").is_none_or(|r| r == "auto"))
        .and_then(|s| wattr(s, "line"))
        .and_then(|v| v.parse::<f32>().ok())
        .map(|v| v / 240.0)
        .filter(|v| (0.5..=4.0).contains(v) && (*v - 1.15).abs() > 0.02 && (*v - 1.08).abs() > 0.02)
        .unwrap_or(0.0);
    let mut runs = Vec::new();
    let mut pb = false;
    runs_of(n, cx, &base, None, &mut runs, &mut pb);
    let mut p = Para { kind, align, indent, line, runs };
    p.normalize();
    let empty = p.is_empty();
    if !(pb && empty) {
        out.push(p);
    }
    if pb {
        out.push(Para::new(Kind::PageBreak));
    }
    let _ = cx.doc_family;
}

fn block(n: roxmltree::Node, cx: &Ctx, out: &mut Vec<Para>) {
    for c in n.children().filter(|c| c.is_element()) {
        match c.tag_name().name() {
            "p" => para_of(c, cx, out),
            "tbl" => {
                for tr in c.children().filter(|r| r.is_element() && r.tag_name().name() == "tr") {
                    let mut row = Para::new(Kind::Body);
                    for (i, tc) in tr.children().filter(|x| x.is_element() && x.tag_name().name() == "tc").enumerate() {
                        let mut cell = Vec::new();
                        block(tc, cx, &mut cell);
                        if i > 0 {
                            let o = row.len();
                            row.insert(o, "\t", &CStyle::default());
                        }
                        for (k, p) in cell.into_iter().filter(|p| p.kind != Kind::PageBreak).enumerate() {
                            if k > 0 {
                                let o = row.len();
                                row.insert(o, " ", &CStyle::default());
                            }
                            row.append(p);
                        }
                    }
                    out.push(row);
                }
            }
            "sdt" | "sdtContent" | "customXml" => block(c, cx, out),
            _ => {}
        }
    }
}

/// Liest eine DOCX-Datei.
pub fn import(bytes: &[u8]) -> Result<TextDoc, String> {
    let mut z = zip::ZipArchive::new(Cursor::new(bytes)).map_err(|_| "Das ist keine Word-Datei (.docx).".to_string())?;
    let doc_xml = read_entry(&mut z, "word/document.xml").ok_or("In der Datei fehlt der Text (word/document.xml).")?;
    let styles_xml = read_entry(&mut z, "word/styles.xml").unwrap_or_default();
    let num_xml = read_entry(&mut z, "word/numbering.xml").unwrap_or_default();
    let rels_xml = read_entry(&mut z, "word/_rels/document.xml.rels").unwrap_or_default();
    let opt = roxmltree::ParsingOptions { allow_dtd: false, ..Default::default() };
    let mut cx = Ctx { styles: HashMap::new(), nums: HashMap::new(), links: HashMap::new(), doc_family: Family::Sans };
    let mut td = TextDoc::new();
    if let Ok(s) = roxmltree::Document::parse_with_options(&styles_xml, opt) {
        if let Some(rpr) = s.descendants().find(|n| n.tag_name().name() == "rPrDefault").and_then(|d| child(d, "rPr")) {
            if let Some(f) = child(rpr, "rFonts").and_then(|n| wattr(n, "ascii").or(wattr(n, "hAnsi"))) {
                cx.doc_family = if is_sans(f) { Family::Sans } else { Family::Serif };
            }
            if let Some(sz) = child(rpr, "sz").and_then(|n| wattr(n, "val")).and_then(|v| v.parse::<f32>().ok()) {
                td.size = (sz / 2.0).clamp(6.0, 72.0);
            }
        }
        for st in s.descendants().filter(|n| n.tag_name().name() == "style") {
            let Some(id) = wattr(st, "styleId") else { continue };
            let name = child(st, "name").and_then(|n| wattr(n, "val")).unwrap_or(id).to_ascii_lowercase();
            let outline = child(st, "pPr").and_then(|p| child(p, "outlineLvl")).and_then(|o| wattr(o, "val")).and_then(|v| v.parse::<u8>().ok());
            let kind = match name.as_str() {
                "title" => Some(Kind::Title),
                "heading 1" => Some(Kind::H1),
                "heading 2" => Some(Kind::H2),
                n if n.starts_with("heading ") => Some(Kind::H3),
                "quote" | "intense quote" => Some(Kind::Quote),
                _ => match outline {
                    Some(0) => Some(Kind::H1),
                    Some(1) => Some(Kind::H2),
                    Some(2..=8) => Some(Kind::H3),
                    _ => None,
                },
            };
            cx.styles.insert(id.to_string(), (kind, run_style(child(st, "rPr"), &CStyle::default())));
        }
    }
    td.family = cx.doc_family;
    if let Ok(nx) = roxmltree::Document::parse_with_options(&num_xml, opt) {
        let mut abs: HashMap<String, Vec<bool>> = HashMap::new();
        for a in nx.descendants().filter(|n| n.tag_name().name() == "abstractNum") {
            let Some(id) = wattr(a, "abstractNumId") else { continue };
            let mut v = vec![true; 9];
            for l in a.children().filter(|l| l.tag_name().name() == "lvl") {
                let i = wattr(l, "ilvl").and_then(|v| v.parse::<usize>().ok()).unwrap_or(0).min(8);
                v[i] = child(l, "numFmt").and_then(|f| wattr(f, "val")).is_none_or(|f| f == "bullet" || f == "none");
            }
            abs.insert(id.to_string(), v);
        }
        for n in nx.descendants().filter(|n| n.tag_name().name() == "num") {
            if let (Some(id), Some(a)) = (wattr(n, "numId"), child(n, "abstractNumId").and_then(|a| wattr(a, "val"))) {
                if let Some(v) = abs.get(a) {
                    cx.nums.insert(id.to_string(), v.clone());
                }
            }
        }
    }
    if let Ok(rx) = roxmltree::Document::parse_with_options(&rels_xml, opt) {
        for r in rx.descendants().filter(|n| n.tag_name().name() == "Relationship") {
            if r.attribute("Type").is_some_and(|t| t.ends_with("/hyperlink")) {
                if let (Some(id), Some(t)) = (r.attribute("Id"), r.attribute("Target")) {
                    cx.links.insert(id.to_string(), t.to_string());
                }
            }
        }
    }
    let d = roxmltree::Document::parse_with_options(&doc_xml, opt).map_err(|e| format!("Word-Datei ist beschädigt: {e}"))?;
    let body = d.descendants().find(|n| n.tag_name().name() == "body").ok_or("Word-Datei ohne Inhalt")?;
    let mut paras = Vec::new();
    block(body, &cx, &mut paras);
    if let Some(sp) = body.descendants().filter(|n| n.tag_name().name() == "sectPr").last() {
        let num = |n: Option<roxmltree::Node>, k: &str| n.and_then(|n| wattr(n, k)).and_then(|v| v.parse::<f32>().ok()).map(|v| v / 20.0);
        let sz = child(sp, "pgSz");
        let mar = child(sp, "pgMar");
        let d = PageSpec::default();
        td.page = PageSpec {
            w: num(sz, "w").filter(|v| *v > 72.0).unwrap_or(d.w),
            h: num(sz, "h").filter(|v| *v > 72.0).unwrap_or(d.h),
            ml: num(mar, "left").unwrap_or(d.ml).clamp(0.0, 400.0),
            mr: num(mar, "right").unwrap_or(d.mr).clamp(0.0, 400.0),
            mt: num(mar, "top").map(f32::abs).unwrap_or(d.mt).clamp(0.0, 400.0),
            mb: num(mar, "bottom").map(f32::abs).unwrap_or(d.mb).clamp(0.0, 400.0),
        };
    }
    while paras.last().is_some_and(|p| p.kind == Kind::PageBreak) {
        paras.pop();
    }
    if !paras.is_empty() {
        td.paras = paras;
    }
    Ok(td)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::office::model::Pos;

    #[test]
    fn export_und_wieder_einlesen() {
        let mut d = TextDoc::new();
        d.insert_text(Pos::default(), "Bericht\nEinleitung mit Text\nPunkt eins\nPunkt zwei\nSchluss", &CStyle::default());
        d.paras[0].kind = Kind::Title;
        d.paras[1].align = Align::Justify;
        d.paras[2].kind = Kind::Number;
        d.paras[3].kind = Kind::Number;
        d.paras[3].indent = 1;
        d.restyle(Pos::new(1, 0), Pos::new(1, 10), &|s| {
            s.b = true;
            s.color = 0xFFD93025;
        });
        d.restyle(Pos::new(4, 0), Pos::new(4, 7), &|s| s.link = Some("https://example.org/?a=1&b=2".into()));
        d.paras.insert(4, Para::new(Kind::PageBreak));
        d.footer = "Seite {seite} von {seiten}".into();
        d.page.w = 842.0;
        d.page.h = 595.0;
        let bytes = export(&d, "Bericht <1>").unwrap();
        let back = import(&bytes).unwrap();
        let kinds: Vec<Kind> = back.paras.iter().map(|p| p.kind).collect();
        assert_eq!(kinds, [Kind::Title, Kind::Body, Kind::Number, Kind::Number, Kind::PageBreak, Kind::Body]);
        assert_eq!(back.paras[1].align, Align::Justify);
        assert!(back.paras[1].runs[0].st.b);
        assert_eq!(back.paras[1].runs[0].st.color, 0xFFD93025);
        assert_eq!(back.paras[3].indent, 1);
        assert_eq!(back.paras[5].runs[0].st.link.as_deref(), Some("https://example.org/?a=1&b=2"));
        assert_eq!(back.plain_text(), d.plain_text());
        assert!((back.page.w - 842.0).abs() < 0.1);
    }

    #[test]
    fn kein_docx() {
        assert!(import(b"kein zip").is_err());
    }
}
