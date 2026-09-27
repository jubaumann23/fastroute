//! Parity against the real Java loader (`DsnReader.readBoard` of `reference/bin/freerouting-parity.jar`,
//! built from the reference source).
//!
//! The Java dumps are produced by `testdata/run_java_dump.sh` (see `testdata/java/DumpBoard.java`)
//! with `reference/freerouting` as base directory. By default the dumps committed in
//! `testdata/java_dumps` are checked; set `FR_IO_JAVA_DUMPS=<dir>` to check another set, e.g.
//!
//! ```sh
//! find reference/freerouting/fixtures -iname '*.dsn' -print0 |
//!   xargs -0 crates/fr-io/testdata/run_java_dump.sh /tmp/jdump reference/freerouting
//! FR_IO_JAVA_DUMPS=/tmp/jdump cargo test -p fr-io --release --test java_parity -- --nocapture
//! ```
//!
//! Everything except the trace items must match exactly: the Java dump is taken after
//! `normalizeAllTraces`, which may merge/split pre-routed traces (new ids above the loader's
//! last id, removed ids, changed corners). Trace differences are reported, not failed.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use fr_io::{dump, load_bytes, LoadError};

fn root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../reference/freerouting")
}

/// Files where the typed DSN model (fr-dsn lexer) deliberately or knowingly reads different
/// tokens than the Java scanner; reported, not failed:
/// * `Issue110-RelayModule`: Java's scanner drops the Cyrillic letters of unquoted image
///   names (`Предохранители:Цилиндр_5х20` -> `:`); only names differ.
/// * `Issue229-display-8-digit-hc595`: `(PN "DISPLAY 7-SEG 0.5"")` — fr-dsn reads `""` as a
///   literal quote, the Java scanner closes the string and desynchronizes, losing the
///   components DP3, DP5, DP6, DP8.
/// * `Aleste-520EX_aleste`: `(net "/Gate Array/"A"")` — Java reads the net name
///   `/Gate Array/`, fr-dsn `/Gate Array/A`.
/// * `PDB_OSD_HARDWARE_Quadcopter Power Board`, `newer-motor-controllers_*`: a `--1` token in
///   a `(pins ...)` list; Java reads component `1`, swallows one character of the next token
///   as the hyphen and takes the rest as the pin name (so the next pin is lost); fr-dsn drops
///   the `--1` token and keeps the next pin.
/// * `avr-divecomputer_dc`: U+2010 in an unquoted image name; Java drops the non-ASCII
///   characters, fr-dsn keeps them; only names differ.
const KNOWN_FR_DSN_DIVERGENCES: &[&str] = &[
    "fixtures/Issue110-RelayModule.dsn",
    "fixtures/Issue229-display-8-digit-hc595.dsn",
    "scripts/benchmark/fixtures/PCBench/Aleste-520EX_aleste/reference-routed.dsn",
    "scripts/benchmark/fixtures/PCBench/Aleste-520EX_aleste/unrouted.dsn",
    "scripts/benchmark/fixtures/PCBench/PDB_OSD_HARDWARE_Quadcopter Power Board/reference-routed.dsn",
    "scripts/benchmark/fixtures/PCBench/PDB_OSD_HARDWARE_Quadcopter Power Board/unrouted.dsn",
    "scripts/benchmark/fixtures/PCBench/newer-motor-controllers_busparts/reference-routed.dsn",
    "scripts/benchmark/fixtures/PCBench/newer-motor-controllers_busparts/unrouted.dsn",
    "scripts/benchmark/fixtures/PCBench/newer-motor-controllers_si31-3/reference-routed.dsn",
    "scripts/benchmark/fixtures/PCBench/newer-motor-controllers_si31-3/unrouted.dsn",
    "scripts/benchmark/fixtures/PCBench/avr-divecomputer_dc/reference-routed.dsn",
    "scripts/benchmark/fixtures/PCBench/avr-divecomputer_dc/unrouted.dsn",
];

#[derive(Default, Debug)]
struct Stats {
    files: usize,
    header_diffs: usize,
    item_diffs: usize,
    items: usize,
    trace_diffs: usize,
    board_items: usize,
    board_diffs: usize,
}

/// `ComponentOutline` relative placement is not accessible on fr-engine board items.
fn normalize_board_line(l: &str) -> String {
    if l.contains(" ComponentOutline ") {
        if let (Some(a), Some(b)) = (l.find(" rel="), l.find(" abs=")) {
            return format!("{} rel=? tr=? rot=?{}", &l[..a], &l[b..]);
        }
    }
    l.to_string()
}

fn item_id_kind(line: &str) -> (i32, &str) {
    let mut it = line.split(' ');
    it.next();
    let id = it.next().unwrap().parse().unwrap();
    (id, it.next().unwrap())
}

/// Compares one Java dump with the Rust result; returns a list of problems.
fn compare(java: &str, stats: &mut Stats) -> Vec<String> {
    let mut problems = Vec::new();
    let mut lines = java.lines();
    let source = lines
        .next()
        .unwrap()
        .strip_prefix("source ")
        .unwrap()
        .to_string();
    let result = lines.next().unwrap_or("");
    if KNOWN_FR_DSN_DIVERGENCES.contains(&source.as_str()) {
        eprintln!("{source}: skipped (known fr-dsn lexer divergence)");
        return problems;
    }
    // Synthetic fixtures are dumped with the crate directory as base.
    let path = if source.starts_with("testdata/") {
        Path::new(env!("CARGO_MANIFEST_DIR")).join(&source)
    } else {
        root().join(&source)
    };
    let src = std::fs::read(path).expect("dsn");
    let rust = load_bytes(&src);
    let expected = match &rust {
        Ok(_) => "result OK",
        Err(LoadError::ParseError(_)) => "result ERROR",
        Err(LoadError::OutlineMissing(_)) => "result OUTLINE_MISSING",
        Err(LoadError::JavaException(_)) | Err(LoadError::Unsupported(_)) => "result EXCEPTION",
    };
    if !result.starts_with(expected) {
        problems.push(format!(
            "{source}: result: java '{result}', rust {:?}",
            rust.as_ref().err()
        ));
        return problems;
    }
    let Ok(design) = rust else { return problems };

    let java_header: Vec<&str> = lines
        .clone()
        .take_while(|l| !l.starts_with("item "))
        .collect();
    let java_items: BTreeMap<i32, &str> = lines
        .filter(|l| l.starts_with("item "))
        .map(|l| (item_id_kind(l).0, l))
        .collect();
    // Ground truth is freerouting-parity.jar (built from the reference source), so the
    // source version of adjustPlaneAutorouteSettings applies (see fr_io::plane).
    let adj = design.plane_adjustment.clone();
    let rust_header = dump::dump_header(&design, adj.as_ref());
    let rust_header: Vec<&str> = rust_header.lines().collect();
    if rust_header != java_header {
        stats.header_diffs += 1;
        let n = java_header.len().max(rust_header.len());
        let mut shown = 0;
        for i in 0..n {
            let (j, r) = (java_header.get(i), rust_header.get(i));
            if j != r && shown < 5 {
                problems.push(format!(
                    "{source}: header line {i}:\n  java: {j:?}\n  rust: {r:?}"
                ));
                shown += 1;
            }
        }
    }
    let rust_items: BTreeMap<i32, String> = dump::dump_items(&design, adj.as_ref())
        .into_iter()
        .collect();
    if std::env::var_os("FR_IO_SKIP_BOARD").is_none() {
        // The full board step, including normalizeAllTraces: every item must match exactly.
        let board = fr_io::build_board(design.clone());
        let board_items: BTreeMap<i32, String> =
            dump::dump_board_items(&board).into_iter().collect();
        let java_norm: BTreeMap<i32, String> = java_items
            .iter()
            .map(|(k, v)| (*k, normalize_board_line(v)))
            .collect();
        stats.board_items += java_norm.len();
        let ids: std::collections::BTreeSet<&i32> =
            java_norm.keys().chain(board_items.keys()).collect();
        let mut shown = 0;
        for id in ids {
            let (j, r) = (java_norm.get(id), board_items.get(id));
            if j != r {
                stats.board_diffs += 1;
                if shown < 5 {
                    problems.push(format!(
                        "{source}: board item {id}:\n  java: {j:?}\n  rust: {r:?}"
                    ));
                    shown += 1;
                }
            }
        }
    }
    let max_rust_id = rust_items.keys().next_back().copied().unwrap_or(0);
    let mut shown = 0;
    for (id, jl) in &java_items {
        stats.items += 1;
        let jkind = item_id_kind(jl).1;
        match rust_items.get(id) {
            Some(rl) if rl == jl => {}
            Some(rl) => {
                if jkind == "PolylineTrace" && item_id_kind(rl).1 == "PolylineTrace" {
                    stats.trace_diffs += 1;
                } else {
                    stats.item_diffs += 1;
                    if shown < 5 {
                        problems.push(format!("{source}: item {id}:\n  java: {jl}\n  rust: {rl}"));
                        shown += 1;
                    }
                }
            }
            None => {
                if jkind == "PolylineTrace" && *id > max_rust_id {
                    stats.trace_diffs += 1; // created by normalizeAllTraces
                } else {
                    stats.item_diffs += 1;
                    if shown < 5 {
                        problems.push(format!(
                            "{source}: item {id} missing in rust:\n  java: {jl}"
                        ));
                        shown += 1;
                    }
                }
            }
        }
    }
    for (id, rl) in &rust_items {
        if java_items.contains_key(id) {
            continue;
        }
        if item_id_kind(rl).1 == "PolylineTrace" {
            stats.trace_diffs += 1; // removed by normalizeAllTraces
        } else {
            stats.item_diffs += 1;
            if shown < 5 {
                problems.push(format!(
                    "{source}: item {id} missing in java:\n  rust: {rl}"
                ));
                shown += 1;
            }
        }
    }
    problems
}

#[test]
fn java_parity() {
    let dir = std::env::var_os("FR_IO_JAVA_DUMPS")
        .map(PathBuf::from)
        .unwrap_or_else(|| Path::new(env!("CARGO_MANIFEST_DIR")).join("testdata/java_dumps"));
    let Ok(rd) = std::fs::read_dir(&dir) else {
        eprintln!("no Java dumps at {}, skipping", dir.display());
        return;
    };
    let mut files: Vec<PathBuf> = rd
        .flatten()
        .map(|e| e.path())
        .filter(|p| p.extension().is_some_and(|e| e == "dump" || e == "gz"))
        .collect();
    files.sort();
    let mut stats = Stats::default();
    let mut all_problems = Vec::new();
    let mut failed_files = Vec::new();
    let mut trace_files = Vec::new();
    for f in &files {
        let java = std::fs::read_to_string(f).unwrap();
        stats.files += 1;
        let traces_before = stats.trace_diffs;
        let p = compare(&java, &mut stats);
        if !p.is_empty() {
            failed_files.push(f.file_name().unwrap().to_string_lossy().into_owned());
        }
        if stats.trace_diffs > traces_before {
            trace_files.push((
                f.file_name().unwrap().to_string_lossy().into_owned(),
                stats.trace_diffs - traces_before,
            ));
        }
        all_problems.extend(p);
    }
    eprintln!("{stats:?}");
    for p in all_problems.iter().take(60) {
        eprintln!("{p}");
    }
    eprintln!("trace differences (normalizeAllTraces): {trace_files:?}");
    eprintln!("failed files ({}): {failed_files:?}", failed_files.len());
    assert!(all_problems.is_empty(), "{} problems", all_problems.len());
}
