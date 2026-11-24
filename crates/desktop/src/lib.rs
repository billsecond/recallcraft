//! What RecallCraft's programs share: where things are kept ([`paths`], [`settings`]), the
//! speech model's file ([`models`]), the log ([`logging`]), one running copy each
//! ([`instance`]), starting at sign-in and the supervisor that keeps the service running
//! ([`service`]), the update check and install ([`update`]), the icon drawn in code ([`icon`]) and
//! the version line ([`version`]).
//!
//! Layer L5, desktop only, no windows.
#![deny(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::unimplemented, clippy::todo, clippy::unreachable)]

pub mod icon;
pub mod instance;
pub mod logging;
pub mod models;
pub mod paths;
pub mod service;
pub mod settings;
pub mod update;
pub mod version;
