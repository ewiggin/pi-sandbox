#![cfg(target_os = "linux")]

mod cli;
mod dependencies;
mod diagnostics;
mod ignores;
mod sandbox;
mod state;

use std::error::Error;

type Result<T> = std::result::Result<T, Box<dyn Error>>;

fn main() {
    if let Err(error) = cli::parse().and_then(sandbox::run) {
        eprintln!("pi-sandbox: {error}");
        std::process::exit(1);
    }
}
