//! The General, Drawing, Sound and About pages. Each draws from `y` down and returns the
//! y where it ended.

use super::canvas::{Align, Color, Font, Rect};
use super::ui::{Control, Row, Ui, CAPTION, GLYPH_SOUND};
use super::view::{Action, Ctx, Switch, View, GAP_GROUP, GAP_ROW};
use crate::prefs::{InkChoice, InteractKey};
use crate::sound;

pub const SOURCE_URL: &str = "https://github.com/nilbuild/tinyhand";
pub const LICENSE_URL: &str = "https://github.com/nilbuild/tinyhand/blob/main/LICENSE";

pub fn general(ui: &mut Ui, view: &View, ctx: &Ctx, x: f32, y: f32, w: f32) -> f32 {
    let p = &ctx.prefs;
    let t = ui.t;
    let mut y = ui.header(x, y, "Shortcuts");
    let shortcuts = [(true, "Turn the pointer on or off", &p.toggle_shortcut), (false, "Clear the drawing", &p.clear_shortcut)];
    for (is_toggle, title, shortcut) in shortcuts {
        let recording = view.recording.as_ref().filter(|r| r.is_toggle == is_toggle).map(|r| r.live.as_str());
        let control = Control::Shortcut { text: &shortcut.display, recording, action: Action::Record(is_toggle) };
        y += ui.row(x, y, w, Row::new(title).control(control)) + GAP_ROW;
    }
    y += 4.0;
    y += match &view.shortcut_error {
        Some(error) => ui.note(x, y, w, error, t.danger),
        None => ui.note(x, y, w, "Click a shortcut, then press the new keys. Esc cancels.", t.text2),
    };

    y += GAP_GROUP - 8.0;
    y = ui.header(x, y, "Clicking for real");
    let keys: Vec<(&str, Action)> = InteractKey::ALL.iter().map(|k| (k.title(), Action::Interact(*k))).collect();
    let selected = InteractKey::ALL.iter().position(|k| *k == p.interact_key);
    let row = Row::new("Hold to use the app")
        .sub("Clicks, drags and scrolls reach the app.")
        .control(Control::Segmented { items: keys, selected });
    y += ui.row(x, y, w, row) + GAP_ROW;
    if p.interact_key != InteractKey::Off {
        let detail = if p.interact_keeps_hand {
            "The hand stays, in menus too."
        } else {
            "The normal cursor comes back, in menus too."
        };
        let items = vec![("Show the cursor", Action::KeepHand(false)), ("Keep the hand", Action::KeepHand(true))];
        let selected = Some(if p.interact_keeps_hand { 1 } else { 0 });
        let row = Row::new("While holding").sub(detail).control(Control::Segmented { items, selected });
        y += ui.row(x, y, w, row) + GAP_ROW;
    }
    let row = Row::new("Double-tap Shift to click through")
        .sub("Clicks reach the app and the hand stays. Double-tap again for taps. The tray icon turns orange.")
        .control(Control::Toggle { on: p.double_shift_click_through, action: Action::Switch(Switch::DoubleShift) });
    y += ui.row(x, y, w, row);

    y += GAP_GROUP;
    y = ui.header(x, y, "Startup");
    let mut row = Row::new("Open TinyHand when you log in")
        .control(Control::Toggle { on: ctx.login, action: Action::Switch(Switch::LaunchAtLogin) });
    if view.login_failed {
        row = row.sub("Windows didn't allow that change. Try again in a moment.").sub_color(t.danger);
    }
    y += ui.row(x, y, w, row);
    y
}

/// The "Lines fade" choices: label and seconds (0 keeps lines until cleared).
const FADES: [(&str, u32); 4] =
    [("After 3 seconds", 3), ("After 6 seconds", 6), ("After 10 seconds", 10), ("Keep until cleared", 0)];

pub fn drawing(ui: &mut Ui, _view: &View, ctx: &Ctx, x: f32, y: f32, w: f32) -> f32 {
    let p = &ctx.prefs;
    let t = ui.t;
    let mut y = ui.header(x, y, "Ink");

    let detail = match p.ink_choice {
        InkChoice::Match => "Matches each pointer.".to_string(),
        choice => format!("All pointers draw in {}.", choice.title().to_lowercase()),
    };
    let (h, body) = ui.stacked_row(x, y, w, "Ink colour", Some(&detail), 34.0);
    let mut sx = body.x - 3.0;
    for choice in InkChoice::ALL {
        let r = Rect::new(sx, body.y, 34.0, 34.0);
        swatch(ui, r, choice, p.ink_choice == choice);
        sx += 40.0;
    }
    let hovered = InkChoice::ALL.into_iter().find(|c| ui.hovered(&Action::Ink(*c)));
    if let Some(choice) = hovered {
        ui.c.text_in(choice.title(), Font::Regular, CAPTION, Rect::new(sx + 6.0, body.y, body.right() - sx - 6.0, 34.0), Align::Start, t.text2);
    }
    y += h + GAP_ROW;

    let detail = format!("Clear them any time with {}.", p.clear_shortcut.display);
    let (h, body) = ui.stacked_row(x, y, w, "Lines fade", Some(&detail), 2.0 * 32.0 + 4.0);
    let col_w = body.w / 2.0;
    let current = if p.fade_seconds > 0.0 { p.fade_seconds.round() as u32 } else { 0 };
    for (i, (label, seconds)) in FADES.iter().enumerate() {
        let r = Rect::new(body.x + (i % 2) as f32 * col_w, body.y + (i / 2) as f32 * 36.0, col_w - 8.0, 32.0);
        ui.radio(r, label, current == *seconds, Action::Fade(*seconds));
    }
    y += h;
    y
}

/// One ink colour: a dot (a colour wheel for "match the pointer") with a ring when chosen.
fn swatch(ui: &mut Ui, r: Rect, choice: InkChoice, selected: bool) {
    let t = ui.t;
    let action = Action::Ink(choice);
    let (cx, cy) = (r.cx(), r.cy());
    let radius = if ui.hovered(&action) && !selected { 12.5 } else { 12.0 };
    match choice.color() {
        Some(rgb) => {
            ui.c.fill_circle(cx, cy, radius, Color::rgb(rgb));
            // Light swatches need an edge on a light card, dark ones on a dark card.
            let edge = if t.dark { Color::rgba(0xFFFFFF, 0.25) } else { Color::rgba(0x000000, 0.18) };
            ui.c.stroke_circle(cx, cy, radius - 0.5, 1.0, edge);
        }
        None => {
            let wheel = [0xFF3B30, 0xFF9500, 0xFFCC00, 0x34C759, 0x007AFF, 0xAF52DE];
            for (i, rgb) in wheel.iter().enumerate() {
                ui.c.fill_pie(cx, cy, radius, i as f32 * 60.0, (i + 1) as f32 * 60.0 + 0.5, Color::rgb(*rgb));
            }
        }
    }
    if selected {
        ui.c.stroke_circle(cx, cy, 15.5, 2.0, t.accent);
    } else if ui.hovered(&action) {
        ui.c.stroke_circle(cx, cy, 15.5, 1.0, t.strong_stroke);
    }
    ui.hit(r, action);
}

pub fn sound(ui: &mut Ui, _view: &View, ctx: &Ctx, x: f32, y: f32, w: f32) -> f32 {
    let p = &ctx.prefs;
    let t = ui.t;
    let on = p.hand_sound;
    let mut y = ui.header(x, y, "Tap sound");
    let row = Row::new("Tap sound")
        .sub("A small sound each time the hand taps.")
        .control(Control::Toggle { on, action: Action::Switch(Switch::HandSound) });
    y += ui.row(x, y, w, row) + GAP_ROW;

    let choices = sound::choices();
    let cols = 4usize;
    let gap = 8.0;
    let pad = 12.0;
    let tile_w = ((w - 2.0 * pad - gap * (cols as f32 - 1.0)) / cols as f32).floor();
    let tile_h = 64.0;
    let rows = choices.len().div_ceil(cols);
    let card_h = 2.0 * pad + rows as f32 * tile_h + (rows as f32 - 1.0) * gap;
    let card = t.card;
    ui.card(Rect::new(x, y, w, card_h), card);
    for (i, (id, title)) in choices.iter().enumerate() {
        let r = Rect::new(
            x + pad + (i % cols) as f32 * (tile_w + gap),
            y + pad + (i / cols) as f32 * (tile_h + gap),
            tile_w,
            tile_h,
        );
        sound_tile(ui, r, id, title, on && p.tap_sound == *id, on);
    }
    y += card_h + 8.0;
    y += ui.note(x, y, w, "Click a sound to hear it. Choosing one turns the tap sound on.", t.text2);

    y += GAP_GROUP - 8.0;
    let row = Row::new("Laser dot taps too")
        .sub("Off by default, for quiet pointing.")
        .enabled(on)
        .control(Control::Toggle { on: p.laser_sound, action: Action::Switch(Switch::LaserSound) });
    y += ui.row(x, y, w, row);
    y
}

fn sound_tile(ui: &mut Ui, r: Rect, id: &'static str, title: &str, selected: bool, sound_on: bool) {
    let t = ui.t;
    let action = Action::Sound(id);
    let hover = ui.hovered(&action);
    let pressed = ui.pressed(&action);
    if selected {
        ui.c.fill_round(r, 6.0, t.accent.fade(if t.dark { 0.16 } else { 0.1 }));
        ui.c.stroke_round(r, 6.0, 1.5, t.accent);
    } else {
        let fill = if pressed { t.subtle_pressed } else if hover { t.subtle_hover } else { t.subtle_pressed.fade(0.7) };
        ui.c.fill_round(r, 6.0, fill);
    }
    let dim = if sound_on { 1.0 } else { 0.55 };
    let glyph_color = if selected { t.accent } else { t.text2.fade(dim) };
    ui.c.glyph(GLYPH_SOUND, 16.0, r.cx(), r.y + 22.0, glyph_color);
    let (font, color) = if selected { (Font::Semibold, t.text) } else { (Font::Regular, t.text2.fade(dim)) };
    ui.c.text_in(title, font, CAPTION, Rect::new(r.x + 4.0, r.y + 36.0, r.w - 8.0, 20.0), Align::Center, color);
    ui.hit(r, action);
}

pub fn about(ui: &mut Ui, _view: &View, _ctx: &Ctx, x: f32, y: f32, w: f32) -> f32 {
    let t = ui.t;
    let icon = Rect::new(x, y + 4.0, 64.0, 64.0);
    if !ui.c.bitmap(super::APP_ICON_KEY, icon, 1.0) {
        ui.c.fill_round(icon, 14.0, t.accent);
    }
    let tx = x + 84.0;
    ui.c.text("TinyHand", Font::Semibold, 20.0, tx, y + 2.0, t.text);
    let version = format!("Version {}", env!("CARGO_PKG_VERSION"));
    ui.c.text(&version, Font::Regular, CAPTION, tx, y + 32.0, t.text2);
    ui.c.text("A tiny hand for your screen. Free and open source.", Font::Regular, CAPTION, tx, y + 50.0, t.text2);
    let mut y = y + 92.0;

    let row = Row::new("Made for Windows")
        .sub("A Windows port in Rust of the macOS app by Kamran Ahmed.");
    y += ui.row(x, y, w, row) + GAP_ROW;
    let row = Row::new("Source code").control(Control::External("github.com/nilbuild/tinyhand")).click(Action::Open(SOURCE_URL));
    y += ui.row(x, y, w, row) + GAP_ROW;
    let row = Row::new("Licence")
        .sub("Free to use, change and share under the MIT licence.")
        .control(Control::External("MIT"))
        .click(Action::Open(LICENSE_URL));
    y += ui.row(x, y, w, row);

    y += GAP_GROUP;
    let label = "Reset all settings…";
    let bw = ui.button_width(label, false);
    ui.button(Rect::new(x + w - bw, y, bw, 32.0), label, None, false, Action::ResetAll);
    let note_w = w - bw - 24.0;
    ui.c.text_wrapped(
        "Puts shortcuts, toolbar pointers, size, shadow, drawing and sound back to how they were at first.",
        Font::Regular,
        CAPTION,
        x + 2.0,
        y,
        note_w,
        t.text2,
    );
    y + 40.0
}
