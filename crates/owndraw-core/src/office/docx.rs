//! Word-Dateien (DOCX): Export und Import von Formatvorlagen, Listen, Links, Kopf-/Fußzeile und Seitenzahlen,
//! Seitengröße, Bildern (auch mit Textumfluss), Tabellen, Beschriftungen (SEQ-Feld), Feldern (Seitenzahl, Seriendruck,
//! Stichwort), Fußnoten, Kommentaren, nachverfolgten Änderungen, Spalten und Wasserzeichen (nur Export).
//! Objekte (Formen, Formeln, Diagramme) gehen als Bild hinaus und kommen als Bild zurück.

use super::fonts::Family;
use super::fields::{field_text, FieldCx, Numbers};
use super::model::{Align, CStyle, Img, Kind, PageSpec, Para, Run, TextDoc, VAlign};
use super::objects::{Cell, Comment, Extras, Field, Obj, ObjKind, Reply, Table, Wrap, OBJ};
use super::render::Images;
use std::borrow::Cow;
use std::cell::{Cell as StdCell, RefCell};
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

/// Zeichenformat als `w:rPr`-Inhalt.
fn rpr_xml(st: &CStyle) -> String {
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
    pr
}

/// Lauf mit beliebigem Inhalt (`inner` ist fertiges XML).
fn run_raw(st: &CStyle, inner: &str) -> String {
    let pr = rpr_xml(st);
    if pr.is_empty() {
        format!("<w:r>{inner}</w:r>")
    } else {
        format!("<w:r><w:rPr>{pr}</w:rPr>{inner}</w:r>")
    }
}

/// Lauf mit Text; `del` schreibt `w:delText` (gelöschte Änderung).
fn run_xml(r: &Run, out: &mut String, del: bool) {
    let tag = if del { "w:delText" } else { "w:t" };
    let mut inner = String::new();
    for (i, part) in r.text.split('\t').enumerate() {
        if i > 0 {
            inner.push_str("<w:tab/>");
        }
        if !part.is_empty() {
            inner.push_str(&format!(r#"<{tag} xml:space="preserve">{}</{tag}>"#, esc(part)));
        }
    }
    out.push_str(&run_raw(&r.st, &inner));
}

fn style_id(k: Kind) -> Option<&'static str> {
    match k {
        Kind::Title => Some("Title"),
        Kind::H1 => Some("Heading1"),
        Kind::H2 => Some("Heading2"),
        Kind::H3 => Some("Heading3"),
        Kind::Quote => Some("Quote"),
        Kind::Caption => Some("Caption"),
        Kind::Bullet | Kind::Number => Some("ListParagraph"),
        _ => None,
    }
}

fn hf_xml(tag: &str, text: &str, center: bool, wm: &str) -> String {
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
    if !wm.is_empty() && tag == "hdr" {
        body.push_str(&watermark_xml(wm));
    }
    let jc = if center { r#"<w:pPr><w:jc w:val="center"/></w:pPr>"# } else { "" };
    format!(r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?><w:{tag} xmlns:w="{W}" xmlns:r="{R}" xmlns:v="urn:schemas-microsoft-com:vml" xmlns:o="urn:schemas-microsoft-com:office:office"><w:p>{jc}{body}</w:p></w:{tag}>"#)
}

/// Wasserzeichen als VML-Textpfad (wie von Word erzeugt), diagonal und halbtransparent.
fn watermark_xml(text: &str) -> String {
    let t = esc(text);
    format!(
        r##"<w:r><w:pict><v:shapetype id="_x0000_t136" coordsize="21600,21600" o:spt="136" adj="10800" path="m@7,l@8,m@5,21600l@6,21600e"><v:formulas><v:f eqn="sum #0 0 10800"/><v:f eqn="prod #0 2 1"/><v:f eqn="sum 21600 0 @1"/><v:f eqn="sum 0 0 @2"/><v:f eqn="sum 21600 0 @3"/><v:f eqn="if @0 @3 0"/><v:f eqn="if @0 21600 @1"/><v:f eqn="if @0 0 @2"/><v:f eqn="if @0 @4 21600"/><v:f eqn="mid @5 @6"/><v:f eqn="mid @8 @5"/><v:f eqn="mid @7 @8"/><v:f eqn="mid @6 @7"/><v:f eqn="sum @6 0 @5"/></v:formulas><v:path textpathok="t" o:connecttype="custom" o:connectlocs="@9,0;@10,10800;@11,21600;@12,10800" o:connectangles="270,180,90,0"/><v:textpath on="t" fitshape="t"/><v:handles><v:h position="#0,bottomRight" xrange="6629,14971"/></v:handles><o:lock v:ext="edit" text="t" shapetype="t"/></v:shapetype><v:shape id="PowerPlusWaterMarkObject1" o:spid="_x0000_s2049" type="#_x0000_t136" style="position:absolute;margin-left:0;margin-top:0;width:430pt;height:107pt;rotation:315;z-index:-251658752;mso-position-horizontal:center;mso-position-horizontal-relative:margin;mso-position-vertical:center;mso-position-vertical-relative:margin" o:allowincell="f" fillcolor="silver" stroked="f"><v:fill opacity=".5"/><v:textpath style="font-family:&quot;Arial&quot;;font-size:1pt" string="{t}"/></v:shape></w:pict></w:r>"##
    )
}

/// Ein Bild als `w:drawing` (inline oder, bei Textumfluss, verankert mit `wrapSquare`).
fn drawing_xml(n: usize, name: &str, rid: usize, cx: i64, cy: i64, wrap: Wrap) -> String {
    let pic = format!(
        r#"<wp:docPr id="{n}" name="Bild {n}"/><wp:cNvGraphicFramePr><a:graphicFrameLocks noChangeAspect="1"/></wp:cNvGraphicFramePr><a:graphic><a:graphicData uri="http://schemas.openxmlformats.org/drawingml/2006/picture"><pic:pic><pic:nvPicPr><pic:cNvPr id="{n}" name="{name}"/><pic:cNvPicPr/></pic:nvPicPr><pic:blipFill><a:blip r:embed="rId{rid}"/><a:stretch><a:fillRect/></a:stretch></pic:blipFill><pic:spPr><a:xfrm><a:off x="0" y="0"/><a:ext cx="{cx}" cy="{cy}"/></a:xfrm><a:prstGeom prst="rect"><a:avLst/></a:prstGeom></pic:spPr></pic:pic></a:graphicData></a:graphic>"#
    );
    let side = match wrap {
        Wrap::Left => "left",
        Wrap::Right => "right",
        Wrap::Inline => {
            return format!(r#"<w:drawing><wp:inline distT="0" distB="0" distL="0" distR="0"><wp:extent cx="{cx}" cy="{cy}"/>{pic}</wp:inline></w:drawing>"#);
        }
    };
    format!(
        r#"<w:drawing><wp:anchor distT="0" distB="0" distL="114300" distR="114300" simplePos="0" relativeHeight="251658240" behindDoc="0" locked="0" layoutInCell="1" allowOverlap="0"><wp:simplePos x="0" y="0"/><wp:positionH relativeFrom="margin"><wp:align>{side}</wp:align></wp:positionH><wp:positionV relativeFrom="paragraph"><wp:posOffset>0</wp:posOffset></wp:positionV><wp:extent cx="{cx}" cy="{cy}"/><wp:effectExtent l="0" t="0" r="0" b="0"/><wp:wrapSquare wrapText="bothSides"/>{pic}</wp:anchor></w:drawing>"#
    )
}

fn jc_of(a: Align) -> &'static str {
    match a {
        Align::Center => "center",
        Align::Right => "right",
        Align::Justify => "both",
        Align::Left => "left",
    }
}

/// Tabelle als `w:tbl` (Spalten in twips, Rahmen, Kopfzeile, Zellformat; Streifen als Zellfarbe).
fn table_xml(t: &Table) -> String {
    let total: i64 = t.cols.iter().map(|c| tw(*c)).sum();
    let mut o = format!(r#"<w:tbl><w:tblPr><w:tblW w:w="{total}" w:type="dxa"/>"#);
    if t.border != 0 {
        let c = hex(t.border);
        o.push_str("<w:tblBorders>");
        for side in ["top", "left", "bottom", "right", "insideH", "insideV"] {
            o.push_str(&format!(r#"<w:{side} w:val="single" w:sz="4" w:space="0" w:color="{c}"/>"#));
        }
        o.push_str("</w:tblBorders>");
    }
    o.push_str(r#"<w:tblLayout w:type="fixed"/></w:tblPr><w:tblGrid>"#);
    for c in &t.cols {
        o.push_str(&format!(r#"<w:gridCol w:w="{}"/>"#, tw(*c)));
    }
    o.push_str("</w:tblGrid>");
    let hd = t.header as usize;
    for (ri, row) in t.rows.iter().enumerate() {
        let head = t.header && ri == 0;
        o.push_str("<w:tr>");
        if head {
            o.push_str("<w:trPr><w:tblHeader/></w:trPr>");
        }
        for (ci, c) in row.iter().enumerate() {
            let w = t.cols.get(ci).map(|w| tw(*w)).unwrap_or(1000);
            let bg = if c.bg != 0 {
                c.bg
            } else if head {
                0xFFE5E7EB
            } else if t.stripes && ri >= hd && (ri - hd) % 2 == 1 {
                0xFFF3F4F6
            } else {
                0
            };
            o.push_str(&format!(r#"<w:tc><w:tcPr><w:tcW w:w="{w}" w:type="dxa"/>"#));
            if bg != 0 {
                o.push_str(&format!(r#"<w:shd w:val="clear" w:color="auto" w:fill="{}"/>"#, hex(bg)));
            }
            o.push_str("</w:tcPr>");
            let st = CStyle { b: c.b || head, i: c.i, color: c.color, ..CStyle::default() };
            for line in c.text.split('\n') {
                o.push_str(&format!(r#"<w:p><w:pPr><w:spacing w:after="0"/><w:jc w:val="{}"/></w:pPr>"#, jc_of(c.align)));
                run_xml(&Run { text: line.to_string(), st: st.clone() }, &mut o, false);
                o.push_str("</w:p>");
            }
            o.push_str("</w:tc>");
        }
        o.push_str("</w:tr>");
    }
    o.push_str("</w:tbl>");
    o
}

/// Zustand während des Schreibens.
struct Xp<'a> {
    doc: &'a TextDoc,
    nums: Numbers,
    /// Fußnotentexte (Fußnote n steht an Index n-1).
    notes: Vec<String>,
    /// Kommentar-ID → Word-IDs (Kommentar und seine Antworten).
    cmt: HashMap<u32, Vec<usize>>,
    /// Zähler für Änderungs-IDs.
    rev: u32,
}

impl Xp<'_> {
    fn rev_attrs(&mut self, id: u32) -> String {
        self.rev += 1;
        let (a, t) = self.doc.ex.revs.iter().find(|r| r.id == id).map(|r| (r.author.as_str(), r.time.as_str())).unwrap_or(("Unbekannt", ""));
        let d = if t.is_empty() { String::new() } else { format!(r#" w:date="{}""#, esc(t)) };
        format!(r#"w:id="{}" w:author="{}"{d}"#, 1000 + self.rev, esc(if a.is_empty() { "Unbekannt" } else { a }))
    }

    /// `inner` in `w:ins`/`w:del` hüllen, wenn der Lauf eine Änderung ist.
    fn wrap_rev(&mut self, st: &CStyle, inner: String) -> String {
        if st.del > 0 {
            format!("<w:del {}>{inner}</w:del>", self.rev_attrs(st.del))
        } else if st.ins > 0 {
            format!("<w:ins {}>{inner}</w:ins>", self.rev_attrs(st.ins))
        } else {
            inner
        }
    }

    fn close_cmt(&self, id: u32, out: &mut String) {
        for w in self.cmt.get(&id).into_iter().flatten() {
            out.push_str(&format!(r#"<w:commentRangeEnd w:id="{w}"/><w:r><w:commentReference w:id="{w}"/></w:r>"#));
        }
    }

    /// Ein Lauf (mit Feldern, Änderungen).
    fn one_run(&mut self, r: &Run, out: &mut String) {
        let Some(f) = &r.st.field else {
            let mut x = String::new();
            run_xml(r, &mut x, r.st.del > 0);
            let x = self.wrap_rev(&r.st, x);
            out.push_str(&x);
            return;
        };
        if r.st.del > 0 {
            return;
        }
        let st = CStyle { field: None, ..r.st.clone() };
        for _ in r.text.chars() {
            let simple = |instr: &str, shown: &str| format!(r#"<w:fldSimple w:instr="{}">{}</w:fldSimple>"#, esc(instr), run_raw(&st, &format!(r#"<w:t xml:space="preserve">{}</w:t>"#, esc(shown))));
            let (xml, wrappable) = match f {
                Field::Page => (simple(" PAGE ", "1"), false),
                Field::Pages => (simple(" NUMPAGES ", "1"), false),
                Field::Merge(n) => (simple(&format!(" MERGEFIELD {} ", n.replace('"', "")), &format!("«{n}»")), false),
                Field::Index(t) => (format!(r#"<w:fldSimple w:instr="{}"/>"#, esc(&format!(" XE \"{}\" ", t.replace('"', "'")))), false),
                Field::Note(t) => {
                    self.notes.push(t.clone());
                    (format!(r#"<w:r><w:rPr><w:rStyle w:val="FootnoteReference"/></w:rPr><w:footnoteReference w:id="{}"/></w:r>"#, self.notes.len()), true)
                }
                _ => {
                    let text = match f {
                        Field::Choice(o, i) => o.get(*i).cloned().unwrap_or_default(),
                        _ => {
                            let pp = |_: usize| 0usize;
                            field_text(f, &FieldCx { doc: self.doc, nums: &self.nums, page: 0, pages: 1, para_page: &pp, note: 0 })
                        }
                    };
                    let mut x = String::new();
                    run_xml(&Run { text, st: st.clone() }, &mut x, false);
                    (x, true)
                }
            };
            let xml = if wrappable { self.wrap_rev(&r.st, xml) } else { xml };
            out.push_str(&xml);
        }
    }

    /// Läufe eines Absatzes: Links, Kommentarbereiche, Felder, Änderungen.
    fn runs_out(&mut self, p: &Para, rels: &mut Vec<String>, out: &mut String) {
        let cmt_of = |x: &Xp, r: &Run| if x.cmt.contains_key(&r.st.cmt) { r.st.cmt } else { 0 };
        let mut cur = 0u32;
        let mut i = 0;
        while i < p.runs.len() {
            let c = cmt_of(self, &p.runs[i]);
            if c != cur {
                self.close_cmt(cur, out);
                for w in self.cmt.get(&c).into_iter().flatten() {
                    out.push_str(&format!(r#"<w:commentRangeStart w:id="{w}"/>"#));
                }
                cur = c;
            }
            if let Some(l) = p.runs[i].st.link.clone() {
                rels.push(format!(r#"Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/hyperlink" Target="{}" TargetMode="External""#, esc(&l)));
                out.push_str(&format!(r#"<w:hyperlink r:id="rId{}">"#, rels.len()));
                while i < p.runs.len() && p.runs[i].st.link.as_ref() == Some(&l) && cmt_of(self, &p.runs[i]) == cur {
                    self.one_run(&p.runs[i], out);
                    i += 1;
                }
                out.push_str("</w:hyperlink>");
            } else {
                self.one_run(&p.runs[i], out);
                i += 1;
            }
        }
        self.close_cmt(cur, out);
    }
}

/// `comments.xml` samt Zuordnung Kommentar-ID → Word-IDs.
fn comments_xml(ex: &Extras) -> (String, HashMap<u32, Vec<usize>>) {
    let mut map = HashMap::new();
    let mut n = 0usize;
    let mut o = format!(r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?><w:comments xmlns:w="{W}">"#);
    let mut one = |author: &str, time: &str, text: &str, ids: &mut Vec<usize>| {
        let ini: String = author.split_whitespace().filter_map(|w| w.chars().next()).take(3).collect();
        let d = if time.is_empty() { String::new() } else { format!(r#" w:date="{}""#, esc(time)) };
        o.push_str(&format!(r#"<w:comment w:id="{n}" w:author="{}"{d} w:initials="{}">"#, esc(author), esc(&ini)));
        for line in text.split('\n') {
            o.push_str(&format!(r#"<w:p><w:r><w:t xml:space="preserve">{}</w:t></w:r></w:p>"#, esc(line)));
        }
        o.push_str("</w:comment>");
        ids.push(n);
        n += 1;
    };
    for c in &ex.comments {
        let mut ids = Vec::new();
        one(&c.author, &c.time, &c.text, &mut ids);
        for r in &c.replies {
            one(&r.author, &r.time, &r.text, &mut ids);
        }
        map.insert(c.id, ids);
    }
    o.push_str("</w:comments>");
    (o, map)
}

/// Bild einbetten: Beziehung und Mediendatei anlegen, `w:drawing` liefern.
fn embed<'a>(media: &mut Vec<(String, Cow<'a, [u8]>)>, rels: &mut Vec<String>, bytes: Cow<'a, [u8]>, ext: &str, size: (f32, f32), content_w: f32, wrap: Wrap) -> String {
    let n = media.len() + 1;
    let name = format!("image{n}.{ext}");
    rels.push(format!(r#"Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/image" Target="media/{name}""#));
    let rid = rels.len();
    media.push((name.clone(), bytes));
    // Größe wie im Satz (auf die Inhaltsbreite begrenzt)
    let k = (content_w / size.0.max(1.0)).min(1.0);
    drawing_xml(n, &name, rid, (size.0 * k * 12700.0) as i64, (size.1 * k * 12700.0) as i64, wrap)
}

/// Ersatztext für ein Objekt, das sich nicht als Bild ausgeben lässt.
fn obj_text(o: &Obj) -> String {
    match &o.kind {
        ObjKind::Formula { src } => format!("[Formel: {src}]"),
        ObjKind::Shape { text, .. } if !text.trim().is_empty() => text.clone(),
        ObjKind::Shape { .. } => "[Form]".into(),
        ObjKind::Drawing { .. } => "[Zeichnung]".into(),
        ObjKind::Chart { .. } => "[Diagramm]".into(),
    }
}

/// Schreibt eine DOCX-Datei.
pub fn export(doc: &TextDoc, title: &str, imgs: &Images) -> Result<Vec<u8>, String> {
    let mut media: Vec<(String, Cow<[u8]>)> = Vec::new();
    let mut rels = vec![
        (r#"Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/styles" Target="styles.xml""#.to_string()),
        (r#"Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/numbering" Target="numbering.xml""#.to_string()),
        (r#"Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/settings" Target="settings.xml""#.to_string()),
    ];
    let (comments, cmt) = if doc.ex.comments.is_empty() { (String::new(), HashMap::new()) } else { comments_xml(&doc.ex) };
    let mut x = Xp { doc, nums: Numbers::new(doc), notes: Vec::new(), cmt, rev: 0 };
    let mut body = String::new();
    let mut nums: Vec<(usize, bool)> = Vec::new(); // (numId, Aufzählung?)
    let mut cur_list: Option<usize> = None;
    let cw = doc.page.content_w();
    for (pi, p) in doc.paras.iter().enumerate() {
        let image = match (p.kind, &p.img, &p.obj) {
            (Kind::Image, Some(im), _) => {
                let Some(bytes) = imgs.get(&im.file) else { continue };
                let ext = im.file.rsplit('.').next().filter(|e| e.len() <= 4).unwrap_or("png").to_ascii_lowercase();
                Some((Cow::Borrowed(bytes.as_slice()), ext, (im.w, im.h)))
            }
            // Objekte als Bild; ohne Bild weiter unten als Text
            (Kind::Object, _, Some(o)) => super::render::object_png(o, 2.0).map(|png| (Cow::Owned(png), "png".to_string(), (o.w, o.h))),
            _ => None,
        };
        if let Some((bytes, ext, size)) = image {
            let d = embed(&mut media, &mut rels, bytes, &ext, size, cw, p.wrap);
            let jc = if p.wrap == Wrap::Inline { format!(r#"<w:pPr><w:jc w:val="{}"/></w:pPr>"#, jc_of(p.align).replace("both", "left")) } else { String::new() };
            body.push_str(&format!("<w:p>{jc}<w:r>{d}</w:r></w:p>"));
            cur_list = None;
            continue;
        }
        if p.kind == Kind::Image {
            continue;
        }
        if p.kind == Kind::PageBreak {
            body.push_str(r#"<w:p><w:r><w:br w:type="page"/></w:r></w:p>"#);
            cur_list = None;
            continue;
        }
        if let (Kind::Table, Some(t)) = (p.kind, &p.table) {
            body.push_str(&table_xml(t));
            // Word braucht nach einer Tabelle einen Absatz (am Ende und zwischen Tabellen)
            if doc.paras.get(pi + 1).is_none_or(|n| n.kind == Kind::Table) {
                body.push_str("<w:p/>");
            }
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
        if p.kind == Kind::Object {
            // kein Bild verfügbar: Ersatztext
            let t = p.obj.as_ref().map(obj_text).unwrap_or_default();
            run_xml(&Run { text: t, st: CStyle::default() }, &mut body, false);
        } else if let (Kind::Caption, Some((label, n))) = (p.kind, x.nums.caps.get(pi).cloned().flatten()) {
            // „Abbildung “ + SEQ-Feld + „: “ + Text
            let plain = CStyle::default();
            run_xml(&Run { text: format!("{label} "), st: plain.clone() }, &mut body, false);
            body.push_str(&format!(r#"<w:fldSimple w:instr=" SEQ {} \* ARABIC "><w:r><w:t>{n}</w:t></w:r></w:fldSimple>"#, esc(&label.replace(' ', "_"))));
            run_xml(&Run { text: ": ".into(), st: plain }, &mut body, false);
            x.runs_out(p, &mut rels, &mut body);
        } else {
            x.runs_out(p, &mut rels, &mut body);
        }
        body.push_str("</w:p>");
    }
    let notes = std::mem::take(&mut x.notes);
    if !comments.is_empty() {
        rels.push(r#"Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/comments" Target="comments.xml""#.to_string());
    }
    let footnotes = if notes.is_empty() {
        String::new()
    } else {
        rels.push(r#"Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/footnotes" Target="footnotes.xml""#.to_string());
        let mut f = format!(
            r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?><w:footnotes xmlns:w="{W}"><w:footnote w:type="separator" w:id="-1"><w:p><w:pPr><w:spacing w:after="0" w:line="240" w:lineRule="auto"/></w:pPr><w:r><w:separator/></w:r></w:p></w:footnote><w:footnote w:type="continuationSeparator" w:id="0"><w:p><w:pPr><w:spacing w:after="0" w:line="240" w:lineRule="auto"/></w:pPr><w:r><w:continuationSeparator/></w:r></w:p></w:footnote>"#
        );
        for (i, t) in notes.iter().enumerate() {
            f.push_str(&format!(
                r#"<w:footnote w:id="{}"><w:p><w:pPr><w:pStyle w:val="FootnoteText"/></w:pPr><w:r><w:rPr><w:rStyle w:val="FootnoteReference"/></w:rPr><w:footnoteRef/></w:r><w:r><w:t xml:space="preserve"> {}</w:t></w:r></w:p></w:footnote>"#,
                i + 1,
                esc(t)
            ));
        }
        f.push_str("</w:footnotes>");
        f
    };
    let settings = format!(
        r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?><w:settings xmlns:w="{W}">{}{}</w:settings>"#,
        if doc.ex.track { "<w:trackRevisions/>" } else { "" },
        if notes.is_empty() { "" } else { r#"<w:footnotePr><w:footnote w:id="-1"/><w:footnote w:id="0"/></w:footnotePr>"# }
    );
    let wm = doc.ex.watermark.trim();
    let pg = &doc.page;
    let mut sect = String::new();
    let mut parts: Vec<(String, String)> = Vec::new();
    for (tag, text, file, center) in [("hdr", &doc.header, "header1.xml", false), ("ftr", &doc.footer, "footer1.xml", true)] {
        // ohne Text keine Kopfzeile, außer sie trägt das Wasserzeichen
        if text.trim().is_empty() && !(tag == "hdr" && !wm.is_empty()) {
            continue;
        }
        let kind = if tag == "hdr" { "header" } else { "footer" };
        rels.push(format!(r#"Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/{kind}" Target="{file}""#));
        sect.push_str(&format!(r#"<w:{kind}Reference w:type="default" r:id="rId{}"/>"#, rels.len()));
        parts.push((format!("word/{file}"), hf_xml(tag, text, center, wm)));
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
    if doc.ex.cols > 1 {
        sect.push_str(&format!(r#"<w:cols w:num="{}" w:space="720"/>"#, doc.ex.cols.min(3)));
    }
    let document = format!(r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?><w:document xmlns:w="{W}" xmlns:r="{R}" xmlns:wp="http://schemas.openxmlformats.org/drawingml/2006/wordprocessingDrawing" xmlns:a="http://schemas.openxmlformats.org/drawingml/2006/main" xmlns:pic="http://schemas.openxmlformats.org/drawingml/2006/picture"><w:body>{body}<w:sectPr>{sect}</w:sectPr></w:body></w:document>"#);
    let font = doc.family.word_name();
    let hs = |id: &str, name: &str, sz: i64, before: i64, after: i64, lvl: i64| {
        format!(
            r#"<w:style w:type="paragraph" w:styleId="{id}"><w:name w:val="{name}"/><w:basedOn w:val="Normal"/><w:next w:val="Normal"/><w:qFormat/><w:pPr><w:keepNext/><w:spacing w:before="{before}" w:after="{after}"/><w:outlineLvl w:val="{lvl}"/></w:pPr><w:rPr><w:rFonts w:ascii="Arial" w:hAnsi="Arial" w:cs="Arial"/><w:b/><w:sz w:val="{sz}"/><w:szCs w:val="{sz}"/></w:rPr></w:style>"#
        )
    };
    let styles = format!(
        r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?><w:styles xmlns:w="{W}"><w:docDefaults><w:rPrDefault><w:rPr><w:rFonts w:ascii="{font}" w:hAnsi="{font}" w:cs="{font}" w:eastAsia="{font}"/><w:sz w:val="{sz}"/><w:szCs w:val="{sz}"/><w:lang w:val="de-AT"/></w:rPr></w:rPrDefault><w:pPrDefault><w:pPr><w:spacing w:after="160" w:line="276" w:lineRule="auto"/></w:pPr></w:pPrDefault></w:docDefaults><w:style w:type="paragraph" w:default="1" w:styleId="Normal"><w:name w:val="Normal"/><w:qFormat/></w:style>{t}{h1}{h2}{h3}<w:style w:type="paragraph" w:styleId="Quote"><w:name w:val="Quote"/><w:basedOn w:val="Normal"/><w:qFormat/><w:pPr><w:ind w:left="360"/></w:pPr><w:rPr><w:i/><w:color w:val="4A4A55"/></w:rPr></w:style><w:style w:type="paragraph" w:styleId="Caption"><w:name w:val="caption"/><w:basedOn w:val="Normal"/><w:next w:val="Normal"/><w:qFormat/><w:pPr><w:spacing w:after="200" w:line="240" w:lineRule="auto"/></w:pPr><w:rPr><w:i/><w:color w:val="44546A"/><w:sz w:val="18"/><w:szCs w:val="18"/></w:rPr></w:style><w:style w:type="paragraph" w:styleId="FootnoteText"><w:name w:val="footnote text"/><w:basedOn w:val="Normal"/><w:pPr><w:spacing w:after="0" w:line="240" w:lineRule="auto"/></w:pPr><w:rPr><w:sz w:val="20"/><w:szCs w:val="20"/></w:rPr></w:style><w:style w:type="character" w:styleId="FootnoteReference"><w:name w:val="footnote reference"/><w:rPr><w:vertAlign w:val="superscript"/></w:rPr></w:style><w:style w:type="paragraph" w:styleId="ListParagraph"><w:name w:val="List Paragraph"/><w:basedOn w:val="Normal"/><w:qFormat/><w:pPr><w:spacing w:after="60"/><w:contextualSpacing/></w:pPr></w:style></w:styles>"#,
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
    let mut ct = String::from(r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?><Types xmlns="http://schemas.openxmlformats.org/package/2006/content-types"><Default Extension="rels" ContentType="application/vnd.openxmlformats-package.relationships+xml"/><Default Extension="xml" ContentType="application/xml"/><Default Extension="png" ContentType="image/png"/><Default Extension="jpg" ContentType="image/jpeg"/><Default Extension="jpeg" ContentType="image/jpeg"/><Default Extension="gif" ContentType="image/gif"/><Default Extension="bmp" ContentType="image/bmp"/><Default Extension="webp" ContentType="image/webp"/><Override PartName="/word/document.xml" ContentType="application/vnd.openxmlformats-officedocument.wordprocessingml.document.main+xml"/><Override PartName="/word/styles.xml" ContentType="application/vnd.openxmlformats-officedocument.wordprocessingml.styles+xml"/><Override PartName="/word/numbering.xml" ContentType="application/vnd.openxmlformats-officedocument.wordprocessingml.numbering+xml"/><Override PartName="/word/settings.xml" ContentType="application/vnd.openxmlformats-officedocument.wordprocessingml.settings+xml"/><Override PartName="/docProps/core.xml" ContentType="application/vnd.openxmlformats-package.core-properties+xml"/>"#);
    if !comments.is_empty() {
        ct.push_str(r#"<Override PartName="/word/comments.xml" ContentType="application/vnd.openxmlformats-officedocument.wordprocessingml.comments+xml"/>"#);
    }
    if !footnotes.is_empty() {
        ct.push_str(r#"<Override PartName="/word/footnotes.xml" ContentType="application/vnd.openxmlformats-officedocument.wordprocessingml.footnotes+xml"/>"#);
    }
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
    files.push(("word/settings.xml".into(), settings));
    if !comments.is_empty() {
        files.push(("word/comments.xml".into(), comments));
    }
    if !footnotes.is_empty() {
        files.push(("word/footnotes.xml".into(), footnotes));
    }
    files.extend(parts);
    for (n, c) in files {
        z.start_file(n, opt).map_err(|e| e.to_string())?;
        z.write_all(c.as_bytes()).map_err(|e| e.to_string())?;
    }
    for (n, b) in media {
        z.start_file(format!("word/media/{n}"), opt).map_err(|e| e.to_string())?;
        z.write_all(&b).map_err(|e| e.to_string())?;
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
    /// Bild-Beziehung → Dateiname im Bildspeicher.
    media: HashMap<String, String>,
    /// Bilder des gerade gelesenen Absatzes (mit Textumfluss).
    pending: RefCell<Vec<(Img, Wrap)>>,
    /// Zusatzdaten (Kommentare, Änderungen), die beim Lesen entstehen.
    ex: RefCell<Extras>,
    /// Word-Kommentar-ID → (Autor, Zeit, Text).
    cmt_words: HashMap<String, (String, String, String)>,
    /// Gerade offene Kommentarbereiche (Word-ID, eigene ID).
    cmt_stack: RefCell<Vec<(String, u32)>>,
    /// Word-Kommentar-ID → eigene ID (für Kommentare mit Bereich).
    cmt_used: RefCell<HashMap<String, u32>>,
    /// Seit Beginn des letzten Bereichs kam noch kein Text.
    cmt_fresh: StdCell<bool>,
    /// Fußnoten-ID → Text.
    notes: HashMap<String, String>,
    /// SEQ-Feld des Absatzes: (Name, Zeichen bis Feldende).
    seq: RefCell<Option<(String, usize)>>,
}

/// Argument eines Feldbefehls (`MERGEFIELD Name \* MERGEFORMAT` → `Name`), ohne Anführungszeichen.
fn instr_arg(instr: &str) -> String {
    let t = instr.trim();
    let rest = t.split_once(char::is_whitespace).map(|x| x.1).unwrap_or("");
    let rest = rest.split(" \\").next().unwrap_or("");
    rest.trim().trim_matches('"').to_string()
}

impl Ctx {
    /// Beginn eines Kommentarbereichs; direkt folgende Bereiche gelten als Antworten auf den vorigen.
    fn cmt_start(&self, wid: &str) {
        let Some((author, time, text)) = self.cmt_words.get(wid).cloned() else { return };
        let mut st = self.cmt_stack.borrow_mut();
        let mut ex = self.ex.borrow_mut();
        let top = st.last().map(|x| x.1).filter(|_| self.cmt_fresh.get());
        let id = match top {
            Some(top) => {
                if let Some(c) = ex.comments.iter_mut().find(|c| c.id == top) {
                    c.replies.push(Reply { author, time, text });
                }
                top
            }
            None => {
                let id = ex.id();
                ex.comments.push(Comment { id, author, time, text, ..Comment::default() });
                id
            }
        };
        self.cmt_used.borrow_mut().insert(wid.to_string(), id);
        st.push((wid.to_string(), id));
        self.cmt_fresh.set(true);
    }

    fn cmt_end(&self, wid: &str) {
        let mut st = self.cmt_stack.borrow_mut();
        if let Some(i) = st.iter().rposition(|(w, _)| w == wid) {
            st.remove(i);
        }
    }

    fn cmt_cur(&self) -> u32 {
        self.cmt_stack.borrow().last().map(|x| x.1).unwrap_or(0)
    }
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
                if st.cmt == 0 {
                    st.cmt = cx.cmt_cur();
                }
                let mut text = String::new();
                for t in c.children().filter(|t| t.is_element()) {
                    match t.tag_name().name() {
                        "t" | "delText" => text.push_str(t.text().unwrap_or("")),
                        "tab" => text.push('\t'),
                        "br" if wattr(t, "type") == Some("page") => *page_break = true,
                        "br" | "cr" => text.push(' '),
                        "noBreakHyphen" => text.push('-'),
                        "footnoteReference" => {
                            // Fußnote: eigenes Feld-Zeichen mit dem Text aus footnotes.xml
                            if !text.is_empty() {
                                out.push(Run { text: std::mem::take(&mut text), st: st.clone() });
                            }
                            let note = wattr(t, "id").and_then(|i| cx.notes.get(i)).cloned().unwrap_or_default();
                            out.push(Run { text: OBJ.to_string(), st: CStyle { field: Some(Field::Note(note)), va: VAlign::Base, ..st.clone() } });
                            cx.cmt_fresh.set(false);
                        }
                        "drawing" | "pict" | "object" => {
                            let rid = t.descendants().find(|b| matches!(b.tag_name().name(), "blip" | "imagedata")).and_then(|b| b.attributes().find(|a| matches!(a.name(), "embed" | "id")).map(|a| a.value()));
                            if let Some(file) = rid.and_then(|id| cx.media.get(id)) {
                                let emu = |k: &str| t.descendants().find(|e| e.tag_name().name() == "extent").and_then(|e| e.attribute(k)).and_then(|v| v.parse::<f32>().ok()).map(|v| v / 12700.0);
                                let (w, h) = match (emu("cx"), emu("cy")) {
                                    (Some(w), Some(h)) if w >= 1.0 && h >= 1.0 => (w, h),
                                    _ => (200.0, 150.0),
                                };
                                cx.pending.borrow_mut().push((Img { file: file.clone(), w, h }, wrap_of(t)));
                            }
                        }
                        _ => {}
                    }
                }
                if !text.is_empty() {
                    cx.cmt_fresh.set(false);
                    out.push(Run { text, st });
                }
            }
            "hyperlink" => {
                let target = c.attributes().find(|a| a.name() == "id").and_then(|a| cx.links.get(a.value())).cloned();
                runs_of(c, cx, base, target.as_deref().or(link), out, page_break);
            }
            "ins" | "del" => {
                // nachverfolgte Änderung: Läufe bekommen die Änderungs-ID
                let id = cx.ex.borrow_mut().rev(wattr(c, "author").unwrap_or(""), wattr(c, "date").unwrap_or(""));
                let mut b = base.clone();
                if c.tag_name().name() == "ins" {
                    b.ins = id;
                } else {
                    b.del = id;
                }
                runs_of(c, cx, &b, link, out, page_break);
            }
            "commentRangeStart" => {
                if let Some(id) = wattr(c, "id") {
                    cx.cmt_start(id);
                }
            }
            "commentRangeEnd" => {
                if let Some(id) = wattr(c, "id") {
                    cx.cmt_end(id);
                }
            }
            "fldSimple" => {
                let instr = wattr(c, "instr").unwrap_or("");
                let kw = instr.split_whitespace().next().unwrap_or("").to_ascii_uppercase();
                let field = match kw.as_str() {
                    "PAGE" => Some(Field::Page),
                    "NUMPAGES" => Some(Field::Pages),
                    "MERGEFIELD" => Some(Field::Merge(instr_arg(instr))),
                    "XE" => Some(Field::Index(instr_arg(instr))),
                    _ => None,
                };
                match field {
                    Some(f) => {
                        // ein Zeichen je Feld, Format vom ersten Ergebnislauf
                        let mut tmp = Vec::new();
                        runs_of(c, cx, base, None, &mut tmp, page_break);
                        let st = tmp.first().map(|r| r.st.clone()).unwrap_or_else(|| base.clone());
                        out.push(Run { text: OBJ.to_string(), st: CStyle { field: Some(f), link: None, ..st } });
                        cx.cmt_fresh.set(false);
                    }
                    None => {
                        runs_of(c, cx, base, link, out, page_break);
                        if kw == "SEQ" && cx.seq.borrow().is_none() {
                            let name = instr.split_whitespace().nth(1).unwrap_or("Abbildung").replace('_', " ");
                            let end = out.iter().map(|r| r.text.chars().count()).sum();
                            *cx.seq.borrow_mut() = Some((name, end));
                        }
                    }
                }
            }
            "smartTag" | "sdt" | "sdtContent" => runs_of(c, cx, base, link, out, page_break),
            _ => {}
        }
    }
}

/// Textumfluss eines Bildes: verankert mit Umfluss und Ausrichtung links/rechts, sonst eigene Zeile.
fn wrap_of(t: roxmltree::Node) -> Wrap {
    let Some(a) = t.descendants().find(|e| e.tag_name().name() == "anchor") else { return Wrap::Inline };
    let sq = a.children().find(|e| matches!(e.tag_name().name(), "wrapSquare" | "wrapTight" | "wrapThrough"));
    let Some(sq) = sq else { return Wrap::Inline };
    let align = a.children().find(|e| e.tag_name().name() == "positionH").and_then(|h| h.children().find(|e| e.tag_name().name() == "align")).and_then(|e| e.text()).unwrap_or("");
    match (align, sq.attribute("wrapText")) {
        ("left", _) => Wrap::Left,
        ("right", _) => Wrap::Right,
        (_, Some("left")) => Wrap::Right,
        (_, Some("right")) => Wrap::Left,
        _ => Wrap::Inline,
    }
}

/// Erste `n` Zeichen der Läufe entfernen.
fn cut_chars(runs: &mut [Run], mut n: usize) {
    for r in runs.iter_mut() {
        if n == 0 {
            break;
        }
        let k = r.text.chars().count().min(n);
        r.text = r.text.chars().skip(k).collect();
        n -= k;
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
        cmt: a.cmt,
        ins: a.ins,
        del: a.del,
        ..CStyle::default()
    }
}

/// Vorspann „Abbildung 3“ am Anfang eines Beschriftungs-Textes: (Art, Zeichenzahl); ohne Treffer leer.
fn caption_prefix(runs: &[Run]) -> (String, usize) {
    let t: String = runs.iter().map(|r| r.text.as_str()).collect();
    let label: String = t.chars().take_while(|c| c.is_alphabetic()).collect();
    let rest = &t[label.len()..];
    let sp = rest.chars().take_while(|c| c.is_whitespace()).count();
    let digits = rest.chars().skip(sp).take_while(|c| c.is_ascii_digit()).count();
    if label.is_empty() || sp == 0 || digits == 0 {
        return (String::new(), 0);
    }
    (label.clone(), label.chars().count() + sp + digits)
}

fn para_of(n: roxmltree::Node, cx: &Ctx, out: &mut Vec<Para>) {
    let ppr = child(n, "pPr");
    cx.seq.take();
    let mut kind = Kind::Body;
    let mut base = CStyle::default();
    let mut indent = 0u8;
    if let Some(sid) = ppr.and_then(|p| child(p, "pStyle")).and_then(|s| wattr(s, "val")) {
        if let Some((k, cs)) = cx.styles.get(sid) {
            kind = k.unwrap_or(Kind::Body);
            base = cs.clone();
            // Vorlagen von Überschriften bringen ihr Format über die eigene Vorlage mit
            if kind.is_heading() || kind == Kind::Caption {
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
    let mut cap = String::new();
    if kind == Kind::Caption {
        // „Abbildung n: “ kommt beim Satz wieder dazu: Vorspann entfernen
        let (label, n) = match cx.seq.take() {
            Some(x) => x,
            None => caption_prefix(&runs),
        };
        if n > 0 {
            cut_chars(&mut runs, n);
            let t: String = runs.iter().map(|r| r.text.as_str()).collect();
            cut_chars(&mut runs, t.chars().take_while(|c| matches!(c, ':' | '.') || c.is_whitespace()).count());
            cap = label;
        }
    }
    let mut p = Para { kind, align, indent, line, runs, cap, ..Para::default() };
    p.normalize();
    let empty = p.is_empty();
    let pics = std::mem::take(&mut *cx.pending.borrow_mut());
    if !(empty && (pb || !pics.is_empty())) {
        out.push(p);
    }
    for (im, wrap) in pics {
        out.push(Para { align, img: Some(im), wrap, ..Para::new(Kind::Image) });
    }
    if pb {
        out.push(Para::new(Kind::PageBreak));
    }
    let _ = cx.doc_family;
}

/// Zelle: Absätze als Text (durch `\n` verbunden); fett/kursiv, wenn alle Läufe so sind.
fn cell_of(tc: roxmltree::Node, cx: &Ctx) -> Cell {
    let mut lines = Vec::new();
    let (mut any, mut all_b, mut all_i, mut color) = (false, true, true, 0u32);
    let mut align = None;
    for p in tc.children().filter(|p| p.is_element() && p.tag_name().name() == "p") {
        if align.is_none() {
            align = child(p, "pPr").and_then(|x| child(x, "jc")).and_then(|j| wattr(j, "val")).map(|v| match v {
                "center" => Align::Center,
                "right" | "end" => Align::Right,
                "both" | "distribute" => Align::Justify,
                _ => Align::Left,
            });
        }
        let (mut runs, mut pb) = (Vec::new(), false);
        runs_of(p, cx, &CStyle::default(), None, &mut runs, &mut pb);
        let mut line = String::new();
        for r in runs.iter().filter(|r| r.st.del == 0) {
            line.extend(r.text.chars().filter(|c| *c != OBJ));
            if !r.text.trim().is_empty() {
                any = true;
                all_b &= r.st.b;
                all_i &= r.st.i;
                if color == 0 {
                    color = r.st.color;
                }
            }
        }
        lines.push(line);
    }
    cx.pending.borrow_mut().clear();
    let bg = child(tc, "tcPr").and_then(|p| child(p, "shd")).and_then(|s| wattr(s, "fill")).filter(|v| *v != "auto").map(color_of).unwrap_or(0);
    Cell { text: lines.join("\n"), b: any && all_b, i: any && all_i, align: align.unwrap_or_default(), bg, color }
}

/// Tabelle (`w:tbl`) als Tabellen-Absatz.
fn table_of(tbl: roxmltree::Node, cx: &Ctx) -> Para {
    let mut cols: Vec<f32> = child(tbl, "tblGrid")
        .map(|g| g.children().filter(|c| c.is_element() && c.tag_name().name() == "gridCol").filter_map(|c| wattr(c, "w").and_then(|v| v.parse::<f32>().ok())).map(|v| v / 20.0).collect())
        .unwrap_or_default();
    let mut rows: Vec<Vec<Cell>> = Vec::new();
    let mut header = false;
    for (ri, tr) in tbl.children().filter(|r| r.is_element() && r.tag_name().name() == "tr").enumerate() {
        if ri == 0 {
            header = child(tr, "trPr").is_some_and(|p| child(p, "tblHeader").is_some());
        }
        let mut row = Vec::new();
        for tc in tr.children().filter(|x| x.is_element() && x.tag_name().name() == "tc") {
            row.push(cell_of(tc, cx));
            let span = child(tc, "tcPr").and_then(|p| child(p, "gridSpan")).and_then(|g| wattr(g, "val")).and_then(|v| v.parse::<usize>().ok()).unwrap_or(1).clamp(1, 20);
            for _ in 1..span {
                row.push(Cell::default());
            }
        }
        rows.push(row);
    }
    rows.truncate(1000);
    let n = rows.iter().map(Vec::len).max().unwrap_or(1).clamp(1, 20);
    cols.truncate(20);
    while cols.len() < n {
        cols.push(72.0);
    }
    let pr = child(tbl, "tblPr");
    let border = match pr.and_then(|p| child(p, "tblBorders")) {
        Some(b) => ["top", "left", "bottom", "right", "insideH", "insideV"]
            .iter()
            .find_map(|s| child(b, s).filter(|e| !matches!(wattr(*e, "val"), Some("none" | "nil"))))
            .map(|e| match wattr(e, "color").filter(|c| *c != "auto").map(color_of) {
                Some(c) if c != 0 => c,
                _ => 0xFF000000,
            })
            .unwrap_or(0),
        None if pr.and_then(|p| child(p, "tblStyle")).and_then(|s| wattr(s, "val")).is_some_and(|v| v.contains("Grid")) => 0xFFB8BCC6,
        None => 0,
    };
    let mut t = Table { cols, rows, header, stripes: false, border };
    t.fix();
    Para { kind: Kind::Table, table: Some(t), runs: vec![Run::default()], ..Para::default() }
}

fn block(n: roxmltree::Node, cx: &Ctx, out: &mut Vec<Para>) {
    for c in n.children().filter(|c| c.is_element()) {
        match c.tag_name().name() {
            "p" => para_of(c, cx, out),
            "tbl" => out.push(table_of(c, cx)),
            "sdt" | "sdtContent" | "customXml" => block(c, cx, out),
            _ => {}
        }
    }
}

/// Liest eine DOCX-Datei.
pub fn import(bytes: &[u8]) -> Result<(TextDoc, Images), String> {
    let mut z = zip::ZipArchive::new(Cursor::new(bytes)).map_err(|_| "Das ist keine Word-Datei (.docx).".to_string())?;
    let doc_xml = read_entry(&mut z, "word/document.xml").ok_or("In der Datei fehlt der Text (word/document.xml).")?;
    let styles_xml = read_entry(&mut z, "word/styles.xml").unwrap_or_default();
    let num_xml = read_entry(&mut z, "word/numbering.xml").unwrap_or_default();
    let rels_xml = read_entry(&mut z, "word/_rels/document.xml.rels").unwrap_or_default();
    let opt = roxmltree::ParsingOptions { allow_dtd: false, ..Default::default() };
    let mut cx = Ctx { styles: HashMap::new(), nums: HashMap::new(), links: HashMap::new(), doc_family: Family::Sans, media: HashMap::new(),
        pending: RefCell::new(Vec::new()),
        ex: RefCell::new(Extras::default()),
        cmt_words: HashMap::new(),
        cmt_stack: RefCell::new(Vec::new()),
        cmt_used: RefCell::new(HashMap::new()),
        cmt_fresh: StdCell::new(false),
        notes: HashMap::new(),
        seq: RefCell::new(None),
    };
    let mut store = crate::doc::ImageStore::default();
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
                "caption" | "beschriftung" => Some(Kind::Caption),
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
            if let (true, Some(id), Some(t)) = (r.attribute("Type").is_some_and(|t| t.ends_with("/image")), r.attribute("Id"), r.attribute("Target")) {
                let path = if let Some(abs) = t.strip_prefix('/') { abs.to_string() } else { format!("word/{}", t.trim_start_matches("./")) };
                let mut buf = Vec::new();
                if let Ok(mut e) = z.by_name(&path) {
                    let _ = e.by_ref().take(64 << 20).read_to_end(&mut buf);
                }
                // nur Rasterbilder, die wir lesen können (keine EMF/WMF)
                if let Ok(fmt) = image::guess_format(&buf) {
                    let ext = fmt.extensions_str().first().copied().unwrap_or("png");
                    cx.media.insert(id.to_string(), store.add(buf, ext));
                }
            }
            if r.attribute("Type").is_some_and(|t| t.ends_with("/hyperlink")) {
                if let (Some(id), Some(t)) = (r.attribute("Id"), r.attribute("Target")) {
                    cx.links.insert(id.to_string(), t.to_string());
                }
            }
        }
    }
    // Fußnoten, Kommentare, Einstellungen (fehlende oder kaputte Teile werden übergangen)
    let para_texts = |n: roxmltree::Node| -> String {
        n.children().filter(|p| p.is_element() && p.tag_name().name() == "p").map(|p| p.descendants().filter(|t| t.tag_name().name() == "t").filter_map(|t| t.text()).collect::<String>()).collect::<Vec<_>>().join("\n")
    };
    let mut cmt_order: Vec<String> = Vec::new();
    if let Some(fx) = read_entry(&mut z, "word/footnotes.xml") {
        if let Ok(fd) = roxmltree::Document::parse_with_options(&fx, opt) {
            for f in fd.descendants().filter(|n| n.tag_name().name() == "footnote") {
                if let (Some(id), None | Some("normal")) = (wattr(f, "id"), wattr(f, "type")) {
                    cx.notes.insert(id.to_string(), para_texts(f).trim().to_string());
                }
            }
        }
    }
    if let Some(cxml) = read_entry(&mut z, "word/comments.xml") {
        if let Ok(cd) = roxmltree::Document::parse_with_options(&cxml, opt) {
            for c in cd.descendants().filter(|n| n.tag_name().name() == "comment") {
                if let Some(id) = wattr(c, "id") {
                    cmt_order.push(id.to_string());
                    cx.cmt_words.insert(id.to_string(), (wattr(c, "author").unwrap_or("").to_string(), wattr(c, "date").unwrap_or("").to_string(), para_texts(c)));
                }
            }
        }
    }
    if let Some(sx) = read_entry(&mut z, "word/settings.xml") {
        if let Ok(sd) = roxmltree::Document::parse_with_options(&sx, opt) {
            cx.ex.borrow_mut().track = sd.descendants().find(|n| n.tag_name().name() == "trackRevisions").is_some_and(|n| on(Some(n)));
        }
    }
    // Wasserzeichen (VML-Textpfad in einer Kopfzeile)
    let hdrs: Vec<String> = (0..z.len()).filter_map(|i| z.by_index_raw(i).ok().map(|f| String::from_utf8_lossy(f.name_raw()).into_owned())).filter(|n| n.starts_with("word/header") && n.ends_with(".xml")).collect();
    for h in hdrs {
        let wm = read_entry(&mut z, &h).and_then(|x| {
            let d = roxmltree::Document::parse_with_options(&x, opt).ok()?;
            let r = d.descendants().filter(|n| n.tag_name().name() == "textpath").find_map(|n| n.attribute("string").map(String::from));
            r
        });
        if let Some(w) = wm.filter(|w| !w.trim().is_empty()) {
            cx.ex.borrow_mut().watermark = w;
            break;
        }
    }
    let d = roxmltree::Document::parse_with_options(&doc_xml, opt).map_err(|e| format!("Word-Datei ist beschädigt: {e}"))?;
    let body = d.descendants().find(|n| n.tag_name().name() == "body").ok_or("Word-Datei ohne Inhalt")?;
    let mut paras = Vec::new();
    block(body, &cx, &mut paras);
    if let Some(sp) = body.descendants().rfind(|n| n.tag_name().name() == "sectPr") {
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
    // Kommentare ohne Bereich bleiben als Kommentare erhalten
    for wid in &cmt_order {
        if !cx.cmt_used.borrow().contains_key(wid) {
            if let Some((author, time, text)) = cx.cmt_words.get(wid).cloned() {
                let mut ex = cx.ex.borrow_mut();
                let id = ex.id();
                ex.comments.push(Comment { id, author, time, text, ..Comment::default() });
            }
        }
    }
    if let Some(sp) = body.descendants().rfind(|n| n.tag_name().name() == "sectPr") {
        let n = child(sp, "cols").and_then(|c| wattr(c, "num")).and_then(|v| v.parse::<u8>().ok()).unwrap_or(1);
        cx.ex.borrow_mut().cols = n.clamp(1, 3);
    }
    // leere Absätze, die Word nach/zwischen Tabellen verlangt, wieder entfernen
    let mut i = 1;
    while i < paras.len() {
        let leer = paras[i].kind == Kind::Body && paras[i].is_empty() && paras[i].id == 0;
        if leer && paras[i - 1].kind == Kind::Table && paras.get(i + 1).is_none_or(|n| n.kind == Kind::Table) {
            paras.remove(i);
        } else {
            i += 1;
        }
    }
    let cw = td.page.content_w();
    for p in paras.iter_mut() {
        if let Some(t) = &mut p.table {
            if t.cols.iter().sum::<f32>() > cw + 1.0 {
                t.fit(cw);
            }
        }
    }
    td.ex = cx.ex.take();
    while paras.last().is_some_and(|p| p.kind == Kind::PageBreak) {
        paras.pop();
    }
    if !paras.is_empty() {
        td.paras = paras;
    }
    Ok((td, store.files))
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
        let bytes = export(&d, "Bericht <1>", &Images::new()).unwrap();
        let (back, _) = import(&bytes).unwrap();
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

    fn feld(f: Field) -> CStyle {
        CStyle { field: Some(f), ..CStyle::default() }
    }

    #[test]
    fn tabelle_hin_und_zurueck() {
        let mut t = Table::new(3, 2, 300.0);
        t.header = true;
        t.stripes = true;
        t.rows[0][0] = Cell { text: "Name".into(), b: true, ..Cell::default() };
        t.rows[1][0] = Cell { text: "Zeile 1\nZeile 2".into(), i: true, align: Align::Right, bg: 0xFFFDE68A, ..Cell::default() };
        t.rows[1][1] = Cell::new("42");
        let mut d = TextDoc::new();
        d.paras = vec![Para::with_text(Kind::Body, "Vor"), Para { kind: Kind::Table, table: Some(t.clone()), runs: vec![Run::default()], ..Para::default() }];
        let (back, _) = import(&export(&d, "T", &Images::new()).unwrap()).unwrap();
        // der Absatz nach der letzten Tabelle ist nur für Word
        assert_eq!(back.paras.iter().map(|p| p.kind).collect::<Vec<_>>(), [Kind::Body, Kind::Table]);
        let bt = back.paras[1].table.as_ref().unwrap();
        assert!(bt.header && bt.border == t.border);
        assert!(bt.cols.iter().zip(&t.cols).all(|(a, b)| (a - b).abs() < 0.1));
        assert_eq!(bt.rows[0][0].text, "Name");
        assert!(bt.rows[0][0].b);
        assert_eq!(bt.rows[1][0].text, "Zeile 1\nZeile 2");
        assert!(bt.rows[1][0].i && !bt.rows[1][0].b);
        assert_eq!(bt.rows[1][0].align, Align::Right);
        assert_eq!(bt.rows[1][0].bg, 0xFFFDE68A);
        assert_eq!(bt.rows[1][1].text, "42");
        assert_eq!(bt.rows[2][1].bg, 0xFFF3F4F6, "Streifen als Zellfarbe");
    }

    #[test]
    fn fussnote_kommentar_aenderungen_felder() {
        let mut d = TextDoc::new();
        d.ex.comments.push(Comment { id: 5, author: "Eva Kern".into(), time: "2026-01-02T10:00:00Z".into(), text: "Prüfen\nbitte".into(), replies: vec![Reply { author: "Max".into(), time: String::new(), text: "Erledigt".into() }], done: false });
        let r1 = d.ex.rev("Eva", "2026-01-02T10:00:00Z");
        let r2 = d.ex.rev("Max", "2026-01-03T11:00:00Z");
        d.ex.track = true;
        d.ex.cols = 2;
        let o = OBJ.to_string();
        let run = |t: &str, st: CStyle| Run { text: t.into(), st };
        d.paras = vec![Para {
            runs: vec![
                run("Anfang ", CStyle::default()),
                run("kommentiert", CStyle { cmt: 5, b: true, ..CStyle::default() }),
                run(" Note", CStyle::default()),
                run(&o, feld(Field::Note("Fußnotentext".into()))),
                run(" neu", CStyle { ins: r1, ..CStyle::default() }),
                run(" alt", CStyle { del: r2, ..CStyle::default() }),
                run(" Seite ", CStyle::default()),
                run(&o, feld(Field::Page)),
                run("/", CStyle::default()),
                run(&o, feld(Field::Pages)),
                run(" ", CStyle::default()),
                run(&o, feld(Field::Merge("Kunde".into()))),
                run(&o, feld(Field::Index("Stichwort".into()))),
                run(&o, feld(Field::Check(true))),
            ],
            ..Para::default()
        }];
        let bytes = export(&d, "F", &Images::new()).unwrap();
        let mut z = zip::ZipArchive::new(Cursor::new(&bytes[..])).unwrap();
        let notes = read_entry(&mut z, "word/footnotes.xml").unwrap();
        assert!(notes.contains(r#"w:id="-1""#) && notes.contains("Fußnotentext"));
        let doc_xml = read_entry(&mut z, "word/document.xml").unwrap();
        assert!(doc_xml.contains("<w:ins ") && doc_xml.contains("<w:del ") && doc_xml.contains("w:delText"));
        assert!(doc_xml.contains(r#"w:instr=" PAGE ""#) && doc_xml.contains("NUMPAGES") && doc_xml.contains("MERGEFIELD Kunde") && doc_xml.contains("XE"));
        assert!(doc_xml.contains(r#"<w:cols w:num="2""#));
        assert!(read_entry(&mut z, "word/settings.xml").unwrap().contains("trackRevisions"));
        // alle Teile müssen wohlgeformtes XML sein
        for n in ["word/document.xml", "word/comments.xml", "word/footnotes.xml", "word/settings.xml", "word/styles.xml", "[Content_Types].xml", "word/_rels/document.xml.rels"] {
            roxmltree::Document::parse(&read_entry(&mut z, n).unwrap_or_else(|| panic!("{n} fehlt"))).unwrap_or_else(|e| panic!("{n}: {e}"));
        }
        let (back, _) = import(&bytes).unwrap();
        assert!(back.ex.track);
        assert_eq!(back.ex.cols, 2);
        // Kommentar mit Antwort
        assert_eq!(back.ex.comments.len(), 1);
        let c = &back.ex.comments[0];
        assert_eq!((c.author.as_str(), c.text.as_str()), ("Eva Kern", "Prüfen\nbitte"));
        assert_eq!(c.replies.len(), 1);
        let p = &back.paras[0];
        let kom = p.runs.iter().find(|r| r.st.cmt != 0).unwrap();
        assert_eq!((kom.text.as_str(), kom.st.cmt, kom.st.b), ("kommentiert", c.id, true));
        // Änderungen
        let ins = p.runs.iter().find(|r| r.st.ins > 0).unwrap();
        let del = p.runs.iter().find(|r| r.st.del > 0).unwrap();
        assert_eq!((ins.text.as_str(), del.text.as_str()), (" neu", " alt"));
        assert_eq!(back.ex.revs.iter().find(|r| r.id == ins.st.ins).unwrap().author, "Eva");
        assert_eq!(back.ex.revs.iter().find(|r| r.id == del.st.del).unwrap().author, "Max");
        // Felder
        let fields: Vec<&Field> = p.runs.iter().filter_map(|r| r.st.field.as_ref()).collect();
        assert_eq!(fields[0], &Field::Note("Fußnotentext".into()));
        assert!(fields.contains(&&Field::Page) && fields.contains(&&Field::Pages));
        assert!(fields.contains(&&Field::Merge("Kunde".into())) && fields.contains(&&Field::Index("Stichwort".into())));
        assert!(p.runs.iter().filter(|r| r.st.field.is_some()).all(|r| r.text.chars().all(|c| c == OBJ)));
        assert!(p.text().contains("Anfang kommentiert"));
        assert!(p.text().ends_with('☒'), "Kontrollkästchen als Text");
    }

    #[test]
    fn beschriftung_hin_und_zurueck() {
        let mut d = TextDoc::new();
        d.paras = vec![Para::with_text(Kind::Body, "Text"), Para { cap: "Abbildung".into(), ..Para::with_text(Kind::Caption, "Ein Haus") }, Para { cap: "Abbildung".into(), ..Para::with_text(Kind::Caption, "Ein Baum") }];
        let bytes = export(&d, "B", &Images::new()).unwrap();
        let mut z = zip::ZipArchive::new(Cursor::new(&bytes[..])).unwrap();
        let x = read_entry(&mut z, "word/document.xml").unwrap();
        assert!(x.contains(r#"SEQ Abbildung \* ARABIC"#) && x.contains(r#"w:val="Caption""#));
        let (back, _) = import(&bytes).unwrap();
        assert_eq!(back.paras[1].kind, Kind::Caption);
        assert_eq!(back.paras[1].cap, "Abbildung");
        assert_eq!(back.paras[1].text(), "Ein Haus");
        assert_eq!(back.paras[2].text(), "Ein Baum");
    }

    #[test]
    fn objekt_ersatztext_und_umfluss() {
        let mut pm = tiny_skia::Pixmap::new(3, 3).unwrap();
        pm.fill(tiny_skia::Color::from_rgba8(255, 0, 0, 255));
        let mut st = crate::doc::ImageStore::default();
        let name = st.add(pm.encode_png().unwrap(), "png");
        let mut d = TextDoc::new();
        let mut img = Para::image(&name, 100.0, 50.0);
        img.wrap = Wrap::Right;
        let mut left = Para::image(&name, 100.0, 50.0);
        left.wrap = Wrap::Left;
        let formel = Para { kind: Kind::Object, obj: Some(Obj { kind: ObjKind::Formula { src: "a^2+b^2".into() }, w: 100.0, h: 40.0 }), runs: vec![Run::default()], ..Para::default() };
        d.paras = vec![img, left, Para::with_text(Kind::Body, "Fließtext"), formel];
        let bytes = export(&d, "O", &st.files).unwrap();
        let mut z = zip::ZipArchive::new(Cursor::new(&bytes[..])).unwrap();
        let x = read_entry(&mut z, "word/document.xml").unwrap();
        assert!(x.contains("<wp:anchor") && x.contains("wrapSquare") && !x.contains("<wp:inline"));
        let (back, _) = import(&bytes).unwrap();
        let w: Vec<Wrap> = back.paras.iter().filter(|p| p.kind == Kind::Image).map(|p| p.wrap).collect();
        assert_eq!(w, [Wrap::Right, Wrap::Left]);
        assert!(back.plain_text().contains("[Formel: a^2+b^2]") || super::super::render::object_png(d.paras[3].obj.as_ref().unwrap(), 2.0).is_some());
    }

    #[test]
    fn wasserzeichen_wird_geschrieben_und_gelesen() {
        let mut d = TextDoc::new();
        d.ex.watermark = "ENTWURF".into();
        let bytes = export(&d, "W", &Images::new()).unwrap();
        let (back, _) = import(&bytes).unwrap();
        assert_eq!(back.ex.watermark, "ENTWURF");
    }

    #[test]
    fn kein_docx() {
        assert!(import(b"kein zip").is_err());
    }

    #[test]
    fn bilder_hin_und_zurueck() {
        let mut pm = tiny_skia::Pixmap::new(3, 3).unwrap();
        pm.fill(tiny_skia::Color::from_rgba8(0, 0, 255, 255));
        let png = pm.encode_png().unwrap();
        let mut st = crate::doc::ImageStore::default();
        let name = st.add(png.clone(), "png");
        let mut d = TextDoc::new();
        d.paras = vec![Para::with_text(Kind::Body, "Text"), Para::image(&name, 300.0, 150.0), Para::with_text(Kind::Body, "Ende")];
        let bytes = export(&d, "Bild", &st.files).unwrap();
        let (back, imgs) = import(&bytes).unwrap();
        let im = back.paras.iter().find_map(|p| p.img.clone()).expect("Bild gelesen");
        assert_eq!(imgs.get(&im.file), Some(&png));
        assert!((im.w - 300.0).abs() < 0.5 && (im.h - 150.0).abs() < 0.5);
        assert_eq!(back.paras.iter().map(|p| p.kind).collect::<Vec<_>>(), [Kind::Body, Kind::Image, Kind::Body]);
        assert_eq!(back.paras[1].align, Align::Center);
    }
}
