//! `mneme-models`: a development tool that lists the Whisper model catalog
//! and downloads a model into this crate's `models/` folder, so that tests
//! and `mneme-bench` can use it.
//!
//! It uses the crate's own downloader: HTTPS only, from the pinned Hugging
//! Face revision of the catalog, verified against the catalog's SHA-256
//! before the file gets its final name. The app does not use this binary; it
//! has its own models folder.

use std::io::Write as _;
use std::path::{Path, PathBuf};
use std::process::ExitCode;
use std::time::{Duration, Instant};

use mneme_transcribe::{DownloadState, ModelError, ModelStore, catalog, installed_model_path};

const DEFAULT_MODELS_DIR: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/models");

const HELP: &str = "\
mneme-models: list the Whisper model catalog, or download a model for
development.

USAGE
  mneme-models list [--models-dir <dir>]
  mneme-models fetch <id> [--models-dir <dir>]

  list          Show every model of the catalog, its size, and whether it is
                in the models folder. Reads only.
  fetch <id>    Download a model (for example base.en) into the models
                folder and verify its SHA-256. This uses the network.

  --models-dir <dir>
                The models folder. Default: this crate's models/ folder,
                which git ignores. Do not point `fetch` at the app's models
                folder while the app is running, or at a folder another
                `fetch` is downloading into: starting one removes unfinished
                downloads (*.part files) in the folder.
  -h, --help    Show this help.
";

enum Command {
    Help,
    List { models_dir: PathBuf },
    Fetch { id: String, models_dir: PathBuf },
}

fn parse<I: IntoIterator<Item = String>>(arguments: I) -> Result<Command, String> {
    let mut words = Vec::new();
    let mut models_dir = PathBuf::from(DEFAULT_MODELS_DIR);
    let mut arguments = arguments.into_iter();
    while let Some(argument) = arguments.next() {
        match argument.as_str() {
            "-h" | "--help" => return Ok(Command::Help),
            "--models-dir" => {
                models_dir = PathBuf::from(
                    arguments
                        .next()
                        .filter(|value| !value.is_empty())
                        .ok_or("--models-dir needs a value")?,
                );
            }
            other => match other.strip_prefix("--models-dir=") {
                Some(value) if !value.is_empty() => models_dir = PathBuf::from(value),
                Some(_) => return Err("--models-dir needs a value".to_owned()),
                None if other.starts_with('-') => {
                    return Err(format!("unknown argument `{other}`"));
                }
                None => words.push(argument),
            },
        }
    }
    match words.as_slice() {
        [command] if command == "list" => Ok(Command::List { models_dir }),
        [command, id] if command == "fetch" => Ok(Command::Fetch {
            id: id.clone(),
            models_dir,
        }),
        [command] if command == "fetch" => {
            Err("fetch needs a model id, such as base.en".to_owned())
        }
        [] => Err("a command is needed: list or fetch <id>".to_owned()),
        _ => Err(format!("cannot make sense of `{}`", words.join(" "))),
    }
}

fn main() -> ExitCode {
    let result = match parse(std::env::args().skip(1)) {
        Ok(Command::Help) => {
            print!("{HELP}");
            return ExitCode::SUCCESS;
        }
        Ok(Command::List { models_dir }) => {
            list(&models_dir);
            Ok(())
        }
        Ok(Command::Fetch { id, models_dir }) => fetch(&id, &models_dir),
        Err(message) => {
            eprintln!("mneme-models: {message}");
            eprintln!("Try `mneme-models --help`.");
            return ExitCode::from(2);
        }
    };
    match result {
        Ok(()) => ExitCode::SUCCESS,
        Err(message) => {
            eprintln!("mneme-models: {message}");
            ExitCode::FAILURE
        }
    }
}

/// Prints the catalog. Looks at the folder without opening a model store,
/// so it changes nothing there.
fn list(models_dir: &Path) {
    println!("Models folder: {}", models_dir.display());
    println!("{:<16} {:>15}  {:<10} name", "id", "bytes", "installed");
    for spec in catalog() {
        let installed = installed_model_path(models_dir, &spec.id).is_ok();
        println!(
            "{:<16} {:>15}  {:<10} {}",
            spec.id,
            spec.size_bytes,
            if installed { "yes" } else { "no" },
            spec.name
        );
    }
}

fn fetch(id: &str, models_dir: &Path) -> Result<(), String> {
    if let Ok(path) = installed_model_path(models_dir, id) {
        println!("Already installed: {}", path.display());
        return Ok(());
    }
    let store = ModelStore::open(models_dir).map_err(|error| error.to_string())?;
    let spec = store.spec(id).map_err(|error| match error {
        ModelError::UnknownModel { .. } => format!(
            "unknown model `{id}`. The catalog has: {}",
            catalog()
                .iter()
                .map(|spec| spec.id.as_str())
                .collect::<Vec<_>>()
                .join(", ")
        ),
        other => other.to_string(),
    })?;
    println!(
        "Downloading {} ({} bytes) from {}",
        spec.name, spec.size_bytes, spec.url
    );

    let started = Instant::now();
    let mut last_line = Instant::now() - Duration::from_secs(60);
    let download = store
        .start_download(id, move |progress| {
            // One progress line every two seconds, and one for every change
            // of state after downloading.
            let due = last_line.elapsed() >= Duration::from_secs(2);
            if progress.state == DownloadState::Downloading && !due {
                return;
            }
            last_line = Instant::now();
            let percent = if progress.total_bytes == 0 {
                0.0
            } else {
                progress.received_bytes as f64 * 100.0 / progress.total_bytes as f64
            };
            let mut stderr = std::io::stderr();
            let _ = writeln!(
                stderr,
                "  {:<11} {:>5.1}%  {} of {} bytes",
                progress.state.as_str(),
                percent,
                progress.received_bytes,
                progress.total_bytes
            );
        })
        .map_err(|error| error.to_string())?;
    let path = download.wait().map_err(|error| error.to_string())?;
    println!(
        "Verified SHA-256 {} in {:.0} s",
        spec.sha256,
        started.elapsed().as_secs_f64()
    );
    println!("Saved {}", path.display());
    Ok(())
}
