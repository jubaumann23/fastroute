//! SES writer, SES reader and post-load parity against the Java source build
//! (`reference/bin/freerouting-parity.jar`), ground truth from `testdata/java/SesHarness.java`:
//!
//! * `<name>.ses`: `SesWriter` output after `DsnReader.readBoard` — must be byte-identical;
//! * `<name>.post.dump`: board dump after `HeadlessBoardManager.loadFromSpecctraDsn` with
//!   default settings — must be identical ([`fr_io::post_load::load_from_specctra_dsn`]);
//! * `<name>+<ses>.dump` / `.ses`: board dump and `SesWriter` output after `SesReader.read`
//!   of a routed session — must be identical.
//!
//! The committed subset in `testdata/ses_ground_truth` runs by default; set
//! `FR_IO_SES_DIR=<dir>` for a full run (generation: see `testdata/run_java_dump.sh`).

use std::path::{Path, PathBuf};

use fr_io::{build_board, dump, load_bytes, post_load, ses_reader, ses_writer};

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

fn repo() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../..")
}

/// `ComponentOutline` relative placement is not accessible on fr-engine board items.
fn normalize(text: &str) -> String {
    let mut out = String::new();
    for l in text.lines() {
        if l.contains(" ComponentOutline ") {
            if let (Some(a), Some(b)) = (l.find(" rel="), l.find(" abs=")) {
                out.push_str(&format!("{} rel=? tr=? rot=?{}\n", &l[..a], &l[b..]));
                continue;
            }
        }
        out.push_str(l);
        out.push('\n');
    }
    out
}

fn first_diff(a: &str, b: &str) -> String {
    for (i, (x, y)) in a.lines().zip(b.lines()).enumerate() {
        if x != y {
            return format!("line {i}:\n  java: {x}\n  rust: {y}");
        }
    }
    format!("line counts {} vs {}", a.lines().count(), b.lines().count())
}

#[test]
fn ses_and_post_load_parity() {
    let dir = std::env::var_os("FR_IO_SES_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(|| Path::new(env!("CARGO_MANIFEST_DIR")).join("testdata/ses_ground_truth"));
    let Ok(rd) = std::fs::read_dir(&dir) else {
        eprintln!("no ground truth at {}, skipping", dir.display());
        return;
    };
    let mut files: Vec<PathBuf> = rd.flatten().map(|e| e.path()).collect();
    files.sort();
    let mut problems = Vec::new();
    let (mut n_ses, mut n_post, mut n_session) = (0, 0, 0);
    for f in &files {
        let fname = f.file_name().unwrap().to_string_lossy().into_owned();
        if !fname.ends_with(".dump") {
            continue;
        }
        let java = std::fs::read_to_string(f).unwrap();
        let mut lines = java.lines();
        let source = lines
            .next()
            .unwrap()
            .strip_prefix("source ")
            .unwrap()
            .to_string();
        if KNOWN_FR_DSN_DIVERGENCES.contains(&source.as_str()) {
            continue;
        }
        let src = std::fs::read(repo().join("reference/freerouting").join(&source)).unwrap();
        let design_name = Path::new(&source)
            .file_name()
            .unwrap()
            .to_string_lossy()
            .into_owned();
        if let Some(stem) = fname.strip_suffix(".post.dump") {
            // SES of the freshly loaded board
            let design = load_bytes(&src).expect("load");
            let board = build_board(design);
            let rust_ses = ses_writer::ses_bytes(&board, &design_name);
            let java_ses = std::fs::read(dir.join(format!("{stem}.ses"))).unwrap();
            n_ses += 1;
            if rust_ses != java_ses {
                problems.push(format!(
                    "{source}: SES differs, {}",
                    first_diff(
                        &String::from_utf8_lossy(&java_ses),
                        &String::from_utf8_lossy(&rust_ses)
                    )
                ));
            }
            // post-load board
            let mut settings = fr_settings::default_settings(8);
            let board = post_load::load_from_specctra_dsn(&src, &mut settings).expect("load");
            let rust = dump::dump_board(&board);
            let java_body = normalize(
                &java
                    .lines()
                    .skip(1)
                    .map(|l| format!("{l}\n"))
                    .collect::<String>(),
            );
            n_post += 1;
            if rust != java_body {
                problems.push(format!(
                    "{source}: post-load dump differs, {}",
                    first_diff(&java_body, &rust)
                ));
            }
        } else {
            // DSN + SES
            let session = lines
                .next()
                .unwrap()
                .strip_prefix("session ")
                .unwrap()
                .to_string();
            let ses = std::fs::read(repo().join(&session)).unwrap();
            let design = load_bytes(&src).expect("load");
            let mut board = build_board(design);
            ses_reader::read_ses(&ses, &mut board);
            let rust = dump::dump_board(&board);
            let java_body = normalize(
                &java
                    .lines()
                    .skip(2)
                    .map(|l| format!("{l}\n"))
                    .collect::<String>(),
            );
            n_session += 1;
            let dump_ok = rust == java_body;
            if !dump_ok {
                problems.push(format!(
                    "{source} + {session}: board differs, {}",
                    first_diff(&java_body, &rust)
                ));
            }
            let stem = fname.strip_suffix(".dump").unwrap();
            let java_ses = std::fs::read(dir.join(format!("{stem}.ses"))).unwrap();
            let rust_ses = ses_writer::ses_bytes(&board, &design_name);
            if dump_ok && rust_ses != java_ses {
                problems.push(format!(
                    "{source} + {session}: SES differs, {}",
                    first_diff(
                        &String::from_utf8_lossy(&java_ses),
                        &String::from_utf8_lossy(&rust_ses)
                    )
                ));
            }
        }
    }
    eprintln!(
        "SES files {n_ses}, post-load boards {n_post}, sessions {n_session}, problems {}",
        problems.len()
    );
    for p in problems.iter().take(30) {
        eprintln!("{p}");
    }
    assert!(problems.is_empty());
}
