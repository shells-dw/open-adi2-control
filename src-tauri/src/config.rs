//! App settings, ~/.config/open-adi2-control/config.json (written only when something changes).

use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::path::PathBuf;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(default)]
pub struct Connection {
    /// "usb" | "pi" | "demo"
    pub mode: String,
    /// ADI-2-Pi address: host[:port]
    pub pi_address: String,
    /// demo unit: "pro" | "se" | "dac"
    pub demo_model: String,
}

impl Default for Connection {
    fn default() -> Self {
        Connection { mode: "usb".into(), pi_address: String::new(), demo_model: "se".into() }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(default)]
pub struct Volume {
    /// outputs whose volumes move together when changed in the app ("line", "ph12", "ph34")
    pub link: Vec<String>,
    /// highest volume the app sets per output, dB (missing/null = no limit)
    pub limit: BTreeMap<String, Option<f64>>,
    /// limit how fast the app raises the volume, dB/s (0 = off)
    pub rise: f64,
    /// volume for the Recall button per output, dB
    pub recall: BTreeMap<String, f64>,
    /// volume keys and tray: step in dB
    pub step: f64,
}

impl Default for Volume {
    fn default() -> Self {
        Volume { link: vec![], limit: BTreeMap::new(), rise: 0.0, recall: BTreeMap::new(), step: 1.0 }
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
    pub theme: String,
}

impl Default for Ui {
    fn default() -> Self {
        Ui { wheel: "all".into(), page: "line".into(), theme: "dark".into() }
    }
}

/// GNOME key bindings as they were before the app took over the volume keys.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Default)]
pub struct GnomeSaved {
    pub keys: BTreeMap<String, String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(default)]
pub struct Config {
    pub connection: Connection,
    pub volume: Volume,
    pub keys: Keys,
    pub osd: Osd,
    pub ui: Ui,
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
            connection: Connection::default(),
            volume: Volume::default(),
            keys: Keys::default(),
            osd: Osd::default(),
            ui: Ui::default(),
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
        std::fs::read_to_string(file()).ok().and_then(|s| serde_json::from_str(&s).ok()).unwrap_or_default()
    }

    pub fn save(&self) {
        let dir = config_dir();
        let _ = std::fs::create_dir_all(&dir);
        let tmp = dir.join("config.json.tmp");
        if let Ok(s) = serde_json::to_string_pretty(self) {
            if std::fs::write(&tmp, s + "\n").is_ok() {
                let _ = std::fs::rename(&tmp, file());
            }
        }
    }

    /// Apply a partial update (JSON object merged into the current settings).
    pub fn patched(&self, patch: &serde_json::Value) -> Result<Config, String> {
        let mut cur = serde_json::to_value(self).map_err(|e| e.to_string())?;
        merge(&mut cur, patch);
        let mut c: Config = serde_json::from_value(cur).map_err(|e| e.to_string())?;
        c.volume.step = c.volume.step.clamp(0.5, 6.0);
        c.volume.rise = c.volume.rise.clamp(0.0, 100.0);
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
