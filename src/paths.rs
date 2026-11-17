//! Where TinyHand keeps its files.

use std::path::PathBuf;

/// `%APPDATA%\TinyHand`, or the folder named by `TH_DATA_DIR` (used by tests).
pub fn data_dir() -> PathBuf {
    if let Some(dir) = std::env::var_os("TH_DATA_DIR") {
        return PathBuf::from(dir);
    }
    let base = std::env::var_os("APPDATA")
        .map(PathBuf::from)
        .unwrap_or_else(|| std::env::temp_dir());
    base.join("TinyHand")
}

pub fn settings_file() -> PathBuf {
    data_dir().join("settings.json")
}

pub fn state_file() -> PathBuf {
    data_dir().join("state.json")
}

pub fn log_file() -> PathBuf {
    data_dir().join("tinyhand.log")
}

/// Custom hands live here: one PNG per hand plus `hands.json`.
pub fn hands_dir() -> PathBuf {
    if let Some(dir) = std::env::var_os("TH_HANDS_DIR") {
        return PathBuf::from(dir);
    }
    data_dir().join("Hands")
}
