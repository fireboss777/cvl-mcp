//! CLI check mode: `cvl-mcp file1 [file2 ...]` (the default shell behavior).
//! One `certoraCVLFormatter` run per file; diagnostics printed; exits
//! non-zero on any error severity.
use crate::formatter;
use std::path::{Path, PathBuf};

/// Exit codes: 0 = no errors, 1 = errors found, 2 = infrastructure error.
pub fn run(files: &[String]) -> u8 {
    if files.is_empty() {
        eprintln!("cvl-mcp: no file given (pass .spec/.cvl files; run with no args for MCP mode)");
        return 2;
    }
    let mut readable: Vec<PathBuf> = Vec::new();
    for f in files {
        if Path::new(f).is_file() {
            readable.push(PathBuf::from(f));
        } else {
            eprintln!("cvl-mcp: {f}: no such file");
        }
    }
    if readable.is_empty() {
        return 2;
    }

    let mut total_errs = 0usize;
    let mut total_warns = 0usize;
    let mut processed = 0usize;
    for f in &readable {
        let disp = f.display().to_string();
        match formatter::lint(&disp) {
            Ok(result) => {
                processed += 1;
                for d in &result.diagnostics {
                    if d.is_error() {
                        total_errs += 1;
                    } else {
                        total_warns += 1;
                    }
                    println!(
                        "{disp}:{} [{:7}] {}",
                        d.range.start.to_human(),
                        d.severity_name(),
                        d.message
                    );
                }
            }
            Err(e) => eprintln!("cvl-mcp: {e}"),
        }
    }
    if processed == 0 {
        return 2; // infrastructure error: formatter unavailable / files unreadable
    }
    println!(
        "cvl-mcp: {} file(s), {} error(s), {} warning(s)",
        processed, total_errs, total_warns
    );
    if total_errs > 0 {
        1
    } else {
        0
    }
}
