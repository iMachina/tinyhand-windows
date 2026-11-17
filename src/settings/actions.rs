//! What clicks and keys do. Everything here runs with the window state *not* borrowed:
//! it takes the borrow briefly through [`with_state`] for each change, so it is free to show
//! message boxes and dialogs, open links, and call into the app.

use super::view::{Action, Field, Maker, Recording, Section, Switch};
use super::{edits, invalidate, maker, pointers, with_state, TIMER_COPIED};
use crate::prefs::{self, Shortcut, MOD_ALT, MOD_CONTROL, MOD_SHIFT, MOD_WIN};
use crate::{app, custom, registry, sound, styles};
use std::path::Path;
use windows::core::{w, HSTRING, PCWSTR};
use windows::Win32::Foundation::HWND;
use windows::Win32::UI::Input::KeyboardAndMouse::{GetKeyState, SetFocus};
use windows::Win32::UI::Shell::ShellExecuteW;
use windows::Win32::UI::WindowsAndMessaging::*;

pub const PRESS_A_SHORTCUT: &str = "Press a shortcut…";
const NEED_MODIFIER: &str = "Add Ctrl, Alt, Shift or Win";

/// Carries out a click.
pub fn perform(hwnd: HWND, action: Action) {
    match action {
        Action::Section(section) => {
            with_state(|s| {
                s.view.section = section;
                s.view.making_hand = false;
                s.view.scroll = 0.0;
                s.view.clear_messages();
            });
        }
        Action::Switch(switch) => toggle(switch),
        Action::Record(is_toggle) => start_recording(hwnd, is_toggle),
        Action::Interact(key) => prefs::update(|p| p.interact_key = key),
        Action::KeepHand(keep) => prefs::update(|p| p.interact_keeps_hand = keep),
        Action::Tile(id) => {
            app::with(|a| a.pick(&id));
        }
        Action::Unpin(id) => match pointers::unpin(&prefs::get().pinned(), &id) {
            Ok(list) => {
                prefs::update(|p| p.pinned_styles = list);
                set_pin_message(None);
            }
            Err(_) => set_pin_message(Some("Keep at least one pointer in the toolbar.".into())),
        },
        Action::Pin(id) => match pointers::pin(&prefs::get().pinned(), &id) {
            Ok(list) => {
                prefs::update(|p| p.pinned_styles = list);
                set_pin_message(None);
            }
            Err(_) => set_pin_message(Some(format!("The toolbar is full. Remove one above to add {}.", styles::title(&id)))),
        },
        Action::ResetToolbar => {
            prefs::update(|p| p.pinned_styles = styles::default_pinned());
            set_pin_message(None);
        }
        Action::DeleteHand(id) => delete_hand(hwnd, &id),
        Action::MakeHand => {
            let name = with_state(|s| {
                s.view.making_hand = true;
                s.view.scroll = 0.0;
                s.view.hand_message = None;
                let m = &mut s.view.maker;
                m.draft = None;
                m.error = None;
                m.save_error = None;
                m.copied = false;
                s.edits.name
            });
            if let Some(name) = name {
                edits::set_text(name, "");
            }
        }
        Action::Ink(choice) => prefs::update(|p| p.ink_choice = choice),
        Action::Fade(seconds) => prefs::update(|p| p.fade_seconds = seconds as f64),
        Action::Sound(id) => {
            prefs::update(|p| {
                p.tap_sound = id.to_string();
                p.hand_sound = true;
            });
            sound::play(id);
        }
        Action::Open(url) => open_url(hwnd, url),
        Action::ResetAll => reset_all(hwnd),
        Action::Back => {
            with_state(|s| {
                s.view.making_hand = false;
                s.view.scroll = 0.0;
            });
        }
        Action::OpenChatGpt => {
            let idea = idea_text();
            open_url(hwnd, &maker::chatgpt_url(&idea));
        }
        Action::CopyPrompt => {
            let idea = idea_text();
            if maker::copy_text(hwnd, &crate::handmaker::prompt_for(&idea)) {
                with_state(|s| s.view.maker.copied = true);
                unsafe {
                    SetTimer(Some(hwnd), TIMER_COPIED, 1600, None);
                }
            }
        }
        Action::ChooseFile => {
            if let Some(path) = maker::choose_file(hwnd) {
                load_draft(&path);
            }
        }
        Action::AddHand => add_hand(),
        Action::Focus(field) => {
            let target = with_state(|s| match field {
                Field::Idea => s.edits.idea,
                Field::Name => s.edits.name,
            });
            if let Some(edit) = target {
                if unsafe { IsWindowVisible(edit) }.as_bool() {
                    unsafe {
                        let _ = SetFocus(Some(edit));
                    }
                }
            }
        }
        // These act while the mouse is down (see the window procedure).
        Action::PointerSize | Action::ScrollThumb => {}
    }
    invalidate();
}

fn set_pin_message(message: Option<String>) {
    with_state(|s| s.view.pin_message = message);
}

fn toggle(switch: Switch) {
    match switch {
        Switch::DoubleShift => prefs::update(|p| p.double_shift_click_through = !p.double_shift_click_through),
        Switch::Shadow => prefs::update(|p| p.show_shadow = !p.show_shadow),
        Switch::ShowToolbar => prefs::update(|p| p.show_toolbar = !p.show_toolbar),
        Switch::InkButton => prefs::update(|p| p.show_ink_button = !p.show_ink_button),
        Switch::HandSound => prefs::update(|p| p.hand_sound = !p.hand_sound),
        Switch::LaserSound => prefs::update(|p| p.laser_sound = !p.laser_sound),
        Switch::LaunchAtLogin => {
            let ok = registry::set_launch_at_login(!registry::launch_at_login());
            with_state(|s| s.view.login_failed = !ok);
        }
    }
}

fn idea_text() -> String {
    with_state(|s| s.edits.idea).map(edits::text).unwrap_or_default()
}

fn ask(hwnd: HWND, text: &str) -> bool {
    let answer = unsafe {
        MessageBoxW(Some(hwnd), &HSTRING::from(text), w!("TinyHand"), MB_OKCANCEL | MB_ICONWARNING | MB_DEFBUTTON2)
    };
    answer == IDOK
}

fn open_url(hwnd: HWND, url: &str) {
    unsafe {
        ShellExecuteW(Some(hwnd), w!("open"), &HSTRING::from(url), PCWSTR::null(), PCWSTR::null(), SW_SHOWNORMAL);
    }
}

fn reset_all(hwnd: HWND) {
    let text = "Reset all settings?\n\nShortcuts, toolbar pointers, size, shadow, drawing and sound go back to how they were at first. Open at login stays as it is.";
    if !ask(hwnd, text) {
        return;
    }
    prefs::reset_all();
    with_state(|s| s.view.clear_messages());
}

fn delete_hand(hwnd: HWND, id: &str) {
    let title = styles::title(id);
    if !ask(hwnd, &format!("Delete {title}?\n\nThe hand is removed from TinyHand for good.")) {
        return;
    }
    custom::remove(id);
    styles::forget(id);
    prefs::update(|p| {
        p.pinned_styles = pointers::without(&p.pinned_styles, id);
        if p.pointer_style == id {
            p.pointer_style = "lavender".into();
        }
    });
    app::with(|a| a.refresh());
    with_state(|s| {
        if let Some(c) = s.canvas.as_mut() {
            c.forget_bitmap(&format!("thumb:{id}"));
        }
        s.view.hand_message = None;
    });
}

/// Checks a picture with the hand maker and shows the result on the hand maker page.
pub fn load_draft(path: &Path) {
    let wait = unsafe { LoadCursorW(None, IDC_WAIT).ok() };
    let before = wait.map(|c| unsafe { SetCursor(Some(c)) });
    let (draft, error, name) = maker::draft_from(path);
    if let Some(cursor) = before {
        unsafe { SetCursor(Some(cursor)) };
    }
    let ok = draft.is_some();
    let name_edit = with_state(|s| {
        s.view.section = Section::Appearance;
        s.view.making_hand = true;
        if let Some(c) = s.canvas.as_mut() {
            c.retain_bitmaps(|k| !k.starts_with("draft:"));
        }
        let m = &mut s.view.maker;
        m.draft = draft;
        m.error = error;
        m.save_error = None;
        m.name = name.clone();
        s.edits.name
    });
    if let (true, Some(edit)) = (ok, name_edit) {
        edits::set_text(edit, &name);
    }
    invalidate();
}

/// Saves the draft as a custom hand, puts it in the toolbar if there's room, and uses it.
fn add_hand() {
    let Some((draft, name_edit, idea_edit, typed)) =
        with_state(|s| s.view.maker.draft.take().map(|d| (d, s.edits.name, s.edits.idea, s.view.maker.name.clone()))).flatten()
    else {
        return;
    };
    let name = if name_edit.is_invalid() { typed } else { edits::text(name_edit) };
    match custom::add(&draft.made.image, draft.made.ink, &name) {
        Ok(id) => {
            let title = styles::title(&id);
            let message = match pointers::with_new_hand(&prefs::get().pinned(), &id) {
                Some(list) => {
                    prefs::update(|p| p.pinned_styles = list);
                    format!("{title} is in your toolbar.")
                }
                None => format!("{title} is ready. The toolbar is full, so it waits under Add a pointer."),
            };
            app::with(|a| a.pick(&id));
            app::with(|a| a.refresh());
            with_state(|s| {
                s.view.making_hand = false;
                s.view.maker = Maker::default();
                s.view.hand_message = Some(message);
                // Show the end of the page, where the new hand and the message are.
                s.view.scroll = f32::MAX;
                if let Some(c) = s.canvas.as_mut() {
                    c.retain_bitmaps(|k| !k.starts_with("draft:"));
                }
            });
            edits::set_text(idea_edit, "");
            edits::set_text(name_edit, "");
        }
        Err(message) => {
            with_state(|s| {
                s.view.maker.draft = Some(draft);
                s.view.maker.save_error = Some(message);
            });
        }
    }
    invalidate();
}

// ---- Shortcut recorder -----------------------------------------------------------------

/// What a key press means to the shortcut recorder.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Capture {
    /// Esc on its own: stop recording.
    Cancel,
    /// Only modifiers so far: keep waiting.
    Waiting,
    /// A key without Ctrl, Alt, Shift or Win.
    NeedModifier,
    /// A whole shortcut: virtual key and `MOD_*` flags.
    Got(u32, u32),
}

/// Sorts a key press (virtual key `vk` with the `MOD_*` flags held) for the recorder.
pub fn capture(vk: u32, mods: u32) -> Capture {
    const ESCAPE: u32 = 0x1B;
    match vk {
        // Shift, Ctrl, Alt (either side), the Windows keys, and IME/injected-text keys.
        0x10 | 0x11 | 0x12 | 0x5B | 0x5C | 0xA0..=0xA5 | 0xE5 | 0xE7 | 0xFF | 0 => Capture::Waiting,
        ESCAPE if mods == 0 => Capture::Cancel,
        _ if mods == 0 => Capture::NeedModifier,
        _ => Capture::Got(vk, mods),
    }
}

/// What the recorder shows while modifiers are held: "Ctrl+Alt+…".
pub fn live_text(mods: u32) -> String {
    if mods == 0 {
        return PRESS_A_SHORTCUT.to_string();
    }
    let mut parts = Vec::new();
    for (flag, name) in [(MOD_CONTROL, "Ctrl"), (MOD_ALT, "Alt"), (MOD_SHIFT, "Shift"), (MOD_WIN, "Win")] {
        if mods & flag != 0 {
            parts.push(name);
        }
    }
    format!("{}+…", parts.join("+"))
}

fn held_modifiers() -> u32 {
    let down = |vk: i32| unsafe { GetKeyState(vk) } < 0;
    let mut mods = 0;
    if down(0x11) {
        mods |= MOD_CONTROL;
    }
    if down(0x12) {
        mods |= MOD_ALT;
    }
    if down(0x10) {
        mods |= MOD_SHIFT;
    }
    if down(0x5B) || down(0x5C) {
        mods |= MOD_WIN;
    }
    mods
}

fn start_recording(hwnd: HWND, is_toggle: bool) {
    let already = with_state(|s| {
        let already = s.view.recording.is_some();
        s.view.recording = Some(Recording { is_toggle, live: PRESS_A_SHORTCUT.to_string() });
        s.view.shortcut_error = None;
        already
    });
    if already == Some(false) {
        // Keep the current shortcut from firing while the new one is typed.
        app::with(|a| a.suspend_hotkeys(true));
    }
    unsafe {
        let _ = SetFocus(Some(hwnd));
    }
}

/// Stops recording without changing anything.
pub fn stop_recording() {
    let was = with_state(|s| s.view.recording.take().is_some()).unwrap_or(false);
    if was {
        app::with(|a| a.suspend_hotkeys(false));
    }
    invalidate();
}

/// A key went down while recording.
pub fn recorder_key_down(vk: u32) {
    let mods = held_modifiers();
    match capture(vk, mods) {
        Capture::Cancel => stop_recording(),
        Capture::Waiting => set_live(live_text(mods)),
        Capture::NeedModifier => set_live(NEED_MODIFIER.to_string()),
        Capture::Got(vk, mods) => {
            let Some(is_toggle) = with_state(|s| s.view.recording.as_ref().map(|r| r.is_toggle)).flatten() else { return };
            let shortcut = Shortcut::new(vk, mods);
            let display = shortcut.display.clone();
            let ok = app::with(|a| a.try_set_shortcut(is_toggle, shortcut)).unwrap_or(false);
            with_state(|s| {
                s.view.shortcut_error =
                    (!ok).then(|| format!("That shortcut is taken: {display} is already in use. Pick a different one."));
            });
            stop_recording();
        }
    }
}

/// A key came up while recording: show the modifiers still held.
pub fn recorder_key_up() {
    let mods = held_modifiers();
    with_state(|s| {
        if let Some(r) = s.view.recording.as_mut() {
            if mods != 0 || r.live != NEED_MODIFIER {
                r.live = live_text(mods);
            }
        }
    });
    invalidate();
}

fn set_live(text: String) {
    with_state(|s| {
        if let Some(r) = s.view.recording.as_mut() {
            r.live = text;
        }
    });
    invalidate();
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn recorder_sorts_keys() {
        assert_eq!(capture(0x1B, 0), Capture::Cancel);
        assert_eq!(capture(0x1B, MOD_CONTROL), Capture::Got(0x1B, MOD_CONTROL));
        assert_eq!(capture('H' as u32, 0), Capture::NeedModifier);
        assert_eq!(capture('H' as u32, MOD_CONTROL | MOD_ALT), Capture::Got('H' as u32, MOD_CONTROL | MOD_ALT));
        assert_eq!(capture('A' as u32, MOD_SHIFT), Capture::Got('A' as u32, MOD_SHIFT));
        assert_eq!(capture(0x70, MOD_WIN), Capture::Got(0x70, MOD_WIN));
        for modifier in [0x10, 0x11, 0x12, 0x5B, 0x5C, 0xA0, 0xA1, 0xA2, 0xA3, 0xA4, 0xA5] {
            assert_eq!(capture(modifier, MOD_CONTROL), Capture::Waiting);
            assert_eq!(capture(modifier, 0), Capture::Waiting);
        }
    }

    #[test]
    fn live_text_lists_modifiers_in_order() {
        assert_eq!(live_text(0), PRESS_A_SHORTCUT);
        assert_eq!(live_text(MOD_ALT | MOD_CONTROL), "Ctrl+Alt+…");
        assert_eq!(live_text(MOD_WIN | MOD_SHIFT), "Shift+Win+…");
    }

    #[test]
    fn shortcut_text_matches_prefs() {
        // The recorder hands over MOD_* flags that prefs describes the same way.
        assert_eq!(Shortcut::new('H' as u32, MOD_CONTROL | MOD_ALT).display, "Ctrl+Alt+H");
    }
}
