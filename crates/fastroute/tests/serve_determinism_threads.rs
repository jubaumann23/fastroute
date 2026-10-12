//! pcbkit D1/M1: `fastroute serve` gives the same SES for the same request at threads > 1, also
//! under the address-space limit the toolkit runs it with.
//!
//! The toolkit starts the router as `prlimit --as=4000000000 fastroute serve`. Each thread that
//! used the C allocator got its own glibc arena (64 MB of address space), so a route reserved
//! 4.4 GB for 0.6 GB resident; the optimizer's thread pool then failed to start in some runs and
//! the optimizer fell back to another evaluation (another SES), and some boards aborted on a failed
//! allocation. The server now caps the arenas (`malloc_arenas.rs`) and the optimizer keeps its
//! algorithm without a pool (`optimizer.rs`, D1).

use std::io::{BufRead, BufReader, Write};
use std::path::PathBuf;
use std::process::{Command, Stdio};

use fr_serve::serde_json::{json, Value};

const BIN: &str = env!("CARGO_BIN_EXE_fastroute");
const THREADS: u32 = 8;
/// Address-space limit of the limited run. The capped server reserves about 1.1 GB here (32-core
/// host); without the cap it reserved several GB and the limit broke its thread pools.
const ADDRESS_SPACE: u64 = 2_000_000_000;

fn dsn() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../fr-engine/tests/data/grid4x3.dsn")
}

/// The work caps and pcbkit stages the toolkit turns on (`fastroute_serve.py` `WORK_CAPS`).
fn settings() -> Value {
    json!({
        "router.max_tighten_rounds": 50,
        "router.max_connection_expansions": 100000,
        "router.tighten_min_move_um": 1,
        "router.tighten_stable_chamfer": true,
        "router.recover_attempts": 8,
    })
}

/// One server process (under `prlimit --as=limit` if given): hello, load, route from scratch,
/// export. Returns the SES text and the server's peak virtual size in kB.
fn route_ses(limit: Option<u64>) -> (String, u64) {
    let mut cmd = match limit {
        Some(bytes) => {
            let mut c = Command::new("prlimit");
            c.arg(format!("--as={bytes}")).arg(BIN);
            c
        }
        None => Command::new(BIN),
    };
    let mut child = cmd.arg("serve").stdin(Stdio::piped()).stdout(Stdio::piped()).stderr(Stdio::null()).spawn().expect("spawn fastroute serve");
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
    call(1, "hello", json!({ "protocol": "1.0.0", "client": "serve_determinism_threads", "threads": THREADS, "settings": settings() }));
    call(2, "load", json!({ "dsn": { "path": dsn().canonicalize().unwrap().to_str().unwrap() } }));
    call(3, "route", json!({ "seed": 0, "from": "scratch" }));
    let ses = call(4, "export", json!({ "format": "ses" }))["text"].as_str().unwrap().to_string();
    let vm_peak = vm_peak_kb(child.id());
    call(5, "shutdown", json!({}));
    drop(stdin);
    assert!(child.wait().unwrap().success());
    (ses, vm_peak)
}

/// `VmPeak` of `pid` (under prlimit: prlimit execs the server, same pid); 0 where unknown.
fn vm_peak_kb(pid: u32) -> u64 {
    std::fs::read_to_string(format!("/proc/{pid}/status"))
        .ok()
        .and_then(|s| s.lines().find(|l| l.starts_with("VmPeak:")).and_then(|l| l.split_whitespace().nth(1)?.parse().ok()))
        .unwrap_or(0)
}

fn has_prlimit() -> bool {
    Command::new("prlimit").arg("--version").stdout(Stdio::null()).stderr(Stdio::null()).status().is_ok_and(|s| s.success())
}

#[test]
fn the_same_request_twice_at_eight_threads_gives_the_same_ses() {
    let (first, peak) = route_ses(None);
    let (second, _) = route_ses(None);
    assert!(first == second, "two routes of the same request differ");
    eprintln!("VmPeak {} MB", peak / 1024);
}

#[test]
fn an_address_space_limit_does_not_change_the_ses() {
    if !has_prlimit() {
        eprintln!("SKIP: no prlimit");
        return;
    }
    let (free, peak) = route_ses(None);
    assert!(peak * 1024 < ADDRESS_SPACE, "the server reserved {} MB of address space", peak / 1024);
    for _ in 0..2 {
        let (limited, _) = route_ses(Some(ADDRESS_SPACE));
        assert!(limited == free, "the route under prlimit --as={ADDRESS_SPACE} differs");
    }
}
