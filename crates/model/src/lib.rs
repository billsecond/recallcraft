//! RecallCraft's plain data: the two sound streams, who said a line, a call and its
//! transcript, the settings, and how times are shown. No files, no devices: the other crates
//! do the I/O.
//!
//! Layer L1.
#![deny(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::unimplemented, clippy::todo, clippy::unreachable)]

pub mod call;
pub mod settings;
pub mod speaker;
pub mod time;

pub use call::{Call, Line};
pub use settings::{AudioExport, ExportSettings, MicMode, RecordMode, Settings, TranscriptFormat};
pub use speaker::{Speaker, Stream};

/// RecallCraft's version.
pub const VERSION: &str = env!("CARGO_PKG_VERSION");
