//! App settings, ~/.config/open-adi2-control/config.json (written only when something changes).

use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::path::PathBuf;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(default)]
pub struct Connection {
    /// shown in the window's unit switch ('' = named after the mode)
    pub name: String,
    /// "usb" | "pi" | "demo"
    pub mode: String,
    /// ADI-2-Pi address: host[:port]
    pub pi_address: String,
    /// ADI-2-Pi access key ('' = none); the settings file is readable by the user only
    pub pi_key: String,
    /// demo unit: "pro" | "se" | "dac"
    pub demo_model: String,
}

impl Default for Connection {
    fn default() -> Self {
        Connection { name: String::new(), mode: "usb".into(), pi_address: String::new(), pi_key: String::new(), demo_model: "se".into() }
    }
}

impl Connection {
    pub fn label(&self) -> String {
        if !self.name.trim().is_empty() {
            return self.name.trim().to_string();
        }
        match self.mode.as_str() {
            "pi" => format!("ADI-2-Pi {}", self.pi_address),
            "demo" => "Demo".into(),
            _ => "USB".into(),
        }
    }
}

/// A named set of volume limits per output, dB (missing/null = no limit).
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Default)]
#[serde(default)]
pub struct Profile {
    pub name: String,
    pub limit: BTreeMap<String, Option<f64>>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(default)]
pub struct Volume {
    /// outputs whose volumes move together when changed in the app ("line", "ph12", "ph34")
    pub link: Vec<String>,
    /// settings files before 0.2.0: one set of limits, moved into `profiles` on load
    #[serde(skip_serializing)]
    pub limit: BTreeMap<String, Option<f64>>,
    /// volume-limit profiles; `profile` is the active one
    pub profiles: Vec<Profile>,
    pub profile: usize,
    /// limit how fast the app raises the volume, dB/s (0 = off)
    pub rise: f64,
    /// volume for the Recall button per output, dB
    pub recall: BTreeMap<String, f64>,
    /// volume keys and tray: step in dB
    pub step: f64,
}

impl Default for Volume {
    fn default() -> Self {
        Volume { link: vec![], limit: BTreeMap::new(), profiles: vec![Profile { name: "Default".into(), limit: BTreeMap::new() }], profile: 0, rise: 0.0, recall: BTreeMap::new(), step: 1.0 }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(default)]
pub struct Keys {
    /// the desktop's volume keys control the ADI-2
    pub enabled: bool,
    /// "auto" (the output last turned, on the unit or here) | "line" | "ph12" | "ph34"
    pub target: String,
    /// "output": only while the ADI-2 is the computer's sound output, else the system volume;
    /// "always": always the ADI-2
    pub when: String,
}

impl Default for Keys {
    fn default() -> Self {
        Keys { enabled: false, target: "auto".into(), when: "output".into() }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(default)]
pub struct Osd {
    pub enabled: bool,
    /// also when the volume is turned on the unit itself
    pub knob: bool,
    /// "auto" | "window" | "notification" | "kde"
    pub style: String,
}

impl Default for Osd {
    fn default() -> Self {
        Osd { enabled: true, knob: true, style: "auto".into() }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(default)]
pub struct Ui {
    /// "all" | "volume" | "off": what the mouse wheel may change
    pub wheel: String,
    /// last page shown
    pub page: String,
    /// "dark" | "light" | "system"
    pub theme: String,
    /// switch to the page of the output the unit works on (knob turned; status message on a DAC)
    pub follow: bool,
    /// smaller panels and knobs
    pub compact: bool,
}

impl Default for Ui {
    fn default() -> Self {
        Ui { wheel: "all".into(), page: "line".into(), theme: "dark".into(), follow: true, compact: false }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Default)]
#[serde(default)]
pub struct Updates {
    /// ask GitHub once a day whether a new version exists (off unless switched on)
    pub check: bool,
    /// version the user chose to skip ("" = none)
    pub skip: String,
}

/// GNOME key bindings as they were before the app took over the volume keys.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Default)]
pub struct GnomeSaved {
    pub keys: BTreeMap<String, String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(default)]
pub struct Config {
    /// settings files before 0.2.0: the one connection, moved into `units` on load
    #[serde(skip_serializing)]
    pub connection: Option<Connection>,
    /// saved connections; `active` is the one in use
    pub units: Vec<Connection>,
    pub active: usize,
    pub volume: Volume,
    pub keys: Keys,
    pub osd: Osd,
    pub ui: Ui,
    pub updates: Updates,
    pub tray: bool,
    /// closing the window keeps the app in the tray
    pub close_to_tray: bool,
    pub autostart: bool,
    /// start without the window (tray only)
    pub start_hidden: bool,
    pub gnome_saved: Option<GnomeSaved>,
}

impl Default for Config {
    fn default() -> Self {
        Config {
            connection: None,
            units: vec![Connection::default()],
            active: 0,
            volume: Volume::default(),
            keys: Keys::default(),
            osd: Osd::default(),
            ui: Ui::default(),
            updates: Updates::default(),
            tray: true,
            close_to_tray: true,
            autostart: false,
            start_hidden: false,
            gnome_saved: None,
        }
    }
}

pub fn config_dir() -> PathBuf {
    let base = std::env::var_os("XDG_CONFIG_HOME")
        .filter(|v| !v.is_empty())
        .map(PathBuf::from)
        .unwrap_or_else(|| home().join(".config"));
    base.join("open-adi2-control")
}

pub fn home() -> PathBuf {
    std::env::var_os("HOME").map(PathBuf::from).unwrap_or_else(|| PathBuf::from("/tmp"))
}

fn file() -> PathBuf {
    config_dir().join("config.json")
}

impl Config {
    pub fn load() -> Config {
        let mut c: Config = std::fs::read_to_string(file()).ok().and_then(|s| serde_json::from_str(&s).ok()).unwrap_or_default();
        c.migrate();
        c
    }

    /// Settings written by versions before 0.2.0.
    fn migrate(&mut self) {
        if let Some(conn) = self.connection.take() {
            self.units = vec![conn];
            self.active = 0;
        }
        if self.units.is_empty() {
            self.units.push(Connection::default());
        }
        if !self.volume.limit.is_empty() && self.volume.profiles.iter().all(|p| p.limit.is_empty()) {
            if self.volume.profiles.is_empty() {
                self.volume.profiles.push(Profile { name: "Default".into(), limit: BTreeMap::new() });
            }
            self.volume.profiles[0].limit = std::mem::take(&mut self.volume.limit);
        }
        if self.volume.profiles.is_empty() {
            self.volume.profiles.push(Profile { name: "Default".into(), limit: BTreeMap::new() });
        }
        self.active = self.active.min(self.units.len() - 1);
        self.volume.profile = self.volume.profile.min(self.volume.profiles.len() - 1);
    }

    pub fn unit(&self) -> &Connection {
        &self.units[self.active.min(self.units.len() - 1)]
    }

    /// Volume limit of an output in the active profile, dB.
    pub fn limit_db(&self, ch: &str) -> Option<f64> {
        self.volume.profiles.get(self.volume.profile).and_then(|p| p.limit.get(ch).copied().flatten())
    }

    pub fn save(&self) {
        let dir = config_dir();
        let _ = std::fs::create_dir_all(&dir);
        let tmp = dir.join("config.json.tmp");
        if let Ok(s) = serde_json::to_string_pretty(self) {
            if std::fs::write(&tmp, s + "\n").is_ok() {
                // holds the ADI-2-Pi access key
                use std::os::unix::fs::PermissionsExt;
                let _ = std::fs::set_permissions(&tmp, std::fs::Permissions::from_mode(0o600));
                let _ = std::fs::rename(&tmp, file());
            }
        }
    }

    /// Apply a partial update (JSON object merged into the current settings).
    pub fn patched(&self, patch: &serde_json::Value) -> Result<Config, String> {
        let mut cur = serde_json::to_value(self).map_err(|e| e.to_string())?;
        merge(&mut cur, patch);
        let mut c: Config = serde_json::from_value(cur).map_err(|e| e.to_string())?;
        c.migrate();
        c.volume.step = c.volume.step.clamp(0.5, 6.0);
        c.volume.rise = c.volume.rise.clamp(0.0, 100.0);
        for p in &mut c.volume.profiles {
            p.name = p.name.trim().chars().take(24).collect();
            if p.name.is_empty() {
                p.name = "Profile".into();
            }
        }
        for u in &mut c.units {
            u.name = u.name.trim().chars().take(24).collect();
        }
        Ok(c)
    }
}

fn merge(a: &mut serde_json::Value, b: &serde_json::Value) {
    match (a, b) {
        (serde_json::Value::Object(a), serde_json::Value::Object(b)) => {
            for (k, v) in b {
                merge(a.entry(k.clone()).or_insert(serde_json::Value::Null), v);
            }
        }
        (a, b) => *a = b.clone(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn migrates_old_settings() {
        let old = r#"{"connection":{"mode":"pi","pi_address":"adi2-pi.local"},"volume":{"limit":{"ph12":0.0,"line":null}}}"#;
        let mut c: Config = serde_json::from_str(old).unwrap();
        c.migrate();
        assert_eq!(c.units.len(), 1);
        assert_eq!(c.unit().mode, "pi");
        assert_eq!(c.limit_db("ph12"), Some(0.0));
        assert_eq!(c.limit_db("line"), None);
        let json = serde_json::to_value(&c).unwrap();
        assert!(json.get("connection").is_none());
        assert!(json["volume"].get("limit").is_none());
        // a patch keeps the migrated shape
        let p = c.patched(&serde_json::json!({ "volume": { "profile": 5 } })).unwrap();
        assert_eq!(p.volume.profile, 0);
    }
}
