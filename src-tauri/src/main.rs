//! Open ADI-2 Control – a remote for RME ADI-2 series converters on Linux.
//!
//!   open-adi2-control                     start (or show the running app)
//!   open-adi2-control --hidden            start in the tray only
//!   open-adi2-control volume-up | volume-down | mute
//!                                    for the desktop's volume keys (see keys.rs)

mod config;
mod device;
mod hub;
mod keys;
mod mock;
mod model;
mod osd;
mod pi;
mod protocol;
mod schema;
mod seq;
mod tray;

use config::Config;
use hub::Hub;
use model::Command;
use serde_json::Value;
use std::collections::BTreeMap;
use std::sync::Arc;
use tauri::{AppHandle, Manager, WebviewUrl, WebviewWindowBuilder, WindowEvent};

pub fn show_main(app: &AppHandle) {
    if let Some(w) = app.get_webview_window("main") {
        let _ = w.show();
        let _ = w.unminimize();
        let _ = w.set_focus();
    } else {
        let _ = main_window(app).map(|w| w.show());
    }
}

pub fn quit(app: &AppHandle) {
    app.state::<Arc<Hub>>().stop();
    keys::remove_socket();
    app.exit(0);
}

fn main_window(app: &AppHandle) -> tauri::Result<tauri::WebviewWindow> {
    WebviewWindowBuilder::new(app, "main", WebviewUrl::App("index.html".into()))
        .title("Open ADI-2 Control")
        .inner_size(1060.0, 760.0)
        .min_inner_size(720.0, 520.0)
        .visible(false)
        .build()
}

fn osd_window(app: &AppHandle) -> tauri::Result<tauri::WebviewWindow> {
    WebviewWindowBuilder::new(app, osd::WINDOW, WebviewUrl::App("osd.html".into()))
        .title("ADI-2 volume")
        .inner_size(340.0, 84.0)
        .resizable(false)
        .decorations(false)
        .transparent(true)
        .always_on_top(true)
        .skip_taskbar(true)
        .focused(false)
        .visible(false)
        .shadow(false)
        .build()
}

// ---------------------------------------------------------------- autostart

fn autostart_file() -> std::path::PathBuf {
    let base = std::env::var_os("XDG_CONFIG_HOME").filter(|v| !v.is_empty()).map(Into::into).unwrap_or_else(|| config::home().join(".config"));
    std::path::PathBuf::from(base).join("autostart").join("open-adi2-control.desktop")
}

fn set_autostart(on: bool) -> Result<(), String> {
    let f = autostart_file();
    if !on {
        let _ = std::fs::remove_file(&f);
        return Ok(());
    }
    std::fs::create_dir_all(f.parent().unwrap()).map_err(|e| e.to_string())?;
    let exe = keys::exe_path();
    let exec = if exe.contains(' ') { format!("\"{exe}\"") } else { exe };
    let text = format!(
        "[Desktop Entry]\nType=Application\nName=Open ADI-2 Control\nComment=Remote for RME ADI-2 series converters\nExec={exec} --hidden\nIcon=open-adi2-control\nTerminal=false\nX-GNOME-Autostart-enabled=true\n"
    );
    std::fs::write(&f, text).map_err(|e| e.to_string())
}

// ---------------------------------------------------------------- commands for the window

type Res<T> = Result<T, String>;

#[tauri::command]
fn snapshot(hub: tauri::State<Arc<Hub>>) -> Value {
    hub.snapshot()
}

#[tauri::command]
fn schema_json() -> String {
    schema::SCHEMA_JSON.to_string()
}

#[tauri::command]
fn set(hub: tauri::State<Arc<Hub>>, items: Vec<(String, i32)>) -> Res<()> {
    hub.inner().set(items)
}

#[tauri::command]
fn setup(hub: tauri::State<Arc<Hub>>, action: String, n: u8, name: Option<String>) -> Res<()> {
    let c = match action.as_str() {
        "load" => Command::LoadSetup(n),
        "save" => Command::SaveSetup(n, name),
        "rename" => Command::RenameSetup(n, name.unwrap_or_default()),
        "factory" => Command::LoadFactory,
        _ => return Err("unknown setup action".into()),
    };
    hub.command(c)
}

#[tauri::command]
fn store_preset(hub: tauri::State<Arc<Hub>>, n: u8, name: Option<String>, l: BTreeMap<u8, i32>, r: BTreeMap<u8, i32>, dual: bool) -> Res<()> {
    hub.command(Command::StorePreset { n, name, l, r, dual })
}

#[tauri::command]
fn rename_preset(hub: tauri::State<Arc<Hub>>, n: u8, name: String) -> Res<()> {
    hub.command(Command::RenamePreset(n, name))
}

#[tauri::command]
fn refresh(hub: tauri::State<Arc<Hub>>) -> Res<()> {
    hub.command(Command::Refresh)
}

#[tauri::command]
fn set_config(app: AppHandle, hub: tauri::State<Arc<Hub>>, patch: Value) -> Res<Config> {
    let old = hub.config();
    let mut new = old.patched(&patch)?;
    let mut problems: Vec<String> = Vec::new();
    if new.keys.enabled != old.keys.enabled && keys::gnome_available() {
        if new.keys.enabled {
            if let Err(e) = keys::gnome_apply(&mut new) {
                problems.push(e);
                new.keys.enabled = false;
            }
        } else {
            keys::gnome_restore(&mut new);
        }
    }
    if new.autostart != old.autostart {
        if let Err(e) = set_autostart(new.autostart) {
            problems.push(format!("start at login: {e}"));
            new.autostart = old.autostart;
        }
    }
    *hub.cfg.lock().unwrap() = new.clone();
    new.save();
    if new.tray != old.tray {
        if new.tray {
            let _ = tray::create(&app);
        } else {
            tray::remove(&app);
        }
    }
    if new.connection != old.connection {
        hub.inner().connect();
    }
    tray::refresh(&app);
    hub.emit_config();
    if problems.is_empty() {
        Ok(new)
    } else {
        Err(problems.join("; "))
    }
}

#[tauri::command]
async fn discover() -> Res<Vec<pi::Found>> {
    tauri::async_runtime::spawn_blocking(|| pi::discover(std::time::Duration::from_millis(2500))).await.map_err(|e| e.to_string())?
}

#[tauri::command]
fn keys_status() -> keys::KeysStatus {
    keys::status()
}

#[tauri::command]
fn osd_test(app: AppHandle, hub: tauri::State<Arc<Hub>>) {
    let cfg = hub.config();
    let info = hub
        .target_output()
        .and_then(|c| hub.osd_info(&c.key))
        .unwrap_or(osd::Info { title: "ADI-2".into(), value: "−20.0 dB".into(), frac: Some(0.7), muted: false });
    osd::show(&app, &cfg, info);
}

#[tauri::command]
fn osd_style() -> &'static str {
    osd::auto_style()
}

pub const PROJECT_URL: &str = "https://github.com/shells-dw/open-adi2-control";

/// The project page in the default browser (only this fixed address).
#[tauri::command]
fn open_project_page() -> Res<()> {
    std::process::Command::new("xdg-open").arg(PROJECT_URL).spawn().map(|_| ()).map_err(|e| format!("can't start xdg-open: {e}"))
}

#[tauri::command]
fn key_action(hub: tauri::State<Arc<Hub>>, action: String) {
    hub.inner().key_action(&action, false);
}

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let hidden = args.iter().any(|a| a == "--hidden");
    if let Some(action) = args.iter().find(|a| keys::ACTIONS.contains(&a.as_str())) {
        // from a volume key: to the running app, else the system volume
        if !keys::send_to_running(action) {
            keys::system_volume(action);
        }
        return;
    }
    if args.iter().any(|a| a == "--help" || a == "-h") {
        println!("usage: open-adi2-control [--hidden] | volume-up | volume-down | mute");
        return;
    }
    if args.iter().any(|a| a == "--version") {
        println!("{}", env!("CARGO_PKG_VERSION"));
        return;
    }
    // already running: show its window
    if keys::send_to_running("show") {
        return;
    }

    let cfg = Config::load();
    tauri::Builder::default()
        .invoke_handler(tauri::generate_handler![
            snapshot,
            schema_json,
            set,
            setup,
            store_preset,
            rename_preset,
            refresh,
            set_config,
            discover,
            keys_status,
            osd_test,
            osd_style,
            open_project_page,
            key_action
        ])
        .setup(move |app| {
            let handle = app.handle().clone();
            let hub = Hub::new(handle.clone(), cfg.clone());
            app.manage(hub.clone());
            main_window(&handle)?;
            osd_window(&handle)?;
            if cfg.tray {
                if let Err(e) = tray::create(&handle) {
                    eprintln!("tray: {e}");
                }
            }
            let h2 = handle.clone();
            if let Err(e) = keys::serve(move |cmd| {
                let h3 = h2.clone();
                match cmd.as_str() {
                    "show" => {
                        let h4 = h3.clone();
                        let _ = h3.run_on_main_thread(move || show_main(&h4));
                    }
                    a if keys::ACTIONS.contains(&a) => h3.state::<Arc<Hub>>().inner().key_action(a, true),
                    _ => {}
                }
            }) {
                eprintln!("command socket: {e}");
            }
            hub.connect();
            if !(hidden || (cfg.start_hidden && cfg.tray)) {
                show_main(&handle);
            }
            Ok(())
        })
        .on_window_event(|window, event| {
            if let WindowEvent::CloseRequested { api, .. } = event {
                if window.label() == "main" {
                    let app = window.app_handle();
                    let cfg = app.state::<Arc<Hub>>().config();
                    if cfg.tray && cfg.close_to_tray && app.tray_by_id("main").is_some() {
                        api.prevent_close();
                        let _ = window.hide();
                    } else {
                        quit(app);
                    }
                } else if window.label() == osd::WINDOW {
                    api.prevent_close();
                    let _ = window.hide();
                }
            }
        })
        .build(tauri::generate_context!())
        .expect("error while building the app")
        .run(|app, event| {
            if let tauri::RunEvent::ExitRequested { api, code, .. } = event {
                // closing the last window with the tray on: keep running
                let cfg = app.state::<Arc<Hub>>().config();
                if code.is_none() && cfg.tray && app.tray_by_id("main").is_some() {
                    api.prevent_exit();
                } else {
                    keys::remove_socket();
                }
            }
        });
}
