//! GamingZones core: pure Rust, no `unsafe`, no Win32. Everything here is unit-tested.
//!
//! All coordinates are physical pixels.
#![forbid(unsafe_code)]

pub mod config;
pub mod edit;
pub mod gaming;
pub mod geometry;
pub mod hotkey;
pub mod ipc;
pub mod layout;
pub mod store;

pub use config::{Color, DisabledZones, GamingFit, LogLevel, Settings};
pub use geometry::{Insets, Rect};
pub use hotkey::{Hotkey, Modifiers};
