//! WAV files (16-bit PCM), written as the sound is made, so an hour-long call never has to be
//! in memory at once.

use std::fs::File;
use std::io::{BufWriter, Seek, SeekFrom, Write};
use std::path::Path;

/// A WAV file being written.
pub struct WavWriter {
    out: BufWriter<File>,
    channels: u16,
    data_bytes: u64,
}

/// The most a WAV file can hold (its sizes are 32-bit).
const MAX_DATA: u64 = u32::MAX as u64 - 64;

impl WavWriter {
    pub fn create(path: &Path, channels: u16, rate: u32) -> std::io::Result<WavWriter> {
        let mut out = BufWriter::new(File::create(path)?);
        let channels = channels.clamp(1, 2);
        let block = channels * 2;
        out.write_all(b"RIFF")?;
        out.write_all(&0u32.to_le_bytes())?;
        out.write_all(b"WAVEfmt ")?;
        out.write_all(&16u32.to_le_bytes())?;
        out.write_all(&1u16.to_le_bytes())?;
        out.write_all(&channels.to_le_bytes())?;
        out.write_all(&rate.to_le_bytes())?;
        out.write_all(&(rate * u32::from(block)).to_le_bytes())?;
        out.write_all(&block.to_le_bytes())?;
        out.write_all(&16u16.to_le_bytes())?;
        out.write_all(b"data")?;
        out.write_all(&0u32.to_le_bytes())?;
        Ok(WavWriter { out, channels, data_bytes: 0 })
    }

    /// Interleaved samples, -1 to 1.
    pub fn write(&mut self, samples: &[f32]) -> std::io::Result<()> {
        let mut bytes = Vec::with_capacity(samples.len() * 2);
        for s in samples {
            if self.data_bytes + bytes.len() as u64 + 2 > MAX_DATA {
                break;
            }
            let v = if s.is_finite() { (s.clamp(-1.0, 1.0) * 32767.0).round() as i16 } else { 0 };
            bytes.extend_from_slice(&v.to_le_bytes());
        }
        self.data_bytes += bytes.len() as u64;
        self.out.write_all(&bytes)
    }

    pub fn channels(&self) -> u16 {
        self.channels
    }

    /// Writes the sizes into the header.
    pub fn finish(mut self) -> std::io::Result<()> {
        self.out.flush()?;
        let mut f = self.out.into_inner().map_err(|e| e.into_error())?;
        let data = self.data_bytes.min(MAX_DATA) as u32;
        f.seek(SeekFrom::Start(4))?;
        f.write_all(&(36 + data).to_le_bytes())?;
        f.seek(SeekFrom::Start(40))?;
        f.write_all(&data.to_le_bytes())?;
        f.flush()
    }
}

/// A WAV file's sound.
#[derive(Clone, Debug, PartialEq)]
pub struct Wav {
    pub rate: u32,
    pub channels: u16,
    /// Interleaved, -1 to 1.
    pub samples: Vec<f32>,
}

/// The largest WAV file read.
const MAX_READ: u64 = 2 * 1024 * 1024 * 1024;

/// Reads a WAV file: 8, 16, 24 or 32-bit PCM, or 32-bit floats.
pub fn read(path: &Path) -> Result<Wav, String> {
    let meta = std::fs::metadata(path).map_err(|e| format!("{}: {e}", path.display()))?;
    if meta.len() > MAX_READ {
        return Err(format!("{}: too large", path.display()));
    }
    let b = std::fs::read(path).map_err(|e| format!("{}: {e}", path.display()))?;
    parse(&b).map_err(|e| format!("{}: {e}", path.display()))
}

fn u16_at(b: &[u8], i: usize) -> Option<u16> {
    Some(u16::from_le_bytes([*b.get(i)?, *b.get(i + 1)?]))
}

fn u32_at(b: &[u8], i: usize) -> Option<u32> {
    Some(u32::from_le_bytes([*b.get(i)?, *b.get(i + 1)?, *b.get(i + 2)?, *b.get(i + 3)?]))
}

pub fn parse(b: &[u8]) -> Result<Wav, String> {
    if b.get(..4) != Some(b"RIFF") || b.get(8..12) != Some(b"WAVE") {
        return Err("not a WAV file".into());
    }
    let mut at = 12usize;
    let mut fmt: Option<(u16, u16, u32, u16)> = None;
    while at + 8 <= b.len() {
        let id = b.get(at..at + 4).unwrap_or_default();
        let size = u32_at(b, at + 4).ok_or("cut short")? as usize;
        let body = at + 8;
        let end = body.saturating_add(size).min(b.len());
        if id == b"fmt " {
            let format = u16_at(b, body).ok_or("cut short")?;
            let channels = u16_at(b, body + 2).ok_or("cut short")?;
            let rate = u32_at(b, body + 4).ok_or("cut short")?;
            let bits = u16_at(b, body + 14).ok_or("cut short")?;
            // WAVE_FORMAT_EXTENSIBLE: the real format is in its sub-format.
            let format = if format == 0xFFFE { u16_at(b, body + 24).unwrap_or(1) } else { format };
            fmt = Some((format, channels, rate, bits));
        } else if id == b"data" {
            let (format, channels, rate, bits) = fmt.ok_or("no format before the sound")?;
            if channels == 0 || channels > 8 || rate == 0 {
                return Err("an odd format".into());
            }
            let data = b.get(body..end).unwrap_or_default();
            let samples: Vec<f32> = match (format, bits) {
                (1, 8) => data.iter().map(|v| (f32::from(*v) - 128.0) / 128.0).collect(),
                (1, 16) => data.as_chunks::<2>().0.iter().map(|c| f32::from(i16::from_le_bytes(*c)) / 32768.0).collect(),
                (1, 24) => data.as_chunks::<3>().0.iter().map(|[a, b, c]| (i32::from_le_bytes([0, *a, *b, *c]) >> 8) as f32 / 8_388_608.0).collect(),
                (1, 32) => data.as_chunks::<4>().0.iter().map(|c| i32::from_le_bytes(*c) as f32 / 2_147_483_648.0).collect(),
                (3, 32) => data
                    .as_chunks::<4>()
                    .0
                    .iter()
                    .map(|c| f32::from_le_bytes(*c))
                    .map(|v| if v.is_finite() { v.clamp(-1.0, 1.0) } else { 0.0 })
                    .collect(),
                _ => return Err(format!("format {format} with {bits} bits isn't read")),
            };
            return Ok(Wav { rate, channels, samples });
        }
        at = body.saturating_add(size + (size & 1));
    }
    Err("no sound in it".into())
}

/// Interleaved sound made mono at another rate (linearly: fine for speech).
pub fn mono_at(w: &Wav, rate: u32) -> Vec<f32> {
    let ch = usize::from(w.channels.max(1));
    let mono: Vec<f32> = w.samples.chunks_exact(ch).map(|f| f.iter().sum::<f32>() / ch as f32).collect();
    if w.rate == rate || mono.is_empty() {
        return mono;
    }
    let step = f64::from(w.rate) / f64::from(rate.max(1));
    let n = (mono.len() as f64 / step) as usize;
    (0..n)
        .map(|i| {
            let p = i as f64 * step;
            let k = p.floor() as usize;
            let f = (p - k as f64) as f32;
            let a = mono.get(k).copied().unwrap_or(0.0);
            let b = mono.get(k + 1).copied().unwrap_or(a);
            a + (b - a) * f
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_wav_reads_back() {
        let d = crate::temp_dir("wav-read");
        let p = d.join("b.wav");
        let mut w = WavWriter::create(&p, 2, 22_050).unwrap();
        w.write(&[0.5, -0.5, 0.25, -0.25]).unwrap();
        w.finish().unwrap();
        let back = read(&p).unwrap();
        assert_eq!((back.rate, back.channels, back.samples.len()), (22_050, 2, 4));
        assert!((back.samples[0] - 0.5).abs() < 1e-3);
        let m = mono_at(&Wav { rate: 8000, channels: 1, samples: vec![0.0, 1.0] }, 16_000);
        assert_eq!(m.len(), 4);
        assert!((m[1] - 0.5).abs() < 1e-6);
        assert!(parse(b"RIFFxxxxWAVE").is_err());
        assert!(parse(b"nope").is_err());
        let _ = std::fs::remove_dir_all(&d);
    }

    #[test]
    fn a_wav_has_its_sizes() {
        let d = crate::temp_dir("wav");
        let p = d.join("a.wav");
        let mut w = WavWriter::create(&p, 2, 48_000).unwrap();
        w.write(&[0.5, -0.5, 2.0, f32::NAN]).unwrap();
        w.finish().unwrap();
        let b = std::fs::read(&p).unwrap();
        assert_eq!(b.len(), 44 + 8);
        assert_eq!(&b[0..4], b"RIFF");
        assert_eq!(u32::from_le_bytes([b[4], b[5], b[6], b[7]]), 44);
        assert_eq!(u32::from_le_bytes([b[40], b[41], b[42], b[43]]), 8);
        assert_eq!(i16::from_le_bytes([b[44], b[45]]), 16384);
        assert_eq!(i16::from_le_bytes([b[48], b[49]]), 32767);
        assert_eq!(i16::from_le_bytes([b[50], b[51]]), 0);
        let _ = std::fs::remove_dir_all(&d);
    }
}
