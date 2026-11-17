//! The small toolbar at the bottom of the screen: pick a pointer, change the ink colour,
//! mute the tap sound, or exit. It is drawn on the overlay itself rather than being a window,
//! so it can never steal focus from the app being presented.
//!
//! Coordinates are overlay pixels. Sizes are multiplied by the monitor's DPI scale.

use crate::prefs::{InkChoice, Prefs};
use crate::render::{Color, Font, Renderer, GLYPH_CLOSE, GLYPH_MUTE, GLYPH_VOLUME};
use crate::styles;
use std::time::Instant;

#[derive(Clone, Debug, PartialEq)]
pub enum Action {
    Pick(String),
    ToggleInkTray,
    Ink(InkChoice),
    ToggleSound,
    Exit,
}

#[derive(Clone, Debug, PartialEq)]
enum Item {
    Style(String),
    Divider,
    Ink,
    Sound,
    Exit,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Rect {
    pub x: f32,
    pub y: f32,
    pub w: f32,
    pub h: f32,
}

impl Rect {
    pub fn contains(&self, p: (f32, f32)) -> bool {
        p.0 >= self.x && p.0 < self.x + self.w && p.1 >= self.y && p.1 < self.y + self.h
    }

    fn union(&self, o: &Rect) -> Rect {
        let x = self.x.min(o.x);
        let y = self.y.min(o.y);
        Rect { x, y, w: (self.x + self.w).max(o.x + o.w) - x, h: (self.y + self.h).max(o.y + o.h) - y }
    }

    fn grow(&self, dx: f32, dy: f32) -> Rect {
        Rect { x: self.x - dx, y: self.y - dy, w: self.w + 2.0 * dx, h: self.h + 2.0 * dy }
    }
}

const BUTTON: f32 = 40.0;
const SPACING: f32 = 6.0;
const PAD_X: f32 = 10.0;
const PAD_Y: f32 = 8.0;
const SWATCH: f32 = 30.0;
const PANEL: Color = Color(0.11, 0.11, 0.125, 0.9);

pub struct Toolbar {
    styles: Vec<String>,
    current: String,
    sound_on: bool,
    show_ink_button: bool,
    ink_choice: InkChoice,
    /// Whether the toolbar should be visible; `alpha` eases towards it.
    pub revealed: bool,
    alpha: f32,
    ink_open: bool,
    tray_alpha: f32,
    hint: Vec<(String, String)>,
    hint_on: bool,
    hint_alpha: f32,
    hover: Option<(Item, Instant)>,
    /// Bottom-centre anchor of the panel and the DPI scale.
    anchor: (f32, f32),
    scale: f32,
}

fn approach(value: &mut f32, target: f32, dt: f32, seconds: f32) -> bool {
    if *value == target {
        return false;
    }
    let step = dt / seconds;
    if *value < target {
        *value = (*value + step).min(target);
    } else {
        *value = (*value - step).max(target);
    }
    true
}

impl Toolbar {
    pub fn new() -> Toolbar {
        Toolbar {
            styles: styles::default_pinned(),
            current: "lavender".into(),
            sound_on: true,
            show_ink_button: true,
            ink_choice: InkChoice::Match,
            revealed: false,
            alpha: 0.0,
            ink_open: false,
            tray_alpha: 0.0,
            hint: Vec::new(),
            hint_on: false,
            hint_alpha: 0.0,
            hover: None,
            anchor: (0.0, 0.0),
            scale: 1.0,
        }
    }

    /// Brings the toolbar in line with the settings and the pointer in use.
    pub fn sync(&mut self, prefs: &Prefs, current: &str) {
        let mut shown = prefs.pinned();
        if !shown.iter().any(|s| s == current) {
            shown.push(current.to_string());
        }
        self.styles = shown;
        self.current = current.to_string();
        self.sound_on = prefs.sound_on(current);
        self.ink_choice = prefs.ink_choice;
        if self.show_ink_button != prefs.show_ink_button {
            self.show_ink_button = prefs.show_ink_button;
            self.ink_open = false;
        }
    }

    /// Positions the toolbar: centred, just above the bottom of the monitor's work area.
    pub fn place(&mut self, center_x: f32, work_bottom: f32, scale: f32) {
        self.scale = scale;
        self.anchor = (center_x, work_bottom - 14.0 * scale);
        self.ink_open = false;
    }

    fn items(&self) -> Vec<Item> {
        let mut items: Vec<Item> = self.styles.iter().cloned().map(Item::Style).collect();
        items.push(Item::Divider);
        if self.show_ink_button {
            items.push(Item::Ink);
        }
        items.push(Item::Sound);
        items.push(Item::Exit);
        items
    }

    fn layout(&self) -> (Rect, Vec<(Item, Rect)>) {
        let s = self.scale;
        let items = self.items();
        let widths: Vec<f32> = items.iter().map(|i| if *i == Item::Divider { 1.0 } else { BUTTON }).collect();
        let inner: f32 = widths.iter().sum::<f32>() + SPACING * (items.len() as f32 - 1.0);
        let w = ((inner + 2.0 * PAD_X) * s).round();
        let h = ((BUTTON + 2.0 * PAD_Y) * s).round();
        let panel = Rect { x: (self.anchor.0 - w / 2.0).round(), y: (self.anchor.1 - h).round(), w, h };
        let mut x = panel.x + PAD_X * s;
        let mut out = Vec::with_capacity(items.len());
        for (item, width) in items.into_iter().zip(widths) {
            let r = if item == Item::Divider {
                Rect { x, y: panel.y + (h - 24.0 * s) / 2.0, w: s.max(1.0), h: 24.0 * s }
            } else {
                Rect { x, y: panel.y + PAD_Y * s, w: BUTTON * s, h: BUTTON * s }
            };
            x += (width + SPACING) * s;
            out.push((item, r));
        }
        (panel, out)
    }

    fn tray_layout(&self) -> Option<(Rect, Vec<(InkChoice, Rect)>)> {
        let s = self.scale;
        let (panel, items) = self.layout();
        let ink = items.iter().find(|(i, _)| *i == Item::Ink)?.1;
        let n = InkChoice::ALL.len() as f32;
        let w = (n * SWATCH + (n - 1.0) * 2.0 + 12.0) * s;
        let h = (SWATCH + 10.0) * s;
        let tray = Rect { x: (ink.x + ink.w / 2.0 - w / 2.0).round(), y: (panel.y - 8.0 * s - h).round(), w, h };
        let mut x = tray.x + 6.0 * s;
        let mut out = Vec::new();
        for choice in InkChoice::ALL {
            out.push((choice, Rect { x, y: tray.y + 5.0 * s, w: SWATCH * s, h: SWATCH * s }));
            x += (SWATCH + 2.0) * s;
        }
        Some((tray, out))
    }

    /// The area that takes clicks: the panel, plus the ink tray while it is open.
    pub fn frame(&self) -> Rect {
        let (panel, _) = self.layout();
        match self.tray_layout() {
            Some((tray, _)) if self.ink_open => panel.union(&tray),
            _ => panel,
        }
    }

    /// True when the mouse is close enough that the toolbar should show itself.
    pub fn is_near(&self, mouse: (f32, f32)) -> bool {
        let s = self.scale;
        let f = self.frame();
        let above = Rect { y: f.y - 90.0 * s, ..f };
        f.grow(80.0 * s, 0.0).union(&above).grow(0.0, 10.0 * s).contains(mouse)
    }

    pub fn set_revealed(&mut self, revealed: bool) {
        self.revealed = revealed;
        if !revealed {
            self.ink_open = false;
            self.hover = None;
        }
    }

    /// Hides everything at once, with no fade.
    pub fn reset(&mut self) {
        self.revealed = false;
        self.alpha = 0.0;
        self.ink_open = false;
        self.tray_alpha = 0.0;
        self.hint_on = false;
        self.hint_alpha = 0.0;
        self.hover = None;
    }

    pub fn show_hint(&mut self, parts: &[(&str, &str)]) {
        self.hint = parts.iter().map(|(k, t)| (k.to_string(), t.to_string())).collect();
        self.hint_on = true;
    }

    pub fn hide_hint(&mut self) {
        self.hint_on = false;
    }

    pub fn toggle_ink_tray(&mut self) {
        self.ink_open = !self.ink_open && self.show_ink_button;
    }

    pub fn close_ink_tray(&mut self) {
        self.ink_open = false;
    }

    /// Advances the fades. Returns true while something is still changing.
    pub fn animate(&mut self, dt: f32) -> bool {
        let a = approach(&mut self.alpha, if self.revealed { 1.0 } else { 0.0 }, dt, if self.revealed { 0.15 } else { 0.35 });
        let b = approach(&mut self.tray_alpha, if self.ink_open && self.revealed { 1.0 } else { 0.0 }, dt, 0.12);
        let c = approach(&mut self.hint_alpha, if self.hint_on { 1.0 } else { 0.0 }, dt, if self.hint_on { 0.2 } else { 0.3 });
        a || b || c
    }

    /// True if anything of the toolbar is on screen.
    pub fn visible(&self) -> bool {
        self.alpha > 0.0 || self.hint_alpha > 0.0
    }

    /// What a click at `p` does, if it lands on a button.
    pub fn hit(&self, p: (f32, f32)) -> Option<Action> {
        if !self.revealed {
            return None;
        }
        if self.ink_open {
            if let Some((_, swatches)) = self.tray_layout() {
                if let Some((choice, _)) = swatches.iter().find(|(_, r)| r.contains(p)) {
                    return Some(Action::Ink(*choice));
                }
            }
        }
        let (_, items) = self.layout();
        let (item, _) = items.into_iter().find(|(_, r)| r.contains(p))?;
        match item {
            Item::Style(id) => Some(Action::Pick(id)),
            Item::Ink => Some(Action::ToggleInkTray),
            Item::Sound => Some(Action::ToggleSound),
            Item::Exit => Some(Action::Exit),
            Item::Divider => None,
        }
    }

    /// Tracks which button the mouse is over. Returns true if that changed, or a tooltip is due.
    pub fn hover(&mut self, mouse: (f32, f32), now: Instant) -> bool {
        let over = if self.revealed {
            self.layout().1.into_iter().find(|(i, r)| *i != Item::Divider && r.contains(mouse)).map(|(i, _)| i)
        } else {
            None
        };
        let same = match (&self.hover, &over) {
            (Some((a, _)), Some(b)) => a == b,
            (None, None) => true,
            _ => false,
        };
        if !same {
            self.hover = over.map(|i| (i, now));
            return true;
        }
        // Redraw around the moment the tooltip appears.
        matches!(&self.hover, Some((_, since)) if {
            let t = now.duration_since(*since).as_secs_f32();
            (TOOLTIP_DELAY..TOOLTIP_DELAY + 0.25).contains(&t)
        })
    }

    fn tooltip(&self, item: &Item) -> String {
        match item {
            Item::Style(id) => styles::title(id),
            Item::Ink => "Ink colour".into(),
            Item::Sound if self.sound_on => "Tap sound on \u{2014} click to mute".into(),
            Item::Sound => "Tap sound off \u{2014} click to unmute".into(),
            Item::Exit => "Exit  (Esc)".into(),
            Item::Divider => String::new(),
        }
    }

    pub fn draw(&mut self, r: &mut Renderer, now: Instant) {
        let s = self.scale;
        let (panel, items) = self.layout();

        if self.alpha > 0.0 {
            let a = self.alpha;
            glass(r, &panel, 16.0 * s, a);
            for (item, b) in &items {
                let hovered = matches!(&self.hover, Some((h, _)) if h == item);
                let selected = match item {
                    Item::Style(id) => *id == self.current,
                    Item::Ink => self.ink_open,
                    _ => false,
                };
                if selected {
                    r.fill_round_rect(b.x, b.y, b.w, b.h, 10.0 * s, Color::white(0.18 * a));
                } else if hovered {
                    r.fill_round_rect(b.x, b.y, b.w, b.h, 10.0 * s, Color::white(0.09 * a));
                }
                let (cx, cy) = (b.x + b.w / 2.0, b.y + b.h / 2.0);
                match item {
                    Item::Style(id) if id == styles::LASER => r.draw_laser_icon(cx, cy, b.w, a),
                    Item::Style(id) => r.draw_thumb(id, b.x, b.y, b.w, a),
                    Item::Divider => r.fill_rect(b.x, b.y, b.w, b.h, Color::white(0.18 * a)),
                    Item::Ink => ink_dot(r, cx, cy, b.w, Some(self.ink_choice.resolved(&self.current)), a),
                    Item::Sound => {
                        let (glyph, strength) = if self.sound_on { (GLYPH_VOLUME, 0.85) } else { (GLYPH_MUTE, 0.45) };
                        r.text_centered(glyph, Font::Icons, 17.0 * s, cx, cy, Color::white(strength * a));
                    }
                    Item::Exit => r.text_centered(GLYPH_CLOSE, Font::Icons, 14.0 * s, cx, cy, Color::white(0.85 * a)),
                }
            }
        }

        if self.tray_alpha > 0.0 {
            if let Some((tray, swatches)) = self.tray_layout() {
                let a = self.tray_alpha * self.alpha;
                glass(r, &tray, 14.0 * s, a);
                for (choice, b) in swatches {
                    if choice == self.ink_choice {
                        r.fill_ellipse(b.x + b.w / 2.0, b.y + b.h / 2.0, b.w / 2.0, b.h / 2.0, Color::white(0.2 * a));
                    }
                    ink_dot(r, b.x + b.w / 2.0, b.y + b.h / 2.0, b.w, choice.color(), a);
                }
            }
        }

        if self.hint_alpha > 0.0 && !self.hint.is_empty() {
            let a = self.hint_alpha;
            let size = 13.0 * s;
            let gap = 14.0 * s;
            let mut widths = Vec::new();
            let mut total = 0.0;
            let mut line_h: f32 = 0.0;
            for (key, text) in &self.hint {
                let (kw, kh) = r.measure_text(key, Font::TextBold, size);
                let (tw, _) = r.measure_text(&format!(" {text}"), Font::Text, size);
                widths.push((kw, tw));
                total += kw + tw;
                line_h = line_h.max(kh);
            }
            total += gap * (self.hint.len() as f32 - 1.0);
            let w = total + 32.0 * s;
            let h = line_h + 18.0 * s;
            let above = if self.ink_open { self.frame().y } else { panel.y };
            let pill = Rect { x: (self.anchor.0 - w / 2.0).round(), y: (above - 12.0 * s - h).round(), w, h };
            glass(r, &pill, 12.0 * s, a);
            let mut x = pill.x + 16.0 * s;
            let y = (pill.y + 9.0 * s).round();
            for ((key, text), (kw, tw)) in self.hint.iter().zip(widths) {
                r.text(key, Font::TextBold, size, x, y, Color::white(a));
                r.text(&format!(" {text}"), Font::Text, size, x + kw, y, Color::white(0.7 * a));
                x += kw + tw + gap;
            }
        }

        if let Some((item, since)) = &self.hover {
            if self.alpha >= 1.0 && now.duration_since(*since).as_secs_f32() >= TOOLTIP_DELAY {
                let label = self.tooltip(item);
                if let Some((_, b)) = items.iter().find(|(i, _)| i == item) {
                    let size = 12.0 * s;
                    let (tw, th) = r.measure_text(&label, Font::Text, size);
                    let w = tw + 16.0 * s;
                    let h = th + 8.0 * s;
                    let top = if self.ink_open { self.frame().y } else { panel.y };
                    let x = (b.x + b.w / 2.0 - w / 2.0).round();
                    let y = (top - 6.0 * s - h).round();
                    r.fill_round_rect(x, y, w, h, 6.0 * s, Color(0.07, 0.07, 0.08, 0.95));
                    r.text(&label, Font::Text, size, x + 8.0 * s, y + 4.0 * s, Color::white(0.92));
                }
            }
        }
    }
}

const TOOLTIP_DELAY: f32 = 0.6;

/// The dark, slightly see-through panel everything sits on.
fn glass(r: &mut Renderer, b: &Rect, radius: f32, alpha: f32) {
    r.fill_round_rect(b.x, b.y + 2.0, b.w, b.h, radius, Color::black(0.18 * alpha));
    r.fill_round_rect(b.x, b.y, b.w, b.h, radius, PANEL.fade(alpha));
    r.stroke_round_rect(b.x + 0.5, b.y + 0.5, b.w - 1.0, b.h - 1.0, radius, 1.0, Color::white(0.12 * alpha));
}

/// A glossy dot in the ink colour; a rainbow wheel when `color` is `None` ("match the pointer").
pub fn ink_dot(r: &mut Renderer, cx: f32, cy: f32, side: f32, color: Option<u32>, alpha: f32) {
    let k = side / 64.0;
    let radius = 13.0 * k;
    r.fill_ellipse(cx, cy + 1.0 * k, radius + 1.5 * k, radius + 1.5 * k, Color::black(0.35 * alpha));
    match color {
        Some(rgb) => {
            r.fill_ellipse(cx, cy, radius, radius, Color::hex(rgb, alpha));
            r.fill_ellipse(cx - radius * 0.2, cy - radius * 0.4, radius * 0.35, radius * 0.25, Color::white(0.35 * alpha));
        }
        None => {
            let wheel = [0xFF3B30, 0xFF9500, 0xFFCC00, 0x34C759, 0x007AFF, 0xAF52DE];
            for (i, rgb) in wheel.iter().enumerate() {
                r.fill_pie(cx, cy, radius, i as f32 * 60.0, (i + 1) as f32 * 60.0 + 0.5, Color::hex(*rgb, alpha));
            }
        }
    }
}
