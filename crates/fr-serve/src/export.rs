//! Op `export` (SPEC 5.9): the Specctra session, to a path or inline.

use serde_json::{json, Map, Value};

use crate::proto::{as_str, check_keys, req, ProtoError, R};
use crate::session::Session;
use crate::sha256::sha256_hex;

pub fn handle(session: &mut Session, args: &Map<String, Value>) -> R<Value> {
    check_keys(args, &["format", "path"], "export")?;
    match as_str(req(args, "format", "export")?, "export.format")? {
        "ses" => {}
        other => return Err(ProtoError::bad_request(format!("export.format: '{other}' is not 'ses'"))),
    }
    let path = args.get("path").map(|v| as_str(v, "export.path")).transpose()?;
    if let Some(p) = path {
        if p.is_empty() || !std::path::Path::new(p).is_absolute() {
            return Err(ProtoError::bad_request("export.path must be an absolute path"));
        }
    }
    let loaded = session.loaded()?;
    let bytes = fr_io::ses_writer::ses_bytes(&loaded.board, &loaded.name);
    let mut result = json!({ "format": "ses", "bytes": bytes.len(), "sha256": sha256_hex(&bytes) });
    match path {
        Some(p) => {
            std::fs::write(p, &bytes)
                .map_err(|e| ProtoError::new("io_error", format!("cannot write '{p}': {e}")).with_details(json!({ "path": p })))?;
            result["path"] = json!(p);
        }
        None => {
            let text = String::from_utf8(bytes).map_err(|_| ProtoError::new("internal", "the session is not valid UTF-8"))?;
            result["text"] = json!(text);
        }
    }
    Ok(result)
}
