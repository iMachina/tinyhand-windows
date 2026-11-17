//! "Make your own hand": checks a picture of a hand on a transparent background, puts it on
//! a stick so it looks like the built-in hands, and picks a matching colour for the menu.
//! Pure image code (a port of `HandMaker` in the macOS app), no Windows dependencies.

use crate::handasset;
use image::{imageops, RgbaImage};
use std::path::Path;

/// Text to paste into an image generator to get a picture that passes [`make`].
/// The bracketed part is replaced by [`prompt_for`].
pub const PROMPT: &str = "Make a pointer image for TinyHand, a Windows app that shows a toy hand on screen.\n\
\n\
Draw one cartoon right hand in this style: [describe your hand, e.g. knitted wool in mustard yellow]\n\
\n\
Rules:\n\
- We see the BACK of the hand. The palm faces away from us. The knuckles and the smooth back of the hand face the viewer.\n\
- The index finger points straight up. The other three fingers are folded down, so we only see their knuckles, never their fingertips or fingernails.\n\
- The raised index finger sits at the far left of the knuckle row, right next to the thumb. Three folded knuckles sit to its right. It must never look like a middle finger.\n\
- The thumb is tucked against the left side of the hand. Nothing crosses the front of the fist: no thumb, no fingertips, no fingernails, no palm.\n\
- The fingertip is the highest point of the image. Nothing above or beside it.\n\
- The hand ends at the bottom in a wrist cuff or a clean flat wrist, facing straight down, centred.\n\
- Upright, not tilted. Whole hand visible, nothing cropped.\n\
- Transparent background. No shadow, no floor, no outline glow, no text, no stick, no arm.\n\
- Soft 3D toy look, bright and readable at small sizes.\n\
- Portrait image, hand filling most of the height with a little empty space around it.";

const PLACEHOLDER: &str = "[describe your hand, e.g. knitted wool in mustard yellow]";

/// [`PROMPT`] with the style placeholder replaced by `idea`, or by a default style when
/// `idea` is blank.
pub fn prompt_for(idea: &str) -> String {
    let trimmed = idea.trim();
    let style = if trimmed.is_empty() { "a soft vinyl toy in a bright colour" } else { trimmed };
    PROMPT.replace(PLACEHOLDER, style)
}

/// A finished hand on a stick.
pub struct Made {
    /// 1024x1536 picture with straight (not premultiplied) alpha.
    pub image: RgbaImage,
    /// Colour for the hand in menus, as `0xRRGGBB`.
    pub ink: u32,
}

const UNREADABLE: &str = "That file isn't an image TinyHand can read.";
const NO_ALPHA: &str = "The image needs a transparent background. Save it as a PNG with transparency.";
const TOO_SMALL: &str = "The image is too small. Use one at least 512 pixels tall.";
const NOT_TRANSPARENT: &str = "The background isn't transparent, or the hand touches the edge of the image.";
const EMPTY: &str = "The image is empty.";
const SEVERAL: &str = "The image has more than one thing in it. Use a single hand.";
const NOT_UPRIGHT: &str = "The hand should stand upright, taller than it is wide.";
const SMALL_IN_FRAME: &str = "The hand is too small in the image. Crop it closer.";
const NOT_FINGER: &str = "The top of the image should be a single finger pointing up.";
const NO_WRIST: &str = "The hand should end in a flat wrist or cuff at the bottom.";
const SOFT: &str = "The hand has a soft shadow or glow around it. Ask for no shadow.";
const NO_STICK: &str = "TinyHand couldn't put that hand on a stick.";

/// Reads the picture at `path`, checks that it looks like a single upright hand on a
/// transparent background, and puts it on a stick. The error is a message for the user.
pub fn make(path: &Path) -> Result<Made, String> {
    let image = read(path)?;
    let pixels = Pixels::new(&image);
    check(&pixels)?;
    let composed = compose(&image, &pixels).ok_or_else(|| NO_STICK.to_string())?;
    Ok(Made { image: composed, ink: ink(&pixels) })
}

/// Straight-alpha RGBA pixels, with the image size as `usize` for indexing.
struct Pixels<'a> {
    width: usize,
    height: usize,
    rgba: &'a [u8],
}

impl<'a> Pixels<'a> {
    fn new(image: &'a RgbaImage) -> Self {
        Pixels { width: image.width() as usize, height: image.height() as usize, rgba: image.as_raw() }
    }

    fn alpha(&self, x: usize, y: usize) -> u8 {
        self.rgba[(y * self.width + x) * 4 + 3]
    }
}

/// Inclusive pixel bounds.
#[derive(Clone, Copy, Debug, PartialEq)]
struct Bounds {
    min_x: usize,
    min_y: usize,
    max_x: usize,
    max_y: usize,
}

impl Bounds {
    fn width(&self) -> usize {
        self.max_x - self.min_x + 1
    }

    fn height(&self) -> usize {
        self.max_y - self.min_y + 1
    }
}

fn read(path: &Path) -> Result<RgbaImage, String> {
    let unreadable = |_| UNREADABLE.to_string();
    let decoded = image::ImageReader::open(path)
        .map_err(unreadable)?
        .with_guessed_format()
        .map_err(unreadable)?
        .decode()
        .map_err(|_| UNREADABLE.to_string())?;
    if !decoded.color().has_alpha() {
        return Err(NO_ALPHA.to_string());
    }
    if decoded.width() < 64 || decoded.height() < 64 {
        return Err(TOO_SMALL.to_string());
    }
    Ok(decoded.to_rgba8())
}

fn check(p: &Pixels) -> Result<(), String> {
    let w = p.width;
    let h = p.height;
    let fail = |message: &str| Err(message.to_string());

    // Almost nothing solid along the border.
    let mut edge = 0usize;
    let mut edge_solid = 0usize;
    for x in 0..w {
        edge += 2;
        edge_solid += usize::from(p.alpha(x, 0) > 32) + usize::from(p.alpha(x, h - 1) > 32);
    }
    for y in 0..h {
        edge += 2;
        edge_solid += usize::from(p.alpha(0, y) > 32) + usize::from(p.alpha(w - 1, y) > 32);
    }
    if edge_solid as f64 / edge as f64 > 0.02 {
        return fail(NOT_TRANSPARENT);
    }

    // One connected shape, found on a coarse grid.
    let step = (w.min(h) / 256).max(1);
    let gw = w / step;
    let gh = h / step;
    let mut grid = vec![false; gw * gh];
    for gy in 0..gh {
        for gx in 0..gw {
            grid[gy * gw + gx] = p.alpha(gx * step, gy * step) > 128;
        }
    }
    let mut seen = vec![false; gw * gh];
    let mut sizes: Vec<usize> = Vec::new();
    for start in 0..grid.len() {
        if !grid[start] || seen[start] {
            continue;
        }
        let mut stack = vec![start];
        seen[start] = true;
        let mut n = 0;
        while let Some(i) = stack.pop() {
            n += 1;
            let x = i % gw;
            let y = i / gw;
            let mut visit = |j: usize| {
                if grid[j] && !seen[j] {
                    seen[j] = true;
                    stack.push(j);
                }
            };
            if x + 1 < gw {
                visit(i + 1);
            }
            if x > 0 {
                visit(i - 1);
            }
            if y + 1 < gh {
                visit(i + gw);
            }
            if y > 0 {
                visit(i - gw);
            }
        }
        sizes.push(n);
    }
    let Some(&biggest) = sizes.iter().max() else {
        return fail(EMPTY);
    };
    if (biggest as f64) / (sizes.iter().sum::<usize>() as f64) < 0.97 {
        return fail(SEVERAL);
    }

    // Upright and filling the frame.
    let Some(b) = bounds(p, 128) else {
        return fail(EMPTY);
    };
    let bw = b.width();
    let bh = b.height();
    let aspect = bh as f64 / bw as f64;
    if !(1.05..=2.8).contains(&aspect) {
        return fail(NOT_UPRIGHT);
    }
    if (bh as f64) / (h as f64) < 0.5 {
        return fail(SMALL_IN_FRAME);
    }

    // A single finger at the top.
    let top_band = (bh * 8 / 100).max(2);
    let mut finger_min = w;
    let mut finger_max = 0;
    for y in b.min_y..(b.min_y + top_band).min(h) {
        for x in b.min_x..=b.max_x {
            if p.alpha(x, y) > 128 {
                finger_min = finger_min.min(x);
                finger_max = finger_max.max(x);
            }
        }
    }
    if (finger_max as f64 - finger_min as f64 + 1.0) / bw as f64 > 0.4 {
        return fail(NOT_FINGER);
    }

    // A flat wrist at the bottom.
    let (base_width, _) = bottom_span(p, b);
    if (base_width as f64) / (bw as f64) < 0.2 {
        return fail(NO_WRIST);
    }

    // Hard edges: few half-transparent pixels.
    let mut soft = 0usize;
    let mut any = 0usize;
    for y in (0..h).step_by(2) {
        for x in (0..w).step_by(2) {
            let a = p.alpha(x, y);
            if a > 20 {
                any += 1;
                if a < 200 {
                    soft += 1;
                }
            }
        }
    }
    if soft as f64 / any.max(1) as f64 > 0.08 {
        return fail(SOFT);
    }
    Ok(())
}

/// Bounds of the pixels with alpha above `threshold`, or `None` if there are none.
fn bounds(p: &Pixels, threshold: u8) -> Option<Bounds> {
    let mut b = Bounds { min_x: p.width, min_y: p.height, max_x: 0, max_y: 0 };
    let mut any = false;
    for y in 0..p.height {
        for x in 0..p.width {
            if p.alpha(x, y) > threshold {
                any = true;
                b.min_x = b.min_x.min(x);
                b.max_x = b.max_x.max(x);
                b.min_y = b.min_y.min(y);
                b.max_y = b.max_y.max(y);
            }
        }
    }
    any.then_some(b)
}

/// Width and horizontal centre of the solid part of the bottom few rows of the hand.
fn bottom_span(p: &Pixels, b: Bounds) -> (usize, f64) {
    let bh = b.height();
    let mut lo = p.width;
    let mut hi = None;
    let first = b.max_y.saturating_sub((bh * 4 / 100).max(2)).max(b.min_y);
    for y in first..=b.max_y {
        for x in b.min_x..=b.max_x {
            if p.alpha(x, y) > 128 {
                lo = lo.min(x);
                hi = Some(hi.map_or(x, |h: usize| h.max(x)));
            }
        }
    }
    match hi {
        Some(hi) => (hi - lo + 1, (lo + hi) as f64 / 2.0),
        None => (0, (b.min_x + b.max_x) as f64 / 2.0),
    }
}

const CANVAS_W: u32 = 1024;
const CANVAS_H: u32 = 1536;
/// Where the fingertip lands, from the top of the canvas.
const HAND_TOP: f64 = 150.0;
const HAND_MAX_HEIGHT: f64 = 560.0;
const HAND_MAX_WIDTH: f64 = 700.0;
/// Where the rounded bottom of the stick ends, from the top of the canvas.
const ROD_BOTTOM: f64 = 1462.0;

/// Grey stops across the stick's width: position, then sRGB colour.
const ROD_GRADIENT: [(f64, [f64; 3]); 5] = [
    (0.0, [0.0, 0.0, 0.0]),
    (0.22, [18.0, 18.0, 20.0]),
    (0.45, [58.0, 58.0, 62.0]),
    (0.75, [18.0, 18.0, 20.0]),
    (1.0, [0.0, 0.0, 0.0]),
];

/// Draws the stick and the scaled hand on a 1024x1536 canvas. Coordinates are top-left based.
fn compose(image: &RgbaImage, p: &Pixels) -> Option<RgbaImage> {
    let b = bounds(p, 128)?;
    let bw = b.width() as f64;
    let bh = b.height() as f64;
    let scale = (HAND_MAX_HEIGHT / bh).min(HAND_MAX_WIDTH / bw);
    let (_, wrist_center) = bottom_span(p, b);

    // Only scale the part of the picture that has anything in it, plus a transparent margin
    // wide enough for the filter, so huge empty canvases stay cheap.
    let content = bounds(p, 0)?;
    let margin = (3.0 / scale.min(1.0)).ceil() as usize + 2;
    let x0 = content.min_x.saturating_sub(margin);
    let y0 = content.min_y.saturating_sub(margin);
    let x1 = (content.max_x + 1 + margin).min(p.width);
    let y1 = (content.max_y + 1 + margin).min(p.height);
    let (crop_w, crop_h) = (x1 - x0, y1 - y0);
    let mut part = imageops::crop_imm(image, x0 as u32, y0 as u32, crop_w as u32, crop_h as u32).to_image();
    handasset::premultiply(&mut part);
    let draw_w = ((crop_w as f64 * scale).round() as u32).max(1);
    let draw_h = ((crop_h as f64 * scale).round() as u32).max(1);
    let scaled = imageops::resize(&part, draw_w, draw_h, imageops::FilterType::Lanczos3);
    // The scale actually used after rounding the size to whole pixels.
    let sx = draw_w as f64 / crop_w as f64;
    let sy = draw_h as f64 / crop_h as f64;

    // The middle of the wrist sits on the canvas's centre line; the fingertip at HAND_TOP.
    let left = CANVAS_W as f64 / 2.0 - (wrist_center + 0.5 - x0 as f64) * sx;
    let top = HAND_TOP - (b.min_y - y0) as f64 * sy;
    let hand_bottom = HAND_TOP + bh * sy;

    let mut canvas = RgbaImage::new(CANVAS_W, CANVAS_H);
    let rod_w = (bw * scale * 0.18).clamp(54.0, 76.0);
    draw_rod(&mut canvas, hand_bottom - 30.0, ROD_BOTTOM, rod_w);
    draw_over(&mut canvas, &scaled, left.round() as i64, top.round() as i64);
    handasset::unpremultiply(&mut canvas);
    Some(canvas)
}

/// Fills a vertical capsule centred on the canvas, from `top` to `bottom`, with the grey
/// gradient. Writes premultiplied pixels with antialiased edges.
fn draw_rod(canvas: &mut RgbaImage, top: f64, bottom: f64, width: f64) {
    let radius = width / 2.0;
    let cx = canvas.width() as f64 / 2.0;
    let left = cx - radius;
    // The centre line between the two round ends.
    let mut line_top = top + radius;
    let mut line_bottom = bottom - radius;
    if line_bottom < line_top {
        line_top = (top + bottom) / 2.0;
        line_bottom = line_top;
    }
    let y_range = top.floor().max(0.0) as u32..(bottom.ceil() as u32).min(canvas.height());
    let x_range = left.floor().max(0.0) as u32..((left + width).ceil() as u32).min(canvas.width());
    for y in y_range {
        let py = y as f64 + 0.5;
        let dy = if py < line_top {
            line_top - py
        } else if py > line_bottom {
            py - line_bottom
        } else {
            0.0
        };
        for x in x_range.clone() {
            let px = x as f64 + 0.5;
            let dx = px - cx;
            let distance = (dx * dx + dy * dy).sqrt() - radius;
            let coverage = (0.5 - distance).clamp(0.0, 1.0);
            if coverage <= 0.0 {
                continue;
            }
            let colour = rod_colour(((px - left) / width).clamp(0.0, 1.0));
            let pixel = canvas.get_pixel_mut(x, y);
            for c in 0..3 {
                pixel.0[c] = (colour[c] * coverage).round() as u8;
            }
            pixel.0[3] = (255.0 * coverage).round() as u8;
        }
    }
}

/// The stick's gradient colour at `t` (0 = left edge, 1 = right edge).
fn rod_colour(t: f64) -> [f64; 3] {
    for pair in ROD_GRADIENT.windows(2) {
        let (t0, c0) = pair[0];
        let (t1, c1) = pair[1];
        if t <= t1 {
            let f = ((t - t0) / (t1 - t0)).clamp(0.0, 1.0);
            return [0, 1, 2].map(|i| c0[i] + (c1[i] - c0[i]) * f);
        }
    }
    ROD_GRADIENT[ROD_GRADIENT.len() - 1].1
}

/// Premultiplied "source over" of `src` onto `dst` with its top-left corner at (`left`, `top`).
fn draw_over(dst: &mut RgbaImage, src: &RgbaImage, left: i64, top: i64) {
    let (dw, dh) = (dst.width() as i64, dst.height() as i64);
    for (x, y, s) in src.enumerate_pixels() {
        let dx = left + x as i64;
        let dy = top + y as i64;
        if dx < 0 || dy < 0 || dx >= dw || dy >= dh {
            continue;
        }
        let sa = s.0[3] as u32;
        if sa == 0 {
            continue;
        }
        let d = dst.get_pixel_mut(dx as u32, dy as u32);
        let keep = 255 - sa;
        for c in 0..4 {
            // The filter can ring a colour above its alpha; keep it valid premultiplied.
            let sc = (s.0[c] as u32).min(sa);
            d.0[c] = (sc + (d.0[c] as u32 * keep + 127) / 255).min(255) as u8;
        }
    }
}

/// The hand's main colour, for menus: the most common saturated colour among the solid
/// pixels, toned down if very bright, or a neutral grey for a grey hand.
fn ink(p: &Pixels) -> u32 {
    #[derive(Clone, Copy, Default)]
    struct Bucket {
        n: usize,
        r: f64,
        g: f64,
        b: f64,
    }
    // 6 levels per channel. Pixels are straight alpha, so the colour is just the value / 255
    // (the macOS version divides premultiplied values by alpha to get the same thing).
    let mut buckets = [Bucket::default(); 216];
    for y in (0..p.height).step_by(3) {
        for x in (0..p.width).step_by(3) {
            let i = (y * p.width + x) * 4;
            if p.rgba[i + 3] <= 200 {
                continue;
            }
            let r = p.rgba[i] as f64 / 255.0;
            let g = p.rgba[i + 1] as f64 / 255.0;
            let b = p.rgba[i + 2] as f64 / 255.0;
            let key = (r * 5.0) as usize * 36 + (g * 5.0) as usize * 6 + (b * 5.0) as usize;
            let e = &mut buckets[key];
            e.n += 1;
            e.r += r;
            e.g += g;
            e.b += b;
        }
    }

    let mut best = (-1.0, 0.53, 0.33, 0.95);
    for e in buckets.iter().filter(|e| e.n > 0) {
        let n = e.n as f64;
        let (r, g, b) = (e.r / n, e.g / n, e.b / n);
        let hi = r.max(g).max(b);
        let lo = r.min(g).min(b);
        let sat = if hi == 0.0 { 0.0 } else { (hi - lo) / hi };
        let lum = 0.3 * r + 0.59 * g + 0.11 * b;
        let score = n * (0.25 + sat) * if lum > 0.12 && lum < 0.85 { 1.0 } else { 0.15 };
        if score > best.0 {
            best = (score, r, g, b);
        }
    }

    let (_, mut r, mut g, mut b) = best;
    let hi = r.max(g).max(b);
    let lo = r.min(g).min(b);
    if hi - lo < 0.08 {
        (r, g, b) = (0.42, 0.45, 0.5);
    } else if hi > 0.82 {
        r *= 0.82 / hi;
        g *= 0.82 / hi;
        b *= 0.82 / hi;
    }
    let channel = |v: f64| (v * 255.0).min(255.0) as u32;
    channel(r) << 16 | channel(g) << 8 | channel(b)
}

#[cfg(test)]
mod tests {
    use super::*;
    use image::{Rgba, RgbImage};
    use std::path::PathBuf;

    const LAVENDER: [u8; 4] = [180, 150, 230, 255];

    fn test_dir() -> PathBuf {
        let dir = std::env::temp_dir().join(format!("tinyhand-handmaker-tests-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn save(image: &RgbaImage, name: &str) -> PathBuf {
        let path = test_dir().join(format!("{name}.png"));
        image.save(&path).unwrap();
        path
    }

    /// Fills the half-open rectangle [x0, x1) x [y0, y1).
    fn fill(image: &mut RgbaImage, x0: u32, y0: u32, x1: u32, y1: u32, colour: [u8; 4]) {
        for y in y0..y1 {
            for x in x0..x1 {
                image.put_pixel(x, y, Rgba(colour));
            }
        }
    }

    /// A 400x600 picture with a blocky hand: a thin finger at the top left of a wider fist,
    /// flat wrist at the bottom.
    fn blocky_hand(colour: [u8; 4]) -> RgbaImage {
        let mut image = RgbaImage::new(400, 600);
        fill(&mut image, 110, 60, 160, 260, colour);
        fill(&mut image, 100, 240, 300, 500, colour);
        image
    }

    fn rejection(image: &RgbaImage, name: &str) -> String {
        match make(&save(image, name)) {
            Ok(_) => panic!("{name} should have been rejected"),
            Err(message) => message,
        }
    }

    #[test]
    fn prompt_fills_in_the_style() {
        assert!(PROMPT.contains("a Windows app"));
        assert!(!PROMPT.contains("Mac"));
        assert!(PROMPT.ends_with("a little empty space around it."));
        let p = prompt_for("  knitted wool in mustard yellow \n");
        assert!(p.contains("in this style: knitted wool in mustard yellow\n"));
        assert!(!p.contains('['));
        assert!(prompt_for(" ").contains("in this style: a soft vinyl toy in a bright colour\n"));
    }

    #[test]
    fn makes_a_hand_on_a_stick() {
        let made = make(&save(&blocky_hand(LAVENDER), "good")).unwrap();
        let image = &made.image;
        assert_eq!(image.dimensions(), (1024, 1536));

        let m = handasset::measure(image).expect("measurable");
        assert!(m.wrist < m.rod_end, "{m:?}");
        assert!((m.rod_x - 0.5).abs() < 0.01, "{m:?}");
        // Fingertip at 150, hand 560 tall (bounding box 441 tall scaled by 560/441).
        assert!((m.tip * 1536.0 - 150.0).abs() <= 1.5, "{m:?}");
        assert!((m.wrist * 1536.0 - 710.0).abs() <= 3.0, "{m:?}");
        assert!((m.rod_end * 1536.0 - 1461.0).abs() <= 1.5, "{m:?}");
        assert!(m.tip_x < 0.45, "finger should be left of the stick: {m:?}");

        // The hand keeps its colour (straight alpha), the stick is dark and opaque.
        let hand = image.get_pixel(512, 500).0;
        assert_eq!(hand, LAVENDER);
        let stick = image.get_pixel(512, 1200).0;
        assert_eq!(stick[3], 255);
        assert!(stick[0] < 70 && stick[1] < 70 && stick[2] < 70, "{stick:?}");
        // Edge of the stick is antialiased, beyond it is empty.
        let rod_w = (200.0f64 * 560.0 / 441.0 * 0.18).clamp(54.0, 76.0);
        let edge = (512.0 + rod_w / 2.0) as u32;
        assert!(image.get_pixel(edge + 2, 1200).0[3] == 0);
        assert!(image.get_pixel(edge - 2, 1200).0[3] == 255);
        assert_eq!(image.get_pixel(512, 1470).0[3], 0);
        assert_eq!(image.get_pixel(512, 140).0[3], 0);
    }

    #[test]
    fn rejects_opaque_background() {
        let mut image = RgbaImage::new(400, 600);
        fill(&mut image, 0, 0, 400, 600, [255, 255, 255, 255]);
        fill(&mut image, 110, 60, 160, 260, LAVENDER);
        assert_eq!(rejection(&image, "opaque"), NOT_TRANSPARENT);
    }

    #[test]
    fn rejects_hand_touching_the_edge() {
        let mut image = blocky_hand(LAVENDER);
        fill(&mut image, 100, 240, 300, 600, LAVENDER);
        assert_eq!(rejection(&image, "edge"), NOT_TRANSPARENT);
    }

    #[test]
    fn rejects_two_things() {
        let mut image = blocky_hand(LAVENDER);
        fill(&mut image, 320, 100, 380, 200, LAVENDER);
        assert_eq!(rejection(&image, "two"), SEVERAL);
    }

    #[test]
    fn rejects_empty_image() {
        assert_eq!(rejection(&RgbaImage::new(400, 600), "empty"), EMPTY);
    }

    #[test]
    fn rejects_wide_hand() {
        let mut image = RgbaImage::new(400, 600);
        fill(&mut image, 50, 200, 350, 400, LAVENDER);
        assert_eq!(rejection(&image, "wide"), NOT_UPRIGHT);
    }

    #[test]
    fn rejects_hand_small_in_frame() {
        let mut image = RgbaImage::new(400, 600);
        fill(&mut image, 150, 200, 200, 300, LAVENDER);
        assert_eq!(rejection(&image, "smallframe"), SMALL_IN_FRAME);
    }

    #[test]
    fn rejects_wide_top() {
        let mut image = RgbaImage::new(400, 600);
        fill(&mut image, 100, 60, 300, 500, LAVENDER);
        assert_eq!(rejection(&image, "block"), NOT_FINGER);
    }

    #[test]
    fn rejects_pointed_bottom() {
        let mut image = RgbaImage::new(400, 600);
        fill(&mut image, 110, 60, 160, 260, LAVENDER);
        fill(&mut image, 100, 240, 300, 470, LAVENDER);
        fill(&mut image, 190, 470, 210, 500, LAVENDER);
        assert_eq!(rejection(&image, "pointed"), NO_WRIST);
    }

    #[test]
    fn rejects_soft_glow() {
        let mut image = RgbaImage::new(400, 600);
        fill(&mut image, 80, 40, 320, 520, [255, 255, 255, 100]);
        fill(&mut image, 110, 60, 160, 260, LAVENDER);
        fill(&mut image, 100, 240, 300, 500, LAVENDER);
        assert_eq!(rejection(&image, "glow"), SOFT);
    }

    #[test]
    fn rejects_tiny_image() {
        let mut image = RgbaImage::new(40, 60);
        fill(&mut image, 15, 10, 25, 50, LAVENDER);
        assert_eq!(rejection(&image, "tiny"), TOO_SMALL);
    }

    #[test]
    fn rejects_image_without_alpha() {
        let path = test_dir().join("rgb.png");
        RgbImage::from_pixel(400, 600, image::Rgb([255, 255, 255])).save(&path).unwrap();
        assert_eq!(make(&path).err().as_deref(), Some(NO_ALPHA));
    }

    #[test]
    fn rejects_unreadable_files() {
        let path = test_dir().join("junk.png");
        std::fs::write(&path, b"this is not a picture").unwrap();
        assert_eq!(make(&path).err().as_deref(), Some(UNREADABLE));
        let missing = test_dir().join("missing.png");
        assert_eq!(make(&missing).err().as_deref(), Some(UNREADABLE));
    }

    fn ink_of(image: &RgbaImage) -> u32 {
        ink(&Pixels::new(image))
    }

    fn close(a: u32, b: u32, tolerance: i32) -> bool {
        (0..3).all(|i| ((a >> (i * 8) & 0xFF) as i32 - (b >> (i * 8) & 0xFF) as i32).abs() <= tolerance)
    }

    #[test]
    fn ink_picks_the_main_colour() {
        let red = ink_of(&blocky_hand([200, 60, 60, 255]));
        assert!(close(red, 0xC83C3C, 2), "{red:06x}");

        // Bright colours are toned down so the top channel is about 0.82.
        let bright = ink_of(&blocky_hand([255, 100, 100, 255]));
        assert!(close(bright, 0xD15252, 2), "{bright:06x}");

        // A blue hand with a white cuff and a dark grey thumb is still blue (toned down,
        // since 220 is above 0.82).
        let mut image = blocky_hand([40, 90, 220, 255]);
        fill(&mut image, 100, 440, 300, 500, [250, 250, 250, 255]);
        fill(&mut image, 100, 300, 140, 400, [60, 60, 60, 255]);
        // Half-transparent pixels are ignored.
        fill(&mut image, 170, 100, 200, 200, [255, 0, 0, 150]);
        let blue = ink_of(&image);
        assert!(close(blue, 0x2655D1, 2), "{blue:06x}");
    }

    #[test]
    fn ink_falls_back_to_grey() {
        assert_eq!(ink_of(&blocky_hand([128, 128, 128, 255])), 0x6B727F);
        assert_eq!(ink_of(&blocky_hand([20, 20, 22, 255])), 0x6B727F);
    }

    /// Cuts the hand off a built-in hand-on-a-stick and puts it back on a stick.
    #[test]
    fn remakes_a_built_in_hand() {
        let candidates = [
            Path::new(env!("CARGO_MANIFEST_DIR")).join("tinyhand-main/assets/hand_lavender.png"),
            // file!() is absolute when the module is pulled in with #[path].
            Path::new(file!()).with_file_name("../tinyhand-main/assets/hand_lavender.png"),
        ];
        let Some(source) = candidates.iter().find(|p| p.exists()) else {
            eprintln!("hand_lavender.png not found, skipping");
            return;
        };
        let original = image::open(source).unwrap().to_rgba8();
        let m = handasset::measure(&original).unwrap();
        let wrist = (m.wrist * original.height() as f32) as u32;
        // Keep a little transparent space below the wrist, but crop the rest away so the
        // hand fills enough of the picture.
        let mut hand = imageops::crop_imm(&original, 0, 0, original.width(), wrist + 40).to_image();
        let h = hand.height();
        fill(&mut hand, 0, wrist, original.width(), h, [0, 0, 0, 0]);

        let made = make(&save(&hand, "lavender_hand")).unwrap();
        let out = std::env::var_os("TH_TEST_OUT")
            .map(PathBuf::from)
            .unwrap_or_else(|| test_dir().join("lavender_made.png"));
        made.image.save(&out).unwrap();
        eprintln!("remade hand written to {}", out.display());

        let n = handasset::measure(&made.image).unwrap();
        assert!(n.wrist < n.rod_end);
        assert!((n.rod_x - 0.5).abs() < 0.01, "{n:?}");
        assert!((n.tip * 1536.0 - 150.0).abs() <= 2.0, "{n:?}");
        assert!(n.tip_x < 0.5, "{n:?}");
        let (r, g, b) = ((made.ink >> 16) & 0xFF, (made.ink >> 8) & 0xFF, made.ink & 0xFF);
        assert!(b > g && r > g, "lavender ink expected, got {:06x}", made.ink);
    }
}
