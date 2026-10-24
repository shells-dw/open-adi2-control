//! Volume popup.
//!
//! - KDE Plasma: Plasma's own on-screen display (org.kde.osdService).
//! - X11: a small always-on-top window of the app at the bottom of the screen.
//! - Elsewhere (GNOME and other Wayland desktops don't let apps place windows): a desktop
//!   notification that replaces the previous one, with a level bar where the notification
//!   server shows one.

use crate::config::Config;
use serde::Serialize;
use std::collections::HashMap;
use std::sync::atomic::{AtomicU32, AtomicU64, Ordering};
use std::sync::Mutex;
use std::time::Duration;
use tauri::{AppHandle, Emitter, Manager, PhysicalPosition};
use zbus::zvariant::Value;

#[derive(Debug, Clone, Serialize)]
pub struct Info {
    pub title: String,
    pub value: String,
    /// 0…1 for the level bar
    pub frac: Option<f64>,
    pub muted: bool,
}

pub const WINDOW: &str = "osd";
const SHOW_FOR: Duration = Duration::from_millis(1600);

static NOTIFY_ID: AtomicU32 = AtomicU32::new(0);
static SHOW_SEQ: AtomicU64 = AtomicU64::new(0);
static BUS: Mutex<Option<zbus::blocking::Connection>> = Mutex::new(None);

fn bus() -> Option<zbus::blocking::Connection> {
    let mut b = BUS.lock().unwrap();
    if b.is_none() {
        *b = zbus::blocking::Connection::session().ok();
    }
    b.clone()
}

pub fn is_kde() -> bool {
    std::env::var("XDG_CURRENT_DESKTOP").unwrap_or_default().to_ascii_uppercase().contains("KDE")
}

pub fn is_x11() -> bool {
    std::env::var_os("WAYLAND_DISPLAY").is_none() || std::env::var("GDK_BACKEND").map(|b| b == "x11").unwrap_or(false)
}

/// What "auto" means on this desktop.
pub fn auto_style() -> &'static str {
    if is_kde() {
        "kde"
    } else if is_x11() {
        "window"
    } else {
        "notification"
    }
}

pub fn show(app: &AppHandle, cfg: &Config, info: Info) {
    let style = if cfg.osd.style == "auto" { auto_style().to_string() } else { cfg.osd.style.clone() };
    let app = app.clone();
    // D-Bus calls and window work off the caller's thread
    std::thread::spawn(move || {
        let ok = match style.as_str() {
            "kde" => kde(&info),
            "window" => window(&app, &info),
            _ => false,
        };
        if !ok {
            notify(&info);
        }
    });
}

fn kde(info: &Info) -> bool {
    let Some(c) = bus() else { return false };
    let icon = if info.muted { "audio-volume-muted" } else { "audio-volume-high" };
    let text = format!("{}  {}", info.title, info.value);
    c.call_method(Some("org.kde.plasmashell"), "/org/kde/osdService", Some("org.kde.osdService"), "showText", &(icon, text.as_str()))
        .is_ok()
}

fn notify(info: &Info) -> bool {
    let Some(c) = bus() else { return false };
    let mut hints: HashMap<&str, Value> = HashMap::new();
    hints.insert("transient", Value::from(true));
    hints.insert("x-canonical-private-synchronous", Value::from("adi2-volume"));
    hints.insert("category", Value::from("device"));
    if let Some(f) = info.frac {
        hints.insert("value", Value::from((f * 100.0).round() as i32));
    }
    let icon = if info.muted { "audio-volume-muted" } else { "audio-volume-high" };
    let actions: Vec<&str> = vec![];
    let reply = c.call_method(
        Some("org.freedesktop.Notifications"),
        "/org/freedesktop/Notifications",
        Some("org.freedesktop.Notifications"),
        "Notify",
        &("Open ADI-2 Control", NOTIFY_ID.load(Ordering::SeqCst), icon, info.title.as_str(), info.value.as_str(), actions, hints, 1500i32),
    );
    match reply.and_then(|m| m.body().deserialize::<u32>().map_err(Into::into)) {
        Ok(id) => {
            NOTIFY_ID.store(id, Ordering::SeqCst);
            true
        }
        Err(_) => false,
    }
}

fn place(app: &AppHandle, w: &tauri::WebviewWindow) {
    let mon = app
        .cursor_position()
        .ok()
        .and_then(|p| app.monitor_from_point(p.x, p.y).ok().flatten())
        .or_else(|| app.primary_monitor().ok().flatten());
    if let (Some(m), Ok(size)) = (mon, w.outer_size()) {
        let x = m.position().x + (m.size().width as i32 - size.width as i32) / 2;
        let y = m.position().y + m.size().height as i32 - size.height as i32 - (m.size().height as f64 * 0.12) as i32;
        let _ = w.set_position(PhysicalPosition::new(x, y));
    }
}

fn window(app: &AppHandle, info: &Info) -> bool {
    let Some(w) = app.get_webview_window(WINDOW) else { return false };
    // bottom centre of the monitor the pointer is on (or the primary one)
    place(app, &w);
    let _ = w.emit("osd", info);
    let _ = w.show();
    place(app, &w); // the size is only certain once shown
    let _ = w.set_always_on_top(true);
    let seq = SHOW_SEQ.fetch_add(1, Ordering::SeqCst) + 1;
    std::thread::sleep(SHOW_FOR);
    if SHOW_SEQ.load(Ordering::SeqCst) == seq {
        let _ = w.hide();
    }
    true
}
