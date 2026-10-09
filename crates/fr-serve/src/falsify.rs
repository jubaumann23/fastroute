//! Test support (feature `test-hooks`, never in a shipped build): drives one in-process session so a
//! test can take objects off the board, which the protocol has no op for, and check that the objects
//! `blockers` names are causal. Used by `crates/fastroute/tests/serve_blockers.rs`.

use serde_json::{json, Map, Value};

use crate::ops::blockers;
use crate::session::Session;

pub struct Probe {
    session: Session,
}

impl Probe {
    /// A session with `hello` and `load` done.
    pub fn open(dsn: &str, threads: u32) -> Probe {
        let mut p = Probe { session: Session::new() };
        let hello = json!({ "protocol": "1.0.0", "client": "falsify", "threads": threads });
        crate::hello(&mut p.session, hello.as_object().unwrap(), "falsify").expect("hello");
        p.call("load", json!({ "dsn": { "path": dsn } }));
        p
    }

    /// One op, its result; panics on a protocol error.
    pub fn call(&mut self, op: &str, args: Value) -> Value {
        let a: &Map<String, Value> = args.as_object().expect("args are an object");
        let r = match op {
            "load" => crate::load::handle(&mut self.session, a),
            "route" => crate::route::handle(&mut self.session, a),
            _ => crate::ops::dispatch(&mut self.session, op, a),
        };
        match r {
            Ok(v) => v,
            Err(e) => panic!("{op}: {}", e.message),
        }
    }

    /// Removes the items with these ids from the board (no re-route); the number removed.
    pub fn rip(&mut self, ids: &[i64]) -> usize {
        let board = &mut self.session.board.as_mut().expect("loaded").board;
        let keys: Vec<_> = ids.iter().filter_map(|&id| board.get_item(fr_engine::ids::ItemId(id as i32))).collect();
        let n = keys.len();
        board.remove_items(keys);
        n
    }

    /// Routes `connection` alone (as `blockers` does) after taking the items `remove` off the alone
    /// board: true when it routes.
    pub fn alone_without(&self, connection: &Value, remove: &[i64]) -> bool {
        let loaded = self.session.loaded().expect("loaded");
        let board = &loaded.board;
        let (net, from, to) = (connection["net"].as_str().unwrap(), connection["from"].as_str().unwrap(), connection["to"].as_str().unwrap());
        let (_, open) = crate::facts::connections(board);
        let conn = open
            .iter()
            .find(|u| u.net == net && ((u.from == from && u.to == to) || (u.from == to && u.to == from)))
            .expect("the connection is open");
        let ids: Vec<i32> = remove.iter().map(|&i| i as i32).collect();
        blockers::alone_route(board, conn, conn.net_no, &loaded.settings, &ids, false).is_some_and(|r| r.routed)
    }

    /// Item ids of the wires and vias among the listed `blockers` objects.
    pub fn wire_ids(list: &[Value]) -> Vec<i64> {
        list.iter().filter(|x| matches!(x["kind"].as_str(), Some("wire" | "via"))).filter_map(|x| x["id"].as_i64()).collect()
    }
}
