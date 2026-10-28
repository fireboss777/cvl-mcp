# cvl-mcp

Fast CVL2 lint built around the **`certoraCVLFormatter` shortcut** (`certora-cli`).
One small Rust binary, two entry modes, zero network:

- **CLI check** (default): `cvl-mcp file.spec [more.spec ...]`
  Lints every file, prints each diagnostic,
  and exits `0` (no errors) / `1` (errors found) / `2` (infrastructure error).
- **MCP stdio server** (no args): spawned by any MCP-compatible client
  (OpenCode, Claude Code, Codex, Cline, Cursor…). Exposes `cvl.lint(path)`
  which returns the **parser's
  diagnostics** (severity, line, character, message, code, source) — never a
  plain "valid" boolean.

The heavy lifting stays with Certora's own AST extractor (`java -jar
ASTExtraction.jar`, invoked by the Python wrapper); this crate is only a thin,
typed wrapper. The shortcut restarts its JVM on every call: ~0.7 s per lint,
no resident server and no hardcoded binary path.

## Setup

`certoraCVLFormatter` ships with [**`certora-cli`**](https://pypi.org/project/certora-cli/) —
the Certora Prover runner (Python package). It needs Java and must be on `$PATH`:

```sh
which certoraCVLFormatter     # must print a path (pip/conda/venv/etc.)
```

Missing `certora-cli`? `pip install certora-cli` (or via conda) — Java is
required at runtime.

Before pointing an MCP client at the binary, symlink it to a stable path
(`ln -sf target/release/cvl-mcp ~/.local/bin/cvl-mcp`) — `target/` is wiped by
`cargo clean`.

The formatter is run **without** `-w`: files are never modified (its formatted
stdout is discarded).

## Build & test

```sh
cargo build --release          # binary at target/release/cvl-mcp
cargo fmt                       # cargo test = unit + e2e (CLI e2e skips w/o certora-cli)

# CLI — use any .spec/.cvl file (absolute or relative path)
target/release/cvl-mcp <file.spec>
```

## Result contract

Both modes share the exact same JSON shape:

```json
{
  "uri": "file:///…/x.spec",
  "server": {"name": "certoraCVLFormatter"},
  "ok": false,
  "summary": {"error": 2, "warning": 0},
  "diagnostics": [
    {"severity": 1, "severityName": "error", "code": null,
     "range": {"start": {"line": 0, "character": 30},
               "end": {"line": 0, "character": 36}},
     "rangeHuman": {"start": "1:31", "end": "1:37"},
     "source": "certoraCVLFormatter",
     "message": "Since CVL 2, `import` statements must end with `;`"}
  ]
}
```

`ok` is false iff any diagnostic has `severity == 1`. JSON-RPC error codes:
`-32602` invalid arguments or unknown tool, `-32601` unknown method,
`-32000` infrastructure failures (formatter missing, timeout, unreadable file).

> **Note on ranges:** the formatter reports parse errors *without* positions
> (only messages like ``unexpected token near `;` ``). Ranges are best-effort —
> the blamed token is located in the source text (successive messages map to
> successive occurrences). Only parse errors are detectable: semantic issues
> (undefined symbols, …) are not reported. Severity is therefore always `error`.

## Performance

Each call restarts the formatter's JVM: budget **~0.7 s per file**, plus a
hard **30 s deadline** per call (a wedged JVM is killed instead of hanging
the MCP client).

## Layout

- `src/proto.rs`      — newline-delimited JSON framing for the MCP stdio transport
- `src/formatter.rs`  — one `certoraCVLFormatter` run + stderr parsing + best-effort ranges
- `src/check.rs`      — CLI per-file driver + diagnostic printing
- `src/mcp.rs`        — JSON-RPC 2.0 loop: initialize / tools/list / tools/call
- `src/main.rs`       — dispatch: no args ⇒ MCP, else ⇒ check
