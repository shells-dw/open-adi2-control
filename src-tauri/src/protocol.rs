//! RME ADI-2 series SysEx codec (the protocol of the unit's USB-MIDI port).
//!
//! Source: RME "MIDI-Protocol for RME ADI-2 DAC, ADI-2 Pro and ADI-2/4 Pro SE/EX" 1.0
//! (2026-10-29), with corrections found on an ADI-2 Pro EX.
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
        "se" | "ex" => ID_SE,
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

/// Decoded status message (about once a second while a remote is active). Byte numbers as in
/// RME's table (byte 6 = first data byte); the IOStatus words are 4 × 7 bits, first byte = bits 0–6.
#[derive(Debug, Clone, Default, serde::Serialize)]
pub struct Status {
    pub hex: String,
    // byte 6
    /// bits 1–0: active output (ADI-2 DAC); active output of channel 1/2 (Pro models)
    pub active_out: u8,
    /// bits 3–2: device revision (1 = ADI-2/4 Pro EX on device ID 0x73)
    pub revision: u8,
    /// bits 5–4
    pub active_line: u8,
    /// bit 6 ("Phones 34 active" in the table; meaning not confirmed)
    pub ph34_active: bool,
    /// IOStatus1 / IOStatus2 (bytes 7–10 / 11–14)
    pub io1: u32,
    pub io2: u32,
    // IOStatus1
    pub aes_lock: bool,
    pub spdif_lock: bool,
    pub aes_sync: bool,
    pub spdif_sync: bool,
    /// SPDIF input: false coax, true optical
    pub spdif_optical: bool,
    pub adat: bool,
    /// 0 internal, 1 AES, 2 SPDIF, 3 internal
    pub clock: u8,
    /// sample rate codes: 0 32k, 1 44.1k, 2 48k, 4 64k, 5 88.2k, 6 96k, 8 128k, 9 176.4k, 10 192k
    pub aes_rate: u8,
    pub spdif_rate: u8,
    /// 5 bits
    pub int_rate: u8,
    pub ph1_plug: bool,
    pub ph2_plug: bool,
    pub ph1_overcurrent: bool,
    pub ph2_overcurrent: bool,
    // IOStatus2
    /// bits 0 and 1 ("High Load")
    pub high_load: u8,
    pub dsd_usb: bool,
    pub dsd_line: bool,
    pub dsd_phones: bool,
    pub dsd_digital_out: bool,
    pub dsd_aes: bool,
    pub dsd_spdif: bool,
    pub aes_24bit: bool,
    pub spdif_24bit: bool,
    pub aes_professional: bool,
    pub spdif_professional: bool,
    pub aes_emphasis: bool,
    pub spdif_emphasis: bool,
    pub aes_non_audio: bool,
    pub spdif_non_audio: bool,
    pub ph1_dc_error: bool,
    pub ph2_dc_error: bool,
    pub power_fail: bool,
    /// AES / SPDIF input has a valid signal (lock and sync)
    pub aes_in: bool,
    pub spdif_in: bool,
    // byte 15
    /// bits 2–0: current device mode
    pub mode: u8,
    /// bit 3: balanced phones mode
    pub balanced: bool,
    /// bits 6–4: DSD decode on outputs
    pub dsd: u8,
    // byte 16
    /// bits 3–0
    pub protocol: u8,
    /// bit 4: 8-channel mode (Pro models)
    pub eight_channel: bool,
    // byte 17 (not sent by every firmware)
    /// bits 4–0: sample rate in DAC mode
    pub dac_rate: Option<u8>,
    /// bit 5: 4.4 mm Pentaconn plug detected (ADI-2/4)
    pub penta: bool,
    // byte 18
    /// bits 2–0: bit test detected
    pub bit_test: Option<u8>,
    pub dc_line: bool,
    pub dc_phones: bool,
    /// bytes after byte 18
    pub extra: String,
}

fn word28(b: &[u8]) -> u32 {
    (b[0] as u32 & 0x7f) | ((b[1] as u32 & 0x7f) << 7) | ((b[2] as u32 & 0x7f) << 14) | ((b[3] as u32 & 0x7f) << 21)
}

pub fn parse_status(data: &[u8]) -> Option<Status> {
    if data.len() < 11 {
        return None;
    }
    let b6 = data[0];
    let b15 = data[9];
    let io1 = word28(&data[1..5]);
    let io2 = word28(&data[5..9]);
    let bit = |w: u32, n: u32| w & (1 << n) != 0;
    let field = |w: u32, lo: u32, len: u32| ((w >> lo) & ((1 << len) - 1)) as u8;
    let b17 = data.get(11).copied();
    let b18 = data.get(12).copied();
    Some(Status {
        hex: hex(data),
        active_out: b6 & 0x3,
        revision: (b6 >> 2) & 0x3,
        active_line: (b6 >> 4) & 0x3,
        ph34_active: b6 & 0x40 != 0,
        io1,
        io2,
        aes_lock: bit(io1, 0),
        spdif_lock: bit(io1, 1),
        aes_sync: bit(io1, 2),
        spdif_sync: bit(io1, 3),
        spdif_optical: bit(io1, 4),
        adat: bit(io1, 5),
        clock: field(io1, 6, 2),
        aes_rate: field(io1, 8, 4),
        spdif_rate: field(io1, 12, 4),
        int_rate: field(io1, 16, 5),
        ph1_plug: bit(io1, 21),
        ph1_overcurrent: bit(io1, 25),
        ph2_overcurrent: bit(io1, 26),
        ph2_plug: bit(io1, 27),
        high_load: field(io2, 0, 2),
        dsd_usb: bit(io2, 2),
        dsd_line: bit(io2, 3),
        dsd_phones: bit(io2, 4),
        dsd_digital_out: bit(io2, 5),
        dsd_aes: bit(io2, 6),
        dsd_spdif: bit(io2, 7),
        aes_24bit: bit(io2, 8),
        spdif_24bit: bit(io2, 9),
        aes_professional: bit(io2, 10),
        spdif_professional: bit(io2, 11),
        aes_emphasis: bit(io2, 12),
        spdif_emphasis: bit(io2, 13),
        aes_non_audio: bit(io2, 16),
        spdif_non_audio: bit(io2, 17),
        ph1_dc_error: bit(io2, 18),
        ph2_dc_error: bit(io2, 19),
        power_fail: bit(io2, 24),
        aes_in: bit(io1, 0) && bit(io1, 2),
        spdif_in: bit(io1, 1) && bit(io1, 3),
        mode: b15 & 0x7,
        balanced: b15 & 0x8 != 0,
        dsd: (b15 >> 4) & 0x7,
        protocol: data[10] & 0x0f,
        eight_channel: data[10] & 0x10 != 0,
        dac_rate: b17.map(|b| b & 0x1f),
        penta: b17.is_some_and(|b| b & 0x20 != 0),
        bit_test: b18.map(|b| b & 0x7),
        dc_line: b18.is_some_and(|b| b & 0x08 != 0),
        dc_phones: b18.is_some_and(|b| b & 0x10 != 0),
        extra: hex(data.get(13..).unwrap_or(&[])),
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
mod status_tests {
    use super::*;

    fn data(hex: &str) -> Vec<u8> {
        hex.split_whitespace().map(|h| u8::from_str_radix(h, 16).unwrap()).collect()
    }

    #[test]
    fn rme_example() {
        // RME table, "Examples": status of an ADI-2/4 Pro SE
        let s = parse_status(&data("11 30 7E 08 00 00 00 02 04 01 11 00")).unwrap();
        assert_eq!(s.active_out, 1);
        assert_eq!(s.revision, 0);
        assert_eq!(s.active_line, 1);
        assert!(s.spdif_optical && s.adat);
        assert_eq!(s.int_rate, 2, "48 kHz");
        assert_eq!(s.mode, 1, "AD/DA");
        assert_eq!(s.protocol, 1);
        assert!(s.eight_channel);
        assert_eq!(s.dac_rate, Some(0));
        assert_eq!(s.bit_test, None);
    }

    /// The four bytes of an IOStatus word from a list of set bits numbered with the first
    /// byte as bits 27–21 (the numbering of the ADI-2 Pro EX captures these tests use).
    fn word_from_old_bits(bits: &[u32]) -> Vec<u8> {
        let mut b = [0u8; 4];
        for &k in bits {
            b[3 - (k / 7) as usize] |= 1 << (k % 7);
        }
        b.to_vec()
    }

    #[test]
    fn pro_ex_captures() {
        let status = |io1_old: &[u32]| {
            let mut d = vec![0x01];
            d.extend(word_from_old_bits(io1_old));
            d.extend([0, 0, 0, 0, 0x02, 0x02, 0x0a, 0, 0]);
            parse_status(&d).unwrap()
        };
        // nothing connected, 48 kHz
        let s = status(&[27, 25, 20, 19, 16, 14, 10, 8, 7, 6]);
        assert!(!s.aes_in && !s.spdif_in);
        assert_eq!(s.int_rate, 2);
        assert_eq!(s.spdif_rate, 15, "not detected");
        // AES connected at 48 kHz: lock + sync, AES rate 48 kHz
        let s = status(&[27, 26, 25, 23, 21, 20, 16, 10, 6, 3]);
        assert!(s.aes_lock && s.aes_sync && s.aes_in && !s.spdif_in);
        assert_eq!(s.aes_rate, 2);
        // AES at 44.1 kHz
        let s = status(&[27, 26, 25, 23, 21, 20, 15, 9, 6, 3]);
        assert_eq!(s.aes_rate, 1);
        assert_eq!(s.int_rate, 1);
        // coax SPDIF
        let s = status(&[24, 22, 20, 16, 14, 10, 6]);
        assert!(s.spdif_in && !s.aes_in && !s.spdif_optical);
    }
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

#[cfg(test)]
mod rme_example_tests {
    //! The "Examples" sheet of RME's MIDI table 1.0: an ADI-2/4 Pro SE's answer to `03 09`
    //! (8 parameter messages and 9 setup names) and the answers to EQ preset requests.
    use super::*;
    use std::collections::BTreeMap;

    fn bytes(hex: &str) -> Vec<u8> {
        hex.split_whitespace().map(|h| u8::from_str_radix(h, 16).unwrap()).collect()
    }

    fn params(msgs: &[&str]) -> BTreeMap<(u8, u8), i32> {
        let mut out = BTreeMap::new();
        for m in msgs {
            match parse(&bytes(m)).expect("parses").msg {
                Msg::Params(ps) => {
                    for p in ps {
                        out.insert((p.addr, p.idx), p.value);
                    }
                }
                other => panic!("not a parameter message: {other:?}"),
            }
        }
        out
    }

    const DUMP: &[&str] = &[
            "F0 00 20 0D 73 01 00 20 03 00 40 00 00 60 00 01 00 00 01 20 00 01 40 00 01 60 02 02 00 00 02 20 00 02 40 00 02 60 00 03 00 00 08 40 00 08 60 01 09 00 00 09 20 64 09 40 63 09 60 02 0A 03 74 0A 20 0A 0A 40 00 0A 67 68 0B 00 0A 0B 20 00 0B 53 74 0B 60 0A 0C 00 01 0C 20 00 0C 57 68 0C 60 0A 10 40 00 10 60 01 11 00 00 11 20 64 11 40 63 11 60 02 12 03 74 12 20 0A 12 40 00 12 67 68 F7",
            "F0 00 20 0D 73 01 13 00 0A 13 20 00 13 53 74 13 60 0A 14 00 01 14 20 00 14 57 68 14 60 0A 18 20 00 18 40 00 18 60 01 19 00 00 19 20 64 19 40 00 19 60 00 1A 00 00 1A 20 00 1A 40 00 1A 60 00 1B 1B 73 1B 20 00 1B 40 00 1B 60 00 1C 00 00 1C 20 00 1C 40 00 20 40 00 20 60 00 21 00 0C 21 20 19 21 40 55 21 60 02 22 03 74 22 20 0A 22 40 00 22 67 68 23 00 0A 23 20 00 23 53 74 23 60 0A F7",
            "F0 00 20 0D 73 01 24 00 01 24 20 00 24 57 68 24 60 0A 28 40 00 28 60 00 29 00 0C 29 20 19 29 40 55 29 60 02 2A 03 74 2A 20 0A 2A 40 00 2A 67 68 2B 00 0A 2B 20 00 2B 53 74 2B 60 0A 2C 00 01 2C 20 00 2C 57 68 2C 60 0A 25 00 00 25 20 00 25 40 55 25 60 09 26 00 00 26 35 0A 26 40 07 1D 00 00 1D 20 0E 1D 40 0E 1D 7F 44 30 20 00 30 40 02 30 60 01 31 00 00 31 20 64 31 40 00 31 60 00 F7",
            "F0 00 20 0D 73 01 32 00 00 32 20 00 32 40 00 32 60 00 33 1F 1C 33 20 00 33 40 00 33 60 00 34 00 00 34 20 00 34 40 01 38 40 01 38 60 01 39 00 0C 39 20 41 39 40 55 39 60 02 3A 03 74 3A 20 0A 3A 40 00 3A 67 68 3B 00 0A 3B 20 00 3B 53 74 3B 60 0A 3C 00 01 3C 20 00 3C 57 68 3C 60 0A 40 40 01 40 60 01 41 00 0C 41 20 41 41 40 55 41 60 02 42 03 74 42 20 0A 42 40 00 42 67 68 43 00 0A F7",
            "F0 00 20 0D 73 01 43 20 00 43 53 74 43 60 0A 44 00 01 44 20 00 44 57 68 44 60 0A 3D 00 00 3D 20 00 3D 40 55 3D 60 09 3E 00 00 3E 35 0A 3E 40 07 35 00 00 35 20 0E 35 40 0E 35 7F 44 48 20 00 48 40 00 48 60 01 49 00 00 49 20 64 49 40 00 49 60 00 4A 00 00 4A 20 00 4A 40 00 4A 60 00 4B 1D 54 4B 20 00 4B 40 00 4B 60 00 4C 00 00 4C 20 00 4C 40 00 4C 60 00 50 40 00 50 60 01 51 00 0C F7",
            "F0 00 20 0D 73 01 51 20 41 51 40 08 51 60 00 52 03 74 52 20 0A 52 40 00 52 67 68 53 00 0A 53 20 00 53 53 74 53 60 0A 54 00 01 54 20 08 54 55 0A 54 60 05 58 40 00 58 60 01 59 00 0C 59 20 41 59 40 08 59 60 00 5A 03 74 5A 20 0A 5A 40 00 5A 67 68 5B 00 0A 5B 20 00 5B 53 74 5B 60 0A 5C 00 01 5C 20 08 5C 55 0A 5C 60 05 55 00 00 55 20 00 55 40 55 55 60 09 56 00 00 56 35 0A 56 40 07 F7",
            "F0 00 20 0D 73 01 4D 00 00 4D 20 0E 4D 40 0E 4D 7F 44 60 60 01 60 70 00 60 20 00 60 30 01 61 70 01 62 00 01 61 00 01 61 10 00 61 20 05 61 30 01 61 40 01 64 00 00 64 10 00 64 20 00 64 30 00 64 40 00 64 50 00 65 00 00 65 10 00 65 20 00 65 30 00 63 10 02 63 20 33 63 30 30 63 40 35 63 50 2A 62 60 31 62 70 32 63 00 34 67 60 00 0E 60 00 0F 00 00 26 60 00 27 00 00 3E 60 00 3F 00 00 F7",
            "F0 00 20 0D 73 01 56 60 00 57 00 02 F7",
        ];

    const SETUP_NAMES: &[&str] = &[
            "F0 00 20 0D 73 05 21 20 20 20 20 20 20 20 53 65 74 75 70 20 31 00 00 F7",
            "F0 00 20 0D 73 05 22 20 20 20 20 20 20 20 53 65 74 75 70 20 32 00 00 F7",
            "F0 00 20 0D 73 05 23 20 20 20 20 20 20 20 53 65 74 75 70 20 33 00 00 F7",
            "F0 00 20 0D 73 05 24 20 20 20 20 20 20 20 53 65 74 75 70 20 34 00 00 F7",
            "F0 00 20 0D 73 05 25 20 20 20 20 20 20 20 53 65 74 75 70 20 35 00 00 F7",
            "F0 00 20 0D 73 05 26 20 20 20 20 20 20 20 53 65 74 75 70 20 36 00 00 F7",
            "F0 00 20 0D 73 05 27 20 20 20 20 20 20 20 53 65 74 75 70 20 37 00 00 F7",
            "F0 00 20 0D 73 05 28 20 20 20 20 20 20 20 53 65 74 75 70 20 38 00 00 F7",
            "F0 00 20 0D 73 05 29 20 20 20 20 20 20 20 53 65 74 75 70 20 39 00 00 F7",
        ];

    const PRESET_2: &[&str] = &[
            "F0 00 20 0D 73 01 68 20 10 68 60 00 69 00 0C 69 20 19 69 40 0A 69 60 00 6A 03 74 6A 20 0A 6A 40 00 6A 67 68 6B 00 0A 6B 20 00 6B 53 74 6B 60 0A 6C 00 01 6C 20 00 6C 57 68 6C 60 0A 70 60 00 71 00 0C 71 20 19 71 40 0A 71 60 00 72 03 74 72 20 0A 72 40 00 72 67 68 73 00 0A 73 20 00 73 53 74 73 60 0A 74 00 01 74 20 00 74 57 68 74 60 0A 6D 40 55 6D 20 00 6D 60 07 6E 35 0A 6E 00 00 F7",
            "F0 00 20 0D 73 01 6E 35 0A 6E 00 00 6E 40 07 F7",
        ];

    #[test]
    fn full_dump() {
        let p = params(DUMP);
        assert_eq!(p.len(), 282); // 7 × 40 parameters + 2
        // table defaults: EQ bands 100/500/1000/5000/10000 Hz (×10 flag above 2047), Q 1.0,
        // band 1/5 type Shelf, Bass 85 Hz, Treble 6500 Hz
        assert_eq!((p[&(1, 5)], p[&(1, 8)], p[&(1, 11)], p[&(1, 14)], p[&(1, 18)]), (100, 500, 1000, 5000, 10000));
        assert_eq!((p[&(1, 3)], p[&(1, 16)], p[&(1, 9)]), (1, 1, 10));
        assert_eq!((p[&(4, 22)], p[&(4, 25)]), (85, 6500));
        // volumes in 0.1 dB: Line Out −52.5 dB, Phones 1/2 −10 dB, Phones 3/4 −30 dB
        assert_eq!((p[&(3, 12)], p[&(6, 12)], p[&(9, 12)]), (-525, -100, -300));
        // loudness: bass/treble gain 7.0 dB in 0.5 dB steps, Low Vol Ref −30 dB in 0.5 dB steps
        assert_eq!((p[&(3, 21)], p[&(3, 22)], p[&(3, 23)]), (14, 14, -60));
        // remappable keys: the ADI-2/4 Pro defaults of the "Remap Keys" sheet (51, 48, 53, 42, 49, 50, 52)
        assert_eq!([p[&(12, 26)], p[&(12, 27)], p[&(12, 28)], p[&(12, 29)], p[&(12, 22)], p[&(12, 23)], p[&(12, 24)]], [51, 48, 53, 42, 49, 50, 52]);
        // device values: Basic Mode AD/DA, Clock INT, Sample Rate 48 kHz, Load Volume present
        assert_eq!((p[&(12, 6)], p[&(12, 15)], p[&(12, 16)]), (1, 1, 1));
        assert!(p.contains_key(&(12, 62)));
        // nothing on the preset addresses or beyond the device's 64 indices
        assert!(p.keys().all(|(a, i)| *a <= 12 && (*a != 12 || *i < 64)));
    }

    #[test]
    fn setup_names() {
        for (i, m) in SETUP_NAMES.iter().enumerate() {
            match parse(&bytes(m)).unwrap().msg {
                Msg::SetupName { n, name } => {
                    assert_eq!(n as usize, i + 1);
                    assert_eq!(name, format!("Setup {}", i + 1));
                }
                other => panic!("{other:?}"),
            }
        }
    }

    #[test]
    fn eq_preset_answers() {
        // name first, then the flags word (sent at index 1), then the bands
        match parse(&bytes("F0 00 20 0D 73 05 02 20 20 20 20 50 72 6F 50 68 69 6C 65 20 38 00 00 F7")).unwrap().msg {
            Msg::PresetName { n, name } => assert_eq!((n, name.as_str()), (2, "ProPhile 8")),
            other => panic!("{other:?}"),
        }
        let p = params(PRESET_2);
        let flag = decode_preset_flag(p[&(13, 1)]);
        assert_eq!((flag.n, flag.dual, flag.empty), (2, false, false));
        // "ProPhile 8": band 1 Peak +6 dB at 25 Hz, Q 1.0; the rest default
        assert_eq!((p[&(13, 3)], p[&(13, 4)], p[&(13, 5)], p[&(13, 6)]), (0, 12, 25, 10));
        assert_eq!((p[&(14, 4)], p[&(14, 5)]), (12, 25));
        assert!(p.contains_key(&(13, 21)) && p.contains_key(&(13, 26)), "Bass/Treble are part of a preset");
        assert!(!p.contains_key(&(13, 20)) && !p.contains_key(&(13, 27)), "B/T Enable and Load B/T are not");
        // an empty slot: only the flags word, all four low bits set
        let e = params(&["F0 00 20 0D 73 01 68 20 2F F7"]);
        assert!(decode_preset_flag(e[&(13, 1)]).empty);
        match parse(&bytes("F0 00 20 0D 73 05 03 20 20 45 51 20 50 72 65 73 65 74 20 33 00 7F 00 F7")).unwrap().msg {
            Msg::PresetName { n, name } => assert_eq!((n, name.as_str()), (3, "EQ Preset 3")),
            other => panic!("{other:?}"),
        }
    }

    #[test]
    fn set_examples() {
        // "Set Volume of Phones 34 to -10dB" and "Set Input-Range to 13dBu"
        assert_eq!(encode_param(Param { addr: 9, idx: 12, value: -100 }), [0x4B, 0x1F, 0x1C]);
        assert_eq!(encode_param(Param { addr: 0, idx: 1, value: 2 }), [0x00, 0x20, 0x02]);
        assert_eq!(frame(ID_SE, cmd::PARAMS_TO_DEVICE, &[0x4B, 0x1F, 0x1C]), bytes("F0 00 20 0D 73 02 4B 1F 1C F7"));
    }
}
