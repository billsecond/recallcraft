//! How far apart the voices of some WAV files are, as RecallCraft tells voices apart:
//! `cargo run --release -p recallcraft-speech --example voices -- a.wav b.wav …`.

use recallcraft_speech::voices::{Analyzer, distance};

fn main() -> Result<(), String> {
    let files: Vec<String> = std::env::args().skip(1).collect();
    let a = Analyzer::new();
    let mut fps = Vec::new();
    for f in &files {
        let b = std::fs::read(f).map_err(|e| format!("{f}: {e}"))?;
        let w = parse(&b).ok_or(format!("{f}: not a 16-bit WAV"))?;
        match a.fingerprint(&w) {
            Some(fp) => fps.push((f.rsplit(['/', '\\']).next().unwrap_or(f).to_string(), fp)),
            None => println!("{f}: too little speech"),
        }
    }
    print!("{:>8}", "");
    for (n, _) in &fps {
        print!("{n:>8}");
    }
    println!();
    for (n, x) in &fps {
        print!("{n:>8}");
        for (_, y) in &fps {
            print!("{:>8.3}", distance(&x.v, &y.v));
        }
        println!("   ({} frames)", x.frames);
    }
    Ok(())
}

/// A 16-bit PCM WAV as mono 16 kHz.
fn parse(b: &[u8]) -> Option<Vec<f32>> {
    let rate = u32::from_le_bytes(b.get(24..28)?.try_into().ok()?);
    let ch = u16::from_le_bytes(b.get(22..24)?.try_into().ok()?) as usize;
    let data = b.windows(4).position(|w| w == b"data")? + 8;
    let s: Vec<f32> = b.get(data..)?.chunks_exact(2 * ch).map(|c| f32::from(i16::from_le_bytes([c[0], c[1]])) / 32768.0).collect();
    let step = rate as f64 / 16_000.0;
    Some((0..(s.len() as f64 / step) as usize).map(|i| s[((i as f64 * step) as usize).min(s.len() - 1)]).collect())
}
