//! Imports documents into a Ledgerlens data directory from the command line.
//!
//! `cargo run -p ll-app --example import -- <data-dir> <file>...`

use std::{env, fs, process::ExitCode};

fn main() -> ExitCode {
    let mut args = env::args_os().skip(1);
    let Some(data_dir) = args.next() else {
        eprintln!("usage: import <data-dir> <file>...");
        return ExitCode::FAILURE;
    };
    let mut app = match ll_app::App::open(data_dir.as_ref()) {
        Ok(app) => app,
        Err(error) => {
            eprintln!("cannot open {}: {error}", data_dir.to_string_lossy());
            return ExitCode::FAILURE;
        }
    };
    let mut failed = false;
    for path in args {
        let name = path.to_string_lossy().into_owned();
        let result = fs::read(&path)
            .map_err(ll_core::Error::from)
            .and_then(|bytes| app.import(&name, &bytes));
        match result {
            Ok(outcome) => println!(
                "{name}: {} ({}){}",
                outcome.document.status.as_str(),
                outcome.document.vendor.as_deref().unwrap_or("no vendor"),
                if outcome.duplicate { ", already imported" } else { "" }
            ),
            Err(error) => {
                failed = true;
                eprintln!("{name}: {error}");
            }
        }
    }
    if failed { ExitCode::FAILURE } else { ExitCode::SUCCESS }
}
