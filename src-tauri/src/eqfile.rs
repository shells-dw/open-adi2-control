//! EQ preset files in ADI-2 Remote's format (.adieqpr).
//!
//! XML, one root element (ADI-2 Remote writes `ADI-2DAC` for every model), a `General` section
//! with the format version, then per slot `EQPreset{n}Left` / `EQPreset{n}Right` sections of
//! `<val e="name" v="value,"/>` entries (numbers with a trailing comma), and a `PresetNames`
//! section with `EQPresetName{n}`. Values are display units: gain in dB, frequency in Hz, Q.
//!
//! Entry names by EQ index: 2 EQEnable, 3 EQBand1Type, 4/5/6 EQBand1Gain/Freq/Q … 16 EQBand5Type,
//! 17/18/19 EQBand5Gain/Freq/Q, 20 BassTrebleEn, 21/22/23 EQBassGain/Freq/Q,
//! 24/25/26 EQTrebleGain/Freq/Q, 27 "Load BT w. EQ", 28 "EQ Preset Select"; "DualEQ" in the
//! Left section marks a preset with separate right-channel bands.

use crate::model::Preset;
use crate::schema;
use quick_xml::events::Event;
use quick_xml::Reader;
use std::collections::BTreeMap;

/// (entry name, EQ index)
const ENTRIES: &[(&str, u8)] = &[
    ("EQEnable", 2),
    ("EQBand1Type", 3),
    ("EQBand1Gain", 4),
    ("EQBand1Freq", 5),
    ("EQBand1Q", 6),
    ("EQBand2Gain", 7),
    ("EQBand2Freq", 8),
    ("EQBand2Q", 9),
    ("EQBand3Gain", 10),
    ("EQBand3Freq", 11),
    ("EQBand3Q", 12),
    ("EQBand4Gain", 13),
    ("EQBand4Freq", 14),
    ("EQBand4Q", 15),
    ("EQBand5Type", 16),
    ("EQBand5Gain", 17),
    ("EQBand5Freq", 18),
    ("EQBand5Q", 19),
    ("BassTrebleEn", 20),
    ("EQBassGain", 21),
    ("EQBassFreq", 22),
    ("EQBassQ", 23),
    ("EQTrebleGain", 24),
    ("EQTrebleFreq", 25),
    ("EQTrebleQ", 26),
    ("Load BT w. EQ", 27),
    ("EQ Preset Select", 28),
];

fn eq_param(idx: u8) -> Option<&'static serde_json::Value> {
    schema::schema()["params"]["eq"].as_array()?.iter().find(|p| p["idx"].as_u64() == Some(idx as u64))
}

/// Display value → protocol integer for an EQ index (scale and range from schema.json).
fn to_raw(idx: u8, v: f64) -> i32 {
    let v = if v.is_finite() { v } else { 0.0 };
    match eq_param(idx) {
        Some(p) if p["type"] == "level" => schema::from_display(p, v),
        Some(p) => (v.round() as i32).clamp(0, p["names"].as_array().map_or(1, |n| n.len() as i32 - 1)),
        None => (v.round() as i32).clamp(0, 1),
    }
}

fn to_display(idx: u8, raw: i32) -> f64 {
    match eq_param(idx) {
        Some(p) if p["type"] == "level" => schema::to_display(p, raw),
        _ => raw as f64,
    }
}

fn esc(s: &str) -> String {
    s.replace('&', "&amp;").replace('<', "&lt;").replace('>', "&gt;").replace('"', "&quot;")
}

/// The given presets as one .adieqpr file. Slots keep their numbers.
pub fn write(presets: &[Preset]) -> String {
    let mut out = String::from("<ADI-2DAC>\n\t<General>\n\t\t<val e=\"Version\" v=\"2\"/>\n\t</General>\n");
    let val = |out: &mut String, e: &str, v: f64| out.push_str(&format!("\t\t<val e=\"{e}\" v=\"{v:.2},\"/>\n"));
    for p in presets {
        let side = |out: &mut String, which: &str, vals: &BTreeMap<u8, i32>, left: bool| {
            out.push_str(&format!("\t<EQPreset{}{which}>\n", p.n));
            for &(e, idx) in ENTRIES {
                if !left && !(2..=19).contains(&idx) {
                    continue;
                }
                let raw = vals.get(&idx).or_else(|| p.l.get(&idx)).copied();
                match (idx, raw) {
                    (2, _) => val(out, e, 1.0),
                    (28, _) => val(out, e, 0.0),
                    (_, Some(r)) => val(out, e, to_display(idx, r)),
                    (_, None) => {}
                }
                if left && idx == 26 {
                    val(out, "DualEQ", if p.dual { 1.0 } else { 0.0 });
                }
            }
            out.push_str(&format!("\t</EQPreset{}{which}>\n", p.n));
        };
        side(&mut out, "Left", &p.l, true);
        side(&mut out, "Right", if p.dual { &p.r } else { &p.l }, false);
    }
    out.push_str("\t<PresetNames>\n");
    for p in presets {
        out.push_str(&format!("\t\t<val e=\"EQPresetName{}\" v=\"{}\"/>\n", p.n, esc(&p.name)));
    }
    out.push_str("\t</PresetNames>\n</ADI-2DAC>\n");
    out
}

/// The presets in a .adieqpr file, by slot. A right section that differs from the left one
/// in bands 1–5 marks the preset as dual.
pub fn read(text: &str) -> Result<Vec<Preset>, String> {
    let mut reader = Reader::from_str(text);
    reader.config_mut().trim_text(true);
    let mut section = String::new();
    let mut slots: BTreeMap<u8, Preset> = BTreeMap::new();
    let mut names: BTreeMap<u8, String> = BTreeMap::new();
    let mut dual_flag: BTreeMap<u8, bool> = BTreeMap::new();
    let mut seen_root = false;
    loop {
        let ev = reader.read_event().map_err(|e| format!("not a valid preset file: {e}"))?;
        match ev {
            Event::Start(e) if e.name().as_ref() != b"val" => {
                let name = String::from_utf8_lossy(e.name().as_ref()).to_string();
                if !seen_root {
                    seen_root = true;
                } else {
                    section = name;
                }
            }
            Event::End(e) if e.name().as_ref() != b"val" => section.clear(),
            // `<val …/>`, or `<val …></val>` as some writers produce it
            Event::Empty(e) | Event::Start(e) if e.name().as_ref() == b"val" => {
                let mut en = String::new();
                let mut v = String::new();
                for a in e.attributes().flatten() {
                    let val = String::from_utf8_lossy(&a.value).to_string();
                    match a.key.as_ref() {
                        b"e" => en = val,
                        b"v" => v = val,
                        _ => {}
                    }
                }
                if section == "PresetNames" {
                    if let Some(n) = en.strip_prefix("EQPresetName").and_then(|n| n.parse::<u8>().ok()) {
                        names.insert(n, unesc(&v));
                    }
                    continue;
                }
                let (n, left) = match section.strip_prefix("EQPreset") {
                    Some(rest) if rest.ends_with("Left") => (rest.trim_end_matches("Left").parse::<u8>().ok(), true),
                    Some(rest) if rest.ends_with("Right") => (rest.trim_end_matches("Right").parse::<u8>().ok(), false),
                    _ => (None, true),
                };
                let Some(n) = n.filter(|n| (1..=20).contains(n)) else { continue };
                let num: f64 = match v.trim_end_matches(',').trim().parse() {
                    Ok(x) => x,
                    Err(_) => continue,
                };
                if en == "DualEQ" {
                    dual_flag.insert(n, num != 0.0);
                    continue;
                }
                let Some(&(_, idx)) = ENTRIES.iter().find(|(e, _)| *e == en) else { continue };
                if idx == 2 || idx == 28 {
                    continue; // EQ on/off and the active preset belong to the output, not the preset
                }
                let p = slots.entry(n).or_insert_with(|| Preset { n, ..Default::default() });
                if left {
                    p.l.insert(idx, to_raw(idx, num));
                } else if (3..=19).contains(&idx) {
                    p.r.insert(idx, to_raw(idx, num));
                }
            }
            Event::Eof => break,
            _ => {}
        }
    }
    if !seen_root {
        return Err("not a preset file".into());
    }
    let mut out: Vec<Preset> = slots
        .into_values()
        .map(|mut p| {
            p.name = names.remove(&p.n).unwrap_or_default();
            p.empty = Some(false);
            let differs = p.r.iter().any(|(i, v)| p.l.get(i) != Some(v));
            p.dual = dual_flag.get(&p.n).copied().unwrap_or(false) || differs;
            if !p.dual {
                p.r.clear();
            }
            p
        })
        .collect();
    out.retain(|p| !p.l.is_empty());
    if out.is_empty() {
        return Err("the file contains no EQ presets".into());
    }
    Ok(out)
}

fn unesc(s: &str) -> String {
    quick_xml::escape::unescape(s).map(|c| c.into_owned()).unwrap_or_else(|_| s.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn preset(n: u8) -> Preset {
        // the table's EQ defaults, then a few changes
        let def = &schema::schema()["eqDefaults"]["values"];
        let mut l = BTreeMap::new();
        for i in 3..=26u8 {
            if i != 20 {
                l.insert(i, def[i.to_string()].as_i64().unwrap_or(0) as i32);
            }
        }
        l.insert(3, 0); // band 1 peak
        l.insert(4, 12); // +6 dB
        l.insert(5, 25);
        l.insert(6, 10);
        l.insert(16, 1);
        l.insert(20, 1);
        l.insert(22, 85);
        l.insert(23, 9);
        l.insert(25, 6500);
        l.insert(26, 7);
        Preset { n, name: "Pro & Phile".into(), empty: Some(false), dual: false, l, r: BTreeMap::new() }
    }

    #[test]
    fn round_trip() {
        let mut p = preset(2);
        p.dual = true;
        p.r = p.l.iter().filter(|(i, _)| (3..=19).contains(*i)).map(|(i, v)| (*i, *v)).collect();
        p.r.insert(4, -4);
        let text = write(&[preset(1), p.clone()]);
        assert!(text.contains("<EQPreset2Left>"));
        assert!(text.contains(r#"<val e="EQBand1Gain" v="6.00,"/>"#));
        assert!(text.contains(r#"<val e="EQPresetName2" v="Pro &amp; Phile"/>"#));
        let back = read(&text).unwrap();
        assert_eq!(back.len(), 2);
        assert_eq!(back[0].n, 1);
        assert!(!back[0].dual);
        assert_eq!(back[0].l, preset(1).l);
        assert_eq!(back[0].name, "Pro & Phile");
        assert!(back[1].dual);
        assert_eq!(back[1].r[&4], -4);
        assert_eq!(back[1].l[&22], 85);
    }

    #[test]
    fn adi2_pi_export() {
        // the shape ADI-2-Pi's headphone EQ writes (one slot, no DualEQ difference)
        let text = r#"<ADI-2DAC>
	<General>
		<val e="Version" v="2"/>
	</General>
	<EQPreset5Left>
		<val e="EQEnable" v="1.00,"/>
		<val e="EQBand1Type" v="1.00,"/>
		<val e="EQBand1Gain" v="-2.50,"/>
		<val e="EQBand1Freq" v="105.00,"/>
		<val e="EQBand1Q" v="0.70,"/>
		<val e="BassTrebleEn" v="0.00,"/>
		<val e="DualEQ" v="0.00,"/>
		<val e="EQ Preset Select" v="0.00,"/>
	</EQPreset5Left>
	<EQPreset5Right>
		<val e="EQEnable" v="1.00,"/>
		<val e="EQBand1Gain" v="-2.50,"/>
	</EQPreset5Right>
	<PresetNames>
		<val e="EQPresetName5" v="HD 650"/>
	</PresetNames>
</ADI-2DAC>
"#;
        let p = read(text).unwrap();
        assert_eq!(p.len(), 1);
        assert_eq!(p[0].n, 5);
        assert_eq!(p[0].name, "HD 650");
        assert_eq!(p[0].l[&3], 1);
        assert_eq!(p[0].l[&4], -5);
        assert_eq!(p[0].l[&5], 105);
        assert_eq!(p[0].l[&6], 7);
        assert!(!p[0].dual);
        assert!(read("hello").is_err());
        // values outside the unit's ranges are clamped, odd numbers ignored
        let wild = text.replace(r#"v="-2.50,""#, r#"v="999,""#).replace(r#"v="105.00,""#, r#"v="0,""#).replace(r#"v="0.70,""#, r#"v="nan,""#);
        let w = read(&wild).unwrap();
        assert_eq!((w[0].l[&4], w[0].l[&5], w[0].l[&6]), (24, 20, 5));
        assert_eq!(read(&text.replace("HD 650", "HD&apos;s &#38; co")).unwrap()[0].name, "HD's & co");
        // <val …></val> instead of <val …/>
        let open_form = text.replace("/>", "></val>").replace(r#"<val e="Version" v="2"></val>"#, r#"<val e="Version" v="2"/>"#);
        assert_eq!(read(&open_form).unwrap()[0].l[&5], 105);
    }
}
