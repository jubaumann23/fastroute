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
    let mut settings = headless_merger(&cli, &env, Some(&dsn_settings), None, procs).merge(procs);
    eprintln!(
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
    let mut board = fr_io::post_load::load_from_specctra_dsn(&data, &mut settings)
        .map_err(|e| format!("{path}: {e:?}"))?;
    fr_io::post_load::prepare_for_routing(&mut board, &mut settings, None);
    eprintln!(
        "built board in {:.1} ms: {} items ({} pins, {} vias, {} traces)",
        t.elapsed().as_secs_f64() * 1e3,
        board.get_items().len(),
        board.get_pins().len(),
        board.get_vias().len(),
        board.get_traces().len(),
    );

    if let Some(out) = args.design_out {
        eprintln!("warning: routing is not implemented yet; writing the unrouted session");
        // The Java CLI names the session after the input file stem.
        let design_name = std::path::Path::new(&path)
            .file_stem()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_default();
        let bytes = fr_io::ses_writer::ses_bytes(&board, &design_name);
        std::fs::write(&out, bytes).map_err(|e| format!("{out}: {e}"))?;
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
