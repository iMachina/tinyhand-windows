//! Hands the user made themselves: one PNG per hand plus a `hands.json` list, kept in
//! [`crate::paths::hands_dir`]. A port of `CustomHands` in the macOS app. Callers are
//! responsible for telling the rest of the app when the list changes.

use crate::paths;
use image::{ImageFormat, RgbaImage};
use serde::{Deserialize, Serialize};
use std::fs;
use std::io::Cursor;
use std::path::{Path, PathBuf};
use std::sync::{Mutex, MutexGuard};

/// One entry of `hands.json`.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct CustomHand {
    /// `custom-` followed by 8 lowercase hex digits; also the PNG's file name.
    pub id: String,
    /// Shown in menus, at most 16 characters.
    pub name: String,
    /// Colour for the hand in menus, as `0xRRGGBB`.
    pub ink: u32,
}

const ID_PREFIX: &str = "custom-";
const MAX_NAME_CHARS: usize = 16;

/// The list as last loaded or saved; `None` until first needed.
static LOADED: Mutex<Option<Vec<CustomHand>>> = Mutex::new(None);

fn lock() -> MutexGuard<'static, Option<Vec<CustomHand>>> {
    LOADED.lock().unwrap_or_else(|poisoned| poisoned.into_inner())
}

/// The cached list, loading it from disk first if needed.
fn cached(slot: &mut Option<Vec<CustomHand>>) -> &mut Vec<CustomHand> {
    slot.get_or_insert_with(load)
}

fn list_path() -> PathBuf {
    paths::hands_dir().join("hands.json")
}

/// Reads `hands.json`; a missing or broken file is an empty list.
fn load() -> Vec<CustomHand> {
    fs::read(list_path())
        .ok()
        .and_then(|data| serde_json::from_slice(&data).ok())
        .unwrap_or_default()
}

/// All custom hands, in the order they were added. Loaded from disk once, then cached.
pub fn all() -> Vec<CustomHand> {
    cached(&mut lock()).clone()
}

/// The custom hand with this id, if there is one. Ids not starting with `custom-` never match.
pub fn hand(id: &str) -> Option<CustomHand> {
    if !id.starts_with(ID_PREFIX) {
        return None;
    }
    cached(&mut lock()).iter().find(|h| h.id == id).cloned()
}

/// Where the picture for the hand `id` is (or would be) stored.
pub fn png_path(id: &str) -> PathBuf {
    paths::hands_dir().join(format!("{id}.png"))
}

/// Saves `image` (straight alpha) as a new custom hand and adds it to the list. A blank
/// `name` becomes "My hand N". Returns the new hand's id; the error is a message for the user.
pub fn add(image: &RgbaImage, ink: u32, name: &str) -> Result<String, String> {
    fs::create_dir_all(paths::hands_dir()).map_err(save_error)?;
    let mut png = Vec::new();
    image
        .write_to(&mut Cursor::new(&mut png), ImageFormat::Png)
        .map_err(save_error)?;

    let mut slot = lock();
    let list = cached(&mut slot);
    let id = loop {
        let id = format!("{ID_PREFIX}{}", random_hex8());
        if !list.iter().any(|h| h.id == id) && !png_path(&id).exists() {
            break id;
        }
    };
    write_atomic(&png_path(&id), &png).map_err(save_error)?;

    let mut updated = list.clone();
    updated.push(CustomHand { id: id.clone(), name: clean_name(name, list.len()), ink });
    if let Err(e) = save(&updated) {
        let _ = fs::remove_file(png_path(&id));
        return Err(save_error(e));
    }
    *slot = Some(updated);
    Ok(id)
}

/// Deletes the custom hand `id`: its picture and its entry in the list. Does nothing for ids
/// that aren't custom hands.
pub fn remove(id: &str) {
    if !id.starts_with(ID_PREFIX) || id.contains(['/', '\\', ':']) || id.contains("..") {
        return;
    }
    let _ = fs::remove_file(png_path(id));
    let mut slot = lock();
    let updated: Vec<CustomHand> = cached(&mut slot).iter().filter(|h| h.id != id).cloned().collect();
    if save(&updated).is_ok() {
        *slot = Some(updated);
    }
}

/// Forgets the cached list so the next call reads `hands.json` again.
#[cfg(test)]
pub fn reload() {
    *lock() = None;
}

/// A name for a hand made from the picture at `path`: the first two words of the file name,
/// capitalised, or "My hand N" when that is empty, longer than 16 characters, or the
/// generic "ChatGPT Image ..." name.
pub fn suggested_name(path: &Path) -> String {
    let base = path
        .file_stem()
        .map(|s| s.to_string_lossy().replace(['_', '-'], " "))
        .unwrap_or_default();
    let words = base.split_whitespace().take(2).collect::<Vec<_>>().join(" ");
    if words.is_empty()
        || words.chars().count() > MAX_NAME_CHARS
        || words.to_lowercase().starts_with("chatgpt image")
    {
        return default_name(all().len());
    }
    let mut chars = words.chars();
    match chars.next() {
        Some(first) => first.to_uppercase().chain(chars).collect(),
        None => words,
    }
}

fn default_name(count: usize) -> String {
    format!("My hand {}", count + 1)
}

/// Trims `name` and cuts it to 16 characters; blank names get the default for a list that
/// already has `count` hands.
fn clean_name(name: &str, count: usize) -> String {
    let trimmed = name.trim();
    if trimmed.is_empty() {
        return default_name(count);
    }
    trimmed.chars().take(MAX_NAME_CHARS).collect::<String>().trim_end().to_string()
}

fn save(list: &[CustomHand]) -> std::io::Result<()> {
    fs::create_dir_all(paths::hands_dir())?;
    let json = serde_json::to_vec(list).map_err(std::io::Error::other)?;
    write_atomic(&list_path(), &json)
}

/// Writes `data` to a temporary file next to `path`, then renames it over `path`.
fn write_atomic(path: &Path, data: &[u8]) -> std::io::Result<()> {
    let name = path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
    let temp = path.with_file_name(format!(".{name}.{}.tmp", random_hex8()));
    let result = fs::write(&temp, data).and_then(|()| fs::rename(&temp, path));
    if result.is_err() {
        let _ = fs::remove_file(&temp);
    }
    result
}

fn save_error(e: impl std::fmt::Display) -> String {
    format!("TinyHand couldn't save the hand. ({e})")
}

/// 8 random lowercase hex digits. `RandomState` is seeded from the operating system's
/// random source, which is plenty for picking file names.
fn random_hex8() -> String {
    use std::collections::hash_map::RandomState;
    use std::hash::{BuildHasher, Hasher};
    use std::sync::atomic::{AtomicU64, Ordering};
    use std::time::{SystemTime, UNIX_EPOCH};

    static COUNTER: AtomicU64 = AtomicU64::new(0);
    let mut hasher = RandomState::new().build_hasher();
    hasher.write_u64(COUNTER.fetch_add(1, Ordering::Relaxed));
    hasher.write_u128(SystemTime::now().duration_since(UNIX_EPOCH).map_or(0, |d| d.as_nanos()));
    hasher.write_u32(std::process::id());
    format!("{:08x}", hasher.finish() as u32)
}

#[cfg(test)]
mod tests {
    use super::*;
    use image::Rgba;

    #[test]
    fn names_are_cleaned() {
        assert_eq!(clean_name("  Lava  ", 0), "Lava");
        assert_eq!(clean_name(" \n", 2), "My hand 3");
        assert_eq!(clean_name("abcdefghijklmnopqrstuvwxyz", 0), "abcdefghijklmnop");
        // Characters, not bytes.
        assert_eq!(clean_name("ééééééééééééééééééé", 0), "éééééééééééééééé");
        assert_eq!(clean_name("abcdefghijklmno pq", 0), "abcdefghijklmno");
    }

    #[test]
    fn random_ids_look_right() {
        let a = random_hex8();
        let b = random_hex8();
        assert_eq!(a.len(), 8);
        assert!(a.chars().all(|c| c.is_ascii_hexdigit() && !c.is_ascii_uppercase()));
        assert_ne!(a, b);
    }

    /// Everything that touches `TH_HANDS_DIR` and the shared cache, in one test so nothing races.
    #[test]
    fn store_round_trip_and_suggested_names() {
        let dir = std::env::temp_dir().join(format!("tinyhand-custom-tests-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        std::env::set_var("TH_HANDS_DIR", &dir);
        reload();

        assert!(all().is_empty());
        assert_eq!(suggested_name(Path::new("C:/x/ChatGPT Image Apr 3, 2025, 10_15_02 AM.png")), "My hand 1");

        let image = RgbaImage::from_pixel(8, 12, Rgba([10, 20, 30, 128]));
        let first = add(&image, 0x123456, "  Lava hand  ").unwrap();
        assert!(first.starts_with("custom-") && first.len() == 15, "{first}");
        assert!(first[7..].chars().all(|c| matches!(c, '0'..='9' | 'a'..='f')));
        assert!(png_path(&first).exists());
        let second = add(&image, 0xABCDEF, "").unwrap();
        assert_ne!(first, second);

        let hands = all();
        assert_eq!(hands.len(), 2);
        assert_eq!(hands[0], CustomHand { id: first.clone(), name: "Lava hand".into(), ink: 0x123456 });
        assert_eq!(hands[1].name, "My hand 2");
        assert_eq!(hand(&second).unwrap().ink, 0xABCDEF);
        assert_eq!(hand("lavender"), None);
        assert_eq!(hand("custom-00000000"), None);

        // The PNG keeps straight alpha.
        let saved = image::open(png_path(&first)).unwrap().to_rgba8();
        assert_eq!(saved.get_pixel(3, 3).0, [10, 20, 30, 128]);

        // hands.json has the same shape as the macOS app's.
        let json: serde_json::Value = serde_json::from_slice(&fs::read(dir.join("hands.json")).unwrap()).unwrap();
        assert_eq!(json[0]["id"], first.as_str());
        assert_eq!(json[0]["name"], "Lava hand");
        assert_eq!(json[0]["ink"], 0x123456);
        assert_eq!(json.as_array().unwrap().len(), 2);
        let leftovers = fs::read_dir(&dir).unwrap().filter(|e| {
            e.as_ref().unwrap().file_name().to_string_lossy().ends_with(".tmp")
        });
        assert_eq!(leftovers.count(), 0);

        // The list survives a reload from disk.
        reload();
        assert_eq!(all(), hands);
        assert_eq!(suggested_name(Path::new("")), "My hand 3");

        remove(&first);
        assert!(!png_path(&first).exists());
        assert_eq!(hand(&first), None);
        reload();
        assert_eq!(all().iter().map(|h| h.id.clone()).collect::<Vec<_>>(), vec![second.clone()]);
        remove("lavender");
        remove("custom-../hands");
        assert_eq!(all().len(), 1);

        // Hand-edited file is picked up after reload; a broken one reads as empty.
        fs::write(dir.join("hands.json"), r#"[{"id":"custom-0badf00d","name":"Edited","ink":255}]"#).unwrap();
        reload();
        assert_eq!(hand("custom-0badf00d").unwrap().name, "Edited");
        fs::write(dir.join("hands.json"), "not json").unwrap();
        reload();
        assert!(all().is_empty());

        // Suggested names.
        assert_eq!(suggested_name(Path::new("C:/pics/lava_lamp_hand.png")), "Lava lamp");
        assert_eq!(suggested_name(Path::new("knitted-wool.webp")), "Knitted wool");
        assert_eq!(suggested_name(Path::new("  spaced__out  .png")), "Spaced out");
        assert_eq!(suggested_name(Path::new("mustard.png")), "Mustard");
        assert_eq!(suggested_name(Path::new("élan vital.png")), "Élan vital");
        assert_eq!(suggested_name(Path::new("abcdefghij klmnop.png")), "My hand 1");
        assert_eq!(suggested_name(Path::new("abcdefgh klmnopq.png")), "Abcdefgh klmnopq");
        assert_eq!(suggested_name(Path::new("chatgpt-image-2025.png")), "My hand 1");
        assert_eq!(suggested_name(Path::new("___.png")), "My hand 1");

        let _ = fs::remove_dir_all(&dir);
        reload();
    }
}
