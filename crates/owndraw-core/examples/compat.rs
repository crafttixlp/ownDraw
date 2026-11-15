//! Kompatibilitaetstest mit der Java-Version: compat <in.owndraw> <arbeitsordner> <out.owndraw>
//! Importiert ein Board, rendert es (PNG/PDF/Website-ZIP) und exportiert es wieder.

use owndraw_core::{board::Board, export, item, store::Store};
use std::fs::File;

fn main() {
    let a: Vec<String> = std::env::args().collect();
    let st = Store::new(std::path::Path::new(&a[2]));
    let id = st
        .import_board(File::open(&a[1]).unwrap())
        .expect("Import fehlgeschlagen");
    println!("importiert: id={id} name={}", st.name(&id));
    item::set_img_dir(st.img_dir());
    let mut b = Board::new(1.0);
    b.from_json(&st.load(&id).unwrap());
    println!(
        "{} Objekte, {} Ebenen, aktiv {}",
        b.items.len(),
        b.layers.len(),
        b.active
    );
    for it in &b.items {
        println!("  {}", it.json());
    }
    std::fs::write(format!("{}/board.png", a[2]), b.export_png().unwrap()).unwrap();
    std::fs::write(format!("{}/board.pdf", a[2]), b.export_pdf().unwrap()).unwrap();
    assert!(export::write_zip(
        &b,
        "Kompat",
        File::create(format!("{}/web.zip", a[2])).unwrap()
    ));
    st.save(
        &id,
        &b.to_json().to_string(),
        Some(&b.thumb().encode_png().unwrap()),
        true,
    );
    assert!(st.export_board(&id, File::create(&a[3]).unwrap()));
    println!("exportiert nach {}", a[3]);
}
