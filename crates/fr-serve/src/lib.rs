//! `fastroute serve`: the router side of the claude-pcb-rules router protocol (JSON lines over
//! stdin/stdout, docs/router-protocol/SPEC.md in that repository). One request line, one response
//! line; logs go to stderr only. All protocol logic lives in this crate; the CLI only dispatches
//! `fastroute serve` here.
//!
//! Hermetic: the `FREEROUTING__ROUTER__*` environment is NOT read (SPEC 7); only the hello settings
//! and the DSN's own settings reach the router, so the cache key covers every input.

mod facts;
mod load;
mod export;
mod proto;
mod route;
mod session;
mod settings;
mod sha256;

pub mod ops;

/// In-process probe for the blockers falsification test (test-only).
#[cfg(feature = "test-hooks")]
#[doc(hidden)]
pub mod falsify;

/// Re-exported for the CLI crate's protocol tests (the only place besides this crate that speaks JSON).
#[doc(hidden)]
pub use serde_json;

use std::io::{BufRead, Write};
use std::panic::{catch_unwind, AssertUnwindSafe};
use std::process::ExitCode;

use serde_json::{json, Map, Value};

use proto::{err_line, ok_line, ProtoError, Request, MAX_LINE, OPS, R};
use session::Session;

/// The protocol version this server implements.
pub const PROTOCOL: &str = "1.0.0";

/// `build` (SPEC 7): sha256 of the running executable, computed once.
fn build_hash() -> Result<String, String> {
    let exe = std::fs::read("/proc/self/exe")
        .or_else(|_| std::env::current_exe().and_then(std::fs::read))
        .map_err(|e| format!("cannot read the executable for the build hash: {e}"))?;
    Ok(sha256::sha256_hex(&exe))
}

/// Minimal stderr logger (warnings of all modules, pipeline progress).
struct Logger;

impl log::Log for Logger {
    fn enabled(&self, m: &log::Metadata) -> bool {
        m.level() <= log::Level::Warn || (m.level() == log::Level::Info && m.target().starts_with("fr_engine::pipeline"))
    }
    fn log(&self, r: &log::Record) {
        if self.enabled(r.metadata()) {
            let _ = writeln!(std::io::stderr().lock(), "fastroute serve {} {}", r.level(), r.args());
        }
    }
    fn flush(&self) {}
}

static LOGGER: Logger = Logger;

enum Line {
    Eof,
    Text(String),
    TooLong,
    NotUtf8,
}

/// Reads one line of at most [`MAX_LINE`] bytes; a longer one is drained and reported.
fn read_line(r: &mut impl BufRead) -> std::io::Result<Line> {
    let mut buf: Vec<u8> = Vec::new();
    let mut too_long = false;
    loop {
        let chunk = r.fill_buf()?;
        if chunk.is_empty() {
            if buf.is_empty() && !too_long {
                return Ok(Line::Eof);
            }
            break;
        }
        let (take, found) = match chunk.iter().position(|&b| b == b'\n') {
            Some(i) => (i + 1, true),
            None => (chunk.len(), false),
        };
        if !too_long {
            if buf.len() + take > MAX_LINE + 1 {
                too_long = true;
                buf = Vec::new();
            } else {
                buf.extend_from_slice(&chunk[..take]);
            }
        }
        r.consume(take);
        if found {
            break;
        }
    }
    if too_long {
        return Ok(Line::TooLong);
    }
    while matches!(buf.last(), Some(b'\n' | b'\r')) {
        buf.pop();
    }
    Ok(String::from_utf8(buf).map_or(Line::NotUtf8, Line::Text))
}

fn hello(session: &mut Session, args: &Map<String, Value>, build: &str) -> R<Value> {
    use proto::{as_int, as_obj, as_str, check_keys, req};
    check_keys(args, &["protocol", "client", "threads", "settings"], "hello")?;
    let protocol = as_str(req(args, "protocol", "hello")?, "hello.protocol")?;
    let parts: Vec<&str> = protocol.split('.').collect();
    if parts.len() != 3 || parts.iter().any(|p| p.is_empty() || !p.bytes().all(|b| b.is_ascii_digit())) {
        return Err(ProtoError::bad_request("hello.protocol must be MAJOR.MINOR.PATCH"));
    }
    if as_str(req(args, "client", "hello")?, "hello.client")?.is_empty() {
        return Err(ProtoError::bad_request("hello.client must not be empty"));
    }
    let threads = as_int(req(args, "threads", "hello")?, 1, 4096, "hello.threads")? as usize;
    let given = args.get("settings").map(|v| as_obj(v, "hello.settings")).transpose()?;
    let settings = settings::SessionSettings::from_hello(threads, given)?;
    if session.hello.is_some() {
        return Err(ProtoError::new("bad_state", "hello was already done in this session"));
    }
    if parts[0] != PROTOCOL.split('.').next().unwrap_or("") {
        session.refused = true;
        return Err(ProtoError::new(
            "version_mismatch",
            format!("this server speaks protocol {PROTOCOL}; the client sent {protocol}"),
        )
        .with_details(json!({ "server": PROTOCOL, "client": protocol })));
    }
    let result = json!({
        "protocol": PROTOCOL,
        "router": {
            "name": "fastroute",
            "version": env!("CARGO_PKG_VERSION"),
            "upstream": format!("parisxmas/fastroute {}", env!("CARGO_PKG_VERSION")),
        },
        "build": build,
        "threads": threads,
        "capabilities": ops::capabilities(),
        "settings": { "applied": settings.applied, "unknown": settings.unknown },
    });
    session.hello = Some(settings);
    Ok(result)
}

/// Answers one parsed request; `Ok(None)` result means shutdown was handled.
fn dispatch(session: &mut Session, req: &Request, build: &str) -> R<Value> {
    // test hook, compiled only with the `test-hooks` feature (SPEC 7: the shipped server reads no
    // environment): `FR_SERVE_TEST_PANIC=<op>` makes that op panic, to reach the `internal` error
    #[cfg(feature = "test-hooks")]
    if std::env::var("FR_SERVE_TEST_PANIC").is_ok_and(|op| op == req.op) {
        panic!("FR_SERVE_TEST_PANIC");
    }
    if !OPS.contains(&req.op.as_str()) {
        return Err(ProtoError::new("unknown_op", format!("no such op '{}'", req.op)));
    }
    if req.op == "shutdown" {
        proto::check_keys(&req.args, &[], "shutdown")?;
        return Ok(json!({}));
    }
    if session.refused {
        return Err(ProtoError::new("version_mismatch", "the protocol version was refused: only shutdown is accepted"));
    }
    if req.op == "hello" {
        return hello(session, &req.args, build);
    }
    if session.hello.is_none() {
        return Err(ProtoError::new("no_hello", format!("'{}' before the hello handshake", req.op)));
    }
    match req.op.as_str() {
        "load" => load::handle(session, &req.args),
        "route" => route::handle(session, &req.args),
        "export" => export::handle(session, &req.args),
        _ => ops::dispatch(session, &req.op, &req.args),
    }
}

fn respond(out: &mut impl Write, line: &str) -> std::io::Result<()> {
    out.write_all(line.as_bytes())?;
    out.write_all(b"\n")?;
    out.flush()
}

/// Serves the protocol on stdin/stdout until `shutdown` or end of input.
pub fn run() -> ExitCode {
    let _ = log::set_logger(&LOGGER);
    log::set_max_level(log::LevelFilter::Info);
    let build = match build_hash() {
        Ok(b) => b,
        Err(e) => {
            eprintln!("fastroute serve: {e}");
            return ExitCode::FAILURE;
        }
    };
    let stdin = std::io::stdin();
    let mut input = stdin.lock();
    let stdout = std::io::stdout();
    let mut out = stdout.lock();
    let mut session = Session::new();
    loop {
        let text = match read_line(&mut input) {
            Ok(Line::Eof) => return ExitCode::SUCCESS, // implicit shutdown
            Ok(Line::Text(t)) => t,
            Ok(Line::TooLong) => {
                let e = ProtoError::new("bad_json", format!("the line is longer than {MAX_LINE} bytes"));
                if respond(&mut out, &err_line(None, &build, &e)).is_err() {
                    return ExitCode::FAILURE;
                }
                continue;
            }
            Ok(Line::NotUtf8) => {
                let e = ProtoError::new("bad_json", "the line is not valid UTF-8");
                if respond(&mut out, &err_line(None, &build, &e)).is_err() {
                    return ExitCode::FAILURE;
                }
                continue;
            }
            Err(e) => {
                eprintln!("fastroute serve: cannot read stdin: {e}");
                return ExitCode::FAILURE;
            }
        };
        let (line, shutdown) = match proto::parse_request(&text) {
            Err((id, e)) => (err_line(id, &build, &e), false),
            Ok(req) => {
                let outcome = catch_unwind(AssertUnwindSafe(|| dispatch(&mut session, &req, &build)));
                match outcome {
                    Ok(Ok(result)) => (ok_line(req.id, &build, result), req.op == "shutdown"),
                    Ok(Err(e)) => (err_line(Some(req.id), &build, &e), false),
                    Err(panic) => {
                        let msg = panic
                            .downcast_ref::<&str>()
                            .map(|s| s.to_string())
                            .or_else(|| panic.downcast_ref::<String>().cloned())
                            .unwrap_or_else(|| "panic".into());
                        log::error!("internal error in '{}': {msg}", req.op);
                        let e = ProtoError::new("internal", format!("internal error in '{}': {msg}", req.op));
                        (err_line(Some(req.id), &build, &e), false)
                    }
                }
            }
        };
        if respond(&mut out, &line).is_err() {
            return ExitCode::FAILURE;
        }
        if shutdown {
            return ExitCode::SUCCESS;
        }
    }
}
