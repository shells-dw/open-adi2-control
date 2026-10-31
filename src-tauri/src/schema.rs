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

/// Keys a model's entries are looked up under: the ADI-2/4 Pro EX ("ex") uses its own entry
/// where there is one, otherwise the SE's (same device ID 0x73).
pub fn model_keys(model: &str) -> &'static [&'static str] {
    match model {
        "ex" => &["ex", "se"],
        "se" => &["se"],
        "dac" => &["dac"],
        _ => &["pro"],
    }
}

pub fn available(p: &Value, model: &str) -> bool {
    let keys = model_keys(model);
    let named = |list: &Value| list.as_array().is_some_and(|l| l.iter().any(|m| keys.iter().any(|k| m == k)));
    if p["only"].is_array() && !named(&p["only"]) {
        return false;
    }
    if p["not"].is_array() && named(&p["not"]) {
        return false;
    }
    true
}

/// A parameter that exists only on some channels (`"channels": ["ph34"]`, e.g. Rear TRS Source).
pub fn on_channel(p: &Value, ch: &str) -> bool {
    match p["channels"].as_array() {
        Some(list) => list.iter().any(|c| c == ch),
        None => true,
    }
}

/// Model of device ID 0x73 after its status message: device revision 1 = ADI-2/4 Pro EX.
pub fn model_with_revision(model: &str, revision: u8) -> &str {
    match (model, revision) {
        ("se" | "ex", 1) => "ex",
        ("se" | "ex", _) => "se",
        (m, _) => m,
    }
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
/// `dac_mode`: Basic Mode is DAC (ADI-2 Pro, ADI-2/4), which has its own Source list.
pub fn value_names(p: &Value, model: &str, ch: Option<&Channel>, dac_mode: bool) -> Option<Vec<String>> {
    let mut n = p.get("names")?;
    if n.is_object() && (n.get("line").is_some() || n.get("phones").is_some()) {
        n = if ch.is_some_and(|c| c.key == "line") { &n["line"] } else { &n["phones"] };
    }
    if n.is_object() {
        let dac_list = if dac_mode && model != "dac" { n.get("dacmode") } else { None };
        n = dac_list
            .or_else(|| model_keys(model).iter().find_map(|k| n.get(*k)))
            .or_else(|| n.get("pro"))
            .or_else(|| n.as_object().and_then(|o| o.values().next()))?;
    }
    Some(n.as_array()?.iter().filter_map(|v| v.as_str().map(String::from)).collect())
}

/// Protocol value of a select's first name (0, or e.g. -6 for the ADI-2/4 Pro EX input levels).
pub fn value_base(p: &Value, model: &str) -> i32 {
    model_keys(model).iter().find_map(|k| p["base"][*k].as_i64()).unwrap_or(0) as i32
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

/// A value as text: "on"/"off", the name of a select value, or the display number.
/// A select value the list doesn't name (not in RME's table, e.g. from a newer model) is `#n`,
/// never a neighbouring name.
pub fn format_value(p: &Value, model: &str, ch: Option<&Channel>, dac_mode: bool, raw: i32) -> String {
    match p["type"].as_str().unwrap_or("level") {
        "toggle" => (if raw != 0 { "on" } else { "off" }).to_string(),
        "action" => "sent".to_string(),
        "select" => value_names(p, model, ch, dac_mode)
            .and_then(|n| usize::try_from(raw as i64 - value_base(p, model) as i64).ok().and_then(|i| n.get(i).cloned()))
            .unwrap_or_else(|| format!("#{raw}")),
        _ => {
            let d = to_display(p, raw);
            let dec = p["decimals"].as_u64().unwrap_or(0) as usize;
            format!("{d:.dec$}")
        }
    }
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
pub fn parse_value(p: &Value, model: &str, ch: Option<&Channel>, dac_mode: bool, text: &str, current: Option<i32>) -> Result<i32, String> {
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
            let names = value_names(p, model, ch, dac_mode).unwrap_or_default();
            // protocol value of the first name
            let base = value_base(p, model);
            // "#n": the raw number, as `get` prints a value the list doesn't name
            if let Some(n) = t.strip_prefix('#').and_then(|n| n.parse::<i32>().ok()) {
                return Ok(n);
            }
            if let Ok(i) = t.parse::<i32>() {
                if names.is_empty() || (base..base + names.len() as i32).contains(&i) {
                    return Ok(i);
                }
            }
            if let Some(i) = names.iter().position(|n| n.eq_ignore_ascii_case(t)) {
                return Ok(i as i32 + base);
            }
            if t.eq_ignore_ascii_case("next") || t.eq_ignore_ascii_case("prev") {
                let n = names.len().max(1) as i32;
                let d = if t.eq_ignore_ascii_case("next") { 1 } else { -1 };
                return Ok((current.unwrap_or(base) - base + d).rem_euclid(n) + base);
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
            let n: f64 = num.replace(',', ".").parse().ok().filter(|n: &f64| n.is_finite()).ok_or_else(|| format!("{}: not a number", id))?;
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
        assert_eq!(parse_value(vol.param, "pro", vol.channel, false, "-20.5", None), Ok(-205));
        assert_eq!(parse_value(vol.param, "pro", vol.channel, false, "+2", Some(-205)), Ok(-185));
        assert_eq!(parse_value(vol.param, "pro", vol.channel, false, "-=1", Some(-205)), Ok(-215));
        assert!(parse_value(vol.param, "pro", vol.channel, false, "+2", None).is_err(), "relative needs the current value");
        assert_eq!(parse_value(vol.param, "pro", vol.channel, false, "99", None), Ok(from_display(vol.param, 6.0)), "clamped to the range");
        let mute = resolve("ph12", "mute", "L").unwrap();
        assert_eq!(parse_value(mute.param, "pro", mute.channel, false, "toggle", Some(1)), Ok(0));
        assert_eq!(parse_value(mute.param, "pro", mute.channel, false, "on", None), Ok(1));
        assert_eq!(parse_value(mute.param, "pro", mute.channel, false, "maybe", None), Err("mute: on, off or toggle".into()));
        assert!(parse_value(vol.param, "pro", vol.channel, false, "nan", None).is_err());
        assert!(parse_value(vol.param, "pro", vol.channel, false, "+inf", Some(0)).is_err());
        let ir = resolve("in", "inref", "L").unwrap();
        assert_eq!(format_value(ir.param, "ex", ir.channel, false, i32::MAX), format!("#{}", i32::MAX));
        let g = resolve("line", "b1g", "R").unwrap();
        assert_eq!(g.addr, 5, "right EQ address");
        assert_eq!(parse_value(g.param, "pro", g.channel, false, "2.5", None), Ok(5));
        let ty = resolve("line", "b1type", "L").unwrap();
        assert_eq!(parse_value(ty.param, "pro", ty.channel, false, "peak", None), Ok(0));
        assert_eq!(parse_value(ty.param, "pro", ty.channel, false, "next", Some(3)), Ok(0), "wraps around");
        assert!(resolve("line", "nothing", "L").is_none());
        assert_eq!(parse_value(ty.param, "pro", ty.channel, false, "#7", None), Ok(7), "raw number as printed by get");
        assert_eq!(resolve("device", "srate", "L").map(|r| r.addr), Some(12));
    }
}

#[cfg(test)]
mod value_list_tests {
    use super::*;

    fn label(model: &str, ch: &str, id: &str, raw: i32) -> String {
        let r = resolve(ch, id, "L").unwrap();
        format_value(r.param, model, r.channel, false, raw)
    }

    #[test]
    fn line_levels_per_model() {
        // ADI-2/4 Pro SE (and Pro EX): five levels, ascending
        let se: Vec<String> = (0..5).map(|i| label("se", "line", "ref", i)).collect();
        assert_eq!(se, ["+1 dBu", "+7 dBu", "+13 dBu", "+19 dBu", "+24 dBu"]);
        assert_eq!(label("pro", "line", "ref", 0), "+4 dBu");
        assert_eq!(label("pro", "line", "ref", 3), "+24 dBu");
        assert_eq!(label("dac", "line", "ref", 0), "-5 dBu");
        // the phones list on phones outputs
        assert_eq!(label("se", "ph12", "ref", 0), "IEM");
        assert_eq!(label("pro", "ph34", "ref", 1), "Hi-Power");
        assert_eq!(model_name("se"), "ADI-2/4 Pro SE");
        assert_eq!(model_name("ex"), "ADI-2/4 Pro EX");
    }

    #[test]
    fn unknown_values_are_numbers() {
        // 2/4 Pro EX values missing from RME's table: shown as #n
        assert_eq!(label("pro", "line", "ref", 4), "#4");
        assert_eq!(label("se", "in", "riaa", 6), "#6");
        assert_eq!(label("se", "ph34", "src", 6), "#6");
        assert_eq!(label("se", "line", "ref", -1), "#-1");
        assert_eq!(label("se", "line", "mute", 1), "on");
    }

    #[test]
    fn adi24_pro_ex() {
        assert_eq!(model_with_revision("se", 1), "ex");
        assert_eq!(model_with_revision("se", 0), "se");
        assert_eq!(model_with_revision("pro", 1), "pro");
        // input levels -35 … +24 dBu are MIDI values -6 … 4
        assert_eq!(label("ex", "in", "inref", -6), "-35 dBu");
        assert_eq!(label("ex", "in", "inref", 0), "+1 dBu");
        assert_eq!(label("ex", "in", "inref", 4), "+24 dBu");
        assert_eq!(label("se", "in", "inref", 0), "+1 dBu");
        let r = resolve("in", "inref", "L").unwrap();
        assert_eq!(parse_value(r.param, "ex", r.channel, false, "-35 dBu", None), Ok(-6));
        assert_eq!(parse_value(r.param, "ex", r.channel, false, "next", Some(4)), Ok(-6), "wraps to the lowest");
        // 0 dB is RIAA value 1 on the EX only
        assert_eq!(label("ex", "in", "riaa", 1), "0 dB");
        assert_eq!(label("se", "in", "riaa", 1), "+14 dB");
        assert_eq!(label("ex", "in", "dcfilt", 0), "ADC");
        assert_eq!(label("se", "in", "dcfilt", 0), "RME");
        assert_eq!(label("ex", "ph34", "src", 6), "ADAT 3/4");
        assert_eq!(label("se", "ph34", "src", 6), "#6");
        // the EX takes the SE's lists where it has none of its own
        assert_eq!(label("ex", "line", "dafilt", 5), "Brickwall");
        assert_eq!(label("ex", "line", "ref", 4), "+24 dBu");
        let imp = param("input", "balimp").unwrap();
        assert!(available(imp, "ex") && !available(imp, "se"));
        assert!(available(param("input", "riaa").unwrap(), "ex"), "SE parameters apply to the EX");
        // Rear TRS Source belongs to Phones 3/4 (RME's dump: index 19 on address 9 only)
        let rt = param("output", "reartrs").unwrap();
        assert!(on_channel(rt, "ph34") && !on_channel(rt, "line") && on_channel(param("output", "vol").unwrap(), "line"));
        assert!(!available(param("device", "dsddirect").unwrap(), "ex"));
    }

    #[test]
    fn source_in_dac_mode() {
        let r = resolve("line", "src", "L").unwrap();
        assert_eq!(format_value(r.param, "pro", r.channel, true, 3), "Optical");
        assert_eq!(format_value(r.param, "pro", r.channel, false, 3), "Analog");
        assert_eq!(format_value(r.param, "dac", r.channel, true, 3), "USB", "the ADI-2 DAC has no DAC mode list");
    }
}
