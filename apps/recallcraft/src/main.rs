//! RecallCraft: keeps the last two days of what was said, finds your calls and writes them down
//! as they happen.
//!
//! ```text
//! recallcraft                      start the service (and open the window), or open the window
//!                                  of the one running
//! recallcraft --replay             play the last seconds again (what the shortcut does); while
//!                                  one plays, stop it
//! recallcraft --stop-replay        stop the replay playing
//! recallcraft --pause <minutes>    stop recording for a while (0: until --resume)
//! recallcraft --resume             record again
//! recallcraft --settings           open the window at its settings
//! recallcraft --check-updates      ask GitHub for a newer RecallCraft now (a notice answers)
//! recallcraft --install-update     install the update that's waiting (after the call, during one)
//! recallcraft --quit               stop the running service
//! recallcraft --background         the sign-in entry: run the service, and start it again if it
//!                                  stops unexpectedly
//! recallcraft --tray               run the service itself, with nothing shown
//! recallcraft --hear <model>       the worker that hears speech (the service starts it)
//! recallcraft service install|uninstall|status
//!                                  start when you sign in, or not
//! recallcraft --version
//! ```
#![cfg_attr(all(target_os = "windows", not(debug_assertions)), windows_subsystem = "windows")]
#![deny(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::unimplemented, clippy::todo, clippy::unreachable)]

mod browser;
mod graphics;
mod hotkeys;
mod replay;
mod service;
mod speech_model;
mod update;

use std::process::ExitCode;

use recallcraft_desktop::{logging, service as signin, version};

const USAGE: &str = "usage: recallcraft [--replay] [--stop-replay] [--pause <minutes>] [--resume] [--settings] [--quit]
       recallcraft --check-updates | --install-update
       recallcraft --background | --tray
       recallcraft service install|uninstall|status
       recallcraft --version";

/// Set when the service starts the window, so the window knows its sound is left out of the
/// recording (it's one of the service's own processes).
pub const FROM_SERVICE: &str = "RECALLCRAFT_FROM_SERVICE";

fn service_command(what: Option<&str>) -> ExitCode {
    let result = match what {
        Some("install") => signin::enable(),
        Some("uninstall") => signin::disable(),
        Some("status") => {
            let s = signin::status();
            Ok(format!("{} ({})", if s.enabled { "starts when you sign in" } else { "does not start when you sign in" }, s.how))
        }
        _ => Err("usage: recallcraft service install|uninstall|status".to_string()),
    };
    match result {
        Ok(m) => {
            println!("{m}");
            ExitCode::SUCCESS
        }
        Err(e) => {
            eprintln!("error: {e}");
            ExitCode::FAILURE
        }
    }
}

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    if args.iter().any(|a| a == "--version" || a == "-V") {
        println!("{}", version::line("RecallCraft"));
        return ExitCode::SUCCESS;
    }
    if args.iter().any(|a| a == "--help" || a == "-h") {
        println!("{USAGE}");
        return ExitCode::SUCCESS;
    }
    if args.first().map(String::as_str) == Some("service") {
        return service_command(args.get(1).map(String::as_str));
    }
    logging::init();
    if let Some(i) = args.iter().position(|a| a == "--hear") {
        logging::to_file("hearer");
        let Some(model) = args.get(i + 1) else {
            eprintln!("usage: recallcraft --hear <model.gguf>");
            return ExitCode::FAILURE;
        };
        let code = recallcraft_engine::worker::serve(std::path::Path::new(model), &mut std::io::stdin().lock(), &mut std::io::stdout().lock());
        return ExitCode::from(u8::try_from(code.clamp(0, 255)).unwrap_or(1));
    }
    if args.iter().any(|a| a == "--background") {
        logging::to_file("supervisor");
        let Ok(exe) = std::env::current_exe() else { return ExitCode::FAILURE };
        log::info!("{}: supervising the service", version::line("RecallCraft"));
        let code = signin::supervise(&exe);
        return ExitCode::from(u8::try_from(code.clamp(0, 255)).unwrap_or(1));
    }
    if args.iter().any(|a| a == "--window") {
        return browser::run(&args);
    }
    if args.iter().any(|a| a == "--notice") {
        return browser::notice::run();
    }
    if args.iter().any(|a| a == "--replay-bar") {
        return browser::replay_bar::run();
    }
    service::run(&args)
}
