//! Mouse and keyboard input for the settings window. The mouse is hit-tested against the
//! rectangles the last paint recorded; clicks are carried out by [`actions::perform`] once
//! the window state is released.

use super::view::Action;
use super::{actions, invalidate, invalidate_hwnd, pointers, with_state, Drag};
use crate::prefs;
use windows::Win32::Foundation::{HWND, LPARAM, WPARAM};
use windows::Win32::UI::Input::KeyboardAndMouse::*;
use windows::Win32::UI::WindowsAndMessaging::{PostMessageW, WM_CLOSE};

/// What letting go of the mouse button does.
enum Release {
    Click(Action),
    Reorder(String, usize),
}

pub fn mouse_move(hwnd: HWND, lparam: LPARAM) {
    let start_tracking = with_state(|s| {
        let (x, y) = s.to_dips(lparam);
        s.mouse = Some((x, y));
        let mut dirty = false;
        // A pointer in the toolbar row starts moving once the mouse has gone a few DIPs.
        if s.drag.is_none() {
            if let Some(Action::Tile(id)) = s.pressed.clone() {
                let (px, py) = s.press_at;
                if (x - px).abs() > 4.0 || (y - py).abs() > 4.0 {
                    if let Some(r) = s.frame.rect_of(&Action::Tile(id.clone())) {
                        s.drag = Some(Drag::Tile { id, grab_dx: px - r.x, x: r.x });
                    }
                }
            }
        }
        match s.drag {
            Some(Drag::Tile { grab_dx, x: ref mut tile_x, .. }) => {
                *tile_x = x - grab_dx;
                dirty = true;
            }
            Some(Drag::Slider) => {
                s.set_size_from(x);
                dirty = true;
            }
            Some(Drag::Scroll { grab_dy }) => {
                s.scroll_thumb_to(y - grab_dy);
                dirty = true;
            }
            None => {}
        }
        let hover = if s.drag.is_some() { s.pressed.clone() } else { s.frame.hit_test(x, y) };
        if hover != s.hover {
            s.hover = hover;
            dirty = true;
        }
        if dirty {
            invalidate_hwnd(s.hwnd);
        }
        !std::mem::replace(&mut s.tracking_leave, true)
    });
    if start_tracking == Some(true) {
        let mut track = TRACKMOUSEEVENT {
            cbSize: std::mem::size_of::<TRACKMOUSEEVENT>() as u32,
            dwFlags: TME_LEAVE,
            hwndTrack: hwnd,
            dwHoverTime: 0,
        };
        unsafe {
            let _ = TrackMouseEvent(&mut track);
        }
    }
}

pub fn mouse_down(hwnd: HWND, lparam: LPARAM) {
    unsafe {
        // Clicking the page takes the keyboard away from the text fields. Done first and with
        // nothing borrowed, because the edit control reports losing focus straight away.
        if GetFocus() != hwnd {
            let _ = SetFocus(Some(hwnd));
        }
    }
    let cancel_recording = with_state(|s| {
        let (x, y) = s.to_dips(lparam);
        s.mouse = Some((x, y));
        let hit = s.frame.hit_test(x, y);
        s.hover = hit.clone();
        s.pressed = hit.clone();
        s.press_at = (x, y);
        match hit {
            Some(Action::PointerSize) => {
                s.drag = Some(Drag::Slider);
                s.set_size_from(x);
            }
            Some(Action::ScrollThumb) => {
                let top = s.frame.scroll_thumb.map(|r| r.y).unwrap_or(y);
                s.drag = Some(Drag::Scroll { grab_dy: y - top });
            }
            _ => {}
        }
        invalidate_hwnd(s.hwnd);
        match s.view.recording.as_ref() {
            Some(r) => s.pressed != Some(Action::Record(r.is_toggle)),
            None => false,
        }
    })
    .unwrap_or(false);
    unsafe {
        SetCapture(hwnd);
    }
    if cancel_recording {
        actions::stop_recording();
    }
}

pub fn mouse_up(hwnd: HWND, lparam: LPARAM) {
    let release = with_state(|s| {
        let (x, y) = s.to_dips(lparam);
        let pressed = s.pressed.take();
        let drag = s.drag.take();
        let over = s.frame.hit_test(x, y);
        s.hover = over.clone();
        invalidate_hwnd(s.hwnd);
        match drag {
            Some(Drag::Tile { id, x: tile_x, .. }) => Some(Release::Reorder(id, s.drop_slot(tile_x))),
            Some(_) => None,
            None => match pressed {
                Some(action) if Some(&action) == over.as_ref() => Some(Release::Click(action)),
                _ => None,
            },
        }
    })
    .flatten();
    unsafe {
        let _ = ReleaseCapture();
    }
    match release {
        Some(Release::Click(action)) => actions::perform(hwnd, action),
        Some(Release::Reorder(id, slot)) => {
            let order = pointers::reorder(&prefs::get().pinned(), &id, slot);
            prefs::update(|p| p.pinned_styles = order);
            invalidate();
        }
        None => {}
    }
}

pub fn key_down(hwnd: HWND, key: u32) {
    let ctrl = unsafe { GetKeyState(VK_CONTROL.0 as i32) } < 0;
    let shift = unsafe { GetKeyState(VK_SHIFT.0 as i32) } < 0;
    if key == VK_ESCAPE.0 as u32 {
        // Normally handled in `pre_translate`; this covers other message loops.
        unsafe {
            let _ = PostMessageW(Some(hwnd), WM_CLOSE, WPARAM(0), LPARAM(0));
        }
        return;
    }
    with_state(|s| {
        let page = s.frame.viewport.h * 0.85;
        match VIRTUAL_KEY(key as u16) {
            VK_TAB if ctrl => {
                let next = s.view.section.step(if shift { -1 } else { 1 });
                s.view.section = next;
                s.view.making_hand = false;
                s.view.scroll = 0.0;
                s.view.clear_messages();
                invalidate_hwnd(s.hwnd);
            }
            VK_PRIOR => s.scroll_by(-page),
            VK_NEXT | VK_SPACE => s.scroll_by(page),
            VK_UP => s.scroll_by(-48.0),
            VK_DOWN => s.scroll_by(48.0),
            VK_HOME => s.scroll_by(-f32::MAX / 2.0),
            VK_END => s.scroll_by(f32::MAX / 2.0),
            _ => {}
        }
    });
}
