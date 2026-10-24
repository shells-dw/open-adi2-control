//! The ADI-2 parameter list (schema.json): channels, parameters, ranges, value names per model.
//! The window gets the whole file; the Rust side only needs a few lookups.

use serde_json::Value;
use std::sync::OnceLock;

pub const SCHEMA_JSON: &str = include_str!("../schema.json");

pub fn schema() -> &'static Value {
    static S: OnceLock<Value> = OnceLock::new();
    S.get_or_init(|| serde_json::from_str(SCHEMA_JSON).expect("schema.json"))
}

#[derive(Debug, Clone)]
pub struct Channel {
    pub key: String,
    pub name: String,
    pub dac_name: Option<String>,
    pub addr: u8,
    pub eq: u8,
    pub eq_r: u8,
    pub output: bool,
}

impl Channel {
    pub fn label(&self, model: Option<&str>) -> String {
        if model == Some("dac") {
            if let Some(n) = &self.dac_name {
                return n.clone();
            }
        }
        self.name.clone()
    }
}

pub fn channels() -> &'static [Channel] {
    static C: OnceLock<Vec<Channel>> = OnceLock::new();
    C.get_or_init(|| {
        schema()["channels"]
            .as_array()
            .unwrap()
            .iter()
            .map(|c| Channel {
                key: c["key"].as_str().unwrap().into(),
                name: c["name"].as_str().unwrap().into(),
                dac_name: c["dacName"].as_str().map(Into::into),
                addr: c["addr"].as_u64().unwrap() as u8,
                eq: c["eq"].as_u64().unwrap() as u8,
                eq_r: c["eqR"].as_u64().unwrap() as u8,
                output: c["kind"] == "output",
            })
            .collect()
    })
}

pub fn channel(key: &str) -> Option<&'static Channel> {
    channels().iter().find(|c| c.key == key)
}

pub fn outputs() -> impl Iterator<Item = &'static Channel> {
    channels().iter().filter(|c| c.output)
}

/// A parameter of a section ("output", "input", "eq", "device") by id.
pub fn param(section: &str, id: &str) -> Option<&'static Value> {
    schema()["params"][section].as_array()?.iter().find(|p| p["id"] == id)
}

pub fn idx(section: &str, id: &str) -> u8 {
    param(section, id).and_then(|p| p["idx"].as_u64()).expect("schema index") as u8
}

/// Range of a level parameter as protocol integers.
pub fn raw_range(p: &Value) -> (i32, i32) {
    let scale = p["scale"].as_f64().unwrap_or(1.0);
    let lo = p["min"].as_f64().map(|v| (v / scale).round() as i32).unwrap_or(-2048);
    let hi = p["max"].as_f64().map(|v| (v / scale).round() as i32).unwrap_or(2047);
    (lo, hi)
}

pub fn available(p: &Value, model: &str) -> bool {
    if let Some(only) = p["only"].as_array() {
        if !only.iter().any(|m| m == model) {
            return false;
        }
    }
    if let Some(not) = p["not"].as_array() {
        if not.iter().any(|m| m == model) {
            return false;
        }
    }
    true
}

pub fn model_name(model: &str) -> String {
    schema()["modelNames"][model].as_str().unwrap_or("ADI-2").to_string()
}

pub mod ids {
    /// output parameter indices used outside the window
    pub const VOL: u8 = 12;
    pub const MUTE: u8 = 15;
    pub const DIM: u8 = 16;
    /// EQ address indices
    pub const EQ_ENABLE: u8 = 2;
    pub const LOAD_BT: u8 = 27;
    pub const EQ_PRESET: u8 = 28;
    pub const DUAL_EQ_OUT: u8 = 11;
    pub const DUAL_EQ_IN: u8 = 8;
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn lookups() {
        assert_eq!(channel("ph34").unwrap().addr, 9);
        assert_eq!(idx("output", "vol"), ids::VOL);
        assert_eq!(idx("output", "mute"), ids::MUTE);
        assert_eq!(idx("eq", "eqpreset"), ids::EQ_PRESET);
        assert_eq!(raw_range(param("output", "vol").unwrap()), (-1145, 60));
        assert!(!available(param("device", "basic").unwrap(), "dac"));
    }
}
