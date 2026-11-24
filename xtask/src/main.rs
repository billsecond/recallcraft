//! Workspace tooling: `cargo xtask <command>`. `cargo` and `git` are run through
//! `std::process::Command`.

mod assets;
mod icon;
mod names;
mod version;

use std::path::{Path, PathBuf};
use std::process::{Command, ExitCode};

const USAGE: &str = "\
usage: cargo xtask <command>

commands:
  ci              fmt --check, clippy -D warnings, test, assets, names, layers (stops at the first failure)
  assets          check that every icon, model and other asset file is attributed in ATTRIBUTION.md
  names [hash <word>]
                  check that no file names the products RecallCraft is modelled on, or their makers
                  (hash: print the entry for another word)
  layers          check that nothing below the programs depends on windows, trays or shortcuts
  icon            write the icon in every size the packages use into assets/app-icon
  version [set X.Y.Z[-pre]]
                  show or set the workspace version
";

/// What only the programs (apps/) may depend on: windows, the tray, shortcuts, dialogs.
const UI_CRATES: &[&str] = &["eframe", "egui", "winit", "tray-icon", "global-hotkey", "rfd"];

/// Workspace root (parent of the xtask crate).
pub fn root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).parent().map(Path::to_path_buf).unwrap_or_else(|| PathBuf::from("."))
}

/// `cargo`, run in the workspace root.
pub fn cargo() -> Command {
    let mut c = Command::new(std::env::var("CARGO").unwrap_or_else(|_| "cargo".into()));
    c.current_dir(root());
    c
}

/// Runs `cmd`, failing unless it succeeds.
pub fn run(mut cmd: Command, what: &str) -> Result<(), String> {
    eprintln!("$ {what}");
    let status = cmd.status().map_err(|e| format!("{what}: failed to start: {e}"))?;
    if status.success() { Ok(()) } else { Err(format!("{what}: exited with {status}")) }
}

fn cargo_step(args: &'static [&'static str]) -> Result<(), String> {
    let mut c = cargo();
    c.args(args);
    run(c, &format!("cargo {}", args.join(" ")))
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
        println!("layers: the crates have no windows, trays or shortcuts");
        Ok(())
    } else {
        Err(format!("only the programs may depend on these:\n  {}", bad.join("\n  ")))
    }
}

fn ci() -> Result<(), String> {
    type Step = (&'static str, Box<dyn Fn() -> Result<(), String>>);
    let steps: Vec<Step> = vec![
        ("fmt", Box::new(|| cargo_step(&["fmt", "--all", "--check"]))),
        ("clippy", Box::new(|| cargo_step(&["clippy", "--workspace", "--all-targets", "--", "-D", "warnings"]))),
        ("test", Box::new(|| cargo_step(&["test", "--workspace"]))),
        ("assets", Box::new(|| assets::run(&root()))),
        ("names", Box::new(|| names::run(&root(), &[]))),
        ("layers", Box::new(layers)),
    ];
    let mut done = Vec::new();
    for (name, f) in &steps {
        eprintln!("\n=== ci: {name} ===");
        if let Err(e) = f() {
            println!("\nCI summary:");
            for d in &done {
                println!("  ok    {d}");
            }
            println!("  FAIL  {name}: {e}");
            for (n, _) in steps.iter().skip(done.len() + 1) {
                println!("  skip  {n}");
            }
            return Err(format!("ci failed at `{name}`"));
        }
        done.push(*name);
    }
    println!("\nCI summary: all {} steps passed ({})", done.len(), done.join(", "));
    Ok(())
}

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let rest: Vec<&str> = args.iter().skip(1).map(String::as_str).collect();
    let result = match args.first().map(String::as_str) {
        Some("ci") => ci(),
        Some("assets") => assets::run(&root()),
        Some("names") => names::run(&root(), &rest),
        Some("layers") => layers(),
        Some("icon") => icon::run(&root()),
        Some("version") => version::run(&root(), &rest),
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
