# convert-assets

A one-off tool that turns the macOS TinyHand art in `tinyhand-main/` into the files the Windows app embeds, under `assets/`:
hand pictures as lossy WebP (quality 90, lossless alpha), 128x128 PNG thumbnails, the tap sounds (checked to be plain PCM WAV),
`tinyhand.ico` (made from `AppIcon.icns`), and the 64x64 tray glyphs (rendered from the menu-bar SVGs).
It checks every WebP decodes with the `image` crate and measures the same as its source PNG, and stops with an error if not.

Run it from the project root:

    cargo run --release --manifest-path tools/convert-assets/Cargo.toml

The outputs under `assets/` are checked in, so you only need to run this again when the source art changes. It overwrites existing outputs.
It shares `src/handasset.rs` with the app, so the thumbnails are cut exactly the way the app measures hands.
