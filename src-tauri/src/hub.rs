//! The app's centre: runs the connection (USB, ADI-2-Pi or demo), keeps the model, and passes
//! changes to the window, the tray and the volume popup. App-side volume extras like ADI-2
//! Remote's (limit, link, rise speed, recall) live here, so they apply to the window, the tray
//! and the volume keys alike.

use crate::config::Config;
use crate::device::{self, In};
use crate::model::{empty_presets, key, Command, DeviceInfo, Origin, Preset, Update};
use crate::protocol::Status;
use crate::schema::{self, ids, Channel};
use crate::{keys, mock, osd, pi, seq, tray};
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
        })
    }

    pub fn config(&self) -> Config {
        self.cfg.lock().unwrap().clone()
    }

    // ---------------------------------------------------------------- connection

    /// (Re)start the connection the settings ask for.
    pub fn connect(self: &Arc<Self>) {
        let conn = self.config().connection;
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
                std::thread::Builder::new().name("adi2-pi".into()).spawn(move || pi::run(addr, rx, out)).ok();
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
                    for (k, v) in list {
                        if m.p.get(&k) == Some(&v) {
                            continue;
                        }
                        m.p.insert(k.clone(), v);
                        if let Some(c) = schema::outputs().find(|c| key(c.addr, ids::VOL) == k || key(c.addr, ids::MUTE) == k || key(c.addr, ids::DIM) == k) {
                            tray_dirty = true;
                            if k == vol_key(c) {
                                vol_changed = Some(c.key.clone());
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
                    // (the first read-out of all settings doesn't count as turning)
                    if let Some(ch) = vol_changed.filter(|_| self.model.lock().unwrap().device.ready) {
                        *self.last_out.lock().unwrap() = ch.clone();
                        let cfg = self.config();
                        // the knob on the unit (or another remote): show the popup
                        if origin == Origin::Device && cfg.osd.enabled && cfg.osd.knob && self.model.lock().unwrap().device.ready {
                            if let Some(info) = self.osd_info(&ch) {
                                osd::show(&self.app, &cfg, info);
                            }
                        }
                    }
                    json!({ "kind": "params", "p": changed, "origin": origin })
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
        })
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
        if let Some(Some(db)) = self.cfg.lock().unwrap().volume.limit.get(ch) {
            v = v.min((db * 10.0).round() as i32);
        }
        v
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
        let to_adi = !from_key || cfg.connection.mode != "usb" || cfg.keys.when == "always" || keys::adi_is_default_output();
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
