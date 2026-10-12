//! MCP server for TuneOx over stdio, so LLM agents can drive the library.
//!
//! Tools come straight from the `tuneox-commands` registry. The server speaks
//! both protocol eras on one connection:
//!
//! - **Modern** (`2026-07-28`): stateless. Every request names its protocol
//!   version in `_meta`; results carry `resultType`; `server/discover`
//!   describes the server.
//! - **Legacy** (`2025-11-25` and earlier): an `initialize` handshake
//!   negotiates the version; results have no `resultType`.

use std::cell::RefCell;
use std::collections::HashSet;
use std::io::{self, BufRead, Write};
use std::rc::Rc;

use serde_json::{Map, Value, json};
use tuneox_commands::{Context, Effect};

/// Versions served with per-request `_meta`, newest first.
pub const MODERN_VERSIONS: &[&str] = &["2026-07-28"];
/// Versions served after an `initialize` handshake, newest first.
pub const LEGACY_VERSIONS: &[&str] = &["2025-11-25", "2025-06-18", "2025-03-26", "2024-11-05"];

const PROTOCOL_VERSION: &str = "io.modelcontextprotocol/protocolVersion";
const CLIENT_CAPABILITIES: &str = "io.modelcontextprotocol/clientCapabilities";
const SERVER_INFO: &str = "io.modelcontextprotocol/serverInfo";

const PARSE_ERROR: i64 = -32700;
const INVALID_REQUEST: i64 = -32600;
const METHOD_NOT_FOUND: i64 = -32601;
const INVALID_PARAMS: i64 = -32602;
const UNSUPPORTED_PROTOCOL_VERSION: i64 = -32022;

const INSTRUCTIONS: &str = "TuneOx is a DJ library manager, an open-source rekordbox alternative. \
These tools read and change the user's TuneOx library. Track and playlist IDs are integers \
returned by search_tracks and list_playlists. Cue and beat positions are microseconds from the \
start of the track. Keys use classic notation (Am, F#m, Db); Camelot (8A) and Open Key (1m) are \
accepted as input. To build a harmonic set, use search_tracks with bpm_min, bpm_max and \
compatible_with_key. If the library is empty, offer to run import_rekordbox, previewing with \
dry_run first.";

/// Serves MCP requests read line by line from `input`, writing responses
/// and notifications to `output`, until `input` ends. Returns `output`.
pub fn serve<R: BufRead, W: Write + 'static>(
    ctx: &mut Context,
    input: R,
    output: W,
) -> io::Result<W> {
    let out = Rc::new(RefCell::new(output));
    let server = Server::new();
    for line in input.lines() {
        let line = line?;
        if line.trim().is_empty() {
            continue;
        }
        if let Some(response) = server.handle_line(ctx, &line, &out) {
            write_message(&out, &response)?;
        }
    }
    // Drop any progress handler still holding the writer.
    ctx.set_progress(|_| {});
    let out = Rc::try_unwrap(out)
        .map_err(|_| io::Error::other("MCP output still in use"))?
        .into_inner();
    Ok(out)
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Era {
    Modern,
    Legacy,
}

struct Server {
    tools: Value,
    names: HashSet<&'static str>,
}

type Failure = (i64, String);

impl Server {
    fn new() -> Self {
        let catalog = tuneox_commands::catalog();
        let names = catalog.iter().map(|c| c.name).collect();
        let tools = catalog
            .into_iter()
            .map(|c| {
                json!({
                    "name": c.name,
                    "title": title(c.name),
                    "description": c.description,
                    "inputSchema": c.input_schema,
                    "annotations": annotations(c.effect),
                })
            })
            .collect();
        Self { tools, names }
    }

    fn handle_line(
        &self,
        ctx: &mut Context,
        line: &str,
        out: &Rc<RefCell<impl Write + 'static>>,
    ) -> Option<Value> {
        let message: Value = match serde_json::from_str(line) {
            Ok(message) => message,
            Err(e) => return Some(error(Value::Null, PARSE_ERROR, format!("parse error: {e}"))),
        };
        let Some(message) = message.as_object() else {
            return Some(error(
                Value::Null,
                INVALID_REQUEST,
                "send one JSON-RPC message per line; batches aren't supported".into(),
            ));
        };
        let method = message.get("method").and_then(Value::as_str);
        match (message.get("id"), method) {
            // Notifications (and responses, since we never send requests) get no reply.
            (None, _) => None,
            (Some(_), None) if message.contains_key("result") || message.contains_key("error") => {
                None
            }
            (Some(id), Some(method)) if !id.is_null() => {
                Some(self.handle_request(ctx, id.clone(), method, message.get("params"), out))
            }
            (Some(id), _) => Some(error(
                id.clone(),
                INVALID_REQUEST,
                "requests need a non-null id and a method".into(),
            )),
        }
    }

    fn handle_request(
        &self,
        ctx: &mut Context,
        id: Value,
        method: &str,
        params: Option<&Value>,
        out: &Rc<RefCell<impl Write + 'static>>,
    ) -> Value {
        let params = params.cloned().unwrap_or_else(|| json!({}));
        let meta = params.get("_meta");
        let era = match meta.and_then(|m| m.get(PROTOCOL_VERSION)) {
            None => Era::Legacy,
            Some(version) => {
                let version = version.as_str().unwrap_or_default();
                if !MODERN_VERSIONS.contains(&version) {
                    return json!({
                        "jsonrpc": "2.0",
                        "id": id,
                        "error": {
                            "code": UNSUPPORTED_PROTOCOL_VERSION,
                            "message": "Unsupported protocol version",
                            "data": { "supported": MODERN_VERSIONS, "requested": version },
                        },
                    });
                }
                if meta.and_then(|m| m.get(CLIENT_CAPABILITIES)).is_none() {
                    return error(
                        id,
                        INVALID_PARAMS,
                        format!("_meta must include {CLIENT_CAPABILITIES}"),
                    );
                }
                Era::Modern
            }
        };

        let result = match (method, era) {
            ("initialize", Era::Legacy) => Ok(initialize(&params)),
            ("server/discover", _) => Ok(json!({
                "supportedVersions": MODERN_VERSIONS,
                "capabilities": { "tools": {} },
                "instructions": INSTRUCTIONS,
            })),
            ("ping", _) => Ok(json!({})),
            ("tools/list", _) => Ok(json!({ "tools": self.tools })),
            ("tools/call", _) => self.call_tool(ctx, &params, out),
            (other, _) => Err((METHOD_NOT_FOUND, format!("method not found: {other}"))),
        };
        match result {
            Ok(mut result) => {
                if era == Era::Modern {
                    result["resultType"] = json!("complete");
                    let mut meta = Map::new();
                    meta.insert(SERVER_INFO.to_string(), server_info());
                    result["_meta"] = Value::Object(meta);
                }
                json!({ "jsonrpc": "2.0", "id": id, "result": result })
            }
            Err((code, message)) => error(id, code, message),
        }
    }

    fn call_tool(
        &self,
        ctx: &mut Context,
        params: &Value,
        out: &Rc<RefCell<impl Write + 'static>>,
    ) -> Result<Value, Failure> {
        let name = params
            .get("name")
            .and_then(Value::as_str)
            .ok_or((INVALID_PARAMS, "tools/call needs a tool name".to_string()))?;
        if !self.names.contains(name) {
            return Err((INVALID_PARAMS, format!("Unknown tool: {name}")));
        }
        let arguments = params
            .get("arguments")
            .cloned()
            .unwrap_or_else(|| json!({}));

        if let Some(token) = params.pointer("/_meta/progressToken").cloned() {
            let out = Rc::clone(out);
            let mut sent = 0u64;
            ctx.set_progress(move |p| {
                // Progress must increase on every notification, and stages
                // restart their own counts, so count notifications instead.
                sent += 1;
                let _ = write_message(
                    &out,
                    &json!({
                        "jsonrpc": "2.0",
                        "method": "notifications/progress",
                        "params": {
                            "progressToken": token,
                            "progress": sent,
                            "message": format!("{} {}/{}", p.stage, p.done, p.total),
                        },
                    }),
                );
            });
        }
        let outcome = tuneox_commands::run_json(ctx, name, arguments);
        ctx.set_progress(|_| {});

        // Failures inside a tool go back to the model as tool errors so it can
        // correct itself, not as protocol errors.
        let (text, is_error) = match outcome {
            Ok(output) => (strip_nulls(output).to_string(), false),
            Err(e) => (e.to_string(), true),
        };
        Ok(json!({
            "content": [{ "type": "text", "text": text }],
            "isError": is_error,
        }))
    }
}

fn initialize(params: &Value) -> Value {
    let requested = params
        .get("protocolVersion")
        .and_then(Value::as_str)
        .unwrap_or_default();
    let version = LEGACY_VERSIONS
        .iter()
        .find(|&&v| v == requested)
        .unwrap_or(&LEGACY_VERSIONS[0]);
    json!({
        "protocolVersion": version,
        "capabilities": { "tools": { "listChanged": false } },
        "serverInfo": server_info(),
        "instructions": INSTRUCTIONS,
    })
}

fn server_info() -> Value {
    json!({ "name": "tuneox", "title": "TuneOx", "version": env!("CARGO_PKG_VERSION") })
}

fn annotations(effect: Effect) -> Value {
    match effect {
        Effect::Read => json!({
            "readOnlyHint": true, "destructiveHint": false,
            "idempotentHint": true, "openWorldHint": false,
        }),
        Effect::Edit => json!({
            "readOnlyHint": false, "destructiveHint": false,
            "idempotentHint": false, "openWorldHint": false,
        }),
        Effect::Destructive | Effect::DeviceWrite => json!({
            "readOnlyHint": false, "destructiveHint": true,
            "idempotentHint": false, "openWorldHint": false,
        }),
    }
}

/// `search_tracks` → `Search tracks`.
fn title(name: &str) -> String {
    let words = name.replace('_', " ");
    let mut chars = words.chars();
    chars
        .next()
        .map(|first| first.to_uppercase().chain(chars).collect())
        .unwrap_or_default()
}

/// Drops null fields so results cost the model fewer tokens.
fn strip_nulls(value: Value) -> Value {
    match value {
        Value::Object(map) => Value::Object(
            map.into_iter()
                .filter(|(_, v)| !v.is_null())
                .map(|(k, v)| (k, strip_nulls(v)))
                .collect::<Map<_, _>>(),
        ),
        Value::Array(items) => Value::Array(items.into_iter().map(strip_nulls).collect()),
        other => other,
    }
}

fn error(id: Value, code: i64, message: String) -> Value {
    json!({ "jsonrpc": "2.0", "id": id, "error": { "code": code, "message": message } })
}

fn write_message(out: &Rc<RefCell<impl Write>>, message: &Value) -> io::Result<()> {
    let mut out = out.borrow_mut();
    serde_json::to_writer(&mut *out, message)?;
    out.write_all(b"\n")?;
    out.flush()
}

#[cfg(test)]
mod tests;
