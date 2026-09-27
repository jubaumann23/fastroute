//! `fastroute`: command-line entry point. Accepts the Freerouting CLI
//! arguments (`-de board.dsn -do board.ses -mp N --router.x.y=v ...`).

use std::process::ExitCode;
use std::time::Instant;

use fr_settings::{
    available_processors, headless_merger, CliSettings, DsnFileSettings, EnvironmentSettings,
};

struct Args {
    design_in: Option<String>,
    design_out: Option<String>,
    rest: Vec<String>,
}

fn parse_args() -> Result<Args, String> {
    let mut args = Args {
        design_in: None,
        design_out: None,
        rest: Vec::new(),
    };
    let raw: Vec<String> = std::env::args().skip(1).collect();
    let mut i = 0;
    while i < raw.len() {
        let a = &raw[i];
        let value = || raw.get(i + 1).cloned().ok_or(format!("{a} needs a value"));
        match a.as_str() {
            "-h" | "--help" => {
                println!("usage: fastroute -de <design.dsn> [-do <out.ses>] [-mp <passes>] [--router.<path>=<value> ...]");
                std::process::exit(0);
            }
            "-de" => args.design_in = Some(value()?),
            "-do" => args.design_out = Some(value()?),
            _ => {}
        }
        args.rest.push(a.clone());
        i += 1;
    }
    Ok(args)
}

fn run() -> Result<(), String> {
    let args = parse_args()?;
    let cli = CliSettings::parse(&args.rest);
    for w in &cli.warnings {
        eprintln!("warning: {w}");
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
        eprintln!("warning: {w}");
    }
    let dsn_settings = DsnFileSettings::from_dsn(&dsn);
    let settings = headless_merger(&cli, &env, Some(&dsn_settings), None, procs).merge(procs);
    eprintln!(
        "loaded '{}' in {:.1} ms: {} layers, {} components, {} nets, {} wires",
        dsn.name,
        t.elapsed().as_secs_f64() * 1e3,
        dsn.structure.layers.len(),
        dsn.placement.iter().map(|c| c.places.len()).sum::<usize>(),
        dsn.network.nets.len(),
        dsn.wiring.wires.len(),
    );
    eprintln!(
        "autorouter max passes: {:?}, optimizer threads: {:?}",
        settings.autorouter.max_passes, settings.optimizer.max_threads
    );
    if args.design_out.is_some() {
        return Err("routing is not implemented yet (board model port in progress)".into());
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
