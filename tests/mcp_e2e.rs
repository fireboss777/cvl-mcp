//! End-to-end MCP handshake over the stdio transport, exactly as a client
//! sends it: newline-delimited JSON, no Content-Length headers. Guards the
//! original framing regression. Needs no external binary.

use std::io::Write;
use std::process::{Command, Stdio};

#[test]
fn handshake_and_tool_calls_over_newline_framing() {
    let mut child = Command::new(env!("CARGO_BIN_EXE_cvl-mcp"))
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("spawn cvl-mcp");

    {
        let stdin = child.stdin.as_mut().expect("piped stdin");
        writeln!(
            stdin,
            r#"{{"jsonrpc":"2.0","id":0,"method":"initialize","params":{{"protocolVersion":"2025-11-25","capabilities":{{}},"clientInfo":{{"name":"e2e","version":"0"}}}}}}"#
        )
        .unwrap();
        writeln!(
            stdin,
            r#"{{"jsonrpc":"2.0","method":"notifications/initialized"}}"#
        )
        .unwrap();
        writeln!(stdin, r#"{{"jsonrpc":"2.0","id":1,"method":"tools/list"}}"#).unwrap();
        writeln!(
            stdin,
            r#"{{"jsonrpc":"2.0","id":2,"method":"tools/call","params":{{"name":"cvl.lint","arguments":{{}}}}}}"#
        )
        .unwrap();
        writeln!(
            stdin,
            r#"{{"jsonrpc":"2.0","id":3,"method":"no/such/method"}}"#
        )
        .unwrap();
    } // stdin dropped → EOF → server must exit cleanly

    let out = child.wait_with_output().expect("wait for cvl-mcp");
    assert!(out.status.success(), "server should exit 0 on EOF");
    let text = String::from_utf8_lossy(&out.stdout);

    // initialize: echo the requested protocol version + serverInfo
    assert!(text.contains(r#""id":0"#), "initialize answered\n{text}");
    assert!(
        text.contains("2025-11-25"),
        "protocolVersion echoed\n{text}"
    );
    assert!(text.contains("serverInfo"), "serverInfo present\n{text}");

    // tools/list advertises cvl.lint
    assert!(text.contains("cvl.lint"), "tool listed\n{text}");

    // missing path → -32602 with a human message
    assert!(text.contains(r#""id":2"#), "tools/call answered\n{text}");
    assert!(text.contains("-32602"), "invalid params code\n{text}");
    assert!(
        text.contains("'path' argument is required"),
        "human message\n{text}"
    );

    // unknown method → -32601
    assert!(
        text.contains(r#""id":3"#),
        "unknown method answered\n{text}"
    );
    assert!(text.contains("-32601"), "method-not-found code\n{text}");

    // responses are newline-delimited: no Content-Length headers anywhere
    assert!(!text.contains("Content-Length"), "no LSP framing leaked");
}
