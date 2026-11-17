//! A small immediate-mode widget kit in the style of Windows 11 Settings.
//!
//! Each widget draws itself and records its clickable rectangle with [`Ui::hit`]. Hover and
//! pressed looks come from the action under the mouse in the previous frame, which the window
//! passes in when it starts a paint.

use super::canvas::{Align, Canvas, Color, Font, Rect};
use super::view::{Action, Frame, Hit};
use crate::styles;

pub const BODY: f32 = 14.0;
pub const CAPTION: f32 = 12.0;

pub const GLYPH_BACK: &str = "\u{E72B}";
pub const GLYPH_CHEVRON: &str = "\u{E76C}";
pub const GLYPH_OPEN: &str = "\u{E8A7}";
pub const GLYPH_DELETE: &str = "\u{E74D}";
pub const GLYPH_CLOSE: &str = "\u{E711}";
pub const GLYPH_COPY: &str = "\u{E8C8}";
pub const GLYPH_CHECK: &str = "\u{E73E}";
pub const GLYPH_DOWNLOAD: &str = "\u{E896}";
pub const GLYPH_WARNING: &str = "\u{E7BA}";
pub const GLYPH_SOUND: &str = "\u{E8D6}";
pub const GLYPH_POINTER: &str = "\u{E7C9}";

/// The colours of one theme, after Windows 11's light and dark palettes.
#[derive(Clone, Copy, Debug)]
pub struct Theme {
    pub dark: bool,
    pub bg: Color,
    pub nav_selected: Color,
    pub card: Color,
    pub card_hover: Color,
    pub card_pressed: Color,
    pub card_border: Color,
    pub text: Color,
    pub text2: Color,
    pub text3: Color,
    pub accent: Color,
    pub accent_hover: Color,
    pub accent_pressed: Color,
    pub on_accent: Color,
    pub control: Color,
    pub control_hover: Color,
    pub control_pressed: Color,
    pub control_border: Color,
    pub subtle_hover: Color,
    pub subtle_pressed: Color,
    pub strong_stroke: Color,
    pub field: Color,
    pub danger: Color,
    pub warning: Color,
}

impl Theme {
    pub fn new(dark: bool) -> Theme {
        if dark {
            Theme {
                dark,
                bg: Color::rgb(0x202020),
                nav_selected: Color::rgba(0xFFFFFF, 0.065),
                card: Color::rgb(0x2B2B2B),
                card_hover: Color::rgb(0x333333),
                card_pressed: Color::rgb(0x292929),
                card_border: Color::rgb(0x1C1C1C),
                text: Color::rgb(0xFFFFFF),
                text2: Color::rgb(0xCFCFCF),
                text3: Color::rgb(0x7A7A7A),
                accent: Color::rgb(0x60CDFF),
                accent_hover: Color::rgb(0x5AB9E8),
                accent_pressed: Color::rgb(0x52A5CF),
                on_accent: Color::rgb(0x000000),
                control: Color::rgb(0x373737),
                control_hover: Color::rgb(0x3C3C3C),
                control_pressed: Color::rgb(0x313131),
                control_border: Color::rgb(0x454545),
                subtle_hover: Color::rgba(0xFFFFFF, 0.06),
                subtle_pressed: Color::rgba(0xFFFFFF, 0.04),
                strong_stroke: Color::rgb(0x9E9E9E),
                field: Color::rgb(0x323232),
                danger: Color::rgb(0xFF99A4),
                warning: Color::rgb(0xFCE100),
            }
        } else {
            Theme {
                dark,
                bg: Color::rgb(0xF3F3F3),
                nav_selected: Color::rgba(0x000000, 0.055),
                card: Color::rgb(0xFBFBFB),
                card_hover: Color::rgb(0xF4F4F4),
                card_pressed: Color::rgb(0xF0F0F0),
                card_border: Color::rgb(0xE3E3E3),
                text: Color::rgb(0x1B1B1B),
                text2: Color::rgb(0x5D5D5D),
                text3: Color::rgb(0xA0A0A0),
                accent: Color::rgb(0x005FB8),
                accent_hover: Color::rgb(0x1A6FC0),
                accent_pressed: Color::rgb(0x3381C9),
                on_accent: Color::rgb(0xFFFFFF),
                control: Color::rgb(0xFEFEFE),
                control_hover: Color::rgb(0xF7F7F7),
                control_pressed: Color::rgb(0xF2F2F2),
                control_border: Color::rgb(0xD6D6D6),
                subtle_hover: Color::rgba(0x000000, 0.045),
                subtle_pressed: Color::rgba(0x000000, 0.03),
                strong_stroke: Color::rgb(0x8A8A8A),
                field: Color::rgb(0xFFFFFF),
                danger: Color::rgb(0xC42B1C),
                warning: Color::rgb(0x9D5D00),
            }
        }
    }
}

/// The control on the right of a [`Row`].
pub enum Control<'s> {
    None,
    Toggle { on: bool, action: Action },
    /// A row of options, one selected.
    Segmented { items: Vec<(&'s str, Action)>, selected: Option<usize> },
    Button { label: &'s str, icon: Option<&'static str>, accent: bool, action: Action },
    /// A shortcut recorder: the keys, or what it says while recording.
    Shortcut { text: &'s str, recording: Option<&'s str>, action: Action },
    /// A slider at `t` (0 to 1) with small and large pointer icons either side.
    Slider { t: f32, action: Action },
    /// A trailing chevron, for rows that open a page (set [`Row::click`]).
    Chevron,
    /// Trailing grey detail text and an "opens elsewhere" arrow (set [`Row::click`]).
    External(&'s str),
}

/// One setting card: title, optional grey subtitle, control on the right.
pub struct Row<'s> {
    pub title: &'s str,
    pub subtitle: Option<&'s str>,
    pub subtitle_color: Option<Color>,
    pub control: Control<'s>,
    pub enabled: bool,
    /// A pointer style pictured on the left.
    pub picture: Option<&'s str>,
    /// Makes the whole card one button.
    pub click: Option<Action>,
}

impl<'s> Row<'s> {
    pub fn new(title: &'s str) -> Row<'s> {
        Row { title, subtitle: None, subtitle_color: None, control: Control::None, enabled: true, picture: None, click: None }
    }

    pub fn sub(mut self, subtitle: &'s str) -> Self {
        self.subtitle = Some(subtitle);
        self
    }

    pub fn sub_color(mut self, color: Color) -> Self {
        self.subtitle_color = Some(color);
        self
    }

    pub fn control(mut self, control: Control<'s>) -> Self {
        self.control = control;
        self
    }

    pub fn enabled(mut self, enabled: bool) -> Self {
        self.enabled = enabled;
        self
    }

    pub fn picture(mut self, style: &'s str) -> Self {
        self.picture = Some(style);
        self
    }

    pub fn click(mut self, action: Action) -> Self {
        self.click = Some(action);
        self
    }
}

pub struct Ui<'a> {
    pub c: &'a mut Canvas,
    pub t: Theme,
    pub hover: Option<Action>,
    pub pressed: Option<Action>,
    /// What this paint has laid out so far.
    pub frame: Frame,
    clip: Option<Rect>,
}

const RADIUS: f32 = 6.0;
const ROW_PAD: f32 = 16.0;

impl<'a> Ui<'a> {
    pub fn new(c: &'a mut Canvas, t: Theme, hover: Option<Action>, pressed: Option<Action>) -> Ui<'a> {
        Ui { c, t, hover, pressed, frame: Frame::default(), clip: None }
    }

    /// Clips drawing and hit rectangles to `clip` (or stops clipping with `None`).
    pub fn set_clip(&mut self, clip: Option<Rect>) {
        if self.clip.is_some() {
            self.c.pop_clip();
        }
        if let Some(r) = clip {
            self.c.push_clip(r);
        }
        self.clip = clip;
    }

    /// Records a clickable rectangle (clipped to the scrolling area if one is set).
    pub fn hit(&mut self, r: Rect, action: Action) {
        let rect = match self.clip {
            Some(clip) => match r.intersect(&clip) {
                Some(r) => r,
                None => return,
            },
            None => r,
        };
        self.frame.hits.push(Hit { rect, action });
    }

    pub fn hovered(&self, action: &Action) -> bool {
        self.hover.as_ref() == Some(action)
    }

    /// Pressed and still under the mouse.
    pub fn pressed(&self, action: &Action) -> bool {
        self.pressed.as_ref() == Some(action) && self.hovered(action)
    }

    // ---- Layout pieces -----------------------------------------------------------------

    /// A group heading. Returns the y where the group's first card goes.
    pub fn header(&mut self, x: f32, y: f32, text: &str) -> f32 {
        self.c.text(text, Font::Semibold, BODY, x + 2.0, y, self.t.text);
        y + 28.0
    }

    pub fn card(&mut self, r: Rect, fill: Color) {
        self.c.fill_round(r, RADIUS, fill);
        self.c.stroke_round(r, RADIUS, 1.0, self.t.card_border);
    }

    /// A line of small wrapped text. Returns the height it took, plus a little space after.
    pub fn note(&mut self, x: f32, y: f32, w: f32, text: &str, color: Color) -> f32 {
        self.c.text_wrapped(text, Font::Regular, CAPTION, x + 2.0, y, w - 4.0, color) + 8.0
    }

    /// The fill for a card that may be a button.
    fn card_fill(&self, action: Option<&Action>) -> Color {
        match action {
            Some(a) if self.pressed(a) => self.t.card_pressed,
            Some(a) if self.hovered(a) => self.t.card_hover,
            _ => self.t.card,
        }
    }

    /// Draws a setting card. Returns its height.
    pub fn row(&mut self, x: f32, y: f32, w: f32, row: Row) -> f32 {
        let (cw, ch) = self.control_size(&row.control);
        let text_x = x + ROW_PAD + if row.picture.is_some() { 48.0 } else { 0.0 };
        let text_right = x + w - ROW_PAD - if cw > 0.0 { cw + 20.0 } else { 0.0 };
        let text_w = (text_right - text_x).max(40.0);
        let title_h = self.c.measure(row.title, Font::Regular, BODY).1;
        let sub_h = row.subtitle.map(|s| self.c.measure_wrapped(s, Font::Regular, CAPTION, text_w)).unwrap_or(0.0);
        let content_h = title_h + if sub_h > 0.0 { 1.0 + sub_h } else { 0.0 };
        let h = (content_h + 26.0).max(if row.picture.is_some() { 64.0 } else { 58.0 }).round();

        let r = Rect::new(x, y, w, h);
        let click = if row.enabled { row.click.clone() } else { None };
        let fill = self.card_fill(click.as_ref());
        self.card(r, fill);
        if let Some(a) = click {
            self.hit(r, a);
        }

        let t = self.t;
        let (title_color, sub_color) = if row.enabled {
            (t.text, row.subtitle_color.unwrap_or(t.text2))
        } else {
            (t.text3, t.text3)
        };
        if let Some(style) = row.picture {
            self.pointer_picture(style, Rect::new(x + ROW_PAD, r.cy() - 18.0, 36.0, 36.0), if row.enabled { 1.0 } else { 0.4 });
        }
        let ty = (y + (h - content_h) / 2.0).round();
        self.c.text(row.title, Font::Regular, BODY, text_x, ty, title_color);
        if let Some(s) = row.subtitle {
            self.c.text_wrapped(s, Font::Regular, CAPTION, text_x, ty + title_h + 1.0, text_w, sub_color);
        }
        if cw > 0.0 {
            let cr = Rect::new(x + w - ROW_PAD - cw, (r.cy() - ch / 2.0).round(), cw, ch);
            self.draw_control(cr, row.control, row.enabled);
        }
        h
    }

    /// A card with a title and subtitle on top and room for custom content below.
    /// Returns the card's height and the rectangle for the content.
    pub fn stacked_row(&mut self, x: f32, y: f32, w: f32, title: &str, subtitle: Option<&str>, body_h: f32) -> (f32, Rect) {
        let text_w = w - 2.0 * ROW_PAD;
        let title_h = self.c.measure(title, Font::Regular, BODY).1;
        let sub_h = subtitle.map(|s| self.c.measure_wrapped(s, Font::Regular, CAPTION, text_w)).unwrap_or(0.0);
        let head_h = title_h + if sub_h > 0.0 { 1.0 + sub_h } else { 0.0 };
        let h = (14.0 + head_h + 12.0 + body_h + 14.0).round();
        let card = self.t.card;
        self.card(Rect::new(x, y, w, h), card);
        let t = self.t;
        self.c.text(title, Font::Regular, BODY, x + ROW_PAD, y + 14.0, t.text);
        if let Some(s) = subtitle {
            self.c.text_wrapped(s, Font::Regular, CAPTION, x + ROW_PAD, y + 14.0 + title_h + 1.0, text_w, t.text2);
        }
        (h, Rect::new(x + ROW_PAD, y + 14.0 + head_h + 12.0, text_w, body_h))
    }

    fn control_size(&mut self, control: &Control) -> (f32, f32) {
        match control {
            Control::None => (0.0, 0.0),
            Control::Toggle { .. } => (self.toggle_width(), 32.0),
            Control::Segmented { items, .. } => (self.segmented_width(items), 32.0),
            Control::Button { label, icon, .. } => (self.button_width(label, icon.is_some()), 32.0),
            Control::Shortcut { text, recording, .. } => (self.shortcut_width(text, *recording), 32.0),
            Control::Slider { .. } => (236.0, 32.0),
            Control::Chevron => (16.0, 16.0),
            Control::External(detail) => (self.c.measure(detail, Font::Regular, CAPTION).0 + 24.0, 20.0),
        }
    }

    fn draw_control(&mut self, r: Rect, control: Control, enabled: bool) {
        let t = self.t;
        match control {
            Control::None => {}
            Control::Toggle { on, action } => self.toggle(r, on, enabled, action),
            Control::Segmented { items, selected } => self.segmented(r.x, r.y, &items, selected),
            Control::Button { label, icon, accent, action } => self.button(r, label, icon, accent, action),
            Control::Shortcut { text, recording, action } => self.shortcut(r, text, recording, action),
            Control::Slider { t: value, action } => {
                self.c.glyph(GLYPH_POINTER, 11.0, r.x + 8.0, r.cy(), t.text2);
                self.c.glyph(GLYPH_POINTER, 17.0, r.right() - 10.0, r.cy(), t.text2);
                self.slider(Rect::new(r.x + 22.0, r.y, r.w - 48.0, r.h), value, action);
            }
            Control::Chevron => self.c.glyph(GLYPH_CHEVRON, 12.0, r.cx(), r.cy(), t.text2),
            Control::External(detail) => {
                self.c.text_in(detail, Font::Regular, CAPTION, Rect::new(r.x, r.y, r.w - 22.0, r.h), Align::End, t.text2);
                self.c.glyph(GLYPH_OPEN, 12.0, r.right() - 7.0, r.cy(), t.text2);
            }
        }
    }

    // ---- Widgets -----------------------------------------------------------------------

    pub fn toggle_width(&mut self) -> f32 {
        let on = self.c.measure("On", Font::Regular, BODY).0;
        let off = self.c.measure("Off", Font::Regular, BODY).0;
        on.max(off) + 12.0 + 40.0
    }

    /// A Windows 11 toggle switch with "On"/"Off" to its left, right-aligned in `r`.
    pub fn toggle(&mut self, r: Rect, on: bool, enabled: bool, action: Action) {
        let t = self.t;
        let hover = enabled && self.hovered(&action);
        let pressed = enabled && self.pressed(&action);
        let dim = if enabled { 1.0 } else { 0.45 };
        let track = Rect::new(r.right() - 40.0, r.cy() - 10.0, 40.0, 20.0);
        self.c.text_in(
            if on { "On" } else { "Off" },
            Font::Regular,
            BODY,
            Rect::new(r.x, r.y, r.w - 52.0, r.h),
            Align::End,
            if enabled { t.text } else { t.text3 },
        );
        // The knob grows a little under the mouse, as in Windows 11.
        let knob_r = if hover || pressed { 7.0 } else { 6.0 };
        if on {
            let fill = if pressed { t.accent_pressed } else if hover { t.accent_hover } else { t.accent };
            self.c.fill_round(track, 10.0, fill.fade(dim));
            self.c.fill_circle(track.right() - 10.0, track.cy(), knob_r, t.on_accent.fade(dim));
        } else {
            if hover {
                self.c.fill_round(track, 10.0, t.subtle_hover);
            }
            self.c.stroke_round(track, 10.0, 1.0, t.strong_stroke.fade(dim));
            self.c.fill_circle(track.x + 10.0, track.cy(), knob_r - 1.0, t.text2.fade(dim));
        }
        if enabled {
            self.hit(r.inset(-4.0, -4.0), action);
        }
    }

    pub fn segmented_width(&mut self, items: &[(&str, Action)]) -> f32 {
        items.iter().map(|(label, _)| self.c.measure(label, Font::Regular, BODY).0 + 26.0).sum::<f32>() + 4.0
    }

    /// A row of options with the selected one filled in the accent colour.
    pub fn segmented(&mut self, x: f32, y: f32, items: &[(&str, Action)], selected: Option<usize>) {
        let t = self.t;
        let w = self.segmented_width(items);
        let outer = Rect::new(x, y, w, 32.0);
        self.c.fill_round(outer, 5.0, t.control);
        self.c.stroke_round(outer, 5.0, 1.0, t.control_border);
        let mut sx = x + 2.0;
        for (i, (label, action)) in items.iter().enumerate() {
            let sw = self.c.measure(label, Font::Regular, BODY).0 + 26.0;
            let seg = Rect::new(sx, y + 2.0, sw, 28.0);
            let color = if selected == Some(i) {
                let fill = if self.pressed(action) { t.accent_pressed } else if self.hovered(action) { t.accent_hover } else { t.accent };
                self.c.fill_round(seg, 4.0, fill);
                t.on_accent
            } else {
                if self.pressed(action) {
                    self.c.fill_round(seg, 4.0, t.subtle_pressed);
                } else if self.hovered(action) {
                    self.c.fill_round(seg, 4.0, t.subtle_hover);
                }
                t.text
            };
            self.c.text_in(label, Font::Regular, BODY, seg, Align::Center, color);
            self.hit(seg, action.clone());
            sx += sw;
        }
    }

    /// A horizontal slider. The thumb's centre runs from `r.x + 10` to `r.right() - 10`.
    pub fn slider(&mut self, r: Rect, value: f32, action: Action) {
        let t = self.t;
        let track = Rect::new(r.x + 10.0, r.cy() - 2.0, r.w - 20.0, 4.0);
        let value = value.clamp(0.0, 1.0);
        let tx = track.x + track.w * value;
        self.c.fill_round(track, 2.0, t.strong_stroke.fade(0.6));
        self.c.fill_round(Rect::new(track.x, track.y, tx - track.x, track.h), 2.0, t.accent);
        let active = self.hovered(&action) || self.pressed.as_ref() == Some(&action);
        self.c.fill_circle(tx, r.cy(), 10.0, t.control);
        self.c.stroke_circle(tx, r.cy(), 9.5, 1.0, t.control_border);
        let inner = if self.pressed.as_ref() == Some(&action) { 5.0 } else if active { 7.0 } else { 6.0 };
        self.c.fill_circle(tx, r.cy(), inner, t.accent);
        self.hit(r, action);
        self.frame.slider_track = Some(track);
    }

    pub fn button_width(&mut self, label: &str, icon: bool) -> f32 {
        self.c.measure(label, Font::Regular, BODY).0 + 24.0 + if icon { 22.0 } else { 0.0 }
    }

    /// A push button: the standard look, or filled with the accent colour.
    pub fn button(&mut self, r: Rect, label: &str, icon: Option<&str>, accent: bool, action: Action) {
        let t = self.t;
        let pressed = self.pressed(&action);
        let hover = self.hovered(&action);
        let text = if accent {
            let fill = if pressed { t.accent_pressed } else if hover { t.accent_hover } else { t.accent };
            self.c.fill_round(r, 4.0, fill);
            t.on_accent
        } else {
            let fill = if pressed { t.control_pressed } else if hover { t.control_hover } else { t.control };
            self.c.fill_round(r, 4.0, fill);
            self.c.stroke_round(r, 4.0, 1.0, t.control_border);
            if pressed {
                t.text2
            } else {
                t.text
            }
        };
        let label_w = self.c.measure(label, Font::Regular, BODY).0;
        let total = label_w + if icon.is_some() { 22.0 } else { 0.0 };
        let mut x = (r.cx() - total / 2.0).round();
        if let Some(g) = icon {
            self.c.glyph(g, 12.0, x + 7.0, r.cy(), text);
            x += 22.0;
        }
        self.c.text_in(label, Font::Regular, BODY, Rect::new(x, r.y, label_w + 1.0, r.h), Align::Start, text);
        self.hit(r, action);
    }

    /// Accent-coloured text that underlines on hover. Returns its width.
    pub fn link(&mut self, x: f32, y: f32, label: &str, size: f32, action: Action) -> f32 {
        let t = self.t;
        let (w, h) = self.c.measure(label, Font::Regular, size);
        let color = if self.pressed(&action) { t.accent.fade(0.7) } else { t.accent };
        self.c.text(label, Font::Regular, size, x, y, color);
        if self.hovered(&action) {
            self.c.line(x, y + h - 2.0, x + w, y + h - 2.0, 1.0, color);
        }
        self.hit(Rect::new(x - 4.0, y - 2.0, w + 8.0, h + 4.0), action);
        w
    }

    /// A radio button with its label, in `r`.
    pub fn radio(&mut self, r: Rect, label: &str, selected: bool, action: Action) {
        let t = self.t;
        let hover = self.hovered(&action);
        let pressed = self.pressed(&action);
        let cx = r.x + 10.0;
        let cy = r.cy();
        if selected {
            let fill = if pressed { t.accent_pressed } else if hover { t.accent_hover } else { t.accent };
            self.c.fill_circle(cx, cy, 10.0, fill);
            let inner = if pressed { 3.5 } else if hover { 5.0 } else { 4.0 };
            self.c.fill_circle(cx, cy, inner, t.on_accent);
        } else {
            let fill = if pressed { t.subtle_pressed } else if hover { t.subtle_hover } else { t.control.fade(0.0) };
            self.c.fill_circle(cx, cy, 10.0, fill);
            self.c.stroke_circle(cx, cy, 9.5, 1.0, t.strong_stroke);
        }
        self.c.text_in(label, Font::Regular, BODY, Rect::new(r.x + 30.0, r.y, r.w - 30.0, r.h), Align::Start, t.text);
        self.hit(r, action);
    }

    // ---- Shortcut recorder -------------------------------------------------------------

    fn keycaps(text: &str) -> Vec<&str> {
        text.split('+').filter(|k| !k.is_empty()).collect()
    }

    pub fn shortcut_width(&mut self, text: &str, recording: Option<&str>) -> f32 {
        let inner = match recording {
            Some(live) => self.c.measure(live, Font::Regular, BODY).0 + 8.0,
            None => {
                let caps = Self::keycaps(text);
                caps.iter().map(|k| self.c.measure(k, Font::Semibold, CAPTION).0 + 16.0).sum::<f32>() + 4.0 * caps.len().saturating_sub(1) as f32
            }
        };
        (inner + 16.0).max(168.0)
    }

    /// A shortcut shown as keycaps; click to record a new one.
    pub fn shortcut(&mut self, r: Rect, text: &str, recording: Option<&str>, action: Action) {
        let t = self.t;
        let pressed = self.pressed(&action);
        let hover = self.hovered(&action);
        let fill = if pressed { t.control_pressed } else if hover { t.control_hover } else { t.control };
        self.c.fill_round(r, 4.0, fill);
        match recording {
            Some(live) => {
                self.c.stroke_round(r, 4.0, 2.0, t.accent);
                self.c.text_in(live, Font::Regular, BODY, r, Align::Center, t.text2);
            }
            None => {
                self.c.stroke_round(r, 4.0, 1.0, t.control_border);
                let caps = Self::keycaps(text);
                let widths: Vec<f32> = caps.iter().map(|k| self.c.measure(k, Font::Semibold, CAPTION).0 + 16.0).collect();
                let total = widths.iter().sum::<f32>() + 4.0 * widths.len().saturating_sub(1) as f32;
                let mut x = (r.cx() - total / 2.0).round();
                for (cap, w) in caps.iter().zip(widths) {
                    let key = Rect::new(x, r.cy() - 11.0, w, 22.0);
                    self.c.fill_round(key, 4.0, t.accent);
                    self.c.text_in(cap, Font::Semibold, CAPTION, key, Align::Center, t.on_accent);
                    x += w + 4.0;
                }
            }
        }
        self.hit(r, action);
    }

    // ---- Pictures ----------------------------------------------------------------------

    /// Draws a pointer style's picture in `r`: its thumbnail, or the glowing dot for the laser.
    pub fn pointer_picture(&mut self, style: &str, r: Rect, opacity: f32) {
        if style == styles::LASER {
            let k = r.w / 64.0;
            let (cx, cy) = (r.cx(), r.cy());
            let red = Color::rgb(styles::INK_RED);
            self.c.fill_circle(cx, cy, 22.0 * k, red.fade(0.12 * opacity));
            self.c.fill_circle(cx, cy, 16.0 * k, red.fade(0.22 * opacity));
            self.c.fill_circle(cx, cy, 11.0 * k, red.fade(0.32 * opacity));
            self.c.fill_circle(cx, cy, 7.0 * k, red.fade(opacity));
            self.c.fill_circle(cx, cy, 3.0 * k, Color { r: 1.0, g: 0.86, b: 0.82, a: 0.95 * opacity });
            return;
        }
        let key = format!("thumb:{style}");
        if !self.c.has_bitmap(&key) {
            if let Some(image) = styles::thumbnail(style) {
                self.c.add_bitmap(&key, &image);
            }
        }
        if !self.c.bitmap(&key, r, opacity) {
            // The picture couldn't be loaded: a quiet placeholder rather than a gap.
            let t = self.t;
            self.c.fill_round(r.inset(r.w * 0.2, r.h * 0.2), 6.0, t.text3.fade(0.3 * opacity));
        }
    }
}
