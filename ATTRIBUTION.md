# Attribution

Every file RecallCraft ships that isn't its own source code, and where it came from.

| File | Author | Source | Licence |
|---|---|---|---|
| `assets/models/whisper-base.en-q8_0.gguf` | OpenAI (Whisper base.en); weights in 8 bits by the ScreenGrabCraft contributors | https://huggingface.co/openai/whisper-base.en, converted by `crates/speech/examples/convert_model.rs` | MIT (`assets/models/LICENSE-WHISPER.txt`) |
| `assets/app-icon/recallcraft.ico` | RecallCraft contributors | drawn in code, `crates/desktop/src/icon.rs` (`cargo xtask icon`) | MIT OR Apache-2.0 |

## Code from ScreenGrabCraft

RecallCraft's sibling, ScreenGrabCraft (https://github.com/billsecond/screengrabcraft, MIT OR
Apache-2.0, the same licence), is where these come from, adapted:

- `crates/speech/src/{gguf,model,mel,decode}.rs` and `examples/convert_model.rs`: the Whisper
  engine on candle, unchanged.
- `crates/desktop/src/{instance,logging}.rs`, much of `service.rs`, and `apps/recallcraft/src/graphics.rs`.
- The microphone capture outside Windows (`crates/audio/src/cpal_capture.rs`), and the echo rules.

## Libraries

The crates RecallCraft builds with are listed in `Cargo.lock`, each under its own licence (MIT,
Apache-2.0, BSD or similar): among them candle (Whisper), rusty-opus (Opus, BSD-3-Clause), wasapi
and cpal (sound devices), egui/eframe (the window), tray-icon and global-hotkey.
