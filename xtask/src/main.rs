//! Workspace tooling: `cargo xtask <command>`. `cargo` is run through `std::process::Command`.

use std::path::{Path, PathBuf};
use std::process::{Command, ExitCode};

const USAGE: &str = "\
usage: cargo xtask <command>

commands:
  ci       fmt --check, clippy -D warnings, test, layers (stops at the first failure)
  layers   check that nothing below the programs depends on windows, trays or shortcuts
  icon     draw the icon into assets/app-icon/recallcraft.ico
";

/// What only the programs (apps/) may depend on: windows, the tray, shortcuts, dialogs.
const UI_CRATES: &[&str] = &["eframe", "egui", "winit", "tray-icon", "global-hotkey", "rfd"];

fn root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).parent().map(Path::to_path_buf).unwrap_or_else(|| PathBuf::from("."))
}

fn cargo(args: &[&str]) -> Result<(), String> {
    println!("== cargo {}", args.join(" "));
    let status =
        Command::new(std::env::var("CARGO").unwrap_or_else(|_| "cargo".into())).args(args).current_dir(root()).status().map_err(|e| e.to_string())?;
    if status.success() { Ok(()) } else { Err(format!("cargo {} failed", args.join(" "))) }
}

fn layers() -> Result<(), String> {
    let crates = root().join("crates");
    let mut bad = Vec::new();
    for entry in std::fs::read_dir(&crates).map_err(|e| e.to_string())?.flatten() {
        let manifest = entry.path().join("Cargo.toml");
        let Ok(text) = std::fs::read_to_string(&manifest) else { continue };
        for line in text.lines() {
            let name = line.split('=').next().unwrap_or_default().trim();
            if UI_CRATES.contains(&name) {
                bad.push(format!("{}: {name}", manifest.display()));
            }
        }
    }
    if bad.is_empty() {
        println!("== layers: the crates have no windows, trays or shortcuts");
        Ok(())
    } else {
        Err(format!("only the programs may depend on these:\n  {}", bad.join("\n  ")))
    }
}

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let result = match args.first().map(String::as_str) {
        Some("ci") => cargo(&["fmt", "--all", "--check"])
            .and_then(|()| cargo(&["clippy", "--workspace", "--all-targets", "--", "-D", "warnings"]))
            .and_then(|()| cargo(&["test", "--workspace"]))
            .and_then(|()| layers()),
        Some("layers") => layers(),
        Some("icon") => {
            let out = root().join("assets").join("app-icon").join("recallcraft.ico");
            std::fs::write(&out, recallcraft_desktop::icon::ico()).map(|()| println!("{}", out.display())).map_err(|e| e.to_string())
        }
        Some("-h" | "--help" | "help") | None => {
            print!("{USAGE}");
            Ok(())
        }
        Some(other) => Err(format!("unknown command `{other}`\n\n{USAGE}")),
    };
    match result {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("error: {e}");
            ExitCode::FAILURE
        }
    }
}
