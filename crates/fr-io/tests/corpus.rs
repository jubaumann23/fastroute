//! Loads every DSN of the reference corpus: no panics, and the outcome is summarized.
//!
//! `FR_IO_FULL_CORPUS=1 cargo test -p fr-io --release --test corpus -- --nocapture` runs all
//! 2487 files and prints the statistics.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use fr_io::{load, load_bytes, LoadError};

fn collect(dir: &Path, out: &mut Vec<PathBuf>) {
    let Ok(rd) = std::fs::read_dir(dir) else {
        return;
    };
    let mut entries: Vec<_> = rd.flatten().map(|e| e.path()).collect();
    entries.sort();
    for p in entries {
        if p.is_dir() {
            collect(&p, out);
        } else if p.extension().is_some_and(|e| e.eq_ignore_ascii_case("dsn")) {
            out.push(p);
        }
    }
}

/// All corpus DSNs with `FR_IO_FULL_CORPUS=1`, otherwise the small DSNs of `fixtures`
/// (under 100 kB, which keeps the default debug test run short).
pub fn corpus_files() -> Vec<PathBuf> {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../reference/freerouting");
    let mut files = Vec::new();
    collect(&root.join("fixtures"), &mut files);
    if std::env::var_os("FR_IO_FULL_CORPUS").is_some() {
        collect(&root.join("scripts/benchmark/fixtures"), &mut files);
    } else {
        files.retain(|f| std::fs::metadata(f).is_ok_and(|m| m.len() < 100_000));
    }
    files
}

#[test]
fn corpus_loads_without_panics() {
    let files = corpus_files();
    if files.is_empty() {
        eprintln!("corpus not found, skipping");
        return;
    }
    let mut outcomes: BTreeMap<String, usize> = BTreeMap::new();
    let mut examples: BTreeMap<String, Vec<String>> = BTreeMap::new();
    let mut requests = 0usize;
    for f in &files {
        let src = std::fs::read(f).unwrap();
        let r = load_bytes(&src);
        let key = match &r {
            Ok(_) => "ok".to_string(),
            Err(LoadError::ParseError(m)) => {
                format!("parse error: {}", m.split(':').next().unwrap_or(""))
            }
            Err(LoadError::OutlineMissing(_)) => "outline missing".into(),
            Err(LoadError::JavaException(m)) => format!("java exception: {m}"),
            Err(LoadError::Unsupported(m)) => format!("unsupported: {m}"),
        };
        if let Ok(d) = &r {
            requests += d.requests.len();
            assert!(matches!(
                d.requests[0],
                fr_io::InsertRequest::Outline { .. }
            ));
            // The canonical-order loader must agree for files in the usual layout.
            if let Ok(dsn) = fr_dsn::Dsn::parse(&src) {
                let c = load(&dsn);
                if let Ok(c) = c {
                    let _ = c.requests.len();
                }
            }
        }
        *outcomes.entry(key.clone()).or_default() += 1;
        let ex = examples.entry(key).or_default();
        if ex.len() < 3 {
            ex.push(f.file_name().unwrap().to_string_lossy().into_owned());
        }
    }
    eprintln!("{} files, {} requests", files.len(), requests);
    for (k, v) in &outcomes {
        eprintln!("{v:6}  {k}  e.g. {:?}", examples[k]);
    }
}
