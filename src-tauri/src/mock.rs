//! A simulated ADI-2 on a fake MIDI port: Demo mode, and the tests of the USB engine.
//!
//! Behaves like the real unit: silent until it gets a message with its own device ID; answers
//! "read all" with parameter dumps, the setup names and a status message; applies sets without
//! echoing them; sends a status message about once a second while a remote is active; answers
//! EQ preset requests with flags word, data and name; stores presets on the flags word at index 2.

use crate::device::{In, LinkEvent, Transport};
use crate::protocol::{self as p, Msg, Param};
use crate::schema;
use std::collections::BTreeMap;
use std::sync::mpsc::{self, RecvTimeoutError, Sender};
use std::time::{Duration, Instant};

pub struct MockTransport {
    tx: Sender<Option<Vec<u8>>>,
}

impl Transport for MockTransport {
    fn write(&self, buf: &[u8]) -> bool {
        self.tx.send(Some(buf.to_vec())).is_ok()
    }
    fn close(&self) {
        let _ = self.tx.send(None);
    }
}

#[derive(Clone, Default)]
struct StoredPreset {
    name: String,
    dual: bool,
    l: BTreeMap<u8, i32>,
    r: BTreeMap<u8, i32>,
}

struct Unit {
    /// ADI-2/4 Pro EX: device revision 1 in the status message
    ex: bool,
    id: u8,
    state: BTreeMap<(u8, u8), i32>,
    setups: Vec<String>,
    presets: Vec<Option<StoredPreset>>,
    temp_l: BTreeMap<u8, i32>,
    temp_r: BTreeMap<u8, i32>,
    temp_name: Option<String>,
}

fn defaults(model: &str) -> BTreeMap<(u8, u8), i32> {
    let s = schema::schema();
    let mut st = BTreeMap::new();
    let eq_def = &s["eqDefaults"]["values"];
    for c in s["channels"].as_array().unwrap() {
        if !schema::available(c, model) {
            continue;
        }
        let addr = c["addr"].as_u64().unwrap() as u8;
        let section = if c["kind"] == "input" { "input" } else { "output" };
        let line = c["line"].as_bool().unwrap_or(false);
        for prm in s["params"][section].as_array().unwrap() {
            if !schema::available(prm, model) || !schema::on_channel(prm, c["key"].as_str().unwrap_or("")) {
                continue;
            }
            st.insert((addr, prm["idx"].as_u64().unwrap() as u8), 0);
        }
        if section == "output" {
            st.insert((addr, 12), if line { -200 } else { -300 }); // volume
            st.insert((addr, 21), 14); // loudness bass +7 dB
            st.insert((addr, 22), 14);
            st.insert((addr, 23), -60); // Low Vol Ref -30 dB (0.5 dB steps)
            st.insert((addr, 2), 1);
            st.insert((addr, 18), 2); // DC protection: Filter
            st.insert((addr, 5), 100); // width 1.00
            st.insert((addr, 3), 1); // auto ref level
        }
        for side in ["eq", "eqR"] {
            let ea = c[side].as_u64().unwrap() as u8;
            for prm in s["params"]["eq"].as_array().unwrap() {
                let idx = prm["idx"].as_u64().unwrap() as u8;
                if side == "eqR" && !(3..=19).contains(&idx) {
                    continue; // B/T and preset select only on the left address
                }
                if c["kind"] == "input" && (20..=26).contains(&idx) {
                    continue; // the input has no Bass/Treble
                }
                let v = eq_def[idx.to_string()].as_i64().unwrap_or(0) as i32;
                st.insert((ea, idx), v);
            }
            st.insert((ea, 2), 0);
            st.insert((ea, 28), 0);
        }
    }
    for prm in s["params"]["device"].as_array().unwrap() {
        if schema::available(prm, model) && prm["type"] != "action" {
            st.insert((12, prm["idx"].as_u64().unwrap() as u8), 0);
        }
    }
    if model != "dac" {
        st.insert((12, 6), 2); // Basic Mode USB
    }
    st.insert((12, 16), 1); // 48 kHz
    st.insert((12, 3), 1);
    st
}

fn demo_presets() -> Vec<Option<StoredPreset>> {
    let mut v: Vec<Option<StoredPreset>> = vec![None; 20];
    let base = |name: &str, bands: &[(u8, i32, i32, i32)], bass: i32, treble: i32| {
        let def = &schema::schema()["eqDefaults"]["values"];
        let mut l: BTreeMap<u8, i32> = p::preset_idx_l().map(|i| (i, def[i.to_string()].as_i64().unwrap_or(0) as i32)).collect();
        for &(band, gain, freq, q) in bands {
            let gi = [4u8, 7, 10, 13, 17][(band - 1) as usize];
            l.insert(gi, gain);
            l.insert(gi + 1, freq);
            l.insert(gi + 2, q);
        }
        l.insert(21, bass);
        l.insert(24, treble);
        let r = p::preset_idx_r().map(|i| (i, l[&i])).collect();
        Some(StoredPreset { name: name.into(), dual: false, l, r })
    };
    v[0] = base("Flat", &[], 0, 0);
    v[1] = base("Warm", &[(1, 6, 80, 7), (4, -3, 3500, 15)], 2, -2);
    v[2] = base("Bright", &[(5, 5, 9000, 7)], 0, 3);
    v[3] = base("Low Shelf +4", &[(1, 8, 105, 7)], 0, 0);
    v[4] = base("Speech", &[(1, -10, 120, 7), (3, 4, 2500, 10)], 0, 0);
    v
}

pub fn start(model: &str, engine: Sender<In>) -> MockTransport {
    let (tx, rx) = mpsc::channel::<Option<Vec<u8>>>();
    let model = model.to_string();
    std::thread::Builder::new()
        .name("demo-unit".into())
        .spawn(move || {
            let mut u = Unit {
                ex: model == "ex",
                id: p::id_of(&model),
                state: defaults(&model),
                setups: (1..=9).map(|n| if n <= 3 { ["Speakers", "Headphones", "Night"][n - 1].to_string() } else { format!("Setup {n}") }).collect(),
                presets: demo_presets(),
                temp_l: BTreeMap::new(),
                temp_r: BTreeMap::new(),
                temp_name: None,
            };
            let send = |bytes: Vec<u8>| engine.send(In::Link(LinkEvent::Data(bytes))).is_ok();
            let name = format!("Demo {}", schema::model_name(&model));
            std::thread::sleep(Duration::from_millis(30));
            // (the ADI-2/4 Pro EX answers as the SE; its status message tells them apart)
            let hint = p::model_of(p::id_of(&model)).to_string();
            let _ = engine.send(In::Link(LinkEvent::Open { port: name, hint: Some(hint) }));
            let mut active = false;
            let mut next_status = Instant::now();
            let mut splitter = p::Splitter::default();
            loop {
                let wait = next_status.saturating_duration_since(Instant::now());
                match rx.recv_timeout(wait) {
                    Ok(None) | Err(RecvTimeoutError::Disconnected) => break,
                    Ok(Some(bytes)) => {
                        let mut msgs = Vec::new();
                        splitter.push(&bytes, |m| msgs.push(m));
                        for m in msgs {
                            if let Some(pm) = p::parse(&m) {
                                if pm.id != u.id {
                                    continue;
                                }
                                active = true;
                                for out in u.route(&m) {
                                    if !send(out) {
                                        return;
                                    }
                                }
                            }
                        }
                    }
                    Err(RecvTimeoutError::Timeout) => {}
                }
                if Instant::now() >= next_status {
                    next_status = Instant::now() + Duration::from_secs(1);
                    if active && !send(p::frame(u.id, p::cmd::STATUS, &u.status())) {
                        break;
                    }
                }
            }
        })
        .expect("demo thread");
    MockTransport { tx }
}

impl Unit {
    fn status(&self) -> Vec<u8> {
        let mode = self.state.get(&(12, 6)).copied().unwrap_or(2) as u8;
        // byte 6: active output 1, revision; IOStatus1: clock internal (3), 48 kHz, phones 1 plugged
        let b6 = 0x01 | if self.ex { 1 << 2 } else { 0 };
        vec![b6, 0x40, 0x01, 0x08, 0x01, 0, 0, 0, 0, mode & 0x7, 0x01, 0, 0]
    }

    fn dump(&self) -> Vec<Vec<u8>> {
        let params: Vec<Param> = self.state.iter().map(|(&(addr, idx), &value)| Param { addr, idx, value }).collect();
        let mut out: Vec<Vec<u8>> = params
            .chunks(p::MAX_PARAMS_PER_MSG)
            .map(|c| p::frame(self.id, p::cmd::PARAMS_TO_REMOTE, &c.iter().flat_map(|q| p::encode_param(*q)).collect::<Vec<_>>()))
            .collect();
        for (i, n) in self.setups.iter().enumerate() {
            let mut d = vec![0x21 + i as u8];
            d.extend(p::name_bytes(n));
            out.push(p::frame(self.id, p::cmd::NAME_TO_REMOTE, &d));
        }
        out.push(p::frame(self.id, p::cmd::STATUS, &self.status()));
        out
    }

    fn preset_msgs(&self, n: u8) -> Vec<Vec<u8>> {
        let mut out = Vec::new();
        let (name, flag) = match &self.presets[(n - 1) as usize] {
            Some(sp) => (sp.name.clone(), p::encode_preset_flag(n, sp.dual)),
            None => (format!("EQ Preset {n}"), ((n as i32 - 1) << 4) | 0xf),
        };
        let mut d = vec![n];
        d.extend(p::name_bytes(&name));
        out.push(p::frame(self.id, p::cmd::NAME_TO_REMOTE, &d));
        let mut params = vec![Param { addr: 13, idx: 1, value: flag }];
        if let Some(sp) = &self.presets[(n - 1) as usize] {
            params.extend(sp.l.iter().map(|(&idx, &value)| Param { addr: 13, idx, value }));
            params.extend(sp.r.iter().map(|(&idx, &value)| Param { addr: 14, idx, value }));
        }
        for c in params.chunks(p::MAX_PARAMS_PER_MSG) {
            out.push(p::frame(self.id, p::cmd::PARAMS_TO_REMOTE, &c.iter().flat_map(|q| p::encode_param(*q)).collect::<Vec<_>>()));
        }
        out
    }

    fn handle(&mut self, m: p::Parsed) -> Vec<Vec<u8>> {
        match (m.cmd, m.msg) {
            (p::cmd::PARAMS_TO_DEVICE, Msg::Params(ps)) => {
                for q in ps {
                    match q.addr {
                        13 if q.idx == p::PRESET_STORE_IDX => {
                            let f = p::decode_preset_flag(q.value);
                            if (1..=20).contains(&f.n) {
                                let name = self.temp_name.take().or_else(|| self.presets[(f.n - 1) as usize].as_ref().map(|s| s.name.clone())).unwrap_or_default();
                                self.presets[(f.n - 1) as usize] =
                                    Some(StoredPreset { name, dual: f.dual, l: std::mem::take(&mut self.temp_l), r: std::mem::take(&mut self.temp_r) });
                            }
                        }
                        13 => {
                            self.temp_l.insert(q.idx, q.value);
                        }
                        14 => {
                            self.temp_r.insert(q.idx, q.value);
                        }
                        _ => {
                            if self.state.contains_key(&(q.addr, q.idx)) {
                                self.state.insert((q.addr, q.idx), q.value);
                            }
                            if q.idx == 28 {
                                self.load_preset(q.addr, q.value);
                            }
                        }
                    }
                }
                Vec::new()
            }
            (p::cmd::NAME_TO_DEVICE, Msg::SetupName { n, name }) => {
                self.setups[(n - 1) as usize] = name;
                Vec::new()
            }
            (p::cmd::NAME_TO_DEVICE, Msg::PresetName { name, .. }) => {
                self.temp_name = Some(name);
                Vec::new()
            }
            _ => Vec::new(),
        }
    }

    /// Selecting preset n on an output loads it into that output's EQ (B/T only with Load B/T on).
    fn load_preset(&mut self, eq_addr: u8, value: i32) {
        let Some(c) = schema::channels().iter().find(|c| c.eq == eq_addr) else { return };
        if !(2..=21).contains(&value) {
            return;
        }
        let Some(sp) = self.presets[(value - 2) as usize].clone() else { return };
        for i in p::preset_idx_r() {
            if let Some(v) = sp.l.get(&i) {
                self.state.insert((c.eq, i), *v);
                self.state.insert((c.eq_r, i), if sp.dual { *sp.r.get(&i).unwrap_or(v) } else { *v });
            }
        }
        if self.state.get(&(c.eq, 27)).copied().unwrap_or(0) != 0 {
            for i in p::PRESET_BT_IDX {
                if let Some(v) = sp.l.get(&i) {
                    self.state.insert((c.eq, i), *v);
                }
            }
        }
        let dual_idx = if c.output { 11 } else { 8 };
        self.state.insert((c.addr, dual_idx), sp.dual as i32);
    }

    /// Requests and setup commands carry one data byte.
    fn handle_raw(&mut self, raw: &[u8]) -> Option<Vec<Vec<u8>>> {
        let cmd = raw[5];
        let d = *raw.get(6)?;
        match cmd {
            p::cmd::REQUEST if d == p::REQ_ALL => Some(self.dump()),
            p::cmd::REQUEST if (0x0a..=0x1d).contains(&d) => Some(self.preset_msgs(d - 0x09)),
            p::cmd::SETUP if (1..=9).contains(&d) => {
                // loading a setup: the demo just changes the volumes a little
                for c in schema::outputs() {
                    if let Some(v) = self.state.get_mut(&(c.addr, 12)) {
                        *v = -150 - 50 * d as i32;
                    }
                }
                Some(Vec::new())
            }
            p::cmd::SETUP => Some(Vec::new()),
            _ => None,
        }
    }
}

// requests and setup commands carry one data byte: those first, then parsed messages
impl Unit {
    fn route(&mut self, raw: &[u8]) -> Vec<Vec<u8>> {
        if let Some(out) = self.handle_raw(raw) {
            return out;
        }
        match p::parse(raw) {
            Some(pm) => self.handle(pm),
            None => Vec::new(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn demo_status() {
        let u = Unit { ex: true, id: p::ID_SE, state: defaults("ex"), setups: vec![], presets: vec![], temp_l: BTreeMap::new(), temp_r: BTreeMap::new(), temp_name: None };
        let s = p::parse_status(&u.status()).unwrap();
        assert_eq!(s.revision, 1);
        assert_eq!((s.clock, s.int_rate), (3, 2));
        assert!(s.ph1_plug && !s.ph2_plug);
        assert!(u.state.contains_key(&(0, 13)), "Bal. Impedance on the EX");
        assert!(!u.state.contains_key(&(12, 63)), "no state for the toggle action");
    }
}
