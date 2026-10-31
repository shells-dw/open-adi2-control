//! The ADI-2 over its own MIDI port (USB, or the built-in demo unit).
//!
//! One thread per connection. It
//! - identifies the unit (device ID from the port name, else tries Pro, SE, DAC),
//! - keeps every parameter in memory ("addr.idx" → protocol integer),
//! - combines sets into as few SysEx messages as possible; the unit doesn't echo sets, so the
//!   model is updated on send and read back 1.5 s after the last change,
//! - re-reads everything every 30 s and the 20 EQ presets every 2 minutes,
//! - notices when the unit stops answering (MIDI Control switched off) and picks up again.

use crate::model::{empty_presets, key, split_key, Command, DeviceInfo, Origin, Preset, Problem, Update};
use crate::protocol::{self as p, Msg, Param};
use crate::schema::{self, ids};
use std::collections::{BTreeMap, HashMap};
use std::sync::mpsc::{Receiver, RecvTimeoutError};
use std::time::{Duration, Instant};

const PROBE_TIMEOUT: Duration = Duration::from_millis(700);
const PROBE_RETRY: Duration = Duration::from_secs(5);
const READY_FALLBACK: Duration = Duration::from_secs(3);
const FLUSH: Duration = Duration::from_millis(8);
const READBACK: Duration = Duration::from_millis(1500);
const RESYNC: Duration = Duration::from_secs(30);
const PRESETS_EVERY: Duration = Duration::from_secs(120);
const PRESET_GAP: Duration = Duration::from_millis(40);
const SILENT: Duration = Duration::from_secs(4);
const REPROBE: Duration = Duration::from_secs(3);
const WATCH: Duration = Duration::from_secs(1);

/// What the MIDI port reports.
#[derive(Debug, Clone)]
pub enum LinkEvent {
    Open { port: String, hint: Option<String> },
    Closed(String),
    Data(Vec<u8>),
    Problem(Option<Problem>),
    /// the port dropped input (sequencer buffer overrun): the settings are read again
    Lost,
}

pub enum In {
    Link(LinkEvent),
    Cmd(Command),
    Stop,
}

/// Writes complete SysEx messages to the unit.
pub trait Transport: Send {
    fn write(&self, buf: &[u8]) -> bool;
    /// called once when the engine stops
    fn close(&self) {}
}

/// Model hint from the USB name: "ADI-2 DAC", "ADI-2/4 Pro SE", "ADI-2 Pro …".
pub fn model_hint(name: &str) -> Option<&'static str> {
    let n = name.to_ascii_uppercase();
    if n.contains("ADI-2 DAC") || n.contains("ADI-2DAC") {
        Some("dac")
    } else if n.contains("ADI-2/4") {
        Some("se")
    } else if n.contains("ADI-2") {
        Some("pro")
    } else {
        None
    }
}

#[derive(Debug, Clone, PartialEq)]
enum Timer {
    Probe,
    Ready,
    Flush,
    Readback,
    Resync,
    Watch,
    Presets,
    /// read EQ preset n
    PresetReq(u8),
    /// a message that must go out (preset store flags word, read-back of one preset)
    Send(Vec<u8>),
}

impl Timer {
    /// one of its kind in the list at a time
    fn single(&self) -> bool {
        !matches!(self, Timer::Send(_))
    }
}

pub struct Engine {
    t: Box<dyn Transport>,
    out: Box<dyn Fn(Update) + Send>,
    timers: Vec<(Instant, Timer)>,
    splitter: p::Splitter,

    connected: bool,
    port: Option<String>,
    problem: Option<Problem>,
    probe_list: Vec<&'static str>,
    probing: Option<u8>,
    id: Option<u8>,
    ready: bool,
    answering: bool,
    last_rx: Instant,
    last_reprobe: Instant,

    state: HashMap<String, i32>,
    pending: BTreeMap<String, i32>,
    /// when each key was last sent: a dump requested before that may still arrive with the old
    /// value and is ignored for the key until the read-back
    sent_at: HashMap<String, Instant>,
    setups: Vec<String>,
    presets: Vec<Preset>,
    preset_cursor: Option<u8>,
    last_status: Option<Vec<u8>>,
    last_device: Option<DeviceInfo>,
    /// since the port was opened: messages sent, bytes and RME messages received (for the
    /// "doesn't answer" text)
    sent: u32,
    rx_bytes: usize,
    rx_rme: u32,
}

pub fn run(t: Box<dyn Transport>, rx: Receiver<In>, out: Box<dyn Fn(Update) + Send>) {
    let mut e = Engine::new(t, out);
    e.emit_device();
    loop {
        let now = Instant::now();
        let next = e.timers.iter().map(|(at, _)| *at).min();
        let msg = match next {
            Some(at) if at <= now => None,
            Some(at) => match rx.recv_timeout(at - now) {
                Ok(m) => Some(m),
                Err(RecvTimeoutError::Timeout) => None,
                Err(RecvTimeoutError::Disconnected) => break,
            },
            None => match rx.recv() {
                Ok(m) => Some(m),
                Err(_) => break,
            },
        };
        match msg {
            Some(In::Stop) => break,
            Some(In::Link(ev)) => e.on_link(ev),
            Some(In::Cmd(c)) => e.on_command(c),
            None => e.run_due(),
        }
    }
    e.t.close();
}

impl Engine {
    fn new(t: Box<dyn Transport>, out: Box<dyn Fn(Update) + Send>) -> Self {
        Engine {
            t,
            out,
            timers: Vec::new(),
            splitter: p::Splitter::default(),
            connected: false,
            port: None,
            problem: Some(Problem::new("notfound", "Looking for the ADI-2…")),
            probe_list: Vec::new(),
            probing: None,
            id: None,
            ready: false,
            answering: false,
            last_rx: Instant::now(),
            last_reprobe: Instant::now(),
            state: HashMap::new(),
            pending: BTreeMap::new(),
            sent_at: HashMap::new(),
            setups: vec![String::new(); 9],
            presets: empty_presets(),
            preset_cursor: None,
            last_status: None,
            last_device: None,
            sent: 0,
            rx_bytes: 0,
            rx_rme: 0,
        }
    }

    // ---------------------------------------------------------------- timers

    fn at(&mut self, delay: Duration, t: Timer) {
        if t.single() {
            self.timers.retain(|(_, x)| *x != t);
        }
        self.timers.push((Instant::now() + delay, t));
    }

    fn cancel(&mut self, t: &Timer) {
        self.timers.retain(|(_, x)| x != t);
    }

    fn has(&self, t: &Timer) -> bool {
        self.timers.iter().any(|(_, x)| x == t)
    }

    fn run_due(&mut self) {
        let now = Instant::now();
        let mut due: Vec<(Instant, Timer)> = Vec::new();
        self.timers.retain(|(at, t)| {
            if *at <= now {
                due.push((*at, t.clone()));
                false
            } else {
                true
            }
        });
        due.sort_by_key(|(at, _)| *at);
        for (_, t) in due {
            match t {
                Timer::Probe => self.probe(),
                Timer::Ready => self.set_ready(),
                Timer::Flush => self.flush(),
                Timer::Readback => self.refresh(),
                Timer::Resync => {
                    self.refresh();
                    self.at(RESYNC, Timer::Resync);
                }
                Timer::Watch => self.watch(),
                Timer::Presets => {
                    self.request_presets();
                    self.at(PRESETS_EVERY, Timer::Presets);
                }
                Timer::PresetReq(n) => {
                    if let Some(id) = self.id {
                        self.tx(&p::request_eq_preset(id, n));
                    }
                }
                Timer::Send(buf) => {
                    if self.id.is_some() {
                        self.tx(&buf);
                    }
                }
            }
        }
    }

    // ---------------------------------------------------------------- output

    fn emit(&self, u: Update) {
        (self.out)(u);
    }

    fn info(&self) -> DeviceInfo {
        DeviceInfo {
            connected: self.connected,
            ready: self.ready,
            answering: self.answering,
            id: self.id,
            model: self.id.map(|i| p::model_of(i).to_string()),
            port: self.port.clone(),
            problem: if self.connected && self.answering { None } else { self.problem.clone() },
            read_only: false,
        }
    }

    fn emit_device(&mut self) {
        let d = self.info();
        if self.last_device.as_ref() != Some(&d) {
            self.last_device = Some(d.clone());
            self.emit(Update::Device(d));
        }
    }

    fn tx(&mut self, buf: &[u8]) -> bool {
        let ok = self.t.write(buf);
        if ok {
            self.sent += 1;
        }
        ok
    }

    /// What went over the port so far, in plain words.
    fn traffic(&self) -> String {
        let rx = if self.rx_bytes == 0 {
            "nothing received".to_string()
        } else if self.rx_rme == 0 {
            format!("received {} bytes but no RME message", self.rx_bytes)
        } else {
            format!("received {} RME messages, none with the expected device ID", self.rx_rme)
        };
        format!("{} requests sent, {rx}", self.sent)
    }

    // ---------------------------------------------------------------- connection

    fn on_link(&mut self, ev: LinkEvent) {
        match ev {
            LinkEvent::Open { port, hint } => {
                self.reset_unit();
                self.connected = true;
                self.port = Some(port);
                let first = hint.as_deref().and_then(|h| ["pro", "se", "dac"].into_iter().find(|m| *m == h)).unwrap_or("pro");
                self.probe_list = std::iter::once(first).chain(["pro", "se", "dac"].into_iter().filter(|m| *m != first)).collect();
                self.emit(Update::Reset);
                self.emit_device();
                self.probe();
            }
            LinkEvent::Closed(reason) => {
                self.reset_unit();
                self.connected = false;
                self.port = None;
                self.problem = Some(Problem::new("disconnected", format!("ADI-2 disconnected ({reason})")));
                self.emit_device();
            }
            LinkEvent::Problem(pr) => {
                if !self.connected {
                    self.problem = pr;
                    self.emit_device();
                }
            }
            LinkEvent::Data(bytes) => {
                self.rx_bytes += bytes.len();
                let mut msgs = Vec::new();
                self.splitter.push(&bytes, |m| msgs.push(m));
                for m in msgs {
                    self.on_sysex(&m);
                }
            }
            LinkEvent::Lost => {
                self.splitter = p::Splitter::default();
                self.refresh();
                self.request_presets_after(Duration::from_millis(500));
            }
        }
    }

    fn reset_unit(&mut self) {
        self.timers.clear();
        self.splitter = p::Splitter::default();
        self.id = None;
        self.probing = None;
        self.ready = false;
        self.answering = false;
        self.state.clear();
        self.pending.clear();
        self.sent_at.clear();
        self.setups = vec![String::new(); 9];
        self.presets = empty_presets();
        self.preset_cursor = None;
        self.last_status = None;
        self.sent = 0;
        self.rx_bytes = 0;
        self.rx_rme = 0;
    }

    fn probe(&mut self) {
        if !self.connected {
            return;
        }
        if self.probe_list.is_empty() {
            // no answer on any ID: keep trying every PROBE_RETRY
            self.problem = Some(Problem::new(
                "silent",
                format!(
                    "The ADI-2 is connected but doesn't answer ({}). Is MIDI Control ON (menu Device Mode / DSD; ADI-2 DAC: Remap Keys/Diag)?",
                    self.traffic()
                ),
            ));
            self.emit_device();
            self.probe_list = vec!["pro", "se", "dac"];
            self.at(PROBE_RETRY, Timer::Probe);
            return;
        }
        let m = self.probe_list.remove(0);
        let id = p::id_of(m);
        self.probing = Some(id);
        self.tx(&p::request_all(id));
        self.at(PROBE_TIMEOUT, Timer::Probe);
    }

    fn identified(&mut self, id: u8) {
        self.cancel(&Timer::Probe);
        self.id = Some(id);
        self.answering = true;
        self.problem = None;
        self.last_rx = Instant::now();
        self.emit_device();
        self.at(RESYNC, Timer::Resync);
        self.at(WATCH, Timer::Watch);
        // the dump takes about 1.5 s; ready once the setup names arrived, or after 3 s
        self.at(READY_FALLBACK, Timer::Ready);
    }

    fn set_ready(&mut self) {
        self.cancel(&Timer::Ready);
        if self.ready || self.id.is_none() {
            return;
        }
        self.ready = true;
        self.emit_device();
        self.request_presets();
        self.at(PRESETS_EVERY, Timer::Presets);
    }

    fn watch(&mut self) {
        self.at(WATCH, Timer::Watch);
        let Some(id) = self.id else { return };
        let now = Instant::now();
        if self.answering && now - self.last_rx > SILENT {
            self.answering = false;
            self.pending.clear();
            self.cancel(&Timer::Flush);
            self.problem = Some(Problem::new(
                "silent",
                "The ADI-2 stopped answering. MIDI Control on the unit is probably OFF. The app picks up again by itself.",
            ));
            self.emit_device();
            self.last_reprobe = now;
        }
        if !self.answering && now - self.last_reprobe >= REPROBE {
            self.last_reprobe = now;
            self.tx(&p::request_all(id));
        }
    }

    fn refresh(&mut self) {
        if let Some(id) = self.id {
            self.tx(&p::request_all(id));
        }
    }

    fn request_presets(&mut self) {
        self.request_presets_after(Duration::ZERO);
    }

    fn request_presets_after(&mut self, delay: Duration) {
        if self.id.is_none() {
            return;
        }
        for n in 1..=20u8 {
            self.at(delay + PRESET_GAP * (n as u32 - 1), Timer::PresetReq(n));
        }
    }

    // ---------------------------------------------------------------- receive

    fn on_sysex(&mut self, buf: &[u8]) {
        let Some(m) = p::parse(buf) else { return };
        self.rx_rme += 1;
        match self.id {
            None => {
                // an answer to an earlier probe counts too (the unit answers only its own ID)
                if self.probing.is_some() && matches!(m.id, p::ID_DAC | p::ID_PRO | p::ID_SE) {
                    self.identified(m.id);
                } else {
                    return;
                }
            }
            Some(id) if id != m.id => return,
            _ => {}
        }
        self.last_rx = Instant::now();
        if !self.answering {
            self.answering = true;
            self.problem = None;
            self.emit_device();
            // anything may have changed meanwhile; the last reprobe already asked for the dump
            if self.last_reprobe.elapsed() > Duration::from_secs(2) {
                self.refresh();
            }
            self.request_presets();
        }
        match m.msg {
            Msg::Params(params) if m.cmd == p::cmd::PARAMS_TO_REMOTE => self.on_params(params),
            Msg::SetupName { n, name } => {
                let i = (n - 1) as usize;
                if self.setups[i] != name {
                    self.setups[i] = name;
                    if self.ready && n != 9 {
                        self.emit(Update::Setups(self.setups.clone()));
                    }
                }
                if n == 9 {
                    self.emit(Update::Setups(self.setups.clone()));
                    self.set_ready();
                }
            }
            Msg::PresetName { n, name } => {
                let e = &mut self.presets[(n - 1) as usize];
                // an empty slot reports a placeholder ("EQ Preset 3"); its flags word clears it
                let name = if e.empty == Some(true) { String::new() } else { name };
                if e.name != name {
                    e.name = name;
                    self.presets_changed();
                }
            }
            Msg::Status(data) if self.last_status.as_deref() != Some(&data[..]) => {
                self.last_status = Some(data.clone());
                if let Some(s) = p::parse_status(&data) {
                    self.emit(Update::Status(s));
                }
            }
            _ => {}
        }
    }

    fn on_params(&mut self, params: Vec<Param>) {
        let mut changed: Vec<(String, i32)> = Vec::new();
        let mut presets_touched = false;
        for q in &params {
            // EQ preset data (13 = left, 14 = right) is not a live setting
            // (the unit sends it at index 1, the table says 2; index 2 is never data on address 13)
            if q.addr == p::PRESET_ADDR_L && (q.idx == p::PRESET_FLAG_IDX || q.idx == p::PRESET_STORE_IDX) {
                // the flags word comes first; that preset's data follows
                let f = p::decode_preset_flag(q.value);
                self.preset_cursor = Some(f.n);
                if let Some(e) = self.presets.get_mut((f.n - 1) as usize) {
                    if e.empty != Some(f.empty) || e.dual != f.dual {
                        presets_touched = true;
                    }
                    e.empty = Some(f.empty);
                    e.dual = f.dual;
                    if f.empty && (!e.l.is_empty() || !e.r.is_empty() || !e.name.is_empty()) {
                        e.l.clear();
                        e.r.clear();
                        e.name.clear();
                        presets_touched = true;
                    }
                }
                continue;
            }
            if q.addr == p::PRESET_ADDR_L || q.addr == p::PRESET_ADDR_R {
                if let Some(e) = self.preset_cursor.and_then(|n| self.presets.get_mut((n - 1) as usize)) {
                    let side = if q.addr == p::PRESET_ADDR_L { &mut e.l } else { &mut e.r };
                    if side.get(&q.idx) != Some(&q.value) {
                        side.insert(q.idx, q.value);
                        presets_touched = true;
                    }
                }
                continue;
            }
            let k = key(q.addr, q.idx);
            if self.pending.contains_key(&k) || self.sent_at.get(&k).is_some_and(|t| t.elapsed() < READBACK) {
                continue; // the app's own value wins until the read-back after it
            }
            if self.state.get(&k) != Some(&q.value) {
                self.state.insert(k.clone(), q.value);
                changed.push((k, q.value));
            }
        }
        // a preset selected on the unit: show its values right away (only when sent alone)
        for q in &params {
            if q.idx == ids::EQ_PRESET
                && changed.iter().any(|(k, _)| *k == key(q.addr, q.idx))
                && !params.iter().any(|o| o.addr == q.addr && o.idx != ids::EQ_PRESET)
            {
                self.preset_loaded(q.addr, q.value, &mut changed);
            }
        }
        if !changed.is_empty() {
            self.emit(Update::Params(changed, Origin::Device));
        }
        if presets_touched {
            self.presets_changed();
        }
    }

    fn presets_changed(&self) {
        self.emit(Update::Presets(self.presets.clone()));
    }

    /// Selecting EQ preset n on an output loads it into that output's EQ. The unit doesn't report
    /// that by itself, so the model takes the preset's values now (bands always, Bass/Treble when
    /// the output's "Load B/T with preset" is on); the read-back confirms them.
    fn preset_loaded(&mut self, eq_addr: u8, value: i32, changed: &mut Vec<(String, i32)>) {
        let Some(ch) = schema::channels().iter().find(|c| c.eq == eq_addr) else { return };
        if !(2..=21).contains(&value) {
            return;
        }
        let pr = self.presets[(value - 2) as usize].clone();
        if pr.empty != Some(false) || pr.l.is_empty() {
            return;
        }
        let mut put = |state: &mut HashMap<String, i32>, addr: u8, i: u8, v: Option<i32>| {
            if let Some(v) = v {
                let k = key(addr, i);
                if state.get(&k) != Some(&v) {
                    state.insert(k.clone(), v);
                    changed.retain(|(x, _)| *x != k);
                    changed.push((k, v));
                }
            }
        };
        for i in p::preset_idx_r() {
            put(&mut self.state, ch.eq, i, pr.l.get(&i).copied());
            let rv = if pr.dual { pr.r.get(&i).or(pr.l.get(&i)) } else { pr.l.get(&i) };
            put(&mut self.state, ch.eq_r, i, rv.copied());
        }
        if self.state.get(&key(ch.eq, ids::LOAD_BT)).copied().unwrap_or(0) != 0 {
            for i in p::PRESET_BT_IDX {
                put(&mut self.state, ch.eq, i, pr.l.get(&i).copied());
            }
        }
        let dual_idx = if ch.output { ids::DUAL_EQ_OUT } else { ids::DUAL_EQ_IN };
        put(&mut self.state, ch.addr, dual_idx, Some(pr.dual as i32));
    }

    // ---------------------------------------------------------------- commands

    fn usable(&self) -> Option<u8> {
        if self.connected && self.answering {
            self.id
        } else {
            None
        }
    }

    fn on_command(&mut self, c: Command) {
        let Some(id) = self.usable() else {
            if let Command::Refresh = c {
                self.refresh();
            }
            return;
        };
        match c {
            Command::Set(items) => {
                let mut applied = Vec::new();
                for (k, v) in items {
                    let Some((addr, idx)) = split_key(&k) else { continue };
                    if addr > 12 || (addr == 12 && idx == schema::idx("device", "srate")) {
                        continue; // preset data and read-only sample rate
                    }
                    self.state.insert(k.clone(), v);
                    self.pending.insert(k.clone(), v);
                    applied.retain(|(x, _): &(String, i32)| *x != k);
                    applied.push((k.clone(), v));
                    if idx == ids::EQ_PRESET && schema::channels().iter().any(|c| c.eq == addr) {
                        self.preset_loaded(addr, v, &mut applied);
                    }
                }
                if !self.has(&Timer::Flush) {
                    self.at(FLUSH, Timer::Flush);
                }
                if !applied.is_empty() {
                    self.emit(Update::Params(applied, Origin::App));
                }
            }
            Command::LoadSetup(n) if (1..=9).contains(&n) => {
                self.tx(&p::load_setup(id, n));
                self.after_setup_change(Duration::from_millis(600));
            }
            Command::SaveSetup(n, name) if (1..=9).contains(&n) => {
                self.flush(); // pending sets belong in the saved state
                if let Some(name) = name.filter(|s| !p::clean_name(s).is_empty()) {
                    self.setup_name(id, n, &name);
                }
                self.tx(&p::save_setup(id, n));
                self.at(Duration::from_millis(600), Timer::Readback);
            }
            Command::RenameSetup(n, name) if (1..=9).contains(&n) && !p::clean_name(&name).is_empty() => {
                self.setup_name(id, n, &name);
                self.at(Duration::from_millis(600), Timer::Readback);
            }
            Command::LoadFactory => {
                self.tx(&p::load_factory(id));
                self.after_setup_change(Duration::from_millis(800));
            }
            Command::StorePreset { n, name, l, r, dual } if (1..=20).contains(&n) => {
                // complete partial presets with RME's defaults, so a stored preset is always whole
                let def = &schema::schema()["eqDefaults"]["values"];
                let fill = |vals: &BTreeMap<u8, i32>, list: Vec<u8>| -> BTreeMap<u8, i32> {
                    list.into_iter()
                        .filter_map(|i| vals.get(&i).copied().or_else(|| def[i.to_string()].as_i64().map(|v| v as i32)).map(|v| (i, v)))
                        .collect()
                };
                let l = fill(&l, p::preset_idx_l().collect());
                let r = if dual { fill(&r, p::preset_idx_r().collect()) } else { fill(&l, p::preset_idx_r().collect()) };
                self.store_preset(id, n, &l, &r, dual, name.as_deref());
                let e = &mut self.presets[(n - 1) as usize];
                e.empty = Some(false);
                e.dual = dual;
                e.l = l;
                e.r = r;
                if let Some(name) = name {
                    e.name = p::clean_name(&name);
                }
                self.presets_changed();
            }
            Command::RenamePreset(n, name) if (1..=20).contains(&n) => {
                let e = self.presets[(n - 1) as usize].clone();
                if e.empty != Some(false) || e.l.is_empty() {
                    return;
                }
                let r = if e.dual { e.r.clone() } else { p::preset_idx_r().filter_map(|i| e.l.get(&i).map(|v| (i, *v))).collect() };
                self.store_preset(id, n, &e.l, &r, e.dual, Some(&name));
                self.presets[(n - 1) as usize].name = p::clean_name(&name);
                self.presets_changed();
            }
            Command::Refresh => {
                self.refresh();
                self.request_presets();
            }
            _ => {}
        }
    }

    fn after_setup_change(&mut self, delay: Duration) {
        self.at(delay, Timer::Readback);
        // the presets after the dump
        self.request_presets_after(delay + Duration::from_millis(400));
    }

    fn setup_name(&mut self, id: u8, n: u8, name: &str) {
        let clean = p::clean_name(name);
        self.tx(&p::name_setup(id, n, &clean));
        self.setups[(n - 1) as usize] = clean;
        self.emit(Update::Setups(self.setups.clone()));
    }

    /// The unit keeps preset data and name in a temporary memory and stores them when the flags
    /// word for the slot arrives: data, name, flags word, then read it back.
    fn store_preset(&mut self, id: u8, n: u8, l: &BTreeMap<u8, i32>, r: &BTreeMap<u8, i32>, dual: bool, name: Option<&str>) {
        let lv: Vec<(u8, i32)> = l.iter().map(|(a, b)| (*a, *b)).collect();
        let rv: Vec<(u8, i32)> = r.iter().map(|(a, b)| (*a, *b)).collect();
        let mut msgs = p::store_eq_preset(id, n, &lv, &rv, dual);
        let flags = msgs.pop().unwrap();
        for m in msgs {
            self.tx(&m);
        }
        if let Some(name) = name {
            self.tx(&p::name_eq_preset(id, n, name));
        }
        self.at(Duration::from_millis(30), Timer::Send(flags));
        self.at(Duration::from_millis(430), Timer::PresetReq(n));
    }

    fn flush(&mut self) {
        self.cancel(&Timer::Flush);
        let Some(id) = self.id else { return };
        if self.pending.is_empty() {
            return;
        }
        let now = Instant::now();
        self.sent_at.retain(|_, t| now.duration_since(*t) < READBACK);
        let params: Vec<Param> = std::mem::take(&mut self.pending)
            .into_iter()
            .filter_map(|(k, value)| {
                self.sent_at.insert(k.clone(), now);
                split_key(&k).map(|(addr, idx)| Param { addr, idx, value })
            })
            .collect();
        for m in p::params_to_device(id, &params) {
            self.tx(&m);
        }
        self.at(READBACK, Timer::Readback);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::Update;
    use std::sync::mpsc;
    use std::sync::{Arc, Mutex};

    struct Seen {
        device: DeviceInfo,
        p: HashMap<String, i32>,
        setups: Vec<String>,
        presets: Vec<Preset>,
        status: bool,
    }

    fn start(model: &str) -> (mpsc::Sender<In>, Arc<Mutex<Seen>>) {
        let (tx, rx) = mpsc::channel();
        let t = crate::mock::start(model, tx.clone());
        let seen = Arc::new(Mutex::new(Seen { device: DeviceInfo::default(), p: HashMap::new(), setups: vec![], presets: vec![], status: false }));
        let s2 = seen.clone();
        std::thread::spawn(move || {
            run(
                Box::new(t),
                rx,
                Box::new(move |u| {
                    let mut s = s2.lock().unwrap();
                    match u {
                        Update::Device(d) => s.device = d,
                        Update::Params(list, _) => s.p.extend(list),
                        Update::Setups(v) => s.setups = v,
                        Update::Presets(v) => s.presets = v,
                        Update::Status(_) => s.status = true,
                        Update::Reset => s.p.clear(),
                    }
                }),
            )
        });
        (tx, seen)
    }

    fn until(seen: &Arc<Mutex<Seen>>, what: &str, f: impl Fn(&Seen) -> bool) {
        let t0 = Instant::now();
        while t0.elapsed() < Duration::from_secs(8) {
            if f(&seen.lock().unwrap()) {
                return;
            }
            std::thread::sleep(Duration::from_millis(20));
        }
        panic!("timed out waiting for {what}");
    }

    #[test]
    fn identifies_reads_and_sets() {
        // hint "pro" first, but the demo unit is an SE: the probe must find it
        let (tx, seen) = start("se");
        until(&seen, "ready", |s| s.device.ready && s.device.answering);
        let s = seen.lock().unwrap();
        assert_eq!(s.device.model.as_deref(), Some("se"));
        assert_eq!(s.p.get("9.12"), Some(&-300)); // Phones 3/4 volume
        assert_eq!(s.setups[0], "Speakers");
        drop(s);
        until(&seen, "presets", |s| s.presets.len() == 20 && s.presets[1].name == "Warm" && s.presets[19].empty == Some(true));
        until(&seen, "status", |s| s.status);

        // a set shows at once and survives the read-back
        tx.send(In::Cmd(Command::Set(vec![("9.12".into(), -205)]))).unwrap();
        until(&seen, "set", |s| s.p.get("9.12") == Some(&-205));
        std::thread::sleep(Duration::from_millis(2200));
        assert_eq!(seen.lock().unwrap().p.get("9.12"), Some(&-205));

        // selecting preset 2 on Line Out loads its bands into the model
        tx.send(In::Cmd(Command::Set(vec![("4.28".into(), 3)]))).unwrap();
        until(&seen, "preset loaded", |s| s.p.get("4.4") == Some(&6) && s.p.get("4.5") == Some(&80));

        // store a preset with a name, read back from the unit
        let mut l = BTreeMap::new();
        l.insert(4u8, 8);
        l.insert(5u8, 3000);
        tx.send(In::Cmd(Command::StorePreset { n: 12, name: Some("Test EQ".into()), l, r: BTreeMap::new(), dual: false })).unwrap();
        until(&seen, "stored preset", |s| s.presets[11].name == "Test EQ" && s.presets[11].empty == Some(false) && s.presets[11].l.get(&5) == Some(&3000));
        std::thread::sleep(Duration::from_millis(800));
        let s = seen.lock().unwrap();
        assert_eq!(s.presets[11].l.get(&18), Some(&10000), "filled with defaults and read back");
        assert_eq!(s.presets[11].name, "Test EQ");
        drop(s);

        // rename a setup
        tx.send(In::Cmd(Command::RenameSetup(4, "Late".into()))).unwrap();
        until(&seen, "setup name", |s| s.setups[3] == "Late");
        let _ = tx.send(In::Stop);
    }

    struct Null;
    impl Transport for Null {
        fn write(&self, _: &[u8]) -> bool {
            true
        }
    }

    #[test]
    fn preset_flags_word_at_either_index() {
        // the unit sends the flags word at index 1, the table lists index 2
        let mut e = Engine::new(Box::new(Null), Box::new(|_| {}));
        for idx in [p::PRESET_FLAG_IDX, p::PRESET_STORE_IDX] {
            let n = idx + 4;
            e.on_params(vec![
                Param { addr: p::PRESET_ADDR_L, idx, value: p::encode_preset_flag(n, true) },
                Param { addr: p::PRESET_ADDR_L, idx: 4, value: 7 },
                Param { addr: p::PRESET_ADDR_R, idx: 4, value: -3 },
            ]);
            let e2 = &e.presets[(n - 1) as usize];
            assert_eq!(e2.empty, Some(false));
            assert!(e2.dual);
            assert_eq!(e2.l.get(&4), Some(&7));
            assert_eq!(e2.r.get(&4), Some(&-3));
            assert!(e.state.is_empty(), "preset data is not a live setting");
        }
        // an empty slot clears stored data
        e.on_params(vec![Param { addr: p::PRESET_ADDR_L, idx: p::PRESET_FLAG_IDX, value: ((5 - 1) << 4) | 0xf }]);
        assert_eq!(e.presets[4].empty, Some(true));
        assert!(e.presets[4].l.is_empty());
    }
}
