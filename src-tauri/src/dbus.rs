//! D-Bus interface on the session bus, for scripts and desktop widgets.
//!
//! Name `io.github.shells_dw.OpenAdi2Control`, object `/io/github/shells_dw/OpenAdi2Control`,
//! interface `io.github.shells_dw.OpenAdi2Control1`:
//!   Run(as args) → s   the same commands as the command line (`get`, `set`, `setup`, `preset`,
//!                      `refresh`, `volume-up`, `volume-down`, `mute`, `show`); errors come back
//!                      as `io.github.shells_dw.OpenAdi2Control1.Error`
//!   Get(s channel, s id) → s   one value, as `get <channel> <id>`
//!
//! Example: `busctl --user call io.github.shells_dw.OpenAdi2Control /io/github/shells_dw/OpenAdi2Control
//! io.github.shells_dw.OpenAdi2Control1 Run as 3 set ph12 vol -30`

use crate::hub::Hub;
use std::sync::Arc;

pub const NAME: &str = "io.github.shells_dw.OpenAdi2Control";
pub const PATH: &str = "/io/github/shells_dw/OpenAdi2Control";

#[derive(Debug, zbus::DBusError)]
#[zbus(prefix = "io.github.shells_dw.OpenAdi2Control1")]
enum Error {
    #[zbus(error)]
    ZBus(zbus::Error),
    Error(String),
}

struct Remote {
    hub: Arc<Hub>,
}

#[zbus::interface(name = "io.github.shells_dw.OpenAdi2Control1")]
impl Remote {
    fn run(&self, args: Vec<String>) -> Result<String, Error> {
        if args.is_empty() {
            return Err(Error::Error("no command".into()));
        }
        self.hub.cli(&args).map_err(Error::Error)
    }

    fn get(&self, channel: String, id: String) -> Result<String, Error> {
        self.hub.cli(&["get".into(), channel, id]).map_err(Error::Error)
    }
}

/// Register on the session bus. The connection lives as long as the app (kept in a thread).
pub fn start(hub: Arc<Hub>) {
    let _ = std::thread::Builder::new().name("dbus".into()).spawn(move || {
        let conn = zbus::blocking::connection::Builder::session()
            .and_then(|b| b.name(NAME))
            .and_then(|b| b.serve_at(PATH, Remote { hub }))
            .and_then(|b| b.build());
        match conn {
            Ok(c) => loop {
                // the connection serves calls on its own executor; keep it alive
                std::thread::park();
                let _ = &c;
            },
            Err(e) => eprintln!("D-Bus interface: {e}"),
        }
    });
}
