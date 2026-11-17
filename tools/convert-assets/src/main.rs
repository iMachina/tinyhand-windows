//! Converts the macOS TinyHand assets (in `tinyhand-main/`) into the files the Windows app
//! embeds (in `assets/`). Re-runnable: every output is overwritten.
//!
//! Run from anywhere: `cargo run --release --manifest-path tools/convert-assets/Cargo.toml`

#[path = "../../../src/handasset.rs"]
#[allow(dead_code)]
mod handasset;

use std::fs;
use std::io::Cursor;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Mutex;

use image::codecs::png::{CompressionType, FilterType as PngFilter, PngEncoder};
use image::imageops::{self, FilterType};
use image::{ImageEncoder, RgbaImage};

type Result<T> = std::result::Result<T, String>;

/// Lossy colour quality for the hand WebPs (0..100).
const WEBP_QUALITY: f32 = 90.0;
/// Alpha plane quality; 100 = lossless alpha.
const WEBP_ALPHA_QUALITY: i32 = 100;
/// Encoder effort (0..6).
const WEBP_METHOD: i32 = 6;
/// Largest allowed difference between a metric measured on the PNG and on the decoded WebP.
const METRIC_TOLERANCE: f32 = 0.005;

const ICO_SIZES: &[u32] = &[16, 20, 24, 32, 40, 48, 64, 128, 256];
const TRAY_SIZE: u32 = 64;

fn main() {
    if let Err(e) = run() {
        eprintln!("\nERROR: {e}");
        std::process::exit(1);
    }
}

fn run() -> Result<()> {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("..").join("..");
    let root = root.canonicalize().map_err(|e| format!("project root: {e}"))?;
    // Drop Windows' verbatim `\\?\` prefix so printed paths read normally.
    let root = match root.to_str().and_then(|s| s.strip_prefix(r"\\?\")) {
        Some(plain) => PathBuf::from(plain),
        None => root,
    };
    let src = root.join("tinyhand-main");
    let out = root.join("assets");
    if !src.is_dir() {
        return Err(format!("source folder not found: {}", src.display()));
    }
    println!("source: {}\noutput: {}\n", src.display(), out.display());

    for sub in ["hands", "thumbs", "sounds"] {
        mkdir(&out.join(sub))?;
    }

    convert_hands(&src.join("assets"), &out)?;
    convert_sounds(&src.join("assets").join("sounds"), &out.join("sounds"))?;
    convert_icon(&src, &out.join("tinyhand.ico"))?;
    convert_tray(&src.join("assets"), &out)?;

    summary(&out)
}

// ---------------------------------------------------------------------------------------------
// Hands: full-size lossy WebP + 128x128 PNG thumbnail
// ---------------------------------------------------------------------------------------------

fn convert_hands(src: &Path, out: &Path) -> Result<()> {
    let mut jobs: Vec<(String, PathBuf)> = Vec::new();
    for entry in fs::read_dir(src).map_err(|e| format!("{}: {e}", src.display()))? {
        let path = entry.map_err(|e| e.to_string())?.path();
        let name = path.file_name().and_then(|n| n.to_str()).unwrap_or("");
        if let Some(id) = name.strip_prefix("hand_").and_then(|n| n.strip_suffix(".png")) {
            jobs.push((id.to_string(), path.clone()));
        }
    }
    jobs.sort();
    if jobs.is_empty() {
        return Err(format!("no hand_*.png files in {}", src.display()));
    }
    println!(
        "Hands: {} files, WebP quality {WEBP_QUALITY}, alpha_quality {WEBP_ALPHA_QUALITY}, method {WEBP_METHOD}",
        jobs.len()
    );

    let next = AtomicUsize::new(0);
    let results: Mutex<Vec<(String, Result<(u64, u64, u64)>)>> = Mutex::new(Vec::new());
    let workers = std::thread::available_parallelism().map(|n| n.get()).unwrap_or(4).min(jobs.len());
    std::thread::scope(|s| {
        for _ in 0..workers {
            s.spawn(|| loop {
                let i = next.fetch_add(1, Ordering::SeqCst);
                let Some((id, path)) = jobs.get(i) else { break };
                let r = convert_hand(id, path, out);
                results.lock().unwrap().push((id.clone(), r));
            });
        }
    });

    let mut results = results.into_inner().unwrap();
    results.sort_by(|a, b| a.0.cmp(&b.0));
    let mut failures = Vec::new();
    let (mut png_total, mut webp_total) = (0u64, 0u64);
    println!("  {:<10} {:>10} {:>10} {:>7} {:>8}", "id", "png", "webp", "ratio", "thumb");
    for (id, r) in &results {
        match r {
            Ok((png, webp, thumb)) => {
                png_total += png;
                webp_total += webp;
                println!(
                    "  {:<10} {:>10} {:>10} {:>6.1}% {:>8}",
                    id,
                    png,
                    webp,
                    *webp as f64 * 100.0 / *png as f64,
                    thumb
                );
            }
            Err(e) => {
                println!("  {id:<10} FAILED: {e}");
                failures.push(format!("{id}: {e}"));
            }
        }
    }
    println!(
        "  total PNG {} -> WebP {} ({:.1}%)\n",
        human(png_total),
        human(webp_total),
        webp_total as f64 * 100.0 / png_total.max(1) as f64
    );
    if !failures.is_empty() {
        return Err(format!("{} hand(s) failed:\n  {}", failures.len(), failures.join("\n  ")));
    }
    Ok(())
}

/// Returns (source PNG bytes, WebP bytes, thumbnail bytes).
fn convert_hand(id: &str, path: &Path, out: &Path) -> Result<(u64, u64, u64)> {
    let png_bytes = fs::read(path).map_err(|e| format!("read: {e}"))?;
    let original = image::load_from_memory(&png_bytes).map_err(|e| format!("decode PNG: {e}"))?.to_rgba8();
    let (w, h) = original.dimensions();
    let m_orig = handasset::measure(&original).ok_or("measure failed on the original PNG")?;

    // Full-size lossy WebP with lossless alpha.
    let webp = encode_webp(&original)?;
    let decoded = image::load_from_memory(&webp).map_err(|e| format!("image crate cannot decode the WebP: {e}"))?;
    if !matches!(decoded.color(), image::ColorType::Rgba8) {
        return Err(format!("decoded WebP is {:?}, expected Rgba8", decoded.color()));
    }
    let decoded = decoded.to_rgba8();
    if decoded.dimensions() != (w, h) || (w, h) != (1024, 1536) {
        return Err(format!("size mismatch: PNG {w}x{h}, decoded WebP {:?} (expected 1024x1536)", decoded.dimensions()));
    }
    let m_webp = handasset::measure(&decoded).ok_or("measure failed on the decoded WebP")?;
    compare_metrics(&m_orig, &m_webp)?;
    write(&out.join("hands").join(format!("{id}.webp")), &webp)?;

    // Thumbnail.
    let mut pre = original.clone();
    handasset::premultiply(&mut pre);
    let m_pre = handasset::measure(&pre).ok_or("measure failed on the premultiplied PNG")?;
    let mut thumb = handasset::thumbnail(&pre, &m_pre);
    handasset::unpremultiply(&mut thumb);
    let thumb_png = encode_png(&thumb)?;
    write(&out.join("thumbs").join(format!("{id}.png")), &thumb_png)?;

    Ok((png_bytes.len() as u64, webp.len() as u64, thumb_png.len() as u64))
}

fn encode_webp(img: &RgbaImage) -> Result<Vec<u8>> {
    let mut config = webp::WebPConfig::new().map_err(|_| "WebPConfig init failed")?;
    config.lossless = 0;
    config.quality = WEBP_QUALITY;
    config.alpha_quality = WEBP_ALPHA_QUALITY;
    config.alpha_compression = 1; // compress the alpha plane losslessly
    config.alpha_filtering = 2; // best
    config.method = WEBP_METHOD;
    config.use_sharp_yuv = 1; // sharper, more accurate RGB->YUV at colour edges
    config.exact = 0; // let libwebp simplify RGB under fully transparent pixels
    let encoder = webp::Encoder::from_rgba(img.as_raw(), img.width(), img.height());
    let mem = encoder.encode_advanced(&config).map_err(|e| format!("WebP encode failed: {e:?}"))?;
    Ok(mem.to_vec())
}

fn compare_metrics(a: &handasset::Metrics, b: &handasset::Metrics) -> Result<()> {
    let fields = [
        ("aspect", a.aspect, b.aspect),
        ("hand_top", a.hand_top, b.hand_top),
        ("tip", a.tip, b.tip),
        ("wrist", a.wrist, b.wrist),
        ("rod_end", a.rod_end, b.rod_end),
        ("tip_x", a.tip_x, b.tip_x),
        ("rod_x", a.rod_x, b.rod_x),
        ("hand_min_x", a.hand_min_x, b.hand_min_x),
        ("hand_max_x", a.hand_max_x, b.hand_max_x),
    ];
    let bad: Vec<String> = fields
        .iter()
        .filter(|(_, x, y)| (x - y).abs() > METRIC_TOLERANCE)
        .map(|(n, x, y)| format!("{n}: png {x:.4} vs webp {y:.4}"))
        .collect();
    if bad.is_empty() {
        Ok(())
    } else {
        Err(format!("metrics differ after WebP round trip: {}", bad.join(", ")))
    }
}

// ---------------------------------------------------------------------------------------------
// Sounds
// ---------------------------------------------------------------------------------------------

struct WavFormat {
    tag: u16,
    sub_tag: Option<u16>,
    channels: u16,
    rate: u32,
    bits: u16,
}

impl WavFormat {
    fn effective_tag(&self) -> u16 {
        self.sub_tag.unwrap_or(self.tag)
    }
    fn kind(&self) -> &'static str {
        match self.effective_tag() {
            1 => "PCM",
            3 => "float",
            6 => "A-law",
            7 => "mu-law",
            _ => "other",
        }
    }
}

/// Parses a RIFF/WAVE file; returns the format and the byte range of the `data` chunk.
fn parse_wav(bytes: &[u8]) -> Result<(WavFormat, std::ops::Range<usize>)> {
    if bytes.len() < 12 || &bytes[0..4] != b"RIFF" || &bytes[8..12] != b"WAVE" {
        return Err("not a RIFF/WAVE file".into());
    }
    let mut fmt = None;
    let mut data = None;
    let mut pos = 12;
    while pos + 8 <= bytes.len() {
        let id = &bytes[pos..pos + 4];
        let len = u32le(bytes, pos + 4) as usize;
        let body = pos + 8;
        let end = (body + len).min(bytes.len());
        match id {
            b"fmt " => {
                if len < 16 {
                    return Err("fmt chunk too short".into());
                }
                let tag = u16le(bytes, body);
                let sub_tag = if tag == 0xFFFE && len >= 40 { Some(u16le(bytes, body + 24)) } else { None };
                fmt = Some(WavFormat {
                    tag,
                    sub_tag,
                    channels: u16le(bytes, body + 2),
                    rate: u32le(bytes, body + 4),
                    bits: u16le(bytes, body + 14),
                });
            }
            b"data" => data = Some(body..end),
            _ => {}
        }
        pos = body + len + (len & 1);
    }
    Ok((fmt.ok_or("no fmt chunk")?, data.ok_or("no data chunk")?))
}

/// Converts float or 24/32-bit integer samples to a plain 16-bit PCM WAV.
fn to_pcm16(fmt: &WavFormat, data: &[u8]) -> Result<Vec<u8>> {
    let samples: Vec<i16> = match (fmt.effective_tag(), fmt.bits) {
        (3, 32) => data
            .chunks_exact(4)
            .map(|c| (f32::from_le_bytes([c[0], c[1], c[2], c[3]]).clamp(-1.0, 1.0) * 32767.0).round() as i16)
            .collect(),
        (3, 64) => data
            .chunks_exact(8)
            .map(|c| (f64::from_le_bytes(c.try_into().unwrap()).clamp(-1.0, 1.0) * 32767.0).round() as i16)
            .collect(),
        (1, 24) => data.chunks_exact(3).map(|c| i16::from_le_bytes([c[1], c[2]])).collect(),
        (1, 32) => data.chunks_exact(4).map(|c| i16::from_le_bytes([c[2], c[3]])).collect(),
        (1, 16) => data.chunks_exact(2).map(|c| i16::from_le_bytes([c[0], c[1]])).collect(),
        (1, 8) => data.iter().map(|&b| ((b as i16) - 128) << 8).collect(),
        (t, b) => return Err(format!("cannot convert format tag {t} with {b} bits")),
    };
    let data_len = (samples.len() * 2) as u32;
    let block = fmt.channels as u32 * 2;
    let mut w = Vec::with_capacity(44 + data_len as usize);
    w.extend_from_slice(b"RIFF");
    w.extend_from_slice(&(36 + data_len).to_le_bytes());
    w.extend_from_slice(b"WAVEfmt ");
    w.extend_from_slice(&16u32.to_le_bytes());
    w.extend_from_slice(&1u16.to_le_bytes());
    w.extend_from_slice(&fmt.channels.to_le_bytes());
    w.extend_from_slice(&fmt.rate.to_le_bytes());
    w.extend_from_slice(&(fmt.rate * block).to_le_bytes());
    w.extend_from_slice(&(block as u16).to_le_bytes());
    w.extend_from_slice(&16u16.to_le_bytes());
    w.extend_from_slice(b"data");
    w.extend_from_slice(&data_len.to_le_bytes());
    for s in samples {
        w.extend_from_slice(&s.to_le_bytes());
    }
    Ok(w)
}

fn convert_sounds(src: &Path, out: &Path) -> Result<()> {
    let mut files: Vec<PathBuf> = fs::read_dir(src)
        .map_err(|e| format!("{}: {e}", src.display()))?
        .filter_map(|e| e.ok().map(|e| e.path()))
        .filter(|p| p.extension().is_some_and(|x| x.eq_ignore_ascii_case("wav")))
        .collect();
    files.sort();
    println!("Sounds: {} files", files.len());
    println!("  {:<20} {:>8} {:>4} {:>7} {:>5} {:<6} action", "file", "bytes", "ch", "rate", "bits", "kind");
    for path in &files {
        let name = path.file_name().unwrap().to_string_lossy().to_string();
        let bytes = fs::read(path).map_err(|e| format!("{name}: {e}"))?;
        let (fmt, data) = parse_wav(&bytes).map_err(|e| format!("{name}: {e}"))?;
        let plain_pcm = fmt.tag == 1 && (fmt.bits == 8 || fmt.bits == 16);
        let (output, action) = if plain_pcm {
            (bytes.clone(), "copied".to_string())
        } else {
            let converted = to_pcm16(&fmt, &bytes[data]).map_err(|e| format!("{name}: {e}"))?;
            (converted, format!("CONVERTED to 16-bit PCM (tag {:#06x})", fmt.tag))
        };
        println!(
            "  {:<20} {:>8} {:>4} {:>7} {:>5} {:<6} {}",
            name,
            bytes.len(),
            fmt.channels,
            fmt.rate,
            fmt.bits,
            fmt.kind(),
            action
        );
        write(&out.join(&name), &output)?;
    }
    println!();
    Ok(())
}

// ---------------------------------------------------------------------------------------------
// App icon
// ---------------------------------------------------------------------------------------------

/// Returns the biggest PNG stored in an .icns file, with its type code.
fn largest_icns_png(bytes: &[u8]) -> Result<Option<(String, RgbaImage)>> {
    if bytes.len() < 8 || &bytes[0..4] != b"icns" {
        return Err("not an icns file".into());
    }
    let total = (u32be(bytes, 4) as usize).min(bytes.len());
    let mut best: Option<(String, RgbaImage)> = None;
    let mut pos = 8;
    while pos + 8 <= total {
        let kind = String::from_utf8_lossy(&bytes[pos..pos + 4]).to_string();
        let len = u32be(bytes, pos + 4) as usize;
        if len < 8 || pos + len > total {
            break;
        }
        let body = &bytes[pos + 8..pos + len];
        if body.starts_with(b"\x89PNG\r\n\x1a\n") {
            if let Ok(img) = image::load_from_memory(body) {
                let img = img.to_rgba8();
                if best.as_ref().is_none_or(|(_, b)| img.width() > b.width()) {
                    best = Some((kind.clone(), img));
                }
            }
        }
        pos += len;
    }
    Ok(best)
}

fn convert_icon(src: &Path, out: &Path) -> Result<()> {
    let icns_path = src.join("assets").join("AppIcon.icns");
    let from_icns = match fs::read(&icns_path) {
        Ok(b) => largest_icns_png(&b).unwrap_or_else(|e| {
            println!("  warning: {}: {e}", icns_path.display());
            None
        }),
        Err(e) => {
            println!("  warning: {}: {e}", icns_path.display());
            None
        }
    };
    let (origin, source) = match from_icns {
        Some((kind, img)) => (format!("AppIcon.icns entry '{kind}'"), img),
        None => {
            let logo = src.join(".github").join("logo.png");
            let img = image::open(&logo).map_err(|e| format!("{}: {e}", logo.display()))?.to_rgba8();
            ("fallback .github/logo.png".to_string(), img)
        }
    };
    println!("Icon: source {} ({}x{})", origin, source.width(), source.height());

    let mut pre = source;
    handasset::premultiply(&mut pre);
    let mut pngs = Vec::new();
    for &size in ICO_SIZES {
        let mut img = imageops::resize(&pre, size, size, FilterType::Lanczos3);
        handasset::unpremultiply(&mut img);
        pngs.push((size, encode_png(&img)?));
    }

    // ICO container: ICONDIR, ICONDIRENTRY x n, then the PNG images.
    let mut ico = Vec::new();
    ico.extend_from_slice(&0u16.to_le_bytes());
    ico.extend_from_slice(&1u16.to_le_bytes());
    ico.extend_from_slice(&(pngs.len() as u16).to_le_bytes());
    let mut offset = 6 + 16 * pngs.len() as u32;
    for (size, png) in &pngs {
        let dim = if *size >= 256 { 0u8 } else { *size as u8 };
        ico.extend_from_slice(&[dim, dim, 0, 0]);
        ico.extend_from_slice(&1u16.to_le_bytes()); // planes
        ico.extend_from_slice(&32u16.to_le_bytes()); // bits per pixel
        ico.extend_from_slice(&(png.len() as u32).to_le_bytes());
        ico.extend_from_slice(&offset.to_le_bytes());
        offset += png.len() as u32;
    }
    for (_, png) in &pngs {
        ico.extend_from_slice(png);
    }
    write(out, &ico)?;
    let sizes: Vec<String> = ICO_SIZES.iter().map(|s| s.to_string()).collect();
    println!("  wrote tinyhand.ico: {} bytes, PNG entries {}\n", ico.len(), sizes.join(", "));
    Ok(())
}

// ---------------------------------------------------------------------------------------------
// Tray glyphs
// ---------------------------------------------------------------------------------------------

fn rasterize_svg(path: &Path, size: u32) -> Result<RgbaImage> {
    use resvg::{tiny_skia, usvg};
    let data = fs::read(path).map_err(|e| format!("{}: {e}", path.display()))?;
    let tree = usvg::Tree::from_data(&data, &usvg::Options::default())
        .map_err(|e| format!("{}: {e}", path.display()))?;
    let view = tree.size();
    let mut pixmap = tiny_skia::Pixmap::new(size, size).ok_or("pixmap alloc failed")?;
    // Uniform scale, centred, so the viewBox framing is kept.
    let scale = (size as f32 / view.width()).min(size as f32 / view.height());
    let dx = (size as f32 - view.width() * scale) / 2.0;
    let dy = (size as f32 - view.height() * scale) / 2.0;
    let ts = tiny_skia::Transform::from_row(scale, 0.0, 0.0, scale, dx, dy);
    resvg::render(&tree, ts, &mut pixmap.as_mut());
    let mut img = RgbaImage::from_raw(size, size, pixmap.take()).ok_or("pixmap size mismatch")?;
    handasset::unpremultiply(&mut img);
    Ok(img)
}

fn convert_tray(src: &Path, out: &Path) -> Result<()> {
    println!("Tray glyphs ({TRAY_SIZE}x{TRAY_SIZE}):");
    for (svg, fallback, name) in [
        ("menubar-off.svg", "menubar_off@3x.png", "tray_off.png"),
        ("menubar-on.svg", "menubar_on@3x.png", "tray_on.png"),
    ] {
        let svg_path = src.join("icon").join(svg);
        let (img, origin) = match rasterize_svg(&svg_path, TRAY_SIZE) {
            Ok(img) if img.pixels().any(|p| p.0[3] > 0) => (img, format!("icon/{svg} via resvg")),
            other => {
                let why = match other {
                    Ok(_) => "rendered empty".to_string(),
                    Err(e) => e,
                };
                println!("  warning: {svg}: {why}; falling back to {fallback}");
                let mut pre = image::open(src.join(fallback)).map_err(|e| format!("{fallback}: {e}"))?.to_rgba8();
                handasset::premultiply(&mut pre);
                let mut img = imageops::resize(&pre, TRAY_SIZE, TRAY_SIZE, FilterType::Lanczos3);
                handasset::unpremultiply(&mut img);
                (img, format!("{fallback} upscaled"))
            }
        };
        let png = encode_png(&img)?;
        write(&out.join(name), &png)?;
        println!("  {name:<13} {:>6} bytes  from {origin}", png.len());
    }
    println!();
    Ok(())
}

// ---------------------------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------------------------

fn encode_png(img: &RgbaImage) -> Result<Vec<u8>> {
    let mut buf = Cursor::new(Vec::new());
    PngEncoder::new_with_quality(&mut buf, CompressionType::Best, PngFilter::Adaptive)
        .write_image(img.as_raw(), img.width(), img.height(), image::ExtendedColorType::Rgba8)
        .map_err(|e| format!("PNG encode failed: {e}"))?;
    Ok(buf.into_inner())
}

fn mkdir(p: &Path) -> Result<()> {
    fs::create_dir_all(p).map_err(|e| format!("mkdir {}: {e}", p.display()))
}

fn write(p: &Path, bytes: &[u8]) -> Result<()> {
    fs::write(p, bytes).map_err(|e| format!("write {}: {e}", p.display()))
}

fn u16le(b: &[u8], i: usize) -> u16 {
    u16::from_le_bytes([b[i], b[i + 1]])
}
fn u32le(b: &[u8], i: usize) -> u32 {
    u32::from_le_bytes([b[i], b[i + 1], b[i + 2], b[i + 3]])
}
fn u32be(b: &[u8], i: usize) -> u32 {
    u32::from_be_bytes([b[i], b[i + 1], b[i + 2], b[i + 3]])
}

fn human(n: u64) -> String {
    if n >= 1 << 20 {
        format!("{:.2} MB", n as f64 / (1 << 20) as f64)
    } else {
        format!("{:.1} KB", n as f64 / 1024.0)
    }
}

fn summary(out: &Path) -> Result<()> {
    println!("Summary ({}):", out.display());
    println!("  {:<20} {:>6} {:>12} {:>10}", "output", "files", "bytes", "");
    let mut rows: Vec<(String, usize, u64)> = Vec::new();
    for sub in ["hands", "thumbs", "sounds"] {
        let dir = out.join(sub);
        let (mut n, mut total) = (0usize, 0u64);
        for e in fs::read_dir(&dir).map_err(|e| format!("{}: {e}", dir.display()))? {
            let meta = e.map_err(|e| e.to_string())?.metadata().map_err(|e| e.to_string())?;
            if meta.is_file() {
                n += 1;
                total += meta.len();
            }
        }
        rows.push((format!("assets/{sub}/"), n, total));
    }
    for f in ["tinyhand.ico", "tray_off.png", "tray_on.png"] {
        let len = fs::metadata(out.join(f)).map_err(|e| format!("{f}: {e}"))?.len();
        rows.push((format!("assets/{f}"), 1, len));
    }
    let mut grand = 0;
    for (name, n, total) in &rows {
        grand += total;
        println!("  {name:<20} {n:>6} {total:>12} {:>10}", human(*total));
    }
    println!("  {:<20} {:>6} {grand:>12} {:>10}", "total", rows.iter().map(|r| r.1).sum::<usize>(), human(grand));
    Ok(())
}
