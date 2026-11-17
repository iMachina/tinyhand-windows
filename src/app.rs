//! The heart of TinyHand: the tray icon, the shortcuts, and the overlay that shows the hand.
//!
//! Everything here runs on the main thread. The input hooks (see `hook`) run on their own
//! thread and report what happened by posting `EV_*` messages to the main window.

use crate::arm::Arm;
use crate::ink::Ink;
use crate::prefs::{self, Prefs, Shortcut};
use crate::render::Renderer;
use crate::styles::{self, HandAsset};
use crate::toolbar::{Action, Toolbar};
use crate::{automation, cursor, hook, registry, settings, sound};
use std::cell::RefCell;
use std::sync::atomic::{AtomicIsize, Ordering::SeqCst};
use std::sync::Arc;
use std::time::{Duration, Instant};
use windows::core::{w, PCWSTR};
use windows::Win32::Foundation::*;
use windows::Win32::Graphics::Gdi::*;
use windows::Win32::Media::{timeBeginPeriod, timeEndPeriod};
use windows::Win32::System::DataExchange::COPYDATASTRUCT;
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::System::RemoteDesktop::{WTSRegisterSessionNotification, NOTIFY_FOR_THIS_SESSION};
use windows::Win32::UI::HiDpi::{GetDpiForMonitor, MDT_EFFECTIVE_DPI};
use windows::Win32::UI::Input::KeyboardAndMouse::*;
use windows::Win32::UI::Shell::*;
use windows::Win32::UI::WindowsAndMessaging::*;

pub const MAIN_CLASS: PCWSTR = w!("TinyHandMain");
const OVERLAY_CLASS: PCWSTR = w!("TinyHandOverlay");

const WM_TRAY: u32 = WM_APP + 1;
const WM_PREFS_CHANGED: u32 = WM_APP + 2;
/// `WM_COPYDATA` marker for a command sent by a second copy of TinyHand.
pub const COPYDATA_COMMAND: usize = 0x5448_434D;

const HOTKEY_TOGGLE: i32 = 1;
const HOTKEY_CLEAR: i32 = 2;
const TIMER_TICK: usize = 1;

const MENU_TOGGLE: usize = 1;
const MENU_CLEAR: usize = 2;
const MENU_SOUND: usize = 3;
const MENU_SETTINGS: usize = 4;
const MENU_QUIT: usize = 5;

const IDLE_AFTER: f32 = 0.5;
const NIN_SELECT: u32 = WM_USER;
const NIN_KEYSELECT: u32 = WM_USER + 1;

static MAIN_HWND: AtomicIsize = AtomicIsize::new(0);

/// The hidden main window, which owns the tray icon and receives every message.
pub fn main_hwnd() -> HWND {
    HWND(MAIN_HWND.load(SeqCst) as *mut _)
}

thread_local! {
    static APP: RefCell<Option<App>> = const { RefCell::new(None) };
}

/// Runs `f` with the app, then carries out anything it asked to have done afterwards.
/// Returns `None` if the app doesn't exist or is already in use further up the call stack.
pub fn with<R>(f: impl FnOnce(&mut App) -> R) -> Option<R> {
    let result = APP.with(|cell| {
        let mut guard = cell.try_borrow_mut().ok()?;
        guard.as_mut().map(f)
    });
    if result.is_some() {
        run_deferred();
    }
    result
}

/// Things that can't be done while the app is borrowed: they open windows or end the program.
fn run_deferred() {
    let (open_settings, quit) = APP.with(|cell| match cell.try_borrow_mut() {
        Ok(mut guard) => match guard.as_mut() {
            Some(app) => (std::mem::take(&mut app.want_settings), std::mem::take(&mut app.want_quit)),
            None => (false, false),
        },
        Err(_) => (false, false),
    });
    if open_settings {
        settings::show();
    }
    if quit {
        unsafe { PostQuitMessage(0) };
    }
}

pub struct TickResult {
    pub rendered: bool,
    pub busy: bool,
}

pub struct App {
    main: HWND,
    overlay: HWND,
    renderer: Option<Renderer>,
    prefs: Prefs,

    active: bool,
    style: String,
    asset: Option<Arc<HandAsset>>,
    arm: Arm,
    ink: Ink,
    toolbar: Toolbar,

    /// The monitor the overlay currently covers, in screen coordinates.
    frame: Option<RECT>,
    down_point: Option<(f32, f32)>,
    buttons_held: u32,
    normal_cursor: bool,
    own_menu_open: bool,
    reveal_until: Instant,
    hint_until: Option<Instant>,
    still_since: Instant,
    last_mouse: POINT,
    last_tick: Instant,
    last_upkeep: Instant,
    last_key: [i32; 12],
    dirty: bool,
    drag_points: Vec<POINT>,

    hotkeys_on: bool,
    hotkeys_suspended: bool,
    tray_added: bool,
    tray_icon: HICON,
    timer_period: bool,

    /// Frames drawn since start, for measuring.
    pub frames: u64,
    pub want_settings: bool,
    want_quit: bool,
}

fn wide(text: &str) -> Vec<u16> {
    text.encode_utf16().chain(std::iter::once(0)).collect()
}

fn copy_wide(into: &mut [u16], text: &str) {
    let w: Vec<u16> = text.encode_utf16().collect();
    let n = w.len().min(into.len() - 1);
    into[..n].copy_from_slice(&w[..n]);
    into[n] = 0;
}

fn ctrl_down() -> bool {
    unsafe { (GetAsyncKeyState(VK_CONTROL.0 as i32) as u16 & 0x8000) != 0 }
}

impl App {
    // ---- State other modules ask about -------------------------------------------------

    pub fn is_active(&self) -> bool {
        self.active
    }

    pub fn sound_on(&self) -> bool {
        self.prefs.sound_on(&self.style)
    }

    pub fn is_click_through(&self) -> bool {
        self.active && hook::click_through()
    }

    pub fn ink_strokes(&self) -> usize {
        self.ink.strokes.len()
    }

    pub fn is_drawing(&self) -> bool {
        self.ink.is_drawing()
    }

    /// Where a toolbar button for `style` is on screen, for tests.
    pub fn toolbar_frame(&self) -> Option<RECT> {
        let frame = self.frame?;
        let f = self.toolbar.frame();
        Some(RECT {
            left: frame.left + f.x as i32,
            top: frame.top + f.y as i32,
            right: frame.left + (f.x + f.w) as i32,
            bottom: frame.top + (f.y + f.h) as i32,
        })
    }

    // ---- Commands ----------------------------------------------------------------------

    pub fn toggle(&mut self) {
        if self.active {
            self.deactivate();
        } else {
            self.activate();
        }
    }

    pub fn activate(&mut self) {
        if self.active {
            return;
        }
        crate::log!("activate");
        self.active = true;
        self.frame = None;
        self.prefs = prefs::get();
        sound::warm();

        let now = Instant::now();
        self.arm.reset();
        self.buttons_held = 0;
        self.down_point = None;
        self.normal_cursor = false;
        self.toolbar.reset();
        self.toolbar.sync(&self.prefs, &self.style);
        self.reveal_until = now + Duration::from_secs(3);
        self.hint_until = None;
        self.still_since = now;
        self.last_tick = now;
        self.dirty = true;

        self.push_hook_settings();
        hook::set_click_through(false);
        hook::set_active(true);
        cursor::hide();

        if !self.prefs.hint_shown {
            prefs::update(|p| p.hint_shown = true);
            self.prefs.hint_shown = true;
            let hold = format!("Hold {}", self.prefs.interact_key.title());
            let mut parts: Vec<(&str, &str)> = vec![("Click", "to tap"), ("Drag", "to draw")];
            if self.prefs.interact_key != prefs::InteractKey::Off {
                parts.push((&hold, "to click for real"));
            }
            if self.prefs.double_shift_click_through {
                parts.push(("Double-tap Shift", "to click through"));
            }
            parts.push(("Esc", "to exit"));
            self.toolbar.show_hint(&parts);
            self.reveal_until = now + Duration::from_secs(10);
            self.hint_until = Some(now + Duration::from_secs(10));
        }

        unsafe {
            if !self.timer_period {
                timeBeginPeriod(1);
                self.timer_period = true;
            }
            SetTimer(Some(self.main), TIMER_TICK, 15, None);
        }
        // Draw the first frame before the window appears, so it never shows a stale one.
        self.tick();
        unsafe {
            let _ = ShowWindow(self.overlay, SW_SHOWNOACTIVATE);
        }
        self.state_changed();
    }

    pub fn deactivate(&mut self) {
        if !self.active {
            return;
        }
        crate::log!("deactivate");
        self.active = false;
        hook::set_active(false);
        hook::set_click_through(false);
        hook::set_toolbar_rect(None);
        self.arm.held = false;
        self.buttons_held = 0;
        self.down_point = None;
        self.ink.clear();
        self.toolbar.reset();
        self.normal_cursor = false;
        self.own_menu_open = false;
        self.hint_until = None;
        unsafe {
            let _ = KillTimer(Some(self.main), TIMER_TICK);
            let _ = ShowWindow(self.overlay, SW_HIDE);
            if self.timer_period {
                timeEndPeriod(1);
                self.timer_period = false;
            }
        }
        // Leave an empty frame behind for the next time the window is shown.
        if let Some(r) = self.renderer.as_mut() {
            r.begin();
            if r.end().is_err() {
                self.renderer = None;
            }
        }
        cursor::show();
        self.state_changed();
    }

    /// Clears the drawing and turns the pointer off.
    pub fn exit(&mut self) {
        self.clear_ink();
        self.deactivate();
    }

    pub fn quit(&mut self) {
        self.deactivate();
        self.want_quit = true;
    }

    pub fn clear_ink(&mut self) {
        self.ink.clear();
        self.dirty = true;
    }

    /// Switches to a pointer style and remembers it.
    pub fn pick(&mut self, style: &str) {
        self.apply_style(style);
        let chosen = self.style.clone();
        if self.prefs.pointer_style != chosen {
            self.prefs.pointer_style = chosen.clone();
            prefs::update(|p| p.pointer_style = chosen);
        }
        self.toolbar.sync(&self.prefs, &self.style);
        self.state_changed();
    }

    fn apply_style(&mut self, style: &str) {
        self.asset = if style == styles::LASER { None } else { styles::load_hand(style) };
        // A hand whose picture can't be loaded falls back to the laser dot.
        self.style = if self.asset.is_some() { style.to_string() } else { styles::LASER.to_string() };
        self.dirty = true;
    }

    pub fn set_sound(&mut self, on: bool) {
        let laser = self.style == styles::LASER;
        if laser {
            self.prefs.laser_sound = on;
        } else {
            self.prefs.hand_sound = on;
        }
        prefs::update(|p| {
            if laser {
                p.laser_sound = on;
            } else {
                p.hand_sound = on;
            }
        });
        self.toolbar.sync(&self.prefs, &self.style);
        self.state_changed();
    }

    fn set_click_through(&mut self, on: bool) {
        if !self.active {
            return;
        }
        hook::set_click_through(on);
        crate::log!("click-through {}", if on { "on" } else { "off" });
        if on {
            self.release();
        }
        if on {
            self.toolbar.show_hint(&[("Clicks go through", "the hand stays"), ("Double-tap Shift", "to tap again")]);
        } else {
            self.toolbar.show_hint(&[("Taps", "clicks don't reach apps"), ("Double-tap Shift", "to click through")]);
        }
        self.hint_until = Some(Instant::now() + Duration::from_millis(1500));
        self.wake();
        self.state_changed();
    }

    fn push_hook_settings(&self) {
        hook::set_interact_key(self.prefs.interact_key);
        hook::set_double_shift(self.prefs.double_shift_click_through);
        hook::set_esc_enabled(true);
    }

    /// Re-reads the settings and the list of hands and brings everything in line with them.
    /// Called after the settings file changed, and by the settings window after it adds or
    /// deletes a custom hand.
    pub fn refresh(&mut self) {
        let old = std::mem::replace(&mut self.prefs, prefs::get());
        self.push_hook_settings();
        if !self.prefs.double_shift_click_through && self.is_click_through() {
            self.set_click_through(false);
        }
        let wanted = self.prefs.style();
        if wanted != self.style || !styles::is_valid(&self.style) {
            self.apply_style(&wanted);
        }
        if let Some(r) = self.renderer.as_mut() {
            // Custom hands can be deleted or replaced under the same id.
            for id in old.pinned_styles.iter().chain(std::iter::once(&old.pointer_style)) {
                if id.starts_with("custom-") && !styles::is_valid(id) {
                    r.forget_thumb(id);
                }
            }
        }
        if old.toggle_shortcut != self.prefs.toggle_shortcut || old.clear_shortcut != self.prefs.clear_shortcut {
            self.unregister_hotkeys();
            self.register_hotkeys();
        }
        self.toolbar.sync(&self.prefs, &self.style);
        self.dirty = true;
        self.state_changed();
    }

    /// Updates the tray icon and `state.json` after something visible from outside changed.
    fn state_changed(&mut self) {
        self.update_tray();
        automation::write_state(self.active, &self.style, self.sound_on());
    }

    // ---- Shortcuts ---------------------------------------------------------------------

    fn register_hotkeys(&mut self) -> bool {
        if self.hotkeys_on || self.hotkeys_suspended {
            return true;
        }
        let reg = |id: i32, s: &Shortcut| unsafe {
            RegisterHotKey(Some(self.main), id, HOT_KEY_MODIFIERS(s.modifiers) | MOD_NOREPEAT, s.vk).is_ok()
        };
        let a = reg(HOTKEY_TOGGLE, &self.prefs.toggle_shortcut);
        let b = reg(HOTKEY_CLEAR, &self.prefs.clear_shortcut);
        self.hotkeys_on = true;
        if !(a && b) {
            crate::log!("shortcut registration failed: toggle={a} clear={b}");
        }
        a && b
    }

    fn unregister_hotkeys(&mut self) {
        unsafe {
            let _ = UnregisterHotKey(Some(self.main), HOTKEY_TOGGLE);
            let _ = UnregisterHotKey(Some(self.main), HOTKEY_CLEAR);
        }
        self.hotkeys_on = false;
    }

    /// Turns the global shortcuts off while the settings window records a new one.
    pub fn suspend_hotkeys(&mut self, suspended: bool) {
        if suspended {
            self.unregister_hotkeys();
            self.hotkeys_suspended = true;
        } else {
            self.hotkeys_suspended = false;
            self.register_hotkeys();
        }
    }

    /// Tries a new shortcut. If Windows refuses it (another app owns it) the old one is
    /// restored and this returns false.
    pub fn try_set_shortcut(&mut self, is_toggle: bool, shortcut: Shortcut) -> bool {
        let suspended = std::mem::replace(&mut self.hotkeys_suspended, false);
        self.unregister_hotkeys();
        let old = if is_toggle { self.prefs.toggle_shortcut.clone() } else { self.prefs.clear_shortcut.clone() };
        let other = if is_toggle { &self.prefs.clear_shortcut } else { &self.prefs.toggle_shortcut };
        let clash = other.vk == shortcut.vk && other.modifiers == shortcut.modifiers;
        if is_toggle {
            self.prefs.toggle_shortcut = shortcut.clone();
        } else {
            self.prefs.clear_shortcut = shortcut.clone();
        }
        let ok = !clash && self.register_hotkeys();
        if ok {
            prefs::update(|p| {
                if is_toggle {
                    p.toggle_shortcut = shortcut;
                } else {
                    p.clear_shortcut = shortcut;
                }
            });
        } else {
            self.unregister_hotkeys();
            if is_toggle {
                self.prefs.toggle_shortcut = old;
            } else {
                self.prefs.clear_shortcut = old;
            }
            self.register_hotkeys();
        }
        if suspended {
            self.unregister_hotkeys();
            self.hotkeys_suspended = true;
        }
        self.update_tray();
        ok
    }

    // ---- Tray icon ---------------------------------------------------------------------

    fn tray_data(&self) -> NOTIFYICONDATAW {
        let mut nid = NOTIFYICONDATAW {
            cbSize: std::mem::size_of::<NOTIFYICONDATAW>() as u32,
            hWnd: self.main,
            uID: 1,
            uFlags: NIF_ICON | NIF_MESSAGE | NIF_TIP | NIF_SHOWTIP,
            uCallbackMessage: WM_TRAY,
            hIcon: self.tray_icon,
            ..Default::default()
        };
        nid.Anonymous.uVersion = NOTIFYICON_VERSION_4;
        let tip = if self.is_click_through() {
            "TinyHand clicks go through \u{2014} double-tap Shift to tap again".to_string()
        } else if self.active {
            "TinyHand is on \u{2014} click to turn off, right-click for the menu".to_string()
        } else {
            format!("TinyHand \u{2014} click to turn on ({}), right-click for the menu", self.prefs.toggle_shortcut.display)
        };
        copy_wide(&mut nid.szTip, &tip);
        nid
    }

    fn update_tray(&mut self) {
        let tint = if self.is_click_through() {
            0xFF9500
        } else if registry::taskbar_is_light() {
            0x1D1D21
        } else {
            0xFFFFFF
        };
        let icon = make_tray_icon(self.active, tint);
        let old = std::mem::replace(&mut self.tray_icon, icon);
        let nid = self.tray_data();
        unsafe {
            if self.tray_added {
                let _ = Shell_NotifyIconW(NIM_MODIFY, &nid);
            } else if Shell_NotifyIconW(NIM_ADD, &nid).as_bool() {
                let _ = Shell_NotifyIconW(NIM_SETVERSION, &nid);
                self.tray_added = true;
            }
            if !old.is_invalid() {
                let _ = DestroyIcon(old);
            }
        }
    }

    fn remove_tray(&mut self) {
        if self.tray_added {
            let nid = self.tray_data();
            unsafe {
                let _ = Shell_NotifyIconW(NIM_DELETE, &nid);
            }
            self.tray_added = false;
        }
    }

    /// The one-time "TinyHand lives in the tray" balloon.
    fn show_welcome(&mut self) {
        if self.prefs.welcome_shown || !self.tray_added {
            return;
        }
        self.prefs.welcome_shown = true;
        prefs::update(|p| p.welcome_shown = true);
        let mut nid = self.tray_data();
        nid.uFlags |= NIF_INFO;
        nid.dwInfoFlags = NIIF_USER | NIIF_LARGE_ICON;
        copy_wide(&mut nid.szInfoTitle, "TinyHand is ready");
        let text = format!(
            "Click the tray icon or press {} to turn the pointer on. Right-click the icon for settings.",
            self.prefs.toggle_shortcut.display
        );
        copy_wide(&mut nid.szInfo, &text);
        unsafe {
            let _ = Shell_NotifyIconW(NIM_MODIFY, &nid);
        }
    }

    // ---- Mouse events from the hook thread ---------------------------------------------

    fn local(&self, pt: POINT) -> Option<(f32, f32)> {
        let f = self.frame?;
        Some(((pt.x - f.left) as f32, (pt.y - f.top) as f32))
    }

    fn ink_width(&self) -> f32 {
        let h = self.frame.map(|f| (f.bottom - f.top) as f32).unwrap_or(1080.0);
        (h * 0.0055).max(3.0)
    }

    fn on_mouse_down(&mut self, left: bool, pt: POINT) {
        self.arm.press();
        self.arm.held = true;
        self.buttons_held += 1;
        if self.sound_on() {
            sound::play(&self.prefs.tap_sound);
        }
        self.toolbar.hide_hint();
        self.toolbar.close_ink_tray();
        if left {
            self.down_point = self.local(pt);
        }
        self.wake();
    }

    fn on_mouse_up(&mut self, left: bool) {
        self.buttons_held = self.buttons_held.saturating_sub(1);
        if self.buttons_held == 0 {
            self.arm.held = false;
        }
        if left {
            // Catch up on any movement that arrived just before the release.
            self.follow_drag(None);
            self.release();
        }
        self.wake();
    }

    fn release(&mut self) {
        self.arm.held = false;
        self.buttons_held = 0;
        self.down_point = None;
        if self.ink.is_drawing() {
            self.ink.end(self.prefs.fade_seconds);
            self.dirty = true;
        }
    }

    fn drag_to(&mut self, p: (f32, f32)) {
        let Some(start) = self.down_point else { return };
        if !self.ink.is_drawing() {
            if (p.0 - start.0).hypot(p.1 - start.1) < 4.0 {
                return;
            }
            let color = self.prefs.ink_choice.resolved(&self.style);
            self.ink.begin(start, self.ink_width(), color);
        }
        self.ink.extend(p);
        self.dirty = true;
    }

    /// Feeds the mouse positions recorded by the hook (and optionally the current one)
    /// into the line being drawn.
    fn follow_drag(&mut self, current: Option<(f32, f32)>) {
        if self.down_point.is_none() {
            return;
        }
        let mut points = std::mem::take(&mut self.drag_points);
        points.clear();
        hook::take_drag_points(&mut points);
        for pt in &points {
            if let Some(p) = self.local(*pt) {
                self.drag_to(p);
            }
        }
        self.drag_points = points;
        if let Some(p) = current {
            self.drag_to(p);
        }
        if self.ink.is_drawing() {
            self.ink.set_arrow(ctrl_down());
        }
    }

    fn on_toolbar_click(&mut self, pt: POINT) {
        let Some(p) = self.local(pt) else { return };
        let Some(action) = self.toolbar.hit(p) else { return };
        self.toolbar.hide_hint();
        match action {
            Action::Pick(style) => self.pick(&style),
            Action::ToggleInkTray => self.toolbar.toggle_ink_tray(),
            Action::Ink(choice) => {
                self.prefs.ink_choice = choice;
                prefs::update(|p| p.ink_choice = choice);
                self.toolbar.close_ink_tray();
                self.toolbar.sync(&self.prefs, &self.style);
            }
            Action::ToggleSound => {
                let on = self.sound_on();
                self.set_sound(!on);
            }
            Action::Exit => self.exit(),
        }
        self.dirty = true;
    }

    fn wake(&mut self) {
        self.still_since = Instant::now();
    }

    // ---- The frame loop ----------------------------------------------------------------

    /// Moves the overlay onto the monitor under the mouse. Returns false if there is no monitor.
    fn follow_monitor(&mut self, pt: POINT, now: Instant) -> bool {
        unsafe {
            let monitor = MonitorFromPoint(pt, MONITOR_DEFAULTTONEAREST);
            let mut info = MONITORINFO { cbSize: std::mem::size_of::<MONITORINFO>() as u32, ..Default::default() };
            if !GetMonitorInfoW(monitor, &mut info).as_bool() {
                return false;
            }
            let rc = info.rcMonitor;
            if self.frame != Some(rc) || self.renderer.is_none() {
                let moving = self.frame.is_some() && self.frame != Some(rc);
                if moving {
                    self.release();
                    self.reveal_until = now + Duration::from_millis(1500);
                }
                self.frame = Some(rc);
                self.ink.clear();
                self.arm.reset();
                let (w, h) = ((rc.right - rc.left) as u32, (rc.bottom - rc.top) as u32);
                let _ = SetWindowPos(
                    self.overlay,
                    Some(HWND_TOPMOST),
                    rc.left,
                    rc.top,
                    w as i32,
                    h as i32,
                    SWP_NOACTIVATE | SWP_NOREDRAW,
                );
                let ok = match self.renderer.as_mut() {
                    Some(r) => r.resize(w, h).is_ok(),
                    None => false,
                };
                if !ok {
                    self.renderer = Renderer::for_window(self.overlay, w, h)
                        .map_err(|e| crate::log!("renderer failed: {e}"))
                        .ok();
                }
                let (mut dx, mut dy) = (96u32, 96u32);
                let _ = GetDpiForMonitor(monitor, MDT_EFFECTIVE_DPI, &mut dx, &mut dy);
                let scale = dx as f32 / 96.0;
                self.toolbar.place(w as f32 / 2.0, (info.rcWork.bottom - rc.top) as f32, scale);
                self.dirty = true;
            }
        }
        true
    }

    /// One step of the pointer: follow the mouse, update what is shown, draw if needed.
    pub fn tick(&mut self) -> TickResult {
        let idle = TickResult { rendered: false, busy: false };
        if !self.active {
            return idle;
        }
        let now = Instant::now();
        let dt = now.duration_since(self.last_tick).as_secs_f32().min(0.1);
        self.last_tick = now;

        let mut pt = POINT::default();
        unsafe {
            let _ = GetCursorPos(&mut pt);
        }
        if !self.follow_monitor(pt, now) {
            return idle;
        }
        let frame = self.frame.unwrap();
        let (w, h) = ((frame.right - frame.left) as f32, (frame.bottom - frame.top) as f32);
        let mouse = ((pt.x - frame.left) as f32, (pt.y - frame.top) as f32);
        if pt != self.last_mouse {
            self.last_mouse = pt;
            self.still_since = now;
        }

        // The line being drawn.
        self.follow_drag(Some(mouse));
        let drawing = self.ink.is_drawing();
        let fading = self.ink.update(now);

        // The toolbar shows itself near the bottom of the screen and right after turning on.
        if matches!(self.hint_until, Some(t) if now >= t) {
            self.hint_until = None;
            self.toolbar.hide_hint();
        }
        let reveal = self.prefs.show_toolbar && !drawing && (self.toolbar.is_near(mouse) || now < self.reveal_until);
        if reveal != self.toolbar.revealed {
            self.toolbar.set_revealed(reveal);
        }
        let clickable = if self.toolbar.revealed {
            let f = self.toolbar.frame();
            Some(RECT {
                left: frame.left + f.x.floor() as i32,
                top: frame.top + f.y.floor() as i32,
                right: frame.left + (f.x + f.w).ceil() as i32,
                bottom: frame.top + (f.y + f.h).ceil() as i32,
            })
        } else {
            None
        };
        hook::set_toolbar_rect(clickable);
        let toolbar_moving = self.toolbar.animate(dt);
        let hover_changed = self.toolbar.hover(mouse, now);

        // Over the taskbar, menus and our own windows the real cursor comes back.
        let over_normal = hook::zone_at(pt) == hook::Zone::Normal;
        let through = hook::click_through();
        let interacting = hook::interact_key_down() || hook::interact_pressing();
        let normal = !drawing
            && !self.prefs.interact_keeps_hand
            && ((interacting && !through) || over_normal || self.own_menu_open);
        if normal != self.normal_cursor {
            self.normal_cursor = normal;
            if normal {
                cursor::show();
            } else {
                cursor::hide();
            }
            self.dirty = true;
        }

        // Once a second: stay on top, and make sure the cursor is still hidden.
        if now.duration_since(self.last_upkeep) > Duration::from_secs(1) {
            self.last_upkeep = now;
            if !normal {
                cursor::reassert();
            }
            unsafe {
                let _ = SetWindowPos(
                    self.overlay,
                    Some(HWND_TOPMOST),
                    0,
                    0,
                    0,
                    0,
                    SWP_NOMOVE | SWP_NOSIZE | SWP_NOACTIVATE | SWP_NOREDRAW,
                );
            }
        }

        self.arm.step(dt, mouse.0, w);
        let size = self.prefs.pointer_size;
        let hand = match &self.asset {
            Some(asset) if self.style != styles::LASER => {
                Some(self.arm.hand_pose(mouse, (w, h), &asset.metrics, size, self.prefs.show_shadow))
            }
            _ => None,
        };
        let laser = self.arm.laser_pose(mouse, h, size);

        // Only draw when what is on screen would actually change.
        let q = |v: f32| (v * 50.0).round() as i32;
        let key = match &hand {
            Some(p) if !normal => [
                q(p.m[0] * 100.0),
                q(p.m[1] * 100.0),
                q(p.m[4]),
                q(p.m[5]),
                q(p.width),
                q(p.hand_h),
                q(p.rod_len),
                q(p.shadow_dy),
                q(p.shadow_opacity),
                0,
                0,
                1,
            ],
            None if !normal => [q(laser.x), q(laser.y), q(laser.radius), q(laser.glow), 0, 0, 0, 0, 0, 0, 0, 2],
            _ => [0; 12],
        };
        let changed = key != self.last_key || self.dirty || fading || toolbar_moving || hover_changed;
        let mut rendered = false;
        if changed {
            self.last_key = key;
            self.dirty = false;
            if let Some(r) = self.renderer.as_mut() {
                r.begin();
                r.draw_ink(&mut self.ink, now);
                if self.toolbar.visible() {
                    self.toolbar.draw(r, now);
                }
                if !normal {
                    match (&self.asset, &hand) {
                        (Some(asset), Some(pose)) => r.draw_hand(asset, pose),
                        _ => r.draw_laser(&laser),
                    }
                }
                if let Err(e) = r.end() {
                    crate::log!("drawing failed, rebuilding the renderer: {e}");
                    self.renderer = None;
                    self.dirty = true;
                }
                rendered = true;
                self.frames += 1;
            }
        }

        let still = now.duration_since(self.still_since).as_secs_f32() > IDLE_AFTER;
        let waiting_for_fade = self.ink.next_fade(now).map(|t| t - now < Duration::from_millis(60)).unwrap_or(false);
        let busy = !still
            || self.arm.held
            || self.arm.is_settling()
            || drawing
            || fading
            || toolbar_moving
            || waiting_for_fade
            || self.dirty;
        TickResult { rendered, busy }
    }

    // ---- The tray menu -----------------------------------------------------------------

    fn menu_labels(&self) -> (String, String, bool) {
        let toggle = format!(
            "{}\t{}",
            if self.active { "Turn Pointer Off" } else { "Turn Pointer On" },
            self.prefs.toggle_shortcut.display
        );
        let clear = format!("Clear Drawing\t{}", self.prefs.clear_shortcut.display);
        (toggle, clear, self.sound_on())
    }

    fn menu_command(&mut self, command: usize) {
        match command {
            MENU_TOGGLE => self.toggle(),
            MENU_CLEAR => self.clear_ink(),
            MENU_SOUND => {
                let on = self.sound_on();
                self.set_sound(!on);
            }
            MENU_SETTINGS => {
                self.exit();
                self.want_settings = true;
            }
            MENU_QUIT => self.quit(),
            _ => {}
        }
    }
}

/// Shows the tray menu. Runs outside any borrow of the app, because the menu pumps messages.
fn show_tray_menu(main: HWND, x: i32, y: i32) {
    let Some((toggle, clear, sound_on)) = with(|app| {
        app.own_menu_open = true;
        app.menu_labels()
    }) else {
        return;
    };
    let command = unsafe {
        let Ok(menu) = CreatePopupMenu() else { return };
        let item = |id: usize, text: &str, flags: MENU_ITEM_FLAGS| {
            let _ = AppendMenuW(menu, MF_STRING | flags, id, PCWSTR(wide(text).as_ptr()));
        };
        item(MENU_TOGGLE, &toggle, MENU_ITEM_FLAGS(0));
        item(MENU_CLEAR, &clear, MENU_ITEM_FLAGS(0));
        item(MENU_SOUND, "Tap Sound", if sound_on { MF_CHECKED } else { MF_UNCHECKED });
        let _ = AppendMenuW(menu, MF_SEPARATOR, 0, PCWSTR::null());
        item(MENU_SETTINGS, "Settings\u{2026}", MENU_ITEM_FLAGS(0));
        let _ = AppendMenuW(menu, MF_SEPARATOR, 0, PCWSTR::null());
        item(MENU_QUIT, "Quit TinyHand", MENU_ITEM_FLAGS(0));
        // Without this the menu doesn't close when the user clicks elsewhere.
        let _ = SetForegroundWindow(main);
        let picked = TrackPopupMenu(menu, TPM_RETURNCMD | TPM_NONOTIFY | TPM_RIGHTBUTTON, x, y, None, main, None);
        let _ = PostMessageW(Some(main), WM_NULL, WPARAM(0), LPARAM(0));
        let _ = DestroyMenu(menu);
        picked.0 as usize
    };
    with(|app| {
        app.own_menu_open = false;
        app.dirty = true;
        app.menu_command(command);
    });
}

/// Builds the tray icon: the arrow glyph (filled when the pointer is on) in the given colour.
fn make_tray_icon(on: bool, tint: u32) -> HICON {
    static OFF: &[u8] = include_bytes!("../assets/tray_off.png");
    static ON: &[u8] = include_bytes!("../assets/tray_on.png");
    let Ok(glyph) = image::load_from_memory_with_format(if on { ON } else { OFF }, image::ImageFormat::Png) else {
        return HICON::default();
    };
    unsafe {
        let side = GetSystemMetrics(SM_CXSMICON).max(16) as u32;
        let glyph = image::imageops::resize(&glyph.to_rgba8(), side, side, image::imageops::FilterType::Lanczos3);
        let (r, g, b) = ((tint >> 16) as u8, (tint >> 8) as u8, tint as u8);
        let mut bgra = Vec::with_capacity((side * side * 4) as usize);
        for p in glyph.pixels() {
            bgra.extend_from_slice(&[b, g, r, p.0[3]]);
        }
        let color = CreateBitmap(side as i32, side as i32, 1, 32, Some(bgra.as_ptr() as *const _));
        let mask = CreateBitmap(side as i32, side as i32, 1, 1, None);
        let info = ICONINFO { fIcon: TRUE, xHotspot: 0, yHotspot: 0, hbmMask: mask, hbmColor: color };
        let icon = CreateIconIndirect(&info).unwrap_or_default();
        let _ = DeleteObject(color.into());
        let _ = DeleteObject(mask.into());
        icon
    }
}

unsafe extern "system" fn main_proc(hwnd: HWND, msg: u32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    thread_local! {
        static TASKBAR_CREATED: u32 = unsafe { RegisterWindowMessageW(w!("TaskbarCreated")) };
    }
    match msg {
        WM_TRAY => {
            let event = (lparam.0 as u32) & 0xFFFF;
            let x = (wparam.0 & 0xFFFF) as i16 as i32;
            let y = ((wparam.0 >> 16) & 0xFFFF) as i16 as i32;
            match event {
                NIN_SELECT | NIN_KEYSELECT => {
                    with(|app| app.toggle());
                }
                WM_CONTEXTMENU => show_tray_menu(hwnd, x, y),
                _ => {}
            }
            LRESULT(0)
        }
        WM_HOTKEY => {
            with(|app| match wparam.0 as i32 {
                HOTKEY_TOGGLE => app.toggle(),
                HOTKEY_CLEAR => app.clear_ink(),
                _ => {}
            });
            LRESULT(0)
        }
        hook::EV_MOUSE_DOWN => {
            with(|app| app.on_mouse_down(wparam.0 == 1, hook::point_from_lparam(lparam)));
            LRESULT(0)
        }
        hook::EV_MOUSE_UP => {
            with(|app| app.on_mouse_up(wparam.0 == 1));
            LRESULT(0)
        }
        hook::EV_INTERACT_DOWN => {
            with(|app| {
                app.arm.press();
                app.arm.held = true;
                app.wake();
            });
            LRESULT(0)
        }
        hook::EV_INTERACT_UP => {
            with(|app| {
                app.arm.held = false;
                app.wake();
            });
            LRESULT(0)
        }
        hook::EV_TOOLBAR_CLICK => {
            with(|app| app.on_toolbar_click(hook::point_from_lparam(lparam)));
            LRESULT(0)
        }
        hook::EV_ESC => {
            with(|app| app.exit());
            LRESULT(0)
        }
        hook::EV_SHIFT_DOUBLE => {
            with(|app| {
                if app.prefs.double_shift_click_through {
                    let on = hook::click_through();
                    app.set_click_through(!on);
                }
            });
            LRESULT(0)
        }
        hook::EV_WAKE => {
            hook::wake_handled();
            with(|app| app.wake());
            LRESULT(0)
        }
        WM_PREFS_CHANGED => {
            with(|app| app.refresh());
            settings::prefs_changed();
            LRESULT(0)
        }
        WM_TIMER if wparam.0 == TIMER_TICK => {
            // Keeps the pointer moving while a menu or a window drag runs its own message loop.
            with(|app| {
                if app.active && app.last_tick.elapsed() > Duration::from_millis(12) {
                    app.tick();
                }
            });
            LRESULT(0)
        }
        WM_COPYDATA => {
            let data = &*(lparam.0 as *const COPYDATASTRUCT);
            if data.dwData == COPYDATA_COMMAND && !data.lpData.is_null() {
                let bytes = std::slice::from_raw_parts(data.lpData as *const u8, data.cbData as usize);
                let command = String::from_utf8_lossy(bytes).into_owned();
                with(|app| automation::handle(app, &command));
                return LRESULT(1);
            }
            LRESULT(0)
        }
        WM_WTSSESSION_CHANGE => {
            if wparam.0 as u32 == WTS_SESSION_LOCK {
                with(|app| app.deactivate());
            }
            LRESULT(0)
        }
        WM_POWERBROADCAST => {
            if wparam.0 as u32 == PBT_APMSUSPEND {
                with(|app| app.deactivate());
            }
            LRESULT(1)
        }
        WM_DISPLAYCHANGE | WM_DPICHANGED => {
            with(|app| {
                app.frame = None;
                app.dirty = true;
                app.update_tray();
            });
            LRESULT(0)
        }
        WM_SETTINGCHANGE => {
            with(|app| app.update_tray());
            DefWindowProcW(hwnd, msg, wparam, lparam)
        }
        WM_ENDSESSION => {
            if wparam.0 != 0 {
                shutdown();
            }
            LRESULT(0)
        }
        WM_CLOSE => {
            with(|app| app.quit());
            LRESULT(0)
        }
        _ => {
            if msg == TASKBAR_CREATED.with(|m| *m) {
                // Explorer restarted: the tray icon has to be added again.
                with(|app| {
                    app.tray_added = false;
                    app.update_tray();
                });
                return LRESULT(0);
            }
            DefWindowProcW(hwnd, msg, wparam, lparam)
        }
    }
}

unsafe extern "system" fn overlay_proc(hwnd: HWND, msg: u32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    match msg {
        WM_NCHITTEST => LRESULT(HTTRANSPARENT as isize),
        WM_MOUSEACTIVATE => LRESULT(MA_NOACTIVATE as isize),
        WM_ERASEBKGND => LRESULT(1),
        WM_PAINT => {
            // Nothing to paint (DirectComposition draws the window), but the paint request
            // has to be acknowledged properly or Windows sends it again for ever.
            let mut ps = PAINTSTRUCT::default();
            BeginPaint(hwnd, &mut ps);
            let _ = EndPaint(hwnd, &ps);
            LRESULT(0)
        }
        _ => DefWindowProcW(hwnd, msg, wparam, lparam),
    }
}

fn notify_prefs_changed() {
    unsafe {
        let _ = PostMessageW(Some(main_hwnd()), WM_PREFS_CHANGED, WPARAM(0), LPARAM(0));
    }
}

/// Creates the windows, the tray icon and the hooks. Returns false if that fails.
pub fn init() -> bool {
    unsafe {
        let instance: HINSTANCE = GetModuleHandleW(None).map(|m| m.into()).unwrap_or_default();
        let icon = LoadIconW(Some(instance), PCWSTR(1 as *const u16)).unwrap_or_default();
        let main_class = WNDCLASSW {
            lpfnWndProc: Some(main_proc),
            hInstance: instance,
            lpszClassName: MAIN_CLASS,
            hIcon: icon,
            ..Default::default()
        };
        let overlay_class = WNDCLASSW {
            lpfnWndProc: Some(overlay_proc),
            hInstance: instance,
            lpszClassName: OVERLAY_CLASS,
            ..Default::default()
        };
        RegisterClassW(&main_class);
        RegisterClassW(&overlay_class);

        let Ok(main) = CreateWindowExW(
            WS_EX_TOOLWINDOW,
            MAIN_CLASS,
            w!("TinyHand"),
            WS_POPUP,
            0,
            0,
            0,
            0,
            None,
            None,
            Some(instance),
            None,
        ) else {
            return false;
        };
        // Click-through, never focused, always on top, and drawn by DirectComposition
        // (no redirection bitmap), which is what makes it truly transparent.
        let Ok(overlay) = CreateWindowExW(
            WS_EX_NOREDIRECTIONBITMAP | WS_EX_LAYERED | WS_EX_TRANSPARENT | WS_EX_TOPMOST | WS_EX_TOOLWINDOW | WS_EX_NOACTIVATE,
            OVERLAY_CLASS,
            w!("TinyHand pointer"),
            WS_POPUP,
            0,
            0,
            16,
            16,
            None,
            None,
            Some(instance),
            None,
        ) else {
            return false;
        };
        let _ = SetLayeredWindowAttributes(overlay, COLORREF(0), 255, LWA_ALPHA);
        MAIN_HWND.store(main.0 as isize, SeqCst);
        let _ = WTSRegisterSessionNotification(main, NOTIFY_FOR_THIS_SESSION);

        cursor::init();
        hook::start(main);
        prefs::set_on_changed(notify_prefs_changed);

        let prefs = prefs::get();
        let now = Instant::now();
        let mut app = App {
            main,
            overlay,
            renderer: None,
            style: prefs.style(),
            prefs,
            active: false,
            asset: None,
            arm: Arm::default(),
            ink: Ink::default(),
            toolbar: Toolbar::new(),
            frame: None,
            down_point: None,
            buttons_held: 0,
            normal_cursor: false,
            own_menu_open: false,
            reveal_until: now,
            hint_until: None,
            still_since: now,
            last_mouse: POINT::default(),
            last_tick: now,
            last_upkeep: now,
            last_key: [0; 12],
            dirty: true,
            drag_points: Vec::new(),
            hotkeys_on: false,
            hotkeys_suspended: false,
            tray_added: false,
            tray_icon: HICON::default(),
            timer_period: false,
            frames: 0,
            want_settings: false,
            want_quit: false,
        };
        let style = app.style.clone();
        app.apply_style(&style);
        app.register_hotkeys();
        app.state_changed();
        app.show_welcome();
        APP.with(|cell| *cell.borrow_mut() = Some(app));
    }
    true
}

/// Puts everything back: cursor, tray icon, state file. Safe to call more than once.
pub fn shutdown() {
    with(|app| {
        app.deactivate();
        app.unregister_hotkeys();
        app.remove_tray();
    });
    cursor::shutdown();
    automation::remove_state();
}

/// The message loop. While the pointer is on it also drives the frames.
pub fn run() {
    while pump_once(None) {}
    shutdown();
}

/// One turn of the message loop: handle waiting messages, draw a frame if the pointer is on,
/// then sleep until there is something to do (at most `max_wait_ms` if given).
/// Returns false once the app has been asked to quit.
pub fn pump_once(max_wait_ms: Option<u32>) -> bool {
    unsafe {
        let mut msg = MSG::default();
        while PeekMessageW(&mut msg, None, 0, 0, PM_REMOVE).as_bool() {
            if msg.message == WM_QUIT {
                return false;
            }
            if !settings::pre_translate(&msg) {
                let _ = TranslateMessage(&msg);
                DispatchMessageW(&msg);
            }
        }
        let started = Instant::now();
        let wait = match with(|app| app.active.then(|| app.tick())).flatten() {
            None => max_wait_ms.unwrap_or(u32::MAX),
            // Presenting normally waits for the screen's refresh. If it didn't (or nothing
            // was drawn), wait a little so the loop never spins.
            Some(tick) if tick.rendered => {
                if started.elapsed() < Duration::from_millis(3) {
                    4
                } else {
                    0
                }
            }
            Some(tick) if tick.busy => 3,
            Some(_) => 50,
        };
        let wait = max_wait_ms.map(|m| wait.min(m)).unwrap_or(wait);
        if wait > 0 {
            MsgWaitForMultipleObjects(None, false, wait, QS_ALLINPUT);
        }
    }
    true
}
