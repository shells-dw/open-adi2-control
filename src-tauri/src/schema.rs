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

/// Value names of a select parameter for a model and channel (per-model and line/phones lists).
pub fn value_names(p: &Value, model: &str, ch: Option<&Channel>) -> Option<Vec<String>> {
    let mut n = p.get("names")?;
    if n.is_object() && (n.get("line").is_some() || n.get("phones").is_some()) {
        n = if ch.is_some_and(|c| c.key == "line") { &n["line"] } else { &n["phones"] };
    }
    if n.is_object() {
        n = n.get(model).or_else(|| n.get("pro")).or_else(|| n.as_object().and_then(|o| o.values().next()))?;
    }
    Some(n.as_array()?.iter().filter_map(|v| v.as_str().map(String::from)).collect())
}

/// A parameter by channel key ("line", "ph12", "ph34", "in", "device") and id.
pub struct Resolved {
    pub addr: u8,
    pub idx: u8,
    pub param: &'static Value,
    pub channel: Option<&'static Channel>,
}

pub fn resolve(ch: &str, id: &str, side: &str) -> Option<Resolved> {
    if ch == "device" {
        let p = param("device", id)?;
        return Some(Resolved { addr: 12, idx: p["idx"].as_u64()? as u8, param: p, channel: None });
    }
    let c = channel(ch)?;
    if let Some(p) = param("eq", id) {
        let addr = if side.eq_ignore_ascii_case("r") { c.eq_r } else { c.eq };
        return Some(Resolved { addr, idx: p["idx"].as_u64()? as u8, param: p, channel: Some(c) });
    }
    let p = param(if c.output { "output" } else { "input" }, id)?;
    Some(Resolved { addr: c.addr, idx: p["idx"].as_u64()? as u8, param: p, channel: Some(c) })
}

/// Display value of a protocol integer (dB, Hz, Q … as in the window).
pub fn to_display(p: &Value, raw: i32) -> f64 {
    if p["unit"] == "Hz" {
        return raw as f64;
    }
    // rounded so that 7 × 0.1 prints as 0.7
    (raw as f64 * p["scale"].as_f64().unwrap_or(1.0) * 1e4).round() / 1e4
}

/// Protocol integer of a display value, clamped to the parameter's range.
pub fn from_display(p: &Value, v: f64) -> i32 {
    let raw = if p["unit"] == "Hz" { v.round() as i32 } else { (v / p["scale"].as_f64().unwrap_or(1.0)).round() as i32 };
    let (lo, hi) = raw_range(p);
    raw.clamp(lo, hi)
}

/// "+1.5" / "-2" → relative, "on"/"off"/"toggle" for toggles, a name or index for selects.
pub fn parse_value(p: &Value, model: &str, ch: Option<&Channel>, text: &str, current: Option<i32>) -> Result<i32, String> {
    let t = text.trim();
    let id = p["id"].as_str().unwrap_or("value");
    match p["type"].as_str().unwrap_or("level") {
        "toggle" => match t.to_ascii_lowercase().as_str() {
            "1" | "on" | "true" => Ok(1),
            "0" | "off" | "false" => Ok(0),
            "toggle" => Ok(if current.unwrap_or(0) != 0 { 0 } else { 1 }),
            _ => Err(format!("{}: on, off or toggle", id)),
        },
        "select" => {
            let names = value_names(p, model, ch).unwrap_or_default();
            if let Ok(i) = t.parse::<i32>() {
                if names.is_empty() || (0..names.len() as i32).contains(&i) {
                    return Ok(i);
                }
            }
            if let Some(i) = names.iter().position(|n| n.eq_ignore_ascii_case(t)) {
                return Ok(i as i32);
            }
            if t.eq_ignore_ascii_case("next") || t.eq_ignore_ascii_case("prev") {
                let n = names.len().max(1) as i32;
                let d = if t.eq_ignore_ascii_case("next") { 1 } else { -1 };
                return Ok((current.unwrap_or(0) + d).rem_euclid(n));
            }
            Err(format!("{}: one of {}", id, names.join(", ")))
        }
        _ => {
            // "+1" and "-=1" are relative; "-1" is the absolute value −1
            let (rel, num) = if let Some(r) = t.strip_prefix("+=") {
                (true, r.to_string())
            } else if let Some(r) = t.strip_prefix("-=") {
                (true, format!("-{r}"))
            } else if let Some(r) = t.strip_prefix('+') {
                (true, r.to_string())
            } else {
                (false, t.to_string())
            };
            let n: f64 = num.replace(',', ".").parse().map_err(|_| format!("{}: not a number", id))?;
            if rel {
                let cur = current.ok_or("value not known yet")?;
                return Ok(from_display(p, to_display(p, cur) + n));
            }
            Ok(from_display(p, n))
        }
    }
}

#[cfg(test)]
mod cli_tests {
    use super::*;

    #[test]
    fn values_from_the_command_line() {
        let vol = resolve("line", "vol", "L").unwrap();
        assert_eq!(vol.addr, 3);
        assert_eq!(parse_value(vol.param, "pro", vol.channel, "-20.5", None), Ok(-205));
        assert_eq!(parse_value(vol.param, "pro", vol.channel, "+2", Some(-205)), Ok(-185));
        assert_eq!(parse_value(vol.param, "pro", vol.channel, "-=1", Some(-205)), Ok(-215));
        assert!(parse_value(vol.param, "pro", vol.channel, "+2", None).is_err(), "relative needs the current value");
        assert_eq!(parse_value(vol.param, "pro", vol.channel, "99", None), Ok(from_display(vol.param, 6.0)), "clamped to the range");
        let mute = resolve("ph12", "mute", "L").unwrap();
        assert_eq!(parse_value(mute.param, "pro", mute.channel, "toggle", Some(1)), Ok(0));
        assert_eq!(parse_value(mute.param, "pro", mute.channel, "on", None), Ok(1));
        assert_eq!(parse_value(mute.param, "pro", mute.channel, "maybe", None), Err("mute: on, off or toggle".into()));
        let g = resolve("line", "b1g", "R").unwrap();
        assert_eq!(g.addr, 5, "right EQ address");
        assert_eq!(parse_value(g.param, "pro", g.channel, "2.5", None), Ok(5));
        let ty = resolve("line", "b1type", "L").unwrap();
        assert_eq!(parse_value(ty.param, "pro", ty.channel, "peak", None), Ok(0));
        assert_eq!(parse_value(ty.param, "pro", ty.channel, "next", Some(3)), Ok(0), "wraps around");
        assert!(resolve("line", "nothing", "L").is_none());
        assert_eq!(resolve("device", "srate", "L").map(|r| r.addr), Some(12));
    }
}
