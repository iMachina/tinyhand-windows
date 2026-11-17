//! Hides and shows the real mouse cursor everywhere on screen.
//!
//! Windows has no public "hide the cursor system-wide" call, but the Magnification API's
//! `MagShowSystemCursor` does exactly that at the compositor. The cursor comes back by itself
//! if TinyHand exits or crashes, so it can never be left hidden.

use std::sync::atomic::{AtomicBool, Ordering};
use windows::Win32::UI::Magnification::{MagInitialize, MagShowSystemCursor, MagUninitialize};

static READY: AtomicBool = AtomicBool::new(false);
static HIDDEN: AtomicBool = AtomicBool::new(false);

/// Call once at startup, on the main thread.
pub fn init() -> bool {
    let ok = unsafe { MagInitialize().as_bool() };
    READY.store(ok, Ordering::SeqCst);
    ok
}

pub fn hide() {
    if READY.load(Ordering::SeqCst) {
        HIDDEN.store(true, Ordering::SeqCst);
        unsafe {
            let _ = MagShowSystemCursor(false);
        }
    }
}

pub fn show() {
    if READY.load(Ordering::SeqCst) {
        HIDDEN.store(false, Ordering::SeqCst);
        unsafe {
            let _ = MagShowSystemCursor(true);
        }
    }
}

/// Hides the cursor again if it should be hidden. Cheap; called about once a second in case
/// something else (a secure-desktop switch, another magnifier) brought it back.
pub fn reassert() {
    if READY.load(Ordering::SeqCst) && HIDDEN.load(Ordering::SeqCst) {
        unsafe {
            let _ = MagShowSystemCursor(false);
        }
    }
}

/// Call once before exiting.
pub fn shutdown() {
    if READY.swap(false, Ordering::SeqCst) {
        HIDDEN.store(false, Ordering::SeqCst);
        unsafe {
            let _ = MagShowSystemCursor(true);
            let _ = MagUninitialize();
        }
    }
}
