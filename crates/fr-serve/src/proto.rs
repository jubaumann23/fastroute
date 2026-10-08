//! Wire format of the router protocol (docs/router-protocol/SPEC.md section 1): envelope parsing,
//! error codes, response lines and argument helpers.

use serde_json::{json, Map, Value};

/// Largest request line the server accepts (SPEC 1).
pub const MAX_LINE: usize = 256 * 1024 * 1024;

/// The operations of protocol 1.x, in the order of the request schema.
pub const OPS: [&str; 12] =
    ["hello", "load", "move", "lock", "unlock", "route", "blockers", "congestion", "snapshot", "restore", "export", "shutdown"];

/// A protocol error: the `error` member of a failed response.
#[derive(Debug, Clone)]
pub struct ProtoError {
    pub code: &'static str,
    pub message: String,
    pub details: Option<Value>,
}

pub type R<T> = Result<T, ProtoError>;

impl ProtoError {
    pub fn new(code: &'static str, message: impl Into<String>) -> Self {
        ProtoError { code, message: message.into(), details: None }
    }

    pub fn with_details(mut self, details: Value) -> Self {
        self.details = Some(details);
        self
    }

    pub fn bad_request(message: impl Into<String>) -> Self {
        Self::new("bad_request", message)
    }

    /// The op or argument needs a capability this server does not claim.
    pub fn unsupported(what: &str, capability: &str) -> Self {
        Self::new("unsupported", format!("{what} needs the '{capability}' capability, which this server does not claim"))
            .with_details(json!({ "capability": capability }))
    }

    fn to_value(&self) -> Value {
        let mut e = Map::new();
        e.insert("code".into(), json!(self.code));
        e.insert("message".into(), json!(self.message));
        if let Some(d) = &self.details {
            e.insert("details".into(), d.clone());
        }
        Value::Object(e)
    }
}

/// A parsed request line.
#[derive(Debug)]
pub struct Request {
    pub id: i64,
    pub op: String,
    pub args: Map<String, Value>,
}

/// Parses one request line. The error carries the id to answer with (null when the line had none).
pub fn parse_request(line: &str) -> Result<Request, (Option<i64>, ProtoError)> {
    let value: Value = serde_json::from_str(line)
        .map_err(|e| (None, ProtoError::new("bad_json", format!("the line is not valid JSON: {e}"))))?;
    let Value::Object(mut obj) = value else {
        return Err((None, ProtoError::new("bad_json", "the line is not a JSON object")));
    };
    let id = match obj.get("id").and_then(Value::as_i64) {
        Some(id) if id >= 1 => id,
        _ => return Err((None, ProtoError::bad_request("'id' must be an integer >= 1"))),
    };
    let fail = |msg: String| (Some(id), ProtoError::bad_request(msg));
    if let Some(k) = obj.keys().find(|k| !matches!(k.as_str(), "id" | "op" | "args")) {
        return Err(fail(format!("unknown envelope key '{k}'")));
    }
    let op = match obj.remove("op") {
        Some(Value::String(op)) => op,
        _ => return Err(fail("'op' must be a string".into())),
    };
    let args = match obj.remove("args") {
        None => Map::new(),
        Some(Value::Object(m)) => m,
        Some(_) => return Err(fail("'args' must be an object".into())),
    };
    Ok(Request { id, op, args })
}

/// One success response line (without the newline).
pub fn ok_line(id: i64, build: &str, result: Value) -> String {
    json!({ "id": id, "ok": true, "build": build, "result": result }).to_string()
}

/// One error response line (without the newline).
pub fn err_line(id: Option<i64>, build: &str, err: &ProtoError) -> String {
    json!({ "id": id, "ok": false, "build": build, "error": err.to_value() }).to_string()
}

// ---------------------------------------------------------------------------------------- args

/// `bad_request` for the first key of `args` that is not in `allowed`.
pub fn check_keys(args: &Map<String, Value>, allowed: &[&str], what: &str) -> R<()> {
    match args.keys().find(|k| !allowed.contains(&k.as_str())) {
        Some(k) => Err(ProtoError::bad_request(format!("{what}: unknown argument '{k}'"))),
        None => Ok(()),
    }
}

pub fn req<'a>(args: &'a Map<String, Value>, key: &str, what: &str) -> R<&'a Value> {
    args.get(key).ok_or_else(|| ProtoError::bad_request(format!("{what}: missing argument '{key}'")))
}

pub fn as_str<'a>(v: &'a Value, what: &str) -> R<&'a str> {
    v.as_str().ok_or_else(|| ProtoError::bad_request(format!("{what} must be a string")))
}

pub fn as_bool(v: &Value, what: &str) -> R<bool> {
    v.as_bool().ok_or_else(|| ProtoError::bad_request(format!("{what} must be a boolean")))
}

/// An integer in `lo..=hi`.
pub fn as_int(v: &Value, lo: i64, hi: i64, what: &str) -> R<i64> {
    match v.as_i64() {
        Some(n) if (lo..=hi).contains(&n) => Ok(n),
        _ => Err(ProtoError::bad_request(format!("{what} must be an integer in {lo}..={hi}"))),
    }
}

pub fn as_obj<'a>(v: &'a Value, what: &str) -> R<&'a Map<String, Value>> {
    v.as_object().ok_or_else(|| ProtoError::bad_request(format!("{what} must be an object")))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn envelope_rules() {
        assert_eq!(parse_request("nope").unwrap_err().1.code, "bad_json");
        assert_eq!(parse_request("[1]").unwrap_err().1.code, "bad_json");
        let (id, e) = parse_request(r#"{"op":"hello"}"#).unwrap_err();
        assert_eq!((id, e.code), (None, "bad_request"));
        let (id, e) = parse_request(r#"{"id":4,"op":"hello","x":1}"#).unwrap_err();
        assert_eq!((id, e.code), (Some(4), "bad_request"));
        let r = parse_request(r#"{"id":4,"op":"export"}"#).unwrap();
        assert_eq!((r.id, r.op.as_str(), r.args.len()), (4, "export", 0));
    }
}
