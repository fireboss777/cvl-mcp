//! CLI end-to-end against the real `certoraCVLFormatter`. Skipped (passes
//! trivially) when the command is not on PATH, so CI without certora-cli
//! stays green.

use std::process::Command;

fn formatter_on_path() -> bool {
    std::env::var_os("PATH")
        .map(|p| std::env::split_paths(&p).any(|d| d.join("certoraCVLFormatter").is_file()))
        .unwrap_or(false)
}

#[test]
fn cli_exit_codes_and_diagnostics() {
    if !formatter_on_path() {
        eprintln!("skipping: certoraCVLFormatter not on PATH");
        return;
    }
    let dir = std::env::temp_dir().join(format!("cvlmcp-e2e-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let bad = dir.join("bad.spec");
    std::fs::write(
        &bad,
        "rule bad(uint x) {\n  require x > 0;\n  assert x @;\n}\n",
    )
    .unwrap();
    let good = dir.join("good.spec");
    std::fs::write(
        &good,
        "rule r(uint x) {\n  require x > 0;\n  assert x + 1 > x;\n}\n",
    )
    .unwrap();

    let bin = env!("CARGO_BIN_EXE_cvl-mcp");

    // parse error → diagnostics printed, exit 1
    let out = Command::new(bin).arg(&bad).output().unwrap();
    assert_eq!(out.status.code(), Some(1), "bad spec should exit 1");
    let text = String::from_utf8_lossy(&out.stdout);
    assert!(text.contains("[error"), "diagnostic printed\n{text}");
    assert!(text.contains("1 error(s)"), "summary printed\n{text}");

    // clean parse → exit 0
    let out = Command::new(bin).arg(&good).output().unwrap();
    assert_eq!(out.status.code(), Some(0), "good spec should exit 0");

    // missing file → exit 2
    let out = Command::new(bin)
        .arg(dir.join("nope.spec"))
        .output()
        .unwrap();
    assert_eq!(out.status.code(), Some(2), "missing file should exit 2");

    std::fs::remove_dir_all(&dir).ok();
}
