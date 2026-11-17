//! Developer checks run from the command line: `--render-test` draws a sample scene to a PNG
//! without touching the screen.

use crate::arm::Arm;
use crate::handasset;
use crate::ink::Ink;
use crate::prefs::Prefs;
use crate::render::Renderer;
use crate::styles;
use crate::toolbar::Toolbar;
use image::{Rgba, RgbaImage};
use std::time::{Duration, Instant};

/// Draws a hand (or the laser dot), some ink, the toolbar and its hint, and saves the
/// picture over a grey checkerboard so transparency is easy to judge.
pub fn render(out: &str, style: &str, width: u32, height: u32) -> Result<(), String> {
    let mut r = Renderer::offscreen(width, height).map_err(|e| format!("renderer: {e}"))?;
    let (w, h) = (width as f32, height as f32);
    let now = Instant::now();

    let mut ink = Ink::default();
    ink.begin((w * 0.12, h * 0.25), (h * 0.0055).max(3.0), styles::ink_color(style));
    for i in 1..=60 {
        let t = i as f32 / 60.0;
        ink.extend((w * (0.12 + 0.3 * t), h * (0.25 + 0.12 * (t * 9.0).sin() * (1.0 - t))));
    }
    ink.set_arrow(true);
    ink.end(0.0);
    ink.begin((w * 0.1, h * 0.6), (h * 0.0055).max(3.0), 0x30C25A);
    ink.extend((w * 0.3, h * 0.62));
    ink.end(1.0);
    // Half-way through its fade.
    ink.strokes.last_mut().unwrap().fade_at = Some(now - Duration::from_millis(400));
    ink.begin((w * 0.2, h * 0.75), (h * 0.0055).max(3.0), 0xFF8A00);

    let mut prefs = Prefs::default();
    prefs.pinned_styles.push("custom-missing".into());
    let mut toolbar = Toolbar::new();
    toolbar.sync(&prefs, style);
    toolbar.place(w / 2.0, h - 48.0, 1.0);
    toolbar.set_revealed(true);
    toolbar.show_hint(&[("Click", "to tap"), ("Drag", "to draw"), ("Hold Alt", "to click for real"), ("Esc", "to exit")]);
    toolbar.toggle_ink_tray();
    toolbar.animate(10.0);

    let cursor = (w * 0.55, h * 0.4);
    let mut arm = Arm::default();
    arm.step(1.0 / 120.0, cursor.0, w);

    r.begin();
    r.draw_ink(&mut ink, now);
    toolbar.draw(&mut r, now);
    match styles::load_hand(style) {
        Some(asset) => {
            let pose = arm.hand_pose(cursor, (w, h), &asset.metrics, 0.13, true);
            r.draw_hand(&asset, &pose);
            // A second, bigger hand reaching into the top-left corner.
            let mut other = Arm::default();
            other.step(1.0 / 120.0, w * 0.08, w);
            other.press();
            let pose = other.hand_pose((w * 0.08, h * 0.1), (w, h), &asset.metrics, 0.2, true);
            r.draw_hand(&asset, &pose);
        }
        None => r.draw_laser(&arm.laser_pose(cursor, h, 0.13)),
    }
    r.draw_laser(&arm.laser_pose((w * 0.85, h * 0.2), h, 0.13));
    r.end().map_err(|e| format!("drawing: {e}"))?;

    let mut frame = r.read_pixels().map_err(|e| format!("reading back: {e}"))?;
    handasset::unpremultiply(&mut frame);
    let mut canvas = RgbaImage::from_fn(width, height, |x, y| {
        if (x / 24 + y / 24) % 2 == 0 {
            Rgba([236, 236, 240, 255])
        } else {
            Rgba([210, 210, 216, 255])
        }
    });
    image::imageops::overlay(&mut canvas, &frame, 0, 0);
    // Mark the cursor position: the fingertip should sit exactly on it.
    for d in -6i32..=6 {
        for (x, y) in [(cursor.0 as i32 + d, cursor.1 as i32), (cursor.0 as i32, cursor.1 as i32 + d)] {
            if x >= 0 && y >= 0 && (x as u32) < width && (y as u32) < height {
                canvas.put_pixel(x as u32, y as u32, Rgba([0, 200, 255, 255]));
            }
        }
    }
    canvas.save(out).map_err(|e| format!("saving {out}: {e}"))
}
