//! Tray icon and its menu (volume, mute, dim, EQ, which output the keys control, setups).
//! Linux tray menus can't hold sliders, so the volume is a line of text plus +/− entries.

use crate::hub::{fmt_db, vol_key, Hub};
use crate::model::{key, Command};
use crate::schema::{self, ids};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::Duration;
use tauri::image::Image;
use tauri::menu::{CheckMenuItem, Menu, MenuEvent, MenuItem, PredefinedMenuItem, Submenu};
use tauri::tray::TrayIconBuilder;
use tauri::{AppHandle, Emitter, Manager, Wry};

const TRAY_ID: &str = "main";
static PENDING: AtomicBool = AtomicBool::new(false);

fn icon(muted: bool) -> Image<'static> {
    let bytes: &'static [u8] = if muted { include_bytes!("../icons/tray-muted.png") } else { include_bytes!("../icons/tray.png") };
    Image::from_bytes(bytes).expect("tray icon")
}

pub fn create(app: &AppHandle) -> tauri::Result<()> {
    if app.tray_by_id(TRAY_ID).is_some() {
        return Ok(());
    }
    let menu = build_menu(app)?;
    TrayIconBuilder::with_id(TRAY_ID)
        .icon(icon(false))
        .tooltip("Open ADI-2 Control")
        .menu(&menu)
        .on_menu_event(on_menu)
        .build(app)?;
    Ok(())
}

pub fn remove(app: &AppHandle) {
    let _ = app.remove_tray_by_id(TRAY_ID);
}

/// Rebuild the menu soon (changes come in bursts while a knob turns).
pub fn refresh(app: &AppHandle) {
    if app.tray_by_id(TRAY_ID).is_none() || PENDING.swap(true, Ordering::SeqCst) {
        return;
    }
    let app = app.clone();
    std::thread::spawn(move || {
        std::thread::sleep(Duration::from_millis(150));
        PENDING.store(false, Ordering::SeqCst);
        let a2 = app.clone();
        let _ = app.run_on_main_thread(move || {
            if let Some(t) = a2.tray_by_id(TRAY_ID) {
                if let Ok(menu) = build_menu(&a2) {
                    let _ = t.set_menu(Some(menu));
                }
                let hub = a2.state::<Arc<Hub>>();
                let muted = hub.target_output().and_then(|c| hub.value(&key(c.addr, ids::MUTE))).unwrap_or(0) != 0;
                let _ = t.set_icon(Some(icon(muted)));
            }
        });
    });
}

fn build_menu(app: &AppHandle) -> tauri::Result<Menu<Wry>> {
    let hub = app.state::<Arc<Hub>>();
    let cfg = hub.config();
    let (device, setups) = {
        let m = hub.model.lock().unwrap();
        (m.device.clone(), m.setups.clone())
    };
    let model = device.model.clone();
    let usable = hub.usable();
    let target = hub.target_output();

    let header = if usable {
        match target.and_then(|c| hub.value(&vol_key(c)).map(|v| (c, v))) {
            Some((c, v)) => {
                let muted = hub.value(&key(c.addr, ids::MUTE)).unwrap_or(0) != 0;
                format!("{}: {}{}", c.label(model.as_deref()), fmt_db(v), if muted { " dB (muted)" } else { " dB" })
            }
            None => schema::model_name(model.as_deref().unwrap_or("pro")),
        }
    } else {
        device.problem.as_ref().map(|p| p.text.clone()).unwrap_or_else(|| "Not connected".into())
    };
    let header: String = header.chars().take(70).collect();
    let menu = Menu::new(app)?;
    menu.append(&MenuItem::with_id(app, "hdr", header, false, None::<&str>)?)?;
    let step = cfg.volume.step;
    menu.append(&MenuItem::with_id(app, "vol+", format!("Volume up ({step} dB)"), usable, None::<&str>)?)?;
    menu.append(&MenuItem::with_id(app, "vol-", format!("Volume down ({step} dB)"), usable, None::<&str>)?)?;
    if let Some(c) = target {
        for (id, idx, label) in [("mute", ids::MUTE, "Mute"), ("dim", ids::DIM, "Dim")] {
            if let Some(v) = hub.value(&key(c.addr, idx)) {
                menu.append(&CheckMenuItem::with_id(app, id, label, usable, v != 0, None::<&str>)?)?;
            }
        }
        if let Some(v) = hub.value(&key(c.eq, ids::EQ_ENABLE)) {
            menu.append(&CheckMenuItem::with_id(app, "eq", "EQ", usable, v != 0, None::<&str>)?)?;
        }
    }
    menu.append(&PredefinedMenuItem::separator(app)?)?;

    let tgt = Submenu::with_id(app, "tgt", "Volume keys control", true)?;
    let has = |c: &schema::Channel| hub.value(&vol_key(c)).is_some();
    tgt.append(&CheckMenuItem::with_id(app, "tgt:auto", "The output turned last", true, cfg.keys.target == "auto", None::<&str>)?)?;
    for c in schema::outputs().filter(|c| has(c) || !usable) {
        tgt.append(&CheckMenuItem::with_id(app, format!("tgt:{}", c.key), c.label(model.as_deref()), true, cfg.keys.target == c.key, None::<&str>)?)?;
    }
    menu.append(&tgt)?;

    if cfg.volume.profiles.len() > 1 {
        let pr = Submenu::with_id(app, "profiles", "Volume limits", true)?;
        for (i, p) in cfg.volume.profiles.iter().enumerate() {
            pr.append(&CheckMenuItem::with_id(app, format!("profile:{i}"), &p.name, true, cfg.volume.profile == i, None::<&str>)?)?;
        }
        menu.append(&pr)?;
    }
    if cfg.units.len() > 1 {
        let un = Submenu::with_id(app, "units", "Unit", true)?;
        for (i, u) in cfg.units.iter().enumerate() {
            un.append(&CheckMenuItem::with_id(app, format!("unit:{i}"), u.label(), true, cfg.active == i, None::<&str>)?)?;
        }
        menu.append(&un)?;
    }
    let su = Submenu::with_id(app, "setups", "Load setup", usable)?;
    for (i, name) in setups.iter().enumerate() {
        let label = if name.is_empty() { format!("{}", i + 1) } else { format!("{} · {name}", i + 1) };
        su.append(&MenuItem::with_id(app, format!("setup:{}", i + 1), label, usable, None::<&str>)?)?;
    }
    menu.append(&su)?;
    menu.append(&PredefinedMenuItem::separator(app)?)?;
    if let Some(r) = hub.update_offer() {
        menu.append(&MenuItem::with_id(app, "update", format!("Version {} available…", r.version), true, None::<&str>)?)?;
    }
    menu.append(&MenuItem::with_id(app, "show", "Show window", true, None::<&str>)?)?;
    menu.append(&MenuItem::with_id(app, "quit", "Quit", true, None::<&str>)?)?;
    Ok(menu)
}

fn on_menu(app: &AppHandle, ev: MenuEvent) {
    let hub = app.state::<Arc<Hub>>().inner().clone();
    let id = ev.id().as_ref().to_string();
    let toggle = |idx_of: fn(&schema::Channel) -> String| {
        if let Some(c) = hub.target_output() {
            let k = idx_of(c);
            if let Some(v) = hub.value(&k) {
                let _ = hub.command(Command::Set(vec![(k, if v != 0 { 0 } else { 1 })]));
            }
        }
    };
    match id.as_str() {
        "vol+" => hub.key_action("volume-up", false),
        "vol-" => hub.key_action("volume-down", false),
        "mute" => toggle(|c| key(c.addr, ids::MUTE)),
        "dim" => toggle(|c| key(c.addr, ids::DIM)),
        "eq" => toggle(|c| key(c.eq, ids::EQ_ENABLE)),
        "show" => crate::show_main(app),
        "update" => {
            if let Some(r) = hub.update_offer() {
                let _ = crate::open_link(&r.url);
            }
        }
        "quit" => crate::quit(app),
        _ => {
            if let Some(t) = id.strip_prefix("tgt:") {
                let mut cfg = hub.cfg.lock().unwrap();
                cfg.keys.target = t.to_string();
                cfg.save();
                drop(cfg);
                hub.emit_config();
            } else if let Some(n) = id.strip_prefix("setup:").and_then(|n| n.parse::<u8>().ok()) {
                // the window asks for confirmation
                crate::show_main(app);
                let _ = app.emit("ask", serde_json::json!({ "setup": n }));
            } else if let Some(i) = id.strip_prefix("profile:").and_then(|n| n.parse::<usize>().ok()) {
                hub.set_profile(i);
            } else if let Some(i) = id.strip_prefix("unit:").and_then(|n| n.parse::<usize>().ok()) {
                let changed = {
                    let mut cfg = hub.cfg.lock().unwrap();
                    let ok = i < cfg.units.len() && cfg.active != i;
                    if ok {
                        cfg.active = i;
                        cfg.save();
                    }
                    ok
                };
                if changed {
                    hub.emit_config();
                    hub.connect();
                }
            }
        }
    }
    refresh(app);
}
