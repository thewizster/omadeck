//! Device discovery helpers on top of the `elgato-streamdeck` crate.

use serde::{Deserialize, Serialize};

pub use elgato_streamdeck::info::Kind;
pub use elgato_streamdeck::{
    DeviceStateReader, DeviceStateUpdate, StreamDeck, list_devices, new_hidapi, refresh_device_list,
};
pub use hidapi::HidApi;

/// What the configurator needs to know to draw a deck.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DeckInfo {
    pub model: String,
    pub serial: String,
    pub rows: u8,
    pub cols: u8,
    pub key_size: u32,
}

impl DeckInfo {
    pub fn new(kind: Kind, serial: &str) -> Self {
        let fmt = kind.key_image_format();
        Self {
            model: model_name(kind).into(),
            serial: serial.into(),
            rows: kind.row_count(),
            cols: kind.column_count(),
            key_size: fmt.size.0 as u32,
        }
    }

    pub fn key_count(&self) -> u8 {
        self.rows * self.cols
    }

    /// Layout used when no deck is around: the classic 15-key Stream Deck.
    pub fn fallback() -> Self {
        Self { model: "Stream Deck".into(), serial: String::new(), rows: 3, cols: 5, key_size: 72 }
    }
}

pub fn model_name(kind: Kind) -> &'static str {
    match kind {
        Kind::Original => "Stream Deck",
        Kind::OriginalV2 => "Stream Deck (V2)",
        Kind::Mini | Kind::MiniMk2 | Kind::MiniMk2Module => "Stream Deck Mini",
        Kind::MiniDiscord => "Stream Deck Mini (Discord)",
        Kind::Xl | Kind::XlV2 | Kind::XlV2Module => "Stream Deck XL",
        Kind::Mk2 | Kind::Mk2Module => "Stream Deck MK.2",
        Kind::Mk2Scissor => "Stream Deck MK.2 (Scissor)",
        Kind::Neo => "Stream Deck Neo",
        Kind::Pedal => "Stream Deck Pedal",
        Kind::Plus => "Stream Deck +",
        Kind::PlusXl => "Stream Deck + XL",
    }
}

/// Enumerate attached decks without opening them (safe while the daemon owns the device).
pub fn detect() -> Vec<DeckInfo> {
    match new_hidapi() {
        Ok(hid) => list_devices(&hid).into_iter().map(|(k, s)| DeckInfo::new(k, &s)).collect(),
        Err(e) => {
            log::warn!("hidapi: {e}");
            Vec::new()
        }
    }
}
