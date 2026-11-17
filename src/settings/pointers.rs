//! The Appearance page: the pointers in the toolbar (click to use, drag to reorder, × to
//! remove), the pointers you can add, size and toolbar options, and your own hands.
//! Also the pure rules for changing the toolbar's list, which the tests below cover.

use super::canvas::{Align, Color, Font, Rect};
use super::ui::{Control, Row, Ui, BODY, CAPTION, GLYPH_CLOSE, GLYPH_DELETE};
use super::view::{Action, Ctx, PinnedRow, Switch, View, GAP_GROUP, GAP_ROW};
use crate::prefs::{self, MAX_PINNED};
use crate::styles;

/// Why the toolbar list couldn't change.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PinError {
    Full,
    LastOne,
}

/// The list with `id` added at the end.
pub fn pin(pinned: &[String], id: &str) -> Result<Vec<String>, PinError> {
    if pinned.iter().any(|p| p == id) {
        return Ok(pinned.to_vec());
    }
    if pinned.len() >= MAX_PINNED {
        return Err(PinError::Full);
    }
    let mut out = pinned.to_vec();
    out.push(id.to_string());
    Ok(out)
}

/// The list without `id`. The toolbar always keeps at least one pointer.
pub fn unpin(pinned: &[String], id: &str) -> Result<Vec<String>, PinError> {
    if !pinned.iter().any(|p| p == id) {
        return Ok(pinned.to_vec());
    }
    if pinned.len() <= 1 {
        return Err(PinError::LastOne);
    }
    Ok(pinned.iter().filter(|p| *p != id).cloned().collect())
}

/// The list with `id` moved to position `to` (clamped to the list).
pub fn reorder(pinned: &[String], id: &str, to: usize) -> Vec<String> {
    let mut out: Vec<String> = pinned.iter().filter(|p| *p != id).cloned().collect();
    if out.len() == pinned.len() {
        return out;
    }
    out.insert(to.min(out.len()), id.to_string());
    out
}

/// Which of `count` slots, starting at `left` and `step` apart, a tile centred at `x` is over.
pub fn slot_at(x: f32, left: f32, step: f32, count: usize) -> usize {
    if count == 0 || step <= 0.0 {
        return 0;
    }
    let i = ((x - left) / step).floor();
    (i.max(0.0) as usize).min(count - 1)
}

/// The list with a newly made hand added before the laser dot (if the laser is last), or
/// `None` when the toolbar is full.
pub fn with_new_hand(pinned: &[String], id: &str) -> Option<Vec<String>> {
    if pinned.len() >= MAX_PINNED {
        return None;
    }
    let mut out = pinned.to_vec();
    let at = if out.last().map(String::as_str) == Some(styles::LASER) { out.len() - 1 } else { out.len() };
    out.insert(at, id.to_string());
    Some(out)
}

/// The saved list after the hand `id` is deleted; never empty.
pub fn without(pinned: &[String], id: &str) -> Vec<String> {
    let out: Vec<String> = pinned.iter().filter(|p| *p != id).cloned().collect();
    if out.is_empty() {
        styles::default_pinned()
    } else {
        out
    }
}

pub fn page(ui: &mut Ui, view: &View, ctx: &Ctx, x: f32, y: f32, w: f32) -> f32 {
    let p = &ctx.prefs;
    let t = ui.t;
    let pinned = p.pinned();
    let current = p.style();
    let full = pinned.len() >= MAX_PINNED;

    // ---- In your toolbar ----
    ui.c.text("In your toolbar", Font::Semibold, BODY, x + 2.0, y, t.text);
    let reset_w = ui.c.measure("Reset", Font::Regular, CAPTION).0;
    ui.link(x + w - reset_w - 2.0, y + 2.0, "Reset", CAPTION, Action::ResetToolbar);
    let mut y = y + 28.0;

    let pad = 10.0;
    let gap = 6.0;
    let slot_w = ((w - 2.0 * pad - gap * (MAX_PINNED as f32 - 1.0)) / MAX_PINNED as f32).floor();
    let slot_h = 60.0;
    let card_h = 2.0 * pad + slot_h;
    let card = t.card;
    ui.card(Rect::new(x, y, w, card_h), card);
    let row = PinnedRow { left: x + pad, top: y + pad, step: slot_w + gap, slot_w, count: pinned.len() };
    ui.frame.pinned_row = Some(row);

    let dragging = ctx.drag.as_ref().filter(|d| pinned.contains(&d.id));
    let order = match dragging {
        Some(d) => reorder(&pinned, &d.id, slot_at(d.x + slot_w / 2.0, row.left, row.step, row.count)),
        None => pinned.clone(),
    };
    for i in 0..MAX_PINNED {
        let r = Rect::new(row.left + i as f32 * row.step, row.top, slot_w, slot_h);
        match order.get(i) {
            Some(id) if dragging.map(|d| &d.id) != Some(id) => {
                // The × shows while the tile or the × itself is under the mouse, and stays
                // while the × is held down so letting go over it still counts.
                let unpin = Action::Unpin(id.clone());
                let show_badge = dragging.is_none()
                    && (ui.hovered(&Action::Tile(id.clone())) || ui.hovered(&unpin) || ui.pressed.as_ref() == Some(&unpin));
                tile(ui, r, id, *id == current, show_badge, false);
            }
            _ => ui.c.stroke_round_dashed(r, 6.0, 1.0, t.text3.fade(0.7)),
        }
    }
    if let Some(d) = dragging {
        let max_x = row.left + (row.count.max(1) - 1) as f32 * row.step;
        let r = Rect::new(d.x.clamp(row.left, max_x), row.top, slot_w, slot_h);
        tile(ui, r, &d.id, d.id == current, false, true);
    }
    y += card_h + 8.0;

    let hovered_tile = order.iter().find(|id| ui.hovered(&Action::Tile((*id).clone())));
    let hovered_badge = order.iter().find(|id| ui.hovered(&Action::Unpin((*id).clone())));
    let hint = if dragging.is_some() {
        "Drop it where you want it in the toolbar.".to_string()
    } else if let Some(id) = hovered_badge {
        format!("Remove {} from the toolbar.", styles::title(id))
    } else if let Some(id) = hovered_tile {
        if *id == current {
            format!("{} is the pointer you're using.", styles::title(id))
        } else {
            format!("{}. Click to use it.", styles::title(id))
        }
    } else if full {
        "The toolbar is full. Remove a pointer to add another. Drag to reorder.".to_string()
    } else {
        "Click a pointer to use it. Drag to reorder. Point at one and press × to remove it.".to_string()
    };
    y += ui.note(x, y, w, &hint, t.text2);
    if let Some(message) = &view.pin_message {
        y += ui.note(x, y, w, message, t.danger);
    }

    // ---- Add a pointer ----
    y += GAP_GROUP - 8.0;
    ui.c.text("Add a pointer", Font::Semibold, BODY, x + 2.0, y, t.text);
    let extra: Vec<String> = styles::all_ids().into_iter().filter(|id| !pinned.contains(id)).collect();
    if let Some(id) = extra.iter().find(|id| ui.hovered(&Action::Pin((*id).clone()))) {
        let caption = if full { format!("{} (the toolbar is full)", styles::title(id)) } else { format!("Add {}", styles::title(id)) };
        ui.c.text_in(&caption, Font::Regular, CAPTION, Rect::new(x + 140.0, y, w - 142.0, 20.0), Align::End, t.text2);
    }
    y += 28.0;
    let cols = MAX_PINNED;
    let rows = extra.len().div_ceil(cols).max(1);
    let card_h = 2.0 * pad + rows as f32 * slot_h + (rows as f32 - 1.0) * gap;
    ui.card(Rect::new(x, y, w, card_h), card);
    for (i, id) in extra.iter().enumerate() {
        let r = Rect::new(x + pad + (i % cols) as f32 * (slot_w + gap), y + pad + (i / cols) as f32 * (slot_h + gap), slot_w, slot_h);
        let action = Action::Pin(id.clone());
        if ui.pressed(&action) {
            ui.c.fill_round(r, 6.0, t.subtle_pressed);
        } else if ui.hovered(&action) {
            ui.c.fill_round(r, 6.0, t.subtle_hover);
        }
        if *id == current {
            ui.c.stroke_round(r, 6.0, 2.0, t.accent);
        }
        ui.pointer_picture(id, Rect::new(r.cx() - 20.0, r.cy() - 20.0, 40.0, 40.0), 1.0);
        ui.hit(r, action);
    }
    y += card_h + 8.0;
    y += ui.note(x, y, w, "Click a pointer to add it to the toolbar.", t.text2);

    // ---- Pointer ----
    y += GAP_GROUP - 8.0;
    y = ui.header(x, y, "Pointer");
    let size = (p.pointer_size - prefs::POINTER_SIZE_MIN) / (prefs::POINTER_SIZE_MAX - prefs::POINTER_SIZE_MIN);
    y += ui.row(x, y, w, Row::new("Size").control(Control::Slider { t: size, action: Action::PointerSize })) + GAP_ROW;
    let row = Row::new("Shadow under the pointer").control(Control::Toggle { on: p.show_shadow, action: Action::Switch(Switch::Shadow) });
    y += ui.row(x, y, w, row);

    // ---- Toolbar ----
    y += GAP_GROUP;
    y = ui.header(x, y, "Toolbar");
    let detail = if p.show_toolbar {
        "Pops up when the pointer turns on and near the bottom of the screen."
    } else {
        "Stays hidden. Press Esc to exit, or use the tray icon."
    };
    let row = Row::new("Show the toolbar")
        .sub(detail)
        .control(Control::Toggle { on: p.show_toolbar, action: Action::Switch(Switch::ShowToolbar) });
    y += ui.row(x, y, w, row) + GAP_ROW;
    let row = Row::new("Ink button")
        .sub("Change the colour while you present.")
        .control(Control::Toggle { on: p.show_ink_button, action: Action::Switch(Switch::InkButton) });
    y += ui.row(x, y, w, row);

    // ---- Your hands ----
    y += GAP_GROUP;
    y = ui.header(x, y, "Your hands");
    for id in styles::custom_ids() {
        let name = styles::title(&id);
        let detail = if pinned.contains(&id) { "In your toolbar" } else { "Not in your toolbar" };
        let row = Row::new(&name).picture(&id).sub(detail).control(Control::Button {
            label: "Delete",
            icon: Some(GLYPH_DELETE),
            accent: false,
            action: Action::DeleteHand(id.clone()),
        });
        y += ui.row(x, y, w, row) + GAP_ROW;
    }
    let row = Row::new("Make your own hand…")
        .sub("Turn any idea into a hand with ChatGPT or another image tool.")
        .control(Control::Chevron)
        .click(Action::MakeHand);
    y += ui.row(x, y, w, row);
    if let Some(message) = &view.hand_message {
        y += 8.0;
        y += ui.note(x, y, w, message, t.text2);
    }
    y
}

/// A pointer in the toolbar row.
fn tile(ui: &mut Ui, r: Rect, id: &str, current: bool, show_badge: bool, lifted: bool) {
    let t = ui.t;
    let action = Action::Tile(id.to_string());
    if lifted {
        ui.c.fill_round(Rect::new(r.x - 2.0, r.y + 1.0, r.w + 4.0, r.h + 6.0), 9.0, Color::rgba(0, 0.08));
        ui.c.fill_round(Rect::new(r.x - 1.0, r.y + 1.0, r.w + 2.0, r.h + 3.0), 8.0, Color::rgba(0, 0.1));
    }
    let fill = if lifted || ui.pressed(&action) {
        t.control_pressed
    } else if ui.hovered(&action) || show_badge {
        t.control_hover
    } else {
        t.control
    };
    ui.c.fill_round(r, 6.0, fill);
    if current {
        ui.c.stroke_round(r, 6.0, 2.0, t.accent);
    } else {
        ui.c.stroke_round(r, 6.0, 1.0, t.control_border);
    }
    ui.pointer_picture(id, Rect::new(r.cx() - 20.0, r.cy() - 20.0, 40.0, 40.0), 1.0);
    if !lifted {
        ui.hit(r, action);
    }
    if show_badge {
        let unpin = Action::Unpin(id.to_string());
        let (cx, cy) = (r.right() - 9.0, r.y + 9.0);
        let fill = if ui.hovered(&unpin) {
            Color::rgb(0xC42B1C)
        } else if t.dark {
            Color::rgb(0x6E6E6E)
        } else {
            Color::rgba(0, 0.6)
        };
        ui.c.fill_circle(cx, cy, 8.0, fill);
        ui.c.glyph(GLYPH_CLOSE, 7.0, cx, cy, Color::rgb(0xFFFFFF));
        ui.hit(Rect::new(cx - 9.0, cy - 9.0, 18.0, 18.0), unpin);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ids(list: &[&str]) -> Vec<String> {
        list.iter().map(|s| s.to_string()).collect()
    }

    #[test]
    fn pin_adds_at_the_end_until_full() {
        assert_eq!(pin(&ids(&["a"]), "b"), Ok(ids(&["a", "b"])));
        assert_eq!(pin(&ids(&["a", "b"]), "a"), Ok(ids(&["a", "b"])));
        let full: Vec<String> = (0..MAX_PINNED).map(|i| i.to_string()).collect();
        assert_eq!(pin(&full, "x"), Err(PinError::Full));
    }

    #[test]
    fn unpin_keeps_one() {
        assert_eq!(unpin(&ids(&["a", "b"]), "a"), Ok(ids(&["b"])));
        assert_eq!(unpin(&ids(&["a"]), "a"), Err(PinError::LastOne));
        assert_eq!(unpin(&ids(&["a"]), "z"), Ok(ids(&["a"])));
    }

    #[test]
    fn reorder_moves_one_item() {
        let list = ids(&["a", "b", "c", "d"]);
        assert_eq!(reorder(&list, "a", 2), ids(&["b", "c", "a", "d"]));
        assert_eq!(reorder(&list, "d", 0), ids(&["d", "a", "b", "c"]));
        assert_eq!(reorder(&list, "b", 99), ids(&["a", "c", "d", "b"]));
        assert_eq!(reorder(&list, "zz", 1), list);
    }

    #[test]
    fn slots_are_clamped() {
        assert_eq!(slot_at(5.0, 10.0, 60.0, 4), 0);
        assert_eq!(slot_at(69.0, 10.0, 60.0, 4), 0);
        assert_eq!(slot_at(70.0, 10.0, 60.0, 4), 1);
        assert_eq!(slot_at(1000.0, 10.0, 60.0, 4), 3);
        assert_eq!(slot_at(1000.0, 10.0, 60.0, 0), 0);
    }

    #[test]
    fn new_hands_go_before_the_laser() {
        assert_eq!(with_new_hand(&ids(&["a", "laser"]), "h"), Some(ids(&["a", "h", "laser"])));
        assert_eq!(with_new_hand(&ids(&["laser", "a"]), "h"), Some(ids(&["laser", "a", "h"])));
        let full: Vec<String> = (0..MAX_PINNED).map(|i| i.to_string()).collect();
        assert_eq!(with_new_hand(&full, "h"), None);
    }

    #[test]
    fn deleting_the_last_hand_restores_defaults() {
        assert_eq!(without(&ids(&["a", "h"]), "h"), ids(&["a"]));
        assert_eq!(without(&ids(&["h"]), "h"), styles::default_pinned());
    }
}
