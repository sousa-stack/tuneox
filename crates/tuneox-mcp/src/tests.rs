use serde_json::{Value, json};
use tuneox_commands::Context;
use tuneox_core::{MusicalKey, TrackInfo};
use tuneox_library::Library;

use crate::serve;

fn context() -> Context {
    let mut library = Library::open_in_memory().unwrap();
    let import = library.import("test", "lib").unwrap();
    for (id, title, bpm, key) in [("1", "Cinema", 125.0, "11A"), ("2", "Cheque", 122.0, "4A")] {
        import
            .upsert_track(
                id,
                &TrackInfo {
                    path: format!("/music/{title}.aiff"),
                    title: title.to_string(),
                    bpm: Some(bpm),
                    key: MusicalKey::parse(key),
                    ..TrackInfo::default()
                },
                None,
            )
            .unwrap();
    }
    import.commit().unwrap();
    Context::new(library, None)
}

/// Sends each message as one line and returns every line the server wrote.
fn exchange(messages: &[Value]) -> Vec<Value> {
    let input: String = messages.iter().map(|m| format!("{m}\n")).collect();
    let output = serve(&mut context(), input.as_bytes(), Vec::new()).unwrap();
    String::from_utf8(output)
        .unwrap()
        .lines()
        .map(|line| serde_json::from_str(line).unwrap())
        .collect()
}

fn request(id: u64, method: &str, params: Value) -> Value {
    json!({ "jsonrpc": "2.0", "id": id, "method": method, "params": params })
}

fn modern(params: Value) -> Value {
    let mut params = params;
    params["_meta"] = json!({
        "io.modelcontextprotocol/protocolVersion": "2026-07-28",
        "io.modelcontextprotocol/clientCapabilities": {},
    });
    params
}

fn tool_text(response: &Value) -> Value {
    let text = response["result"]["content"][0]["text"].as_str().unwrap();
    serde_json::from_str(text).unwrap_or_else(|_| Value::String(text.to_string()))
}

#[test]
fn legacy_handshake_and_tools() {
    let replies = exchange(&[
        request(
            1,
            "initialize",
            json!({ "protocolVersion": "2025-06-18", "capabilities": {} }),
        ),
        json!({ "jsonrpc": "2.0", "method": "notifications/initialized" }),
        request(2, "tools/list", json!({})),
        request(
            3,
            "tools/call",
            json!({ "name": "search_tracks", "arguments": { "text": "cinema" } }),
        ),
        request(4, "ping", json!({})),
    ]);
    assert_eq!(replies.len(), 4, "notifications get no reply");

    let init = &replies[0]["result"];
    assert_eq!(init["protocolVersion"], "2025-06-18");
    assert_eq!(init["serverInfo"]["name"], "tuneox");
    assert!(
        init.get("resultType").is_none(),
        "legacy results have no resultType"
    );

    let tools = replies[1]["result"]["tools"].as_array().unwrap();
    let search = tools.iter().find(|t| t["name"] == "search_tracks").unwrap();
    assert_eq!(search["title"], "Search tracks");
    assert_eq!(search["inputSchema"]["type"], "object");
    assert_eq!(search["annotations"]["readOnlyHint"], true);
    let import = tools
        .iter()
        .find(|t| t["name"] == "import_rekordbox")
        .unwrap();
    assert_eq!(import["annotations"]["readOnlyHint"], false);

    assert_eq!(replies[2]["result"]["isError"], false);
    let found = tool_text(&replies[2]);
    assert_eq!(found["tracks"][0]["title"], "Cinema");
    assert_eq!(found["tracks"][0]["key"], "F#m");
    assert!(
        found["tracks"][0].get("artist").is_none(),
        "null fields are dropped"
    );

    assert_eq!(replies[3]["result"], json!({}));
}

#[test]
fn unknown_legacy_version_gets_the_newest_legacy_one() {
    let replies = exchange(&[request(
        1,
        "initialize",
        json!({ "protocolVersion": "1999-01-01" }),
    )]);
    assert_eq!(replies[0]["result"]["protocolVersion"], "2025-11-25");
}

#[test]
fn modern_requests_are_stateless() {
    let replies = exchange(&[
        request(1, "server/discover", modern(json!({}))),
        request(
            2,
            "tools/call",
            modern(json!({ "name": "library_summary", "arguments": {} })),
        ),
    ]);
    let discover = &replies[0]["result"];
    assert_eq!(discover["resultType"], "complete");
    assert_eq!(discover["supportedVersions"], json!(["2026-07-28"]));
    assert_eq!(discover["capabilities"]["tools"], json!({}));
    assert_eq!(
        discover["_meta"]["io.modelcontextprotocol/serverInfo"]["name"],
        "tuneox"
    );

    let call = &replies[1]["result"];
    assert_eq!(call["resultType"], "complete");
    assert_eq!(tool_text(&replies[1])["tracks"], 2);
}

#[test]
fn modern_version_and_meta_are_checked() {
    let mut old = modern(json!({}));
    old["_meta"]["io.modelcontextprotocol/protocolVersion"] = json!("1900-01-01");
    let replies = exchange(&[
        request(1, "tools/list", old),
        request(
            2,
            "tools/list",
            json!({ "_meta": { "io.modelcontextprotocol/protocolVersion": "2026-07-28" } }),
        ),
        request(
            3,
            "initialize",
            modern(json!({ "protocolVersion": "2025-11-25" })),
        ),
    ]);
    assert_eq!(replies[0]["error"]["code"], -32022);
    assert_eq!(replies[0]["error"]["data"]["requested"], "1900-01-01");
    assert_eq!(
        replies[0]["error"]["data"]["supported"],
        json!(["2026-07-28"])
    );
    assert_eq!(
        replies[1]["error"]["code"], -32602,
        "clientCapabilities is required"
    );
    assert_eq!(
        replies[2]["error"]["code"], -32601,
        "initialize isn't a modern method"
    );
}

#[test]
fn errors() {
    let replies = exchange(&[
        request(
            1,
            "tools/call",
            json!({ "name": "get_track", "arguments": { "id": 999 } }),
        ),
        request(
            2,
            "tools/call",
            json!({ "name": "get_track", "arguments": { "id": "x" } }),
        ),
        request(3, "tools/call", json!({ "name": "launch_rockets" })),
        request(4, "resources/list", json!({})),
    ]);
    // Tool failures are results the model can act on.
    assert_eq!(replies[0]["result"]["isError"], true);
    assert!(
        tool_text(&replies[0])
            .as_str()
            .unwrap()
            .contains("track 999")
    );
    assert_eq!(replies[1]["result"]["isError"], true);
    // Protocol problems are JSON-RPC errors.
    assert_eq!(replies[2]["error"]["code"], -32602);
    assert_eq!(replies[3]["error"]["code"], -32601);
}

#[test]
fn malformed_input() {
    let input = "not json\n[1, 2]\n{\"jsonrpc\":\"2.0\",\"id\":null,\"method\":\"ping\"}\n\n";
    let output = serve(&mut context(), input.as_bytes(), Vec::new()).unwrap();
    let replies: Vec<Value> = String::from_utf8(output)
        .unwrap()
        .lines()
        .map(|line| serde_json::from_str(line).unwrap())
        .collect();
    let codes: Vec<i64> = replies
        .iter()
        .map(|r| r["error"]["code"].as_i64().unwrap())
        .collect();
    assert_eq!(codes, [-32700, -32600, -32600]);
}
