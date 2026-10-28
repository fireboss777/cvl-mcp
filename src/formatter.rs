//! One-shot CVL lint via `certoraCVLFormatter` (certora-cli): runs
//! `java -jar ASTExtraction.jar` per call — no resident server, command
//! resolved from `$PATH`. Exit 0 = file parses; otherwise the stderr
//! `got errors:` lines become error diagnostics. Ranges are best-effort: the
//! formatter reports no positions, so the blamed token is located in the text.
use serde_json::{json, Value};
use std::io::Read;
use std::path::Path;

/// Position inside a source file (0-based).
#[derive(Clone, Debug, Default, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct Position {
    pub line: u64,
    pub character: u64,
}

impl Position {
    pub fn to_human(&self) -> String {
        format!("{}:{}", self.line + 1, self.character + 1)
    }
}

/// Range of a diagnostic (start..end, both inclusive positions).
#[derive(Clone, Debug, Default, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct Range {
    pub start: Position,
    pub end: Position,
}

/// One diagnostic; serialized manually to the output schema via `to_json`.
#[derive(Clone, Debug)]
pub struct Diagnostic {
    pub range: Range,
    pub severity: Option<u8>,
    pub code: Option<Value>,
    pub source: Option<String>,
    pub message: String,
}

impl Diagnostic {
    pub fn severity_name(&self) -> &'static str {
        match self.severity {
            None | Some(1) => "error",
            Some(2) => "warning",
            Some(3) => "info",
            _ => "hint",
        }
    }

    pub fn is_error(&self) -> bool {
        self.severity == Some(1)
    }

    fn to_json(&self) -> Value {
        json!({
            "severity": self.severity.unwrap_or(1),
            "severityName": self.severity_name(),
            "code": self.code,
            "source": self.source,
            "range": {
                "start": self.range.start,
                "end": self.range.end,
            },
            "rangeHuman": {
                "start": self.range.start.to_human(),
                "end": self.range.end.to_human(),
            },
            "message": self.message,
        })
    }
}

/// Shared result structure: diagnostics + a small human counter, used by CLI and MCP.
#[derive(Clone)]
pub struct LintResult {
    pub uri: String,
    pub server: Value,
    pub diagnostics: Vec<Diagnostic>,
}

impl LintResult {
    pub fn counts(&self) -> (usize, usize) {
        let mut errs = 0;
        let mut warns = 0;
        for d in &self.diagnostics {
            if d.is_error() {
                errs += 1;
            } else {
                warns += 1;
            }
        }
        (errs, warns)
    }

    pub fn ok(&self) -> bool {
        self.counts().0 == 0
    }

    /// Structured JSON, identical for both MCP and CLI outputs.
    pub fn to_json(&self) -> Value {
        let (errs, warns) = self.counts();
        let diags: Vec<Value> = self.diagnostics.iter().map(Diagnostic::to_json).collect();
        json!({
            "uri": self.uri,
            "server": self.server,
            "ok": self.ok(),
            "summary": { "error": errs, "warning": warns },
            "diagnostics": diags,
        })
    }
}

/// The command run per lint, from `$PATH` — without `-w`: files are never
/// modified.
const FORMATTER_CMD: &str = "certoraCVLFormatter";

/// Hard deadline per lint call: a wedged JVM is killed, never left hanging.
const LINT_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(30);

fn server_info() -> Value {
    json!({ "name": "certoraCVLFormatter" })
}

/// Lint one file via `certoraCVLFormatter`. `Ok` with empty diagnostics =
/// file parses; `Err` = infrastructure problem (command missing, …).
///
/// The formatter is watched by a polling deadline: a wedged JVM gets killed
/// after [`LINT_TIMEOUT`] instead of hanging the MCP client forever.
pub fn lint(path: &str) -> Result<LintResult, String> {
    let abs = std::path::absolute(path).map_err(|e| format!("cannot resolve '{path}': {e}"))?;
    if !abs.is_file() {
        return Err(format!("'{path}' is not a file"));
    }
    let text = std::fs::read_to_string(&abs).map_err(|e| format!("{path}: {e}"))?;
    let out = run_formatter(&abs)?;

    let uri = format!("file://{}", abs.display());
    if out.status.success() {
        return Ok(LintResult {
            uri,
            server: server_info(),
            diagnostics: Vec::new(),
        });
    }

    let stderr = String::from_utf8_lossy(&out.stderr);
    let messages = parse_error_messages(&stderr);
    if messages.is_empty() {
        return Err(format!(
            "{FORMATTER_CMD} exited with {}: {}",
            out.status,
            summarize_failure(&stderr)
        ));
    }
    Ok(LintResult {
        uri,
        server: server_info(),
        diagnostics: build_diagnostics(&messages, &text),
    })
}

/// Run `certoraCVLFormatter FILE` capturing output, killing it if it exceeds
/// [`LINT_TIMEOUT`] (stdout/stderr are drained concurrently by reader threads,
/// so large formatted outputs can't deadlock the pipes).
fn run_formatter(abs: &Path) -> Result<std::process::Output, String> {
    let mut child = std::process::Command::new(FORMATTER_CMD)
        .arg(abs)
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .spawn()
        .map_err(|e| format!("failed to run '{FORMATTER_CMD}': {e} (is certora-cli on $PATH?)"))?;

    let stdout = child.stdout.take().expect("piped stdout");
    let stderr = child.stderr.take().expect("piped stderr");
    let drain = |mut r: Box<dyn std::io::Read + Send>| {
        std::thread::spawn(move || {
            let mut buf = Vec::new();
            let _ = r.read_to_end(&mut buf);
            buf
        })
    };
    let out_t = drain(Box::new(stdout));
    let err_t = drain(Box::new(stderr));

    let deadline = std::time::Instant::now() + LINT_TIMEOUT;
    let status = loop {
        match child.try_wait() {
            Ok(Some(status)) => break status,
            Ok(None) if std::time::Instant::now() > deadline => {
                let _ = child.kill();
                let _ = child.wait();
                return Err(format!(
                    "{FORMATTER_CMD} timed out after {}s on {}",
                    LINT_TIMEOUT.as_secs(),
                    abs.display()
                ));
            }
            Ok(None) => std::thread::sleep(std::time::Duration::from_millis(50)),
            Err(e) => {
                let _ = child.kill();
                let _ = child.wait();
                return Err(format!("{FORMATTER_CMD}: wait failed: {e}"));
            }
        }
    };

    Ok(std::process::Output {
        status,
        stdout: out_t.join().unwrap_or_default(),
        stderr: err_t.join().unwrap_or_default(),
    })
}

/// Messages after the `got errors:` marker on stderr.
fn parse_error_messages(stderr: &str) -> Vec<String> {
    let mut msgs = Vec::new();
    let mut collecting = false;
    for line in stderr.lines() {
        if !collecting {
            if let Some(pos) = line.find("got errors:") {
                collecting = true;
                let rest = line[pos + "got errors:".len()..].trim();
                if !rest.is_empty() {
                    msgs.push(rest.to_string());
                }
            }
            continue;
        }
        let t = line.trim();
        if !t.is_empty() {
            msgs.push(t.to_string());
        }
    }
    msgs
}

/// Single-line fallback message when stderr has no `got errors:` block.
fn summarize_failure(stderr: &str) -> String {
    let lines: Vec<&str> = stderr.lines().filter(|l| !l.trim().is_empty()).collect();
    let line = lines
        .iter()
        .rev()
        .find(|l| l.contains("CertoraUserInputError:"))
        .or_else(|| {
            lines
                .iter()
                .rev()
                .find(|l| l.contains("certoraCVLFormatter:"))
        })
        .copied()
        .or_else(|| lines.last().copied())
        .unwrap_or("no output on stderr");
    line.split("CertoraUserInputError:")
        .nth(1)
        .map(str::trim)
        .unwrap_or(line)
        .to_string()
}

fn build_diagnostics(messages: &[String], text: &str) -> Vec<Diagnostic> {
    let mut cursor = 0usize;
    messages
        .iter()
        .map(|m| {
            let range = match token_from_message(m) {
                Some(tok) => locate(text, &tok, &mut cursor),
                None => {
                    let p = position_at(text, cursor);
                    Range {
                        start: p.clone(),
                        end: p,
                    }
                }
            };
            Diagnostic {
                range,
                severity: Some(1),
                code: None,
                source: Some("certoraCVLFormatter".to_string()),
                message: m.clone(),
            }
        })
        .collect()
}

/// Token the message blames: prefer `` near `X` ``, else the first backticked span.
fn token_from_message(msg: &str) -> Option<String> {
    const NEAR: &str = "near `";
    if let Some(i) = msg.find(NEAR) {
        let start = i + NEAR.len();
        if let Some(end) = msg[start..].find('`') {
            let tok = &msg[start..start + end];
            if !tok.is_empty() {
                return Some(tok.to_string());
            }
        }
    }
    let start = msg.find('`')? + 1;
    let end = msg[start..].find('`')?;
    let tok = &msg[start..start + end];
    if tok.is_empty() {
        None
    } else {
        Some(tok.to_string())
    }
}

/// Range of the blamed token: first occurrence at/after `cursor`
/// (wraps once to the start); falls back to the cursor position.
fn locate(text: &str, token: &str, cursor: &mut usize) -> Range {
    let mut idx = text
        .get(*cursor..)
        .and_then(|s| s.find(token))
        .map(|i| *cursor + i);
    if idx.is_none() && *cursor > 0 {
        idx = text.find(token);
    }
    match idx {
        Some(i) => {
            let end_byte = std::cmp::min(i + token.len(), text.len());
            *cursor = end_byte;
            Range {
                start: position_at(text, i),
                end: position_at(text, end_byte),
            }
        }
        None => {
            let p = position_at(text, *cursor);
            Range {
                start: p.clone(),
                end: p,
            }
        }
    }
}

/// Line/character (0-based) of a byte offset, rounded to a char boundary.
fn position_at(text: &str, byte: usize) -> Position {
    let mut b = std::cmp::min(byte, text.len());
    while b > 0 && !text.is_char_boundary(b) {
        b -= 1;
    }
    let before = &text[..b];
    let line = before.matches('\n').count() as u64;
    let character = match before.rfind('\n') {
        Some(p) => before[p + 1..].chars().count() as u64,
        None => before.chars().count() as u64,
    };
    Position { line, character }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_errors_collects_block_after_marker() {
        let stderr = "\
Traceback (most recent call last):
  File \".../certoraCVLFormatter\", line 8, in <module>
Shared.certoraUtils.CertoraUserInputError: Error running formatter on /tmp/x.spec: file could not be parsed. got errors:
Syntax error: unexpected token near `method`
Since CVL 2, `import` statements must end with `;`
";
        let msgs = parse_error_messages(stderr);
        assert_eq!(msgs.len(), 2);
        assert_eq!(msgs[0], "Syntax error: unexpected token near `method`");
        assert_eq!(
            msgs[1],
            "Since CVL 2, `import` statements must end with `;`"
        );
    }

    #[test]
    fn parse_errors_same_line_text_after_marker() {
        let stderr = "Shared.certoraUtils.CertoraUserInputError: error. got errors: Syntax error: unexpected token `x`\n";
        assert_eq!(
            parse_error_messages(stderr),
            vec!["Syntax error: unexpected token `x`"]
        );
    }

    #[test]
    fn parse_errors_without_marker_is_empty() {
        let stderr = "Shared.certoraUtils.CertoraUserInputError: Java or higher is required\n";
        assert_eq!(parse_error_messages(stderr), Vec::<String>::new());
        assert_eq!(parse_error_messages(""), Vec::<String>::new());
    }

    #[test]
    fn summarize_prefers_certora_error_message() {
        let stderr = "Traceback...\nShared.certoraUtils.CertoraUserInputError: Error running formatter on /x.spec: file could not be parsed.\n";
        assert_eq!(
            summarize_failure(stderr),
            "Error running formatter on /x.spec: file could not be parsed."
        );
    }

    #[test]
    fn summarize_fallbacks() {
        assert_eq!(
            summarize_failure("certoraCVLFormatter: error: argument spec_file: no such file\n"),
            "certoraCVLFormatter: error: argument spec_file: no such file"
        );
        assert_eq!(
            summarize_failure("java stack trace\njava.lang.OutOfMemoryError\n"),
            "java.lang.OutOfMemoryError"
        );
        assert_eq!(summarize_failure(""), "no output on stderr");
    }

    #[test]
    fn token_prefers_near_clause() {
        assert_eq!(
            token_from_message("unexpected token near `method` right after"),
            Some("method".to_string())
        );
    }

    #[test]
    fn token_falls_back_to_first_backticked_span() {
        assert_eq!(
            token_from_message("Since CVL 2, `import` statements must end with `;`"),
            Some("import".to_string())
        );
    }

    #[test]
    fn token_none_without_backticks() {
        assert_eq!(token_from_message("plain message"), None);
        assert_eq!(token_from_message("near `` and `bar`"), None);
    }

    #[test]
    fn locate_progresses_cursor() {
        let text = "aa;bb;cc";
        let mut cursor = 0;
        let r1 = locate(text, ";", &mut cursor);
        assert_eq!((r1.start.line, r1.start.character), (0, 2));
        assert_eq!((r1.end.line, r1.end.character), (0, 3));
        assert_eq!(cursor, 3);
        let r2 = locate(text, ";", &mut cursor);
        assert_eq!((r2.start.line, r2.start.character), (0, 5));
        assert_eq!(cursor, 6);
    }

    #[test]
    fn locate_wraps_once_to_start() {
        let text = "ab;c;d";
        let mut cursor = 0;
        let r1 = locate(text, "c", &mut cursor);
        assert_eq!((r1.start.line, r1.start.character), (0, 3));
        assert_eq!(cursor, 4);
        // nothing after cursor → wrap to the first occurrence
        let r2 = locate(text, "c", &mut cursor);
        assert_eq!((r2.start.line, r2.start.character), (0, 3));
    }

    #[test]
    fn locate_falls_back_to_cursor_position() {
        let text = "abc;def";
        let mut cursor = 4;
        let r = locate(text, "zzz", &mut cursor);
        assert_eq!((r.start.line, r.start.character), (0, 4));
        assert_eq!(
            (r.start.line, r.start.character),
            (r.end.line, r.end.character)
        );
    }

    #[test]
    fn locate_across_lines() {
        let text = "def;\ngh;ij";
        let mut cursor = 0;
        let r1 = locate(text, ";", &mut cursor);
        assert_eq!((r1.start.line, r1.start.character), (0, 3));
        assert_eq!((r1.end.line, r1.end.character), (0, 4));
        let r2 = locate(text, ";", &mut cursor);
        assert_eq!((r2.start.line, r2.start.character), (1, 2));
        assert_eq!((r2.end.line, r2.end.character), (1, 3));
    }

    #[test]
    fn position_counts_lines_and_chars() {
        assert_eq!(
            position_at("abc", 1),
            Position {
                line: 0,
                character: 1
            }
        );
        // byte 4 is the 'd': the '\n' byte before it closes line 0
        assert_eq!(
            position_at("abc\ndef", 4),
            Position {
                line: 1,
                character: 0
            }
        );
    }

    #[test]
    fn position_counts_chars_not_bytes() {
        // 'é' is 2 bytes: byte offset 3 is the end, but only 2 chars were seen
        assert_eq!(
            position_at("éa", 3),
            Position {
                line: 0,
                character: 2
            }
        );
    }

    #[test]
    fn position_clamps_to_char_boundary() {
        // byte 1 is inside the 2-byte 'é' → clamps back to the token start
        assert_eq!(
            position_at("éa", 1),
            Position {
                line: 0,
                character: 0
            }
        );
        assert_eq!(
            position_at("ab", 99),
            Position {
                line: 0,
                character: 2
            }
        );
    }
}
