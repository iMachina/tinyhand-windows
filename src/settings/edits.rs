//! The hand maker's two text fields are real EDIT controls laid over boxes the canvas draws,
//! which gives typing, selection, the clipboard and IME input for free.
//!
//! Functions here send messages to the controls, and the controls answer by sending
//! `WM_COMMAND` / `WM_CTLCOLOREDIT` to the settings window, so call them with the window
//! state not borrowed.

use super::canvas::{Color, Rect};
use super::maker::{IDEA_PLACEHOLDER, NAME_PLACEHOLDER};
use windows::core::{w, HSTRING};
use windows::Win32::Foundation::{HWND, LPARAM, WPARAM};
use windows::Win32::Graphics::Gdi::*;
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::UI::WindowsAndMessaging::*;

pub const IDEA_ID: usize = 101;
pub const NAME_ID: usize = 102;

const EM_SETCUEBANNER: u32 = 0x1501;
const EM_LIMITTEXT: u32 = 0x00C5;
const ES_AUTOHSCROLL: u32 = 0x0080;

/// The two edit controls and the GDI objects they are drawn with.
pub struct Edits {
    pub idea: HWND,
    pub name: HWND,
    font: HFONT,
    brush: HBRUSH,
    pub text_color: u32,
    pub back_color: u32,
}

fn create_edit(parent: HWND, id: usize, limit: usize, placeholder: &str) -> HWND {
    unsafe {
        let instance = GetModuleHandleW(None).map(|m| m.into()).ok();
        let style = WS_CHILD | WS_TABSTOP | WINDOW_STYLE(ES_AUTOHSCROLL);
        let Ok(hwnd) = CreateWindowExW(
            WINDOW_EX_STYLE(0),
            w!("EDIT"),
            None,
            style,
            0,
            0,
            10,
            10,
            Some(parent),
            Some(HMENU(id as *mut _)),
            instance,
            None,
        ) else {
            return HWND::default();
        };
        SendMessageW(hwnd, EM_LIMITTEXT, Some(WPARAM(limit)), Some(LPARAM(0)));
        let cue = HSTRING::from(placeholder);
        SendMessageW(hwnd, EM_SETCUEBANNER, Some(WPARAM(1)), Some(LPARAM(cue.as_ptr() as isize)));
        hwnd
    }
}

/// Segoe UI at 14 DIPs for `dpi`. The caller owns the font.
pub fn make_font(dpi: u32) -> HFONT {
    let height = -((14.0 * dpi as f32 / 96.0).round() as i32);
    unsafe {
        CreateFontW(
            height,
            0,
            0,
            0,
            FW_NORMAL.0 as i32,
            0,
            0,
            0,
            DEFAULT_CHARSET,
            OUT_DEFAULT_PRECIS,
            CLIP_DEFAULT_PRECIS,
            CLEARTYPE_QUALITY,
            0,
            w!("Segoe UI"),
        )
    }
}

impl Edits {
    /// Creates both controls, hidden. `field` and `text` are the theme's field colours.
    pub fn create(parent: HWND, dpi: u32, field: Color, text: Color) -> Edits {
        let idea = create_edit(parent, IDEA_ID, 200, IDEA_PLACEHOLDER);
        let name = create_edit(parent, NAME_ID, 16, NAME_PLACEHOLDER);
        let mut edits =
            Edits { idea, name, font: HFONT::default(), brush: HBRUSH::default(), text_color: 0, back_color: 0 };
        edits.set_colors(field, text);
        let font = make_font(dpi);
        apply_font(idea, name, font);
        edits.replace_font(font);
        edits
    }

    /// Keeps `font` (already given to the controls with [`apply_font`]) and frees the old one.
    pub fn replace_font(&mut self, font: HFONT) {
        if !self.font.is_invalid() {
            unsafe {
                let _ = DeleteObject(self.font.into());
            }
        }
        self.font = font;
    }

    /// Colours for `WM_CTLCOLOREDIT`, after a theme change.
    pub fn set_colors(&mut self, field: Color, text: Color) {
        unsafe {
            if !self.brush.is_invalid() {
                let _ = DeleteObject(self.brush.into());
            }
            self.back_color = field.colorref();
            self.text_color = text.colorref();
            self.brush = CreateSolidBrush(windows::Win32::Foundation::COLORREF(self.back_color));
        }
    }

    pub fn brush(&self) -> HBRUSH {
        self.brush
    }

    /// Frees the font and brush. The controls go with their parent window.
    pub fn release(&mut self) {
        unsafe {
            if !self.font.is_invalid() {
                let _ = DeleteObject(self.font.into());
            }
            if !self.brush.is_invalid() {
                let _ = DeleteObject(self.brush.into());
            }
        }
        self.font = HFONT::default();
        self.brush = HBRUSH::default();
    }
}

/// Gives both controls a font. Sends `WM_SETFONT`, so don't hold the state borrow.
pub fn apply_font(idea: HWND, name: HWND, font: HFONT) {
    for hwnd in [idea, name] {
        if !hwnd.is_invalid() {
            unsafe {
                SendMessageW(hwnd, WM_SETFONT, Some(WPARAM(font.0 as usize)), Some(LPARAM(1)));
            }
        }
    }
}

/// Moves an edit control over `rect` (DIPs, scaled by `scale`), or hides it for `None`.
pub fn place(hwnd: HWND, rect: Option<Rect>, scale: f32) {
    if hwnd.is_invalid() {
        return;
    }
    unsafe {
        match rect {
            Some(r) => {
                let x = (r.x * scale).round() as i32;
                let y = (r.y * scale).round() as i32;
                let w = (r.w * scale).round() as i32;
                let h = (r.h * scale).round() as i32;
                let mut now = windows::Win32::Foundation::RECT::default();
                let _ = GetWindowRect(hwnd, &mut now);
                let visible = IsWindowVisible(hwnd).as_bool();
                let parent = GetParent(hwnd).unwrap_or_default();
                let mut origin = windows::Win32::Foundation::POINT { x: now.left, y: now.top };
                let _ = ScreenToClient(parent, &mut origin);
                let same = visible && origin.x == x && origin.y == y && now.right - now.left == w && now.bottom - now.top == h;
                if !same {
                    let _ = SetWindowPos(hwnd, None, x, y, w, h, SWP_NOZORDER | SWP_NOACTIVATE | SWP_SHOWWINDOW);
                }
            }
            None => {
                if IsWindowVisible(hwnd).as_bool() {
                    if windows::Win32::UI::Input::KeyboardAndMouse::GetFocus() == hwnd {
                        if let Ok(parent) = GetParent(hwnd) {
                            let _ = windows::Win32::UI::Input::KeyboardAndMouse::SetFocus(Some(parent));
                        }
                    }
                    let _ = ShowWindow(hwnd, SW_HIDE);
                }
            }
        }
    }
}

/// The text in an edit control.
pub fn text(hwnd: HWND) -> String {
    if hwnd.is_invalid() {
        return String::new();
    }
    let mut buffer = [0u16; 512];
    let len = unsafe { GetWindowTextW(hwnd, &mut buffer) }.max(0) as usize;
    String::from_utf16_lossy(&buffer[..len.min(buffer.len())])
}

pub fn set_text(hwnd: HWND, text: &str) {
    if !hwnd.is_invalid() {
        unsafe {
            let _ = SetWindowTextW(hwnd, &HSTRING::from(text));
        }
    }
}
