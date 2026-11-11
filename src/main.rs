use std::io::{Read, Write};
use std::process::ExitCode;

const USAGE: &str = "usage:
  doublebook call       read one JSON request on stdin, write the response to stdout
  doublebook version";

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    match args.iter().map(String::as_str).collect::<Vec<_>>().as_slice() {
        ["call"] => {
            let mut request = String::new();
            if let Err(e) = std::io::stdin().read_to_string(&mut request) {
                eprintln!("doublebook: stdin: {e}");
                return ExitCode::FAILURE;
            }
            let response = doublebook::api::call(&request);
            let mut out = std::io::stdout().lock();
            let _ = out.write_all(response.as_bytes());
            let _ = out.write_all(b"\n");
            if response.starts_with("{\"ok\":true") {
                ExitCode::SUCCESS
            } else {
                ExitCode::FAILURE
            }
        }
        ["version"] => {
            println!("doublebook {}", doublebook::VERSION);
            ExitCode::SUCCESS
        }
        _ => {
            eprintln!("{USAGE}");
            ExitCode::from(2)
        }
    }
}
