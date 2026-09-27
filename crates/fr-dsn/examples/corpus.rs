//! Parses every file given on stdin (one path per line) and reports failures.
use std::io::BufRead;
use std::time::Instant;

fn main() {
    let (mut ok, mut bad, mut bytes) = (0, 0, 0usize);
    let t = Instant::now();
    for path in std::io::stdin().lock().lines().map_while(Result::ok) {
        let data = std::fs::read(&path).unwrap();
        bytes += data.len();
        match fr_dsn::sexpr::parse(&data) {
            Ok(top) if top.len() == 1 => ok += 1,
            Ok(top) => {
                bad += 1;
                println!("{path}: {} top-level exprs", top.len());
            }
            Err(e) => {
                bad += 1;
                println!("{path}: {e}");
            }
        }
    }
    println!("ok={ok} bad={bad} {:.1} MB in {:?}", bytes as f64 / 1e6, t.elapsed());
}
