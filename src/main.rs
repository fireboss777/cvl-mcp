mod check;
mod formatter;
mod mcp;
mod proto;

use std::process::ExitCode;

const HELP: &str = "\
cvl-mcp — fast CVL2 lint via certoraCVLFormatter

USAGE:
    cvl-mcp [FILE ...]    lint .spec/.cvl file(s) and print server diagnostics
    cvl-mcp               start in MCP stdio mode (launched by MCP clients)

EXIT CODES (check mode): 0 = no errors · 1 = errors found · 2 = infrastructure error
";

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();

    if args.iter().any(|a| a == "--help" || a == "-h") {
        print!("{HELP}");
        return ExitCode::SUCCESS;
    }
    if args.iter().any(|a| a == "--version" || a == "-V") {
        println!("cvl-mcp {}", env!("CARGO_PKG_VERSION"));
        return ExitCode::SUCCESS;
    }

    for a in &args {
        if a.starts_with('-') {
            eprintln!("cvl-mcp: unknown option '{a}' — MCP mode is implicit (no args)");
            std::process::exit(2);
        }
    }

    if args.is_empty() {
        // MCP stdio mode — launched by MCP-compatible clients.
        match mcp::run() {
            Ok(()) => ExitCode::SUCCESS,
            Err(e) => {
                eprintln!("cvl-mcp: {e}");
                ExitCode::from(2)
            }
        }
    } else {
        // Default behavior: check the given files.
        ExitCode::from(check::run(&args))
    }
}
