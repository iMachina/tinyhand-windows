//! What the settings window shows: the sections, the clickable actions, the window's view
//! state, and the top-level layout (sidebar, page title, scrolling content area).
//!
//! Every paint builds a [`Frame`]: the rectangles that were drawn as clickable, with the
//! [`Action`] each one performs. Mouse input is hit-tested against the last frame, so what you
//! see in a `--settings-test` picture is exactly what the mouse can hit.

use super::canvas::{Align, Font, Rect};
use super::ui::{Ui, CAPTION, GLYPH_BACK};
use super::{maker, pages, pointers};
use crate::prefs::{InkChoice, InteractKey, Prefs};
use image::RgbaImage;

/// Client size of the window in DIPs.
pub const WIN_W: f32 = 760.0;
pub const WIN_H: f32 = 560.0;
pub const SIDEBAR_W: f32 = 204.0;
pub const CONTENT_X: f32 = 220.0;
pub const CONTENT_W: f32 = 512.0;
/// Top of the scrolling area, under the page title.
pub const VIEW_TOP: f32 = 84.0;
/// Space between groups of cards, and between cards in a group.
pub const GAP_GROUP: f32 = 26.0;
pub const GAP_ROW: f32 = 4.0;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Section {
    General,
    Appearance,
    Drawing,
    Sound,
    About,
}

impl Section {
    pub const ALL: [Section; 5] = [Section::General, Section::Appearance, Section::Drawing, Section::Sound, Section::About];

    pub fn title(self) -> &'static str {
        match self {
            Section::General => "General",
            Section::Appearance => "Appearance",
            Section::Drawing => "Drawing",
            Section::Sound => "Sound",
            Section::About => "About",
        }
    }

    /// Segoe MDL2 Assets glyph for the sidebar.
    pub fn glyph(self) -> &'static str {
        match self {
            Section::General => "\u{E713}",
            Section::Appearance => "\u{E7C9}",
            Section::Drawing => "\u{E70F}",
            Section::Sound => "\u{E767}",
            Section::About => "\u{E946}",
        }
    }

    pub fn from_name(name: &str) -> Option<Section> {
        Section::ALL.into_iter().find(|s| s.title().eq_ignore_ascii_case(name))
    }

    /// The section `step` places further down the sidebar, wrapping around.
    pub fn step(self, step: i32) -> Section {
        let i = Section::ALL.iter().position(|s| *s == self).unwrap_or(0) as i32;
        let n = Section::ALL.len() as i32;
        Section::ALL[(i + step).rem_euclid(n) as usize]
    }
}

/// The on/off settings.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Switch {
    DoubleShift,
    LaunchAtLogin,
    Shadow,
    ShowToolbar,
    InkButton,
    HandSound,
    LaserSound,
}

/// The two text fields of the hand maker.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Field {
    Idea,
    Name,
}

/// Everything a click can do.
#[derive(Clone, Debug, PartialEq)]
pub enum Action {
    Section(Section),
    Switch(Switch),
    /// Start recording a shortcut: `true` for the on/off shortcut, `false` for "clear".
    Record(bool),
    Interact(InteractKey),
    KeepHand(bool),
    /// The pointer size slider. Pressing or dragging sets the size.
    PointerSize,
    /// A pointer in the toolbar row: a click makes it the current pointer, a drag reorders.
    Tile(String),
    Unpin(String),
    Pin(String),
    ResetToolbar,
    DeleteHand(String),
    MakeHand,
    Ink(InkChoice),
    /// Seconds before lines fade; 0 keeps them.
    Fade(u32),
    Sound(&'static str),
    Open(&'static str),
    ResetAll,
    Back,
    OpenChatGpt,
    CopyPrompt,
    ChooseFile,
    AddHand,
    /// Focus a text field (a click on its padding, outside the edit control).
    Focus(Field),
    ScrollThumb,
}

impl Action {
    /// Links get the hand cursor; everything else keeps the arrow, as in Windows Settings.
    pub fn is_link(&self) -> bool {
        matches!(self, Action::Open(_) | Action::Back | Action::ResetToolbar)
    }
}

/// One clickable rectangle from the last paint.
#[derive(Clone, Debug)]
pub struct Hit {
    pub rect: Rect,
    pub action: Action,
}

/// Where the toolbar row's slots are, for dragging pointers around.
#[derive(Clone, Copy, Debug, Default)]
pub struct PinnedRow {
    pub left: f32,
    pub top: f32,
    pub step: f32,
    pub slot_w: f32,
    pub count: usize,
}

/// What the last paint laid out.
#[derive(Clone, Debug, Default)]
pub struct Frame {
    pub hits: Vec<Hit>,
    pub viewport: Rect,
    /// Height of the scrolling content, including padding.
    pub content_h: f32,
    pub pinned_row: Option<PinnedRow>,
    pub slider_track: Option<Rect>,
    pub scroll_track: Option<Rect>,
    pub scroll_thumb: Option<Rect>,
    /// Where the two edit controls go, or `None` to hide them.
    pub idea_field: Option<Rect>,
    pub name_field: Option<Rect>,
}

impl Frame {
    /// The action under a point. Later (topmost) rectangles win.
    pub fn hit_test(&self, x: f32, y: f32) -> Option<Action> {
        self.hits.iter().rev().find(|h| h.rect.contains(x, y)).map(|h| h.action.clone())
    }

    pub fn rect_of(&self, action: &Action) -> Option<Rect> {
        self.hits.iter().rev().find(|h| &h.action == action).map(|h| h.rect)
    }

    pub fn max_scroll(&self) -> f32 {
        (self.content_h - self.viewport.h).max(0.0)
    }
}

/// A shortcut being recorded.
#[derive(Clone, Debug)]
pub struct Recording {
    pub is_toggle: bool,
    /// What the recorder shows: "Press a shortcut…", the modifiers held so far, or a hint.
    pub live: String,
}

/// A hand that passed the checks and waits for a name.
pub struct Draft {
    pub made: crate::handmaker::Made,
    /// Key of the preview picture in the canvas cache (unique per draft).
    pub key: String,
    /// The preview, premultiplied, ready to upload.
    pub preview: RgbaImage,
}

/// The hand maker's state.
#[derive(Default)]
pub struct Maker {
    pub draft: Option<Draft>,
    /// Why the last picture was turned down.
    pub error: Option<String>,
    /// Why saving failed.
    pub save_error: Option<String>,
    pub copied: bool,
    /// Field text, used only when there are no edit controls (`--settings-test`).
    pub idea: String,
    pub name: String,
    pub focus: Option<Field>,
}

/// Everything about the window that isn't a saved setting.
pub struct View {
    pub section: Section,
    pub making_hand: bool,
    pub scroll: f32,
    pub recording: Option<Recording>,
    pub shortcut_error: Option<String>,
    pub pin_message: Option<String>,
    pub hand_message: Option<String>,
    pub login_failed: bool,
    pub maker: Maker,
}

impl View {
    pub fn new(section: Section) -> View {
        View {
            section,
            making_hand: false,
            scroll: 0.0,
            recording: None,
            shortcut_error: None,
            pin_message: None,
            hand_message: None,
            login_failed: false,
            maker: Maker::default(),
        }
    }

    /// Forgets the one-off messages (after a section change or a reset).
    pub fn clear_messages(&mut self) {
        self.shortcut_error = None;
        self.pin_message = None;
        self.hand_message = None;
        self.login_failed = false;
    }
}

/// A pointer being dragged along the toolbar row.
#[derive(Clone, Debug)]
pub struct TileDrag {
    pub id: String,
    /// Left edge of the dragged tile, in DIPs.
    pub x: f32,
}

/// Facts read fresh for each paint.
pub struct Ctx {
    pub prefs: Prefs,
    pub login: bool,
    pub drag: Option<TileDrag>,
    /// True in the real window, where edit controls draw the field text.
    pub live_edits: bool,
}

/// Draws the whole window.
pub fn draw(ui: &mut Ui, view: &View, ctx: &Ctx) {
    sidebar(ui, view.section);

    let t = ui.t;
    if view.making_hand {
        let back = Action::Back;
        let hover = ui.hovered(&back);
        let color = if hover { t.text } else { t.text2 };
        ui.c.glyph(GLYPH_BACK, 10.0, CONTENT_X + 6.0, 24.0, color);
        let w = ui.c.text(Section::Appearance.title(), Font::Regular, CAPTION, CONTENT_X + 18.0, 16.0, color);
        ui.hit(Rect::new(CONTENT_X - 4.0, 12.0, w + 26.0, 24.0), back);
        ui.c.text("Make your own hand", Font::Semibold, 26.0, CONTENT_X, 36.0, t.text);
    } else {
        ui.c.text(view.section.title(), Font::Semibold, 28.0, CONTENT_X, 26.0, t.text);
    }

    let viewport = Rect::new(SIDEBAR_W, VIEW_TOP, WIN_W - SIDEBAR_W, WIN_H - VIEW_TOP);
    ui.set_clip(Some(viewport));
    let top = VIEW_TOP + 4.0 - view.scroll;
    let bottom = match (view.section, view.making_hand) {
        (Section::General, _) => pages::general(ui, view, ctx, CONTENT_X, top, CONTENT_W),
        (Section::Appearance, false) => pointers::page(ui, view, ctx, CONTENT_X, top, CONTENT_W),
        (Section::Appearance, true) => maker::page(ui, view, ctx, CONTENT_X, top, CONTENT_W),
        (Section::Drawing, _) => pages::drawing(ui, view, ctx, CONTENT_X, top, CONTENT_W),
        (Section::Sound, _) => pages::sound(ui, view, ctx, CONTENT_X, top, CONTENT_W),
        (Section::About, _) => pages::about(ui, view, ctx, CONTENT_X, top, CONTENT_W),
    };
    ui.set_clip(None);
    ui.frame.viewport = viewport;
    ui.frame.content_h = bottom - top + 28.0;
    scrollbar(ui, view.scroll);
}

fn sidebar(ui: &mut Ui, current: Section) {
    let t = ui.t;
    let icon = Rect::new(18.0, 18.0, 32.0, 32.0);
    if !ui.c.bitmap(super::APP_ICON_KEY, icon, 1.0) {
        ui.c.fill_round(icon, 8.0, t.accent);
    }
    ui.c.text("TinyHand", Font::Semibold, 14.0, 60.0, 16.0, t.text);
    ui.c.text("Settings", Font::Regular, CAPTION, 60.0, 35.0, t.text2);

    let mut y = 72.0;
    for section in Section::ALL {
        let r = Rect::new(8.0, y, SIDEBAR_W - 16.0, 36.0);
        let action = Action::Section(section);
        let selected = section == current;
        if selected {
            ui.c.fill_round(r, 4.0, t.nav_selected);
            ui.c.fill_round(Rect::new(r.x, r.cy() - 8.0, 3.0, 16.0), 1.5, t.accent);
        } else if ui.pressed(&action) {
            ui.c.fill_round(r, 4.0, t.subtle_pressed);
        } else if ui.hovered(&action) {
            ui.c.fill_round(r, 4.0, t.subtle_hover);
        }
        ui.c.glyph(section.glyph(), 16.0, r.x + 24.0, r.cy(), t.text);
        ui.c.text_in(section.title(), Font::Regular, 14.0, Rect::new(r.x + 48.0, r.y, r.w - 52.0, r.h), Align::Start, t.text);
        ui.hit(r, action);
        y += 40.0;
    }
}

/// A thin scrollbar on the right, shown when the content is taller than the window.
fn scrollbar(ui: &mut Ui, scroll: f32) {
    let f = &ui.frame;
    let (viewport, content_h, max) = (f.viewport, f.content_h, f.max_scroll());
    if max <= 0.0 {
        return;
    }
    let track = Rect::new(WIN_W - 9.0, viewport.y + 4.0, 6.0, viewport.h - 8.0);
    let thumb_h = (track.h * viewport.h / content_h).max(32.0);
    let thumb_y = track.y + (track.h - thumb_h) * (scroll / max).clamp(0.0, 1.0);
    let active = ui.hovered(&Action::ScrollThumb) || ui.pressed.as_ref() == Some(&Action::ScrollThumb);
    let w = if active { 6.0 } else { 3.0 };
    let thumb = Rect::new(track.right() - w, thumb_y, w, thumb_h);
    let t = ui.t;
    ui.c.fill_round(thumb, w / 2.0, t.text2.fade(if active { 0.7 } else { 0.45 }));
    ui.hit(Rect::new(track.x - 6.0, thumb_y, track.w + 9.0, thumb_h), Action::ScrollThumb);
    ui.frame.scroll_track = Some(track);
    ui.frame.scroll_thumb = Some(Rect::new(track.x, thumb_y, track.w, thumb_h));
}
