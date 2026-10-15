//! The gaming zone: a zone of an exact size (a game's resolution), placed in the layout
//! area. Other zones and guides can use its edges through the built-in guides `gz0.left`,
//! `gz0.right`, `gz0.top` and `gz0.bottom`, so they rearrange themselves around it.
//!
//! ```json
//! "gaming_zones": [ { "size": { "pixels": { "w": 3840, "h": 2160 } }, "x": "centre", "y": 200 } ]
//! ```

use std::fmt;

use serde::{Deserialize, Serialize};

use crate::geometry::Rect;

/// Largest width or height accepted. Nothing has a monitor this big, so a larger size is
/// a typo (and keeps the maths far from overflowing).
pub const MAX_SIZE: u32 = 16384;

/// How big the gaming zone is.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum GamingSize {
    /// An exact resolution.
    Pixels { w: u32, h: u32 },
    /// Step `k` of an aspect ratio: `k·(ratio_w/g) × k·(ratio_h/g)` with `g = gcd`, so the
    /// size always has the exact ratio (16:9 with k = 120 is 1920×1080).
    Aspect { ratio_w: u32, ratio_h: u32, k: u32 },
    /// A base resolution times a whole number (640×360 × 6 = 3840×2160), for pixel-art games.
    IntegerScale { base_w: u32, base_h: u32, scale: u32 },
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SizeError {
    Zero,
    TooLarge { w: u64, h: u64 },
}

impl fmt::Display for SizeError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Zero => write!(f, "its size is zero"),
            Self::TooLarge { w, h } => write!(f, "its size {w}×{h} is larger than {MAX_SIZE} px"),
        }
    }
}

impl std::error::Error for SizeError {}

impl GamingSize {
    /// Width and height in pixels.
    pub fn pixels(&self) -> Result<(i32, i32), SizeError> {
        let (w, h) = match *self {
            Self::Pixels { w, h } => (w as u64, h as u64),
            Self::Aspect { ratio_w, ratio_h, k } => {
                let g = gcd(ratio_w, ratio_h).max(1);
                (k as u64 * (ratio_w / g) as u64, k as u64 * (ratio_h / g) as u64)
            }
            Self::IntegerScale { base_w, base_h, scale } => {
                (base_w as u64 * scale as u64, base_h as u64 * scale as u64)
            }
        };
        if w == 0 || h == 0 {
            Err(SizeError::Zero)
        } else if w > MAX_SIZE as u64 || h > MAX_SIZE as u64 {
            Err(SizeError::TooLarge { w, h })
        } else {
            Ok((w as i32, h as i32))
        }
    }
}

/// Greatest common divisor (`gcd(0, 0)` = 0).
pub fn gcd(mut a: u32, mut b: u32) -> u32 {
    while b != 0 {
        (a, b) = (b, a % b);
    }
    a
}

/// The largest aspect step `k` whose size fits in `w × h` (0 if none does).
pub fn largest_aspect_step(ratio_w: u32, ratio_h: u32, w: i32, h: i32) -> u32 {
    let g = gcd(ratio_w, ratio_h);
    if g == 0 || ratio_w == 0 || ratio_h == 0 || w <= 0 || h <= 0 {
        return 0;
    }
    let (step_w, step_h) = (ratio_w / g, ratio_h / g);
    (w as u32 / step_w).min(h as u32 / step_h)
}

/// Where the gaming zone's top-left corner goes, on one axis.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(untagged)]
pub enum Position {
    /// Pixels from the layout area's left (x) or top (y) edge.
    Px(i32),
    /// `"centre"` (or `"center"`): centred in the layout area.
    Align(Align),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Align {
    #[serde(alias = "center")]
    Centre,
}

impl Default for Position {
    fn default() -> Self {
        Self::Align(Align::Centre)
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct GamingZone {
    pub size: GamingSize,
    #[serde(default)]
    pub x: Position,
    #[serde(default)]
    pub y: Position,
    /// Made from a zone (see `edit::make_gaming_zone`): the zones in that zone's space are its
    /// leftovers, and turning it back into a normal zone merges them again.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub from_zone: bool,
}

impl GamingZone {
    pub fn new(size: GamingSize, x: Position, y: Position) -> GamingZone {
        GamingZone { size, x, y, from_zone: false }
    }

    /// The zone's rectangle. Positions are relative to `area` (the layout area) and
    /// "centre" centres in it. The zone is then kept inside `bounds` (the work area, or the
    /// whole monitor if the layout lets the gaming zone cover the taskbar); a zone bigger
    /// than `bounds` is shrunk to fit.
    pub fn place(&self, area: Rect, bounds: Rect) -> Result<Rect, SizeError> {
        let (w, h) = self.size.pixels()?;
        let (w, h) = (w.min(bounds.width().max(0)), h.min(bounds.height().max(0)));
        let x = place_axis(self.x, area.left, area.width(), w, bounds.left, bounds.right);
        let y = place_axis(self.y, area.top, area.height(), h, bounds.top, bounds.bottom);
        Ok(Rect::from_xywh(x, y, w, h))
    }
}

fn place_axis(pos: Position, start: i32, len: i32, size: i32, lo: i32, hi: i32) -> i32 {
    let want = match pos {
        Position::Px(px) => start as i64 + px as i64,
        Position::Align(Align::Centre) => start as i64 + (len as i64 - size as i64).div_euclid(2),
    };
    want.clamp(lo as i64, (hi - size).max(lo) as i64) as i32
}

#[cfg(test)]
mod tests {
    use super::*;

    const WORK: Rect = Rect::new(0, 0, 6144, 2500);
    const MONITOR: Rect = Rect::new(0, 0, 6144, 2560);

    fn px(w: u32, h: u32) -> GamingSize {
        GamingSize::Pixels { w, h }
    }

    #[test]
    fn sizes() {
        assert_eq!(px(1920, 1080).pixels(), Ok((1920, 1080)));
        let aspect = |ratio_w, ratio_h, k| GamingSize::Aspect { ratio_w, ratio_h, k }.pixels();
        assert_eq!(aspect(16, 9, 120), Ok((1920, 1080)));
        assert_eq!(aspect(32, 18, 120), Ok((1920, 1080))); // same ratio, reduced first
        assert_eq!(aspect(16, 10, 160), Ok((1280, 800))); // 8:5 steps
        assert_eq!(aspect(21, 9, 480), Ok((3360, 1440))); // 7:3 steps
        assert_eq!(aspect(4, 3, 480), Ok((1920, 1440)));
        let scaled = GamingSize::IntegerScale { base_w: 640, base_h: 360, scale: 6 };
        assert_eq!(scaled.pixels(), Ok((3840, 2160)));
    }

    #[test]
    fn bad_sizes() {
        assert_eq!(px(0, 1080).pixels(), Err(SizeError::Zero));
        assert_eq!(GamingSize::Aspect { ratio_w: 0, ratio_h: 0, k: 5 }.pixels(), Err(SizeError::Zero));
        assert_eq!(GamingSize::Aspect { ratio_w: 16, ratio_h: 9, k: 0 }.pixels(), Err(SizeError::Zero));
        assert_eq!(px(16385, 10).pixels(), Err(SizeError::TooLarge { w: 16385, h: 10 }));
        let huge = GamingSize::IntegerScale { base_w: u32::MAX, base_h: u32::MAX, scale: u32::MAX };
        assert!(matches!(huge.pixels(), Err(SizeError::TooLarge { .. })));
    }

    #[test]
    fn aspect_steps() {
        assert_eq!(gcd(1920, 1080), 120);
        assert_eq!(gcd(7, 0), 7);
        // 16:9 in 6144×2500: width allows 384 steps, height 277.
        assert_eq!(largest_aspect_step(16, 9, 6144, 2500), 277);
        assert_eq!(GamingSize::Aspect { ratio_w: 16, ratio_h: 9, k: 277 }.pixels(), Ok((4432, 2493)));
        assert_eq!(largest_aspect_step(16, 9, 10, 10), 0);
        assert_eq!(largest_aspect_step(0, 9, 100, 100), 0);
    }

    #[test]
    fn json_shape() {
        let z: GamingZone = serde_json::from_str(
            r#"{ "size": { "aspect": { "ratio_w": 16, "ratio_h": 9, "k": 120 } }, "x": 100, "y": "center" }"#,
        )
        .unwrap();
        assert_eq!(z.x, Position::Px(100));
        assert_eq!(z.y, Position::Align(Align::Centre));
        let json = serde_json::to_string(&z).unwrap();
        assert!(json.contains(r#""y":"centre""#), "{json}");
        assert_eq!(serde_json::from_str::<GamingZone>(&json).unwrap(), z);
        // Position defaults to centre.
        let z: GamingZone =
            serde_json::from_str(r#"{ "size": { "integer_scale": { "base_w": 640, "base_h": 360, "scale": 2 } } }"#)
                .unwrap();
        assert_eq!((z.x, z.y), (Position::default(), Position::default()));
        assert!(serde_json::from_str::<GamingZone>(r#"{ "size": { "pixels": { "w": -1, "h": 5 } } }"#).is_err());
        assert!(
            serde_json::from_str::<GamingZone>(r#"{ "size": { "pixels": { "w": 1, "h": 5 } }, "x": "left" }"#).is_err()
        );
    }

    #[test]
    fn placement() {
        let zone = |x, y| GamingZone::new(px(3840, 2160), x, y);
        let centre = Position::default();
        // Centred in Tom's work area.
        assert_eq!(zone(centre, centre).place(WORK, WORK), Ok(Rect::new(1152, 170, 4992, 2330)));
        // Pixel positions are relative to the area, in screen coordinates.
        let area = Rect::new(6144, 0, 9984, 2016);
        let small = GamingZone::new(px(1920, 1080), Position::Px(10), Position::Px(20));
        assert_eq!(small.place(area, area), Ok(Rect::from_xywh(6154, 20, 1920, 1080)));
        // Clamped so it stays inside.
        assert_eq!(zone(Position::Px(5000), Position::Px(-50)).place(WORK, WORK), Ok(Rect::new(2304, 0, 6144, 2160)));
        // Too big: shrunk to fit.
        let big = GamingZone::new(px(7000, 3000), centre, centre);
        assert_eq!(big.place(WORK, WORK), Ok(WORK));
    }

    #[test]
    fn over_the_taskbar() {
        // Taller than the work area: shrunk to it normally, but allowed over the taskbar.
        let tall = GamingZone::new(px(4552, 2560), Position::default(), Position::default());
        assert_eq!(tall.place(WORK, WORK), Ok(Rect::new(796, 0, 5348, 2500)));
        assert_eq!(tall.place(WORK, MONITOR), Ok(Rect::new(796, 0, 5348, 2560)));
        // Taskbar on top (work area starts at 60): centring in the work area would start at
        // 30, which is fine over the taskbar.
        let work_top = Rect::new(0, 60, 6144, 2560);
        assert_eq!(tall.place(work_top, MONITOR).map(|r| r.top), Ok(0));
        let short = GamingZone::new(px(1920, 1080), Position::Px(0), Position::Px(-40));
        assert_eq!(short.place(work_top, MONITOR).map(|r| r.top), Ok(20));
        assert_eq!(short.place(work_top, work_top).map(|r| r.top), Ok(60));
    }
}
