//! Measures board clone time (input for the parallel autorouter design).
//! Usage: cargo run --release -p fr-io --example clone_bench -- board.dsn
use std::time::Instant;

fn main() {
    let path = std::env::args().nth(1).expect("board.dsn");
    let src = std::fs::read(&path).unwrap();
    let mut settings = fr_settings::default_settings(1);
    let t = Instant::now();
    let mut board = fr_io::post_load::load_from_specctra_dsn(&src, &mut settings).expect("load");
    fr_io::post_load::prepare_for_routing(&mut board, &mut settings, None);
    println!("load {:.1} ms, {} items", t.elapsed().as_secs_f64() * 1e3, board.get_items().len());
    let t = Instant::now();
    let n = 20;
    for _ in 0..n {
        let c = board.clone();
        std::hint::black_box(&c);
    }
    println!("clone {:.2} ms", t.elapsed().as_secs_f64() * 1e3 / n as f64);
}
