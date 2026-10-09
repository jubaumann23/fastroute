//! F1: `fastroute serve` results do not depend on CPU load. Eight concurrent server processes
//! (two threads each) route the same corpus board while `4 x cores` busy threads oversubscribe
//! the machine; every export must be byte-identical, in two repetitions. Wall-clock stops of
//! the optimizer (greedy phase budget, default optimizer budget) must not be live in serve.
//! Every wait has a hard timeout so a hang fails instead of stalling the gate.

use std::io::{BufRead, BufReader, Write};
use std::path::PathBuf;
use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc;
use std::sync::Arc;
use std::time::Duration;

use fr_serve::serde_json::{json, Value};

const BIN: &str = env!("CARGO_BIN_EXE_fastroute");
const SERVERS: usize = 8;
const THREADS: u32 = 2;
const REPEATS: usize = 2;
const RUN_TIMEOUT: Duration = Duration::from_secs(900);
const BOARDS: [&str; 1] = ["hb200"];

fn corpus(key: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../reference/pcbkit-corpus/det").join(key).join("board.dsn")
}

/// One server process: hello, load, route all from scratch, export the SES text.
fn route_ses(dsn: PathBuf) -> String {
    let mut child = Command::new(BIN)
        .arg("serve")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .expect("spawn fastroute serve");
    let mut stdin = child.stdin.take().unwrap();
    let mut stdout = BufReader::new(child.stdout.take().unwrap());
    let mut call = |id: i64, op: &str, args: Value| -> Value {
        writeln!(stdin, "{}", json!({ "id": id, "op": op, "args": args })).unwrap();
        stdin.flush().unwrap();
        let mut line = String::new();
        assert!(stdout.read_line(&mut line).unwrap() > 0, "server closed stdout during {op}");
        let r: Value = fr_serve::serde_json::from_str(&line).unwrap();
        assert_eq!(r["ok"], json!(true), "{op}: {r}");
        r["result"].clone()
    };
    call(1, "hello", json!({ "protocol": "1.0.0", "client": "serve_determinism_load", "threads": THREADS, "settings": {} }));
    call(2, "load", json!({ "dsn": { "path": dsn.canonicalize().unwrap().to_str().unwrap() } }));
    call(3, "route", json!({ "seed": 0, "from": "scratch" }));
    let ses = call(4, "export", json!({ "format": "ses" }))["text"].as_str().unwrap().to_string();
    call(5, "shutdown", json!({}));
    drop(stdin);
    assert!(child.wait().unwrap().success());
    ses
}

#[test]
fn oversubscribed_concurrent_serves_are_byte_identical() {
    let cores = std::thread::available_parallelism().map(|n| n.get()).unwrap_or(4);
    let stop = Arc::new(AtomicBool::new(false));
    let burners: Vec<_> = (0..cores * 4)
        .map(|_| {
            let stop = stop.clone();
            std::thread::spawn(move || {
                let mut x = 1u64;
                while !stop.load(Ordering::Relaxed) {
                    x = std::hint::black_box(x.wrapping_mul(6364136223846793005).wrapping_add(1));
                }
            })
        })
        .collect();

    let mut failure: Option<String> = None;
    'outer: for board in BOARDS {
        let dsn = corpus(board);
        let mut reference: Option<String> = None;
        for rep in 0..REPEATS {
            let (tx, rx) = mpsc::channel();
            for i in 0..SERVERS {
                let (tx, dsn) = (tx.clone(), dsn.clone());
                std::thread::spawn(move || {
                    let _ = tx.send((i, route_ses(dsn)));
                });
            }
            drop(tx);
            for _ in 0..SERVERS {
                match rx.recv_timeout(RUN_TIMEOUT) {
                    Ok((i, ses)) => match &reference {
                        None => reference = Some(ses),
                        Some(r) if *r == ses => {}
                        Some(_) => {
                            failure = Some(format!("{board}: server {i} in repetition {rep} differs from the first SES"));
                            break 'outer;
                        }
                    },
                    Err(e) => {
                        failure = Some(format!("{board}: a server did not answer in {RUN_TIMEOUT:?} (repetition {rep}): {e}"));
                        break 'outer;
                    }
                }
            }
        }
    }
    stop.store(true, Ordering::Relaxed);
    for b in burners {
        let _ = b.join();
    }
    if let Some(f) = failure {
        panic!("{f}");
    }
}
