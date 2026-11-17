//! User settings, stored as JSON in `%APPDATA%\TinyHand\settings.json`.

use crate::{paths, styles};
use serde::{Deserialize, Serialize};
use std::sync::{Mutex, OnceLock};

pub const MOD_ALT: u32 = 0x1;
pub const MOD_CONTROL: u32 = 0x2;
pub const MOD_SHIFT: u32 = 0x4;
pub const MOD_WIN: u32 = 0x8;

/// A global keyboard shortcut: a virtual-key code plus `MOD_*` flags.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Shortcut {
    pub vk: u32,
    pub modifiers: u32,
    pub display: String,
}

impl Shortcut {
    pub fn new(vk: u32, modifiers: u32) -> Self {
        Shortcut { vk, modifiers, display: Self::describe(vk, modifiers) }
    }

    pub fn toggle_default() -> Self {
        Shortcut::new('H' as u32, MOD_CONTROL | MOD_ALT)
    }

    pub fn clear_default() -> Self {
        Shortcut::new('X' as u32, MOD_CONTROL | MOD_ALT)
    }

    /// "Ctrl+Alt+H" style text for a shortcut.
    pub fn describe(vk: u32, modifiers: u32) -> String {
        let mut parts: Vec<String> = Vec::new();
        if modifiers & MOD_CONTROL != 0 {
            parts.push("Ctrl".into());
        }
        if modifiers & MOD_ALT != 0 {
            parts.push("Alt".into());
        }
        if modifiers & MOD_SHIFT != 0 {
            parts.push("Shift".into());
        }
        if modifiers & MOD_WIN != 0 {
            parts.push("Win".into());
        }
        parts.push(key_name(vk));
        parts.join("+")
    }
}

/// A readable name for a virtual-key code.
pub fn key_name(vk: u32) -> String {
    match vk {
        0x30..=0x39 | 0x41..=0x5A => (vk as u8 as char).to_string(),
        0x70..=0x87 => format!("F{}", vk - 0x6F),
        0x60..=0x69 => format!("Num {}", vk - 0x60),
        0x20 => "Space".into(),
        0x0D => "Enter".into(),
        0x09 => "Tab".into(),
        0x08 => "Backspace".into(),
        0x2E => "Delete".into(),
        0x2D => "Insert".into(),
        0x24 => "Home".into(),
        0x23 => "End".into(),
        0x21 => "Page Up".into(),
        0x22 => "Page Down".into(),
        0x25 => "Left".into(),
        0x26 => "Up".into(),
        0x27 => "Right".into(),
        0x28 => "Down".into(),
        0xBA => ";".into(),
        0xBB => "=".into(),
        0xBC => ",".into(),
        0xBD => "-".into(),
        0xBE => ".".into(),
        0xBF => "/".into(),
        0xC0 => "`".into(),
        0xDB => "[".into(),
        0xDC => "\\".into(),
        0xDD => "]".into(),
        0xDE => "'".into(),
        _ => format!("Key {vk:#04X}"),
    }
}

/// The key that, while held, lets clicks through to the app underneath.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum InteractKey {
    Alt,
    Ctrl,
    Win,
    Off,
}

impl InteractKey {
    pub const ALL: [InteractKey; 4] = [InteractKey::Alt, InteractKey::Ctrl, InteractKey::Win, InteractKey::Off];

    pub fn title(self) -> &'static str {
        match self {
            InteractKey::Alt => "Alt",
            InteractKey::Ctrl => "Ctrl",
            InteractKey::Win => "Win",
            InteractKey::Off => "Off",
        }
    }
}

/// The colour lines are drawn in.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum InkChoice {
    Match,
    Red,
    Orange,
    Yellow,
    Green,
    Blue,
    Purple,
    White,
    Black,
}

impl InkChoice {
    pub const ALL: [InkChoice; 9] = [
        InkChoice::Match,
        InkChoice::Red,
        InkChoice::Orange,
        InkChoice::Yellow,
        InkChoice::Green,
        InkChoice::Blue,
        InkChoice::Purple,
        InkChoice::White,
        InkChoice::Black,
    ];

    pub fn title(self) -> &'static str {
        match self {
            InkChoice::Match => "Match the pointer",
            InkChoice::Red => "Red",
            InkChoice::Orange => "Orange",
            InkChoice::Yellow => "Yellow",
            InkChoice::Green => "Green",
            InkChoice::Blue => "Blue",
            InkChoice::Purple => "Purple",
            InkChoice::White => "White",
            InkChoice::Black => "Black",
        }
    }

    /// The fixed colour as 0xRRGGBB, or `None` for "match the pointer".
    pub fn color(self) -> Option<u32> {
        match self {
            InkChoice::Match => None,
            InkChoice::Red => Some(styles::INK_RED),
            InkChoice::Orange => Some(0xFF8A00),
            InkChoice::Yellow => Some(0xFFCC00),
            InkChoice::Green => Some(0x30C25A),
            InkChoice::Blue => Some(0x0A84FF),
            InkChoice::Purple => Some(0x8754F2),
            InkChoice::White => Some(0xFFFFFF),
            InkChoice::Black => Some(0x1D1D21),
        }
    }

    pub fn resolved(self, style: &str) -> u32 {
        self.color().unwrap_or_else(|| styles::ink_color(style))
    }
}

pub const POINTER_SIZE_MIN: f32 = 0.08;
pub const POINTER_SIZE_MAX: f32 = 0.20;
pub const MAX_PINNED: usize = 8;

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct Prefs {
    pub toggle_shortcut: Shortcut,
    pub clear_shortcut: Shortcut,
    /// Height of the hand as a fraction of the screen height.
    pub pointer_size: f32,
    pub show_shadow: bool,
    /// Seconds before a finished line fades. Zero or less keeps lines until cleared.
    pub fade_seconds: f64,
    pub hand_sound: bool,
    pub laser_sound: bool,
    pub tap_sound: String,
    pub interact_key: InteractKey,
    /// Keep showing the hand (not the normal cursor) while the interact key is held.
    pub interact_keeps_hand: bool,
    pub double_shift_click_through: bool,
    pub ink_choice: InkChoice,
    pub show_toolbar: bool,
    pub show_ink_button: bool,
    pub pinned_styles: Vec<String>,
    pub pointer_style: String,
    /// The first-use hint under the toolbar has been shown.
    pub hint_shown: bool,
    /// The "TinyHand lives in the tray" balloon has been shown.
    pub welcome_shown: bool,
}

impl Default for Prefs {
    fn default() -> Self {
        Prefs {
            toggle_shortcut: Shortcut::toggle_default(),
            clear_shortcut: Shortcut::clear_default(),
            pointer_size: 0.13,
            show_shadow: true,
            fade_seconds: 3.0,
            hand_sound: true,
            laser_sound: false,
            tap_sound: "thock".into(),
            interact_key: InteractKey::Alt,
            interact_keeps_hand: false,
            double_shift_click_through: true,
            ink_choice: InkChoice::Match,
            show_toolbar: true,
            show_ink_button: true,
            pinned_styles: styles::default_pinned(),
            pointer_style: "lavender".into(),
            hint_shown: false,
            welcome_shown: false,
        }
    }
}

impl Prefs {
    /// The saved pointer style, falling back to lavender if it no longer exists.
    pub fn style(&self) -> String {
        if styles::is_valid(&self.pointer_style) {
            self.pointer_style.clone()
        } else {
            "lavender".into()
        }
    }

    /// The toolbar's pointers: known styles only, no repeats, never empty.
    pub fn pinned(&self) -> Vec<String> {
        let mut out: Vec<String> = Vec::new();
        for id in &self.pinned_styles {
            if styles::is_valid(id) && !out.contains(id) {
                out.push(id.clone());
            }
        }
        if out.is_empty() {
            styles::default_pinned()
        } else {
            out
        }
    }

    /// Whether taps make a sound for the given style.
    pub fn sound_on(&self, style: &str) -> bool {
        if style == styles::LASER {
            self.laser_sound
        } else {
            self.hand_sound
        }
    }

    fn sanitize(&mut self) {
        if !self.pointer_size.is_finite() {
            self.pointer_size = 0.13;
        }
        self.pointer_size = self.pointer_size.clamp(POINTER_SIZE_MIN, POINTER_SIZE_MAX);
        if !self.fade_seconds.is_finite() {
            self.fade_seconds = 3.0;
        }
    }
}

static PREFS: Mutex<Option<Prefs>> = Mutex::new(None);
static ON_CHANGED: OnceLock<fn()> = OnceLock::new();

fn load() -> Prefs {
    let mut prefs = std::fs::read(paths::settings_file())
        .ok()
        .and_then(|bytes| serde_json::from_slice::<Prefs>(&bytes).ok())
        .unwrap_or_default();
    prefs.sanitize();
    prefs
}

fn save(prefs: &Prefs) {
    let path = paths::settings_file();
    if let Some(dir) = path.parent() {
        let _ = std::fs::create_dir_all(dir);
    }
    if let Ok(json) = serde_json::to_vec_pretty(prefs) {
        let tmp = path.with_extension("json.tmp");
        if std::fs::write(&tmp, json).is_ok() {
            let _ = std::fs::rename(&tmp, &path);
        }
    }
}

/// A copy of the current settings.
pub fn get() -> Prefs {
    let mut guard = PREFS.lock().unwrap();
    guard.get_or_insert_with(load).clone()
}

/// Changes settings, saves them, and tells the app (asynchronously) if anything changed.
pub fn update(change: impl FnOnce(&mut Prefs)) {
    let changed = {
        let mut guard = PREFS.lock().unwrap();
        let prefs = guard.get_or_insert_with(load);
        let before = prefs.clone();
        change(prefs);
        prefs.sanitize();
        if *prefs != before {
            save(prefs);
            true
        } else {
            false
        }
    };
    if changed {
        if let Some(notify) = ON_CHANGED.get() {
            notify();
        }
    }
}

/// Puts everything back to defaults, except the one-time hint flags.
pub fn reset_all() {
    update(|p| {
        let fresh = Prefs { hint_shown: p.hint_shown, welcome_shown: p.welcome_shown, ..Prefs::default() };
        *p = fresh;
    });
}

/// Registers the function called after settings change. It must not block.
pub fn set_on_changed(notify: fn()) {
    let _ = ON_CHANGED.set(notify);
}
