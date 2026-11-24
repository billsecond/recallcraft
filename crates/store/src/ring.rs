//! The recording: everything heard, in two streams, kept as Opus packets in files of five
//! minutes each (`ring/<mic|system>/<start ms>.rca`).
//!
//! A file is a header and records, appended as the sound comes in and flushed every second, so
//! the window can play and show what's being recorded:
//!
//! ```text
//! header   "RCRING01"  version u16  stream u8  channels u8  rate u32  frame u16  [6]  created µs u64   (32 bytes)
//! run      'R'  start µs u64                 the packets after it follow one another, 20 ms apart
//! packet   'P'  level u8  length u16  bytes  one 20 ms packet; level is its loudness, (dB + 100) × 2
//! ```
//!
//! All numbers are little-endian; times are microseconds since 1970 (UTC). Silence (sound
//! quieter than [`GATE_DB`] for longer than a second) isn't kept: the run ends, and the next
//! sound starts a new one at its own time. A record cut short at the end of a file (it's being
//! written, or the program stopped) is left out. When a file is closed, a `.lvl` file beside it
//! keeps its loudness each second, so the overview of two days reads a few kilobytes.

use std::collections::VecDeque;
use std::fs::File;
use std::io::{BufWriter, Read, Seek, SeekFrom, Write};
use std::path::{Path, PathBuf};
use std::time::Instant;

use recallcraft_model::Stream;

use crate::StoreError;
use crate::opus::{Decoder, Encoder, FRAME, MAX_PACKET, RATE};

pub const FRAME_US: u64 = 20_000;
/// How long one file covers.
pub const SEGMENT_US: u64 = 300_000_000;
/// Quieter than this, a frame is silence.
pub const GATE_DB: f32 = -72.0;
/// Frames of silence kept after sound, so a voice's end isn't cut (1 s).
const HANGOVER: u32 = 50;
/// A block of sound starting within this of where the last one ended carries on from it.
const JOIN_US: u64 = 60_000;
/// The level of nothing at all.
pub const SILENT_DB: f32 = -100.0;

const MAGIC: &[u8; 8] = b"RCRING01";
const HEADER: usize = 32;
const TAG_RUN: u8 = b'R';
const TAG_PACKET: u8 = b'P';
/// The largest file read (a damaged one can be any size).
const MAX_FILE: u64 = 64 * 1024 * 1024;
/// Files read and kept in memory at once.
const CACHE: usize = 8;

/// A loudness as the byte kept with a packet.
pub fn level_byte(db: f32) -> u8 {
    if !db.is_finite() {
        return 0;
    }
    ((db.clamp(SILENT_DB, 0.0) - SILENT_DB) * 2.0).round() as u8
}

/// The loudness a byte stands for.
pub fn byte_level(b: u8) -> f32 {
    f32::from(b.min(200)) / 2.0 + SILENT_DB
}

/// How loud some samples are, in dB below full scale.
pub fn rms_db(samples: &[f32]) -> f32 {
    if samples.is_empty() {
        return SILENT_DB;
    }
    let sum: f64 = samples.iter().map(|s| if s.is_finite() { f64::from(*s) * f64::from(*s) } else { 0.0 }).sum();
    (10.0 * (sum / samples.len() as f64 + 1e-10).log10()).max(f64::from(SILENT_DB)) as f32
}

/// Samples at 48 kHz in `us`.
pub fn us_to_samples(us: u64, rate: u32) -> u64 {
    (u128::from(us) * u128::from(rate) / 1_000_000) as u64
}

pub fn samples_to_us(n: u64, rate: u32) -> u64 {
    (u128::from(n) * 1_000_000 / u128::from(rate.max(1))) as u64
}

fn stream_dir(root: &Path, stream: Stream) -> PathBuf {
    root.join(stream.dir_name())
}

/// A file of the recording.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SegmentInfo {
    pub path: PathBuf,
    pub start_us: u64,
}

impl SegmentInfo {
    /// Where it ends at the latest.
    pub fn end_us(&self) -> u64 {
        self.start_us.saturating_add(SEGMENT_US + 1_000_000)
    }
}

/// The files of a stream, oldest first.
pub fn segments(root: &Path, stream: Stream) -> Vec<SegmentInfo> {
    let Ok(entries) = std::fs::read_dir(stream_dir(root, stream)) else { return Vec::new() };
    let mut out: Vec<SegmentInfo> = entries
        .flatten()
        .filter_map(|e| {
            let path = e.path();
            if !path.extension().is_some_and(|x| x == "rca") {
                return None;
            }
            let ms: u64 = path.file_stem()?.to_str()?.parse().ok()?;
            Some(SegmentInfo { path, start_us: ms.saturating_mul(1000) })
        })
        .collect();
    out.sort_by_key(|s| s.start_us);
    out
}

// ---- writing ----

struct OpenSegment {
    file: BufWriter<File>,
    path: PathBuf,
    start_us: u64,
    /// The loudest packet each second.
    seconds: Vec<u8>,
}

struct StreamWriter {
    stream: Stream,
    dir: PathBuf,
    bitrate: u32,
    /// Samples not yet a whole frame, from `pending_us`.
    pending: Vec<f32>,
    pending_us: u64,
    /// When the next sound should start, if it carries on from the last.
    expect_us: Option<u64>,
    /// The encoder of the run being written.
    enc: Option<Encoder>,
    next_us: u64,
    quiet: u32,
    seg: Option<OpenSegment>,
    written: u64,
}

impl StreamWriter {
    fn new(root: &Path, stream: Stream) -> StreamWriter {
        let bitrate = match stream {
            Stream::Mic => 24_000,
            Stream::System => 32_000,
        };
        StreamWriter {
            stream,
            dir: stream_dir(root, stream),
            bitrate,
            pending: Vec::new(),
            pending_us: 0,
            expect_us: None,
            enc: None,
            next_us: 0,
            quiet: 0,
            seg: None,
            written: 0,
        }
    }

    fn push(&mut self, start_us: u64, samples: &[f32]) -> Result<(), StoreError> {
        // Sound close to where the last ended carries on from it, on the same 20 ms steps (the
        // device's timing wobbles a little); anything else is a new stretch, and what was left
        // over (less than a frame) goes.
        let continues = self.expect_us.is_some_and(|e| start_us.abs_diff(e) <= JOIN_US);
        if !continues {
            self.pending.clear();
            self.pending_us = start_us;
            self.enc = None;
        }
        self.pending.extend_from_slice(samples);
        self.expect_us = Some(self.pending_us + samples_to_us(self.pending.len() as u64, RATE));
        let mut at = 0usize;
        while self.pending.len() - at >= FRAME {
            let mut frame = [0f32; FRAME];
            frame.copy_from_slice(self.pending.get(at..at + FRAME).unwrap_or(&[0.0; FRAME]));
            let us = self.pending_us;
            self.frame(&frame, us)?;
            at += FRAME;
            self.pending_us += FRAME_US;
        }
        self.pending.drain(..at);
        Ok(())
    }

    fn frame(&mut self, frame: &[f32; FRAME], us: u64) -> Result<(), StoreError> {
        let db = rms_db(frame);
        if db > GATE_DB {
            self.quiet = 0;
        } else {
            self.quiet = self.quiet.saturating_add(1);
        }
        if db <= GATE_DB && (self.enc.is_none() || self.quiet > HANGOVER) {
            self.enc = None;
            return Ok(());
        }
        let roll = match &self.seg {
            None => true,
            // A file holds one five-minute stretch of the clock (:00 to :05, …).
            Some(s) => us / SEGMENT_US != s.start_us / SEGMENT_US || us < s.start_us,
        };
        if roll {
            self.close();
            self.open(us)?;
        }
        let new_run = self.enc.is_none() || self.next_us != us;
        if new_run {
            self.enc = Some(Encoder::new(self.bitrate, self.stream == Stream::Mic)?);
        }
        let Some(enc) = self.enc.as_mut() else { return Ok(()) };
        let packet = enc.encode(frame)?;
        let Some(seg) = self.seg.as_mut() else { return Ok(()) };
        if new_run {
            seg.file.write_all(&[TAG_RUN])?;
            seg.file.write_all(&us.to_le_bytes())?;
        }
        let level = level_byte(db);
        let len = packet.len().min(MAX_PACKET) as u16;
        seg.file.write_all(&[TAG_PACKET, level])?;
        seg.file.write_all(&len.to_le_bytes())?;
        seg.file.write_all(packet.get(..usize::from(len)).unwrap_or_default())?;
        self.written += 4 + u64::from(len);
        let second = (us.saturating_sub(seg.start_us) / 1_000_000) as usize;
        if second < 400 {
            if seg.seconds.len() <= second {
                seg.seconds.resize(second + 1, 0);
            }
            if let Some(s) = seg.seconds.get_mut(second) {
                *s = (*s).max(level);
            }
        }
        self.next_us = us + FRAME_US;
        Ok(())
    }

    fn open(&mut self, us: u64) -> Result<(), StoreError> {
        std::fs::create_dir_all(&self.dir)?;
        let mut ms = us / 1000;
        let (file, path) = loop {
            let path = self.dir.join(format!("{ms}.rca"));
            match std::fs::OpenOptions::new().write(true).create_new(true).open(&path) {
                Ok(f) => break (f, path),
                Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists && ms < us / 1000 + 1000 => ms += 1,
                Err(e) => return Err(e.into()),
            }
        };
        let mut file = BufWriter::with_capacity(64 * 1024, file);
        let mut header = [0u8; HEADER];
        header[..8].copy_from_slice(MAGIC);
        header[8..10].copy_from_slice(&1u16.to_le_bytes());
        header[10] = self.stream.id();
        header[11] = 1;
        header[12..16].copy_from_slice(&RATE.to_le_bytes());
        header[16..18].copy_from_slice(&(FRAME as u16).to_le_bytes());
        header[24..32].copy_from_slice(&us.to_le_bytes());
        file.write_all(&header)?;
        self.seg = Some(OpenSegment { file, path, start_us: us, seconds: Vec::new() });
        self.enc = None;
        Ok(())
    }

    fn flush(&mut self) {
        if let Some(s) = self.seg.as_mut()
            && let Err(e) = s.file.flush()
        {
            log::warn!("writing the {} recording: {e}", self.stream.dir_name());
        }
    }

    /// Closes the file being written, and writes its loudness beside it.
    fn close(&mut self) {
        if let Some(mut s) = self.seg.take() {
            if let Err(e) = s.file.flush() {
                log::warn!("closing {}: {e}", s.path.display());
            }
            let _ = std::fs::write(s.path.with_extension("lvl"), &s.seconds);
        }
        self.enc = None;
    }
}

/// Writes both streams. Sound is handed over as mono 48 kHz samples with the time (µs since
/// 1970) of the first.
pub struct RingWriter {
    streams: [StreamWriter; 2],
    flushed: Instant,
}

impl RingWriter {
    pub fn new(root: &Path) -> RingWriter {
        RingWriter { streams: [StreamWriter::new(root, Stream::Mic), StreamWriter::new(root, Stream::System)], flushed: Instant::now() }
    }

    fn of(&mut self, stream: Stream) -> &mut StreamWriter {
        let [mic, system] = &mut self.streams;
        match stream {
            Stream::Mic => mic,
            Stream::System => system,
        }
    }

    pub fn push(&mut self, stream: Stream, start_us: u64, samples: &[f32]) -> Result<(), StoreError> {
        let w = self.of(stream);
        let r = w.push(start_us, samples);
        if r.is_err() {
            // A file that can't be written: the next sound starts a fresh one.
            w.close();
        }
        if self.flushed.elapsed().as_millis() >= 1000 {
            self.flush();
        }
        r
    }

    /// The end of a stream's sound for now (a device stopped, recording paused): the next sound
    /// starts a run of its own.
    pub fn gap(&mut self, stream: Stream) {
        let w = self.of(stream);
        w.pending.clear();
        w.expect_us = None;
        w.enc = None;
    }

    pub fn flush(&mut self) {
        for w in &mut self.streams {
            w.flush();
        }
        self.flushed = Instant::now();
    }

    /// Closes both files (when the service stops).
    pub fn close(&mut self) {
        for w in &mut self.streams {
            w.close();
        }
    }

    /// Bytes written since the start.
    pub fn written(&self) -> u64 {
        self.streams.iter().map(|w| w.written).sum()
    }
}

impl Drop for RingWriter {
    fn drop(&mut self) {
        self.close();
    }
}

// ---- reading ----

#[derive(Clone, Copy, Debug)]
struct Packet {
    at: u32,
    len: u16,
    level: u8,
}

#[derive(Clone, Debug, Default)]
struct Run {
    start_us: u64,
    packets: Vec<Packet>,
}

impl Run {
    fn end_us(&self) -> u64 {
        self.start_us + self.packets.len() as u64 * FRAME_US
    }
}

struct Loaded {
    path: PathBuf,
    data: Vec<u8>,
    parsed: usize,
    runs: Vec<Run>,
    broken: bool,
    used: u64,
}

impl Loaded {
    fn open(path: &Path) -> Loaded {
        let mut l = Loaded { path: path.to_path_buf(), data: Vec::new(), parsed: 0, runs: Vec::new(), broken: false, used: 0 };
        l.refresh();
        l
    }

    /// Reads what was written since last time, and the records in it.
    fn refresh(&mut self) {
        if self.broken {
            return;
        }
        let Ok(meta) = std::fs::metadata(&self.path) else { return };
        let len = meta.len().min(MAX_FILE);
        if len <= self.data.len() as u64 {
            return;
        }
        let Ok(mut f) = File::open(&self.path) else { return };
        if f.seek(SeekFrom::Start(self.data.len() as u64)).is_err() {
            return;
        }
        let mut more = Vec::new();
        if f.take(len - self.data.len() as u64).read_to_end(&mut more).is_err() {
            return;
        }
        self.data.extend_from_slice(&more);
        self.parse();
    }

    fn parse(&mut self) {
        if self.parsed == 0 {
            if self.data.len() < HEADER {
                return;
            }
            if self.data.get(..8) != Some(MAGIC.as_slice()) {
                log::warn!("{} isn't a recording; left out", self.path.display());
                self.broken = true;
                return;
            }
            self.parsed = HEADER;
        }
        let d = &self.data;
        let mut at = self.parsed;
        while let Some(&tag) = d.get(at) {
            match tag {
                TAG_RUN => {
                    let Some(b) = d.get(at + 1..at + 9) else { break };
                    let mut t = [0u8; 8];
                    t.copy_from_slice(b);
                    self.runs.push(Run { start_us: u64::from_le_bytes(t), packets: Vec::new() });
                    at += 9;
                }
                TAG_PACKET => {
                    let (Some(&level), Some(l0), Some(l1)) = (d.get(at + 1), d.get(at + 2), d.get(at + 3)) else { break };
                    let len = u16::from_le_bytes([*l0, *l1]);
                    let start = at + 4;
                    if d.len() < start + usize::from(len) {
                        break;
                    }
                    if usize::from(len) <= MAX_PACKET
                        && let Some(run) = self.runs.last_mut()
                        && let Ok(off) = u32::try_from(start)
                    {
                        run.packets.push(Packet { at: off, len, level });
                    }
                    at = start + usize::from(len);
                }
                _ => {
                    log::warn!("{} is damaged at byte {at}; the rest is left out", self.path.display());
                    self.broken = true;
                    break;
                }
            }
        }
        self.parsed = at;
    }

    fn packet(&self, p: &Packet) -> &[u8] {
        let a = p.at as usize;
        self.data.get(a..a + usize::from(p.len)).unwrap_or_default()
    }
}

/// Where the decoder left off, so reading on from there doesn't start afresh.
struct DecodeState {
    path: PathBuf,
    run: usize,
    next: usize,
    dec: Decoder,
}

/// Reads the recording: sound, loudness, and what there is.
pub struct RingReader {
    root: PathBuf,
    listed: [(Option<Instant>, Vec<SegmentInfo>); 2],
    cache: VecDeque<Loaded>,
    tick: u64,
    decoding: [Option<DecodeState>; 2],
}

/// Packets decoded before the first one wanted, so the decoder has settled.
const PRE_ROLL: usize = 3;
/// The most samples read at once (two minutes at 48 kHz): longer stretches are read in pieces.
pub const MAX_READ: usize = 48_000 * 120;

impl RingReader {
    pub fn new(root: &Path) -> RingReader {
        RingReader { root: root.to_path_buf(), listed: Default::default(), cache: VecDeque::new(), tick: 0, decoding: [None, None] }
    }

    pub fn root(&self) -> &Path {
        &self.root
    }

    /// The files of a stream (listed again at most every half second).
    pub fn segments(&mut self, stream: Stream) -> Vec<SegmentInfo> {
        let i = usize::from(stream.id());
        let fresh = self.listed.get(i).and_then(|(t, _)| *t).is_some_and(|t| t.elapsed().as_millis() < 500);
        if !fresh {
            let list = segments(&self.root, stream);
            if let Some(l) = self.listed.get_mut(i) {
                *l = (Some(Instant::now()), list);
            }
        }
        self.listed.get(i).map(|(_, l)| l.clone()).unwrap_or_default()
    }

    fn load(&mut self, path: &Path) -> Option<&Loaded> {
        self.tick += 1;
        let tick = self.tick;
        if let Some(i) = self.cache.iter().position(|l| l.path == path) {
            let l = self.cache.get_mut(i)?;
            l.refresh();
            l.used = tick;
            return self.cache.get(i);
        }
        if self.cache.len() >= CACHE
            && let Some(oldest) = self.cache.iter().enumerate().min_by_key(|(_, l)| l.used).map(|(i, _)| i)
        {
            self.cache.remove(oldest);
        }
        let mut l = Loaded::open(path);
        l.used = tick;
        self.cache.push_back(l);
        self.cache.back()
    }

    /// The files that may hold sound in `[from, to)`.
    fn overlapping(&mut self, stream: Stream, from_us: u64, to_us: u64) -> Vec<SegmentInfo> {
        let all = self.segments(stream);
        let mut out: Vec<SegmentInfo> = Vec::new();
        for (i, s) in all.iter().enumerate() {
            let end = all.get(i + 1).map(|n| n.start_us.max(s.start_us)).unwrap_or(u64::MAX).min(s.end_us());
            if s.start_us < to_us && from_us < end {
                out.push(s.clone());
            }
        }
        out
    }

    /// `n` samples of a stream from `from_us`, at `rate` (48000 or 16000): silence where nothing
    /// was kept.
    pub fn read(&mut self, stream: Stream, from_us: u64, n: usize, rate: u32) -> Vec<f32> {
        let n = n.min(MAX_READ);
        let rate = if rate == 16_000 { 16_000 } else { RATE };
        let mut out = vec![0f32; n];
        if n == 0 {
            return out;
        }
        let to_us = from_us.saturating_add(samples_to_us(n as u64, rate));
        let frame = rate as usize / 50;
        let si = usize::from(stream.id());
        for seg in self.overlapping(stream, from_us, to_us) {
            let Some(loaded) = self.load(&seg.path) else { continue };
            // The runs and packets wanted, worked out before decoding (the cache is borrowed).
            let mut wanted: Vec<(usize, usize, usize, u64)> = Vec::new();
            for (ri, run) in loaded.runs.iter().enumerate() {
                if run.packets.is_empty() || run.start_us >= to_us || run.end_us() <= from_us {
                    continue;
                }
                let k0 = (from_us.saturating_sub(run.start_us) / FRAME_US) as usize;
                let k1 = (to_us.saturating_sub(run.start_us).div_ceil(FRAME_US) as usize).min(run.packets.len());
                if k0 < k1 {
                    wanted.push((ri, k0, k1, run.start_us));
                }
            }
            for (ri, k0, k1, run_start) in wanted {
                let reuse = self
                    .decoding
                    .get(si)
                    .and_then(|d| d.as_ref())
                    .is_some_and(|d| d.path == seg.path && d.run == ri && d.next == k0 && d.dec.rate() == rate);
                let mut state = match self.decoding.get_mut(si).and_then(Option::take) {
                    Some(d) if reuse => d,
                    _ => DecodeState { path: seg.path.clone(), run: ri, next: k0.saturating_sub(PRE_ROLL), dec: Decoder::new(rate) },
                };
                let Some(loaded) = self.cache.iter().find(|l| l.path == seg.path) else { continue };
                let Some(run) = loaded.runs.get(ri) else { continue };
                for k in state.next..k1 {
                    let Some(p) = run.packets.get(k) else { break };
                    let pcm = state.dec.decode(loaded.packet(p));
                    if k < k0 {
                        continue;
                    }
                    let packet_us = run_start + k as u64 * FRAME_US;
                    // Where the packet's first sample falls in `out` (it may start before it).
                    let offset = us_to_samples(packet_us, rate) as i64 - us_to_samples(from_us, rate) as i64;
                    for (j, s) in pcm.iter().take(frame).enumerate() {
                        let pos = offset + j as i64;
                        if pos < 0 {
                            continue;
                        }
                        if let Some(o) = out.get_mut(pos as usize) {
                            *o += *s;
                        }
                    }
                }
                state.next = k1;
                if let Some(d) = self.decoding.get_mut(si) {
                    *d = Some(state);
                }
            }
        }
        out
    }

    /// The loudest moment of each `step_us` from `from_us` to `to_us`, in dB ([`SILENT_DB`] where
    /// nothing was kept). Whole seconds come from the `.lvl` files of closed files when the step
    /// is a second or more.
    pub fn levels(&mut self, stream: Stream, from_us: u64, to_us: u64, step_us: u64) -> Vec<f32> {
        let step = step_us.max(FRAME_US);
        let count = (to_us.saturating_sub(from_us).div_ceil(step) as usize).min(200_000);
        let mut out = vec![0u8; count];
        let mut put = |at_us: u64, level: u8| {
            if at_us < from_us || at_us >= to_us {
                return;
            }
            if let Some(o) = out.get_mut(((at_us - from_us) / step) as usize) {
                *o = (*o).max(level);
            }
        };
        let segs = self.overlapping(stream, from_us, to_us);
        let last = segs.last().map(|s| s.path.clone());
        for seg in segs {
            let lvl = seg.path.with_extension("lvl");
            if step >= 1_000_000
                && Some(&seg.path) != last.as_ref()
                && let Ok(bytes) = std::fs::read(&lvl)
            {
                for (i, b) in bytes.iter().enumerate().take(400) {
                    if *b > 0 {
                        put(seg.start_us + i as u64 * 1_000_000, *b);
                    }
                }
                continue;
            }
            let Some(loaded) = self.load(&seg.path) else { continue };
            for run in &loaded.runs {
                if run.start_us >= to_us || run.end_us() <= from_us {
                    continue;
                }
                for (k, p) in run.packets.iter().enumerate() {
                    put(run.start_us + k as u64 * FRAME_US, p.level);
                }
            }
        }
        out.into_iter().map(|b| if b == 0 { SILENT_DB } else { byte_level(b) }).collect()
    }

    /// The stretches of `[from, to)` where a stream has sound, merged when less than `join_us`
    /// apart.
    pub fn sounds(&mut self, stream: Stream, from_us: u64, to_us: u64, join_us: u64) -> Vec<(u64, u64)> {
        let mut out: Vec<(u64, u64)> = Vec::new();
        for seg in self.overlapping(stream, from_us, to_us) {
            let Some(loaded) = self.load(&seg.path) else { continue };
            for run in &loaded.runs {
                let (a, b) = (run.start_us.max(from_us), run.end_us().min(to_us));
                if a >= b {
                    continue;
                }
                match out.last_mut() {
                    Some(last) if a <= last.1.saturating_add(join_us) => last.1 = last.1.max(b),
                    _ => out.push((a, b)),
                }
            }
        }
        out
    }

    /// The latest moment a stream has sound for (µs), if any.
    pub fn latest_us(&mut self, stream: Stream) -> Option<u64> {
        let segs = self.segments(stream);
        for seg in segs.iter().rev().take(3) {
            if let Some(l) = self.load(&seg.path)
                && let Some(end) = l.runs.iter().filter(|r| !r.packets.is_empty()).map(Run::end_us).max()
            {
                return Some(end);
            }
        }
        None
    }

    /// The earliest moment the recording holds, if anything.
    pub fn earliest_us(&mut self) -> Option<u64> {
        Stream::ALL.iter().filter_map(|s| self.segments(*s).first().map(|x| x.start_us)).min()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const T0: u64 = 1_764_000_000_000_000;

    fn tone(n: usize, from: usize, amp: f32) -> Vec<f32> {
        (from..from + n).map(|i| (i as f32 * 2.0 * std::f32::consts::PI * 440.0 / 48_000.0).sin() * amp).collect()
    }

    #[test]
    fn sound_comes_back_where_it_was_and_silence_is_not_kept() {
        let root = crate::temp_dir("ring");
        let mut w = RingWriter::new(&root);
        // One second of tone, two of silence, one of tone, in blocks of 10 ms.
        let mut at = 0usize;
        for second in 0..4 {
            for _ in 0..100 {
                let block = if second == 1 || second == 2 { vec![0.0; 480] } else { tone(480, at, 0.3) };
                w.push(Stream::Mic, T0 + samples_to_us(at as u64, RATE), &block).unwrap();
                at += 480;
            }
        }
        w.flush();
        let mut r = RingReader::new(&root);
        // Read while it's still open, as the window does.
        let sound = r.read(Stream::Mic, T0, 48_000 * 4, RATE);
        let db = |a: usize, b: usize| rms_db(&sound[a * 480..b * 480]);
        assert!(db(10, 90) > -15.0, "the first second: {}", db(10, 90));
        assert!(db(250, 290) < -90.0, "the silence: {}", db(250, 290));
        assert!(db(310, 390) > -15.0, "the last second: {}", db(310, 390));
        // The silence after a second of hangover isn't in the file: two runs.
        let runs = r.sounds(Stream::Mic, T0, T0 + 5_000_000, 0);
        assert_eq!(runs.len(), 2, "{runs:?}");
        assert!(runs[0].1 - runs[0].0 < 2_100_000);
        let levels = r.levels(Stream::Mic, T0, T0 + 4_000_000, 1_000_000);
        assert!(levels[0] > -15.0 && levels[2] == SILENT_DB && levels[3] > -15.0, "{levels:?}");
        assert!(r.latest_us(Stream::Mic).unwrap() >= T0 + 3_900_000);
        assert_eq!(r.latest_us(Stream::System), None);
        // Reading on from where it left off gives the same sound as reading at once.
        let mut r2 = RingReader::new(&root);
        let mut pieces = Vec::new();
        for k in 0..40 {
            pieces.extend(r2.read(Stream::Mic, T0 + k * 100_000, 4800, RATE));
        }
        let diff: f32 = pieces.iter().zip(&sound).map(|(a, b)| (a - b).abs()).fold(0.0, f32::max);
        assert!(diff < 1e-4, "{diff}");
        // At 16 kHz too.
        let s16 = r.read(Stream::Mic, T0, 16_000, 16_000);
        assert!(rms_db(&s16[1600..14400]) > -15.0);
        drop(w);
        // Closed: its loudness file is there and gives the same overview.
        let mut r3 = RingReader::new(&root);
        let lv = r3.levels(Stream::Mic, T0, T0 + 4_000_000, 1_000_000);
        assert_eq!(lv.len(), 4);
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn files_roll_over_every_five_minutes_and_a_jump_starts_a_new_run() {
        let root = crate::temp_dir("ring-roll");
        let mut w = RingWriter::new(&root);
        let block = tone(4800, 0, 0.2);
        // Just before the end of a file's time, and on into the next.
        let start = T0 + SEGMENT_US - 500_000;
        for k in 0..10u64 {
            w.push(Stream::System, start + k * 100_000, &block).unwrap();
        }
        // An hour later.
        w.push(Stream::System, T0 + 3_600_000_000, &block).unwrap();
        w.close();
        let segs = segments(&root, Stream::System);
        assert_eq!(segs.len(), 3, "{segs:?}");
        let mut r = RingReader::new(&root);
        let across = r.read(Stream::System, start, 48_000, RATE);
        assert!(rms_db(&across[2400..45_000]) > -20.0, "sound across the files' edge");
        let runs = r.sounds(Stream::System, T0, T0 + 4_000_000_000, 100_000);
        assert_eq!(runs.len(), 2, "the hour between is two stretches: {runs:?}");
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn damaged_and_strange_files_are_cut_short_not_a_crash() {
        let root = crate::temp_dir("ring-junk");
        let dir = root.join("mic");
        std::fs::create_dir_all(&dir).unwrap();
        // Not a recording at all.
        std::fs::write(dir.join("1764000000000.rca"), b"hello").unwrap();
        // A header, a run, and junk.
        let mut f = Vec::new();
        f.extend_from_slice(MAGIC);
        f.resize(HEADER, 0);
        f.push(TAG_RUN);
        f.extend_from_slice(&(T0 + 400_000_000).to_le_bytes());
        f.extend_from_slice(&[TAG_PACKET, 100, 3, 0, 0xfc, 0xff, 0xfe]);
        f.extend_from_slice(&[TAG_PACKET, 100, 0xff, 0xff, 1]);
        std::fs::write(dir.join("1764000400000.rca"), &f).unwrap();
        // Garbage after a good start.
        let mut g = f.clone();
        g.truncate(HEADER + 9 + 7);
        g.extend_from_slice(&[0x99; 50]);
        std::fs::write(dir.join("1764000800000.rca"), &g).unwrap();
        // Names that aren't recordings.
        std::fs::write(dir.join("notes.txt"), b"x").unwrap();
        std::fs::write(dir.join("abc.rca"), b"x").unwrap();
        let mut r = RingReader::new(&root);
        assert_eq!(r.segments(Stream::Mic).len(), 3);
        let s = r.read(Stream::Mic, T0 + 399_000_000, 48_000 * 2, RATE);
        assert_eq!(s.len(), 48_000 * 2);
        assert_eq!(r.read(Stream::Mic, T0, usize::MAX / 8, RATE).len(), MAX_READ);
        let _ = r.levels(Stream::Mic, 0, u64::MAX, 1);
        let _ = r.levels(Stream::Mic, T0, T0 + 1_000_000_000, 1_000_000);
        let _ = r.sounds(Stream::Mic, 0, u64::MAX, u64::MAX);
        let _ = r.latest_us(Stream::Mic);
        assert_eq!(r.read(Stream::Mic, u64::MAX - 10, 100, RATE).len(), 100);
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn levels_round_trip() {
        assert_eq!(level_byte(SILENT_DB), 0);
        assert_eq!(level_byte(0.0), 200);
        assert_eq!(level_byte(f32::NAN), 0);
        assert!((byte_level(level_byte(-37.0)) + 37.0).abs() < 0.6);
        assert_eq!(rms_db(&[]), SILENT_DB);
        assert!(rms_db(&[1.0; 100]).abs() < 0.01);
    }
}
