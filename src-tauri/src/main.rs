//! Open ADI-2 Control – a remote for RME ADI-2 series converters on Linux.
//!
//!   open-adi2-control                     start (or show the running app)
//!   open-adi2-control --hidden            start in the tray only
//!   open-adi2-control volume-up | volume-down | mute
//!                                    for the desktop's volume keys (see keys.rs)
//!   open-adi2-control get [ch [id]] | set <ch> <id> <value> [R] | setup <n> | preset <ch> <n>
//!                                    control from scripts; answered by the running app

mod config;
mod dbus;
mod device;
mod eqfile;
mod hpeq;
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
mod update;
mod wake;

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
    if new.unit() != old.unit() {
        hub.inner().connect();
    }
    if new.volume.profile != old.volume.profile || new.volume.profiles != old.volume.profiles {
        hub.inner().apply_limits();
    }
    if new.updates.check && !old.updates.check {
        let h = hub.inner().clone();
        std::thread::spawn(move || h.check_updates(false));
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

pub const PROJECT_URL: &str = update::REPO_URL;

/// Open a link of this project in the default browser: the project page or one of its releases.
pub fn open_link(url: &str) -> Res<()> {
    if url != PROJECT_URL && !update::trusted(url) {
        return Err("not a link of this project".into());
    }
    keys::host_command("xdg-open").arg(url).spawn().map(|_| ()).map_err(|e| format!("can't start xdg-open: {e}"))
}

/// Window messages to stderr (debug builds; the webview's console isn't visible otherwise).
#[tauri::command]
fn ui_log(msg: String) {
    if cfg!(debug_assertions) {
        eprintln!("ui: {msg}");
    }
}

#[tauri::command]
fn open_project_page() -> Res<()> {
    open_link(PROJECT_URL)
}

pub const ADI2_PI_URL: &str = "https://github.com/shells-dw/adi2-pi";

#[tauri::command]
fn open_adi2_pi_page() -> Res<()> {
    keys::host_command("xdg-open").arg(ADI2_PI_URL).spawn().map(|_| ()).map_err(|e| format!("can't start xdg-open: {e}"))
}

/// Let the user pick a text file; Ok(None) when cancelled.
#[tauri::command]
async fn pick_file(app: AppHandle, title: String, ext: Vec<String>, label: String) -> Res<Option<(String, String)>> {
    use tauri_plugin_dialog::DialogExt;
    let path = tauri::async_runtime::spawn_blocking(move || {
        let ext: Vec<&str> = ext.iter().map(String::as_str).collect();
        let mut d = app.dialog().file().set_title(&title).add_filter(&label, &ext);
        if let Some(w) = app.get_webview_window("main") {
            d = d.set_parent(&w);
        }
        d.blocking_pick_file()
    })
    .await
    .map_err(|e| e.to_string())?;
    let Some(path) = path.and_then(|p| p.into_path().ok()) else { return Ok(None) };
    let meta = std::fs::metadata(&path).map_err(|e| e.to_string())?;
    if meta.len() > 8 << 20 {
        return Err("the file is too large".into());
    }
    let text = std::fs::read_to_string(&path).map_err(|e| format!("can't read the file: {e}"))?;
    Ok(Some((path.file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_default(), text)))
}

/// Save text under a name the user picks; Ok(None) when cancelled.
#[tauri::command]
async fn save_file(app: AppHandle, title: String, name: String, ext: Vec<String>, label: String, text: String) -> Res<Option<String>> {
    use tauri_plugin_dialog::DialogExt;
    let path = tauri::async_runtime::spawn_blocking(move || {
        let ext: Vec<&str> = ext.iter().map(String::as_str).collect();
        let mut d = app.dialog().file().set_title(&title).set_file_name(&name).add_filter(&label, &ext);
        if let Some(w) = app.get_webview_window("main") {
            d = d.set_parent(&w);
        }
        d.blocking_save_file()
    })
    .await
    .map_err(|e| e.to_string())?;
    let Some(path) = path.and_then(|p| p.into_path().ok()) else { return Ok(None) };
    std::fs::write(&path, text).map_err(|e| format!("can't write the file: {e}"))?;
    Ok(Some(path.to_string_lossy().to_string()))
}

/// The unit's EQ presets as a .adieqpr file (ADI-2 Remote's format). Empty slots are skipped
/// unless `slots` names them.
#[tauri::command]
fn export_presets(hub: tauri::State<Arc<Hub>>, slots: Option<Vec<u8>>) -> Res<String> {
    let m = hub.model.lock().unwrap();
    let list: Vec<_> = m
        .presets
        .iter()
        .filter(|p| match &slots {
            Some(s) => s.contains(&p.n),
            None => p.empty == Some(false),
        })
        .cloned()
        .collect();
    if list.is_empty() {
        return Err("no presets to export".into());
    }
    Ok(eqfile::write(&list))
}

/// Presets given by the window (headphone EQ result) as a .adieqpr file.
#[tauri::command]
fn write_presets(presets: Vec<model::Preset>) -> Res<String> {
    if presets.is_empty() || presets.iter().any(|p| !(1..=20).contains(&p.n)) {
        return Err("no presets to write".into());
    }
    Ok(eqfile::write(&presets))
}

/// The presets in a .adieqpr file, for the import dialog.
#[tauri::command]
fn parse_presets(text: String) -> Res<Vec<model::Preset>> {
    eqfile::read(&text)
}

#[tauri::command]
fn open_release_link(url: String) -> Res<()> {
    open_link(&url)
}

#[tauri::command]
async fn update_check(hub: tauri::State<'_, Arc<Hub>>) -> Res<Value> {
    let h = hub.inner().clone();
    tauri::async_runtime::spawn_blocking(move || {
        h.check_updates(false);
        h.update_view()
    })
    .await
    .map_err(|e| e.to_string())
}

#[tauri::command]
fn update_skip(hub: tauri::State<Arc<Hub>>, version: String) -> Value {
    {
        let mut cfg = hub.cfg.lock().unwrap();
        cfg.updates.skip = version;
        cfg.save();
    }
    hub.emit_config();
    tray::refresh(&hub.app_handle());
    hub.update_view()
}

#[tauri::command]
fn key_action(hub: tauri::State<Arc<Hub>>, action: String) {
    hub.inner().key_action(&action, false);
}

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let hidden = args.iter().any(|a| a == "--hidden");
    if let Some(action) = args.first().filter(|a| keys::ACTIONS.contains(&a.as_str())) {
        // from a volume key: to the running app, else the system volume
        if !keys::send_to_running(action) {
            keys::system_volume(action);
        }
        return;
    }
    if matches!(args.first().map(String::as_str), Some("get" | "set" | "setup" | "preset" | "refresh" | "ping")) {
        match keys::ask_running(&args) {
            Some(Ok(t)) => println!("{t}"),
            Some(Err(t)) => {
                eprintln!("{t}");
                std::process::exit(1);
            }
            None => {
                eprintln!("Open ADI-2 Control is not running");
                std::process::exit(2);
            }
        }
        return;
    }
    if args.iter().any(|a| a == "--help" || a == "-h") {
        println!(concat!(
            "usage: open-adi2-control [--hidden]\n",
            "       open-adi2-control volume-up | volume-down | mute\n",
            "       open-adi2-control get [line|ph12|ph34|in|device [id [R]]]\n",
            "       open-adi2-control set <line|ph12|ph34|in|device> <id> <value> [R]\n",
            "           value: a number (+1 / -=1 relative), on/off/toggle, a choice by name or index, next/prev\n",
            "       open-adi2-control setup <1-9> | preset <output> <1-20|manual|temp> | refresh\n",
            "       ids: see src-tauri/schema.json (vol, mute, dim, src, eqen, eqpreset, b1g …)"
        ));
        return;
    }
    if args.iter().any(|a| a == "--version") {
        println!("{}", env!("CARGO_PKG_VERSION"));
        return;
    }
    // already running: show its window (a second autostart just ends)
    if keys::send_to_running(if hidden { "ping" } else { "show" }) {
        return;
    }

    let cfg = Config::load();
    tauri::Builder::default()
        .plugin(tauri_plugin_dialog::init())
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
            ui_log,
            hpeq::eq_index,
            hpeq::eq_file,
            open_adi2_pi_page,
            pick_file,
            save_file,
            export_presets,
            parse_presets,
            write_presets,
            open_release_link,
            update_check,
            update_skip,
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
            if let Err(e) = keys::serve(move |args| h2.state::<Arc<Hub>>().inner().cli(&args)) {
                eprintln!("command socket: {e}");
            }
            hub.connect();
            hub.start_update_timer();
            wake::start(hub.clone());
            dbus::start(hub.clone());
            // the autostart entry and the GNOME key bindings name the executable: keep them
            // pointing at this one (an AppImage may have been renamed or replaced)
            if cfg.autostart {
                let _ = set_autostart(true);
            }
            if cfg.keys.enabled {
                std::thread::spawn(keys::gnome_refresh_commands);
            }
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
