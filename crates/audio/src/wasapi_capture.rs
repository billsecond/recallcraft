//! Windows: the microphone and the computer's sound through WASAPI.
//!
//! The computer's sound is a process loopback that leaves out RecallCraft's own processes
//! (Windows 10 2004 and later): everything every other program plays, on whichever speaker,
//! without RecallCraft's replays. Where that isn't there, the default speaker's loopback is used
//! instead. Both are asked for 48 kHz stereo floats (Windows converts) and made mono here.

use std::collections::VecDeque;
use std::panic::{AssertUnwindSafe, catch_unwind};
use std::sync::atomic::Ordering;
use std::sync::mpsc::Sender;

use wasapi::{AudioClient, DeviceEnumerator, Direction, Role, SampleType, StreamMode, WaveFormat, initialize_mta};

use crate::capture::{Mic, Ready, Shared, Source};
use crate::devices::Device;
use crate::place::Placer;
use crate::{Block, RATE};

const CHANNELS: usize = 2;

fn com() {
    // On a thread that already chose the other kind (a window's), this fails harmlessly: the
    // calls below work either way.
    let _ = initialize_mta();
}

pub fn microphones() -> Vec<Device> {
    com();
    let Ok(e) = DeviceEnumerator::new() else { return Vec::new() };
    let Ok(all) = e.get_device_collection(Direction::Capture) else { return Vec::new() };
    (&all).into_iter().flatten().filter_map(|d| Some(Device { id: d.get_id().ok()?, name: d.get_friendlyname().ok()? })).collect()
}

pub fn default_microphone(communications: bool) -> Option<Device> {
    com();
    let e = DeviceEnumerator::new().ok()?;
    let role = if communications { Role::Communications } else { Role::Console };
    let d = e.get_default_device_for_role(Direction::Capture, role).ok()?;
    Some(Device { id: d.get_id().ok()?, name: d.get_friendlyname().ok()? })
}

fn open(source: &Source) -> Result<(AudioClient, String, String), String> {
    let e = DeviceEnumerator::new().map_err(|e| e.to_string())?;
    match source {
        Source::System => {
            match catch_unwind(|| AudioClient::new_application_loopback_client(std::process::id(), false)) {
                Ok(Ok(c)) => return Ok((c, "Computer sound".to_string(), String::new())),
                Ok(Err(err)) => log::warn!("the computer's sound without RecallCraft's isn't there ({err}); recording the speaker's instead"),
                Err(_) => log::warn!("the process loopback failed unexpectedly; recording the speaker's instead"),
            }
            let d = e.get_default_device(Direction::Render).map_err(|e| format!("no speaker: {e}"))?;
            let name = d.get_friendlyname().unwrap_or_else(|_| "Speaker".to_string());
            let id = d.get_id().unwrap_or_default();
            Ok((d.get_iaudioclient().map_err(|e| e.to_string())?, name, id))
        }
        Source::Mic(m) => {
            let d = match m {
                Mic::Default => e.get_default_device(Direction::Capture),
                Mic::Communications => e.get_default_device_for_role(Direction::Capture, Role::Communications),
                Mic::Id(id) => e.get_device(id).or_else(|_| e.get_default_device(Direction::Capture)),
                Mic::Named(name) => e
                    .get_device_collection(Direction::Capture)
                    .and_then(|all| all.get_device_with_name(name))
                    .or_else(|_| e.get_default_device(Direction::Capture)),
            }
            .map_err(|e| {
                // "Element not found": Windows has no microphone at all (over Remote Desktop, the
                // Remote Desktop app has to share the one at the other end).
                let text = e.to_string();
                if text.contains("0x80070490") { "no microphone found".to_string() } else { format!("no microphone: {text}") }
            })?;
            let name = d.get_friendlyname().unwrap_or_else(|_| "Microphone".to_string());
            let id = d.get_id().unwrap_or_default();
            Ok((d.get_iaudioclient().map_err(|e| e.to_string())?, name, id))
        }
    }
}

pub(crate) fn run(source: Source, tx: Sender<Block>, ready: Ready, shared: &Shared) {
    com();
    let r = catch_unwind(AssertUnwindSafe(|| capture(&source, &tx, &ready, shared)));
    match r {
        Ok(Ok(())) => {}
        Ok(Err(e)) => {
            log::warn!("{}: {e}", source.stream().label());
            let _ = ready.try_send(Err(e));
        }
        Err(_) => {
            log::error!("{}: the device failed unexpectedly", source.stream().label());
            let _ = ready.try_send(Err("the device failed unexpectedly".to_string()));
        }
    }
}

fn now_us() -> u64 {
    recallcraft_model::time::now_us()
}

fn capture(source: &Source, tx: &Sender<Block>, ready: &Ready, shared: &Shared) -> Result<(), String> {
    let (mut client, name, id) = open(source)?;
    let format = WaveFormat::new(32, 32, SampleType::Float, RATE as usize, CHANNELS, None);
    let mode = StreamMode::EventsShared { autoconvert: true, buffer_duration_hns: 200_000 };
    client.initialize_client(&format, Direction::Capture, mode, None).map_err(|e| format!("{name}: {e}"))?;
    let event = client.set_get_eventhandle().map_err(|e| e.to_string())?;
    let reader = client.get_audiocaptureclient().map_err(|e| e.to_string())?;
    client.start_stream().map_err(|e| format!("{name}: {e}"))?;
    log::info!("recording {} ({name})", source.stream().label());
    let _ = ready.send(Ok((name.clone(), id)));
    let stream = source.stream();
    let mut placer = Placer::new();
    let mut bytes: VecDeque<u8> = VecDeque::new();
    let frame_bytes = 4 * CHANNELS;
    let mut quiet_waits = 0u32;
    while !shared.stop.load(Ordering::SeqCst) {
        if event.wait_for_event(200).is_err() {
            // Nothing came: a speaker's loopback is quiet while nothing plays.
            quiet_waits += 1;
            if quiet_waits > 5 {
                placer.reset();
            }
            continue;
        }
        quiet_waits = 0;
        loop {
            match reader.get_next_packet_size() {
                Ok(Some(0)) | Ok(None) => break,
                Ok(Some(_)) => {}
                Err(e) => return Err(format!("{name}: {e}")),
            }
            reader.read_from_device_to_deque(&mut bytes).map_err(|e| format!("{name}: {e}"))?;
        }
        let whole = bytes.len() / frame_bytes * frame_bytes;
        if whole == 0 {
            continue;
        }
        let raw: Vec<u8> = bytes.drain(..whole).collect();
        let floats: Vec<f32> = raw.as_chunks::<4>().0.iter().map(|b| f32::from_le_bytes(*b)).collect();
        let mono = crate::to_mono(&floats, CHANNELS);
        shared.set_level(crate::peak(&mono));
        shared.heard();
        let start_us = placer.place(mono.len() as u64, now_us());
        if tx.send(Block { stream, start_us, samples: mono }).is_err() {
            break;
        }
    }
    let _ = client.stop_stream();
    Ok(())
}
