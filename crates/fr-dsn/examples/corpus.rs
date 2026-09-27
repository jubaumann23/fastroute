//! Parses every DSN file given on stdin (one path per line) into the typed
//! model and reports failures and summary statistics.
use std::io::BufRead;
use std::time::Instant;

fn main() {
    let (mut ok, mut bad, mut bytes, mut warns) = (0, 0, 0usize, 0usize);
    let (mut nets, mut pins, mut wires) = (0usize, 0usize, 0usize);
    let t = Instant::now();
    for path in std::io::stdin().lock().lines().map_while(Result::ok) {
        let data = std::fs::read(&path).unwrap();
        bytes += data.len();
        match fr_dsn::Dsn::parse(&data) {
            Ok(d) => {
                ok += 1;
                warns += d.warnings.len();
                nets += d.network.nets.len();
                pins += d.network.nets.iter().map(|n| n.pins.len()).sum::<usize>();
                wires += d.wiring.wires.len();
                if std::env::var_os("SHOW_WARNINGS").is_some() {
                    for w in &d.warnings {
                        println!("{path}: warning: {w}");
                    }
                }
            }
            Err(e) => {
                bad += 1;
                println!("{path}: {e}");
            }
        }
    }
    println!(
        "ok={ok} bad={bad} warnings={warns} nets={nets} pins={pins} wires={wires} \
         {:.1} MB in {:?}",
        bytes as f64 / 1e6,
        t.elapsed()
    );
}
