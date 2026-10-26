//! The app's centre: runs the connection (USB, ADI-2-Pi or demo), keeps the model, and passes
//! changes to the window, the tray and the volume popup. App-side volume extras like ADI-2
//! Remote's (limit, link, rise speed, recall) live here, so they apply to the window, the tray
//! and the volume keys alike.

use crate::config::Config;
use crate::device::{self, In};
use crate::model::{empty_presets, key, Command, DeviceInfo, Origin, Preset, Update};
use crate::protocol::Status;
use crate::schema::{self, ids, Channel};
use crate::{keys, mock, osd, pi, seq, tray, update};
use serde_json::{json, Value};
use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::mpsc::{self, Sender};
use std::sync::{Arc, Mutex};
use std::time::Duration;
use tauri::{AppHandle, Emitter};

#[derive(Default)]
pub struct Model {
    pub device: DeviceInfo,
    pub p: HashMap<String, i32>,
    pub setups: Vec<String>,
    pub presets: Vec<Preset>,
    pub status: Option<Status>,
}

pub struct Hub {
    app: AppHandle,
    pub cfg: Mutex<Config>,
    pub model: Mutex<Model>,
    backend: Mutex<Option<Sender<In>>>,
    generation: AtomicU64,
    ramps: Mutex<HashMap<String, i32>>,
    ramp_running: AtomicBool,
    /// output last turned (on the unit or in the app): what the volume keys control in "auto"
    pub last_out: Mutex<String>,
    pub upd: Mutex<UpdState>,
}

/// What the update check found.
#[derive(Default)]
pub struct UpdState {
    pub checking: bool,
    /// unix seconds of the last finished check
    pub checked_at: Option<u64>,
    pub latest: Option<update::Release>,
    pub error: Option<String>,
    /// version already announced by a desktop notification in this run
    pub notified: Option<String>,
}

fn now_secs() -> u64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0)
}

const RAMP_TICK: Duration = Duration::from_millis(50);

pub fn vol_key(c: &Channel) -> String {
    key(c.addr, ids::VOL)
}

impl Hub {
    pub fn new(app: AppHandle, cfg: Config) -> Arc<Hub> {
        Arc::new(Hub {
            app,
            cfg: Mutex::new(cfg),
            model: Mutex::new(Model { setups: vec![String::new(); 9], presets: empty_presets(), ..Default::default() }),
            backend: Mutex::new(None),
            generation: AtomicU64::new(0),
            ramps: Mutex::new(HashMap::new()),
            ramp_running: AtomicBool::new(false),
            last_out: Mutex::new("line".into()),
            upd: Mutex::new(UpdState::default()),
        })
    }

    pub fn app_handle(&self) -> AppHandle {
        self.app.clone()
    }

    pub fn config(&self) -> Config {
        self.cfg.lock().unwrap().clone()
    }

    // ---------------------------------------------------------------- connection

    /// (Re)start the connection the settings ask for.
    pub fn connect(self: &Arc<Self>) {
        let conn = self.config().unit().clone();
        if let Some(tx) = self.backend.lock().unwrap().take() {
            let _ = tx.send(In::Stop);
        }
        self.ramps.lock().unwrap().clear();
        let gen = self.generation.fetch_add(1, Ordering::SeqCst) + 1;
        self.on_update(gen, Update::Reset);
        self.on_update(gen, Update::Device(DeviceInfo::default()));
        let (tx, rx) = mpsc::channel::<In>();
        let hub = self.clone();
        let out: Box<dyn Fn(Update) + Send> = Box::new(move |u| hub.on_update(gen, u));
        match conn.mode.as_str() {
            "pi" => {
                let addr = conn.pi_address.clone();
                let key = conn.pi_key.trim().to_string();
                std::thread::Builder::new().name("adi2-pi".into()).spawn(move || pi::run(addr, key, rx, out)).ok();
            }
            "demo" => {
                let t = mock::start(&conn.demo_model, tx.clone());
                std::thread::Builder::new().name("engine".into()).spawn(move || device::run(Box::new(t), rx, out)).ok();
            }
            _ => {
                let t = seq::start(tx.clone());
                std::thread::Builder::new().name("engine".into()).spawn(move || device::run(Box::new(t), rx, out)).ok();
            }
        }
        *self.backend.lock().unwrap() = Some(tx);
    }

    pub fn stop(&self) {
        if let Some(tx) = self.backend.lock().unwrap().take() {
            let _ = tx.send(In::Stop);
        }
    }

    fn send(&self, c: Command) -> bool {
        match self.backend.lock().unwrap().as_ref() {
            Some(tx) => tx.send(In::Cmd(c)).is_ok(),
            None => false,
        }
    }

    pub fn command(&self, c: Command) -> Result<(), String> {
        let d = self.model.lock().unwrap().device.clone();
        if !matches!(c, Command::Refresh) && !(d.connected && d.answering) {
            return Err(d.problem.map(|p| p.text).unwrap_or_else(|| "The ADI-2 is not connected.".into()));
        }
        if !matches!(c, Command::Refresh) && d.read_only {
            return Err("View only: this ADI-2-Pi needs its access key for changes. Enter it under Settings → Connection.".into());
        }
        // a preset stored without a name is named after its slot
        let c = match c {
            Command::StorePreset { n, name, l, r, dual } => {
                let name = name.filter(|s| !s.trim().is_empty()).or_else(|| Some(format!("Preset {n}")));
                Command::StorePreset { n, name, l, r, dual }
            }
            c => c,
        };
        // the model takes sets at once (the popup and the tray show the new value); the
        // backend's own report of them then changes nothing
        let optimistic = match &c {
            Command::Set(items) => Some(items.clone()),
            _ => None,
        };
        if !self.send(c) {
            return Err("not connected".into());
        }
        if let Some(items) = optimistic {
            let mut changed = serde_json::Map::new();
            {
                let mut m = self.model.lock().unwrap();
                for (k, v) in items {
                    if m.p.get(&k) != Some(&v) {
                        m.p.insert(k.clone(), v);
                        changed.insert(k, json!(v));
                    }
                }
            }
            if !changed.is_empty() {
                let _ = self.app.emit("update", json!({ "kind": "params", "p": changed, "origin": Origin::App }));
                tray::refresh(&self.app);
            }
        }
        Ok(())
    }

    // ---------------------------------------------------------------- updates

    fn on_update(self: &Arc<Self>, gen: u64, u: Update) {
        if gen != self.generation.load(Ordering::SeqCst) {
            return; // from a connection that has been replaced
        }
        let mut tray_dirty = false;
        let payload = {
            let mut m = self.model.lock().unwrap();
            match u {
                Update::Reset => {
                    m.p.clear();
                    m.setups = vec![String::new(); 9];
                    m.presets = empty_presets();
                    m.status = None;
                    tray_dirty = true;
                    json!({ "kind": "reset" })
                }
                Update::Device(d) => {
                    if m.device == d {
                        return;
                    }
                    m.device = d.clone();
                    tray_dirty = true;
                    json!({ "kind": "device", "device": d })
                }
                Update::Params(list, origin) => {
                    let mut changed = serde_json::Map::new();
                    let mut vol_changed: Option<String> = None;
                    let mut vols = 0;
                    for (k, v) in list {
                        if m.p.get(&k) == Some(&v) {
                            continue;
                        }
                        m.p.insert(k.clone(), v);
                        if let Some(c) = schema::outputs().find(|c| key(c.addr, ids::VOL) == k || key(c.addr, ids::MUTE) == k || key(c.addr, ids::DIM) == k) {
                            tray_dirty = true;
                            if k == vol_key(c) {
                                vol_changed = Some(c.key.clone());
                                vols += 1;
                            }
                        }
                        if k.ends_with(&format!(".{}", ids::EQ_ENABLE)) {
                            tray_dirty = true;
                        }
                        changed.insert(k, json!(v));
                    }
                    if changed.is_empty() {
                        return;
                    }
                    drop(m);
                    // the knob was turned: one output's volume changed on the unit, not a read-out
                    // of many values (a dump or an ADI-2-Pi state message)
                    let turned = origin == Origin::Device && vols == 1 && changed.len() <= 3 && self.model.lock().unwrap().device.ready;
                    // (the first read-out of all settings doesn't count as turning)
                    if let Some(ch) = vol_changed.clone().filter(|_| self.model.lock().unwrap().device.ready) {
                        *self.last_out.lock().unwrap() = ch.clone();
                        let cfg = self.config();
                        // the knob on the unit (or another remote): show the popup
                        if origin == Origin::Device && cfg.osd.enabled && cfg.osd.knob && self.model.lock().unwrap().device.ready {
                            if let Some(info) = self.osd_info(&ch) {
                                osd::show(&self.app, &cfg, info);
                            }
                        }
                    }
                    json!({ "kind": "params", "p": changed, "origin": origin, "turned": if turned { vol_changed.clone() } else { None } })
                }
                Update::Setups(s) => {
                    m.setups = s.clone();
                    tray_dirty = true;
                    json!({ "kind": "setups", "setups": s })
                }
                Update::Presets(list) => {
                    m.presets = list.clone();
                    json!({ "kind": "presets", "presets": list })
                }
                Update::Status(s) => {
                    m.status = Some(s.clone());
                    json!({ "kind": "status", "status": s })
                }
            }
        };
        let _ = self.app.emit("update", payload);
        if tray_dirty {
            tray::refresh(&self.app);
        }
    }

    pub fn snapshot(&self) -> Value {
        let m = self.model.lock().unwrap();
        json!({
            "device": m.device,
            "p": m.p,
            "setups": m.setups,
            "presets": m.presets,
            "status": m.status,
            "config": *self.cfg.lock().unwrap(),
            "lastOut": *self.last_out.lock().unwrap(),
            "version": env!("CARGO_PKG_VERSION"),
            "update": self.update_view(),
        })
    }

    // ---------------------------------------------------------------- update check

    /// A newer version than this one, not skipped by the user.
    pub fn update_offer(&self) -> Option<update::Release> {
        let skip = self.config().updates.skip;
        let u = self.upd.lock().unwrap();
        u.latest.clone().filter(|r| update::is_newer(&r.version, env!("CARGO_PKG_VERSION")) && r.version != skip)
    }

    pub fn update_view(&self) -> Value {
        let offer = self.update_offer();
        let u = self.upd.lock().unwrap();
        json!({
            "checking": u.checking,
            "checkedAt": u.checked_at,
            "latest": u.latest,
            "newer": u.latest.as_ref().is_some_and(|r| update::is_newer(&r.version, env!("CARGO_PKG_VERSION"))),
            "offer": offer,
            "error": u.error,
            "install": update::install_kind(),
            "appimage": std::env::var("APPIMAGE").ok().filter(|s| !s.is_empty()),
        })
    }

    fn emit_update_state(&self) {
        let _ = self.app.emit("update", json!({ "kind": "updates", "update": self.update_view() }));
        tray::refresh(&self.app);
    }

    /// Ask GitHub now (blocking). `auto`: from the daily check (may notify).
    pub fn check_updates(&self, auto: bool) {
        {
            let mut u = self.upd.lock().unwrap();
            if u.checking {
                return;
            }
            u.checking = true;
        }
        self.emit_update_state();
        let res = update::latest();
        {
            let mut u = self.upd.lock().unwrap();
            u.checking = false;
            u.checked_at = Some(now_secs());
            match res {
                Ok(r) => {
                    u.latest = Some(r);
                    u.error = None;
                }
                Err(e) => u.error = Some(e),
            }
        }
        self.emit_update_state();
        if let Some(r) = self.update_offer() {
            let mut u = self.upd.lock().unwrap();
            if auto && u.notified.as_deref() != Some(&r.version) {
                u.notified = Some(r.version.clone());
                drop(u);
                osd::notify_text(&format!("Version {} is available", r.version), "See what's new under Settings → App, or in the tray menu.");
            }
        }
    }

    /// Background: shortly after start, then once a day, while the check is switched on.
    pub fn start_update_timer(self: &Arc<Self>) {
        let hub = self.clone();
        std::thread::Builder::new()
            .name("update-check".into())
            .spawn(move || {
                std::thread::sleep(Duration::from_secs(20));
                loop {
                    let due = {
                        let u = hub.upd.lock().unwrap();
                        u.checked_at.map_or(true, |t| now_secs().saturating_sub(t) >= update::EVERY.as_secs())
                    };
                    if due && hub.config().updates.check {
                        hub.check_updates(true);
                    }
                    std::thread::sleep(Duration::from_secs(600));
                }
            })
            .ok();
    }

    pub fn emit_config(&self) {
        let _ = self.app.emit("update", json!({ "kind": "config", "config": self.config() }));
    }

    // ---------------------------------------------------------------- setting values

    /// Set values from the window: volumes go through limit, link and rise speed.
    pub fn set(self: &Arc<Self>, items: Vec<(String, i32)>) -> Result<(), String> {
        let mut plain = Vec::new();
        for (k, v) in items {
            if let Some(c) = schema::outputs().find(|c| vol_key(c) == k) {
                self.volume_to(&c.key, v)?;
            } else {
                plain.push((k, v));
            }
        }
        if !plain.is_empty() {
            self.command(Command::Set(plain))?;
        }
        Ok(())
    }

    fn limit_raw(&self, ch: &str, v: i32) -> i32 {
        let (lo, hi) = schema::raw_range(schema::param("output", "vol").unwrap());
        let mut v = v.clamp(lo, hi);
        if let Some(db) = self.cfg.lock().unwrap().limit_db(ch) {
            v = v.min((db * 10.0).round() as i32);
        }
        v
    }

    /// Make profile `i` the active volume-limit profile; outputs above its limit are turned down.
    pub fn set_profile(self: &Arc<Self>, i: usize) {
        {
            let mut cfg = self.cfg.lock().unwrap();
            if i >= cfg.volume.profiles.len() {
                return;
            }
            cfg.volume.profile = i;
            cfg.save();
        }
        self.emit_config();
        self.apply_limits();
        tray::refresh(&self.app);
    }

    /// Turn outputs down to the active profile's limits.
    pub fn apply_limits(self: &Arc<Self>) {
        if !self.usable() {
            return;
        }
        for c in schema::outputs() {
            let Some(cur) = self.value(&vol_key(c)) else { continue };
            let lim = self.limit_raw(&c.key, cur);
            if lim < cur {
                let _ = self.command(Command::Set(vec![(vol_key(c), lim)]));
            }
        }
    }

    /// Set an output's volume (protocol integer, 0.1 dB) with limit, link and rise speed.
    pub fn volume_to(self: &Arc<Self>, ch: &str, target: i32) -> Result<(), String> {
        let c = schema::channel(ch).ok_or("unknown output")?;
        let cur = self.model.lock().unwrap().p.get(&vol_key(c)).copied().ok_or("volume not known yet")?;
        let target = self.limit_raw(ch, target);
        let delta = target - cur;
        let mut moves = vec![(c.key.clone(), cur, target)];
        let cfg = self.config();
        if cfg.volume.link.iter().any(|x| x == ch) && delta != 0 {
            for other in &cfg.volume.link {
                if other == ch {
                    continue;
                }
                let Some(oc) = schema::channel(other) else { continue };
                let Some(ov) = self.model.lock().unwrap().p.get(&vol_key(oc)).copied() else { continue };
                moves.push((other.clone(), ov, self.limit_raw(other, ov + delta)));
            }
        }
        let rise = cfg.volume.rise;
        let mut now = Vec::new();
        for (k, cur, t) in moves {
            if rise > 0.0 && t > cur {
                self.ramps.lock().unwrap().insert(k, t);
            } else {
                self.ramps.lock().unwrap().remove(&k);
                now.push((vol_key(schema::channel(&k).unwrap()), t));
            }
        }
        if !now.is_empty() {
            self.command(Command::Set(now))?;
        }
        self.start_ramp();
        Ok(())
    }

    fn start_ramp(self: &Arc<Self>) {
        if self.ramps.lock().unwrap().is_empty() || self.ramp_running.swap(true, Ordering::SeqCst) {
            return;
        }
        let hub = self.clone();
        std::thread::spawn(move || loop {
            std::thread::sleep(RAMP_TICK);
            let rise = hub.config().volume.rise;
            let step = ((rise * 10.0 * RAMP_TICK.as_secs_f64()).round() as i32).max(1);
            let mut sets = Vec::new();
            {
                let mut ramps = hub.ramps.lock().unwrap();
                let m = hub.model.lock().unwrap();
                ramps.retain(|ch, target| {
                    let c = schema::channel(ch).unwrap();
                    let Some(cur) = m.p.get(&vol_key(c)).copied() else { return false };
                    if cur >= *target {
                        return false;
                    }
                    let next = (cur + step).min(*target);
                    sets.push((vol_key(c), next));
                    next < *target
                });
            }
            if !sets.is_empty() && hub.command(Command::Set(sets)).is_err() {
                hub.ramps.lock().unwrap().clear();
            }
            if hub.ramps.lock().unwrap().is_empty() {
                hub.ramp_running.store(false, Ordering::SeqCst);
                // something may have been added between the check and the flag
                if !hub.ramps.lock().unwrap().is_empty() {
                    hub.start_ramp();
                }
                return;
            }
        });
    }

    // ---------------------------------------------------------------- tray, keys, popup

    /// The output the volume keys and the tray control.
    pub fn target_output(&self) -> Option<&'static Channel> {
        let cfg = self.config();
        let m = self.model.lock().unwrap();
        let has = |c: &Channel| m.p.contains_key(&vol_key(c));
        let pick = if cfg.keys.target == "auto" { self.last_out.lock().unwrap().clone() } else { cfg.keys.target.clone() };
        schema::channel(&pick).filter(|c| has(c)).or_else(|| schema::outputs().find(|c| has(c)))
    }

    pub fn usable(&self) -> bool {
        let m = self.model.lock().unwrap();
        m.device.connected && m.device.answering && m.device.ready
    }

    pub fn value(&self, k: &str) -> Option<i32> {
        self.model.lock().unwrap().p.get(k).copied()
    }

    pub fn osd_info(&self, ch: &str) -> Option<osd::Info> {
        let c = schema::channel(ch)?;
        let m = self.model.lock().unwrap();
        let vol = *m.p.get(&vol_key(c))?;
        let muted = m.p.get(&key(c.addr, ids::MUTE)).copied().unwrap_or(0) != 0;
        let dim = m.p.get(&key(c.addr, ids::DIM)).copied().unwrap_or(0) != 0;
        let (lo, hi) = schema::raw_range(schema::param("output", "vol").unwrap());
        Some(osd::Info {
            title: c.label(m.device.model.as_deref()),
            value: if muted { "Muted".into() } else { format!("{} dB{}", fmt_db(vol), if dim { " · Dim" } else { "" }) },
            frac: Some(((vol - lo) as f64 / (hi - lo) as f64).clamp(0.0, 1.0)),
            muted,
        })
    }

    /// A volume key or tray action: "volume-up" | "volume-down" | "mute".
    pub fn key_action(self: &Arc<Self>, action: &str, from_key: bool) {
        let cfg = self.config();
        let to_adi = !from_key || (cfg.keys.enabled && (cfg.unit().mode != "usb" || cfg.keys.when == "always" || keys::adi_is_default_output()));
        if !to_adi || !self.usable() {
            if from_key {
                // not the ADI-2's turn: the keys work as usual
                if let Some(info) = keys::system_volume(action) {
                    if cfg.osd.enabled {
                        osd::show(&self.app, &cfg, info);
                    }
                }
            }
            return;
        }
        let Some(c) = self.target_output() else { return };
        let mut note: Option<String> = None;
        match action {
            "volume-up" | "volume-down" => {
                let step = (cfg.volume.step * 10.0).round() as i32;
                let cur = self.value(&vol_key(c)).unwrap_or(0);
                // from a pending ramp target, so quick presses add up
                let base = self.ramps.lock().unwrap().get(&c.key).copied().unwrap_or(cur);
                let t = if action == "volume-up" { base + step } else { cur.min(base) - step };
                let limited = self.limit_raw(&c.key, t);
                if action == "volume-up" && limited < t && limited <= cur {
                    note = Some("Limit reached".into());
                }
                let _ = self.volume_to(&c.key, limited);
            }
            "mute" => {
                let k = key(c.addr, ids::MUTE);
                match self.value(&k) {
                    Some(v) => {
                        let _ = self.command(Command::Set(vec![(k, if v != 0 { 0 } else { 1 })]));
                    }
                    None => note = Some("Mute isn't available on this output over MIDI".into()),
                }
            }
            _ => return,
        }
        *self.last_out.lock().unwrap() = c.key.clone();
        if cfg.osd.enabled {
            if let Some(mut info) = self.osd_info(&c.key) {
                if let Some(n) = note {
                    info.value = format!("{} · {n}", info.value);
                }
                osd::show(&self.app, &cfg, info);
            }
        }
    }
}

impl Hub {
    /// The whole state as the CLI prints it: channel → id → display value.
    pub fn named_state(&self) -> Value {
        let m = self.model.lock().unwrap();
        let mut out = serde_json::Map::new();
        let put = |out: &mut serde_json::Map<String, Value>, ch: &str, id: &str, v: Value| {
            out.entry(ch.to_string()).or_insert_with(|| json!({})).as_object_mut().unwrap().insert(id.to_string(), v);
        };
        let sch = schema::schema();
        for c in schema::channels() {
            let section = if c.output { "output" } else { "input" };
            for p in sch["params"][section].as_array().unwrap() {
                let k = key(c.addr, p["idx"].as_u64().unwrap() as u8);
                if let Some(v) = m.p.get(&k) {
                    put(&mut out, &c.key, p["id"].as_str().unwrap(), json!(schema::to_display(p, *v)));
                }
            }
            for p in sch["params"]["eq"].as_array().unwrap() {
                let idx = p["idx"].as_u64().unwrap() as u8;
                if let Some(v) = m.p.get(&key(c.eq, idx)) {
                    put(&mut out, &c.key, p["id"].as_str().unwrap(), json!(schema::to_display(p, *v)));
                }
                if let Some(v) = m.p.get(&key(c.eq_r, idx)) {
                    put(&mut out, &c.key, &format!("{}.R", p["id"].as_str().unwrap()), json!(schema::to_display(p, *v)));
                }
            }
        }
        for p in sch["params"]["device"].as_array().unwrap() {
            if let Some(v) = m.p.get(&key(12, p["idx"].as_u64().unwrap() as u8)) {
                put(&mut out, "device", p["id"].as_str().unwrap(), json!(schema::to_display(p, *v)));
            }
        }
        json!({ "device": m.device, "setups": m.setups, "state": out })
    }

    /// A command line from the socket (`open-adi2-control <args>`). Ok = text for stdout.
    pub fn cli(self: &Arc<Self>, args: &[String]) -> Result<String, String> {
        let a = |i: usize| args.get(i).map(|s| s.as_str()).unwrap_or("");
        let model = self.model.lock().unwrap().device.model.clone().unwrap_or_else(|| "pro".into());
        let fmt = |p: &Value, raw: i32| -> String {
            match p["type"].as_str().unwrap_or("level") {
                "toggle" => (if raw != 0 { "on" } else { "off" }).to_string(),
                "select" => schema::value_names(p, &model, None).and_then(|n| n.get(raw as usize).cloned()).unwrap_or_else(|| raw.to_string()),
                _ => {
                    let d = schema::to_display(p, raw);
                    let dec = p["decimals"].as_u64().unwrap_or(0) as usize;
                    format!("{d:.dec$}")
                }
            }
        };
        match a(0) {
            "get" if args.len() == 1 => Ok(self.named_state().to_string()),
            "get" => {
                let (ch, id, side) = (a(1), a(2), a(3));
                if id.is_empty() {
                    let st = self.named_state();
                    return st["state"].get(ch).map(|v| v.to_string()).ok_or_else(|| format!("unknown channel {ch}"));
                }
                let r = schema::resolve(ch, id, side).ok_or_else(|| format!("unknown parameter {ch} {id}"))?;
                let raw = self.value(&key(r.addr, r.idx)).ok_or("not reported by the unit")?;
                Ok(fmt(r.param, raw))
            }
            "set" => {
                let (ch, id, text) = (a(1), a(2), a(3));
                let side = if a(4).eq_ignore_ascii_case("r") { "R" } else { "L" };
                if text.is_empty() {
                    return Err("usage: set <line|ph12|ph34|in|device> <id> <value> [R]".into());
                }
                let r = schema::resolve(ch, id, side).ok_or_else(|| format!("unknown parameter {ch} {id}"))?;
                if r.param["readonly"].as_bool().unwrap_or(false) {
                    return Err(format!("{id} is read-only"));
                }
                let k = key(r.addr, r.idx);
                let cur = self.value(&k);
                if self.usable() && cur.is_none() {
                    return Err(format!("this unit has no {id} on {ch}"));
                }
                let raw = schema::parse_value(r.param, &model, r.channel, text, cur)?;
                self.set(vec![(k.clone(), raw)])?;
                Ok(fmt(r.param, self.value(&k).unwrap_or(raw)))
            }
            "setup" => {
                let n: u8 = a(1).parse().map_err(|_| "usage: setup <1-9>")?;
                self.command(Command::LoadSetup(n))?;
                Ok(format!("loading setup {n}"))
            }
            "preset" => {
                let (ch, what) = (a(1), a(2));
                let r = schema::resolve(ch, "eqpreset", "L").ok_or_else(|| format!("unknown channel {ch}"))?;
                // raw: 0 Manual, 1 Temp, 2.. Preset 1..20
                let raw = match what.to_ascii_lowercase().as_str() {
                    "manual" => 0,
                    "temp" => 1,
                    n => n.parse::<i32>().ok().filter(|n| (1..=20).contains(n)).map(|n| n + 1).ok_or("usage: preset <line|ph12|ph34|in> <1-20|manual|temp>")?,
                };
                let k = key(r.addr, r.idx);
                self.set(vec![(k.clone(), raw)])?;
                Ok(fmt(r.param, self.value(&k).unwrap_or(raw)))
            }
            "refresh" => {
                self.command(Command::Refresh)?;
                Ok("reading".into())
            }
            "volume-up" | "volume-down" | "mute" => {
                self.key_action(a(0), true);
                Ok("ok".into())
            }
            "ping" => Ok("ok".into()),
            "show" => {
                let app = self.app.clone();
                let _ = self.app.run_on_main_thread(move || crate::show_main(&app));
                Ok("ok".into())
            }
            other => Err(format!("unknown command {other}")),
        }
    }
}

pub fn fmt_db(raw: i32) -> String {
    let v = raw as f64 / 10.0;
    let s = format!("{v:.1}");
    if v < 0.0 {
        s.replacen('-', "−", 1)
    } else if v > 0.0 {
        format!("+{s}")
    } else {
        s
    }
}
