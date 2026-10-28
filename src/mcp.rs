//! MCP server over stdio. Handles: initialize, tools/list, tools/call (cvl.lint),
//! ping, notifications/initialized, shutdown, exit. Each lint shells out to
//! `certoraCVLFormatter FILE` once, so host startup cost stays ~0.
use crate::formatter;
use crate::proto::{self, write_line_msg as write_msg};
use serde_json::{json, Value};
use std::error::Error;
use std::io::BufWriter;

/// Successful JSON-RPC result for request `id`.
fn send_result<W: std::io::Write>(
    out: &mut W,
    id: Option<Value>,
    result: Value,
) -> Result<(), Box<dyn Error>> {
    let mut msg = json!({"jsonrpc": "2.0"});
    if let Some(id) = id {
        msg["id"] = id;
    }
    msg["result"] = result;
    Ok(write_msg(out, &msg)?)
}

/// JSON-RPC error reply (-32602 invalid params) for a bad call.
fn send_tool_error<W: std::io::Write>(
    out: &mut W,
    id: Option<Value>,
    message: String,
) -> Result<(), Box<dyn Error>> {
    jsonrpc_error(out, id, -32602, message)
}

/// JSON-RPC error reply (-32000 server error) for infrastructure failures
/// (formatter missing, timeout, unreadable file…).
fn send_server_error<W: std::io::Write>(
    out: &mut W,
    id: Option<Value>,
    message: String,
) -> Result<(), Box<dyn Error>> {
    jsonrpc_error(out, id, -32000, message)
}

fn jsonrpc_error<W: std::io::Write>(
    out: &mut W,
    id: Option<Value>,
    code: i64,
    message: String,
) -> Result<(), Box<dyn Error>> {
    let mut msg = json!({
        "jsonrpc": "2.0",
        "error": {"code": code, "message": message},
    });
    if let Some(id) = id {
        msg["id"] = id;
    }
    write_msg(out, &msg).map_err(Into::into)
}

/// Human summary for `content[].text` (the JSON shape lives in
/// `structuredContent`): one line per diagnostic, or an OK line.
fn human_report(result: &formatter::LintResult) -> String {
    let mut lines = Vec::new();
    for d in &result.diagnostics {
        lines.push(format!(
            "{}:{} [{}] {}",
            result.uri,
            d.range.start.to_human(),
            d.severity_name(),
            d.message
        ));
    }
    let (errs, warns) = result.counts();
    if lines.is_empty() {
        lines.push(format!("{}: no diagnostics", result.uri));
    }
    lines.push(format!("summary: {errs} error(s), {warns} warning(s)"));
    lines.join("\n")
}

fn handle_tools_call<W: std::io::Write>(
    out: &mut W,
    id: Option<Value>,
    msg: &Value,
) -> Result<(), Box<dyn Error>> {
    let name = msg["params"]["name"].as_str().unwrap_or("");
    match name {
        "cvl.lint" => {
            let path = msg["params"]["arguments"]["path"].as_str().unwrap_or("");
            if path.is_empty() {
                return send_tool_error(out, id, "'path' argument is required".to_string());
            }
            match formatter::lint(path) {
                Ok(result) => {
                    let body = result.to_json();
                    let bad = !result.ok();
                    send_result(
                        out,
                        id,
                        json!({
                            "content": [{"type": "text", "text": human_report(&result)}],
                            "structuredContent": body,
                            "isError": bad,
                        }),
                    )
                }
                Err(e) => send_server_error(out, id, e),
            }
        }
        other => send_tool_error(out, id, format!("unknown tool: {other}")),
    }
}

pub fn run() -> Result<(), Box<dyn Error>> {
    let debug = std::env::var("CVLL_DEBUG")
        .map(|v| v != "0" && v != "false")
        .unwrap_or(false);
    let stdin = std::io::stdin();
    let mut reader = std::io::BufReader::new(stdin.lock());
    let stdout = std::io::stdout();
    let mut out = BufWriter::new(stdout.lock());

    let mut shutdown_done = false;

    while let Some(msg) = proto::read_line_msg(&mut reader) {
        if debug {
            eprintln!("CVLL_DEBUG recv: {msg}");
        }
        // Distinguish notification (no "id" member) from request.
        let has_id = msg.get("id").is_some();
        let id = if has_id {
            Some(msg["id"].clone())
        } else {
            None
        };
        let method = msg
            .get("method")
            .and_then(Value::as_str)
            .map(|s| s.to_string());

        match method.as_deref() {
            Some("initialize") => {
                // Echo the client's requested protocol version when provided;
                // strict SDK clients reject versions they didn't offer.
                let protocol_version = msg["params"]["protocolVersion"]
                    .as_str()
                    .unwrap_or("2025-03-26")
                    .to_string();
                send_result(
                    &mut out,
                    id,
                    json!({
                        "protocolVersion": protocol_version,
                        "capabilities": {"tools": {}},
                        "serverInfo": {
                            "name": "cvl-mcp",
                            "version": env!("CARGO_PKG_VERSION"),
                        },
                    }),
                )?;
                // Proactively send the initialized notification — spec-allowed
                // and some clients gate connect on having seen either side send it.
                write_msg(
                    &mut out,
                    &json!({
                        "jsonrpc": "2.0",
                        "method": "notifications/initialized",
                        "params": {},
                    }),
                )?;
            }
            Some("initialized") | Some("notifications/initialized") => { /* nothing to answer */ }
            Some("ping") => {
                send_result(&mut out, id, json!({}))?;
            }
            Some("tools/list") => {
                send_result(
                    &mut out,
                    id,
                    json!({"tools": [{
                        "name": "cvl.lint",
                        "title": "CVL lint",
                        "description": "Parse one .spec/.cvl file with Certora's certoraCVLFormatter (same AST extractor as the Certora Prover). Returns the parser's diagnostics (severity, line, message) — not just a validity bit. Positions are approximate: the formatter reports messages only.",
                        "inputSchema": {
                            "type": "object",
                            "properties": {
                                "path": {"type": "string", "description": "Path to a .spec or .cvl file"}
                            },
                            "required": ["path"],
                        },
                    }]}),
                )?;
            }
            Some("prompts/list") => {
                send_result(&mut out, id, json!({"prompts": []}))?;
            }
            Some("resources/list") => {
                send_result(&mut out, id, json!({"resources": []}))?;
            }
            Some("resources/templates/list") => {
                send_result(&mut out, id, json!({"resourceTemplates": []}))?;
            }
            Some("logging/setLevel") => {
                send_result(&mut out, id, json!({}))?;
            }
            Some("tools/call") => {
                handle_tools_call(&mut out, id, &msg)?;
            }
            Some("shutdown") => {
                send_result(&mut out, id, json!(null))?;
                shutdown_done = true;
            }
            Some("exit") => break,
            _ => {
                // Unknown method: JSON-RPC standard error, but only when a request id exists.
                if let Some(i) = id {
                    let m = json!({
                        "jsonrpc": "2.0",
                        "id": i,
                        "error": {"code": -32601, "message": "method not found".to_string()},
                    });
                    write_msg(&mut out, &m)?;
                }
            }
        }

        if shutdown_done {
            // After a proper shutdown we expect `exit` next; tolerate its absence.
            break;
        }
    }

    Ok(())
}
