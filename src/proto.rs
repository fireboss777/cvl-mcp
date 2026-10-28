//! Framing for the MCP stdio transport: newline-delimited JSON-RPC messages
//! (one message per line, no embedded newlines — per the MCP spec).
use serde_json::Value;
use std::io::{BufRead, Write};

/// Write one MCP message: newline-delimited JSON, no headers.
pub fn write_line_msg<W: Write>(out: &mut W, msg: &Value) -> std::io::Result<()> {
    let body = serde_json::to_string(msg).expect("serialize msg");
    writeln!(out, "{body}")?;
    out.flush()
}

/// Read one newline-delimited MCP message from a `BufRead`; `None` on EOF or
/// malformed line. Blank lines are skipped per the MCP stdio transport spec.
pub fn read_line_msg<R: BufRead>(rdr: &mut R) -> Option<Value> {
    let mut line = String::new();
    loop {
        line.clear();
        match rdr.read_line(&mut line) {
            Ok(0) => return None, // EOF
            Ok(_) => {
                let trimmed = line.trim();
                if trimmed.is_empty() {
                    continue;
                }
                match serde_json::from_str(trimmed) {
                    Ok(v) => return Some(v),
                    Err(_) => continue, // tolerate stray non-JSON output on stdio
                }
            }
            Err(_) => return None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    use std::io::Cursor;

    #[test]
    fn write_then_read_roundtrips() {
        let input = json!({"jsonrpc": "2.0", "id": 1});
        let mut out = Vec::new();
        write_line_msg(&mut out, &input).unwrap();
        assert_eq!(*out.last().unwrap(), b'\n'); // newline-delimited, one line
        let buf = Cursor::new(out.as_slice());
        let mut reader = std::io::BufReader::new(buf);
        assert_eq!(read_line_msg(&mut reader).unwrap(), input);
    }

    #[test]
    fn skips_blank_lines_and_non_json_noise() {
        let input = format!(
            "{}\n{}\n{}\n",
            "", "garbage line on stdout", r#"{"a":true}"#
        );
        let buf = Cursor::new(input.as_bytes());
        let mut reader = std::io::BufReader::new(buf);
        assert_eq!(read_line_msg(&mut reader), Some(json!({"a": true})));
        assert_eq!(read_line_msg(&mut reader), None);
    }

    #[test]
    fn messages_arrive_in_order_then_eof() {
        let input = format!(
            "{}\n{}",
            r#"{"jsonrpc":"2.0","id":0,"method":"initialize"}"#,
            r#"{"jsonrpc":"2.0","id":1,"method":"ping"}"#
        );
        let buf = Cursor::new(input.as_bytes());
        let mut reader = std::io::BufReader::new(buf);
        assert_eq!(
            read_line_msg(&mut reader),
            Some(json!({"jsonrpc":"2.0","id":0,"method":"initialize"}))
        );
        assert_eq!(
            read_line_msg(&mut reader),
            Some(json!({"jsonrpc":"2.0","id":1,"method":"ping"}))
        );
        // missing trailing newline still parses, next read hits EOF
        let input2 = r#"{"a":1}"#;
        let buf2 = Cursor::new(input2.as_bytes());
        let mut reader2 = std::io::BufReader::new(buf2);
        assert_eq!(read_line_msg(&mut reader2), Some(json!({"a": 1})));
        assert_eq!(read_line_msg(&mut reader2), None);
    }

    #[test]
    fn eof_returns_none() {
        let empty: &[u8] = &[];
        let buf = Cursor::new(empty);
        let mut reader = std::io::BufReader::new(buf);
        assert_eq!(read_line_msg(&mut reader), None);
    }
}
