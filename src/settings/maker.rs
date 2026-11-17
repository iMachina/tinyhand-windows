//! "Make your own hand": a sub-page of Appearance that walks through describing a hand,
//! getting a picture made, and checking and adding it. Plus the Windows plumbing it needs:
//! the ChatGPT link, the clipboard and the file picker.

use super::canvas::{Align, Color, Font, Rect};
use super::ui::{Ui, BODY, CAPTION, GLYPH_CHECK, GLYPH_COPY, GLYPH_DOWNLOAD, GLYPH_OPEN, GLYPH_WARNING};
use super::view::{Action, Ctx, Draft, Field, View};
use crate::handmaker::Made;
use crate::{custom, handasset};
use image::{imageops, RgbaImage};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU32, Ordering};
use windows::core::{w, PCWSTR, PWSTR};
use windows::Win32::Foundation::{GlobalFree, HANDLE, HWND};
use windows::Win32::System::DataExchange::{CloseClipboard, EmptyClipboard, OpenClipboard, SetClipboardData};
use windows::Win32::System::Memory::{GlobalAlloc, GlobalLock, GlobalUnlock, GMEM_MOVEABLE};
use windows::Win32::UI::Controls::Dialogs::*;

pub const IDEA_PLACEHOLDER: &str = "e.g. knitted wool in mustard yellow";
pub const NAME_PLACEHOLDER: &str = "My hand";
const FIELD_H: f32 = 34.0;
/// Size of the hand preview in DIPs.
const PREVIEW_W: f32 = 128.0;
const PREVIEW_H: f32 = 160.0;

pub fn page(ui: &mut Ui, view: &View, ctx: &Ctx, x: f32, y: f32, w: f32) -> f32 {
    let m = &view.maker;
    let inner_x = x + 34.0;
    let inner_w = w - 34.0;

    let mut y = step(ui, x, y, w, 1, "Describe it", Some("What should your hand look like?"));
    let field = Rect::new(inner_x, y, inner_w.min(420.0), FIELD_H);
    ui.frame.idea_field = Some(text_field(ui, field, Field::Idea, m.focus == Some(Field::Idea), (!ctx.live_edits).then_some(&m.idea), IDEA_PLACEHOLDER));
    y += FIELD_H + 20.0;

    y = step(
        ui,
        x,
        y,
        w,
        2,
        "Make the image",
        Some("Open it in ChatGPT, or copy the prompt for another image tool. Download the result as a PNG with a transparent background."),
    );
    let open = "Open in ChatGPT";
    let ow = ui.button_width(open, true);
    ui.button(Rect::new(inner_x, y, ow, 32.0), open, Some(GLYPH_OPEN), true, Action::OpenChatGpt);
    let (copy, glyph) = if m.copied { ("Copied", GLYPH_CHECK) } else { ("Copy prompt", GLYPH_COPY) };
    let cw = ui.button_width("Copy prompt", true);
    ui.button(Rect::new(inner_x + ow + 8.0, y, cw, 32.0), copy, Some(glyph), false, Action::CopyPrompt);
    y += 32.0 + 20.0;

    y = step(ui, x, y, w, 3, "Add it", None);
    match &m.draft {
        Some(draft) => preview(ui, view, ctx, draft, inner_x, y, inner_w),
        None => drop_zone(ui, m.error.as_deref(), Rect::new(inner_x, y, inner_w, 130.0)),
    }
    y + if m.draft.is_some() { PREVIEW_H + 20.0 } else { 130.0 }
}

/// A numbered step heading. Returns the y where the step's content goes.
fn step(ui: &mut Ui, x: f32, y: f32, w: f32, number: u32, title: &str, detail: Option<&str>) -> f32 {
    let t = ui.t;
    ui.c.fill_circle(x + 11.0, y + 10.0, 11.0, t.accent);
    ui.c.text_in(&number.to_string(), Font::Semibold, CAPTION, Rect::new(x, y - 1.0, 22.0, 22.0), Align::Center, t.on_accent);
    ui.c.text(title, Font::Semibold, BODY, x + 34.0, y, t.text);
    let mut bottom = y + 20.0;
    if let Some(d) = detail {
        bottom += 2.0 + ui.c.text_wrapped(d, Font::Regular, CAPTION, x + 34.0, y + 22.0, w - 34.0, t.text2);
    }
    bottom + 10.0
}

/// The box a text field sits in. In the real window an edit control is placed over the
/// returned rectangle; for test pictures `text` (or the placeholder) is drawn here instead.
fn text_field(ui: &mut Ui, r: Rect, field: Field, focused: bool, text: Option<&String>, placeholder: &str) -> Rect {
    let t = ui.t;
    ui.c.fill_round(r, 4.0, t.field);
    ui.c.stroke_round(r, 4.0, 1.0, t.control_border);
    let line = Rect::new(r.x + 2.0, r.bottom() - if focused { 2.0 } else { 1.0 }, r.w - 4.0, if focused { 2.0 } else { 1.0 });
    ui.c.fill_rect(line, if focused { t.accent } else { t.strong_stroke });
    let inner = Rect::new(r.x + 11.0, r.cy() - 10.0, r.w - 22.0, 20.0);
    if let Some(text) = text {
        let (shown, color) = if text.is_empty() { (placeholder, t.text3) } else { (text.as_str(), t.text) };
        ui.c.text_in(shown, Font::Regular, BODY, inner, Align::Start, color);
    }
    ui.hit(r, Action::Focus(field));
    inner
}

fn drop_zone(ui: &mut Ui, error: Option<&str>, r: Rect) {
    let t = ui.t;
    let action = Action::ChooseFile;
    let fill = if ui.pressed(&action) {
        t.subtle_pressed
    } else if ui.hovered(&action) {
        t.subtle_hover
    } else {
        t.subtle_pressed.fade(0.6)
    };
    ui.c.fill_round(r, 8.0, fill);
    ui.c.stroke_round_dashed(r, 8.0, 1.2, if ui.hovered(&action) { t.accent } else { t.strong_stroke.fade(0.7) });
    let text_w = r.w - 48.0;
    match error {
        None => {
            ui.c.glyph(GLYPH_DOWNLOAD, 20.0, r.cx(), r.y + 46.0, t.text2);
            ui.c.text_wrapped_centered("Drop the PNG here, or click to choose it", Font::Regular, CAPTION, r.x + 24.0, r.y + 70.0, text_w, t.text2);
        }
        Some(message) => {
            let msg_h = ui.c.measure_wrapped(message, Font::Regular, CAPTION, text_w);
            let total = 22.0 + 8.0 + msg_h + 4.0 + 16.0;
            let top = r.cy() - total / 2.0;
            let warn = if t.dark { t.warning } else { Color::rgb(0xD47500) };
            ui.c.glyph(GLYPH_WARNING, 20.0, r.cx(), top + 11.0, warn);
            ui.c.text_wrapped_centered(message, Font::Regular, CAPTION, r.x + 24.0, top + 30.0, text_w, t.text);
            ui.c.text_wrapped_centered("Try again with another image", Font::Regular, CAPTION, r.x + 24.0, top + 34.0 + msg_h, text_w, t.text2);
        }
    }
    ui.hit(r, action);
}

fn preview(ui: &mut Ui, view: &View, ctx: &Ctx, draft: &Draft, x: f32, y: f32, w: f32) {
    let t = ui.t;
    let m = &view.maker;
    let frame = Rect::new(x, y, PREVIEW_W + 26.0, PREVIEW_H + 20.0);
    ui.c.fill_round(frame, 8.0, Color { r: 0.91, g: 0.81, b: 0.71, a: 1.0 });
    if !ui.c.has_bitmap(&draft.key) {
        ui.c.add_bitmap(&draft.key, &draft.preview);
    }
    ui.c.bitmap(&draft.key, Rect::new(x + 13.0, y + 10.0, PREVIEW_W, PREVIEW_H), 1.0);

    let cx = frame.right() + 22.0;
    let cw = (x + w - cx).max(120.0);
    ui.c.text("Name", Font::Semibold, CAPTION, cx, y + 2.0, t.text2);
    let field = Rect::new(cx, y + 22.0, cw.min(220.0), FIELD_H);
    ui.frame.name_field = Some(text_field(ui, field, Field::Name, m.focus == Some(Field::Name), (!ctx.live_edits).then_some(&m.name), NAME_PLACEHOLDER));
    let mut ny = field.bottom() + 12.0;
    ny += ui.c.text_wrapped(
        "It should show the back of the hand, like the ones in TinyHand. If you see the palm, try another image.",
        Font::Regular,
        CAPTION,
        cx,
        ny,
        cw,
        t.text2,
    );
    ny += 14.0;
    let add = "Add hand";
    let aw = ui.button_width(add, false);
    ui.button(Rect::new(cx, ny, aw, 32.0), add, None, true, Action::AddHand);
    let other = "Use another image";
    let bw = ui.button_width(other, false);
    ui.button(Rect::new(cx + aw + 8.0, ny, bw, 32.0), other, None, false, Action::ChooseFile);
    if let Some(error) = &m.save_error {
        ui.c.text_wrapped(error, Font::Regular, CAPTION, cx, ny + 40.0, cw, t.danger);
    }
}

/// The preview picture for a made hand: the hand cut from the 1024x1536 picture, at twice
/// the preview size so it stays sharp on high-DPI screens, premultiplied.
pub fn preview_image(made: &Made) -> RgbaImage {
    let image = &made.image;
    let (iw, ih) = (image.width(), image.height());
    // The same crop as the macOS app: the hand without most of the stick.
    let crop = if iw >= 912 && ih >= 1090 {
        imageops::crop_imm(image, 112, 90, 800, 1000).to_image()
    } else {
        image.clone()
    };
    let mut small = imageops::resize(&crop, (PREVIEW_W * 2.0) as u32, (PREVIEW_H * 2.0) as u32, imageops::FilterType::Triangle);
    handasset::premultiply(&mut small);
    small
}

/// Runs the hand maker on a picture. Returns the draft (or why it was turned down) and the
/// suggested name. Image work: takes a moment on big pictures.
pub fn draft_from(path: &Path) -> (Option<Draft>, Option<String>, String) {
    static NEXT: AtomicU32 = AtomicU32::new(1);
    match crate::handmaker::make(path) {
        Ok(made) => {
            let preview = preview_image(&made);
            let key = format!("draft:{}", NEXT.fetch_add(1, Ordering::Relaxed));
            (Some(Draft { made, key, preview }), None, custom::suggested_name(path))
        }
        Err(problem) => (None, Some(problem), String::new()),
    }
}

/// Percent-encodes everything but the unreserved URL characters.
pub fn percent_encode(text: &str) -> String {
    let mut out = String::with_capacity(text.len() * 3);
    for b in text.bytes() {
        match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'.' | b'_' | b'~' => out.push(b as char),
            _ => out.push_str(&format!("%{b:02X}")),
        }
    }
    out
}

/// The link that opens ChatGPT with the prompt for `idea` filled in.
pub fn chatgpt_url(idea: &str) -> String {
    format!("https://chatgpt.com/?q={}", percent_encode(&crate::handmaker::prompt_for(idea)))
}

/// Puts `text` on the clipboard. Returns false if the clipboard was busy.
pub fn copy_text(owner: HWND, text: &str) -> bool {
    let wide: Vec<u16> = text.encode_utf16().chain(std::iter::once(0)).collect();
    unsafe {
        if OpenClipboard(Some(owner)).is_err() {
            return false;
        }
        let mut ok = false;
        if EmptyClipboard().is_ok() {
            if let Ok(memory) = GlobalAlloc(GMEM_MOVEABLE, wide.len() * 2) {
                let dest = GlobalLock(memory) as *mut u16;
                if !dest.is_null() {
                    std::ptr::copy_nonoverlapping(wide.as_ptr(), dest, wide.len());
                    let _ = GlobalUnlock(memory);
                    const CF_UNICODETEXT: u32 = 13;
                    ok = SetClipboardData(CF_UNICODETEXT, Some(HANDLE(memory.0))).is_ok();
                }
                if !ok {
                    // The clipboard only takes ownership of the memory on success.
                    let _ = GlobalFree(Some(memory));
                }
            }
        }
        let _ = CloseClipboard();
        ok
    }
}

/// Asks for the hand picture. Runs a modal dialog, so call it with nothing borrowed.
pub fn choose_file(owner: HWND) -> Option<PathBuf> {
    let mut buffer = vec![0u16; 4096];
    let filter: Vec<u16> = "Pictures (*.png, *.webp)\0*.png;*.webp\0All files\0*.*\0\0".encode_utf16().collect();
    let mut ofn = OPENFILENAMEW {
        lStructSize: std::mem::size_of::<OPENFILENAMEW>() as u32,
        hwndOwner: owner,
        lpstrFilter: PCWSTR(filter.as_ptr()),
        lpstrFile: PWSTR(buffer.as_mut_ptr()),
        nMaxFile: buffer.len() as u32,
        lpstrTitle: w!("Pick the PNG of your hand"),
        Flags: OFN_EXPLORER | OFN_FILEMUSTEXIST | OFN_PATHMUSTEXIST | OFN_NOCHANGEDIR | OFN_HIDEREADONLY,
        ..Default::default()
    };
    let picked = unsafe { GetOpenFileNameW(&mut ofn).as_bool() };
    if !picked {
        return None;
    }
    let len = buffer.iter().position(|c| *c == 0).unwrap_or(buffer.len());
    (len > 0).then(|| PathBuf::from(String::from_utf16_lossy(&buffer[..len])))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn percent_encoding() {
        assert_eq!(percent_encode("abc-XYZ_0.9~"), "abc-XYZ_0.9~");
        assert_eq!(percent_encode("a b&c=d/e?"), "a%20b%26c%3Dd%2Fe%3F");
        assert_eq!(percent_encode("é\n"), "%C3%A9%0A");
        assert_eq!(percent_encode("+%#"), "%2B%25%23");
    }

    #[test]
    fn chatgpt_link_carries_the_idea() {
        let url = chatgpt_url("knitted wool");
        assert!(url.starts_with("https://chatgpt.com/?q=Make%20a%20pointer%20image"));
        assert!(url.contains("knitted%20wool"));
        assert!(!url[23..].contains(' ') && !url[23..].contains('&'));
    }
}
