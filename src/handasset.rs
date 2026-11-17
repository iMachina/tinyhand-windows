//! Pure image helpers for hand pictures: measuring where the hand, wrist and stick are,
//! and cutting a square thumbnail. No Windows dependencies, so the asset tool shares this file.

use image::{imageops, RgbaImage};

/// Where the parts of a hand-on-a-stick picture are. Everything is a fraction of the image
/// width (x values) or height (y values), with y growing downwards.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Metrics {
    /// Image width divided by height.
    pub aspect: f32,
    /// Top of the hand, with a hair of padding above the fingertip.
    pub hand_top: f32,
    /// The fingertip row.
    pub tip: f32,
    /// Where the hand ends and the stick begins.
    pub wrist: f32,
    /// Where the stretchable part of the stick ends.
    pub rod_end: f32,
    /// Horizontal centre of the fingertip.
    pub tip_x: f32,
    /// Horizontal centre of the stick.
    pub rod_x: f32,
    pub hand_min_x: f32,
    pub hand_max_x: f32,
}

/// Finds the hand and the stick from the alpha channel. Returns `None` for an empty picture
/// or one that doesn't look like a hand on a stick.
pub fn measure(image: &RgbaImage) -> Option<Metrics> {
    let w = image.width() as usize;
    let h = image.height() as usize;
    if w == 0 || h == 0 {
        return None;
    }
    let raw = image.as_raw();
    let solid = |x: usize, y: usize| raw[(y * w + x) * 4 + 3] > 128;

    let mut counts = vec![0usize; h];
    let mut spans = vec![(usize::MAX, 0usize); h];
    for y in 0..h {
        let mut n = 0;
        let mut lo = usize::MAX;
        let mut hi = 0;
        for x in 0..w {
            if solid(x, y) {
                n += 1;
                if lo == usize::MAX {
                    lo = x;
                }
                hi = x;
            }
        }
        counts[y] = n;
        spans[y] = (lo, hi);
    }
    let center = |y: usize| -> f32 {
        let (lo, hi) = spans[y];
        if lo == usize::MAX {
            0.5
        } else {
            (lo + hi) as f32 / 2.0 / w as f32
        }
    };

    let top = counts.iter().position(|&c| c > 0)?;
    let search_end = h.min(top + (0.45 * h as f64) as usize).max(top + 1);
    let max_w = *counts[top..search_end].iter().max()?;
    if max_w == 0 {
        return None;
    }
    let fist = (top..h).find(|&y| counts[y] as f64 > 0.6 * max_w as f64)?;
    let wrist = (fist..h).find(|&y| (counts[y] as f64) < 0.35 * max_w as f64)?;
    let rod_w = counts[(h - 1).min(wrist + h / 50)];
    let last_row = counts.iter().rposition(|&c| c > 0).unwrap_or(h - 1);
    let from = (h - 1).min(wrist + h / 33);
    let handle = (from..h).find(|&y| counts[y] as f64 > 1.8 * rod_w as f64);
    let rod_end = handle.unwrap_or(last_row);

    let mut min_x = w;
    let mut max_x = 0;
    for span in &spans[top..wrist] {
        if span.0 != usize::MAX {
            min_x = min_x.min(span.0);
            max_x = max_x.max(span.1);
        }
    }
    if min_x > max_x || rod_end <= wrist {
        return None;
    }

    let fh = h as f32;
    Some(Metrics {
        aspect: w as f32 / fh,
        hand_top: (top as f32 - 0.004 * fh).max(0.0) / fh,
        tip: top as f32 / fh,
        wrist: wrist as f32 / fh,
        rod_end: rod_end as f32 / fh,
        tip_x: center((h - 1).min(top + h / 200)),
        rod_x: center((wrist + rod_end) / 2),
        hand_min_x: min_x as f32 / w as f32,
        hand_max_x: max_x as f32 / w as f32,
    })
}

pub const THUMB_SIZE: u32 = 128;

/// A square picture of just the hand, `THUMB_SIZE` pixels on a side.
/// Pass premultiplied pixels to get clean edges; the result has the same alpha convention.
pub fn thumbnail(image: &RgbaImage, m: &Metrics) -> RgbaImage {
    let w = image.width() as f32;
    let h = image.height() as f32;
    let hand_w = (m.hand_max_x - m.hand_min_x) * w;
    let hand_h = (m.wrist - m.hand_top) * h;
    let side = (hand_w.max(hand_h) * 1.06).max(1.0);
    let cx = (m.hand_min_x + m.hand_max_x) / 2.0 * w;
    let cy = (m.hand_top + m.wrist) / 2.0 * h;

    let x0 = (cx - side / 2.0).round() as i64;
    let y0 = (cy - side / 2.0).round() as i64;
    let side_px = side.round().max(1.0) as u32;

    // Copy onto a transparent square so a hand near the edge isn't stretched.
    let mut square = RgbaImage::new(side_px, side_px);
    let sx0 = x0.max(0);
    let sy0 = y0.max(0);
    let sx1 = (x0 + side_px as i64).min(image.width() as i64);
    let sy1 = (y0 + side_px as i64).min(image.height() as i64);
    if sx1 > sx0 && sy1 > sy0 {
        let part = imageops::crop_imm(image, sx0 as u32, sy0 as u32, (sx1 - sx0) as u32, (sy1 - sy0) as u32).to_image();
        imageops::replace(&mut square, &part, sx0 - x0, sy0 - y0);
    }
    imageops::resize(&square, THUMB_SIZE, THUMB_SIZE, imageops::FilterType::Lanczos3)
}

/// Converts straight alpha to premultiplied alpha in place.
pub fn premultiply(image: &mut RgbaImage) {
    for p in image.pixels_mut() {
        let a = p.0[3] as u32;
        if a == 255 {
            continue;
        }
        p.0[0] = ((p.0[0] as u32 * a + 127) / 255) as u8;
        p.0[1] = ((p.0[1] as u32 * a + 127) / 255) as u8;
        p.0[2] = ((p.0[2] as u32 * a + 127) / 255) as u8;
    }
}

/// Converts premultiplied alpha back to straight alpha in place.
pub fn unpremultiply(image: &mut RgbaImage) {
    for p in image.pixels_mut() {
        let a = p.0[3] as u32;
        if a == 255 || a == 0 {
            continue;
        }
        p.0[0] = ((p.0[0] as u32 * 255 + a / 2) / a).min(255) as u8;
        p.0[1] = ((p.0[1] as u32 * 255 + a / 2) / a).min(255) as u8;
        p.0[2] = ((p.0[2] as u32 * 255 + a / 2) / a).min(255) as u8;
    }
}
