//! Re-read the unit after the computer wakes up or the screen is unlocked.
//!
//! Sources: `PrepareForSleep(false)` from logind on the system bus, and `ActiveChanged(false)`
//! from org.freedesktop.ScreenSaver / org.gnome.ScreenSaver on the session bus. Each signal
//! starts one watcher thread; a bus that isn't there is skipped.

use crate::hub::Hub;
use crate::model::Command;
use std::sync::Arc;
use std::time::Duration;
use zbus::blocking::{Connection, Proxy};

/// How long after waking to wait before reading (USB devices need a moment to enumerate).
const AFTER_WAKE: Duration = Duration::from_millis(2500);
const AFTER_UNLOCK: Duration = Duration::from_millis(500);

pub fn start(hub: Arc<Hub>) {
    let h = hub.clone();
    spawn("wake-logind", move || {
        let conn = Connection::system()?;
        let proxy = Proxy::new(&conn, "org.freedesktop.login1", "/org/freedesktop/login1", "org.freedesktop.login1.Manager")?;
        for msg in proxy.receive_signal("PrepareForSleep")? {
            let going_to_sleep: bool = msg.body().deserialize()?;
            if !going_to_sleep {
                refresh(&h, AFTER_WAKE);
            }
        }
        Ok(())
    });
    for (name, path) in [
        ("org.freedesktop.ScreenSaver", "/org/freedesktop/ScreenSaver"),
        ("org.gnome.ScreenSaver", "/org/gnome/ScreenSaver"),
    ] {
        let h = hub.clone();
        spawn("wake-screensaver", move || {
            let conn = Connection::session()?;
            // match on the signal only; the service may not be running yet when the app starts
            let proxy = Proxy::new(&conn, name, path, name)?;
            for msg in proxy.receive_signal("ActiveChanged")? {
                let active: bool = msg.body().deserialize()?;
                if !active {
                    refresh(&h, AFTER_UNLOCK);
                }
            }
            Ok(())
        });
    }
}

fn refresh(hub: &Arc<Hub>, after: Duration) {
    let hub = hub.clone();
    std::thread::spawn(move || {
        std::thread::sleep(after);
        let _ = hub.command(Command::Refresh);
    });
}

fn spawn(name: &'static str, f: impl FnOnce() -> zbus::Result<()> + Send + 'static) {
    let _ = std::thread::Builder::new().name(name.into()).spawn(move || {
        if let Err(e) = f() {
            eprintln!("{name}: {e}");
        }
    });
}
