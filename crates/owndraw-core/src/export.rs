//! Export: PDF (Seite mit eingebettetem Bild), Ebenen als PNG und animiertes Website-Paket (Gegenstueck zu WebExport.java).

use crate::board::Board;
use crate::geom::RectF;
use crate::layer::*;
use std::collections::HashSet;
use std::io::{Seek, Write};
use std::path::{Path, PathBuf};
use tiny_skia::Pixmap;
use zip::write::SimpleFileOptions;

pub mod formats;
pub mod interchange;
pub mod layered;
pub mod office;
pub mod pdfv;
pub mod raster;
pub mod vector;

/// Minimale PDF-Datei mit einer Seite (Groesse pw × ph Punkt), die das Bild ganzflaechig zeigt (RGB, FlateDecode).
pub fn pdf_from_pixmap(pm: &Pixmap, pw: f32, ph: f32) -> Vec<u8> {
    pdf_pages(1, |_| Some((pm.clone(), pw, ph))).unwrap_or_default()
}

/// PDF mit n Seiten; `page(i)` liefert das Bild der Seite i und ihre Groesse in Punkt (das Bild fuellt die Seite).
/// Jede Seite wird sofort komprimiert, damit nie alle Bilder gleichzeitig im Speicher liegen. None, wenn eine Seite fehlt.
pub fn pdf_pages(n: usize, mut page: impl FnMut(usize) -> Option<(Pixmap, f32, f32)>) -> Option<Vec<u8>> {
    let n = n.max(1);
    let mut out: Vec<u8> = Vec::new();
    let mut offs = Vec::new();
    out.extend_from_slice(b"%PDF-1.4\n%\xE2\xE3\xCF\xD3\n");
    let mut obj = |out: &mut Vec<u8>, body: &[u8]| {
        offs.push(out.len());
        out.extend_from_slice(format!("{} 0 obj\n", offs.len()).as_bytes());
        out.extend_from_slice(body);
        out.extend_from_slice(b"\nendobj\n");
    };
    // Objekte: 1 Katalog, 2 Seitenbaum, je Seite i: 3+3i Seite, 4+3i Bild, 5+3i Inhalt
    obj(&mut out, b"<< /Type /Catalog /Pages 2 0 R >>");
    let kids: Vec<String> = (0..n).map(|i| format!("{} 0 R", 3 + 3 * i)).collect();
    obj(&mut out, format!("<< /Type /Pages /Kids [{}] /Count {n} >>", kids.join(" ")).as_bytes());
    for i in 0..n {
        let (pm, pw, ph) = page(i)?;
        let (w, h) = (pm.width(), pm.height());
        let mut rgb = Vec::with_capacity((w * h * 3) as usize);
        for px in pm.pixels() {
            let c = px.demultiply();
            rgb.extend_from_slice(&[c.red(), c.green(), c.blue()]);
        }
        drop(pm);
        let data = miniz_oxide::deflate::compress_to_vec_zlib(&rgb, 6);
        let (io, co) = (4 + 3 * i, 5 + 3 * i);
        obj(
            &mut out,
            format!(
                "<< /Type /Page /Parent 2 0 R /MediaBox [0 0 {pw:.2} {ph:.2}] /Resources << /XObject << /Im0 {io} 0 R >> >> /Contents {co} 0 R >>"
            )
            .as_bytes(),
        );
        let mut v = format!(
            "<< /Type /XObject /Subtype /Image /Width {w} /Height {h} /ColorSpace /DeviceRGB /BitsPerComponent 8 /Filter /FlateDecode /Length {} >>\nstream\n",
            data.len()
        )
        .into_bytes();
        v.extend_from_slice(&data);
        v.extend_from_slice(b"\nendstream");
        obj(&mut out, &v);
        let content = format!("q {pw:.2} 0 0 {ph:.2} 0 0 cm /Im0 Do Q\n");
        let mut v = format!("<< /Length {} >>\nstream\n", content.len()).into_bytes();
        v.extend_from_slice(content.as_bytes());
        v.extend_from_slice(b"endstream");
        obj(&mut out, &v);
    }
    let xref = out.len();
    out.extend_from_slice(format!("xref\n0 {}\n0000000000 65535 f \n", offs.len() + 1).as_bytes());
    for o in &offs {
        out.extend_from_slice(format!("{o:010} 00000 n \n").as_bytes());
    }
    out.extend_from_slice(format!("trailer\n<< /Size {} /Root 1 0 R >>\nstartxref\n{xref}\n%%EOF\n", offs.len() + 1).as_bytes());
    Some(out)
}

/// Exportiert eine einzelne Ebene als transparentes PNG, zugeschnitten auf ihren Inhalt. None wenn leer/Fehler.
pub fn layer_png(v: &Board, l: &Layer) -> Option<Vec<u8>> {
    let mut b: Option<RectF> = None;
    for it in v.items.iter().filter(|it| it.layer == l.id) {
        match b.as_mut() {
            None => b = Some(it.bounds()),
            Some(r) => r.union(&it.bounds()),
        }
    }
    let mut b = b?;
    let m = -(b.width().max(b.height()) * 0.02) - 4.0;
    b.inset(m, m);
    let k = (4096.0 / b.width().max(b.height())).min(3.0);
    v.render_layer(l, &b, k)?.encode_png().ok()
}

/// Schreibt das komplette Website-Paket als ZIP. false bei leerem Board/Fehler.
pub fn write_zip<W: Write + Seek>(v: &Board, board_name: &str, os: W) -> bool {
    (|| -> Option<()> {
        let p = plan(v, 1920.0)?;
        let files: Vec<String> = p.names.iter().map(|n| format!("ebenen/{n}.png")).collect();
        let mut z = zip::ZipWriter::new(os);
        let opt = SimpleFileOptions::default().compression_method(zip::CompressionMethod::Deflated);
        let stored = SimpleFileOptions::default().compression_method(zip::CompressionMethod::Stored);
        z.start_file("index.html", opt).ok()?;
        z.write_all(html(board_name, v.bg_color(), p.w, p.h, &p.ls, &files).as_bytes()).ok()?;
        let comp = v.render_composite(&p.b, p.k, true)?.encode_png().ok()?;
        z.start_file("00-vorschau.png", stored).ok()?;
        z.write_all(&comp).ok()?;
        for (i, l) in p.ls.iter().enumerate() {
            let png = v.render_layer(l, &p.b, p.k)?.encode_png().ok()?;
            z.start_file(files[i].as_str(), stored).ok()?;
            z.write_all(&png).ok()?;
        }
        z.start_file("prompt.txt", opt).ok()?;
        z.write_all(prompt(p.w, p.h, &p.ls, &p.names).as_bytes()).ok()?;
        z.start_file("LIESMICH.txt", opt).ok()?;
        z.write_all(README.as_bytes()).ok()?;
        z.finish().ok()?;
        Some(())
    })()
    .is_some()
}

/// Erzeugt cache_dir/preview/index.html + ebenen/*.png (lange Kante max. 1600 px); None bei leerem Board/Fehler.
pub fn build_preview(cache_dir: &Path, v: &Board) -> Option<PathBuf> {
    let p = plan(v, 1600.0)?;
    let dir = cache_dir.join("preview");
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(dir.join("ebenen")).ok()?;
    let files: Vec<String> = p.names.iter().map(|n| format!("ebenen/{n}.png")).collect();
    for (i, l) in p.ls.iter().enumerate() {
        let png = v.render_layer(l, &p.b, p.k)?.encode_png().ok()?;
        std::fs::write(dir.join(&files[i]), png).ok()?;
    }
    let index = dir.join("index.html");
    std::fs::write(&index, html("Vorschau", v.bg_color(), p.w, p.h, &p.ls, &files)).ok()?;
    Some(index)
}

struct Plan {
    b: RectF,
    k: f32,
    w: i32,
    h: i32,
    ls: Vec<Layer>,
    names: Vec<String>,
}

fn plan(v: &Board, long_edge: f32) -> Option<Plan> {
    let e = v.export_bounds()?;
    let ls: Vec<Layer> = v.layers.iter().filter(|l| l.visible).cloned().collect();
    if ls.is_empty() {
        return None;
    }
    let (bw, bh) = (e.width().max(1.0), e.height().max(1.0));
    let m = bw.max(bh) * 0.02;
    let b = RectF::new(e.left - m, e.top - m, e.left + bw + m, e.top + bh + m);
    let k = (long_edge / b.width().max(b.height())).min(3.0);
    let w = ((b.width() * k).round() as i32).max(1);
    let h = ((b.height() * k).round() as i32).max(1);
    let names = names(&ls);
    Some(Plan { b, k, w, h, ls, names })
}

/// Eindeutige Dateinamen (ohne .png), jeweils mit zweistelligem Positionspraefix.
pub fn names(ls: &[Layer]) -> Vec<String> {
    let mut used = HashSet::new();
    let mut r = Vec::new();
    for (i, l) in ls.iter().enumerate() {
        let mut n = l.file_name();
        let b = n.as_bytes();
        if !(b.len() >= 3 && b[0].is_ascii_digit() && b[1].is_ascii_digit() && b[2] == b'-') {
            n = format!("{:02}-{}", i + 1, n);
        }
        let mut u = n.clone();
        let mut c = 2;
        while used.contains(&u) {
            u = format!("{n}-{c}");
            c += 1;
        }
        used.insert(u.clone());
        r.push(u);
    }
    r
}

fn hex(c: u32) -> String {
    format!("#{:06X}", c & 0xFFFFFF)
}

fn num(f: f32, dec: usize) -> String {
    format!("{:.*}", dec, f)
}

fn de(f: f32, dec: usize) -> String {
    num(f, dec).replace('.', ",")
}

pub fn esc(s: &str) -> String {
    let mut b = String::with_capacity(s.len() + 8);
    for ch in s.chars() {
        match ch {
            '&' => b.push_str("&amp;"),
            '<' => b.push_str("&lt;"),
            '>' => b.push_str("&gt;"),
            '"' => b.push_str("&quot;"),
            '\'' => b.push_str("&#39;"),
            _ => b.push(ch),
        }
    }
    b
}

fn clamp(v: f32, lo: f32, hi: f32) -> f32 {
    if v < lo {
        lo
    } else if v > hi {
        hi
    } else {
        v
    }
}

fn trigger(l: &Layer) -> String {
    let t = match l.anim {
        A_PARALLAX => Some("Scrollen"),
        A_DRIFT | A_FLOAT | A_HUE | A_RISE => Some("dauerhaft"),
        A_FADE_IN | A_SLIDE_UP => Some("Laden"),
        _ => None,
    };
    match t {
        None => (if l.hover { "Hover" } else { "nichts (statisch)" }).to_string(),
        Some(t) if l.hover => format!("{t}, Hover"),
        Some(t) => t.to_string(),
    }
}

pub fn prompt(w: i32, h: i32, ls: &[Layer], names: &[String]) -> String {
    let mut s = String::new();
    s.push_str("Baue aus den hochgeladenen Ebenen eine responsive One-Page-Website (HTML, CSS, JS).\n\n");
    s.push_str("Ebenen (hinten nach vorne): ");
    for (i, n) in names.iter().enumerate() {
        s.push_str(if i > 0 { ", " } else { "" });
        s.push_str(n);
        s.push_str(".png");
    }
    s.push_str(&format!(". Alle sind gleich groß ({w}×{h} px) und liegen deckungsgleich.\n\n"));
    s.push_str("Animationen:\n");
    for (i, l) in ls.iter().enumerate() {
        let a = if l.anim >= 0 && (l.anim as usize) < ANIM_NAMES.len() { l.anim } else { 0 };
        s.push_str(&format!("- {}.png: ", names[i]));
        if a == A_NONE {
            s.push_str("keine Animation");
        } else if a == A_PARALLAX {
            s.push_str(&format!("{}, Faktor {}", ANIM_NAMES[a as usize], de(clamp(l.speed, 0.0, 1.0), 2)));
        } else {
            s.push_str(&format!("{}, Tempo {}×", ANIM_NAMES[a as usize], de(clamp(l.speed, 0.25, 3.0), 2)));
        }
        if l.opacity < 0.995 {
            s.push_str(&format!(", Deckkraft {} %", (l.opacity * 100.0 + 0.5).floor() as i32));
        }
        s.push_str(&format!(", ausgelöst durch {}\n", trigger(l)));
    }
    s.push_str("\nStil: Farben [Hex-Codes], Schrift [Name], Stimmung [z. B. ruhig, verspielt].\n");
    s.push_str("Seitenaufbau: [Hero, Über mich, Galerie, Kontakt]\n");
    s.push_str("Texte: [Überschrift, Untertext, Button-Beschriftung]\n");
    s.push_str("Ziel: läuft flüssig auf Handy und Desktop, keine externen Bibliotheken außer wenn nötig.\n");
    s
}

const README: &str = "Deine animierte Website\n\
1. index.html per Doppelklick im Browser öffnen (der Ordner ebenen/ muss daneben liegen).\n   \
Hinweis: Direkt von der Festplatte wirkt der Hover-Effekt auf der ganzen Bühne statt pixelgenau.\n\
2. Veröffentlichen: ZIP entpacken und den Ordner auf app.netlify.com/drop ziehen,\n   \
oder in ein GitHub-Repository legen und GitHub Pages aktivieren.\n\
3. Ändern: Öffne eine Unterhaltung mit Claude und lade prompt.txt sowie alle Bilder aus ebenen/ hoch.\n   \
Fülle die Platzhalter in eckigen Klammern aus (Farben, Schrift, Texte).\n\
4. 00-vorschau.png zeigt das zusammengesetzte Motiv.\n\
5. Bilder der Ebenen sind deckungsgleich - nicht einzeln zuschneiden.\n\
Viel Spaß!\n";

/// Animierte One-Page-Website mit den Ebenen als deckungsgleiche Bilder.
pub fn html(title: &str, bg: u32, w: i32, h: i32, ls: &[Layer], files: &[String]) -> String {
    let lum = (0.299 * ((bg >> 16) & 255) as f32 + 0.587 * ((bg >> 8) & 255) as f32 + 0.114 * (bg & 255) as f32) / 255.0;
    let dark = lum < 0.5;
    let ink = if dark { "245,245,240" } else { "25,25,30" };
    let t = esc(title);
    let mut s = String::with_capacity(8192);
    s.push_str("<!DOCTYPE html>\n<html lang=\"de\">\n<head>\n<meta charset=\"utf-8\">\n");
    s.push_str("<meta name=\"viewport\" content=\"width=device-width, initial-scale=1\">\n");
    s.push_str(&format!("<meta name=\"theme-color\" content=\"{}\">\n", hex(bg)));
    s.push_str(&format!("<title>{t}</title>\n<style>\n"));
    s.push_str(&format!(":root{{--bg:{};--ink:{ink};--ratio:{w} / {h};--rn:{}}}\n", hex(bg), num(w as f32 / h as f32, 5)));
    s.push_str("*{box-sizing:border-box}\n");
    s.push_str("html{background:var(--bg);scroll-behavior:smooth}\n");
    s.push_str("body{margin:0;background:var(--bg);color:rgb(var(--ink));font-family:system-ui,-apple-system,'Segoe UI',Roboto,'Helvetica Neue',Arial,sans-serif;overflow-x:hidden;-webkit-font-smoothing:antialiased}\n");
    s.push_str(".sr{position:absolute;width:1px;height:1px;overflow:hidden;clip:rect(0 0 0 0);white-space:nowrap}\n");
    s.push_str(".hero{position:relative;display:flex;align-items:center;justify-content:center;min-height:100vh;min-height:100svh;overflow:hidden}\n");
    s.push_str(".stage{position:relative;width:min(100vw,calc(100vh * var(--rn)));width:min(100vw,calc(100svh * var(--rn)));aspect-ratio:var(--ratio);overflow:hidden;contain:paint;isolation:isolate}\n");
    s.push_str(".l{position:absolute;left:0;top:0;width:100%;height:100%;pointer-events:none;will-change:transform}\n");
    s.push_str(".h{position:absolute;left:0;top:0;width:100%;height:100%;transition:transform .3s ease;transform-origin:50% 60%}\n");
    s.push_str(".h.on{transform:scale(1.03) translateY(-1%)}\n");
    s.push_str(".h img{display:block;position:absolute;left:0;top:0;width:100%;height:100%;user-select:none;-webkit-user-drag:none}\n");
    s.push_str(".stage.pt{cursor:pointer}\n");
    s.push_str("@keyframes drift{from{transform:translateX(-3%)}to{transform:translateX(3%)}}\n");
    s.push_str("@keyframes float{from{transform:translateY(-1.5%)}to{transform:translateY(1.5%)}}\n");
    s.push_str("@keyframes fadein{from{opacity:0}to{opacity:1}}\n");
    s.push_str("@keyframes slideup{from{transform:translateY(10%);opacity:0}to{transform:none;opacity:1}}\n");
    s.push_str("@keyframes hue{from{filter:hue-rotate(0deg)}to{filter:hue-rotate(35deg)}}\n");
    s.push_str("@keyframes rise{0%{transform:translateY(0);opacity:.2}25%{opacity:1}75%{opacity:1}100%{transform:translateY(-6%);opacity:.2}}\n");
    s.push_str(".a2{animation:drift 18s ease-in-out infinite alternate}\n");
    s.push_str(".a3{animation:float 4s ease-in-out infinite alternate}\n");
    s.push_str(".a4{animation:fadein 1.2s ease-out both}\n");
    s.push_str(".a5{animation:slideup 1s cubic-bezier(.2,.7,.2,1) both}\n");
    s.push_str(".a6{animation:hue 14s ease-in-out infinite alternate}\n");
    s.push_str(".a7{animation:rise 6s ease-in-out infinite}\n");
    s.push_str(".cue{position:absolute;left:50%;bottom:clamp(14px,3vh,32px);width:22px;height:22px;margin-left:-11px;border-right:2px solid rgba(var(--ink),.45);border-bottom:2px solid rgba(var(--ink),.45);transform:rotate(45deg);animation:cue 2s ease-in-out infinite;z-index:2;opacity:.8}\n");
    s.push_str("@keyframes cue{0%,100%{translate:0 0;opacity:.2}50%{translate:6px 6px;opacity:.8}}\n");
    s.push_str(".more{min-height:100vh;display:flex;flex-direction:column;align-items:center;justify-content:center;gap:14px;padding:48px 16px;text-align:center}\n");
    s.push_str(".more h2{margin:0;font-size:clamp(1.4rem,4vw,2.2rem);font-weight:600;letter-spacing:-.01em}\n");
    s.push_str(".more p{margin:0;max-width:34ch;line-height:1.55;color:rgba(var(--ink),.6);font-size:clamp(.95rem,2.4vw,1.1rem)}\n");
    s.push_str(".more i{display:block;width:48px;height:2px;background:rgba(var(--ink),.25);border-radius:2px}\n");
    s.push_str("@media (max-aspect-ratio:1/1){.hero{min-height:0;padding:max(16px,3vh) 0}.cue{display:none}.more{min-height:70vh}}\n");
    s.push_str("@media (prefers-reduced-motion:reduce){html{scroll-behavior:auto}.l,.h,.h img,.cue{animation:none!important;transition:none!important}.l{transform:none!important}}\n");
    s.push_str("</style>\n</head>\n<body>\n");
    s.push_str(&format!("<header class=\"hero\">\n<h1 class=\"sr\">{t}</h1>\n"));
    s.push_str(&format!("<div class=\"stage\" id=\"stage\" role=\"img\" aria-label=\"{t}\">\n"));
    let mut slide_idx = 0;
    for (i, l) in ls.iter().enumerate() {
        let a = if (0..=A_RISE).contains(&l.anim) { l.anim } else { 0 };
        s.push_str("<div class=\"l\"");
        if a == A_PARALLAX {
            s.push_str(&format!(" data-px=\"{}\"", num(clamp(l.speed, 0.0, 1.0), 3)));
        }
        if l.hover {
            s.push_str(" data-hv=\"1\"");
        }
        s.push_str(&format!(" style=\"opacity:{}\"><div class=\"h\"><img", num(clamp(l.opacity, 0.0, 1.0), 3)));
        if a >= A_DRIFT {
            s.push_str(&format!(" class=\"a{a}\" style=\""));
            let sp = clamp(l.speed, 0.25, 3.0);
            let base = match a {
                A_DRIFT => 18.0,
                A_FLOAT => 4.0,
                A_FADE_IN => 1.2,
                A_SLIDE_UP => 1.0,
                A_HUE => 14.0,
                _ => 6.0,
            };
            s.push_str(&format!("animation-duration:{}s", num(base / sp, 2)));
            if a == A_SLIDE_UP {
                s.push_str(&format!(";animation-delay:{}s", num(0.15 * slide_idx as f32 + 0.1, 2)));
                slide_idx += 1;
            }
            s.push('"');
        }
        s.push_str(&format!(" src=\"{}\" alt=\"\" width=\"{w}\" height=\"{h}\" draggable=\"false\" decoding=\"async\"></div></div>\n", esc(&files[i])));
    }
    s.push_str("</div>\n<span class=\"cue\" aria-hidden=\"true\"></span>\n</header>\n");
    s.push_str(&format!("<main class=\"more\">\n<i></i>\n<h2>{t}</h2>\n<p>Hier geht es weiter. Dieser Bereich wartet auf deine Inhalte.</p>\n</main>\n"));
    s.push_str("<script>\n");
    s.push_str("(function(){\n");
    s.push_str("'use strict';\n");
    s.push_str("var stage=document.getElementById('stage');\n");
    s.push_str("var still=window.matchMedia&&window.matchMedia('(prefers-reduced-motion: reduce)').matches;\n");
    // Parallax
    s.push_str("var px=[].slice.call(stage.querySelectorAll('[data-px]')).map(function(e){return {e:e,f:1-parseFloat(e.getAttribute('data-px'))};});\n");
    s.push_str("var seen=true,tick=false;\n");
    s.push_str("function paint(){tick=false;if(!seen)return;var y=window.pageYOffset||document.documentElement.scrollTop||0;\n");
    s.push_str("for(var i=0;i<px.length;i++){px[i].e.style.transform='translate3d(0,'+(y*px[i].f).toFixed(1)+'px,0)';}}\n");
    s.push_str("function req(){if(!tick){tick=true;window.requestAnimationFrame(paint);}}\n");
    s.push_str("if(px.length&&!still){\n");
    s.push_str("window.addEventListener('scroll',req,{passive:true});window.addEventListener('resize',req,{passive:true});\n");
    s.push_str("if('IntersectionObserver' in window){new IntersectionObserver(function(en){seen=en[0].isIntersecting;if(seen)req();}).observe(stage);}\n");
    s.push_str("req();}\n");
    // Hover
    s.push_str("var hv=[].slice.call(stage.querySelectorAll('[data-hv]')).map(function(e){return {h:e.firstChild,img:e.getElementsByTagName('img')[0],cx:null,w:0,hh:0,ok:false};});\n");
    s.push_str("if(hv.length){\n");
    s.push_str("var coarse=false,any=false;\n");
    s.push_str("hv.forEach(function(o){\n");
    s.push_str("function prep(){try{var m=512,k=Math.min(1,m/Math.max(o.img.naturalWidth,o.img.naturalHeight));\n");
    s.push_str("var c=document.createElement('canvas');c.width=Math.max(1,Math.round(o.img.naturalWidth*k));c.height=Math.max(1,Math.round(o.img.naturalHeight*k));\n");
    s.push_str("var x=c.getContext('2d',{willReadFrequently:true});x.drawImage(o.img,0,0,c.width,c.height);x.getImageData(0,0,1,1);\n");
    s.push_str("o.cx=x;o.w=c.width;o.hh=c.height;o.ok=true;}catch(err){o.ok=false;coarse=true;}}\n");
    s.push_str("if(o.img.complete&&o.img.naturalWidth)prep();else o.img.addEventListener('load',prep);});\n");
    s.push_str("function set(o,on){o.h.classList.toggle('on',on);}\n");
    s.push_str("function off(){hv.forEach(function(o){set(o,false);});stage.classList.remove('pt');}\n");
    s.push_str("stage.addEventListener('mousemove',function(ev){\n");
    s.push_str("var r=stage.getBoundingClientRect(),u=(ev.clientX-r.left)/r.width,v=(ev.clientY-r.top)/r.height;\n");
    s.push_str("if(u<0||u>=1||v<0||v>=1){off();return;}\n");
    s.push_str("any=false;\n");
    s.push_str("hv.forEach(function(o){var on=false;\n");
    s.push_str("if(o.ok){try{on=o.cx.getImageData(Math.floor(u*o.w),Math.floor(v*o.hh),1,1).data[3]>20;}catch(err){o.ok=false;coarse=true;on=true;}}\n");
    s.push_str("else if(coarse||o.img.complete){on=true;}\n");
    s.push_str("set(o,on);any=any||on;});\n");
    s.push_str("stage.classList.toggle('pt',any);});\n");
    s.push_str("stage.addEventListener('mouseleave',off);\n");
    s.push_str("}\n");
    s.push_str("})();\n");
    s.push_str("</script>\n</body>\n</html>\n");
    s
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn names_and_html() {
        let mut ls = Layer::template(1);
        ls.push(Layer::new(9, "Wolken"));
        ls.push(Layer::new(10, "Wolken"));
        let n = names(&ls);
        assert_eq!(n[0], "01-hintergrund");
        assert_eq!(n[7], "08-wolken");
        assert_eq!(n[8], "09-wolken");
        let files: Vec<String> = n.iter().map(|x| format!("ebenen/{x}.png")).collect();
        let h = html("Mein <Board>", 0xFFFBFBF8, 1920, 1080, &ls, &files);
        assert!(h.contains("<title>Mein &lt;Board&gt;</title>"));
        assert!(h.contains("data-px=\"0.200\""));
        assert!(h.contains("class=\"a7\" style=\"animation-duration:6.00s\""));
        assert!(h.contains("animation-delay:0.10s"));
        assert!(h.contains("--rn:1.77778}"));
        assert!(prompt(10, 10, &ls, &n).contains("Parallax (Scrollen), Faktor 0,20"));
    }

    #[test]
    fn pdf_is_wellformed() {
        let mut pm = Pixmap::new(4, 3).unwrap();
        pm.fill(tiny_skia::Color::WHITE);
        let pdf = pdf_from_pixmap(&pm, 2.0, 1.5);
        assert!(pdf.starts_with(b"%PDF-1.4"));
        assert!(pdf.ends_with(b"%%EOF\n"));
    }
}
