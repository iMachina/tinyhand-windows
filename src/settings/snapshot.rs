//! Offscreen drawing for `--settings-test`, and the WIC helpers it shares with the window
//! (the app icon is read from the executable's resources through WIC).

use super::canvas::Canvas;
use super::ui::Theme;
use super::view::{Ctx, Field, Recording, Section, TileDrag, View, WIN_H, WIN_W};
use super::{actions, draw_frame, maker, APP_ICON_KEY};
use crate::{handasset, prefs, registry};
use image::RgbaImage;
use std::path::Path;
use windows::core::PCWSTR;
use windows::Win32::Graphics::Imaging::*;
use windows::Win32::System::Com::{CoCreateInstance, CoInitializeEx, CLSCTX_INPROC_SERVER, COINIT_APARTMENTTHREADED};
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::UI::WindowsAndMessaging::{DestroyIcon, LoadImageW, HICON, IMAGE_ICON, LR_DEFAULTCOLOR};

/// Makes sure COM is ready on this thread (for WIC, the file picker and `ShellExecuteW`).
pub fn init_com() {
    unsafe {
        // Already initialised (either way) is fine.
        let _ = CoInitializeEx(None, COINIT_APARTMENTTHREADED);
    }
}

fn wic() -> Option<IWICImagingFactory> {
    unsafe { CoCreateInstance(&CLSID_WICImagingFactory, None, CLSCTX_INPROC_SERVER).ok() }
}

/// The app icon (resource 1) as premultiplied RGBA, `size` pixels square.
pub fn app_icon(size: i32) -> Option<RgbaImage> {
    unsafe {
        let instance = GetModuleHandleW(None).ok()?;
        let handle = LoadImageW(Some(instance.into()), PCWSTR(1 as *const u16), IMAGE_ICON, size, size, LR_DEFAULTCOLOR).ok()?;
        let icon = HICON(handle.0);
        let bitmap = wic().and_then(|f| f.CreateBitmapFromHICON(icon).ok());
        let _ = DestroyIcon(icon);
        let bitmap = bitmap?;
        let (mut w, mut h) = (0u32, 0u32);
        bitmap.GetSize(&mut w, &mut h).ok()?;
        let mut pixels = vec![0u8; (w * h * 4) as usize];
        bitmap.CopyPixels(std::ptr::null(), w * 4, &mut pixels).ok()?;
        // WIC hands icons over as straight-alpha BGRA.
        for p in pixels.chunks_exact_mut(4) {
            p.swap(0, 2);
        }
        let mut image = RgbaImage::from_raw(w, h, pixels)?;
        handasset::premultiply(&mut image);
        Some(image)
    }
}

/// Fills in the passing states (messages, a shortcut being recorded, a pointer being dragged)
/// so a test picture can show how they look.
fn demo(view: &mut View) {
    view.recording = Some(Recording { is_toggle: true, live: actions::live_text(prefs::MOD_CONTROL | prefs::MOD_ALT) });
    view.shortcut_error = Some("That shortcut is taken: Ctrl+Shift+Q is already in use. Pick a different one.".into());
    view.pin_message = Some("Keep at least one pointer in the toolbar.".into());
    view.hand_message = Some("Knit is in your toolbar.".into());
    view.login_failed = true;
    let m = &mut view.maker;
    m.copied = true;
    m.idea = "knitted wool in mustard yellow".into();
    m.focus = Some(Field::Idea);
    if m.draft.is_some() {
        m.save_error = Some("TinyHand couldn't save the hand. (Example message.)".into());
    }
}

/// Draws one section to a PNG without opening a window.
///
/// `section` is a section name ("general", "appearance", "drawing", "sound", "about") or
/// "handmaker", optionally followed by, in this order:
/// - `:<picture path>` (hand maker only) to show what the hand maker makes of that picture;
/// - `@<DIPs>` to scroll the content;
/// - `#demo` to show messages, a shortcut being recorded and a pointer being dragged;
/// - `!<x>,<y>` to hover the mouse there (prints what is under it, so hit-testing can be checked).
pub fn render_test(out: &str, section: &str, dark: bool) -> Result<(), String> {
    init_com();
    let (section, mouse) = match section.rsplit_once('!') {
        Some((s, point)) => {
            let parsed = point.split_once(',').and_then(|(x, y)| Some((x.trim().parse::<f32>().ok()?, y.trim().parse::<f32>().ok()?)));
            (s, Some(parsed.ok_or_else(|| format!("bad mouse position {point}"))?))
        }
        None => (section, None),
    };
    let (section, show_demo) = match section.strip_suffix("#demo") {
        Some(s) => (s, true),
        None => (section, false),
    };
    let (spec, scroll) = match section.rsplit_once('@') {
        Some((s, px)) => (s, px.parse::<f32>().map_err(|_| format!("bad scroll offset {px}"))?),
        None => (section, 0.0),
    };
    let (name, picture) = match spec.split_once(':') {
        Some((n, p)) => (n, Some(p)),
        None => (spec, None),
    };
    let mut view = if name.eq_ignore_ascii_case("handmaker") {
        let mut v = View::new(Section::Appearance);
        v.making_hand = true;
        v
    } else {
        View::new(Section::from_name(name).ok_or_else(|| format!("unknown section {name}"))?)
    };
    view.scroll = scroll;
    if let Some(path) = picture {
        let (draft, error, suggested) = maker::draft_from(Path::new(path));
        view.maker.draft = draft;
        view.maker.error = error;
        view.maker.name = suggested;
    }
    if show_demo {
        demo(&mut view);
    }

    let factory = wic().ok_or("WIC is not available")?;
    let (w, h) = (WIN_W as u32, WIN_H as u32);
    let bitmap = unsafe {
        factory
            .CreateBitmap(w, h, &GUID_WICPixelFormat32bppPBGRA, WICBitmapCacheOnLoad)
            .map_err(|e| format!("couldn't make a bitmap: {e}"))?
    };
    {
        let mut canvas = Canvas::for_wic(&bitmap).map_err(|e| format!("couldn't start Direct2D: {e}"))?;
        if let Some(icon) = app_icon(128) {
            canvas.add_bitmap(APP_ICON_KEY, &icon);
        }
        let mut ctx = Ctx { prefs: prefs::get(), login: registry::launch_at_login(), drag: None, live_edits: false };
        let theme = Theme::new(dark);
        // Draw once to learn the layout, then again with the scroll clamped, the mouse
        // hovering and the demo drag, exactly as the window would after those events.
        let frame = draw_frame(&mut canvas, theme, &view, &ctx, None, None);
        canvas.end().map_err(|e| format!("drawing failed: {e}"))?;
        view.scroll = view.scroll.clamp(0.0, frame.max_scroll());
        let frame = draw_frame(&mut canvas, theme, &view, &ctx, None, None);
        canvas.end().map_err(|e| format!("drawing failed: {e}"))?;
        if show_demo {
            if let (Some(row), Some(first)) = (frame.pinned_row, ctx.prefs.pinned().first()) {
                ctx.drag = Some(TileDrag { id: first.clone(), x: row.left + row.step * 2.4 });
            }
        }
        let hover = mouse.and_then(|(x, y)| frame.hit_test(x, y));
        if let Some((x, y)) = mouse {
            println!("under the mouse at {x},{y}: {hover:?}");
        }
        draw_frame(&mut canvas, theme, &view, &ctx, hover, None);
        canvas.end().map_err(|e| format!("drawing failed: {e}"))?;
    }

    let mut pixels = vec![0u8; (w * h * 4) as usize];
    unsafe { bitmap.CopyPixels(std::ptr::null(), w * 4, &mut pixels) }.map_err(|e| format!("couldn't read the picture: {e}"))?;
    for p in pixels.chunks_exact_mut(4) {
        p.swap(0, 2);
    }
    let mut image = RgbaImage::from_raw(w, h, pixels).ok_or("picture size mismatch")?;
    handasset::unpremultiply(&mut image);
    image.save(out).map_err(|e| format!("couldn't save {out}: {e}"))
}
