//! What the app knows about the unit, shared by the USB engine, the ADI-2-Pi client and the UI.

use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct Problem {
    pub code: String,
    pub text: String,
}

impl Problem {
    pub fn new(code: &str, text: impl Into<String>) -> Self {
        Problem { code: code.into(), text: text.into() }
    }
}

#[derive(Debug, Clone, Serialize, Default, PartialEq)]
pub struct DeviceInfo {
    /// MIDI port (USB) or bridge (ADI-2-Pi) is there
    pub connected: bool,
    /// all settings have been read
    pub ready: bool,
    /// the unit sends its status messages (MIDI Control on)
    pub answering: bool,
    pub id: Option<u8>,
    /// "dac" | "pro" | "se" | "ex" (ADI-2/4 Pro EX: device ID of the SE, device revision 1)
    pub model: Option<String>,
    /// port or bridge the unit is reached through
    pub port: Option<String>,
    pub problem: Option<Problem>,
    /// ADI-2-Pi with "key needed for changes only" and no key: values can be seen, not changed
    pub read_only: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default, PartialEq)]
#[serde(default)]
pub struct Preset {
    pub n: u8,
    pub name: String,
    /// None = not read yet
    pub empty: Option<bool>,
    pub dual: bool,
    /// EQ index → protocol integer (frequencies in Hz)
    #[serde(rename = "L")]
    pub l: BTreeMap<u8, i32>,
    #[serde(rename = "R")]
    pub r: BTreeMap<u8, i32>,
}

pub fn empty_presets() -> Vec<Preset> {
    (1..=20).map(|n| Preset { n, ..Default::default() }).collect()
}

#[derive(Debug, Clone, Copy, Serialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum Origin {
    /// changed on the unit (knob, keys, IR remote) or read back from it
    Device,
    /// changed by this app
    App,
}

/// Updates from a backend.
#[derive(Debug, Clone)]
pub enum Update {
    Device(DeviceInfo),
    /// everything known so far was replaced (new connection)
    Reset,
    Params(Vec<(String, i32)>, Origin),
    Setups(Vec<String>),
    Presets(Vec<Preset>),
    Status(crate::protocol::Status),
}

/// Commands to a backend. Values are protocol integers, keys "addr.idx".
#[derive(Debug, Clone)]
pub enum Command {
    Set(Vec<(String, i32)>),
    LoadSetup(u8),
    SaveSetup(u8, Option<String>),
    RenameSetup(u8, String),
    LoadFactory,
    StorePreset { n: u8, name: Option<String>, l: BTreeMap<u8, i32>, r: BTreeMap<u8, i32>, dual: bool },
    RenamePreset(u8, String),
    Refresh,
}

pub fn key(addr: u8, idx: u8) -> String {
    format!("{addr}.{idx}")
}

pub fn split_key(k: &str) -> Option<(u8, u8)> {
    let (a, i) = k.split_once('.')?;
    Some((a.parse().ok()?, i.parse().ok()?))
}

/// A key that can be sent: address 0–12 (13/14 are preset data), index within the container
/// (5 bits on channels, 6 bits on the device address).
pub fn valid_key(k: &str) -> bool {
    match split_key(k) {
        Some((12, i)) => i < 64,
        Some((a, i)) => a < 12 && i < 32,
        None => false,
    }
}
