//! Where the hand on its stick goes: someone off the bottom of the screen holds the stick and
//! reaches for the cursor with the fingertip. Pure maths; drawing is in `render`.
//!
//! Screen coordinates here are pixels inside the overlay, y growing downwards.

use crate::handasset::Metrics;

const MIN_ROD_FRAC: f32 = 0.05;
const GRIP_LEAN_FRAC: f32 = 0.30;
const GRIP_DEPTH_FRAC: f32 = 0.12;
const LIFT_FRAC: f32 = 0.06;
/// Per-frame easing amounts at 120 frames a second, as in the original app.
const GRIP_FOLLOW: f32 = 0.08;
const PRESS_DECAY: f32 = 0.18;

/// Everything `render` needs to draw the hand for one frame.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct HandPose {
    /// Maps arm space (x right, y down, hand on top, stick below, in pixels) to the screen:
    /// `x' = x*m[0] + y*m[2] + m[4]`, `y' = x*m[1] + y*m[3] + m[5]`.
    pub m: [f32; 6],
    pub width: f32,
    pub hand_h: f32,
    pub rod_len: f32,
    /// Height in pixels the whole source picture would have at this size.
    pub image_h: f32,
    pub shadow_dx: f32,
    pub shadow_dy: f32,
    pub shadow_blur: f32,
    /// Zero when shadows are switched off.
    pub shadow_opacity: f32,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct LaserPose {
    pub x: f32,
    pub y: f32,
    pub radius: f32,
    pub glow: f32,
}

#[derive(Default)]
pub struct Arm {
    /// 1 right after a tap, easing back to 0. Makes the hand dip.
    press: f32,
    grip_x: Option<f32>,
    grip_target: f32,
    /// True while a button is held: the hand stays pressed.
    pub held: bool,
}

fn ease(per_frame: f32, dt: f32) -> f32 {
    1.0 - (1.0 - per_frame).powf((dt * 120.0).clamp(0.0, 30.0))
}

impl Arm {
    pub fn press(&mut self) {
        self.press = 1.0;
    }

    pub fn reset(&mut self) {
        *self = Arm::default();
    }

    /// Advances the easing by `dt` seconds. `cursor_x` and `width` are in overlay pixels.
    pub fn step(&mut self, dt: f32, cursor_x: f32, width: f32) {
        let target = (cursor_x + GRIP_LEAN_FRAC * width).clamp(width * 0.15, width * 1.05);
        self.grip_target = target;
        self.grip_x = Some(match self.grip_x {
            Some(gx) => gx + (target - gx) * ease(GRIP_FOLLOW, dt),
            None => target,
        });
        if !self.held {
            self.press -= self.press * ease(PRESS_DECAY, dt);
            if self.press < 0.0005 {
                self.press = 0.0;
            }
        }
    }

    /// True while the hand is still gliding or recovering from a tap.
    pub fn is_settling(&self) -> bool {
        if self.press > 0.002 {
            return true;
        }
        match self.grip_x {
            Some(gx) => (self.grip_target - gx).abs() > 0.5,
            None => false,
        }
    }

    /// The hand's pose with its fingertip on the cursor. `size_frac` is the hand's height as
    /// a fraction of the screen height.
    pub fn hand_pose(&self, cursor: (f32, f32), screen: (f32, f32), m: &Metrics, size_frac: f32, shadow: bool) -> HandPose {
        let (w, h) = screen;
        // Work with y pointing up, like the original, then flip at the end.
        let cx = cursor.0;
        let cy = h - cursor.1;
        let grip_x = self.grip_x.unwrap_or_else(|| (cx + GRIP_LEAN_FRAC * w).clamp(w * 0.15, w * 1.05));
        let grip_y = -GRIP_DEPTH_FRAC * h;

        let dx = cx - grip_x;
        let dy = cy - grip_y;
        let reach = dx.hypot(dy);
        let aim_angle = dy.atan2(dx);

        let hand_h = size_frac * h * (1.0 - 0.06 * self.press);
        let s = hand_h / (m.wrist - m.hand_top).max(0.01);
        let width = s * m.aspect;
        let hand_tip = (m.wrist - m.tip) * s;
        let tip_off_x = (m.tip_x - m.rod_x) * width;

        let along = (reach * reach - tip_off_x * tip_off_x).max(0.0).sqrt();
        let rod_len = (along - hand_tip).max(MIN_ROD_FRAC * h);
        let tip_angle = (rod_len + hand_tip).atan2(tip_off_x);
        let rotation = aim_angle - tip_angle;

        let lift = LIFT_FRAC * h * (1.0 - 0.8 * self.press);
        let (sin, cos) = rotation.sin_cos();
        let anchor = m.rod_x * width;
        let total = rod_len + hand_h;
        HandPose {
            m: [
                cos,
                -sin,
                sin,
                cos,
                grip_x - cos * anchor - sin * total,
                h - grip_y + sin * anchor - cos * total,
            ],
            width,
            hand_h,
            rod_len,
            image_h: s,
            shadow_dx: lift * 0.6,
            shadow_dy: lift,
            shadow_blur: (lift * 0.35).max(1.0) + 5.0 * self.press,
            shadow_opacity: if shadow { 0.26 + 0.02 * self.press } else { 0.0 },
        }
    }

    pub fn laser_pose(&self, cursor: (f32, f32), screen_h: f32, size_frac: f32) -> LaserPose {
        let radius = (screen_h * 0.0038 * size_frac / 0.13).max(4.0) * (1.0 + 0.2 * self.press);
        LaserPose { x: cursor.0, y: cursor.1, radius, glow: radius * (2.2 + self.press) }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn metrics() -> Metrics {
        Metrics {
            aspect: 1024.0 / 1536.0,
            hand_top: 0.08,
            tip: 0.084,
            wrist: 0.44,
            rod_end: 0.9,
            tip_x: 0.46,
            rod_x: 0.5,
            hand_min_x: 0.3,
            hand_max_x: 0.7,
        }
    }

    #[test]
    fn fingertip_lands_on_cursor() {
        let m = metrics();
        for &(cx, cy) in &[(400.0f32, 300.0f32), (1800.0, 900.0), (50.0, 1000.0), (1900.0, 40.0)] {
            let mut arm = Arm::default();
            arm.step(1.0 / 120.0, cx, 1920.0);
            let p = arm.hand_pose((cx, cy), (1920.0, 1080.0), &m, 0.13, true);
            // The fingertip in arm space.
            let tx = m.tip_x * p.width;
            let ty = (m.tip - m.hand_top) * p.image_h;
            let sx = tx * p.m[0] + ty * p.m[2] + p.m[4];
            let sy = tx * p.m[1] + ty * p.m[3] + p.m[5];
            assert!((sx - cx).abs() < 1.0 && (sy - cy).abs() < 1.0, "tip at {sx},{sy}, cursor {cx},{cy}");
        }
    }

    #[test]
    fn press_eases_back() {
        let mut arm = Arm::default();
        arm.press();
        assert!(arm.is_settling());
        for _ in 0..240 {
            arm.step(1.0 / 120.0, 500.0, 1920.0);
        }
        assert!(!arm.is_settling());
    }
}
