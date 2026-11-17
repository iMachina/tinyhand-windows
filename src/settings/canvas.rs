//! Direct2D and DirectWrite drawing for the settings window.
//!
//! A [`Canvas`] wraps one `ID2D1RenderTarget`: either a window's (`ID2D1HwndRenderTarget`) or
//! a WIC bitmap's for `--settings-test`. Every coordinate is in DIPs (1/96 inch); the window's
//! target is told the monitor DPI, so Direct2D does the scaling and nothing here needs to.

use image::RgbaImage;
use std::collections::HashMap;
use windows::core::{Interface, Result, HSTRING};
use windows::Win32::Foundation::HWND;
use windows::Win32::Graphics::Direct2D::Common::*;
use windows::Win32::Graphics::Direct2D::*;
use windows::Win32::Graphics::DirectWrite::*;
use windows::Win32::Graphics::Dxgi::Common::*;
use windows::Win32::Graphics::Imaging::IWICBitmap;
use windows_numerics::{Matrix3x2, Vector2};

/// A colour with straight alpha, components 0 to 1.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Color {
    pub r: f32,
    pub g: f32,
    pub b: f32,
    pub a: f32,
}

impl Color {
    /// An opaque colour from `0xRRGGBB`.
    pub const fn rgb(hex: u32) -> Color {
        Color {
            r: ((hex >> 16) & 0xFF) as f32 / 255.0,
            g: ((hex >> 8) & 0xFF) as f32 / 255.0,
            b: (hex & 0xFF) as f32 / 255.0,
            a: 1.0,
        }
    }

    /// `0xRRGGBB` with an alpha.
    pub const fn rgba(hex: u32, a: f32) -> Color {
        let c = Color::rgb(hex);
        Color { a, ..c }
    }

    /// The same colour with its alpha multiplied by `by`.
    pub fn fade(self, by: f32) -> Color {
        Color { a: self.a * by, ..self }
    }

    /// `0x00BBGGRR`, the byte order GDI wants.
    pub fn colorref(self) -> u32 {
        let c = |v: f32| (v.clamp(0.0, 1.0) * 255.0).round() as u32;
        c(self.r) | (c(self.g) << 8) | (c(self.b) << 16)
    }

    fn d2d(self) -> D2D1_COLOR_F {
        D2D1_COLOR_F { r: self.r, g: self.g, b: self.b, a: self.a }
    }
}

/// An axis-aligned rectangle in DIPs.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Rect {
    pub x: f32,
    pub y: f32,
    pub w: f32,
    pub h: f32,
}

impl Rect {
    pub const fn new(x: f32, y: f32, w: f32, h: f32) -> Rect {
        Rect { x, y, w, h }
    }

    pub fn right(&self) -> f32 {
        self.x + self.w
    }

    pub fn bottom(&self) -> f32 {
        self.y + self.h
    }

    pub fn cx(&self) -> f32 {
        self.x + self.w / 2.0
    }

    pub fn cy(&self) -> f32 {
        self.y + self.h / 2.0
    }

    pub fn contains(&self, x: f32, y: f32) -> bool {
        x >= self.x && x < self.right() && y >= self.y && y < self.bottom()
    }

    /// Shrinks the rectangle by `dx` on the left and right and `dy` on the top and bottom.
    pub fn inset(&self, dx: f32, dy: f32) -> Rect {
        Rect::new(self.x + dx, self.y + dy, (self.w - 2.0 * dx).max(0.0), (self.h - 2.0 * dy).max(0.0))
    }

    /// The overlap of two rectangles, if they overlap at all.
    pub fn intersect(&self, other: &Rect) -> Option<Rect> {
        let x = self.x.max(other.x);
        let y = self.y.max(other.y);
        let r = self.right().min(other.right());
        let b = self.bottom().min(other.bottom());
        (r > x && b > y).then(|| Rect::new(x, y, r - x, b - y))
    }

    /// True if `other` lies entirely inside this rectangle.
    pub fn encloses(&self, other: &Rect) -> bool {
        other.x >= self.x && other.y >= self.y && other.right() <= self.right() && other.bottom() <= self.bottom()
    }

    fn d2d(&self) -> D2D_RECT_F {
        D2D_RECT_F { left: self.x, top: self.y, right: self.right(), bottom: self.bottom() }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Font {
    Regular,
    Semibold,
    /// Segoe MDL2 Assets, for the small icons.
    Icons,
}

/// Horizontal placement of text inside a rectangle.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Align {
    Start,
    Center,
    End,
}

const IDENTITY: Matrix3x2 = Matrix3x2 { M11: 1.0, M12: 0.0, M21: 0.0, M22: 1.0, M31: 0.0, M32: 0.0 };

/// Direct2D's "the device went away, build everything again" error.
pub const RECREATE_TARGET: i32 = 0x8899000Cu32 as i32;

pub struct Canvas {
    factory: ID2D1Factory,
    dwrite: IDWriteFactory,
    target: ID2D1RenderTarget,
    /// The same target as a device context, when available, for better image scaling.
    dc: Option<ID2D1DeviceContext>,
    hwnd_target: Option<ID2D1HwndRenderTarget>,
    brush: ID2D1SolidColorBrush,
    dashed: ID2D1StrokeStyle,
    formats: HashMap<(Font, u32), IDWriteTextFormat>,
    bitmaps: HashMap<String, ID2D1Bitmap>,
}

fn factories() -> Result<(ID2D1Factory, IDWriteFactory)> {
    unsafe {
        let factory: ID2D1Factory = D2D1CreateFactory(D2D1_FACTORY_TYPE_SINGLE_THREADED, None)?;
        let dwrite: IDWriteFactory = DWriteCreateFactory(DWRITE_FACTORY_TYPE_SHARED)?;
        Ok((factory, dwrite))
    }
}

impl Canvas {
    /// A canvas that draws on a window. `width` and `height` are the client size in pixels.
    pub fn for_window(hwnd: HWND, width: u32, height: u32, dpi: f32) -> Result<Canvas> {
        let (factory, dwrite) = factories()?;
        unsafe {
            let props = D2D1_RENDER_TARGET_PROPERTIES {
                r#type: D2D1_RENDER_TARGET_TYPE_DEFAULT,
                pixelFormat: D2D1_PIXEL_FORMAT { format: DXGI_FORMAT_B8G8R8A8_UNORM, alphaMode: D2D1_ALPHA_MODE_IGNORE },
                dpiX: dpi,
                dpiY: dpi,
                usage: D2D1_RENDER_TARGET_USAGE_NONE,
                minLevel: D2D1_FEATURE_LEVEL_DEFAULT,
            };
            let hwnd_props = D2D1_HWND_RENDER_TARGET_PROPERTIES {
                hwnd,
                pixelSize: D2D_SIZE_U { width: width.max(1), height: height.max(1) },
                presentOptions: D2D1_PRESENT_OPTIONS_NONE,
            };
            let hwnd_target = factory.CreateHwndRenderTarget(&props, &hwnd_props)?;
            let target: ID2D1RenderTarget = hwnd_target.cast()?;
            Self::finish(factory, dwrite, target, Some(hwnd_target))
        }
    }

    /// A canvas that draws into a WIC bitmap (32bpp premultiplied BGRA) at 96 DPI.
    pub fn for_wic(bitmap: &IWICBitmap) -> Result<Canvas> {
        let (factory, dwrite) = factories()?;
        unsafe {
            let props = D2D1_RENDER_TARGET_PROPERTIES {
                r#type: D2D1_RENDER_TARGET_TYPE_DEFAULT,
                pixelFormat: D2D1_PIXEL_FORMAT {
                    format: DXGI_FORMAT_B8G8R8A8_UNORM,
                    alphaMode: D2D1_ALPHA_MODE_PREMULTIPLIED,
                },
                dpiX: 96.0,
                dpiY: 96.0,
                usage: D2D1_RENDER_TARGET_USAGE_NONE,
                minLevel: D2D1_FEATURE_LEVEL_DEFAULT,
            };
            let target = factory.CreateWicBitmapRenderTarget(bitmap, &props)?;
            // ClearType needs to know the pixel layout of a real screen; greyscale reads well anywhere.
            target.SetTextAntialiasMode(D2D1_TEXT_ANTIALIAS_MODE_GRAYSCALE);
            Self::finish(factory, dwrite, target, None)
        }
    }

    fn finish(
        factory: ID2D1Factory,
        dwrite: IDWriteFactory,
        target: ID2D1RenderTarget,
        hwnd_target: Option<ID2D1HwndRenderTarget>,
    ) -> Result<Canvas> {
        unsafe {
            let brush = target.CreateSolidColorBrush(&Color::rgb(0).d2d(), None)?;
            let dashed = factory.CreateStrokeStyle(
                &D2D1_STROKE_STYLE_PROPERTIES {
                    startCap: D2D1_CAP_STYLE_FLAT,
                    endCap: D2D1_CAP_STYLE_FLAT,
                    dashCap: D2D1_CAP_STYLE_FLAT,
                    lineJoin: D2D1_LINE_JOIN_ROUND,
                    miterLimit: 10.0,
                    dashStyle: D2D1_DASH_STYLE_CUSTOM,
                    dashOffset: 0.0,
                },
                Some(&[4.0, 3.0]),
            )?;
            let dc = target.cast::<ID2D1DeviceContext>().ok();
            Ok(Canvas {
                factory,
                dwrite,
                target,
                dc,
                hwnd_target,
                brush,
                dashed,
                formats: HashMap::new(),
                bitmaps: HashMap::new(),
            })
        }
    }

    /// Changes the window target's pixel size and DPI (after a resize or a monitor change).
    pub fn resize(&mut self, width: u32, height: u32, dpi: f32) -> Result<()> {
        if let Some(t) = &self.hwnd_target {
            unsafe {
                t.Resize(&D2D_SIZE_U { width: width.max(1), height: height.max(1) })?;
                self.target.SetDpi(dpi, dpi);
            }
        }
        Ok(())
    }

    pub fn begin(&mut self, background: Color) {
        unsafe {
            self.target.BeginDraw();
            self.target.SetTransform(&IDENTITY);
            self.target.Clear(Some(&background.d2d()));
        }
    }

    /// Finishes drawing. An error with code [`RECREATE_TARGET`] means the canvas must be rebuilt.
    pub fn end(&mut self) -> Result<()> {
        unsafe { self.target.EndDraw(None, None) }
    }

    fn brush(&self, color: Color) -> &ID2D1SolidColorBrush {
        unsafe { self.brush.SetColor(&color.d2d()) };
        &self.brush
    }

    // ---- Clipping ----------------------------------------------------------------------

    pub fn push_clip(&mut self, r: Rect) {
        unsafe { self.target.PushAxisAlignedClip(&r.d2d(), D2D1_ANTIALIAS_MODE_ALIASED) }
    }

    pub fn pop_clip(&mut self) {
        unsafe { self.target.PopAxisAlignedClip() }
    }

    // ---- Shapes ------------------------------------------------------------------------

    pub fn fill_rect(&mut self, r: Rect, color: Color) {
        unsafe { self.target.FillRectangle(&r.d2d(), self.brush(color)) }
    }

    pub fn fill_round(&mut self, r: Rect, radius: f32, color: Color) {
        let rr = D2D1_ROUNDED_RECT { rect: r.d2d(), radiusX: radius, radiusY: radius };
        unsafe { self.target.FillRoundedRectangle(&rr, self.brush(color)) }
    }

    /// Outlines a rounded rectangle with the line drawn just inside `r`.
    pub fn stroke_round(&mut self, r: Rect, radius: f32, width: f32, color: Color) {
        let r = r.inset(width / 2.0, width / 2.0);
        let rr = D2D1_ROUNDED_RECT { rect: r.d2d(), radiusX: radius, radiusY: radius };
        unsafe { self.target.DrawRoundedRectangle(&rr, self.brush(color), width, None) }
    }

    /// A dashed outline, for drop zones and empty slots.
    pub fn stroke_round_dashed(&mut self, r: Rect, radius: f32, width: f32, color: Color) {
        let r = r.inset(width / 2.0, width / 2.0);
        let rr = D2D1_ROUNDED_RECT { rect: r.d2d(), radiusX: radius, radiusY: radius };
        unsafe { self.target.DrawRoundedRectangle(&rr, self.brush(color), width, &self.dashed) }
    }

    pub fn fill_circle(&mut self, cx: f32, cy: f32, radius: f32, color: Color) {
        let e = D2D1_ELLIPSE { point: Vector2 { X: cx, Y: cy }, radiusX: radius, radiusY: radius };
        unsafe { self.target.FillEllipse(&e, self.brush(color)) }
    }

    pub fn stroke_circle(&mut self, cx: f32, cy: f32, radius: f32, width: f32, color: Color) {
        let e = D2D1_ELLIPSE { point: Vector2 { X: cx, Y: cy }, radiusX: radius, radiusY: radius };
        unsafe { self.target.DrawEllipse(&e, self.brush(color), width, None) }
    }

    pub fn line(&mut self, x0: f32, y0: f32, x1: f32, y1: f32, width: f32, color: Color) {
        unsafe {
            self.target.DrawLine(Vector2 { X: x0, Y: y0 }, Vector2 { X: x1, Y: y1 }, self.brush(color), width, None)
        }
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
                self.target.FillGeometry(&geometry, self.brush(color), None);
            }
        }
    }

    // ---- Text --------------------------------------------------------------------------

    fn format(&mut self, font: Font, size: f32) -> Option<IDWriteTextFormat> {
        let key = (font, (size * 4.0).round() as u32);
        if let Some(f) = self.formats.get(&key) {
            return Some(f.clone());
        }
        let (family, weight) = match font {
            Font::Regular => ("Segoe UI", DWRITE_FONT_WEIGHT_NORMAL),
            Font::Semibold => ("Segoe UI", DWRITE_FONT_WEIGHT_SEMI_BOLD),
            Font::Icons => ("Segoe MDL2 Assets", DWRITE_FONT_WEIGHT_NORMAL),
        };
        let format = unsafe {
            let f = self
                .dwrite
                .CreateTextFormat(
                    &HSTRING::from(family),
                    None,
                    weight,
                    DWRITE_FONT_STYLE_NORMAL,
                    DWRITE_FONT_STRETCH_NORMAL,
                    size,
                    &HSTRING::from("en-gb"),
                )
                .ok()?;
            let _ = f.SetWordWrapping(DWRITE_WORD_WRAPPING_NO_WRAP);
            f
        };
        self.formats.insert(key, format.clone());
        Some(format)
    }

    /// A text layout, wrapped at `wrap` DIPs if given. Returns it with its width and height.
    fn layout(&mut self, text: &str, font: Font, size: f32, wrap: Option<f32>) -> Option<(IDWriteTextLayout, f32, f32)> {
        let format = self.format(font, size)?;
        let wide: Vec<u16> = text.encode_utf16().collect();
        unsafe {
            let layout = self.dwrite.CreateTextLayout(&wide, &format, wrap.unwrap_or(10000.0).max(1.0), 10000.0).ok()?;
            if wrap.is_some() {
                let _ = layout.SetWordWrapping(DWRITE_WORD_WRAPPING_WRAP);
            }
            let mut m = DWRITE_TEXT_METRICS::default();
            layout.GetMetrics(&mut m).ok()?;
            let width = if wrap.is_some() { m.width } else { m.widthIncludingTrailingWhitespace };
            Some((layout, width, m.height))
        }
    }

    fn draw_layout(&mut self, layout: &IDWriteTextLayout, x: f32, y: f32, color: Color) {
        unsafe {
            self.brush.SetColor(&color.d2d());
            self.target.DrawTextLayout(Vector2 { X: x, Y: y }, layout, &self.brush, D2D1_DRAW_TEXT_OPTIONS_NONE);
        }
    }

    /// Width and height of a single line of text.
    pub fn measure(&mut self, text: &str, font: Font, size: f32) -> (f32, f32) {
        self.layout(text, font, size, None).map(|(_, w, h)| (w, h)).unwrap_or((0.0, 0.0))
    }

    /// Height of `text` wrapped to `width`.
    pub fn measure_wrapped(&mut self, text: &str, font: Font, size: f32, width: f32) -> f32 {
        self.layout(text, font, size, Some(width)).map(|(_, _, h)| h).unwrap_or(0.0)
    }

    /// One line of text with its top-left corner at `(x, y)`. Returns its width.
    pub fn text(&mut self, text: &str, font: Font, size: f32, x: f32, y: f32, color: Color) -> f32 {
        let Some((layout, w, _)) = self.layout(text, font, size, None) else { return 0.0 };
        self.draw_layout(&layout, x, y, color);
        w
    }

    /// One line of text placed in `r`: vertically centred, horizontally by `align`.
    pub fn text_in(&mut self, text: &str, font: Font, size: f32, r: Rect, align: Align, color: Color) {
        let Some((layout, w, h)) = self.layout(text, font, size, None) else { return };
        let x = match align {
            Align::Start => r.x,
            Align::Center => r.cx() - w / 2.0,
            Align::End => r.right() - w,
        };
        self.draw_layout(&layout, x.round(), (r.cy() - h / 2.0).round(), color);
    }

    /// Text wrapped to `width`, top-left at `(x, y)`. Returns its height.
    pub fn text_wrapped(&mut self, text: &str, font: Font, size: f32, x: f32, y: f32, width: f32, color: Color) -> f32 {
        let Some((layout, _, h)) = self.layout(text, font, size, Some(width)) else { return 0.0 };
        self.draw_layout(&layout, x, y, color);
        h
    }

    /// Wrapped text centred horizontally in a column `width` wide. Returns its height.
    pub fn text_wrapped_centered(&mut self, text: &str, font: Font, size: f32, x: f32, y: f32, width: f32, color: Color) -> f32 {
        let Some((layout, _, h)) = self.layout(text, font, size, Some(width)) else { return 0.0 };
        unsafe {
            let _ = layout.SetTextAlignment(DWRITE_TEXT_ALIGNMENT_CENTER);
        }
        self.draw_layout(&layout, x, y, color);
        h
    }

    /// An icon from Segoe MDL2 Assets centred on `(cx, cy)`.
    pub fn glyph(&mut self, glyph: &str, size: f32, cx: f32, cy: f32, color: Color) {
        let r = Rect::new(cx - size, cy - size, size * 2.0, size * 2.0);
        self.text_in(glyph, Font::Icons, size, r, Align::Center, color);
    }

    // ---- Pictures ----------------------------------------------------------------------

    /// True if a picture is cached under `key`.
    pub fn has_bitmap(&self, key: &str) -> bool {
        self.bitmaps.contains_key(key)
    }

    /// Uploads a premultiplied RGBA picture and caches it under `key`.
    pub fn add_bitmap(&mut self, key: &str, image: &RgbaImage) {
        let mut bgra = image.as_raw().clone();
        for p in bgra.chunks_exact_mut(4) {
            p.swap(0, 2);
        }
        let props = D2D1_BITMAP_PROPERTIES {
            pixelFormat: D2D1_PIXEL_FORMAT { format: DXGI_FORMAT_B8G8R8A8_UNORM, alphaMode: D2D1_ALPHA_MODE_PREMULTIPLIED },
            dpiX: 96.0,
            dpiY: 96.0,
        };
        let size = D2D_SIZE_U { width: image.width(), height: image.height() };
        if let Ok(bitmap) = unsafe { self.target.CreateBitmap(size, Some(bgra.as_ptr() as *const _), image.width() * 4, &props) } {
            self.bitmaps.insert(key.to_string(), bitmap);
        }
    }

    /// Forgets a cached picture.
    pub fn forget_bitmap(&mut self, key: &str) {
        self.bitmaps.remove(key);
    }

    /// Forgets every cached picture whose key fails `keep`.
    pub fn retain_bitmaps(&mut self, keep: impl Fn(&str) -> bool) {
        self.bitmaps.retain(|k, _| keep(k));
    }

    /// Draws a cached picture stretched to `r`. Returns false if nothing is cached under `key`.
    pub fn bitmap(&mut self, key: &str, r: Rect, opacity: f32) -> bool {
        let Some(bitmap) = self.bitmaps.get(key) else { return false };
        unsafe {
            match &self.dc {
                Some(dc) => dc.DrawBitmap(
                    bitmap,
                    Some(&r.d2d()),
                    opacity,
                    D2D1_INTERPOLATION_MODE_HIGH_QUALITY_CUBIC,
                    None,
                    None,
                ),
                None => self.target.DrawBitmap(bitmap, Some(&r.d2d()), opacity, D2D1_BITMAP_INTERPOLATION_MODE_LINEAR, None),
            }
        }
        true
    }
}
