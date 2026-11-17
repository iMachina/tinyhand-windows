//! `tinyhand://` commands and the `state.json` mirror, so launchers and scripts can drive
//! TinyHand and read what it is doing.

use crate::app::App;
use crate::{paths, styles};
use serde_json::json;

/// Runs a command such as `tinyhand://toggle`, `tinyhand://style/robot` or plain `toggle`.
pub fn handle(app: &mut App, raw: &str) {
    let text = raw.trim().trim_matches('"');
    let lower = text.to_lowercase();
    let rest = lower.strip_prefix("tinyhand://").or_else(|| lower.strip_prefix("tinyhand:")).unwrap_or(&lower);
    let mut parts = rest.split('/').filter(|p| !p.is_empty());
    let command = parts.next().unwrap_or("");
    let argument = parts.next();
    crate::log!("command: {command} {}", argument.unwrap_or(""));

    match command {
        "on" => app.activate(),
        "off" => app.deactivate(),
        "toggle" => app.toggle(),
        "clear" => app.clear_ink(),
        "style" => match argument {
            Some(id) if styles::is_valid(id) => {
                app.pick(id);
                app.activate();
            }
            other => crate::log!("unknown pointer style: {}", other.unwrap_or("none")),
        },
        "sound" => match argument {
            Some("on") => app.set_sound(true),
            Some("off") => app.set_sound(false),
            Some("toggle") | None => {
                let on = app.sound_on();
                app.set_sound(!on);
            }
            Some(other) => crate::log!("unknown sound value: {other}"),
        },
        "settings" => app.want_settings = true,
        "quit" => app.quit(),
        other => crate::log!("unknown command: {other}"),
    }
}

/// Writes the current state to `%APPDATA%\TinyHand\state.json`.
pub fn write_state(active: bool, style: &str, sound_on: bool) {
    let styles: Vec<_> = styles::all_ids()
        .into_iter()
        .map(|id| json!({ "id": id, "name": styles::title(&id), "type": styles::kind(&id) }))
        .collect();
    let state = json!({
        "pid": std::process::id(),
        "active": active,
        "style": style,
        "soundOn": sound_on,
        "styles": styles,
    });
    let path = paths::state_file();
    if let Some(dir) = path.parent() {
        let _ = std::fs::create_dir_all(dir);
    }
    if let Ok(text) = serde_json::to_vec_pretty(&state) {
        let tmp = path.with_extension("json.tmp");
        if std::fs::write(&tmp, text).is_ok() {
            let _ = std::fs::rename(&tmp, &path);
        }
    }
}

pub fn remove_state() {
    let _ = std::fs::remove_file(paths::state_file());
}
