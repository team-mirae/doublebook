use std::io::{Read, Write};
use std::path::Path;
use std::process::ExitCode;

const USAGE: &str = "usage:
  doublebook call                     read one JSON request on stdin, write the response to stdout
  doublebook migrate-terraform DIR [--sort-keys]
                                      convert a Terraform export (one <table>.json per table) to
                                      {\"books\": [{book, warnings, problems}]} on stdout;
                                      --sort-keys for a magic-company SQLite export
  doublebook version";

const TABLES: [&str; 10] = [
    "books",
    "currencies",
    "accounts",
    "clients",
    "batches",
    "entries",
    "postings",
    "entry_revisions",
    "cash_flow_pairs",
    "statement_notes",
];

fn read_export(dir: &Path, sort_keys: bool) -> Result<String, String> {
    let mut parts = Vec::new();
    for t in TABLES {
        let path = dir.join(format!("{t}.json"));
        let text = match std::fs::read_to_string(&path) {
            Ok(s) => s,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => String::new(),
            Err(e) => return Err(format!("{}: {e}", path.display())),
        };
        let text = text.trim();
        parts.push(format!("\"{t}\": {}", if text.is_empty() { "[]" } else { text }));
    }
    parts.push(format!("\"sort_keys\": {sort_keys}"));
    Ok(format!("{{\"op\": \"migrateTerraform\", \"export\": {{{}}}}}", parts.join(",\n")))
}

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let request = match args.iter().map(String::as_str).collect::<Vec<_>>().as_slice() {
        ["call"] => {
            let mut s = String::new();
            if let Err(e) = std::io::stdin().read_to_string(&mut s) {
                eprintln!("doublebook: stdin: {e}");
                return ExitCode::FAILURE;
            }
            s
        }
        ["migrate-terraform", dir] | ["migrate-terraform", dir, "--sort-keys"] => match read_export(Path::new(dir), args.len() == 3) {
            Ok(s) => s,
            Err(e) => {
                eprintln!("doublebook: {e}");
                return ExitCode::FAILURE;
            }
        },
        ["version"] => {
            println!("doublebook {}", doublebook::VERSION);
            return ExitCode::SUCCESS;
        }
        _ => {
            eprintln!("{USAGE}");
            return ExitCode::from(2);
        }
    };
    let response = doublebook::api::call(&request);
    let ok = response.starts_with("{\"ok\":true");
    let mut out = std::io::stdout().lock();
    if let Some(result) = (args[0] == "migrate-terraform" && ok)
        .then(|| serde_json::from_str::<serde_json::Value>(&response).ok())
        .flatten()
    {
        let _ = serde_json::to_writer_pretty(&mut out, &result["result"]);
    } else {
        let _ = out.write_all(response.as_bytes());
    }
    let _ = out.write_all(b"\n");
    if ok { ExitCode::SUCCESS } else { ExitCode::FAILURE }
}
