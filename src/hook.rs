//! Low-level mouse and keyboard hooks: they turn clicks into taps while the pointer is on.
//!
//! The hooks live on their own thread with its own message loop, so a busy main thread can
//! never make the whole system's mouse input lag. The hook thread decides on the spot whether
//! an event is swallowed, and tells the main thread what happened by posting window messages.

use crate::prefs::InteractKey;
use std::sync::atomic::{AtomicBool, AtomicIsize, AtomicU32, AtomicU64, AtomicU8, Ordering::SeqCst};
use std::sync::mpsc::{channel, Sender};
use std::sync::{Mutex, OnceLock};
use std::time::Instant;
use windows::Win32::Foundation::{HWND, LPARAM, LRESULT, POINT, RECT, WPARAM};
use windows::Win32::System::Threading::{GetCurrentProcessId, GetCurrentThreadId};
use windows::Win32::UI::Input::KeyboardAndMouse::*;
use windows::Win32::UI::WindowsAndMessaging::*;

/// Messages posted to the main window. `lparam` carries the screen position for mouse events.
pub const EV_MOUSE_DOWN: u32 = WM_APP + 10; // wparam: 1 if the left button
pub const EV_MOUSE_UP: u32 = WM_APP + 11; // wparam: 1 if the left button
pub const EV_INTERACT_DOWN: u32 = WM_APP + 12;
pub const EV_INTERACT_UP: u32 = WM_APP + 13;
pub const EV_TOOLBAR_CLICK: u32 = WM_APP + 14;
pub const EV_ESC: u32 = WM_APP + 15;
pub const EV_SHIFT_DOUBLE: u32 = WM_APP + 16;
pub const EV_WAKE: u32 = WM_APP + 17;

const CMD_INSTALL: u32 = WM_APP + 1;
const CMD_REMOVE: u32 = WM_APP + 2;

/// Marks input we injected ourselves, so the hooks let it through untouched.
const MAGIC: usize = 0x7448_616E; // "tHan"

/// An unassigned key. Tapping it while Alt or Win is held stops the release of that key
/// from opening a menu bar or the Start menu.
const VK_MASK: VIRTUAL_KEY = VIRTUAL_KEY(0xE8);

static MAIN_HWND: AtomicIsize = AtomicIsize::new(0);
static THREAD_ID: AtomicU32 = AtomicU32::new(0);
static ACTIVE: AtomicBool = AtomicBool::new(false);
static INSTALLED: AtomicBool = AtomicBool::new(false);
static CLICK_THROUGH: AtomicBool = AtomicBool::new(false);
static ESC_ENABLED: AtomicBool = AtomicBool::new(true);
static DOUBLE_SHIFT: AtomicBool = AtomicBool::new(true);
static INTERACT_KEY: AtomicU8 = AtomicU8::new(0);
static INTERACT_PRESSING: AtomicBool = AtomicBool::new(false);
static WAKE_PENDING: AtomicBool = AtomicBool::new(false);
static LEFT_TAP_HELD: AtomicBool = AtomicBool::new(false);
/// Keys and clicks seen so far. Comparing it across a Shift tap tells a clean tap from typing
/// a capital letter, without looking at which keys were pressed.
static INPUT_COUNT: AtomicU64 = AtomicU64::new(0);

static TOOLBAR_RECT: Mutex<Option<RECT>> = Mutex::new(None);
static DRAG_POINTS: Mutex<Vec<POINT>> = Mutex::new(Vec::new());

/// What a pressed mouse button is doing, decided when it goes down.
#[derive(Clone, Copy, PartialEq)]
enum Press {
    None,
    /// Swallowed and shown as a tap.
    Tap,
    /// Reaches the app because the interact key is held or click-through is latched.
    Interact,
    /// Reaches whatever is underneath (taskbar, menus, TinyHand's own windows).
    Pass,
    /// Swallowed: it landed on the toolbar.
    Toolbar,
}

/// State touched only on the hook thread.
struct Local {
    mouse_hook: HHOOK,
    key_hook: HHOOK,
    presses: [Press; 5],
    /// A press was swallowed and its replay hasn't come back through the hook yet.
    replaying: [bool; 5],
    /// Which variants (left, right) of the interact key are physically down.
    phys: [bool; 2],
    /// The system has been told the interact key is up although it is still held.
    stripped: bool,
    shift_is_down: bool,
    shift_down_at: Option<Instant>,
    shift_down_inputs: u64,
    last_shift_tap: Option<Instant>,
    last_shift_tap_inputs: u64,
    esc_swallowed: bool,
}

static mut LOCAL: Local = Local {
    mouse_hook: HHOOK(std::ptr::null_mut()),
    key_hook: HHOOK(std::ptr::null_mut()),
    presses: [Press::None; 5],
    replaying: [false; 5],
    phys: [false; 2],
    stripped: false,
    shift_is_down: false,
    shift_down_at: None,
    shift_down_inputs: 0,
    last_shift_tap: None,
    last_shift_tap_inputs: 0,
    esc_swallowed: false,
};

#[allow(static_mut_refs)]
fn local() -> &'static mut Local {
    // Only ever called on the hook thread.
    unsafe { &mut LOCAL }
}

const SHIFT_TAP_MAX: f64 = 0.3;
const SHIFT_DOUBLE_MAX: f64 = 0.35;

/// Starts the hook thread. `main` receives the `EV_*` messages.
pub fn start(main: HWND) {
    MAIN_HWND.store(main.0 as isize, SeqCst);
    let (ready_tx, ready_rx) = std::sync::mpsc::channel();
    let _ = std::thread::Builder::new().name("TinyHand input hooks".into()).spawn(move || unsafe {
        let mut msg = MSG::default();
        // Force the thread's message queue into existence before announcing the thread id.
        let _ = PeekMessageW(&mut msg, None, WM_USER, WM_USER, PM_NOREMOVE);
        THREAD_ID.store(GetCurrentThreadId(), SeqCst);
        let _ = ready_tx.send(());
        while GetMessageW(&mut msg, None, 0, 0).as_bool() {
            match msg.message {
                CMD_INSTALL => install(),
                CMD_REMOVE => remove(),
                _ => {
                    let _ = TranslateMessage(&msg);
                    DispatchMessageW(&msg);
                }
            }
        }
    });
    let _ = ready_rx.recv();
}

fn install() {
    let l = local();
    if !l.mouse_hook.0.is_null() {
        return;
    }
    l.presses = [Press::None; 5];
    l.replaying = [false; 5];
    l.stripped = false;
    l.shift_is_down = false;
    l.shift_down_at = None;
    l.last_shift_tap = None;
    l.esc_swallowed = false;
    refresh_phys(l);
    unsafe {
        l.mouse_hook = SetWindowsHookExW(WH_MOUSE_LL, Some(mouse_proc), None, 0).unwrap_or_default();
        l.key_hook = SetWindowsHookExW(WH_KEYBOARD_LL, Some(key_proc), None, 0).unwrap_or_default();
    }
    INSTALLED.store(!l.mouse_hook.0.is_null() && !l.key_hook.0.is_null(), SeqCst);
}

fn remove() {
    let l = local();
    unsafe {
        if !l.mouse_hook.0.is_null() {
            let _ = UnhookWindowsHookEx(l.mouse_hook);
            l.mouse_hook = HHOOK::default();
        }
        if !l.key_hook.0.is_null() {
            let _ = UnhookWindowsHookEx(l.key_hook);
            l.key_hook = HHOOK::default();
        }
    }
    INSTALLED.store(false, SeqCst);
    PHYS_DOWN.store(false, SeqCst);
    INTERACT_PRESSING.store(false, SeqCst);
    LEFT_TAP_HELD.store(false, SeqCst);
}

/// Turns click interception on or off. Returns straight away; the hooks are installed
/// or removed on the hook thread a moment later.
pub fn set_active(active: bool) {
    ACTIVE.store(active, SeqCst);
    if !active {
        INTERACT_PRESSING.store(false, SeqCst);
        LEFT_TAP_HELD.store(false, SeqCst);
        DRAG_POINTS.lock().unwrap().clear();
    }
    let thread = THREAD_ID.load(SeqCst);
    if thread != 0 {
        unsafe {
            let _ = PostThreadMessageW(thread, if active { CMD_INSTALL } else { CMD_REMOVE }, WPARAM(0), LPARAM(0));
        }
    }
}

pub fn is_installed() -> bool {
    INSTALLED.load(SeqCst)
}

/// Latched by double-tapping Shift: every click reaches the app, as if the interact key were held.
pub fn set_click_through(on: bool) {
    CLICK_THROUGH.store(on, SeqCst);
}

pub fn click_through() -> bool {
    CLICK_THROUGH.load(SeqCst)
}

pub fn set_esc_enabled(on: bool) {
    ESC_ENABLED.store(on, SeqCst);
}

pub fn set_double_shift(on: bool) {
    DOUBLE_SHIFT.store(on, SeqCst);
}

pub fn set_interact_key(key: InteractKey) {
    let v = match key {
        InteractKey::Off => 0,
        InteractKey::Alt => 1,
        InteractKey::Ctrl => 2,
        InteractKey::Win => 3,
    };
    INTERACT_KEY.store(v, SeqCst);
}

/// True while a click that reaches the app is held down.
pub fn interact_pressing() -> bool {
    INTERACT_PRESSING.load(SeqCst)
}

/// True while the interact key is held, whether or not the system has been told it is up.
pub fn interact_key_down() -> bool {
    // While "stripped" the system believes the key is up, so ask the hook thread's own record.
    PHYS_DOWN.load(SeqCst)
}

static PHYS_DOWN: AtomicBool = AtomicBool::new(false);

fn publish_phys(l: &Local) {
    PHYS_DOWN.store(l.phys[0] || l.phys[1], SeqCst);
}

/// The toolbar's on-screen rectangle while it can be clicked, or `None`.
pub fn set_toolbar_rect(rect: Option<RECT>) {
    *TOOLBAR_RECT.lock().unwrap() = rect;
}

/// Takes the mouse positions recorded since the last call while the left button draws.
pub fn take_drag_points(into: &mut Vec<POINT>) {
    into.append(&mut DRAG_POINTS.lock().unwrap());
}

/// Call when the main thread handles `EV_WAKE`, so the next mouse move can post another.
pub fn wake_handled() {
    WAKE_PENDING.store(false, SeqCst);
}

fn interact_vks() -> Option<(VIRTUAL_KEY, VIRTUAL_KEY)> {
    match INTERACT_KEY.load(SeqCst) {
        1 => Some((VK_LMENU, VK_RMENU)),
        2 => Some((VK_LCONTROL, VK_RCONTROL)),
        3 => Some((VK_LWIN, VK_RWIN)),
        _ => None,
    }
}

fn key_down(vk: VIRTUAL_KEY) -> bool {
    unsafe { (GetAsyncKeyState(vk.0 as i32) as u16 & 0x8000) != 0 }
}

fn refresh_phys(l: &mut Local) {
    l.phys = match interact_vks() {
        Some((a, b)) => [key_down(a), key_down(b)],
        None => [false, false],
    };
    publish_phys(l);
}

fn post(msg: u32, wparam: usize, pt: POINT) {
    let hwnd = HWND(MAIN_HWND.load(SeqCst) as *mut _);
    let lparam = ((pt.y as i32 as u32 as u64) << 32 | (pt.x as i32 as u32 as u64)) as isize;
    unsafe {
        let _ = PostMessageW(Some(hwnd), msg, WPARAM(wparam), LPARAM(lparam));
    }
}

/// Unpacks the screen position from an `EV_*` message's `lparam`.
pub fn point_from_lparam(lparam: LPARAM) -> POINT {
    let v = lparam.0 as u64;
    POINT { x: v as u32 as i32, y: (v >> 32) as u32 as i32 }
}

fn wake() {
    if !WAKE_PENDING.swap(true, SeqCst) {
        post(EV_WAKE, 0, POINT::default());
    }
}

/// Where a point on screen falls, as far as clicks are concerned.
#[derive(Clone, Copy, PartialEq, Debug)]
pub enum Zone {
    /// Ordinary screen: clicks become taps.
    Tap,
    /// The TinyHand toolbar.
    Toolbar,
    /// The taskbar, an open menu, a shell flyout or one of TinyHand's own windows:
    /// clicks go through and the normal cursor shows.
    Normal,
}

const NORMAL_CLASSES: [&str; 8] = [
    "Shell_TrayWnd",
    "Shell_SecondaryTrayWnd",
    "#32768",
    "NotifyIconOverflowWindow",
    "TopLevelWindowForOverflowXamlIsland",
    "Xaml_WindowedPopupClass",
    "Windows.UI.Core.CoreWindow",
    "XamlExplorerHostIslandWindow",
];

pub fn zone_at(pt: POINT) -> Zone {
    if let Some(r) = *TOOLBAR_RECT.lock().unwrap() {
        if pt.x >= r.left && pt.x < r.right && pt.y >= r.top && pt.y < r.bottom {
            return Zone::Toolbar;
        }
    }
    unsafe {
        // The overlay is transparent to hit-testing, so this finds what is underneath it.
        let hit = WindowFromPoint(pt);
        if hit.0.is_null() {
            return Zone::Tap;
        }
        let root = GetAncestor(hit, GA_ROOT);
        let root = if root.0.is_null() { hit } else { root };
        let mut pid = 0u32;
        GetWindowThreadProcessId(root, Some(&mut pid));
        if pid == GetCurrentProcessId() {
            return Zone::Normal;
        }
        let mut buf = [0u16; 96];
        let n = GetClassNameW(root, &mut buf) as usize;
        let class = String::from_utf16_lossy(&buf[..n.min(buf.len())]);
        if NORMAL_CLASSES.iter().any(|c| *c == class) {
            return Zone::Normal;
        }
    }
    Zone::Tap
}

/// True while a classic popup menu is open anywhere.
pub fn menu_open() -> bool {
    unsafe {
        match FindWindowW(windows::core::w!("#32768"), None) {
            Ok(h) => !h.0.is_null() && IsWindowVisible(h).as_bool(),
            Err(_) => false,
        }
    }
}

fn key_input(vk: VIRTUAL_KEY, up: bool) -> INPUT {
    let mut flags = if up { KEYEVENTF_KEYUP } else { KEYBD_EVENT_FLAGS(0) };
    if vk == VK_RMENU || vk == VK_RCONTROL || vk == VK_LWIN || vk == VK_RWIN {
        flags |= KEYEVENTF_EXTENDEDKEY;
    }
    INPUT {
        r#type: INPUT_KEYBOARD,
        Anonymous: INPUT_0 { ki: KEYBDINPUT { wVk: vk, wScan: 0, dwFlags: flags, time: 0, dwExtraInfo: MAGIC } },
    }
}

struct Batch(Vec<INPUT>);
// `INPUT` is plain data; the compiler just can't see that through its union.
unsafe impl Send for Batch {}

static INJECTOR: OnceLock<Sender<Batch>> = OnceLock::new();

/// Queues synthetic input. It is sent from a helper thread, in order, because calling
/// `SendInput` inside a hook callback stalls until Windows gives up on the hook.
fn send(inputs: &[INPUT]) {
    if inputs.is_empty() {
        return;
    }
    let injector = INJECTOR.get_or_init(|| {
        let (tx, rx) = channel::<Batch>();
        let _ = std::thread::Builder::new().name("TinyHand input replay".into()).spawn(move || {
            while let Ok(batch) = rx.recv() {
                unsafe {
                    SendInput(&batch.0, std::mem::size_of::<INPUT>() as i32);
                }
            }
        });
        tx
    });
    let _ = injector.send(Batch(inputs.to_vec()));
}

fn mouse_input(flag: MOUSE_EVENT_FLAGS, data: u32) -> INPUT {
    INPUT {
        r#type: INPUT_MOUSE,
        Anonymous: INPUT_0 { mi: MOUSEINPUT { dx: 0, dy: 0, mouseData: data, dwFlags: flag, time: 0, dwExtraInfo: MAGIC } },
    }
}

fn mask_tap() -> [INPUT; 2] {
    [key_input(VK_MASK, false), key_input(VK_MASK, true)]
}

/// Button index, whether it is a press, and the `SendInput` flag and data that repeat it.
fn classify(msg: u32, mouse_data: u32) -> Option<(usize, bool, MOUSE_EVENT_FLAGS, u32)> {
    let x = (mouse_data >> 16) & 0xFFFF;
    Some(match msg {
        WM_LBUTTONDOWN => (0, true, MOUSEEVENTF_LEFTDOWN, 0),
        WM_LBUTTONUP => (0, false, MOUSEEVENTF_LEFTUP, 0),
        WM_RBUTTONDOWN => (1, true, MOUSEEVENTF_RIGHTDOWN, 0),
        WM_RBUTTONUP => (1, false, MOUSEEVENTF_RIGHTUP, 0),
        WM_MBUTTONDOWN => (2, true, MOUSEEVENTF_MIDDLEDOWN, 0),
        WM_MBUTTONUP => (2, false, MOUSEEVENTF_MIDDLEUP, 0),
        WM_XBUTTONDOWN => (if x == 2 { 4 } else { 3 }, true, MOUSEEVENTF_XDOWN, x),
        WM_XBUTTONUP => (if x == 2 { 4 } else { 3 }, false, MOUSEEVENTF_XUP, x),
        _ => return None,
    })
}

unsafe extern "system" fn mouse_proc(code: i32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    if code < 0 || !ACTIVE.load(SeqCst) {
        return CallNextHookEx(None, code, wparam, lparam);
    }
    let info = &*(lparam.0 as *const MSLLHOOKSTRUCT);
    let msg = wparam.0 as u32;
    if info.dwExtraInfo == MAGIC {
        // Our own replayed press has arrived; later events for that button are real again.
        if let Some((button, true, _, _)) = classify(msg, info.mouseData) {
            local().replaying[button] = false;
        }
        return CallNextHookEx(None, code, wparam, lparam);
    }
    if msg == WM_MOUSEMOVE {
        if LEFT_TAP_HELD.load(SeqCst) {
            DRAG_POINTS.lock().unwrap().push(info.pt);
        }
        wake();
        return CallNextHookEx(None, code, wparam, lparam);
    }
    let Some((button, down, flag, data)) = classify(msg, info.mouseData) else {
        return CallNextHookEx(None, code, wparam, lparam);
    };
    let l = local();
    let left = button == 0;

    if !down {
        let press = std::mem::replace(&mut l.presses[button], Press::None);
        return match press {
            Press::Tap => {
                if left {
                    LEFT_TAP_HELD.store(false, SeqCst);
                }
                post(EV_MOUSE_UP, left as usize, info.pt);
                LRESULT(1)
            }
            Press::Toolbar => LRESULT(1),
            Press::Interact => {
                if !l.presses.contains(&Press::Interact) {
                    INTERACT_PRESSING.store(false, SeqCst);
                    post(EV_INTERACT_UP, 0, info.pt);
                }
                if l.replaying[button] {
                    // The release beat the replayed press: queue it behind the press.
                    send(&[mouse_input(flag, data)]);
                    return LRESULT(1);
                }
                CallNextHookEx(None, code, wparam, lparam)
            }
            Press::Pass | Press::None => CallNextHookEx(None, code, wparam, lparam),
        };
    }

    INPUT_COUNT.fetch_add(1, SeqCst);
    let via_key = l.phys[0] || l.phys[1];
    if via_key || CLICK_THROUGH.load(SeqCst) {
        l.presses[button] = Press::Interact;
        INTERACT_PRESSING.store(true, SeqCst);
        post(EV_INTERACT_DOWN, 0, info.pt);
        if via_key && !l.stripped {
            // Give the app a plain click, not Alt+click: tell the system the key went up,
            // then replay the press. The key stays "up" for the system until it is released.
            if let Some((a, b)) = interact_vks() {
                let mut inputs: Vec<INPUT> = Vec::with_capacity(5);
                if a != VK_LCONTROL {
                    inputs.extend(mask_tap());
                }
                if l.phys[0] {
                    inputs.push(key_input(a, true));
                }
                if l.phys[1] {
                    inputs.push(key_input(b, true));
                }
                inputs.push(mouse_input(flag, data));
                l.stripped = true;
                l.replaying[button] = true;
                send(&inputs);
                return LRESULT(1);
            }
        }
        return CallNextHookEx(None, code, wparam, lparam);
    }

    match zone_at(info.pt) {
        Zone::Toolbar => {
            l.presses[button] = Press::Toolbar;
            if left {
                post(EV_TOOLBAR_CLICK, 0, info.pt);
            }
            LRESULT(1)
        }
        Zone::Normal => {
            l.presses[button] = Press::Pass;
            CallNextHookEx(None, code, wparam, lparam)
        }
        Zone::Tap => {
            l.presses[button] = Press::Tap;
            if left {
                DRAG_POINTS.lock().unwrap().clear();
                LEFT_TAP_HELD.store(true, SeqCst);
            }
            post(EV_MOUSE_DOWN, left as usize, info.pt);
            LRESULT(1)
        }
    }
}

unsafe extern "system" fn key_proc(code: i32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    if code < 0 || !ACTIVE.load(SeqCst) {
        return CallNextHookEx(None, code, wparam, lparam);
    }
    let info = &*(lparam.0 as *const KBDLLHOOKSTRUCT);
    if info.dwExtraInfo == MAGIC {
        return CallNextHookEx(None, code, wparam, lparam);
    }
    let vk = VIRTUAL_KEY(info.vkCode as u16);
    let msg = wparam.0 as u32;
    let down = msg == WM_KEYDOWN || msg == WM_SYSKEYDOWN;
    let l = local();

    // The interact key.
    if let Some((a, b)) = interact_vks() {
        if vk == a || vk == b {
            let i = if vk == a { 0 } else { 1 };
            if down {
                if l.stripped {
                    // Auto-repeat of a key the system already believes is up.
                    l.phys[i] = true;
                    publish_phys(l);
                    return LRESULT(1);
                }
                let first = !l.phys[i];
                l.phys[i] = true;
                publish_phys(l);
                if first {
                    wake();
                    if a != VK_LCONTROL {
                        // Runs after this key-down is delivered; keeps a bare hold-and-release
                        // from opening the menu bar or the Start menu.
                        send(&mask_tap());
                    }
                }
            } else {
                l.phys[i] = false;
                publish_phys(l);
                wake();
                if l.stripped {
                    if !l.phys[0] && !l.phys[1] {
                        l.stripped = false;
                    }
                    return LRESULT(1);
                }
            }
            return CallNextHookEx(None, code, wparam, lparam);
        }
    }

    let is_shift = vk == VK_LSHIFT || vk == VK_RSHIFT || vk == VK_SHIFT;
    if is_shift {
        if !DOUBLE_SHIFT.load(SeqCst) {
            return CallNextHookEx(None, code, wparam, lparam);
        }
        let now = Instant::now();
        if down {
            if !l.shift_is_down {
                l.shift_is_down = true;
                let others = [VK_CONTROL, VK_MENU, VK_LWIN, VK_RWIN].iter().any(|k| key_down(*k));
                if others {
                    l.shift_down_at = None;
                    l.last_shift_tap = None;
                } else {
                    l.shift_down_at = Some(now);
                    l.shift_down_inputs = INPUT_COUNT.load(SeqCst);
                }
            }
        } else {
            l.shift_is_down = false;
            if let Some(down_at) = l.shift_down_at.take() {
                let inputs = INPUT_COUNT.load(SeqCst);
                let clean = now.duration_since(down_at).as_secs_f64() < SHIFT_TAP_MAX && inputs == l.shift_down_inputs;
                if !clean {
                    l.last_shift_tap = None;
                } else {
                    let double = l
                        .last_shift_tap
                        .map(|t| down_at.saturating_duration_since(t).as_secs_f64() < SHIFT_DOUBLE_MAX)
                        .unwrap_or(false)
                        && l.shift_down_inputs == l.last_shift_tap_inputs;
                    if double {
                        l.last_shift_tap = None;
                        post(EV_SHIFT_DOUBLE, 0, POINT::default());
                    } else {
                        l.last_shift_tap = Some(now);
                        l.last_shift_tap_inputs = inputs;
                    }
                }
            }
        }
        return CallNextHookEx(None, code, wparam, lparam);
    }

    if vk == VK_ESCAPE {
        if down {
            let plain = ![VK_CONTROL, VK_MENU, VK_SHIFT, VK_LWIN, VK_RWIN].iter().any(|k| key_down(*k));
            if plain && ESC_ENABLED.load(SeqCst) && !CLICK_THROUGH.load(SeqCst) && !menu_open() {
                if !l.esc_swallowed {
                    l.esc_swallowed = true;
                    post(EV_ESC, 0, POINT::default());
                }
                return LRESULT(1);
            }
        } else if l.esc_swallowed {
            l.esc_swallowed = false;
            return LRESULT(1);
        }
    }

    if down {
        INPUT_COUNT.fetch_add(1, SeqCst);
        if vk == VK_LCONTROL || vk == VK_RCONTROL || vk == VK_CONTROL {
            wake(); // Ctrl adds an arrowhead to the line being drawn.
        }
    } else if vk == VK_LCONTROL || vk == VK_RCONTROL || vk == VK_CONTROL {
        wake();
    }
    CallNextHookEx(None, code, wparam, lparam)
}
