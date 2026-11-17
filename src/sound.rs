//! The tap sound. Sounds are embedded WAV files played with `PlaySoundW` on a helper thread,
//! so a slow audio device can cost a late sound but never a frozen pointer.

use std::sync::mpsc::{channel, Sender};
use std::sync::OnceLock;
use windows::core::PCWSTR;
use windows::Win32::Media::Audio::{PlaySoundW, SND_ASYNC, SND_MEMORY, SND_NODEFAULT};

struct Sound {
    id: &'static str,
    title: &'static str,
    wav: &'static [u8],
}

macro_rules! sounds {
    ($(($id:literal, $title:literal)),* $(,)?) => {
        &[$(Sound { id: $id, title: $title, wav: include_bytes!(concat!("../assets/sounds/tap_", $id, ".wav")) }),*]
    };
}

static SOUNDS: &[Sound] = sounds![
    ("thock", "Thock"),
    ("click", "Click"),
    ("tock", "Tock"),
    ("clack", "Clack"),
    ("typewriter", "Typewriter"),
    ("snap", "Snap"),
    ("wood", "Wood block"),
    ("pop", "Pop"),
    ("boop", "Boop"),
    ("blip", "Blip"),
    ("coin", "Coin"),
    ("pew", "Pew"),
    ("squeak", "Squeak"),
    ("quack", "Quack"),
    ("chirp", "Chirp"),
];

/// Every tap sound as `(id, title)`, in menu order.
pub fn choices() -> Vec<(&'static str, &'static str)> {
    SOUNDS.iter().map(|s| (s.id, s.title)).collect()
}

static PLAYER: OnceLock<Sender<&'static [u8]>> = OnceLock::new();

fn player() -> &'static Sender<&'static [u8]> {
    PLAYER.get_or_init(|| {
        let (tx, rx) = channel::<&'static [u8]>();
        let _ = std::thread::Builder::new().name("TinyHand tap sound".into()).spawn(move || {
            while let Ok(mut wav) = rx.recv() {
                // If taps piled up while the device was slow, only the newest is worth playing.
                while let Ok(newer) = rx.try_recv() {
                    wav = newer;
                }
                unsafe {
                    let _ = PlaySoundW(PCWSTR(wav.as_ptr() as *const u16), None, SND_MEMORY | SND_ASYNC | SND_NODEFAULT);
                }
            }
        });
        tx
    })
}

/// Starts the sound thread ahead of the first tap.
pub fn warm() {
    let _ = player();
}

/// Plays a tap sound by id. Unknown ids fall back to the default sound.
pub fn play(id: &str) {
    let sound = SOUNDS.iter().find(|s| s.id == id).unwrap_or(&SOUNDS[0]);
    let _ = player().send(sound.wav);
}
