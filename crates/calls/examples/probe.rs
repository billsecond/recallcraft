//! Prints which programs have a microphone open just now, and what RecallCraft makes of them:
//! a way to check call detection on a computer (`cargo run -p recallcraft-calls --example probe`).
//! Only program files and device ids are printed, never a window's title.

use recallcraft_calls::{apps, probe};

fn main() {
    let procs = probe::processes();
    let own = probe::own_tree(&procs);
    let capturing = probe::capturing(&procs, &own);
    println!("{} processes; {} with a microphone open", procs.len(), capturing.len());
    for c in &capturing {
        let call = apps::classify(&c.app_exe, true, false, &[], &[]).or_else(|| apps::classify(&c.exe, true, false, &[], &[]));
        println!(
            "  {} (pid {}, app {}) on {}: {}",
            c.exe,
            c.pid,
            c.app_exe,
            c.device_id,
            call.map_or("not a call".to_string(), |a| format!("a call in {}", a.name))
        );
    }
    println!("{} windows with titles", probe::windows().len());
}
