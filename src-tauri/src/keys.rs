//! Volume keys.
//!
//! Desktops don't let an app grab the volume keys away from them, so the keys are bound to
//! commands instead: `open-adi2-control volume-up | volume-down | mute`. Such a command reaches the
//! running app through a socket in $XDG_RUNTIME_DIR and returns at once (no window toolkit is
//! started for it). When the ADI-2 isn't what the computer plays to, or the app isn't running,
//! the command changes the system volume instead, so the keys keep working for other outputs.
//!
//! On GNOME the app sets the bindings up itself (and puts the old ones back when switched off).
//! Other desktops: bind the keys to the commands in their keyboard settings.

use crate::config::{Config, GnomeSaved};
use crate::osd;
use std::io::{Read, Write};
use std::os::unix::net::{UnixListener, UnixStream};
use std::path::PathBuf;
use std::process::Command;
use std::time::Duration;

pub const ACTIONS: [&str; 3] = ["volume-up", "volume-down", "mute"];

pub fn socket_path() -> PathBuf {
    match std::env::var_os("XDG_RUNTIME_DIR").filter(|v| !v.is_empty()) {
        Some(d) => PathBuf::from(d).join("open-adi2-control.sock"),
        None => std::env::temp_dir().join(format!("open-adi2-control-{}.sock", unsafe_uid())),
    }
}

fn unsafe_uid() -> String {
    std::env::var("UID").or_else(|_| std::env::var("USER")).unwrap_or_else(|_| "user".into())
}

/// Send a command to the running app. Ok = it took it.
pub fn send_to_running(cmd: &str) -> bool {
    let Ok(mut s) = UnixStream::connect(socket_path()) else { return false };
    s.set_write_timeout(Some(Duration::from_millis(500))).ok();
    s.set_read_timeout(Some(Duration::from_millis(1500))).ok();
    if s.write_all(format!("{cmd}\n").as_bytes()).is_err() {
        return false;
    }
    let mut reply = [0u8; 2];
    matches!(s.read(&mut reply), Ok(n) if n > 0)
}

/// Listen for commands; `handle` runs for each line.
pub fn serve(handle: impl Fn(String) + Send + 'static) -> std::io::Result<()> {
    let path = socket_path();
    // a socket left over from a crash: nobody answers on it
    if path.exists() && UnixStream::connect(&path).is_err() {
        let _ = std::fs::remove_file(&path);
    }
    let listener = UnixListener::bind(&path)?;
    std::thread::Builder::new().name("commands".into()).spawn(move || {
        for conn in listener.incoming().flatten() {
            let mut conn = conn;
            conn.set_read_timeout(Some(Duration::from_millis(500))).ok();
            let mut buf = [0u8; 64];
            if let Ok(n) = conn.read(&mut buf) {
                let line = String::from_utf8_lossy(&buf[..n]).trim().to_string();
                let _ = conn.write_all(b"ok");
                if !line.is_empty() {
                    handle(line);
                }
            }
        }
    })?;
    Ok(())
}

pub fn remove_socket() {
    let _ = std::fs::remove_file(socket_path());
}

// ---------------------------------------------------------------- system volume

fn run(cmd: &str, args: &[&str]) -> Option<String> {
    let out = Command::new(cmd).args(args).output().ok()?;
    if !out.status.success() {
        return None;
    }
    Some(String::from_utf8_lossy(&out.stdout).to_string())
}

/// Is the computer's default sound output the ADI-2? (PipeWire's wpctl, or pactl.)
/// Neither tool there: can't tell, so yes.
pub fn adi_is_default_output() -> bool {
    let looks_like_adi = |s: &str| {
        let s = s.to_ascii_uppercase();
        s.contains("ADI-2") || s.contains("ADI_2") || s.contains("ADI2")
    };
    if let Some(info) = run("wpctl", &["inspect", "@DEFAULT_AUDIO_SINK@"]) {
        return info.lines().any(|l| (l.contains("node.name") || l.contains("node.description") || l.contains("alsa.card_name")) && looks_like_adi(l));
    }
    if let Some(name) = run("pactl", &["get-default-sink"]) {
        return looks_like_adi(&name);
    }
    true
}

/// Change the system volume the way the volume keys normally do; returns what to show.
pub fn system_volume(action: &str) -> Option<osd::Info> {
    let wp = match action {
        "volume-up" => run("wpctl", &["set-volume", "-l", "1.0", "@DEFAULT_AUDIO_SINK@", "5%+"]),
        "volume-down" => run("wpctl", &["set-volume", "@DEFAULT_AUDIO_SINK@", "5%-"]),
        "mute" => run("wpctl", &["set-mute", "@DEFAULT_AUDIO_SINK@", "toggle"]),
        _ => return None,
    };
    if wp.is_some() {
        // "Volume: 0.45 [MUTED]"
        let v = run("wpctl", &["get-volume", "@DEFAULT_AUDIO_SINK@"]).unwrap_or_default();
        let muted = v.contains("MUTED");
        let frac = v.split_whitespace().nth(1).and_then(|x| x.parse::<f64>().ok());
        return Some(osd::Info {
            title: "Volume".into(),
            value: if muted { "Muted".into() } else { frac.map(|f| format!("{:.0} %", f * 100.0)).unwrap_or_default() },
            frac: frac.map(|f| f.clamp(0.0, 1.0)),
            muted,
        });
    }
    let ok = match action {
        "volume-up" => run("pactl", &["set-sink-volume", "@DEFAULT_SINK@", "+5%"]),
        "volume-down" => run("pactl", &["set-sink-volume", "@DEFAULT_SINK@", "-5%"]),
        _ => run("pactl", &["set-sink-mute", "@DEFAULT_SINK@", "toggle"]),
    };
    ok.map(|_| osd::Info { title: "Volume".into(), value: String::new(), frac: None, muted: false })
}

// ---------------------------------------------------------------- binding the keys

/// Path to start this app with: the AppImage itself when running from one.
pub fn exe_path() -> String {
    std::env::var("APPIMAGE")
        .ok()
        .filter(|s| !s.is_empty())
        .or_else(|| std::env::current_exe().ok().map(|p| p.to_string_lossy().into_owned()))
        .unwrap_or_else(|| "open-adi2-control".into())
}

pub fn command_line(action: &str) -> String {
    let exe = exe_path();
    if exe.contains(' ') || exe.contains('\'') {
        format!("\"{}\" {action}", exe.replace('"', "\\\""))
    } else {
        format!("{exe} {action}")
    }
}

const MK: &str = "org.gnome.settings-daemon.plugins.media-keys";
const CUSTOM: &str = "org.gnome.settings-daemon.plugins.media-keys.custom-keybinding";
const PATH_BASE: &str = "/org/gnome/settings-daemon/plugins/media-keys/custom-keybindings/open-adi2-control-";
/// GNOME 3.34+: the hardware keys are the "-static" keys; older ones used the plain keys.
const STATIC_KEYS: [&str; 3] = ["volume-up-static", "volume-down-static", "volume-mute-static"];
const KEYSYMS: [&str; 3] = ["XF86AudioRaiseVolume", "XF86AudioLowerVolume", "XF86AudioMute"];

fn gsettings(args: &[&str]) -> Option<String> {
    run("gsettings", args).map(|s| s.trim().to_string())
}

pub fn desktop() -> String {
    std::env::var("XDG_CURRENT_DESKTOP").unwrap_or_default()
}

pub fn gnome_available() -> bool {
    gsettings(&["list-keys", MK]).map(|k| k.lines().any(|l| l == "custom-keybindings")).unwrap_or(false)
}

/// Parse a GVariant string array as gsettings prints it: ['a', 'b'] / @as [].
fn parse_strv(s: &str) -> Vec<String> {
    s.trim_start_matches("@as").trim().trim_start_matches('[').trim_end_matches(']').split(',').map(|x| x.trim().trim_matches('\'').to_string()).filter(|x| !x.is_empty()).collect()
}

fn strv(list: &[String]) -> String {
    format!("[{}]", list.iter().map(|x| format!("'{x}'")).collect::<Vec<_>>().join(", "))
}

fn our_paths() -> Vec<String> {
    ACTIONS.iter().map(|a| format!("{PATH_BASE}{a}/")).collect()
}

/// Take over the volume keys on GNOME. Saves the previous bindings in `cfg.gnome_saved`.
pub fn gnome_apply(cfg: &mut Config) -> Result<(), String> {
    if !gnome_available() {
        return Err("GNOME key settings not found".into());
    }
    let have = gsettings(&["list-keys", MK]).unwrap_or_default();
    let static_keys: Vec<&str> = if have.lines().any(|l| l == "volume-up-static") {
        STATIC_KEYS.to_vec()
    } else {
        vec!["volume-up", "volume-down", "volume-mute"]
    };
    if cfg.gnome_saved.is_none() {
        let mut saved = GnomeSaved::default();
        for k in &static_keys {
            if let Some(v) = gsettings(&["get", MK, k]) {
                saved.keys.insert((*k).to_string(), v);
            }
        }
        cfg.gnome_saved = Some(saved);
    }
    for k in &static_keys {
        let empty = if gsettings(&["get", MK, k]).map(|v| v.starts_with('\'')).unwrap_or(false) { "''" } else { "[]" };
        gsettings(&["set", MK, k, empty]).ok_or(format!("can't change {k}"))?;
    }
    let mut list = parse_strv(&gsettings(&["get", MK, "custom-keybindings"]).unwrap_or_default());
    for p in our_paths() {
        if !list.contains(&p) {
            list.push(p);
        }
    }
    for (i, a) in ACTIONS.iter().enumerate() {
        let schema = format!("{CUSTOM}:{PATH_BASE}{a}/");
        gsettings(&["set", &schema, "name", &format!("Open ADI-2 Control: {a}")]);
        gsettings(&["set", &schema, "command", &command_line(a)]);
        gsettings(&["set", &schema, "binding", KEYSYMS[i]]);
    }
    gsettings(&["set", MK, "custom-keybindings", &strv(&list)]).ok_or("can't add key bindings")?;
    Ok(())
}

/// Give the volume keys back to GNOME.
pub fn gnome_restore(cfg: &mut Config) {
    if !gnome_available() {
        return;
    }
    let list: Vec<String> = parse_strv(&gsettings(&["get", MK, "custom-keybindings"]).unwrap_or_default())
        .into_iter()
        .filter(|p| !p.starts_with(PATH_BASE))
        .collect();
    gsettings(&["set", MK, "custom-keybindings", &strv(&list)]);
    for a in ACTIONS {
        let schema = format!("{CUSTOM}:{PATH_BASE}{a}/");
        for k in ["name", "command", "binding"] {
            gsettings(&["reset", &schema, k]);
        }
    }
    match cfg.gnome_saved.take() {
        Some(saved) => {
            for (k, v) in saved.keys {
                gsettings(&["set", MK, &k, &v]);
            }
        }
        None => {
            for k in STATIC_KEYS {
                gsettings(&["reset", MK, k]);
            }
        }
    }
}

/// Are our GNOME bindings in place?
pub fn gnome_active() -> bool {
    let list = parse_strv(&gsettings(&["get", MK, "custom-keybindings"]).unwrap_or_default());
    our_paths().iter().all(|p| list.contains(p))
}

#[derive(serde::Serialize)]
pub struct KeysStatus {
    pub desktop: String,
    /// "gnome": set up automatically; "manual": bind the commands yourself
    pub method: String,
    pub active: bool,
    pub commands: Vec<(String, String)>,
}

pub fn status() -> KeysStatus {
    let gnome = gnome_available();
    KeysStatus {
        desktop: desktop(),
        method: if gnome { "gnome".into() } else { "manual".into() },
        active: gnome && gnome_active(),
        commands: ACTIONS.iter().map(|a| (a.to_string(), command_line(a))).collect(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn strv_round_trip() {
        assert_eq!(parse_strv("@as []"), Vec::<String>::new());
        let v = parse_strv("['/a/', '/b/']");
        assert_eq!(v, vec!["/a/".to_string(), "/b/".to_string()]);
        assert_eq!(strv(&v), "['/a/', '/b/']");
    }
}

#[cfg(test)]
mod gnome_tests {
    use super::*;
    /// Needs gsettings with the GNOME media-keys schema and a session bus:
    /// `dbus-run-session -- cargo test -- --ignored gnome`
    #[test]
    #[ignore]
    fn gnome_apply_and_restore() {
        assert!(gnome_available());
        gsettings(&["set", MK, "custom-keybindings", "['/org/gnome/settings-daemon/plugins/media-keys/custom-keybindings/custom0/']"]);
        let before: Vec<_> = STATIC_KEYS.iter().map(|k| gsettings(&["get", MK, k]).unwrap()).collect();
        let mut cfg = Config::default();
        gnome_apply(&mut cfg).unwrap();
        assert!(gnome_active());
        assert_eq!(gsettings(&["get", MK, "volume-up-static"]).unwrap(), "@as []");
        let list = parse_strv(&gsettings(&["get", MK, "custom-keybindings"]).unwrap());
        assert_eq!(list.len(), 4, "user's own binding kept: {list:?}");
        let up = format!("{CUSTOM}:{PATH_BASE}volume-up/");
        assert_eq!(gsettings(&["get", &up, "binding"]).unwrap(), "'XF86AudioRaiseVolume'");
        assert!(gsettings(&["get", &up, "command"]).unwrap().ends_with(" volume-up'"));
        // applying twice keeps the first saved state
        gnome_apply(&mut cfg).unwrap();
        gnome_restore(&mut cfg);
        assert!(!gnome_active());
        let after: Vec<_> = STATIC_KEYS.iter().map(|k| gsettings(&["get", MK, k]).unwrap()).collect();
        assert_eq!(before, after);
        assert_eq!(parse_strv(&gsettings(&["get", MK, "custom-keybindings"]).unwrap()).len(), 1);
    }
}
