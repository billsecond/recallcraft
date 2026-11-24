//! Lists the microphones and records each source for three seconds, printing only how loud it
//! was (nothing is kept): `cargo run -p recallcraft-audio --example devices`.

use std::sync::mpsc::channel;
use std::time::{Duration, Instant};

use recallcraft_audio::capture::{Capture, Mic, Source};
use recallcraft_audio::devices;

fn main() {
    println!("microphones:");
    for d in devices::microphones() {
        println!("  {}{}", d.name, if devices::is_bluetooth(&d.name) { " (Bluetooth)" } else { "" });
    }
    println!("default: {:?}", devices::default_microphone(false).map(|d| d.name));
    println!("default for calls: {:?}", devices::default_microphone(true).map(|d| d.name));
    for source in [Source::System, Source::Mic(Mic::Default)] {
        let (tx, rx) = channel();
        match Capture::start(source.clone(), tx) {
            Err(e) => println!("{:?}: {e}", source),
            Ok(c) => {
                let started = Instant::now();
                let (mut samples, mut peak, mut blocks) = (0usize, 0f32, 0usize);
                let mut first_late: Option<i64> = None;
                while started.elapsed() < Duration::from_secs(3) {
                    while let Ok(b) = rx.try_recv() {
                        if first_late.is_none() {
                            first_late = Some(recallcraft_model::time::now_us() as i64 - b.start_us as i64);
                        }
                        samples += b.samples.len();
                        blocks += 1;
                        peak = peak.max(recallcraft_audio::peak(&b.samples));
                    }
                    std::thread::sleep(Duration::from_millis(20));
                }
                println!(
                    "{} ({}): {blocks} blocks, {:.2} s of sound in 3 s, loudest {:.3}, first block {} ms old",
                    c.name,
                    match source {
                        Source::System | Source::Speaker(_) => "computer sound",
                        Source::Mic(_) => "microphone",
                    },
                    samples as f32 / 48_000.0,
                    peak,
                    first_late.unwrap_or(0) / 1000
                );
                c.stop();
            }
        }
    }
}
