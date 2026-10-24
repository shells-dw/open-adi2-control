//! RME ADI-2 series SysEx codec (the protocol of the unit's USB-MIDI port).
//!
//! Source: RME "MIDI-Protocol for RME ADI-2 DAC, ADI-2 Pro and ADI-2/4 Pro SE" v0.2,
//! with corrections found on an ADI-2 Pro EX.
//!
//!   F0 00 20 0D <device id> <command> <data…> F7
//!
//! Parameters are packed into 3 × 7-bit bytes (21 bits):
//!   channels (addr 0–11, 13, 14): addr(4) | index(5) | value(12, two's complement)
//!   device   (addr 12)          : addr(4) | index(6) | value(11, unsigned)
//! EQ frequencies use the 12-bit value as 11 bits + bit 11 = "×10" flag; this module always
//! works with plain Hz.

pub const MANUFACTURER: [u8; 3] = [0x00, 0x20, 0x0d];

pub mod cmd {
    pub const PARAMS_TO_REMOTE: u8 = 0x01;
    pub const PARAMS_TO_DEVICE: u8 = 0x02;
    pub const REQUEST: u8 = 0x03;
    pub const SETUP: u8 = 0x04;
    /// EQ preset / setup name, unit → remote
    pub const NAME_TO_REMOTE: u8 = 0x05;
    /// EQ preset / setup name, remote → unit
    pub const NAME_TO_DEVICE: u8 = 0x06;
    pub const STATUS: u8 = 0x07;
}

pub const REQ_ALL: u8 = 0x09;
pub const DEVICE_ADDR: u8 = 12;
/// Max parameters per message: 256-byte SysEx limit minus framing.
pub const MAX_PARAMS_PER_MSG: usize = 80;

pub const ID_DAC: u8 = 0x71;
pub const ID_PRO: u8 = 0x72;
pub const ID_SE: u8 = 0x73;

pub fn model_of(id: u8) -> &'static str {
    match id {
        ID_DAC => "dac",
        ID_SE => "se",
        _ => "pro",
    }
}

pub fn id_of(model: &str) -> u8 {
    match model {
        "dac" => ID_DAC,
        "se" => ID_SE,
        _ => ID_PRO,
    }
}

/// EQ preset data: left bands + Bass/Treble on 13, right bands on 14.
pub const PRESET_ADDR_L: u8 = 13;
pub const PRESET_ADDR_R: u8 = 14;
/// The unit sends the flags word at index 1 …
pub const PRESET_FLAG_IDX: u8 = 1;
/// … and stores a preset only when it receives the flags word at index 2.
pub const PRESET_STORE_IDX: u8 = 2;
/// Bass/Treble gain, freq, Q – part of a preset (left side only).
pub const PRESET_BT_IDX: [u8; 6] = [21, 22, 23, 24, 25, 26];
pub const PRESET_NAME_LEN: usize = 14;

/// Bands 3–19 (both sides).
pub fn preset_idx_r() -> impl Iterator<Item = u8> {
    3u8..=19
}
/// Bands 3–19 plus Bass/Treble 21–26 (left side). Never 20 (B/T Enable) or 27 (Load B/T):
/// an ADI-2 Pro EX switches MIDI Control off when a store contains them.
pub fn preset_idx_l() -> impl Iterator<Item = u8> {
    preset_idx_r().chain(PRESET_BT_IDX)
}

fn is_eq_addr(addr: u8) -> bool {
    matches!(addr, 1 | 2 | 4 | 5 | 7 | 8 | 10 | 11 | 13 | 14)
}

pub fn is_freq(addr: u8, idx: u8) -> bool {
    is_eq_addr(addr) && matches!(idx, 5 | 8 | 11 | 14 | 18 | 22 | 25)
}

pub fn encode_freq(hz: i32) -> u32 {
    let hz = hz.max(10);
    if hz <= 0x7ff {
        hz as u32
    } else {
        let tens = ((hz as f64 / 10.0).round() as u32).min(0x7ff);
        0x800 | tens
    }
}

pub fn decode_freq(raw: u32) -> i32 {
    let raw = raw & 0xfff;
    if raw & 0x800 != 0 {
        ((raw & 0x7ff) * 10) as i32
    } else {
        raw as i32
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Param {
    pub addr: u8,
    pub idx: u8,
    pub value: i32,
}

pub fn encode_param(p: Param) -> [u8; 3] {
    let bits: u32 = if p.addr == DEVICE_ADDR {
        let v = p.value.clamp(0, 0x7ff) as u32;
        ((p.addr as u32) << 17) | (((p.idx & 0x3f) as u32) << 11) | v
    } else {
        let raw = if is_freq(p.addr, p.idx) {
            encode_freq(p.value)
        } else {
            (p.value.clamp(-2048, 2047) as u32) & 0xfff
        };
        (((p.addr & 0x0f) as u32) << 17) | (((p.idx & 0x1f) as u32) << 12) | raw
    };
    [((bits >> 14) & 0x7f) as u8, ((bits >> 7) & 0x7f) as u8, (bits & 0x7f) as u8]
}

pub fn decode_param(b: [u8; 3]) -> Param {
    let bits = ((b[0] as u32 & 0x7f) << 14) | ((b[1] as u32 & 0x7f) << 7) | (b[2] as u32 & 0x7f);
    let addr = ((bits >> 17) & 0x0f) as u8;
    if addr == DEVICE_ADDR {
        return Param { addr, idx: ((bits >> 11) & 0x3f) as u8, value: (bits & 0x7ff) as i32 };
    }
    let idx = ((bits >> 12) & 0x1f) as u8;
    let raw = bits & 0xfff;
    let value = if is_freq(addr, idx) {
        decode_freq(raw)
    } else if raw & 0x800 != 0 {
        raw as i32 - 0x1000
    } else {
        raw as i32
    };
    Param { addr, idx, value }
}

pub fn frame(id: u8, command: u8, data: &[u8]) -> Vec<u8> {
    let mut v = Vec::with_capacity(data.len() + 7);
    v.push(0xf0);
    v.extend_from_slice(&MANUFACTURER);
    v.push(id & 0x7f);
    v.push(command & 0x7f);
    v.extend(data.iter().map(|b| b & 0x7f));
    v.push(0xf7);
    v
}

pub fn params_to_device(id: u8, params: &[Param]) -> Vec<Vec<u8>> {
    params
        .chunks(MAX_PARAMS_PER_MSG)
        .map(|chunk| {
            let data: Vec<u8> = chunk.iter().flat_map(|p| encode_param(*p)).collect();
            frame(id, cmd::PARAMS_TO_DEVICE, &data)
        })
        .collect()
}

pub fn request_all(id: u8) -> Vec<u8> {
    frame(id, cmd::REQUEST, &[REQ_ALL])
}
/// n = 1…9
pub fn load_setup(id: u8, n: u8) -> Vec<u8> {
    frame(id, cmd::SETUP, &[n])
}
pub fn save_setup(id: u8, n: u8) -> Vec<u8> {
    frame(id, cmd::SETUP, &[0x09 + n])
}
pub fn load_factory(id: u8) -> Vec<u8> {
    frame(id, cmd::SETUP, &[0x13])
}
/// EQ preset n (1–20): the unit answers with its name, flags word and data.
pub fn request_eq_preset(id: u8, n: u8) -> Vec<u8> {
    frame(id, cmd::REQUEST, &[0x09 + n])
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PresetFlag {
    pub n: u8,
    pub empty: bool,
    pub dual: bool,
}

/// Flags word: bits 8–4 = preset number (0–19), bit 0 = Dual EQ; all four low bits set = empty.
pub fn decode_preset_flag(value: i32) -> PresetFlag {
    let empty = value & 0xf == 0xf;
    PresetFlag { n: (((value >> 4) & 0x1f) + 1) as u8, empty, dual: !empty && value & 1 == 1 }
}

pub fn encode_preset_flag(n: u8, dual: bool) -> i32 {
    ((n as i32 - 1) << 4) | dual as i32
}

/// Printable ASCII, trimmed, max 14 characters.
pub fn clean_name(name: &str) -> String {
    let s: String = name.chars().filter(|c| (' '..='~').contains(c)).collect();
    s.trim().chars().take(PRESET_NAME_LEN).collect()
}

/// A name as the unit sends it: right-aligned in 14 characters, then 00 00.
pub fn name_bytes(name: &str) -> Vec<u8> {
    let clean = clean_name(name);
    let mut v = format!("{:>width$}", clean, width = PRESET_NAME_LEN).into_bytes();
    v.extend_from_slice(&[0, 0]);
    v
}

pub fn name_eq_preset(id: u8, n: u8, name: &str) -> Vec<u8> {
    let mut d = vec![n];
    d.extend(name_bytes(name));
    frame(id, cmd::NAME_TO_DEVICE, &d)
}

pub fn name_setup(id: u8, n: u8, name: &str) -> Vec<u8> {
    let mut d = vec![0x20 + n];
    d.extend(name_bytes(name));
    frame(id, cmd::NAME_TO_DEVICE, &d)
}

/// Data messages of an EQ preset store, then (last) the flags word that stores it in slot n.
/// `l`/`r`: (EQ index, value) pairs.
pub fn store_eq_preset(id: u8, n: u8, l: &[(u8, i32)], r: &[(u8, i32)], dual: bool) -> Vec<Vec<u8>> {
    let mut params: Vec<Param> = Vec::new();
    for &(idx, value) in l {
        if preset_idx_l().any(|i| i == idx) {
            params.push(Param { addr: PRESET_ADDR_L, idx, value });
        }
    }
    for &(idx, value) in r {
        if preset_idx_r().any(|i| i == idx) {
            params.push(Param { addr: PRESET_ADDR_R, idx, value });
        }
    }
    let mut out = params_to_device(id, &params);
    let flag = Param { addr: PRESET_ADDR_L, idx: PRESET_STORE_IDX, value: encode_preset_flag(n, dual) };
    out.push(frame(id, cmd::PARAMS_TO_DEVICE, &encode_param(flag)));
    out
}

#[derive(Debug, Clone, PartialEq)]
pub enum Msg {
    Params(Vec<Param>),
    SetupName { n: u8, name: String },
    PresetName { n: u8, name: String },
    Status(Vec<u8>),
    Other,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Parsed {
    pub id: u8,
    pub cmd: u8,
    pub msg: Msg,
}

/// Parse a complete SysEx message; None if it isn't an RME ADI-2 message.
pub fn parse(buf: &[u8]) -> Option<Parsed> {
    if buf.len() < 7 || buf[0] != 0xf0 || *buf.last()? != 0xf7 || buf[1..4] != MANUFACTURER {
        return None;
    }
    let id = buf[4];
    let command = buf[5];
    let data = &buf[6..buf.len() - 1];
    let msg = match command {
        cmd::PARAMS_TO_REMOTE | cmd::PARAMS_TO_DEVICE => Msg::Params(
            data.chunks_exact(3).map(|c| decode_param([c[0], c[1], c[2]])).collect(),
        ),
        cmd::NAME_TO_REMOTE | cmd::NAME_TO_DEVICE if !data.is_empty() => {
            let slot = data[0];
            let name: String = data[1..]
                .iter()
                .take_while(|&&b| b != 0)
                .filter(|&&b| (0x20..0x7f).contains(&b))
                .map(|&b| b as char)
                .collect();
            let name = name.trim().to_string();
            match slot {
                0x21..=0x29 => Msg::SetupName { n: slot - 0x20, name },
                1..=20 => Msg::PresetName { n: slot, name },
                _ => Msg::Other,
            }
        }
        cmd::STATUS => Msg::Status(data.to_vec()),
        _ => Msg::Other,
    };
    Some(Parsed { id, cmd: command, msg })
}

/// Decoded status message (about once a second while a remote is active).
#[derive(Debug, Clone, Default, serde::Serialize)]
pub struct Status {
    pub hex: String,
    /// byte 6 bits 1–0
    pub active_out: u8,
    pub revision: u8,
    pub active_line: u8,
    /// current device mode (byte 15 bits 2–0)
    pub mode: u8,
    pub balanced: bool,
    pub dsd: u8,
    pub protocol: u8,
}

pub fn parse_status(data: &[u8]) -> Option<Status> {
    if data.len() < 11 {
        return None;
    }
    let b6 = data[0];
    let b15 = data[9];
    Some(Status {
        hex: hex(data),
        active_out: b6 & 0x3,
        revision: (b6 >> 2) & 0x3,
        active_line: (b6 >> 4) & 0x3,
        mode: b15 & 0x7,
        balanced: b15 & 0x8 != 0,
        dsd: (b15 >> 4) & 0x7,
        protocol: data[10],
    })
}

/// Splits a raw MIDI byte stream into complete SysEx messages.
#[derive(Default)]
pub struct Splitter {
    buf: Option<Vec<u8>>,
}

impl Splitter {
    pub fn push(&mut self, bytes: &[u8], mut on_msg: impl FnMut(Vec<u8>)) {
        for &b in bytes {
            if b == 0xf0 {
                self.buf = Some(vec![b]);
            } else if let Some(buf) = self.buf.as_mut() {
                if b == 0xf7 {
                    buf.push(b);
                    on_msg(self.buf.take().unwrap());
                } else if b >= 0xf8 {
                    // real-time byte inside SysEx: ignore
                } else if b & 0x80 != 0 {
                    self.buf = None; // aborted
                } else {
                    buf.push(b);
                    if buf.len() > 4096 {
                        self.buf = None;
                    }
                }
            }
        }
    }
}

pub fn hex(buf: &[u8]) -> String {
    buf.iter().map(|b| format!("{b:02X}")).collect::<Vec<_>>().join(" ")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rme_example_phones34_volume() {
        // RME table example: F0 00 20 0D 73 02 4B 1F 1C F7 = Phones 3/4 volume −10 dB
        let p = Param { addr: 9, idx: 12, value: -100 };
        let m = params_to_device(ID_SE, &[p]);
        assert_eq!(m[0], vec![0xF0, 0x00, 0x20, 0x0D, 0x73, 0x02, 0x4B, 0x1F, 0x1C, 0xF7]);
        let parsed = parse(&m[0]).unwrap();
        assert_eq!(parsed.msg, Msg::Params(vec![p]));
    }

    #[test]
    fn device_params_use_6_bit_index() {
        let p = Param { addr: 12, idx: 62, value: 1 };
        assert_eq!(decode_param(encode_param(p)), p);
        let p = Param { addr: 12, idx: 16, value: 5 };
        assert_eq!(decode_param(encode_param(p)), p);
    }

    #[test]
    fn frequencies_above_2047_use_tens() {
        for hz in [20, 100, 2047, 5000, 6500, 20000] {
            let p = Param { addr: 4, idx: 8, value: hz };
            assert_eq!(decode_param(encode_param(p)).value, hz);
        }
        // not a frequency index: plain signed value
        let p = Param { addr: 4, idx: 7, value: -24 };
        assert_eq!(decode_param(encode_param(p)), p);
    }

    #[test]
    fn names_are_right_aligned() {
        let b = name_bytes("HD 650");
        assert_eq!(b.len(), 16);
        assert_eq!(&b[..14], b"        HD 650");
        let m = name_setup(ID_PRO, 3, "Speakers");
        let p = parse(&m).unwrap();
        assert_eq!(p.cmd, cmd::NAME_TO_DEVICE);
        assert_eq!(p.msg, Msg::SetupName { n: 3, name: "Speakers".into() });
    }

    #[test]
    fn preset_store_ends_with_flags_word_at_index_2() {
        let l: Vec<(u8, i32)> = preset_idx_l().map(|i| (i, 0)).chain([(20, 1), (27, 1)]).collect();
        let r: Vec<(u8, i32)> = preset_idx_r().map(|i| (i, 0)).collect();
        let msgs = store_eq_preset(ID_PRO, 10, &l, &r, false);
        let last = parse(msgs.last().unwrap()).unwrap();
        assert_eq!(last.msg, Msg::Params(vec![Param { addr: 13, idx: 2, value: 9 << 4 }]));
        // never B/T Enable (20) or Load B/T (27)
        for m in &msgs {
            if let Msg::Params(ps) = parse(m).unwrap().msg {
                assert!(ps.iter().all(|p| !(p.addr == 13 && (p.idx == 20 || p.idx == 27))));
            }
        }
        let f = decode_preset_flag(encode_preset_flag(10, true));
        assert_eq!(f, PresetFlag { n: 10, empty: false, dual: true });
        assert!(decode_preset_flag(0x2f).empty);
    }

    #[test]
    fn splitter_handles_chunks_and_noise() {
        let mut s = Splitter::default();
        let mut out = Vec::new();
        s.push(&[0xF8, 0xF0, 0x00, 0x20], |m| out.push(m));
        s.push(&[0x0D, 0x72, 0x07, 0xF8, 0x01, 0xF7, 0x90], |m| out.push(m));
        assert_eq!(out, vec![vec![0xF0, 0x00, 0x20, 0x0D, 0x72, 0x07, 0x01, 0xF7]]);
    }
}
