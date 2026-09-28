//! `fastroute`: command-line entry point. Accepts the Freerouting CLI arguments
//! (`-de board.dsn -do board.ses -mp N --router.x.y=v ...`), routes the board (fanout,
//! autorouter, optimizer) and writes the Specctra session.
//!
//! Own options:
//! * `--parity`: deterministic Java parity mode (`scripts/java-parity.sh`): the board time
//!   limits in count mode (the deterministic call budget of the parity jar), the stage wall
//!   clock limits disabled, optimizer in `java-compat` mode (Java with `optimizer.max_threads=1`).
//! * `--time-limit-mode=wall|count|disabled` (+ `--time-limit-factor=N`, default 10): overrides
//!   the mode of the board time limits (Java `-Dfreerouting.parity.timeLimitMode`).
//! * `--optimizer-mode=java-compat|parallel`: overrides the optimizer mode (default: `parallel`
//!   when `optimizer.max_threads > 1`, else `java-compat`).
//! * `--no-time-limits`: count-mode board limits and no stage wall clock limits, without
//!   changing the optimizer mode.
//! * `-v`: debug output.

use std::io::Write as _;
use std::process::ExitCode;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, OnceLock};
use std::time::{Duration, Instant};

use fr_engine::board::{TimeLimitMode, TimeLimitPolicy};
use fr_engine::datastructures::StopToken;
use fr_engine::pipeline::{self, OptimizerMode, PipelineContext};
use fr_settings::{available_processors, headless_merger, CliSettings, DsnFileSettings, EnvironmentSettings};

struct Args {
    design_in: Option<String>,
    design_out: Option<String>,
    parity: bool,
    no_time_limits: bool,
    optimizer_mode: Option<String>,
    time_limit_mode: Option<String>,
    time_limit_factor: i64,
    verbose: bool,
    rest: Vec<String>,
}

fn parse_args() -> Result<Args, String> {
    let mut args = Args {
        design_in: None,
        design_out: None,
        parity: false,
        no_time_limits: false,
        optimizer_mode: None,
        time_limit_mode: None,
        time_limit_factor: fr_engine::datastructures::time_limit::DEFAULT_COUNT_FACTOR,
        verbose: false,
        rest: Vec::new(),
    };
    let raw: Vec<String> = std::env::args().skip(1).collect();
    let mut i = 0;
    while i < raw.len() {
        let a = &raw[i];
        let value = || raw.get(i + 1).cloned().ok_or(format!("{a} needs a value"));
        match a.as_str() {
            "-h" | "--help" => {
                println!(
                    "usage: fastroute -de <design.dsn> [-do <out.ses>] [-mp <passes>] [--router.<path>=<value> ...]\n\
                     \x20      [--parity] [--no-time-limits] [--optimizer-mode=java-compat|parallel]\n\
                     \x20      [--time-limit-mode=wall|count|disabled] [--time-limit-factor=N] [-v]"
                );
                std::process::exit(0);
            }
            "--parity" => {
                args.parity = true;
                i += 1;
                continue;
            }
            "--no-time-limits" => {
                args.no_time_limits = true;
                i += 1;
                continue;
            }
            "-v" => {
                args.verbose = true;
                i += 1;
                continue;
            }
            "-de" => args.design_in = Some(value()?),
            "-do" => args.design_out = Some(value()?),
            _ => {
                if let Some(m) = a.strip_prefix("--optimizer-mode=") {
                    args.optimizer_mode = Some(m.to_string());
                    i += 1;
                    continue;
                }
                if let Some(m) = a.strip_prefix("--time-limit-mode=") {
                    args.time_limit_mode = Some(m.to_string());
                    i += 1;
                    continue;
                }
                if let Some(m) = a.strip_prefix("--time-limit-factor=") {
                    args.time_limit_factor = m.parse().map_err(|_| format!("bad --time-limit-factor '{m}'"))?;
                    i += 1;
                    continue;
                }
            }
        }
        args.rest.push(a.clone());
        i += 1;
    }
    Ok(args)
}

/// Minimal stderr logger: pipeline progress (info) and warnings/errors of all modules.
struct Logger {
    start: Instant,
    verbose: bool,
}

impl log::Log for Logger {
    fn enabled(&self, m: &log::Metadata) -> bool {
        let ours = m.target().starts_with("fr_engine::pipeline") || m.target().starts_with("fastroute") || m.target() == "parity";
        match m.level() {
            log::Level::Error => true,
            // (the engine warns like the Java FRLogger, e.g. about degenerate polylines)
            log::Level::Warn => ours || self.verbose,
            log::Level::Info => ours || self.verbose,
            _ => self.verbose && m.target().starts_with("fr_engine::pipeline"),
        }
    }
    fn log(&self, r: &log::Record) {
        if !self.enabled(r.metadata()) {
            return;
        }
        let t = self.start.elapsed().as_secs_f64();
        let level = match r.level() {
            log::Level::Info => "INFO ",
            log::Level::Warn => "WARN ",
            log::Level::Error => "ERROR",
            _ => "DEBUG",
        };
        let _ = writeln!(std::io::stderr().lock(), "{t:9.3} {level} {}", r.args());
    }
    fn flush(&self) {}
}

static LOGGER: OnceLock<Logger> = OnceLock::new();

fn run() -> Result<(), String> {
    let args = parse_args()?;
    let logger = LOGGER.get_or_init(|| Logger { start: Instant::now(), verbose: args.verbose });
    let _ = log::set_logger(logger);
    log::set_max_level(if args.verbose { log::LevelFilter::Debug } else { log::LevelFilter::Info });

    let cli = CliSettings::parse(&args.rest);
    for w in &cli.warnings {
        log::warn!("{w}");
    }
    let env = EnvironmentSettings::from_process_env();
    let procs = available_processors();

    let Some(path) = args.design_in else {
        return Err("no input design given (-de board.dsn)".into());
    };
    let t = Instant::now();
    let data = std::fs::read(&path).map_err(|e| format!("{path}: {e}"))?;
    let dsn = fr_dsn::Dsn::parse(&data).map_err(|e| format!("{path}: {e}"))?;
    for w in &dsn.warnings {
        log::warn!("{w}");
    }
    let dsn_settings = DsnFileSettings::from_dsn(&dsn);
    let mut settings = headless_merger(&cli, &env, Some(&dsn_settings), None, procs).merge(procs);
    log::info!(
        target: "fastroute",
        "parsed '{}' in {:.1} ms: {} layers, {} components, {} nets, {} wires",
        dsn.name,
        t.elapsed().as_secs_f64() * 1e3,
        dsn.structure.layers.len(),
        dsn.placement.iter().map(|c| c.places.len()).sum::<usize>(),
        dsn.network.nets.len(),
        dsn.wiring.wires.len(),
    );

    // HeadlessBoardManager.loadFromSpecctraDsn + RoutingJobScheduler preparation.
    let t = Instant::now();
    let mut board = fr_io::post_load::load_from_specctra_dsn(&data, &mut settings).map_err(|e| format!("{path}: {e:?}"))?;
    fr_io::post_load::prepare_for_routing(&mut board, &mut settings, None);
    pipeline::deferred_post_load_processing(&mut board);
    log::info!(
        target: "fastroute",
        "built board in {:.1} ms: {} items ({} pins, {} vias, {} traces)",
        t.elapsed().as_secs_f64() * 1e3,
        board.get_items().len(),
        board.get_pins().len(),
        board.get_vias().len(),
        board.get_traces().len(),
    );

    // Time limits and optimizer mode.
    let limits = !(args.parity || args.no_time_limits);
    let fired = Arc::new(AtomicBool::new(false));
    let mode = match args.time_limit_mode.as_deref() {
        Some("wall") => TimeLimitMode::WallClock,
        Some("count") => TimeLimitMode::Count { factor: args.time_limit_factor },
        Some("disabled") => TimeLimitMode::Disabled,
        Some(m) => return Err(format!("unknown time limit mode '{m}' (wall, count or disabled)")),
        None if limits => TimeLimitMode::WallClock,
        None => TimeLimitMode::Count { factor: args.time_limit_factor },
    };
    board.time_limits = TimeLimitPolicy { mode, fired: Some(fired.clone()) };
    let optimizer_threads = settings.optimizer.max_threads.unwrap_or(1).max(1) as usize;
    let optimizer_mode = match args.optimizer_mode.as_deref() {
        Some("java-compat") => OptimizerMode::JavaCompat,
        Some("parallel") => OptimizerMode::Parallel { threads: optimizer_threads },
        Some(m) => return Err(format!("unknown optimizer mode '{m}' (java-compat or parallel)")),
        None if args.parity || optimizer_threads <= 1 => OptimizerMode::JavaCompat,
        None => OptimizerMode::Parallel { threads: optimizer_threads },
    };
    let ctx = PipelineContext { stop: StopToken::new(), wall_clock_limits: limits, optimizer_mode };
    log::info!(
        target: "fastroute",
        "optimizer mode: {optimizer_mode:?}, board time limits: {mode:?}, stage wall clock limits: {}",
        if limits { "enabled" } else { "disabled" }
    );

    // Job timeout (RoutingJobSchedulerActionThread monitor thread).
    if limits {
        if let Some(secs) = settings.job_timeout_string.as_deref().and_then(fr_settings::parse_timespan_string) {
            let secs = secs.min(24 * 3600);
            let stop = ctx.stop.clone();
            std::thread::spawn(move || {
                std::thread::sleep(Duration::from_secs(secs.max(0) as u64));
                log::warn!("job timeout ({secs} s) reached, stopping");
                stop.request_stop();
            });
        }
    }

    let t = Instant::now();
    pipeline::run_pipeline(&mut board, &mut settings, &ctx);
    log::info!(target: "fastroute", "routing finished in {:.2} s", t.elapsed().as_secs_f64());
    if fired.load(Ordering::SeqCst) {
        log::info!(target: "fastroute", "note: a board time limit fired ({mode:?})");
    }

    if let Some(out) = args.design_out {
        // The Java CLI names the session after the input file stem.
        let design_name = std::path::Path::new(&path).file_stem().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
        let bytes = fr_io::ses_writer::ses_bytes(&board, &design_name);
        std::fs::write(&out, &bytes).map_err(|e| format!("{out}: {e}"))?;
        log::info!(target: "fastroute", "saved '{out}' ({} bytes)", bytes.len());
    }
    Ok(())
}

fn main() -> ExitCode {
    match run() {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("fastroute: {e}");
            ExitCode::FAILURE
        }
    }
}
