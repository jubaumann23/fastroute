//! End-to-end parity of the routing pipeline with the Java parity build
//! (`scripts/java-parity.sh`, single-thread optimizer, time limits disabled): the SES written by
//! `fastroute --parity` must be byte-identical to the Java output committed in `testdata/`.
//!
//! The DSN inputs are read from `reference/freerouting` (not committed); the tests are skipped
//! if it is missing. Expected outputs:
//! * `bm08_full.ses`, `bm02_full.ses`: `reference/parity-baseline` (default settings: fanout,
//!   autorouter and optimizer).
//! * `pic_programmer_noopt.ses`, `bm07_noopt.ses`: `--router.optimizer.enabled=false`.
//!
//! Also cross-checks the single-sort `ReadSortedRouteItems` of the optimizer against a direct
//! port of the Java O(n²) scan on a routed board.

use std::path::{Path, PathBuf};
use std::process::Command;

use fr_engine::board::RoutingBoard;
use fr_geom::FloatPoint;

fn root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../..")
}

fn fixture(rel: &str) -> Option<PathBuf> {
    let p = root().join("reference/freerouting/scripts/benchmark/fixtures").join(rel);
    if p.exists() {
        Some(p)
    } else {
        eprintln!("skipped: {} not found", p.display());
        None
    }
}

fn route_and_compare(dsn_rel: &str, expected: &str, extra: &[&str]) {
    let Some(dsn) = fixture(dsn_rel) else { return };
    let out = std::env::temp_dir().join(format!("fastroute-test-{}-{}", std::process::id(), expected));
    let status = Command::new(env!("CARGO_BIN_EXE_fastroute"))
        .arg("-de")
        .arg(&dsn)
        .arg("-do")
        .arg(&out)
        .arg("--parity")
        .args(extra)
        .stderr(std::process::Stdio::null())
        .status()
        .expect("run fastroute");
    assert!(status.success());
    let got = std::fs::read(&out).unwrap();
    let _ = std::fs::remove_file(&out);
    let want = std::fs::read(Path::new(env!("CARGO_MANIFEST_DIR")).join("testdata").join(expected)).unwrap();
    assert!(got == want, "{dsn_rel}: SES differs from the Java parity output {expected}");
}

#[test]
fn bm08_full_pipeline_matches_java() {
    route_and_compare("DAC2020_boards/DAC2020_bm08.dsn", "bm08_full.ses", &[]);
}

#[test]
fn bm02_full_pipeline_matches_java() {
    route_and_compare("DAC2020_boards/DAC2020_bm02.dsn", "bm02_full.ses", &[]);
}

#[test]
fn pic_programmer_autorouter_matches_java() {
    route_and_compare("KiCad_10_demos/pic_programmer.dsn", "pic_programmer_noopt.ses", &["--router.optimizer.enabled=false"]);
}

#[test]
fn bm07_autorouter_matches_java() {
    route_and_compare("DAC2020_boards/DAC2020_bm07.dsn", "bm07_noopt.ses", &["--router.optimizer.enabled=false"]);
}

/// Direct port of Java `BatchOptimizer.ReadSortedRouteItems.next()` (O(n²)).
fn java_sorted_route_items(board: &RoutingBoard) -> Vec<i32> {
    let mut min_coor = FloatPoint::new(i32::MIN as f64, i32::MIN as f64);
    let mut min_layer = -1;
    let mut out = Vec::new();
    loop {
        let mut result: Option<i32> = None;
        let mut cur_min = FloatPoint::new(i32::MAX as f64, i32::MAX as f64);
        let mut cur_min_layer = i32::MAX;
        for k in board.get_items() {
            let it = board.item(k);
            if it.is_via() && !it.is_user_fixed() {
                let c = it.center(board).to_float();
                let l = it.first_layer(board);
                if (c.x > min_coor.x || c.x == min_coor.x && (c.y > min_coor.y || c.y == min_coor.y && l > min_layer))
                    && (c.x < cur_min.x || c.x == cur_min.x && (c.y < cur_min.y || c.y == cur_min.y && l < cur_min_layer))
                {
                    cur_min = c;
                    cur_min_layer = l;
                    result = Some(it.id().0);
                }
            }
        }
        for k in board.get_items() {
            let it = board.item(k);
            if it.is_trace() && !board.is_shove_fixed(k) {
                let f = it.first_corner().to_float();
                let l = it.last_corner().to_float();
                let c = if f.x < l.x || f.x == l.x && f.y < l.y { l } else { f };
                let layer = it.trace().layer();
                if (c.x > min_coor.x || c.x == min_coor.x && (c.y > min_coor.y || c.y == min_coor.y && layer > min_layer))
                    && (c.x < cur_min.x || c.x == cur_min.x && (c.y < cur_min.y || c.y == cur_min.y && layer < cur_min_layer))
                {
                    let via = board.normal_contacts(k).iter().any(|x| board.item(x).is_via() && !board.item(x).is_user_fixed());
                    if !via {
                        cur_min = c;
                        cur_min_layer = layer;
                        result = Some(it.id().0);
                    }
                }
            }
        }
        min_coor = cur_min;
        min_layer = cur_min_layer;
        match result {
            Some(id) => out.push(id),
            None => return out,
        }
    }
}

#[test]
fn sorted_route_items_match_java_scan() {
    let Some(dsn) = fixture("DAC2020_boards/DAC2020_bm02.dsn") else { return };
    let data = std::fs::read(dsn).unwrap();
    let mut settings = fr_settings::default_settings(1);
    settings.optimizer.enabled = Some(false);
    let mut board = fr_io::post_load::load_from_specctra_dsn(&data, &mut settings).unwrap();
    fr_io::post_load::prepare_for_routing(&mut board, &mut settings, None);
    board.time_limits = fr_engine::board::TimeLimitPolicy::disabled();
    let ctx = fr_engine::pipeline::PipelineContext { wall_clock_limits: false, ..Default::default() };
    fr_engine::pipeline::run_pipeline(&mut board, &mut settings, &ctx);
    assert!(!board.get_vias().is_empty() && !board.get_traces().is_empty());
    let fast = fr_engine::pipeline::optimizer::sorted_route_item_ids(&board);
    assert!(!fast.is_empty());
    assert_eq!(fast, java_sorted_route_items(&board));
}

/// The `parallel` optimizer mode gives the same result for any thread count.
#[test]
fn parallel_optimizer_is_deterministic() {
    let Some(dsn) = fixture("DAC2020_boards/DAC2020_bm02.dsn") else { return };
    let mut outputs = Vec::new();
    for threads in [1, 4] {
        let out = std::env::temp_dir().join(format!("fastroute-test-{}-par{threads}.ses", std::process::id()));
        let status = Command::new(env!("CARGO_BIN_EXE_fastroute"))
            .arg("-de")
            .arg(&dsn)
            .arg("-do")
            .arg(&out)
            .args(["--no-time-limits", "--optimizer-mode=parallel", &format!("--router.optimizer.max_threads={threads}")])
            .stderr(std::process::Stdio::null())
            .status()
            .expect("run fastroute");
        assert!(status.success());
        outputs.push(std::fs::read(&out).unwrap());
        let _ = std::fs::remove_file(&out);
    }
    assert!(outputs[0] == outputs[1], "parallel optimizer result depends on the thread count");
}
