//! The settings window: a normal top-level window, drawn entirely with Direct2D in the
//! style of Windows 11 Settings, with a sidebar of sections and cards of settings.
//!
//! Layout and input share one source of truth: each paint records the clickable rectangles
//! (see [`view::Frame`]) and the mouse is hit-tested against the last paint.
//!
//! Re-entrancy: the window state lives in a thread-local `RefCell` and is only borrowed for
//! short stretches that never pump messages. Anything that can (message boxes, dialogs,
//! opening links, talking to the app or to child controls) happens in [`actions`] or in the
//! window procedure with the borrow released.

mod actions;
mod canvas;
mod edits;
mod input;
mod maker;
mod pages;
mod pointers;
mod snapshot;
mod ui;
mod view;

use crate::{prefs, registry, styles};
use canvas::{Canvas, RECREATE_TARGET};
use edits::Edits;
use image::RgbaImage;
use std::cell::RefCell;
use ui::{Theme, Ui};
use view::{Action, Ctx, Field, Frame, Section, TileDrag, View, WIN_H, WIN_W};
use windows::core::{w, BOOL, PCWSTR};
use windows::Win32::Foundation::*;
use windows::Win32::Graphics::Dwm::{DwmSetWindowAttribute, DWMWA_USE_IMMERSIVE_DARK_MODE};
use windows::Win32::Graphics::Gdi::*;
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::UI::Controls::WM_MOUSELEAVE;
use windows::Win32::UI::HiDpi::{AdjustWindowRectExForDpi, GetDpiForMonitor, GetDpiForWindow, MDT_EFFECTIVE_DPI};
use windows::Win32::UI::Input::KeyboardAndMouse::*;
use windows::Win32::UI::Shell::{DragAcceptFiles, DragFinish, DragQueryFileW, HDROP};
use windows::Win32::UI::WindowsAndMessaging::*;

/// Draws one section to a PNG, with no window on screen (`--settings-test`). See
/// [`snapshot::render_test`] for the extras `section` can carry.
pub fn render_test(out: &str, section: &str, dark: bool) -> Result<(), String> {
    snapshot::render_test(out, section, dark)
}

/// Canvas cache key of the app icon.
const APP_ICON_KEY: &str = "app-icon";
const CLASS: PCWSTR = w!("TinyHandSettings");
const TIMER_COPIED: usize = 1;
const STYLE: WINDOW_STYLE = WINDOW_STYLE(WS_OVERLAPPED.0 | WS_CAPTION.0 | WS_SYSMENU.0 | WS_MINIMIZEBOX.0 | WS_CLIPCHILDREN.0);

/// Something being dragged with the mouse.
enum Drag {
    /// A pointer in the toolbar row; `x` is the tile's left edge, `grab_dx` where it was held.
    Tile { id: String, grab_dx: f32, x: f32 },
    Slider,
    Scroll { grab_dy: f32 },
}

struct State {
    hwnd: HWND,
    canvas: Option<Canvas>,
    dpi: u32,
    theme: Theme,
    view: View,
    frame: Frame,
    /// Mouse position in DIPs, while it is over the window.
    mouse: Option<(f32, f32)>,
    hover: Option<Action>,
    pressed: Option<Action>,
    press_at: (f32, f32),
    drag: Option<Drag>,
    edits: Edits,
    icon: Option<RgbaImage>,
    tracking_leave: bool,
}

thread_local! {
    static STATE: RefCell<Option<State>> = const { RefCell::new(None) };
}

/// Runs `f` with the window state. `None` if there is no window or the state is in use.
fn with_state<R>(f: impl FnOnce(&mut State) -> R) -> Option<R> {
    STATE.with(|cell| {
        let mut guard = cell.try_borrow_mut().ok()?;
        guard.as_mut().map(f)
    })
}

/// Asks for a repaint of the settings window, if it is open.
fn invalidate() {
    if let Some(hwnd) = with_state(|s| s.hwnd) {
        unsafe {
            let _ = InvalidateRect(Some(hwnd), None, false);
        }
    }
}

/// Draws the whole window onto `canvas` (between `begin` and the caller's `end`).
fn draw_frame(canvas: &mut Canvas, theme: Theme, view: &View, ctx: &Ctx, hover: Option<Action>, pressed: Option<Action>) -> Frame {
    canvas.begin(theme.bg);
    let mut ui = Ui::new(canvas, theme, hover, pressed);
    view::draw(&mut ui, view, ctx);
    ui.set_clip(None);
    std::mem::take(&mut ui.frame)
}

/// Where the edit controls should go after a paint.
struct Placement {
    idea: (HWND, Option<canvas::Rect>),
    name: (HWND, Option<canvas::Rect>),
    scale: f32,
}

impl Placement {
    fn apply(self) {
        for (edit, rect) in [self.idea, self.name] {
            edits::place(edit, rect, self.scale);
            if rect.is_some() {
                // Repaint the control after the page, so the page can never end up on top.
                unsafe {
                    let _ = InvalidateRect(Some(edit), None, false);
                }
            }
        }
    }
}

impl State {
    fn scale(&self) -> f32 {
        self.dpi as f32 / 96.0
    }

    fn to_dips(&self, lparam: LPARAM) -> (f32, f32) {
        let x = (lparam.0 & 0xFFFF) as u16 as i16 as f32;
        let y = ((lparam.0 >> 16) & 0xFFFF) as u16 as i16 as f32;
        (x / self.scale(), y / self.scale())
    }

    fn client_size(&self) -> (u32, u32) {
        let mut rc = RECT::default();
        unsafe {
            let _ = GetClientRect(self.hwnd, &mut rc);
        }
        ((rc.right - rc.left).max(1) as u32, (rc.bottom - rc.top).max(1) as u32)
    }

    fn paint(&mut self) -> Option<Placement> {
        if self.canvas.is_none() {
            let (w, h) = self.client_size();
            let mut canvas = Canvas::for_window(self.hwnd, w, h, self.dpi as f32).ok()?;
            if let Some(icon) = &self.icon {
                canvas.add_bitmap(APP_ICON_KEY, icon);
            }
            self.canvas = Some(canvas);
        }
        let drag = match &self.drag {
            Some(Drag::Tile { id, x, .. }) => Some(TileDrag { id: id.clone(), x: *x }),
            _ => None,
        };
        let ctx = Ctx { prefs: prefs::get(), login: registry::launch_at_login(), drag, live_edits: true };
        let canvas = self.canvas.as_mut()?;
        let frame = draw_frame(canvas, self.theme, &self.view, &ctx, self.hover.clone(), self.pressed.clone());
        if let Err(e) = canvas.end() {
            if e.code().0 == RECREATE_TARGET {
                // The graphics device went away: start again with a new target and pictures.
                self.canvas = None;
                invalidate_hwnd(self.hwnd);
            }
        }
        self.frame = frame;

        let max = self.frame.max_scroll();
        if self.view.scroll > max {
            self.view.scroll = max;
            invalidate_hwnd(self.hwnd);
        }
        // Scrolling or a layout change can move a different control under a still mouse.
        if self.drag.is_none() {
            if let Some((x, y)) = self.mouse {
                let hover = self.frame.hit_test(x, y);
                if hover != self.hover {
                    self.hover = hover;
                    invalidate_hwnd(self.hwnd);
                }
            }
        }
        // Edit controls only show when their whole box is inside the scrolling area.
        let viewport = self.frame.viewport;
        let fit = |r: Option<canvas::Rect>| r.filter(|r| viewport.encloses(r));
        Some(Placement {
            idea: (self.edits.idea, fit(self.frame.idea_field)),
            name: (self.edits.name, fit(self.frame.name_field)),
            scale: self.scale(),
        })
    }

    /// Sets the pointer size from a mouse position over the slider.
    fn set_size_from(&mut self, x: f32) {
        let Some(track) = self.frame.slider_track else { return };
        let t = ((x - track.x) / track.w.max(1.0)).clamp(0.0, 1.0);
        let raw = prefs::POINTER_SIZE_MIN + t * (prefs::POINTER_SIZE_MAX - prefs::POINTER_SIZE_MIN);
        // Steps of 0.0025 keep the settings file from being rewritten on every pixel.
        let value = (raw / 0.0025).round() * 0.0025;
        if (value - prefs::get().pointer_size).abs() > 0.001 {
            prefs::update(|p| p.pointer_size = value);
        }
    }

    /// Scrolls so the scrollbar thumb's top is at `top`.
    fn scroll_thumb_to(&mut self, top: f32) {
        let (Some(track), Some(thumb)) = (self.frame.scroll_track, self.frame.scroll_thumb) else { return };
        let room = (track.h - thumb.h).max(1.0);
        self.view.scroll = ((top - track.y) / room).clamp(0.0, 1.0) * self.frame.max_scroll();
    }

    fn scroll_by(&mut self, dy: f32) {
        self.view.scroll = (self.view.scroll + dy).clamp(0.0, self.frame.max_scroll());
        invalidate_hwnd(self.hwnd);
    }

    /// Where a dragged pointer whose tile starts at `x` would land.
    fn drop_slot(&self, x: f32) -> usize {
        match self.frame.pinned_row {
            Some(row) => pointers::slot_at(x + row.slot_w / 2.0, row.left, row.step, row.count),
            None => 0,
        }
    }
}

fn invalidate_hwnd(hwnd: HWND) {
    unsafe {
        let _ = InvalidateRect(Some(hwnd), None, false);
    }
}

fn set_dark_title_bar(hwnd: HWND, dark: bool) {
    let value = BOOL::from(dark);
    unsafe {
        let _ = DwmSetWindowAttribute(
            hwnd,
            DWMWA_USE_IMMERSIVE_DARK_MODE,
            &value as *const BOOL as *const _,
            std::mem::size_of::<BOOL>() as u32,
        );
    }
}

/// Window size (outer) for the fixed client size at `dpi`.
fn window_size(dpi: u32) -> (i32, i32) {
    let scale = dpi as f32 / 96.0;
    let mut rc = RECT { left: 0, top: 0, right: (WIN_W * scale).round() as i32, bottom: (WIN_H * scale).round() as i32 };
    unsafe {
        let _ = AdjustWindowRectExForDpi(&mut rc, STYLE, false, WINDOW_EX_STYLE(0), dpi);
    }
    (rc.right - rc.left, rc.bottom - rc.top)
}

/// Opens the settings window, or brings it to the front if it is already open.
pub fn show() {
    unsafe {
        if let Some(hwnd) = with_state(|s| s.hwnd) {
            if IsWindow(Some(hwnd)).as_bool() {
                if IsIconic(hwnd).as_bool() {
                    let _ = ShowWindow(hwnd, SW_RESTORE);
                }
                let _ = SetForegroundWindow(hwnd);
                return;
            }
            // The window died without cleaning up; start afresh.
            STATE.with(|cell| {
                if let Ok(mut guard) = cell.try_borrow_mut() {
                    *guard = None;
                }
            });
        }
        snapshot::init_com();
        let instance: HINSTANCE = GetModuleHandleW(None).map(|m| m.into()).unwrap_or_default();
        let class = WNDCLASSEXW {
            cbSize: std::mem::size_of::<WNDCLASSEXW>() as u32,
            lpfnWndProc: Some(window_proc),
            hInstance: instance,
            lpszClassName: CLASS,
            hIcon: LoadIconW(Some(instance), PCWSTR(1 as *const u16)).unwrap_or_default(),
            hCursor: LoadCursorW(None, IDC_ARROW).unwrap_or_default(),
            ..Default::default()
        };
        // Fails harmlessly when the class is already registered from an earlier opening.
        RegisterClassExW(&class);

        // Centre on the monitor under the mouse, sized for that monitor's DPI.
        let mut cursor = POINT::default();
        let _ = GetCursorPos(&mut cursor);
        let monitor = MonitorFromPoint(cursor, MONITOR_DEFAULTTONEAREST);
        let mut info = MONITORINFO { cbSize: std::mem::size_of::<MONITORINFO>() as u32, ..Default::default() };
        let _ = GetMonitorInfoW(monitor, &mut info);
        let (mut dpi_x, mut dpi_y) = (96u32, 96u32);
        if GetDpiForMonitor(monitor, MDT_EFFECTIVE_DPI, &mut dpi_x, &mut dpi_y).is_err() {
            dpi_x = 96;
        }
        let (w, h) = window_size(dpi_x);
        let work = info.rcWork;
        let x = work.left + ((work.right - work.left) - w).max(0) / 2;
        let y = work.top + ((work.bottom - work.top) - h).max(0) / 2;
        let Ok(hwnd) = CreateWindowExW(
            WINDOW_EX_STYLE(0),
            CLASS,
            w!("TinyHand Settings"),
            STYLE,
            x,
            y,
            w,
            h,
            None,
            None,
            Some(instance),
            None,
        ) else {
            crate::log!("settings window could not be created");
            return;
        };

        // The window takes the DPI of wherever it actually landed.
        let dpi = match GetDpiForWindow(hwnd) {
            0 => dpi_x,
            d => d,
        };
        if dpi != dpi_x {
            let (w, h) = window_size(dpi);
            let _ = SetWindowPos(hwnd, None, 0, 0, w, h, SWP_NOMOVE | SWP_NOZORDER | SWP_NOACTIVATE);
        }
        let dark = !registry::apps_use_light_theme();
        set_dark_title_bar(hwnd, dark);
        DragAcceptFiles(hwnd, true);
        let theme = Theme::new(dark);
        let edits = Edits::create(hwnd, dpi, theme.field, theme.text);
        let state = State {
            hwnd,
            canvas: None,
            dpi,
            theme,
            view: View::new(Section::General),
            frame: Frame::default(),
            mouse: None,
            hover: None,
            pressed: None,
            press_at: (0.0, 0.0),
            drag: None,
            edits,
            icon: snapshot::app_icon(128),
            tracking_leave: false,
        };
        STATE.with(|cell| {
            if let Ok(mut guard) = cell.try_borrow_mut() {
                *guard = Some(state);
            }
        });
        let _ = ShowWindow(hwnd, SW_SHOW);
        let _ = SetForegroundWindow(hwnd);
    }
}

/// Settings changed somewhere else (toolbar, tray menu, reset): repaint if open.
pub fn prefs_changed() {
    with_state(|s| {
        if let Some(c) = s.canvas.as_mut() {
            // Pictures of hands that no longer exist.
            c.retain_bitmaps(|k| k.strip_prefix("thumb:").is_none_or(styles::is_valid));
        }
    });
    invalidate();
}

/// Called for every message before `TranslateMessage`. Handles the window's keyboard
/// shortcuts and the shortcut recorder, which must see keys before they become characters.
pub fn pre_translate(msg: &MSG) -> bool {
    let Some((hwnd, recording, idea, name)) = with_state(|s| (s.hwnd, s.view.recording.is_some(), s.edits.idea, s.edits.name))
    else {
        return false;
    };
    let ours = msg.hwnd == hwnd || unsafe { IsChild(hwnd, msg.hwnd) }.as_bool();
    if !ours {
        return false;
    }
    let key = msg.wParam.0 as u32;
    let ctrl = unsafe { GetKeyState(VK_CONTROL.0 as i32) } < 0;
    match msg.message {
        WM_KEYDOWN | WM_SYSKEYDOWN if recording => {
            actions::recorder_key_down(key);
            true
        }
        WM_KEYUP | WM_SYSKEYUP if recording => {
            actions::recorder_key_up();
            true
        }
        WM_CHAR | WM_SYSCHAR if recording => true,
        WM_KEYDOWN if key == VK_ESCAPE.0 as u32 || (ctrl && key == 'W' as u32) => {
            unsafe {
                let _ = PostMessageW(Some(hwnd), WM_CLOSE, WPARAM(0), LPARAM(0));
            }
            true
        }
        WM_KEYDOWN if key == VK_RETURN.0 as u32 && msg.hwnd == name => {
            actions::perform(hwnd, Action::AddHand);
            true
        }
        // Enter in the idea field would only beep.
        WM_KEYDOWN if key == VK_RETURN.0 as u32 && msg.hwnd == idea => true,
        WM_KEYDOWN if key == VK_TAB.0 as u32 && (msg.hwnd == idea || msg.hwnd == name) => {
            let other = if msg.hwnd == idea { name } else { idea };
            unsafe {
                if IsWindowVisible(other).as_bool() {
                    let _ = SetFocus(Some(other));
                }
            }
            true
        }
        _ => false,
    }
}

unsafe extern "system" fn window_proc(hwnd: HWND, msg: u32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    match msg {
        WM_PAINT => {
            let mut ps = PAINTSTRUCT::default();
            BeginPaint(hwnd, &mut ps);
            let placement = with_state(|s| s.paint()).flatten();
            let _ = EndPaint(hwnd, &ps);
            if let Some(p) = placement {
                p.apply();
            }
            LRESULT(0)
        }
        WM_ERASEBKGND => LRESULT(1),
        WM_SIZE => {
            with_state(|s| {
                let (w, h) = s.client_size();
                let dpi = s.dpi as f32;
                if let Some(c) = s.canvas.as_mut() {
                    if c.resize(w, h, dpi).is_err() {
                        s.canvas = None;
                    }
                }
            });
            invalidate_hwnd(hwnd);
            LRESULT(0)
        }
        WM_DPICHANGED => {
            let dpi = (wparam.0 & 0xFFFF) as u32;
            let dpi = dpi.max(48);
            let controls = with_state(|s| {
                s.dpi = dpi;
                s.canvas = None;
                (s.edits.idea, s.edits.name)
            });
            if let Some((idea, name)) = controls {
                // The edit controls get a new font, sent with the state released.
                let font = edits::make_font(dpi);
                edits::apply_font(idea, name, font);
                with_state(|s| s.edits.replace_font(font));
            }
            let rc = &*(lparam.0 as *const RECT);
            let _ = SetWindowPos(hwnd, None, rc.left, rc.top, rc.right - rc.left, rc.bottom - rc.top, SWP_NOZORDER | SWP_NOACTIVATE);
            invalidate_hwnd(hwnd);
            LRESULT(0)
        }
        WM_MOUSEMOVE => {
            input::mouse_move(hwnd, lparam);
            LRESULT(0)
        }
        WM_MOUSELEAVE => {
            with_state(|s| {
                s.tracking_leave = false;
                if s.pressed.is_none() {
                    s.mouse = None;
                    s.hover = None;
                }
            });
            invalidate_hwnd(hwnd);
            LRESULT(0)
        }
        WM_LBUTTONDOWN => {
            input::mouse_down(hwnd, lparam);
            LRESULT(0)
        }
        WM_LBUTTONUP => {
            input::mouse_up(hwnd, lparam);
            LRESULT(0)
        }
        WM_CAPTURECHANGED => {
            if HWND(lparam.0 as *mut _) != hwnd {
                with_state(|s| {
                    s.pressed = None;
                    s.drag = None;
                });
                invalidate_hwnd(hwnd);
            }
            LRESULT(0)
        }
        WM_MOUSEWHEEL => {
            let delta = ((wparam.0 >> 16) & 0xFFFF) as u16 as i16 as f32;
            with_state(|s| s.scroll_by(-delta / 120.0 * 64.0));
            LRESULT(0)
        }
        WM_SETCURSOR if HWND(wparam.0 as *mut _) == hwnd && (lparam.0 & 0xFFFF) as u32 == HTCLIENT => {
            let link = with_state(|s| s.hover.as_ref().is_some_and(Action::is_link)).unwrap_or(false);
            if let Ok(cursor) = LoadCursorW(None, if link { IDC_HAND } else { IDC_ARROW }) {
                SetCursor(Some(cursor));
            }
            LRESULT(1)
        }
        WM_KEYDOWN => {
            input::key_down(hwnd, wparam.0 as u32);
            LRESULT(0)
        }
        WM_COMMAND => {
            let id = wparam.0 & 0xFFFF;
            let code = ((wparam.0 >> 16) & 0xFFFF) as u32;
            const EN_SETFOCUS: u32 = 0x0100;
            const EN_KILLFOCUS: u32 = 0x0200;
            let field = match id {
                edits::IDEA_ID => Some(Field::Idea),
                edits::NAME_ID => Some(Field::Name),
                _ => None,
            };
            if let Some(field) = field {
                match code {
                    EN_SETFOCUS => {
                        with_state(|s| s.view.maker.focus = Some(field));
                    }
                    EN_KILLFOCUS => {
                        with_state(|s| {
                            if s.view.maker.focus == Some(field) {
                                s.view.maker.focus = None;
                            }
                        });
                    }
                    _ => {}
                }
                invalidate_hwnd(hwnd);
            }
            LRESULT(0)
        }
        WM_CTLCOLOREDIT => {
            let colors = with_state(|s| (s.edits.text_color, s.edits.back_color, s.edits.brush()));
            match colors {
                Some((text, back, brush)) => {
                    let hdc = HDC(wparam.0 as *mut _);
                    SetTextColor(hdc, COLORREF(text));
                    SetBkColor(hdc, COLORREF(back));
                    LRESULT(brush.0 as isize)
                }
                None => DefWindowProcW(hwnd, msg, wparam, lparam),
            }
        }
        WM_DROPFILES => {
            let drop = HDROP(wparam.0 as *mut _);
            let mut buffer = [0u16; 1024];
            let len = DragQueryFileW(drop, 0, Some(&mut buffer)) as usize;
            DragFinish(drop);
            if len > 0 {
                let path = std::path::PathBuf::from(String::from_utf16_lossy(&buffer[..len.min(buffer.len())]));
                let _ = SetForegroundWindow(hwnd);
                actions::load_draft(&path);
            }
            LRESULT(0)
        }
        WM_TIMER if wparam.0 == TIMER_COPIED => {
            let _ = KillTimer(Some(hwnd), TIMER_COPIED);
            with_state(|s| s.view.maker.copied = false);
            invalidate_hwnd(hwnd);
            LRESULT(0)
        }
        WM_SETTINGCHANGE | WM_THEMECHANGED => {
            theme_changed(hwnd);
            DefWindowProcW(hwnd, msg, wparam, lparam)
        }
        WM_ACTIVATE => {
            if (wparam.0 & 0xFFFF) as u32 == WA_INACTIVE {
                // Recording only makes sense while the window has the keyboard.
                if with_state(|s| s.view.recording.is_some()) == Some(true) {
                    actions::stop_recording();
                }
            }
            DefWindowProcW(hwnd, msg, wparam, lparam)
        }
        WM_DESTROY => {
            let _ = KillTimer(Some(hwnd), TIMER_COPIED);
            let taken = STATE.with(|cell| cell.try_borrow_mut().ok().and_then(|mut g| g.take()));
            if let Some(mut state) = taken {
                if state.view.recording.is_some() {
                    crate::app::with(|a| a.suspend_hotkeys(false));
                }
                state.edits.release();
            }
            LRESULT(0)
        }
        _ => DefWindowProcW(hwnd, msg, wparam, lparam),
    }
}

fn theme_changed(hwnd: HWND) {
    let dark = !registry::apps_use_light_theme();
    let changed = with_state(|s| {
        if s.theme.dark == dark {
            return None;
        }
        s.theme = Theme::new(dark);
        s.edits.set_colors(s.theme.field, s.theme.text);
        Some((s.edits.idea, s.edits.name))
    })
    .flatten();
    if let Some((idea, name)) = changed {
        set_dark_title_bar(hwnd, dark);
        unsafe {
            let _ = InvalidateRect(Some(idea), None, true);
            let _ = InvalidateRect(Some(name), None, true);
        }
        invalidate_hwnd(hwnd);
    }
}
