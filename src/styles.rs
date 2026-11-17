//! Pointer styles: the built-in hands, custom hands and the laser dot, plus their pictures.
//!
//! A style is identified by a string id: `"lavender"`, `"custom-1a2b3c4d"`, `"laser"`.

use crate::{custom, handasset};
use image::RgbaImage;
use std::collections::HashMap;
use std::sync::{Arc, LazyLock, Mutex};

pub const LASER: &str = "laser";
pub const INK_PURPLE: u32 = 0x8754F2;
pub const INK_RED: u32 = 0xFF291F;

struct BuiltIn {
    id: &'static str,
    title: &'static str,
    ink: u32,
    image: &'static [u8],
    thumb: &'static [u8],
}

macro_rules! built_in {
    ($(($id:literal, $title:literal, $ink:expr)),* $(,)?) => {
        &[$(BuiltIn {
            id: $id,
            title: $title,
            ink: $ink,
            image: include_bytes!(concat!("../assets/hands/", $id, ".webp")),
            thumb: include_bytes!(concat!("../assets/thumbs/", $id, ".png")),
        }),*]
    };
}

static BUILT_IN: &[BuiltIn] = built_in![
    ("lavender", "Lavender toy", INK_PURPLE),
    ("glove", "Cursor glove", INK_PURPLE),
    ("coral", "Coral vinyl", INK_RED),
    ("plush", "Plush", 0x2F86F0),
    ("robot", "Robot", 0x16A87A),
    ("pink", "Bubblegum", 0xEE4F98),
    ("blue", "Sky", 0x2F86F0),
    ("mint", "Mint", 0x16A87A),
    ("lemon", "Lemon", 0xF2B705),
    ("black", "Midnight", 0x1D1D21),
    ("chrome", "Chrome", 0x6B7380),
    ("holo", "Holo", INK_PURPLE),
    ("fuzzy", "Fuzzy", INK_PURPLE),
    ("wood", "Wood", 0x9A5E2C),
    ("marble", "Marble", 0x6B7380),
    ("terrazzo", "Terrazzo", 0xF0561D),
    ("glass", "Glass", 0x1597E5),
    ("jelly", "Jelly", 0xA33FE6),
    ("pastel", "Pastel", 0xEE4F98),
    ("pixel", "Pixel", 0x1D1D21),
    ("lowpoly", "Low poly", INK_PURPLE),
    ("knit", "Knit", INK_PURPLE),
    ("skeleton", "Skeleton", 0x9A5E2C),
    ("astro", "Astronaut", 0x2F86F0),
    ("space", "Space", INK_RED),
    ("monster", "Monster", 0x3E9E2E),
    ("water", "Water", 0x1597E5),
    ("lava", "Lava", 0xF0561D),
    ("grass", "Grass", 0x3E9E2E),
    ("cactus", "Cactus", 0x3E9E2E),
    ("choco", "Chocolate", 0x9A5E2C),
    ("cloud", "Cloud", 0x2F86F0),
    ("mech", "Mech", 0xF0561D),
    ("neon", "Neon", 0xE0309A),
    ("leaf", "Leaf", 0x3E9E2E),
    ("cookie", "Gingerbread", 0x9A5E2C),
];

fn built_in(id: &str) -> Option<&'static BuiltIn> {
    BUILT_IN.iter().find(|b| b.id == id)
}

pub fn built_in_ids() -> Vec<String> {
    BUILT_IN.iter().map(|b| b.id.to_string()).collect()
}

pub fn custom_ids() -> Vec<String> {
    custom::all().into_iter().map(|c| c.id).collect()
}

/// Every style: built-in hands, then custom hands, then the laser dot.
pub fn all_ids() -> Vec<String> {
    let mut ids = built_in_ids();
    ids.extend(custom_ids());
    ids.push(LASER.to_string());
    ids
}

pub fn default_pinned() -> Vec<String> {
    ["lavender", "glove", "coral", "plush", "robot", LASER].iter().map(|s| s.to_string()).collect()
}

pub fn is_valid(id: &str) -> bool {
    id == LASER || built_in(id).is_some() || custom::hand(id).is_some()
}

pub fn is_custom(id: &str) -> bool {
    custom::hand(id).is_some()
}

/// "hand", "custom" or "laser", as written to `state.json`.
pub fn kind(id: &str) -> &'static str {
    if id == LASER {
        "laser"
    } else if is_custom(id) {
        "custom"
    } else {
        "hand"
    }
}

pub fn title(id: &str) -> String {
    if id == LASER {
        return "Laser dot".into();
    }
    if let Some(c) = custom::hand(id) {
        return c.name;
    }
    built_in(id).map(|b| b.title.to_string()).unwrap_or_else(|| id.to_string())
}

/// The colour a style draws in when ink is set to "match the pointer", as 0xRRGGBB.
pub fn ink_color(id: &str) -> u32 {
    if id == LASER {
        return INK_RED;
    }
    if let Some(c) = custom::hand(id) {
        return c.ink & 0xFFFFFF;
    }
    built_in(id).map(|b| b.ink).unwrap_or(INK_PURPLE)
}

/// A decoded hand picture (premultiplied RGBA) and where its parts are.
pub struct HandAsset {
    pub id: String,
    pub image: RgbaImage,
    pub metrics: handasset::Metrics,
}

struct Cache {
    hands: Vec<Arc<HandAsset>>,
    thumbs: HashMap<String, Arc<RgbaImage>>,
}

static CACHE: LazyLock<Mutex<Cache>> =
    LazyLock::new(|| Mutex::new(Cache { hands: Vec::new(), thumbs: HashMap::new() }));

const HAND_CACHE_LIMIT: usize = 2;

fn decode(id: &str) -> Option<RgbaImage> {
    if id == LASER {
        return None;
    }
    if custom::hand(id).is_some() {
        return image::open(custom::png_path(id)).ok().map(|i| i.to_rgba8());
    }
    let b = built_in(id)?;
    image::load_from_memory_with_format(b.image, image::ImageFormat::WebP).ok().map(|i| i.to_rgba8())
}

/// Loads a hand's full-size picture. The two most recently used hands stay in memory.
pub fn load_hand(id: &str) -> Option<Arc<HandAsset>> {
    {
        let mut cache = CACHE.lock().unwrap();
        if let Some(pos) = cache.hands.iter().position(|h| h.id == id) {
            let hit = cache.hands.remove(pos);
            cache.hands.push(hit.clone());
            return Some(hit);
        }
    }
    let mut image = decode(id)?;
    let metrics = handasset::measure(&image)?;
    handasset::premultiply(&mut image);
    let asset = Arc::new(HandAsset { id: id.to_string(), image, metrics });
    let mut cache = CACHE.lock().unwrap();
    cache.hands.push(asset.clone());
    while cache.hands.len() > HAND_CACHE_LIMIT {
        cache.hands.remove(0);
    }
    Some(asset)
}

/// A small square picture of a hand (premultiplied RGBA, `handasset::THUMB_SIZE` on a side).
/// Returns `None` for the laser dot, which is drawn rather than loaded.
pub fn thumbnail(id: &str) -> Option<Arc<RgbaImage>> {
    if id == LASER {
        return None;
    }
    if let Some(hit) = CACHE.lock().unwrap().thumbs.get(id) {
        return Some(hit.clone());
    }
    let thumb = if let Some(b) = built_in(id) {
        let mut t = image::load_from_memory_with_format(b.thumb, image::ImageFormat::Png).ok()?.to_rgba8();
        handasset::premultiply(&mut t);
        t
    } else {
        let asset = load_hand(id)?;
        handasset::thumbnail(&asset.image, &asset.metrics)
    };
    let thumb = Arc::new(thumb);
    CACHE.lock().unwrap().thumbs.insert(id.to_string(), thumb.clone());
    Some(thumb)
}

/// Drops everything cached for a style (after a custom hand is deleted).
pub fn forget(id: &str) {
    let mut cache = CACHE.lock().unwrap();
    cache.hands.retain(|h| h.id != id);
    cache.thumbs.remove(id);
}
