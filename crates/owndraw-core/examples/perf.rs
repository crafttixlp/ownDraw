//! Leistungsmessung des Kerns (ohne UI): `cargo run --release -p owndraw-core --example perf`
//!
//! Szenarien: Laden/Speichern, erstes Bild, Zeichnen eines Strichs (Bild je Bewegung), Verschieben,
//! Mausrad-Zoom, Rückgängig, Radieren, Seitenwechsel. Ausgabe: Mittel/95 %/Max je Bild in ms und die
//! Fläche, die je Bild neu zur GPU muss (Anteil am Bild).

use owndraw_core::board::{Action, Board, Motion, Pointer, ToolType};
use owndraw_core::item::Item;
use owndraw_core::pages::{BoardMode, PageBg, PageSetup};
use owndraw_core::tiny_skia::Pixmap;
use std::rc::Rc;
use std::time::Instant;

const W: f32 = 1920.0;
const H: f32 = 1080.0;

struct Rng(u64);
impl Rng {
    fn f(&mut self) -> f32 {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 7;
        self.0 ^= self.0 << 17;
        (self.0 >> 40) as f32 / (1u64 << 24) as f32
    }
}

/// n Freihand-Striche (Druck, ~80 Punkte) auf einer Fläche ext × ext.
fn strokes(n: usize, ext_w: f32, ext_h: f32, seed: u64) -> Vec<Rc<Item>> {
    let mut r = Rng(seed);
    (0..n)
        .map(|_| {
            let mut it = Item::stroke(0xFF1B1B1F, 3.0, false, true);
            let (mut x, mut y) = (r.f() * ext_w, r.f() * ext_h);
            let (mut dx, mut dy) = (r.f() * 4.0 - 2.0, r.f() * 4.0 - 2.0);
            for k in 0..80 {
                dx += r.f() - 0.5;
                dy += r.f() - 0.5;
                x += dx;
                y += dy;
                it.add_point(x, y, 0.35 + 0.3 * ((k as f32) * 0.15).sin().abs());
            }
            Rc::new(it)
        })
        .collect()
}

fn mv(a: Action, x: f32, y: f32, t: ToolType) -> Motion {
    Motion {
        action: a,
        pointers: vec![Pointer {
            id: 1,
            x,
            y,
            pressure: 0.5,
            tool: t,
        }],
        history: Vec::new(),
        stylus_button: false,
    }
}

#[derive(Default)]
struct Stat {
    ms: Vec<f64>,
    area: f64,
}
impl Stat {
    fn add(&mut self, ms: f64, frac: f64) {
        self.ms.push(ms);
        self.area += frac;
    }
    fn line(&mut self, name: &str) {
        self.ms.sort_by(|a, b| a.partial_cmp(b).unwrap());
        let n = self.ms.len().max(1);
        let mean = self.ms.iter().sum::<f64>() / n as f64;
        let p95 = self.ms[((n as f64 * 0.95) as usize).min(n - 1)];
        let max = *self.ms.last().unwrap_or(&0.0);
        println!(
            "{name:<34} {mean:8.2} {p95:8.2} {max:8.2}   {:5.1} %",
            100.0 * self.area / n as f64
        );
    }
}

/// Ein Bild: render + (wie die Oberfläche) Kopie der zu ladenden Bereiche. Liefert ms und Flächenanteil.
fn frame(b: &mut Board, pm: &mut Pixmap, up: &mut Vec<u8>) -> (f64, f64) {
    let t = Instant::now();
    let f = b.render(pm);
    let mut area = 0u64;
    for r in &f.damage {
        for (_, p) in owndraw_core::board::torus_parts(*r, f.org, pm.width(), pm.height()) {
            up.clear();
            for row in p.y..p.y + p.h {
                let o = ((row * pm.width() + p.x) * 4) as usize;
                up.extend_from_slice(&pm.data()[o..o + (p.w * 4) as usize]);
            }
            area += p.area();
        }
    }
    (
        t.elapsed().as_secs_f64() * 1000.0,
        area as f64 / (pm.width() * pm.height()) as f64,
    )
}

fn scenario(title: &str, ps: f32, n: usize, pages: bool) {
    println!(
        "\n== {title}: {n} Striche, {}×{} px ==",
        (W * ps) as u32,
        (H * ps) as u32
    );
    println!(
        "{:<34} {:>8} {:>8} {:>8}   {:>7}",
        "Szenario (ms je Bild)", "Mittel", "95 %", "Max", "Upload"
    );
    let mut b = Board::new(1.0);
    b.set_size(W, H, ps);
    if pages {
        let ps = PageSetup::new(595.0 * 1.6, 842.0 * 1.6, PageBg::Lined, 20);
        let rs = ps.rects();
        let mut items = Vec::new();
        for (i, r) in rs.iter().enumerate() {
            for it in strokes(n / rs.len(), r.width(), r.height(), 7 + i as u64) {
                let mut it = (*it).clone();
                if let owndraw_core::item::Kind::Stroke(s) = &mut it.kind {
                    for c in s.p.chunks_exact_mut(3) {
                        c[0] += r.left;
                        c[1] += r.top;
                    }
                }
                it.touch();
                items.push(Rc::new(it));
            }
        }
        b.board_mode = BoardMode::Pages(ps);
        b.items = items;
    } else {
        b.items = strokes(n, W * 1.5, H * 1.5, 7);
    }
    b.refresh();
    let mut pm = Pixmap::new((W * ps) as u32, (H * ps) as u32).unwrap();
    let mut up = Vec::new();

    // Laden / Speichern
    let t = Instant::now();
    let v = b.to_json();
    let tree = t.elapsed().as_secs_f64() * 1000.0;
    let s = serde_json::to_string(&v).unwrap();
    let save = t.elapsed().as_secs_f64() * 1000.0;
    println!("{:<34} {tree:8.2}", "  davon to_json (Baum)");
    let t = Instant::now();
    let th = b.thumb();
    let png = th.encode_png().unwrap();
    println!(
        "{:<34} {:8.2}   (PNG {} kB)",
        "Vorschaubild + PNG",
        t.elapsed().as_secs_f64() * 1000.0,
        png.len() / 1000
    );
    let t = Instant::now();
    let v: serde_json::Value = serde_json::from_str(&s).unwrap();
    let mut b2 = Board::new(1.0);
    b2.from_json(&v);
    let load = t.elapsed().as_secs_f64() * 1000.0;
    println!(
        "{:<34} {save:8.2}   (JSON {:.1} MB)",
        "Speichern (JSON)",
        s.len() as f64 / 1e6
    );
    println!("{:<34} {load:8.2}", "Laden (JSON)");
    drop(b2);

    let mut st = Stat::default();
    let (ms, f) = frame(&mut b, &mut pm, &mut up);
    st.add(ms, f);
    st.line("Erstes Bild");
    if let Ok(d) = std::env::var("OWNDRAW_DUMP") {
        let _ = pm.save_png(format!(
            "{d}/{}-{ps}.png",
            if pages { "pages" } else { "inf" }
        ));
    }
    if pages {
        b.show_page(0);
        frame(&mut b, &mut pm, &mut up);
    }

    // Zeichnen: 400 Bewegungen, ein Bild je Bewegung (Stylus, Druck 0.5 wie Maus am Desktop)
    let mut st = Stat::default();
    let (cx, cy) = (W * 0.3, H * 0.4);
    b.on_touch(&mv(Action::Down, cx, cy, ToolType::Stylus));
    frame(&mut b, &mut pm, &mut up);
    for k in 1..400 {
        let a = k as f32 * 0.05;
        b.on_touch(&mv(
            Action::Move,
            cx + a * 40.0,
            cy + (a * 3.0).sin() * 120.0,
            ToolType::Stylus,
        ));
        let (ms, f) = frame(&mut b, &mut pm, &mut up);
        st.add(ms, f);
    }
    st.line("Zeichnen (Bild je Bewegung)");
    let t = Instant::now();
    b.on_touch(&mv(Action::Up, cx + 800.0, cy, ToolType::Stylus));
    frame(&mut b, &mut pm, &mut up);
    println!(
        "{:<34} {:8.2}",
        "Strich beenden + Bild",
        t.elapsed().as_secs_f64() * 1000.0
    );

    // Marker (halbtransparent)
    let mut st = Stat::default();
    b.set_tool(owndraw_core::board::MARKER);
    b.on_touch(&mv(Action::Down, cx, cy + 200.0, ToolType::Stylus));
    frame(&mut b, &mut pm, &mut up);
    for k in 1..400 {
        let a = k as f32 * 0.05;
        b.on_touch(&mv(
            Action::Move,
            cx + a * 40.0,
            cy + 200.0 + (a * 3.0).sin() * 60.0,
            ToolType::Stylus,
        ));
        let (ms, f) = frame(&mut b, &mut pm, &mut up);
        st.add(ms, f);
    }
    b.on_touch(&mv(Action::Up, cx + 800.0, cy + 200.0, ToolType::Stylus));
    frame(&mut b, &mut pm, &mut up);
    st.line("Marker (Bild je Bewegung)");
    b.set_tool(owndraw_core::board::PEN);

    // Verschieben mit dem Finger
    let mut st = Stat::default();
    b.on_touch(&mv(Action::Down, 900.0, 500.0, ToolType::Finger));
    for k in 1..120 {
        b.on_touch(&mv(
            Action::Move,
            900.0 - k as f32 * 7.3,
            500.0 - k as f32 * 3.1,
            ToolType::Finger,
        ));
        let (ms, f) = frame(&mut b, &mut pm, &mut up);
        st.add(ms, f);
    }
    b.on_touch(&mv(
        Action::Up,
        900.0 - 120.0 * 7.3,
        500.0 - 120.0 * 3.1,
        ToolType::Finger,
    ));
    st.line("Verschieben (Finger)");
    let mut st = Stat::default();
    let (ms, f) = frame(&mut b, &mut pm, &mut up);
    st.add(ms, f);
    st.line("Bild nach Verschieben");

    // Mausrad-Zoom
    let mut st = Stat::default();
    let mut now = 1.0;
    for k in 0..30 {
        b.tick(now);
        b.zoom_at(960.0, 540.0, if k < 15 { 1.06 } else { 0.95 });
        let (ms, f) = frame(&mut b, &mut pm, &mut up);
        st.add(ms, f);
        now += 0.016;
    }
    st.line("Mausrad-Zoom");
    b.tick(now + 1.0);
    let mut st = Stat::default();
    let (ms, f) = frame(&mut b, &mut pm, &mut up);
    st.add(ms, f);
    st.line("Bild nach Zoom");

    // Rückgängig / Wiederholen
    let mut st = Stat::default();
    for _ in 0..10 {
        b.undo();
        let (ms, f) = frame(&mut b, &mut pm, &mut up);
        st.add(ms, f);
        b.redo();
        let (ms, f) = frame(&mut b, &mut pm, &mut up);
        st.add(ms, f);
    }
    st.line("Rückgängig/Wiederholen + Bild");

    // Radierer (Bewegung über die Fläche)
    let mut st = Stat::default();
    b.on_touch(&mv(Action::Down, 300.0, 300.0, ToolType::Eraser));
    for k in 1..100 {
        b.on_touch(&mv(
            Action::Move,
            300.0 + k as f32 * 6.0,
            300.0 + k as f32 * 2.0,
            ToolType::Eraser,
        ));
        let (ms, f) = frame(&mut b, &mut pm, &mut up);
        st.add(ms, f);
    }
    b.on_touch(&mv(Action::Up, 900.0, 500.0, ToolType::Eraser));
    let (ms, f) = frame(&mut b, &mut pm, &mut up);
    st.add(ms, f);
    st.line("Radieren (Bild je Bewegung)");

    // Leerlauf-Bild (nichts geändert)
    if let Ok(d) = std::env::var("OWNDRAW_DUMP") {
        b.refresh();
        frame(&mut b, &mut pm, &mut up);
        let _ = pm.save_png(format!(
            "{d}/{}-{ps}-end.png",
            if pages { "pages" } else { "inf" }
        ));
    }
    let mut st = Stat::default();
    for _ in 0..20 {
        b.invalidate();
        let (ms, f) = frame(&mut b, &mut pm, &mut up);
        st.add(ms, f);
    }
    st.line("Bild ohne Änderung");

    if pages {
        let mut st = Stat::default();
        for i in 0..20 {
            b.show_page(i);
            let (ms, f) = frame(&mut b, &mut pm, &mut up);
            st.add(ms, f);
        }
        st.line("Seitenwechsel + Bild");
    }
}

fn main() {
    let t = Instant::now();
    let _ = owndraw_core::text::measure("Start", 14.0);
    println!(
        "Schrift laden: {:.2} ms",
        t.elapsed().as_secs_f64() * 1000.0
    );
    scenario("Unendlich, Desktop", 1.0, 3000, false);
    scenario("Unendlich, HiDPI ×2", 2.0, 3000, false);
    scenario("Seitenmodus 20 Seiten", 1.0, 4000, true);
}
