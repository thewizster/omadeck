//! Shared core of omadeck: configuration, Omarchy theme integration, key rendering,
//! action execution, device discovery and the daemon event protocol.

pub mod action;
pub mod config;
pub mod device;
pub mod ipc;
pub mod paths;
pub mod render;
pub mod theme;

pub use config::{Action, Config, KeyConfig, Style};
pub use device::DeckInfo;
pub use render::Renderer;
pub use theme::{Rgb, Theme};
