//! Headphone measurements and target curves for the Headphone EQ tab.
//!
//! The data is a snapshot of AutoEq (oratory1990 measurements and target curves, MIT) taken from
//! ADI-2-Pi's `web/eq/data`: `index.json` lists headphones and targets with paths of CSV files
//! (frequency, level) under `files/`. It ships with the app as a resource (`eq-data/`).

use std::path::PathBuf;
use tauri::{AppHandle, Manager};

fn data_dir(app: &AppHandle) -> Option<PathBuf> {
    let mut dirs = Vec::new();
    if let Ok(r) = app.path().resource_dir() {
        dirs.push(r.join("eq-data"));
    }
    // a build run from the source tree
    if cfg!(debug_assertions) {
        dirs.push(PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../eq-data"));
    }
    dirs.into_iter().find(|d| d.join("index.json").is_file())
}

/// The list of headphones and targets (index.json as text).
#[tauri::command]
pub fn eq_index(app: AppHandle) -> Result<String, String> {
    let dir = data_dir(&app).ok_or("the headphone data is missing from this installation")?;
    std::fs::read_to_string(dir.join("index.json")).map_err(|e| e.to_string())
}

/// One measurement or target curve (CSV text). `path` must be one listed in index.json.
#[tauri::command]
pub fn eq_file(app: AppHandle, path: String) -> Result<String, String> {
    if path.contains("..") || path.starts_with('/') || !path.ends_with(".csv") {
        return Err("not a headphone data file".into());
    }
    let dir = data_dir(&app).ok_or("the headphone data is missing from this installation")?;
    let index = std::fs::read_to_string(dir.join("index.json")).map_err(|e| e.to_string())?;
    let listed: serde_json::Value = serde_json::from_str(&index).map_err(|e| e.to_string())?;
    let known = ["headphones", "targets"]
        .iter()
        .filter_map(|k| listed[*k].as_array())
        .flatten()
        .any(|e| e["path"].as_str() == Some(path.as_str()));
    if !known {
        return Err("not a headphone data file".into());
    }
    std::fs::read_to_string(dir.join("files").join(&path)).map_err(|e| format!("can't read {path}: {e}"))
}
