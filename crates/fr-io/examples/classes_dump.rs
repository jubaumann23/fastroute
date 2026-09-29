//! Prints net classes, their default item clearance classes and the clearance matrix names.
//! Usage: cargo run --release -p fr-io --example classes_dump -- board.dsn
use fr_engine::rules::default_item_clearance_classes::ItemClass;

fn main() {
    let path = std::env::args().nth(1).expect("board.dsn");
    let src = std::fs::read(&path).unwrap();
    let mut settings = fr_settings::default_settings(1);
    let board = fr_io::post_load::load_from_specctra_dsn(&src, &mut settings).expect("load");
    let m = &board.rules.clearance_matrix;
    let name = |n: i32| m.get_name(n).unwrap_or("?").to_string();
    for c in board.rules.net_classes.iter() {
        let nc = &board.rules.net_classes[c];
        let d = &nc.default_item_clearance_classes;
        println!(
            "{:14} trace={} via={} pin={} smd={}",
            nc.get_name(),
            name(d.get(ItemClass::Trace)),
            name(d.get(ItemClass::Via)),
            name(d.get(ItemClass::Pin)),
            name(d.get(ItemClass::Smd))
        );
    }
    let n = m.get_class_count();
    println!("matrix classes: {:?}", (0..n).map(name).collect::<Vec<_>>());
}
