//! Headless PoC entry. The Tauri window is a separate crate and is not linked here.

use agg_core::check::{run_pinned_core_check, sanitize_report};
use agg_core::pin::pinned_core;
use agg_core::platform::assess_platform;
use std::env;
use std::path::PathBuf;
use std::process::exit;

fn main() {
    let mut args = env::args().skip(1);
    match args.next().as_deref() {
        None | Some("doctor") => {
            println!(
                "{}",
                serde_json::to_string_pretty(&assess_platform()).unwrap_or_else(|_| "{}".into())
            );
        }
        Some("pin") => {
            println!(
                "{}",
                serde_json::to_string_pretty(&pinned_core()).unwrap_or_else(|_| "{}".into())
            );
        }
        Some("core-check") => {
            let mut archive = None;
            let mut binary = None;
            let mut evidence = None;
            while let Some(arg) = args.next() {
                match arg.as_str() {
                    "--archive" => archive = args.next().map(PathBuf::from),
                    "--binary" => binary = args.next().map(PathBuf::from),
                    "--evidence" => evidence = args.next().map(PathBuf::from),
                    other => {
                        eprintln!("unknown argument {other}");
                        exit(2);
                    }
                }
            }
            let Some(archive) = archive else {
                eprintln!("--archive is required");
                exit(2);
            };
            let Some(binary) = binary else {
                eprintln!("--binary is required");
                exit(2);
            };
            match run_pinned_core_check(&archive, &binary) {
                Ok(report) => {
                    let body = sanitize_report(&report, &[]);
                    if let Some(path) = evidence {
                        if let Some(parent) = path.parent() {
                            let _ = std::fs::create_dir_all(parent);
                        }
                        if let Err(err) = std::fs::write(&path, &body) {
                            eprintln!("evidence write: {err}");
                            exit(1);
                        }
                    }
                    println!("{body}");
                    if report.core_integration_pass != "pass" {
                        exit(1);
                    }
                }
                Err(err) => {
                    eprintln!("{err}");
                    exit(1);
                }
            }
        }
        Some(other) => {
            eprintln!("unknown command {other}");
            eprintln!("usage: agg-poc [doctor|pin|core-check --archive FILE --binary FILE]");
            exit(2);
        }
    }
}
