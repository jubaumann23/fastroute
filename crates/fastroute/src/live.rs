//! `--live`: a local web viewer of the routing (http://127.0.0.1:PORT).
//!
//! A small HTTP server on its own threads (std only): `/` serves the viewer page, `/events` a
//! server-sent event stream, `POST /stop` asks the router to stop with the best result so far.
//! The pipeline reports its progress through [`PipelineContext::observer`]; the board is only
//! read, so routing results do not change with `--live`.
//!
//! Events (`event: <name>`, one JSON object per `data:` line):
//! * `board`: static geometry once (layers, nets, outline, pads, keepouts, planes), board units
//! * `frame`: the wiring (traces, vias, airlines), throttled to a few per second
//! * `tick`: progress inside an autorouting pass (cheap, ~10 per second)
//! * `pass`, `stage`, `log`, `done`
//!
//! [`PipelineContext::observer`]: fr_engine::pipeline::PipelineContext::observer

use std::collections::VecDeque;
use std::fmt::Write as _;
use std::io::{BufRead, BufReader, Write};
use std::net::{TcpListener, TcpStream};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::mpsc::{sync_channel, SyncSender, TrySendError};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use fr_engine::board::{BasicBoard, ItemKey, ItemKind, RoutingBoard};
use fr_engine::datastructures::StopToken;
use fr_engine::drc::NetIncompletes;
use fr_engine::ids::NetNo;
use fr_engine::pipeline::LiveEvent;
use fr_geom::{FloatPoint, Shape};

const PAGE: &str = include_str!("live.html");
/// Minimum time between two `frame` events (more when serializing takes long, see
/// [`FRAME_COST_FACTOR`]).
const FRAME_INTERVAL: Duration = Duration::from_millis(250);
/// A frame may take at most 1/FRAME_COST_FACTOR of the routing thread's time.
const FRAME_COST_FACTOR: u32 = 10;
const TICK_INTERVAL: Duration = Duration::from_millis(100);
/// Stage, pass and log events kept for viewers that connect later.
const BACKLOG: usize = 4000;
/// Queued messages per viewer; more are dropped (a slow viewer misses frames).
const CLIENT_QUEUE: usize = 64;

struct Client {
    tx: SyncSender<Arc<str>>,
    pending: Arc<AtomicUsize>,
}

#[derive(Default)]
struct State {
    clients: Vec<Client>,
    board: Option<Arc<str>>,
    events: VecDeque<Arc<str>>,
    frame: Option<Arc<str>>,
    tick: Option<Arc<str>>,
    next_frame: Option<Instant>,
    next_tick: Option<Instant>,
}

pub struct Live {
    start: Instant,
    stop: StopToken,
    state: Mutex<State>,
}

impl Live {
    /// Starts the server on 127.0.0.1:`port` (or one of the next ports if it is taken).
    pub fn start(port: u16, stop: StopToken, start: Instant) -> Result<(Arc<Live>, String), String> {
        let listener = (port..port.saturating_add(10))
            .find_map(|p| TcpListener::bind(("127.0.0.1", p)).ok())
            .ok_or(format!("--live: no free port in {port}..{}", port.saturating_add(9)))?;
        let url = format!("http://{}", listener.local_addr().map_err(|e| e.to_string())?);
        let live = Arc::new(Live { start, stop, state: Mutex::new(State::default()) });
        let l = live.clone();
        std::thread::spawn(move || {
            for stream in listener.incoming().flatten() {
                let l = l.clone();
                std::thread::spawn(move || l.serve(stream));
            }
        });
        Ok((live, url))
    }

    /// The pipeline observer.

    pub fn observe(&self, board: &RoutingBoard, ev: &LiveEvent) {
        let t = self.secs();
        match *ev {
            LiveEvent::Stage(name) => {
                self.event("stage", format!("{{\"t\":{t:.3},\"stage\":{}}}", json_str(name)));
                self.frame(board, true);
            }
            LiveEvent::Connection { pass_no, done, total, counters: c } => {
                let now = Instant::now();
                let tick_due = {
                    let mut st = self.state.lock().unwrap();
                    let due = st.next_tick.is_none_or(|n| now >= n);
                    if due {
                        st.next_tick = Some(now + TICK_INTERVAL);
                    }
                    due
                };
                if tick_due {
                    let msg = format!(
                        "{{\"t\":{t:.3},\"pass\":{pass_no},\"done\":{done},\"total\":{total},\"routed\":{},\"failed\":{},\"ripped\":{},\"skipped\":{}}}",
                        c.routed, c.not_routed, c.ripped, c.skipped
                    );
                    let msg: Arc<str> = sse("tick", &msg).into();
                    let mut st = self.state.lock().unwrap();
                    st.tick = Some(msg.clone());
                    broadcast(&mut st, &msg);
                }
                self.frame(board, false);
            }
            LiveEvent::Fanout { .. } => {}
            LiveEvent::RouterPass { pass_no, secs, counters: c, incomplete, violations, score } => {
                self.event(
                    "pass",
                    format!(
                        "{{\"t\":{t:.3},\"kind\":\"router\",\"pass\":{pass_no},\"secs\":{secs:.3},\"incomplete\":{incomplete},\"violations\":{violations},\"score\":{},\"routed\":{},\"failed\":{},\"ripped\":{}}}",
                        num(score as f64),
                        c.routed,
                        c.not_routed,
                        c.ripped
                    ),
                );
                self.frame(board, false);
            }
            LiveEvent::OptimizerPass { pass_no, secs, incomplete, violations, score } => {
                self.event(
                    "pass",
                    format!(
                        "{{\"t\":{t:.3},\"kind\":\"optimizer\",\"pass\":{pass_no},\"secs\":{secs:.3},\"incomplete\":{incomplete},\"violations\":{violations},\"score\":{}}}",
                        num(score as f64)
                    ),
                );
                self.frame(board, false);
            }
        }
    }

    /// Sends the static geometry of `board` (pins, outline, keepouts do not move while routing).
    pub fn publish_board(&self, board: &BasicBoard, name: &str) {
        let msg: Arc<str> = sse("board", &board_json(board, name)).into();
        let mut st = self.state.lock().unwrap();
        st.board = Some(msg.clone());
        broadcast(&mut st, &msg);
    }

    pub fn log(&self, level: &str, text: &str) {
        let t = self.secs();
        self.event("log", format!("{{\"t\":{t:.3},\"level\":{},\"text\":{}}}", json_str(level.trim()), json_str(text)));
    }

    /// The end of the run: the final board, then waits (at most `wait`) until the viewers got
    /// everything.
    pub fn finish(&self, board: &RoutingBoard, wait: Duration) {
        self.frame(board, true);
        let t = self.secs();
        self.event("done", format!("{{\"t\":{t:.3}}}"));
        let deadline = Instant::now() + wait;
        while Instant::now() < deadline {
            let busy = self.state.lock().unwrap().clients.iter().any(|c| c.pending.load(Ordering::SeqCst) > 0);
            if !busy {
                break;
            }
            std::thread::sleep(Duration::from_millis(20));
        }
    }

    fn secs(&self) -> f64 {
        self.start.elapsed().as_secs_f64()
    }

    fn event(&self, name: &str, json: String) {
        let msg: Arc<str> = sse(name, &json).into();
        let mut st = self.state.lock().unwrap();
        if st.events.len() >= BACKLOG {
            st.events.pop_front();
        }
        st.events.push_back(msg.clone());
        broadcast(&mut st, &msg);
    }

    /// Sends the wiring if a frame is due (or `force`) and someone is watching.
    fn frame(&self, board: &RoutingBoard, force: bool) {
        let now = Instant::now();
        {
            let st = self.state.lock().unwrap();
            if st.clients.is_empty() && !force {
                return;
            }
            if !force && st.next_frame.is_some_and(|n| now < n) {
                return;
            }
        }
        let json = frame_json(board, self.secs());
        let cost = now.elapsed();
        let msg: Arc<str> = sse("frame", &json).into();
        {
            let mut st = self.state.lock().unwrap();
            st.next_frame = Some(Instant::now() + FRAME_INTERVAL.max(cost * FRAME_COST_FACTOR));
            st.frame = Some(msg.clone());
            broadcast(&mut st, &msg);
        }
        log::debug!(target: "fastroute::live", "live frame: {} kB in {:.1} ms", msg.len() / 1024, cost.as_secs_f64() * 1e3);
    }

    fn serve(&self, mut stream: TcpStream) {
        let mut reader = BufReader::new(match stream.try_clone() {
            Ok(s) => s,
            Err(_) => return,
        });
        let mut request = String::new();
        if reader.read_line(&mut request).is_err() {
            return;
        }
        // skip the headers
        let mut line = String::new();
        while reader.read_line(&mut line).is_ok_and(|n| n > 2) {
            line.clear();
        }
        let mut parts = request.split_whitespace();
        let (method, path) = (parts.next().unwrap_or(""), parts.next().unwrap_or(""));
        match (method, path) {
            ("GET", "/") | ("GET", "/index.html") => {
                let _ = write!(
                    stream,
                    "HTTP/1.1 200 OK\r\nContent-Type: text/html; charset=utf-8\r\nContent-Length: {}\r\nCache-Control: no-cache\r\nConnection: close\r\n\r\n{PAGE}",
                    PAGE.len()
                );
            }
            ("GET", "/events") => self.stream_events(stream),
            ("POST", "/stop") => {
                log::warn!(target: "fastroute", "stop requested from the live viewer: finishing with the best result so far");
                self.stop.request_stop();
                super::stop_watchdog();
                let _ = stream.write_all(b"HTTP/1.1 204 No Content\r\nConnection: close\r\n\r\n");
            }
            _ => {
                let _ = stream.write_all(b"HTTP/1.1 404 Not Found\r\nContent-Length: 0\r\nConnection: close\r\n\r\n");
            }
        }
    }

    fn stream_events(&self, mut stream: TcpStream) {
        if stream
            .write_all(b"HTTP/1.1 200 OK\r\nContent-Type: text/event-stream\r\nCache-Control: no-cache\r\nConnection: keep-alive\r\n\r\n")
            .is_err()
        {
            return;
        }
        let (tx, rx) = sync_channel::<Arc<str>>(CLIENT_QUEUE);
        let pending = Arc::new(AtomicUsize::new(0));
        // what happened so far, then the live events (registered under the same lock: nothing
        // is missed or sent twice)
        let backlog: Vec<Arc<str>> = {
            let mut st = self.state.lock().unwrap();
            st.clients.push(Client { tx, pending: pending.clone() });
            st.board.iter().chain(st.events.iter()).chain(st.frame.iter()).chain(st.tick.iter()).cloned().collect()
        };
        let _ = stream.set_nodelay(true);
        for m in backlog {
            if stream.write_all(m.as_bytes()).is_err() {
                return;
            }
        }
        loop {
            match rx.recv_timeout(Duration::from_secs(15)) {
                Ok(m) => {
                    let ok = stream.write_all(m.as_bytes()).is_ok();
                    pending.fetch_sub(1, Ordering::SeqCst);
                    if !ok {
                        return;
                    }
                }
                Err(std::sync::mpsc::RecvTimeoutError::Timeout) => {
                    if stream.write_all(b": keep-alive\n\n").is_err() {
                        return;
                    }
                }
                Err(_) => return,
            }
        }
    }
}

fn broadcast(st: &mut State, msg: &Arc<str>) {
    st.clients.retain(|c| {
        c.pending.fetch_add(1, Ordering::SeqCst);
        match c.tx.try_send(msg.clone()) {
            Ok(()) => true,
            Err(TrySendError::Full(_)) => {
                c.pending.fetch_sub(1, Ordering::SeqCst);
                true
            }
            Err(TrySendError::Disconnected(_)) => false,
        }
    });
}

fn sse(event: &str, json: &str) -> String {
    format!("event: {event}\ndata: {json}\n\n")
}

fn json_str(s: &str) -> String {
    let mut out = String::with_capacity(s.len() + 2);
    out.push('"');
    for ch in s.chars() {
        match ch {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            c if (c as u32) < 0x20 => {
                let _ = write!(out, "\\u{:04x}", c as u32);
            }
            c => out.push(c),
        }
    }
    out.push('"');
    out
}

/// A finite JSON number (NaN / infinity as null).
fn num(v: f64) -> String {
    if v.is_finite() {
        format!("{v:.3}")
    } else {
        "null".into()
    }
}

fn push_points(out: &mut String, pts: &[FloatPoint]) {
    for p in pts {
        let _ = write!(out, ",{},{}", p.x.round() as i64, p.y.round() as i64);
    }
}

/// `[layer,net,0,cx,cy,r]` for a circle, `[layer,net,1,x,y,...]` for a polygon.
fn push_shape(out: &mut String, layer: i32, net: NetNo, shape: &Shape) -> bool {
    match shape {
        Shape::Circle(c) => {
            let _ = write!(out, "[{layer},{net},0,{},{},{}]", c.center.x, c.center.y, c.radius);
        }
        s => {
            let pts = s.corner_approx_arr();
            if pts.len() < 2 {
                return false;
            }
            let _ = write!(out, "[{layer},{net},1");
            push_points(out, &pts);
            out.push(']');
        }
    }
    true
}

fn first_net(board: &BasicBoard, key: ItemKey) -> NetNo {
    board.item(key).net_numbers().first().copied().unwrap_or(0)
}

fn board_json(board: &BasicBoard, name: &str) -> String {
    let upm = board.communication.resolution.max(1) as f64 * 1000.0;
    let bb = board.bounding_box();
    let mut out = String::new();
    let _ = write!(out, "{{\"name\":{},\"upm\":{upm},\"bbox\":[{},{},{},{}],\"layers\":[", json_str(name), bb.ll.x, bb.ll.y, bb.ur.x, bb.ur.y);
    for (i, l) in board.layer_structure.layers.iter().enumerate() {
        if i > 0 {
            out.push(',');
        }
        let _ = write!(out, "{{\"name\":{},\"signal\":{}}}", json_str(&l.name), l.is_signal);
    }
    out.push_str("],\"nets\":[\"\"");
    for n in 1..=board.rules.nets.max_net_number() {
        out.push(',');
        out.push_str(&json_str(board.rules.nets.get(n).map(|n| n.name.as_str()).unwrap_or("")));
    }
    let (mut outline, mut pads, mut keepouts, mut planes) = (String::new(), String::new(), String::new(), String::new());
    let sep = |s: &mut String| {
        if !s.is_empty() {
            s.push(',');
        }
    };
    for key in board.get_items() {
        let it = board.item(key);
        match &it.kind {
            ItemKind::BoardOutline(o) => {
                for s in o.shapes() {
                    sep(&mut outline);
                    outline.push_str("[0");
                    push_points(&mut outline, &s.corner_approx_arr());
                    outline.push(']');
                }
            }
            ItemKind::Pin(_) => {
                let net = first_net(board, key);
                for layer in it.first_layer(board)..=it.last_layer(board) {
                    if let Some(shape) = it.drill_shape_on_layer(board, layer) {
                        let mark = pads.len();
                        sep(&mut pads);
                        if !push_shape(&mut pads, layer, net, &shape) {
                            pads.truncate(mark);
                        }
                    }
                }
            }
            ItemKind::ObstacleArea(a) => {
                let pts = a.get_area(board).corner_approx_arr();
                if pts.len() >= 3 {
                    sep(&mut keepouts);
                    let _ = write!(keepouts, "[{}", a.layer());
                    push_points(&mut keepouts, &pts);
                    keepouts.push(']');
                }
            }
            ItemKind::ConductionArea(c) => {
                let pts = c.area().get_area(board).corner_approx_arr();
                if pts.len() >= 3 {
                    sep(&mut planes);
                    let _ = write!(planes, "[{},{}", c.area().layer(), first_net(board, key));
                    push_points(&mut planes, &pts);
                    planes.push(']');
                }
            }
            _ => {}
        }
    }
    let _ = write!(out, "],\"outline\":[{outline}],\"pads\":[{pads}],\"keepouts\":[{keepouts}],\"planes\":[{planes}]}}");
    out
}

/// The wiring: `traces` `[id,layer,half_width,net,fixed,x0,y0,x1,y1,...]`, `vias`
/// `[id,x,y,radius,net,fixed]`, `air` (the unrouted connections) `[net,x0,y0,x1,y1]`.
fn frame_json(board: &RoutingBoard, t: f64) -> String {
    let upm = board.communication.resolution.max(1) as f64 * 1000.0;
    let max_net = board.rules.nets.max_net_number().max(0) as usize;
    let mut net_items: Vec<Vec<ItemKey>> = vec![Vec::new(); max_net];
    let (mut traces, mut vias) = (String::new(), String::new());
    let (mut n_traces, mut n_vias, mut length) = (0usize, 0usize, 0.0f64);
    for key in board.get_items() {
        let it = board.item(key);
        if it.is_connectable_class() {
            for &n in it.net_numbers() {
                if n >= 1 && (n as usize) <= max_net {
                    net_items[n as usize - 1].push(key);
                }
            }
        }
        let fixed = it.is_user_fixed() as u8;
        match &it.kind {
            ItemKind::Trace(tr) => {
                if n_traces > 0 {
                    traces.push(',');
                }
                n_traces += 1;
                let pts: Vec<FloatPoint> = tr.polyline().corners().iter().map(|c| c.to_float()).collect();
                length += pts.windows(2).map(|w| w[0].distance(&w[1])).sum::<f64>();
                let _ = write!(traces, "[{},{},{},{},{fixed}", it.id().0, tr.layer(), tr.half_width(), first_net(board, key));
                push_points(&mut traces, &pts);
                traces.push(']');
            }
            ItemKind::Via(_) => {
                if n_vias > 0 {
                    vias.push(',');
                }
                n_vias += 1;
                let c = it.center(board).to_float();
                let b = it.bounding_box(board);
                let r = ((b.ur.x as i64 - b.ll.x as i64) / 2).max(1);
                let _ = write!(vias, "[{},{},{},{r},{},{fixed}]", it.id().0, c.x.round() as i64, c.y.round() as i64, first_net(board, key));
            }
            _ => {}
        }
    }
    let mut air = String::new();
    let mut unrouted = 0usize;
    for (i, list) in net_items.iter().enumerate() {
        if list.len() < 2 {
            continue;
        }
        for a in NetIncompletes::new(board, i as NetNo + 1, list).incompletes {
            if unrouted > 0 {
                air.push(',');
            }
            unrouted += 1;
            let _ = write!(
                air,
                "[{},{},{},{},{}]",
                a.net_number,
                a.from_corner.x.round() as i64,
                a.from_corner.y.round() as i64,
                a.to_corner.x.round() as i64,
                a.to_corner.y.round() as i64
            );
        }
    }
    format!(
        "{{\"t\":{t:.3},\"length_mm\":{:.1},\"unrouted\":{unrouted},\"traces\":[{traces}],\"vias\":[{vias}],\"air\":[{air}]}}",
        length / upm
    )
}
