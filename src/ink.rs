//! Lines drawn on the screen by holding and dragging. Finished lines fade after a while.

use std::time::{Duration, Instant};
use windows::Win32::Graphics::Direct2D::ID2D1PathGeometry1;

const FADE_DURATION: f64 = 0.6;

pub struct Stroke {
    pub points: Vec<(f32, f32)>,
    /// 0xRRGGBB.
    pub color: u32,
    pub width: f32,
    pub arrow: bool,
    /// When the fade starts. `None` while drawing, or for lines kept until cleared.
    pub fade_at: Option<Instant>,
    /// Built by the renderer; dropped whenever the line changes.
    pub geometry: Option<ID2D1PathGeometry1>,
}

impl Stroke {
    /// 1 for a solid line down to 0 for a fully faded one.
    pub fn opacity(&self, now: Instant) -> f32 {
        match self.fade_at {
            Some(at) if now > at => {
                let t = (now.duration_since(at).as_secs_f64() / FADE_DURATION).min(1.0) as f32;
                1.0 - t * t
            }
            _ => 1.0,
        }
    }

    /// The two barbs of the arrowhead as `(end, barb_tip)` pairs. Aims along the last
    /// stretch of the line, so a wobble at the very end doesn't twist the head.
    pub fn arrow_head(&self) -> Option<[((f32, f32), (f32, f32)); 2]> {
        if !self.arrow {
            return None;
        }
        let end = *self.points.last()?;
        let size = (self.width * 3.2).max(14.0);
        let dist = |p: &(f32, f32)| (end.0 - p.0).hypot(end.1 - p.1);
        let back = self.points.iter().rev().find(|p| dist(p) >= size * 1.2).or(self.points.first()).copied()?;
        let length = dist(&back);
        if length <= 1.0 {
            return None;
        }
        let head = size.min(length * 0.8);
        let angle = (end.1 - back.1).atan2(end.0 - back.0);
        let spread = std::f32::consts::PI / 7.0;
        let barb = |side: f32| {
            let a = angle + std::f32::consts::PI + side * spread;
            (end, (end.0 + a.cos() * head, end.1 + a.sin() * head))
        };
        Some([barb(-1.0), barb(1.0)])
    }
}

#[derive(Default)]
pub struct Ink {
    pub strokes: Vec<Stroke>,
    drawing: bool,
}

impl Ink {
    pub fn is_drawing(&self) -> bool {
        self.drawing
    }

    pub fn begin(&mut self, at: (f32, f32), width: f32, color: u32) {
        self.end(0.0);
        self.strokes.push(Stroke { points: vec![at], color, width, arrow: false, fade_at: None, geometry: None });
        self.drawing = true;
    }

    fn current(&mut self) -> Option<&mut Stroke> {
        if self.drawing {
            self.strokes.last_mut()
        } else {
            None
        }
    }

    pub fn extend(&mut self, to: (f32, f32)) {
        if let Some(s) = self.current() {
            let last = *s.points.last().unwrap();
            if (to.0 - last.0).hypot(to.1 - last.1) < 1.5 {
                return;
            }
            s.points.push(to);
            s.geometry = None;
        }
    }

    pub fn set_arrow(&mut self, arrow: bool) {
        if let Some(s) = self.current() {
            if s.arrow != arrow {
                s.arrow = arrow;
                s.geometry = None;
            }
        }
    }

    /// Finishes the line being drawn. It fades after `fade_seconds`; zero or less keeps it.
    pub fn end(&mut self, fade_seconds: f64) {
        if let Some(s) = self.current() {
            if fade_seconds > 0.0 {
                s.fade_at = Some(Instant::now() + Duration::from_secs_f64(fade_seconds.max(0.5)));
            }
        }
        self.drawing = false;
    }

    pub fn clear(&mut self) {
        self.strokes.clear();
        self.drawing = false;
    }

    /// Drops lines that have finished fading. Returns true while any line is mid-fade,
    /// which means the screen needs redrawing every frame.
    pub fn update(&mut self, now: Instant) -> bool {
        let fade = Duration::from_secs_f64(FADE_DURATION);
        self.strokes.retain(|s| match s.fade_at {
            Some(at) => now < at + fade,
            None => true,
        });
        self.strokes.iter().any(|s| matches!(s.fade_at, Some(at) if now >= at))
    }

    /// When the next fade begins, if one is pending.
    pub fn next_fade(&self, now: Instant) -> Option<Instant> {
        self.strokes.iter().filter_map(|s| s.fade_at).filter(|at| *at > now).min()
    }
}
