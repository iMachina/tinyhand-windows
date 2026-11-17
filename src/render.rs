//! Direct2D drawing for the overlay: a transparent, full-screen DirectComposition surface that
//! the hand, the ink and the toolbar are drawn onto. Also renders to memory for `--render-test`.
//!
//! All coordinates are physical pixels with the origin at the overlay's top-left corner.

use crate::arm::{HandPose, LaserPose};
use crate::ink::{Ink, Stroke};
use crate::styles::{self, HandAsset};
use image::{imageops, RgbaImage};
use std::collections::HashMap;
use std::mem::ManuallyDrop;
use std::time::Instant;
use windows::core::{Interface, Result, HSTRING};
use windows::Win32::Foundation::{HMODULE, HWND};
use windows::Win32::Graphics::Direct2D::Common::*;
use windows::Win32::Graphics::Direct2D::*;
use windows::Win32::Graphics::Direct3D::*;
use windows::Win32::Graphics::Direct3D11::*;
use windows::Win32::Graphics::DirectComposition::*;
use windows::Win32::Graphics::DirectWrite::*;
use windows::Win32::Graphics::Dxgi::Common::*;
use windows::Win32::Graphics::Dxgi::*;
use windows_numerics::{Matrix3x2, Vector2};

/// A colour with straight (not premultiplied) alpha, components 0 to 1.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Color(pub f32, pub f32, pub f32, pub f32);

impl Color {
    pub fn hex(rgb: u32, alpha: f32) -> Color {
        Color(((rgb >> 16) & 0xFF) as f32 / 255.0, ((rgb >> 8) & 0xFF) as f32 / 255.0, (rgb & 0xFF) as f32 / 255.0, alpha)
    }

    pub fn white(alpha: f32) -> Color {
        Color(1.0, 1.0, 1.0, alpha)
    }

    pub fn black(alpha: f32) -> Color {
        Color(0.0, 0.0, 0.0, alpha)
    }

    pub fn fade(self, by: f32) -> Color {
        Color(self.0, self.1, self.2, self.3 * by)
    }

    fn d2d(self) -> D2D1_COLOR_F {
        D2D1_COLOR_F { r: self.0, g: self.1, b: self.2, a: self.3 }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Font {
    Text,
    TextBold,
    /// Segoe MDL2 Assets, for the speaker and close glyphs.
    Icons,
}

pub const GLYPH_VOLUME: &str = "\u{E767}";
pub const GLYPH_MUTE: &str = "\u{E74F}";
pub const GLYPH_CLOSE: &str = "\u{E711}";

const IDENTITY: Matrix3x2 = Matrix3x2 { M11: 1.0, M12: 0.0, M21: 0.0, M22: 1.0, M31: 0.0, M32: 0.0 };

fn rect(x: f32, y: f32, w: f32, h: f32) -> D2D_RECT_F {
    D2D_RECT_F { left: x, top: y, right: x + w, bottom: y + h }
}

fn bitmap_props(format: DXGI_FORMAT, options: D2D1_BITMAP_OPTIONS) -> D2D1_BITMAP_PROPERTIES1 {
    D2D1_BITMAP_PROPERTIES1 {
        pixelFormat: D2D1_PIXEL_FORMAT { format, alphaMode: D2D1_ALPHA_MODE_PREMULTIPLIED },
        dpiX: 96.0,
        dpiY: 96.0,
        bitmapOptions: options,
        colorContext: ManuallyDrop::new(None),
    }
}

/// The composition pieces that put the swap chain on a window.
struct Presenter {
    swap: IDXGISwapChain1,
    _dcomp: IDCompositionDevice,
    _target: IDCompositionTarget,
    _visual: IDCompositionVisual,
}

pub struct Renderer {
    _d3d: ID3D11Device,
    factory: ID2D1Factory1,
    dc: ID2D1DeviceContext,
    dwrite: IDWriteFactory,
    presenter: Option<Presenter>,
    target: Option<ID2D1Bitmap1>,
    width: u32,
    height: u32,
    brush: ID2D1SolidColorBrush,
    glow: ID2D1RadialGradientBrush,
    round: ID2D1StrokeStyle,
    shadow: ID2D1Effect,
    /// The hand picture scaled for the current pointer size: (style id, pixel height, bitmap).
    hand: Option<(String, u32, ID2D1Bitmap1)>,
    /// Scratch surface the unrotated arm is assembled on.
    arm: Option<(u32, u32, ID2D1Bitmap1)>,
    thumbs: HashMap<String, ID2D1Bitmap1>,
    formats: HashMap<(Font, u32), IDWriteTextFormat>,
    drawing: bool,
}

impl Renderer {
    /// A renderer that shows on `hwnd`, which must have been created with
    /// `WS_EX_NOREDIRECTIONBITMAP`.
    pub fn for_window(hwnd: HWND, width: u32, height: u32) -> Result<Renderer> {
        Self::create(Some(hwnd), width.max(1), height.max(1))
    }

    /// A renderer that draws to memory only; read the result with `read_pixels`.
    pub fn offscreen(width: u32, height: u32) -> Result<Renderer> {
        Self::create(None, width.max(1), height.max(1))
    }

    fn create(hwnd: Option<HWND>, width: u32, height: u32) -> Result<Renderer> {
        unsafe {
            let mut d3d = None;
            let mut made = D3D11CreateDevice(
                None,
                D3D_DRIVER_TYPE_HARDWARE,
                HMODULE::default(),
                D3D11_CREATE_DEVICE_BGRA_SUPPORT,
                None,
                D3D11_SDK_VERSION,
                Some(&mut d3d),
                None,
                None,
            );
            if made.is_err() {
                // No usable GPU (remote session, broken driver): fall back to software.
                made = D3D11CreateDevice(
                    None,
                    D3D_DRIVER_TYPE_WARP,
                    HMODULE::default(),
                    D3D11_CREATE_DEVICE_BGRA_SUPPORT,
                    None,
                    D3D11_SDK_VERSION,
                    Some(&mut d3d),
                    None,
                    None,
                );
            }
            made?;
            let d3d = d3d.unwrap();
            let dxgi: IDXGIDevice = d3d.cast()?;

            let factory: ID2D1Factory1 = D2D1CreateFactory(D2D1_FACTORY_TYPE_SINGLE_THREADED, None)?;
            let device = factory.CreateDevice(&dxgi)?;
            let dc = device.CreateDeviceContext(D2D1_DEVICE_CONTEXT_OPTIONS_NONE)?;
            dc.SetDpi(96.0, 96.0);
            dc.SetUnitMode(D2D1_UNIT_MODE_PIXELS);
            let dwrite: IDWriteFactory = DWriteCreateFactory(DWRITE_FACTORY_TYPE_SHARED)?;

            let presenter = match hwnd {
                Some(hwnd) => {
                    let dxgi_factory: IDXGIFactory2 = CreateDXGIFactory2(DXGI_CREATE_FACTORY_FLAGS(0))?;
                    let desc = DXGI_SWAP_CHAIN_DESC1 {
                        Width: width,
                        Height: height,
                        Format: DXGI_FORMAT_B8G8R8A8_UNORM,
                        SampleDesc: DXGI_SAMPLE_DESC { Count: 1, Quality: 0 },
                        BufferUsage: DXGI_USAGE_RENDER_TARGET_OUTPUT,
                        BufferCount: 2,
                        Scaling: DXGI_SCALING_STRETCH,
                        SwapEffect: DXGI_SWAP_EFFECT_FLIP_SEQUENTIAL,
                        AlphaMode: DXGI_ALPHA_MODE_PREMULTIPLIED,
                        ..Default::default()
                    };
                    let swap = dxgi_factory.CreateSwapChainForComposition(&d3d, &desc, None)?;
                    let dcomp: IDCompositionDevice = DCompositionCreateDevice(&dxgi)?;
                    let target = dcomp.CreateTargetForHwnd(hwnd, true)?;
                    let visual = dcomp.CreateVisual()?;
                    visual.SetContent(&swap)?;
                    target.SetRoot(&visual)?;
                    dcomp.Commit()?;
                    Some(Presenter { swap, _dcomp: dcomp, _target: target, _visual: visual })
                }
                None => None,
            };

            let brush = dc.CreateSolidColorBrush(&Color::white(1.0).d2d(), None)?;
            let red = Color::hex(styles::INK_RED, 1.0);
            let stops = [
                D2D1_GRADIENT_STOP { position: 0.0, color: red.fade(0.75).d2d() },
                D2D1_GRADIENT_STOP { position: 0.3, color: red.fade(0.5).d2d() },
                D2D1_GRADIENT_STOP { position: 0.6, color: red.fade(0.18).d2d() },
                D2D1_GRADIENT_STOP { position: 1.0, color: red.fade(0.0).d2d() },
            ];
            let stop_collection = dc.cast::<ID2D1RenderTarget>()?.CreateGradientStopCollection(&stops, D2D1_GAMMA_2_2, D2D1_EXTEND_MODE_CLAMP)?;
            let glow = dc.CreateRadialGradientBrush(
                &D2D1_RADIAL_GRADIENT_BRUSH_PROPERTIES {
                    center: Vector2 { X: 0.0, Y: 0.0 },
                    gradientOriginOffset: Vector2 { X: 0.0, Y: 0.0 },
                    radiusX: 1.0,
                    radiusY: 1.0,
                },
                None,
                &stop_collection,
            )?;
            let round = factory.CreateStrokeStyle(
                &D2D1_STROKE_STYLE_PROPERTIES1 {
                    transformType: D2D1_STROKE_TRANSFORM_TYPE_NORMAL,
                    startCap: D2D1_CAP_STYLE_ROUND,
                    endCap: D2D1_CAP_STYLE_ROUND,
                    dashCap: D2D1_CAP_STYLE_ROUND,
                    lineJoin: D2D1_LINE_JOIN_ROUND,
                    miterLimit: 10.0,
                    dashStyle: D2D1_DASH_STYLE_SOLID,
                    dashOffset: 0.0,
                },
                None,
            )?;
            let shadow = dc.CreateEffect(&CLSID_D2D1Shadow)?;

            let mut renderer = Renderer {
                _d3d: d3d,
                factory,
                dc,
                dwrite,
                presenter,
                target: None,
                width,
                height,
                brush,
                glow,
                round: round.cast()?,
                shadow,
                hand: None,
                arm: None,
                thumbs: HashMap::new(),
                formats: HashMap::new(),
                drawing: false,
            };
            renderer.make_target()?;
            Ok(renderer)
        }
    }

    fn make_target(&mut self) -> Result<()> {
        unsafe {
            let target = match &self.presenter {
                Some(p) => {
                    let surface: IDXGISurface = p.swap.GetBuffer(0)?;
                    let props =
                        bitmap_props(DXGI_FORMAT_B8G8R8A8_UNORM, D2D1_BITMAP_OPTIONS_TARGET | D2D1_BITMAP_OPTIONS_CANNOT_DRAW);
                    self.dc.CreateBitmapFromDxgiSurface(&surface, Some(&props))?
                }
                None => {
                    let props = bitmap_props(DXGI_FORMAT_B8G8R8A8_UNORM, D2D1_BITMAP_OPTIONS_TARGET);
                    self.dc.CreateBitmap(D2D_SIZE_U { width: self.width, height: self.height }, None, 0, &props)?
                }
            };
            self.target = Some(target);
        }
        Ok(())
    }

    /// Changes the surface size, for when the overlay moves to a different monitor.
    pub fn resize(&mut self, width: u32, height: u32) -> Result<()> {
        let (width, height) = (width.max(1), height.max(1));
        if (width, height) == (self.width, self.height) {
            return Ok(());
        }
        unsafe {
            self.dc.SetTarget(None);
            self.target = None;
            if let Some(p) = &self.presenter {
                p.swap.ResizeBuffers(0, width, height, DXGI_FORMAT_UNKNOWN, DXGI_SWAP_CHAIN_FLAG(0))?;
            }
        }
        self.width = width;
        self.height = height;
        self.make_target()
    }

    /// Starts a frame on a fully transparent surface.
    pub fn begin(&mut self) {
        unsafe {
            if let Some(target) = &self.target {
                self.dc.SetTarget(target);
            }
            self.dc.BeginDraw();
            self.dc.SetTransform(&IDENTITY);
            self.dc.Clear(Some(&Color::black(0.0).d2d()));
        }
        self.drawing = true;
    }

    /// Finishes the frame and shows it. On error the renderer must be thrown away and rebuilt.
    pub fn end(&mut self) -> Result<()> {
        self.drawing = false;
        unsafe {
            self.dc.EndDraw(None, None)?;
            if let Some(p) = &self.presenter {
                p.swap.Present(1, DXGI_PRESENT(0)).ok()?;
            }
        }
        Ok(())
    }

    /// The last frame as premultiplied RGBA. Only for offscreen renderers.
    pub fn read_pixels(&mut self) -> Result<RgbaImage> {
        unsafe {
            let props =
                bitmap_props(DXGI_FORMAT_B8G8R8A8_UNORM, D2D1_BITMAP_OPTIONS_CPU_READ | D2D1_BITMAP_OPTIONS_CANNOT_DRAW);
            let copy = self.dc.CreateBitmap(D2D_SIZE_U { width: self.width, height: self.height }, None, 0, &props)?;
            copy.CopyFromBitmap(None, self.target.as_ref().unwrap(), None)?;
            let mapped = copy.Map(D2D1_MAP_OPTIONS_READ)?;
            let mut out = RgbaImage::new(self.width, self.height);
            for y in 0..self.height as usize {
                let row = std::slice::from_raw_parts(mapped.bits.add(y * mapped.pitch as usize), self.width as usize * 4);
                for x in 0..self.width as usize {
                    let p = &row[x * 4..x * 4 + 4];
                    out.put_pixel(x as u32, y as u32, image::Rgba([p[2], p[1], p[0], p[3]]));
                }
            }
            copy.Unmap()?;
            Ok(out)
        }
    }

    fn bitmap_from(&self, image: &RgbaImage) -> Result<ID2D1Bitmap1> {
        let props = bitmap_props(DXGI_FORMAT_R8G8B8A8_UNORM, D2D1_BITMAP_OPTIONS_NONE);
        unsafe {
            self.dc.CreateBitmap(
                D2D_SIZE_U { width: image.width(), height: image.height() },
                Some(image.as_raw().as_ptr() as *const _),
                image.width() * 4,
                &props,
            )
        }
    }

    fn set_color(&self, color: Color) -> &ID2D1SolidColorBrush {
        unsafe { self.brush.SetColor(&color.d2d()) };
        &self.brush
    }

    // ---- Shapes ----------------------------------------------------------------------

    pub fn fill_rect(&mut self, x: f32, y: f32, w: f32, h: f32, color: Color) {
        unsafe { self.dc.FillRectangle(&rect(x, y, w, h), self.set_color(color)) }
    }

    pub fn fill_round_rect(&mut self, x: f32, y: f32, w: f32, h: f32, radius: f32, color: Color) {
        let rr = D2D1_ROUNDED_RECT { rect: rect(x, y, w, h), radiusX: radius, radiusY: radius };
        unsafe { self.dc.FillRoundedRectangle(&rr, self.set_color(color)) }
    }

    pub fn stroke_round_rect(&mut self, x: f32, y: f32, w: f32, h: f32, radius: f32, line: f32, color: Color) {
        let rr = D2D1_ROUNDED_RECT { rect: rect(x, y, w, h), radiusX: radius, radiusY: radius };
        unsafe { self.dc.DrawRoundedRectangle(&rr, self.set_color(color), line, None) }
    }

    pub fn fill_ellipse(&mut self, cx: f32, cy: f32, rx: f32, ry: f32, color: Color) {
        let e = D2D1_ELLIPSE { point: Vector2 { X: cx, Y: cy }, radiusX: rx, radiusY: ry };
        unsafe { self.dc.FillEllipse(&e, self.set_color(color)) }
    }

    /// A filled pie slice. Angles are in degrees, clockwise from straight up.
    pub fn fill_pie(&mut self, cx: f32, cy: f32, radius: f32, from_deg: f32, to_deg: f32, color: Color) {
        let at = |deg: f32| {
            let a = deg.to_radians();
            Vector2 { X: cx + radius * a.sin(), Y: cy - radius * a.cos() }
        };
        unsafe {
            let Ok(geometry) = self.factory.CreatePathGeometry() else { return };
            let Ok(sink) = geometry.Open() else { return };
            sink.BeginFigure(Vector2 { X: cx, Y: cy }, D2D1_FIGURE_BEGIN_FILLED);
            sink.AddLine(at(from_deg));
            sink.AddArc(&D2D1_ARC_SEGMENT {
                point: at(to_deg),
                size: D2D_SIZE_F { width: radius, height: radius },
                rotationAngle: 0.0,
                sweepDirection: D2D1_SWEEP_DIRECTION_CLOCKWISE,
                arcSize: D2D1_ARC_SIZE_SMALL,
            });
            sink.EndFigure(D2D1_FIGURE_END_CLOSED);
            if sink.Close().is_ok() {
                self.dc.FillGeometry(&geometry, self.set_color(color), None);
            }
        }
    }

    // ---- Text ------------------------------------------------------------------------

    fn format(&mut self, font: Font, size: f32) -> Option<IDWriteTextFormat> {
        let key = (font, (size * 4.0).round() as u32);
        if let Some(f) = self.formats.get(&key) {
            return Some(f.clone());
        }
        let (family, weight) = match font {
            Font::Text => ("Segoe UI", DWRITE_FONT_WEIGHT_NORMAL),
            Font::TextBold => ("Segoe UI", DWRITE_FONT_WEIGHT_SEMI_BOLD),
            Font::Icons => ("Segoe MDL2 Assets", DWRITE_FONT_WEIGHT_NORMAL),
        };
        let format = unsafe {
            self.dwrite
                .CreateTextFormat(
                    &HSTRING::from(family),
                    None,
                    weight,
                    DWRITE_FONT_STYLE_NORMAL,
                    DWRITE_FONT_STRETCH_NORMAL,
                    size,
                    &HSTRING::from("en-us"),
                )
                .ok()?
        };
        unsafe {
            let _ = format.SetWordWrapping(DWRITE_WORD_WRAPPING_NO_WRAP);
        }
        self.formats.insert(key, format.clone());
        Some(format)
    }

    fn layout(&mut self, text: &str, font: Font, size: f32) -> Option<(IDWriteTextLayout, f32, f32)> {
        let format = self.format(font, size)?;
        let wide: Vec<u16> = text.encode_utf16().collect();
        unsafe {
            let layout = self.dwrite.CreateTextLayout(&wide, &format, 10000.0, 10000.0).ok()?;
            let mut m = DWRITE_TEXT_METRICS::default();
            layout.GetMetrics(&mut m).ok()?;
            Some((layout, m.widthIncludingTrailingWhitespace, m.height))
        }
    }

    /// Width and height of a single line of text.
    pub fn measure_text(&mut self, text: &str, font: Font, size: f32) -> (f32, f32) {
        self.layout(text, font, size).map(|(_, w, h)| (w, h)).unwrap_or((0.0, 0.0))
    }

    /// Draws one line of text with its top-left corner at `(x, y)`. Returns its width.
    pub fn text(&mut self, text: &str, font: Font, size: f32, x: f32, y: f32, color: Color) -> f32 {
        let Some((layout, w, _)) = self.layout(text, font, size) else { return 0.0 };
        unsafe {
            self.brush.SetColor(&color.d2d());
            self.dc.DrawTextLayout(Vector2 { X: x, Y: y }, &layout, &self.brush, D2D1_DRAW_TEXT_OPTIONS_NONE);
        }
        w
    }

    /// Draws one line of text centred on `(cx, cy)`.
    pub fn text_centered(&mut self, text: &str, font: Font, size: f32, cx: f32, cy: f32, color: Color) {
        let Some((layout, w, h)) = self.layout(text, font, size) else { return };
        unsafe {
            self.brush.SetColor(&color.d2d());
            let origin = Vector2 { X: (cx - w / 2.0).round(), Y: (cy - h / 2.0).round() };
            self.dc.DrawTextLayout(origin, &layout, &self.brush, D2D1_DRAW_TEXT_OPTIONS_NONE);
        }
    }

    // ---- Pictures --------------------------------------------------------------------

    /// Draws a style's thumbnail in a square. Does nothing for the laser dot.
    pub fn draw_thumb(&mut self, style: &str, x: f32, y: f32, side: f32, opacity: f32) {
        if !self.thumbs.contains_key(style) {
            let Some(image) = styles::thumbnail(style) else { return };
            let Ok(bitmap) = self.bitmap_from(&image) else { return };
            self.thumbs.insert(style.to_string(), bitmap);
        }
        let bitmap = &self.thumbs[style];
        unsafe {
            self.dc.DrawBitmap(
                bitmap,
                Some(&rect(x, y, side, side)),
                opacity,
                D2D1_INTERPOLATION_MODE_HIGH_QUALITY_CUBIC,
                None,
                None,
            );
        }
    }

    /// Forgets a cached thumbnail (after a custom hand is deleted or replaced).
    pub fn forget_thumb(&mut self, style: &str) {
        self.thumbs.remove(style);
        if matches!(&self.hand, Some((id, _, _)) if id == style) {
            self.hand = None;
        }
    }

    /// The hand picture scaled down to roughly the size it is shown at, so the GPU never
    /// has to shrink it by much. Rebuilt only when the size moves to another 64-pixel step.
    fn hand_bitmap(&mut self, asset: &HandAsset, image_h: f32) -> Option<(ID2D1Bitmap1, f32, f32)> {
        let full = asset.image.height();
        let step = 64u32;
        let needed = (image_h.ceil().max(1.0) as u32).div_ceil(step) * step;
        let height = if needed >= full * 9 / 10 { full } else { needed.max(step) };
        let fresh = !matches!(&self.hand, Some((id, h, _)) if *id == asset.id && *h == height);
        if fresh {
            let bitmap = if height == full {
                self.bitmap_from(&asset.image).ok()?
            } else {
                let width = ((asset.image.width() as f32 * height as f32 / full as f32).round() as u32).max(1);
                let small = imageops::resize(&asset.image, width, height, imageops::FilterType::CatmullRom);
                self.bitmap_from(&small).ok()?
            };
            self.hand = Some((asset.id.clone(), height, bitmap));
        }
        let (_, _, bitmap) = self.hand.as_ref()?;
        let size = unsafe { bitmap.GetPixelSize() };
        Some((bitmap.clone(), size.width as f32, size.height as f32))
    }

    fn arm_surface(&mut self, width: u32, height: u32) -> Option<ID2D1Bitmap1> {
        let big_enough = matches!(&self.arm, Some((w, h, _)) if *w >= width && *h >= height);
        if !big_enough {
            let (old_w, old_h) = self.arm.as_ref().map(|(w, h, _)| (*w, *h)).unwrap_or((0, 0));
            let w = width.max(old_w).div_ceil(128) * 128;
            let h = height.max(old_h).div_ceil(256) * 256;
            let props = bitmap_props(DXGI_FORMAT_B8G8R8A8_UNORM, D2D1_BITMAP_OPTIONS_TARGET);
            let bitmap = unsafe { self.dc.CreateBitmap(D2D_SIZE_U { width: w, height: h }, None, 0, &props).ok()? };
            self.arm = Some((w, h, bitmap));
        }
        self.arm.as_ref().map(|(_, _, b)| b.clone())
    }

    /// Draws the hand on its stick, with its shadow.
    pub fn draw_hand(&mut self, asset: &HandAsset, pose: &HandPose) {
        let m = &asset.metrics;
        let Some((picture, pw, ph)) = self.hand_bitmap(asset, pose.image_h) else { return };
        let total = pose.hand_h + pose.rod_len;
        let aw = pose.width.ceil() as u32 + 2;
        let ah = total.ceil() as u32 + 2;
        let Some(arm) = self.arm_surface(aw, ah) else { return };
        let transform = Matrix3x2 {
            M11: pose.m[0],
            M12: pose.m[1],
            M21: pose.m[2],
            M22: pose.m[3],
            M31: pose.m[4],
            M32: pose.m[5],
        };
        unsafe {
            // Assemble the upright arm: the stick stretched to length, the hand on top of it.
            self.dc.SetTarget(&arm);
            self.dc.SetTransform(&IDENTITY);
            self.dc.Clear(Some(&Color::black(0.0).d2d()));
            let rod_src = D2D_RECT_F { left: 0.0, top: m.wrist * ph, right: pw, bottom: m.rod_end * ph };
            let rod_dst = D2D_RECT_F { left: 0.0, top: pose.hand_h - 1.0, right: pose.width, bottom: total };
            self.dc.DrawBitmap(&picture, Some(&rod_dst), 1.0, D2D1_INTERPOLATION_MODE_LINEAR, Some(&rod_src), None);
            let hand_src = D2D_RECT_F { left: 0.0, top: m.hand_top * ph, right: pw, bottom: m.wrist * ph };
            let hand_dst = D2D_RECT_F { left: 0.0, top: 0.0, right: pose.width, bottom: pose.hand_h };
            self.dc.DrawBitmap(&picture, Some(&hand_dst), 1.0, D2D1_INTERPOLATION_MODE_LINEAR, Some(&hand_src), None);
            if let Some(target) = &self.target {
                self.dc.SetTarget(target);
            }

            let used = D2D_RECT_F { left: 0.0, top: 0.0, right: aw as f32, bottom: ah as f32 };
            if pose.shadow_opacity > 0.0 {
                self.shadow.SetInput(0, &arm, true);
                let blur = (pose.shadow_blur / 2.0).max(0.5);
                let _ = self.shadow.SetValue(
                    D2D1_SHADOW_PROP_BLUR_STANDARD_DEVIATION.0 as u32,
                    D2D1_PROPERTY_TYPE_FLOAT,
                    &blur.to_ne_bytes(),
                );
                let color = [0.0f32, 0.0, 0.0, pose.shadow_opacity];
                let bytes: Vec<u8> = color.iter().flat_map(|c| c.to_ne_bytes()).collect();
                let _ = self.shadow.SetValue(D2D1_SHADOW_PROP_COLOR.0 as u32, D2D1_PROPERTY_TYPE_VECTOR4, &bytes);
                let mut shifted = transform;
                shifted.M31 += pose.shadow_dx;
                shifted.M32 += pose.shadow_dy;
                self.dc.SetTransform(&shifted);
                if let Ok(output) = self.shadow.GetOutput() {
                    let pad = blur * 3.0 + 2.0;
                    let area = D2D_RECT_F { left: -pad, top: -pad, right: used.right + pad, bottom: used.bottom + pad };
                    self.dc.DrawImage(
                        &output,
                        None,
                        Some(&area),
                        D2D1_INTERPOLATION_MODE_LINEAR,
                        D2D1_COMPOSITE_MODE_SOURCE_OVER,
                    );
                }
                self.shadow.SetInput(0, None, true);
            }

            self.dc.SetTransform(&transform);
            self.dc.DrawBitmap(&arm, Some(&used), 1.0, D2D1_INTERPOLATION_MODE_LINEAR, Some(&used), None);
            self.dc.SetTransform(&IDENTITY);
        }
    }

    /// Draws the glowing laser dot.
    pub fn draw_laser(&mut self, pose: &LaserPose) {
        let reach = pose.radius + pose.glow;
        unsafe {
            self.glow.SetTransform(&Matrix3x2 { M11: reach, M12: 0.0, M21: 0.0, M22: reach, M31: pose.x, M32: pose.y });
            let e = D2D1_ELLIPSE { point: Vector2 { X: pose.x, Y: pose.y }, radiusX: reach, radiusY: reach };
            self.dc.FillEllipse(&e, &self.glow);
        }
        self.fill_ellipse(pose.x, pose.y, pose.radius, pose.radius, Color::hex(styles::INK_RED, 1.0));
        let core = pose.radius * 0.45;
        self.fill_ellipse(pose.x, pose.y, core, core, Color(1.0, 0.86, 0.82, 0.95));
    }

    /// The small laser-dot picture used on buttons, centred on `(cx, cy)`.
    pub fn draw_laser_icon(&mut self, cx: f32, cy: f32, side: f32, opacity: f32) {
        let k = side / 64.0;
        self.fill_ellipse(cx, cy, 18.0 * k, 18.0 * k, Color::hex(styles::INK_RED, 0.22 * opacity));
        self.fill_ellipse(cx, cy, 12.0 * k, 12.0 * k, Color::hex(styles::INK_RED, 0.3 * opacity));
        self.fill_ellipse(cx, cy, 7.0 * k, 7.0 * k, Color::hex(styles::INK_RED, opacity));
        self.fill_ellipse(cx, cy, 3.0 * k, 3.0 * k, Color(1.0, 0.86, 0.82, 0.95 * opacity));
    }

    // ---- Ink -------------------------------------------------------------------------

    fn stroke_geometry(&self, stroke: &Stroke) -> Option<ID2D1PathGeometry1> {
        let v = |p: (f32, f32)| Vector2 { X: p.0, Y: p.1 };
        unsafe {
            let geometry = self.factory.CreatePathGeometry().ok()?;
            let sink = geometry.Open().ok()?;
            let points = &stroke.points;
            sink.BeginFigure(v(points[0]), D2D1_FIGURE_BEGIN_HOLLOW);
            for i in 1..points.len() {
                let prev = points[i - 1];
                let p = points[i];
                let mid = ((prev.0 + p.0) / 2.0, (prev.1 + p.1) / 2.0);
                sink.AddQuadraticBezier(&D2D1_QUADRATIC_BEZIER_SEGMENT { point1: v(prev), point2: v(mid) });
            }
            sink.AddLine(v(*points.last()?));
            sink.EndFigure(D2D1_FIGURE_END_OPEN);
            if let Some(barbs) = stroke.arrow_head() {
                for (from, to) in barbs {
                    sink.BeginFigure(v(from), D2D1_FIGURE_BEGIN_HOLLOW);
                    sink.AddLine(v(to));
                    sink.EndFigure(D2D1_FIGURE_END_OPEN);
                }
            }
            sink.Close().ok()?;
            Some(geometry)
        }
    }

    /// Draws every line, each with a faint drop shadow, fading the ones on their way out.
    pub fn draw_ink(&mut self, ink: &mut Ink, now: Instant) {
        for stroke in ink.strokes.iter_mut() {
            if stroke.points.is_empty() {
                continue;
            }
            let opacity = stroke.opacity(now);
            if opacity <= 0.0 {
                continue;
            }
            let color = Color::hex(stroke.color, opacity);
            if stroke.points.len() == 1 {
                // A press that hasn't moved yet: a round dot.
                let (x, y) = stroke.points[0];
                let r = stroke.width / 2.0;
                self.fill_ellipse(x + 1.0, y + 1.5, r, r, Color::black(0.18 * opacity));
                self.fill_ellipse(x, y, r, r, color);
                continue;
            }
            if stroke.geometry.is_none() {
                stroke.geometry = self.stroke_geometry(stroke);
            }
            let Some(geometry) = &stroke.geometry else { continue };
            unsafe {
                self.dc.SetTransform(&Matrix3x2 { M31: 1.0, M32: 1.5, ..IDENTITY });
                self.brush.SetColor(&Color::black(0.18 * opacity).d2d());
                self.dc.DrawGeometry(geometry, &self.brush, stroke.width, &self.round);
                self.dc.SetTransform(&IDENTITY);
                self.brush.SetColor(&color.d2d());
                self.dc.DrawGeometry(geometry, &self.brush, stroke.width, &self.round);
            }
        }
    }
}
