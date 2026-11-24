//! Knowing when a call is on, without being told.
//!
//! - [`apps`]: the calling apps RecallCraft knows (meeting apps, chat apps' calls, softphones,
//!   browsers) by their program file.
//! - [`detector`]: a call starts when a calling app has had a microphone open for a couple of
//!   seconds, and ends when it has let go of it for a while (a moment's let-go, rejoining or
//!   changing device, doesn't end it).
//! - [`titles`]: the meeting's title from the app's windows (the window that appeared when the
//!   call started), and the other person's name when the title says it ("Call with …").
//! - [`probe`]: Windows: which programs have a microphone open just now (and which one), and
//!   every program's windows.
//! - [`roster`]: who's on the call and who's speaking, from what the meeting app's window tells
//!   screen readers; [`uia`] reads it (Windows' UI Automation).
//!
//! Layer L3. Everything but [`probe`] and [`uia`] is plain logic, tested without devices.
#![deny(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::unimplemented, clippy::todo, clippy::unreachable)]

pub mod apps;
pub mod detector;
pub mod probe;
pub mod roster;
pub mod titles;
pub mod uia;

pub use apps::{AppKind, CallApp};
pub use detector::{Capturer, Detector, Event, Observation, Rules};
